use crate::collection::CollectionReport;
use crate::metric_support::{RegisterMetricResultExt, prometheus_u64};
use procfs::prelude::Current;
use procfs::{CpuPressure, IoPressure, MemoryPressure, PressureRecord};
use prometheus::{Counter, GaugeVec};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

struct MonotonicPressureCounter {
    metric: Counter,
    previous_micros: AtomicU64,
}

impl MonotonicPressureCounter {
    fn new(name: &'static str, help: &'static str) -> Self {
        Self {
            metric: prometheus::register_counter!(name, help).or_exit(name),
            previous_micros: AtomicU64::new(0),
        }
    }

    fn update(&self, current_micros: u64) {
        let previous = self.previous_micros.swap(current_micros, Ordering::Relaxed);
        let delta = if current_micros >= previous {
            current_micros - previous
        } else {
            // PSI totals reset only with the kernel. If the exporter somehow
            // survives such a reset, start accumulating the new epoch rather
            // than freezing until it catches the old value.
            current_micros
        };
        self.metric.inc_by(prometheus_u64(delta) / 1_000_000.0);
    }
}

struct PressureMetrics {
    cpu_waiting: MonotonicPressureCounter,
    io_waiting: MonotonicPressureCounter,
    io_stalled: MonotonicPressureCounter,
    memory_waiting: MonotonicPressureCounter,
    memory_stalled: MonotonicPressureCounter,
    stall_ratio: GaugeVec,
}

impl PressureMetrics {
    fn new() -> Self {
        Self {
            cpu_waiting: MonotonicPressureCounter::new(
                "pressure_cpu_waiting_seconds_total",
                "Total time in seconds that processes have waited for CPU time",
            ),
            io_waiting: MonotonicPressureCounter::new(
                "pressure_io_waiting_seconds_total",
                "Total time in seconds that processes have waited due to I/O congestion",
            ),
            io_stalled: MonotonicPressureCounter::new(
                "pressure_io_stalled_seconds_total",
                "Total time in seconds no process could make progress due to I/O congestion",
            ),
            memory_waiting: MonotonicPressureCounter::new(
                "pressure_memory_waiting_seconds_total",
                "Total time in seconds that processes have waited for memory",
            ),
            memory_stalled: MonotonicPressureCounter::new(
                "pressure_memory_stalled_seconds_total",
                "Total time in seconds no process could make progress due to memory congestion",
            ),
            stall_ratio: prometheus::register_gauge_vec!(
                "pressure_stall_ratio",
                "Fraction of wall time stalled over the PSI averaging window",
                &["resource", "scope", "window"]
            )
            .or_exit("pressure_stall_ratio"),
        }
    }
}

static PRESSURE_METRICS: OnceLock<PressureMetrics> = OnceLock::new();

fn metrics() -> &'static PressureMetrics {
    PRESSURE_METRICS.get_or_init(PressureMetrics::new)
}

fn set_ratios(metric: &GaugeVec, resource: &str, scope: &str, record: &PressureRecord) {
    for (window, value) in [
        ("10", record.avg10),
        ("60", record.avg60),
        ("300", record.avg300),
    ] {
        // The kernel exposes averages with two decimal places. `procfs`
        // stores them as f32, so round back to that source precision before
        // converting percent to a ratio for Prometheus.
        let ratio = (f64::from(value) * 100.0).round() / 10_000.0;
        metric
            .with_label_values(&[resource, scope, window])
            .set(ratio);
    }
}

pub fn update_metrics() -> CollectionReport {
    let metrics = metrics();
    metrics.stall_ratio.reset();
    let mut report = CollectionReport::success();

    match CpuPressure::current() {
        Ok(pressure) => {
            metrics.cpu_waiting.update(pressure.some.total);
            set_ratios(&metrics.stall_ratio, "cpu", "some", &pressure.some);
        }
        Err(_) => report.record_error(),
    }

    match MemoryPressure::current() {
        Ok(pressure) => {
            metrics.memory_waiting.update(pressure.some.total);
            metrics.memory_stalled.update(pressure.full.total);
            set_ratios(&metrics.stall_ratio, "memory", "some", &pressure.some);
            set_ratios(&metrics.stall_ratio, "memory", "full", &pressure.full);
        }
        Err(_) => report.record_error(),
    }

    match IoPressure::current() {
        Ok(pressure) => {
            metrics.io_waiting.update(pressure.some.total);
            metrics.io_stalled.update(pressure.full.total);
            set_ratios(&metrics.stall_ratio, "io", "some", &pressure.some);
            set_ratios(&metrics.stall_ratio, "io", "full", &pressure.full);
        }
        Err(_) => report.record_error(),
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_ratio_is_normalized_to_fraction() {
        let record = PressureRecord {
            avg10: 12.5,
            avg60: 2.0,
            avg300: 0.5,
            total: 123,
        };
        let gauge = prometheus::GaugeVec::new(
            prometheus::Opts::new("test_pressure_ratio", "test"),
            &["resource", "scope", "window"],
        )
        .unwrap();
        set_ratios(&gauge, "io", "some", &record);
        assert!(
            (gauge.with_label_values(&["io", "some", "10"]).get() - 0.125).abs() < f64::EPSILON
        );
    }
}
