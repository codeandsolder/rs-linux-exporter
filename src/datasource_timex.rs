use crate::collection::CollectionReport;
use crate::metric_support::{RegisterMetricResultExt, prometheus_i64, prometheus_u64};
use crate::runtime::debug_enabled;
use prometheus::{CounterVec, GaugeVec};
use rtime_clock::adjtime::{Timex, adjtimex};
use std::io::ErrorKind;
use std::sync::{Mutex, OnceLock};

const TIME_ERROR: i32 = 5;
const STA_NANO: i64 = 0x2000;
const MICROSECONDS: f64 = 1_000_000.0;
const NANOSECONDS: f64 = 1_000_000_000.0;
const PPM_16_FRAC: f64 = 1_000_000.0 * 65_536.0;

#[derive(Debug, Clone, Copy)]
struct TimexSnapshot {
    sync_status: f64,
    offset_seconds: f64,
    frequency_adjustment_ratio: f64,
    maxerror_seconds: f64,
    estimated_error_seconds: f64,
    status: f64,
    loop_time_constant: f64,
    tick_seconds: f64,
    pps_frequency_hertz: f64,
    pps_jitter_seconds: f64,
    pps_shift_seconds: f64,
    pps_stability_hertz: f64,
    tai_offset_seconds: f64,
    pps_counters: [u64; 4],
}

struct TimexMetrics {
    offset: GaugeVec,
    frequency: GaugeVec,
    maxerror: GaugeVec,
    estimated_error: GaugeVec,
    status: GaugeVec,
    constant: GaugeVec,
    tick: GaugeVec,
    pps_frequency: GaugeVec,
    pps_jitter: GaugeVec,
    pps_shift: GaugeVec,
    pps_stability: GaugeVec,
    tai: GaugeVec,
    sync_status: GaugeVec,
    pps_jitter_total: CounterVec,
    pps_calibration_total: CounterVec,
    pps_error_total: CounterVec,
    pps_stability_exceeded_total: CounterVec,
    previous: Mutex<Option<[u64; 4]>>,
}

impl TimexMetrics {
    fn new() -> Self {
        Self {
            offset: gauge(
                "timex_offset_seconds",
                "Time offset between local system and reference clock.",
            ),
            frequency: gauge(
                "timex_frequency_adjustment_ratio",
                "Local clock frequency adjustment.",
            ),
            maxerror: gauge("timex_maxerror_seconds", "Maximum clock error in seconds."),
            estimated_error: gauge(
                "timex_estimated_error_seconds",
                "Estimated clock error in seconds.",
            ),
            status: gauge("timex_status", "Value of the timex status bits."),
            constant: gauge(
                "timex_loop_time_constant",
                "Phase-locked loop time constant.",
            ),
            tick: gauge("timex_tick_seconds", "Seconds between clock ticks."),
            pps_frequency: gauge("timex_pps_frequency_hertz", "Pulse-per-second frequency."),
            pps_jitter: gauge("timex_pps_jitter_seconds", "Pulse-per-second jitter."),
            pps_shift: gauge(
                "timex_pps_shift_seconds",
                "Pulse-per-second interval duration.",
            ),
            pps_stability: gauge(
                "timex_pps_stability_hertz",
                "Pulse-per-second stability, average of recent frequency changes.",
            ),
            tai: gauge(
                "timex_tai_offset_seconds",
                "International Atomic Time (TAI) offset.",
            ),
            sync_status: gauge(
                "timex_sync_status",
                "Whether the clock is synchronized to a reliable server (1=yes, 0=no).",
            ),
            pps_jitter_total: counter(
                "timex_pps_jitter_total",
                "Pulse-per-second count of jitter limit exceeded events.",
            ),
            pps_calibration_total: counter(
                "timex_pps_calibration_total",
                "Pulse-per-second count of calibration intervals.",
            ),
            pps_error_total: counter(
                "timex_pps_error_total",
                "Pulse-per-second count of calibration errors.",
            ),
            pps_stability_exceeded_total: counter(
                "timex_pps_stability_exceeded_total",
                "Pulse-per-second count of stability limit exceeded events.",
            ),
            previous: Mutex::new(None),
        }
    }

    fn reset_gauges(&self) {
        self.offset.reset();
        self.frequency.reset();
        self.maxerror.reset();
        self.estimated_error.reset();
        self.status.reset();
        self.constant.reset();
        self.tick.reset();
        self.pps_frequency.reset();
        self.pps_jitter.reset();
        self.pps_shift.reset();
        self.pps_stability.reset();
        self.tai.reset();
        self.sync_status.reset();
    }

    fn clear(&self) -> CollectionReport {
        self.reset_gauges();
        self.pps_jitter_total.reset();
        self.pps_calibration_total.reset();
        self.pps_error_total.reset();
        self.pps_stability_exceeded_total.reset();
        let Ok(mut previous) = self.previous.lock() else {
            return CollectionReport::error();
        };
        *previous = None;
        CollectionReport::success()
    }

    fn apply(&self, snapshot: TimexSnapshot) -> CollectionReport {
        self.reset_gauges();
        set(&self.offset, snapshot.offset_seconds);
        set(&self.frequency, snapshot.frequency_adjustment_ratio);
        set(&self.maxerror, snapshot.maxerror_seconds);
        set(&self.estimated_error, snapshot.estimated_error_seconds);
        set(&self.status, snapshot.status);
        set(&self.constant, snapshot.loop_time_constant);
        set(&self.tick, snapshot.tick_seconds);
        set(&self.pps_frequency, snapshot.pps_frequency_hertz);
        set(&self.pps_jitter, snapshot.pps_jitter_seconds);
        set(&self.pps_shift, snapshot.pps_shift_seconds);
        set(&self.pps_stability, snapshot.pps_stability_hertz);
        set(&self.tai, snapshot.tai_offset_seconds);
        set(&self.sync_status, snapshot.sync_status);

        let Ok(mut previous) = self.previous.lock() else {
            return CollectionReport::error();
        };
        let deltas = previous.map_or(snapshot.pps_counters, |old| {
            std::array::from_fn(|index| monotonic_delta(old[index], snapshot.pps_counters[index]))
        });
        increment(&self.pps_jitter_total, deltas[0]);
        increment(&self.pps_calibration_total, deltas[1]);
        increment(&self.pps_error_total, deltas[2]);
        increment(&self.pps_stability_exceeded_total, deltas[3]);
        *previous = Some(snapshot.pps_counters);
        CollectionReport::success()
    }
}

fn gauge(name: &'static str, help: &'static str) -> GaugeVec {
    prometheus::register_gauge_vec!(name, help, &[]).or_exit(name)
}

fn counter(name: &'static str, help: &'static str) -> CounterVec {
    prometheus::register_counter_vec!(name, help, &[]).or_exit(name)
}

fn set(metric: &GaugeVec, value: f64) {
    metric.with_label_values(&[] as &[&str]).set(value);
}

fn increment(metric: &CounterVec, value: u64) {
    metric
        .with_label_values(&[] as &[&str])
        .inc_by(prometheus_u64(value));
}

static METRICS: OnceLock<TimexMetrics> = OnceLock::new();

fn metrics() -> &'static TimexMetrics {
    METRICS.get_or_init(TimexMetrics::new)
}

const fn monotonic_delta(previous: u64, current: u64) -> u64 {
    if current >= previous {
        current - previous
    } else {
        current
    }
}

fn signed<T: Into<i64>>(value: T) -> i64 {
    value.into()
}

fn nonnegative(value: i64, field: &str) -> Result<u64, String> {
    u64::try_from(value).map_err(|_| format!("timex {field} is negative: {value}"))
}

fn snapshot(status: i32, tx: &Timex) -> Result<TimexSnapshot, String> {
    let raw = &tx.0;
    let raw_status = signed(raw.status);
    let divisor = if raw_status & STA_NANO != 0 {
        NANOSECONDS
    } else {
        MICROSECONDS
    };
    Ok(TimexSnapshot {
        sync_status: if status == TIME_ERROR { 0.0 } else { 1.0 },
        offset_seconds: prometheus_i64(signed(raw.offset)) / divisor,
        frequency_adjustment_ratio: 1.0 + prometheus_i64(signed(raw.freq)) / PPM_16_FRAC,
        maxerror_seconds: prometheus_i64(signed(raw.maxerror)) / MICROSECONDS,
        estimated_error_seconds: prometheus_i64(signed(raw.esterror)) / MICROSECONDS,
        status: prometheus_i64(raw_status),
        loop_time_constant: prometheus_i64(signed(raw.constant)),
        tick_seconds: prometheus_i64(signed(raw.tick)) / MICROSECONDS,
        pps_frequency_hertz: prometheus_i64(signed(raw.ppsfreq)) / PPM_16_FRAC,
        pps_jitter_seconds: prometheus_i64(signed(raw.jitter)) / divisor,
        pps_shift_seconds: prometheus_i64(signed(raw.shift)),
        pps_stability_hertz: prometheus_i64(signed(raw.stabil)) / PPM_16_FRAC,
        tai_offset_seconds: prometheus_i64(signed(raw.tai)),
        pps_counters: [
            nonnegative(signed(raw.jitcnt), "jitcnt")?,
            nonnegative(signed(raw.calcnt), "calcnt")?,
            nonnegative(signed(raw.errcnt), "errcnt")?,
            nonnegative(signed(raw.stbcnt), "stbcnt")?,
        ],
    })
}

pub fn update_metrics() -> CollectionReport {
    let mut tx = Timex::new();
    match adjtimex(&mut tx) {
        Ok(status) => match snapshot(status, &tx) {
            Ok(snapshot) => metrics().apply(snapshot),
            Err(error) => {
                if debug_enabled() {
                    eprintln!("timex: invalid kernel response: {error}");
                }
                CollectionReport::error()
            }
        },
        Err(error) if error.kind() == ErrorKind::PermissionDenied => metrics().clear(),
        Err(error) => {
            if debug_enabled() {
                eprintln!("timex: adjtimex query failed: {error}");
            }
            CollectionReport::error()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TIME_ERROR, Timex, monotonic_delta, snapshot};

    #[test]
    fn converts_kernel_units_like_node_exporter() {
        let mut tx = Timex::new();
        tx.0.offset = 250_000;
        tx.0.freq = 65_536;
        tx.0.maxerror = 1_500_000;
        tx.0.esterror = 500_000;
        tx.0.tick = 10_000;
        tx.0.tai = 37;
        let value = snapshot(TIME_ERROR, &tx).unwrap();
        assert_eq!(value.sync_status, 0.0);
        assert_eq!(value.offset_seconds, 0.25);
        assert_eq!(value.frequency_adjustment_ratio, 1.000_001);
        assert_eq!(value.maxerror_seconds, 1.5);
        assert_eq!(value.estimated_error_seconds, 0.5);
        assert_eq!(value.tick_seconds, 0.01);
        assert_eq!(value.tai_offset_seconds, 37.0);
    }

    #[test]
    fn counter_reset_starts_a_new_monotonic_segment() {
        assert_eq!(monotonic_delta(10, 12), 2);
        assert_eq!(monotonic_delta(12, 3), 3);
    }
}
