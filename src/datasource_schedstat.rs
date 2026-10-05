use crate::collection::CollectionReport;
use crate::metric_support::{RegisterMetricResultExt, prometheus_u64};
use crate::runtime::debug_enabled;
use prometheus::CounterVec;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

const SCHEDSTAT_PATH: &str = "/proc/schedstat";
const NS_PER_SECOND: f64 = 1_000_000_000.0;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CpuSchedstat {
    cpu: String,
    running_ns: u64,
    waiting_ns: u64,
    timeslices: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RawCounters {
    running_ns: u64,
    waiting_ns: u64,
    timeslices: u64,
}

struct SchedstatMetrics {
    running_seconds_total: CounterVec,
    waiting_seconds_total: CounterVec,
    timeslices_total: CounterVec,
    previous: Mutex<HashMap<String, RawCounters>>,
}

impl SchedstatMetrics {
    fn new() -> Self {
        Self {
            running_seconds_total: prometheus::register_counter_vec!(
                "schedstat_running_seconds_total",
                "Number of seconds CPU spent running a process.",
                &["cpu"]
            )
            .or_exit("schedstat_running_seconds_total"),
            waiting_seconds_total: prometheus::register_counter_vec!(
                "schedstat_waiting_seconds_total",
                "Number of seconds spent by processes waiting for this CPU.",
                &["cpu"]
            )
            .or_exit("schedstat_waiting_seconds_total"),
            timeslices_total: prometheus::register_counter_vec!(
                "schedstat_timeslices_total",
                "Number of timeslices executed by CPU.",
                &["cpu"]
            )
            .or_exit("schedstat_timeslices_total"),
            previous: Mutex::new(HashMap::new()),
        }
    }

    fn apply(&self, stats: &[CpuSchedstat]) -> CollectionReport {
        let Ok(mut previous) = self.previous.lock() else {
            return CollectionReport::error();
        };
        let mut seen = HashSet::with_capacity(stats.len());
        for cpu in stats {
            let current = RawCounters {
                running_ns: cpu.running_ns,
                waiting_ns: cpu.waiting_ns,
                timeslices: cpu.timeslices,
            };
            let old = previous
                .insert(cpu.cpu.clone(), current)
                .unwrap_or_default();
            let labels = [cpu.cpu.as_str()];
            self.running_seconds_total
                .with_label_values(&labels)
                .inc_by(
                    prometheus_u64(monotonic_delta(old.running_ns, current.running_ns))
                        / NS_PER_SECOND,
                );
            self.waiting_seconds_total
                .with_label_values(&labels)
                .inc_by(
                    prometheus_u64(monotonic_delta(old.waiting_ns, current.waiting_ns))
                        / NS_PER_SECOND,
                );
            self.timeslices_total
                .with_label_values(&labels)
                .inc_by(prometheus_u64(monotonic_delta(
                    old.timeslices,
                    current.timeslices,
                )));
            seen.insert(cpu.cpu.clone());
        }

        let stale = previous
            .keys()
            .filter(|cpu| !seen.contains(*cpu))
            .cloned()
            .collect::<Vec<_>>();
        let mut report = CollectionReport::success();
        for cpu in stale {
            let labels = [cpu.as_str()];
            for family in [
                &self.running_seconds_total,
                &self.waiting_seconds_total,
                &self.timeslices_total,
            ] {
                if let Err(error) = family.remove_label_values(&labels) {
                    report.record_error();
                    if debug_enabled() {
                        eprintln!("schedstat: failed to remove stale CPU {cpu:?}: {error}");
                    }
                }
            }
            previous.remove(&cpu);
        }
        report
    }

    fn clear_unavailable(&self) -> CollectionReport {
        let Ok(mut previous) = self.previous.lock() else {
            return CollectionReport::error();
        };
        let cpus = previous.keys().cloned().collect::<Vec<_>>();
        let mut report = CollectionReport::success();
        for cpu in cpus {
            let labels = [cpu.as_str()];
            for family in [
                &self.running_seconds_total,
                &self.waiting_seconds_total,
                &self.timeslices_total,
            ] {
                if let Err(error) = family.remove_label_values(&labels) {
                    report.record_error();
                    if debug_enabled() {
                        eprintln!("schedstat: failed to clear CPU {cpu:?}: {error}");
                    }
                }
            }
        }
        previous.clear();
        report
    }
}

static SCHEDSTAT_METRICS: OnceLock<SchedstatMetrics> = OnceLock::new();

fn metrics() -> &'static SchedstatMetrics {
    SCHEDSTAT_METRICS.get_or_init(SchedstatMetrics::new)
}

const fn monotonic_delta(previous: u64, current: u64) -> u64 {
    if current >= previous {
        current - previous
    } else {
        current
    }
}

fn parse_schedstat(contents: &str) -> Result<Vec<CpuSchedstat>, String> {
    let mut cpus = Vec::new();
    for (line_index, line) in contents.lines().enumerate() {
        let line_number = line_index + 1;
        let mut fields = line.split_whitespace();
        let Some(name) = fields.next() else {
            continue;
        };
        let Some(cpu) = name.strip_prefix("cpu") else {
            continue;
        };
        if cpu.is_empty() || !cpu.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let values = fields.collect::<Vec<_>>();
        if values.len() < 9 {
            return Err(format!(
                "line {line_number}: CPU {cpu} has {} scheduler fields, expected at least 9",
                values.len()
            ));
        }
        let parse = |index: usize, field: &str| -> Result<u64, String> {
            values[index].parse::<u64>().map_err(|error| {
                format!(
                    "line {line_number}: CPU {cpu} invalid {field} value {:?}: {error}",
                    values[index]
                )
            })
        };
        cpus.push(CpuSchedstat {
            cpu: cpu.to_string(),
            running_ns: parse(6, "running nanoseconds")?,
            waiting_ns: parse(7, "waiting nanoseconds")?,
            timeslices: parse(8, "timeslices")?,
        });
    }
    if cpus.is_empty() {
        return Err("no CPU scheduler rows found".to_string());
    }
    Ok(cpus)
}

fn update_metrics_from_path(path: &Path) -> CollectionReport {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) => {
            if debug_enabled() {
                eprintln!("schedstat: failed to read {}: {error}", path.display());
            }
            let mut report = metrics().clear_unavailable();
            report.record_error();
            return report;
        }
    };
    let stats = match parse_schedstat(&contents) {
        Ok(stats) => stats,
        Err(error) => {
            if debug_enabled() {
                eprintln!("schedstat: failed to parse {}: {error}", path.display());
            }
            let mut report = metrics().clear_unavailable();
            report.record_error();
            return report;
        }
    };
    metrics().apply(&stats)
}

pub fn update_metrics() -> CollectionReport {
    update_metrics_from_path(Path::new(SCHEDSTAT_PATH))
}

#[cfg(test)]
mod tests {
    use super::{CpuSchedstat, RawCounters, SchedstatMetrics, monotonic_delta, parse_schedstat};
    use prometheus::{CounterVec, Opts};
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[test]
    fn parses_v17_and_ignores_domains() {
        let input = "version 17\ntimestamp 123\ncpu0 0 0 0 0 0 0 41857993446785 20494255521318 57339171\ndomain0 MC ff 0 0 0\ncpu12 1 2 3 4 5 6 700 800 900 1000\n";
        assert_eq!(
            parse_schedstat(input).unwrap(),
            vec![
                CpuSchedstat {
                    cpu: "0".to_string(),
                    running_ns: 41_857_993_446_785,
                    waiting_ns: 20_494_255_521_318,
                    timeslices: 57_339_171,
                },
                CpuSchedstat {
                    cpu: "12".to_string(),
                    running_ns: 700,
                    waiting_ns: 800,
                    timeslices: 900,
                },
            ]
        );
    }

    #[test]
    fn future_trailing_fields_are_ignored() {
        let input = "cpu0 0 0 0 0 0 0 7 8 9 10 11\n";
        let parsed = parse_schedstat(input).unwrap();
        assert_eq!(parsed[0].running_ns, 7);
        assert_eq!(parsed[0].waiting_ns, 8);
        assert_eq!(parsed[0].timeslices, 9);
    }

    #[test]
    fn rejects_malformed_cpu_rows_and_empty_tables() {
        assert!(parse_schedstat("version 17\n").is_err());
        assert!(parse_schedstat("cpu0 0 0 0\n").is_err());
        assert!(parse_schedstat("cpu0 0 0 0 0 0 0 nope 8 9\n").is_err());
    }

    #[test]
    fn unavailable_collection_removes_stale_cpu_series() {
        let metrics = SchedstatMetrics {
            running_seconds_total: CounterVec::new(
                Opts::new("test_schedstat_running_seconds_total", "test"),
                &["cpu"],
            )
            .unwrap(),
            waiting_seconds_total: CounterVec::new(
                Opts::new("test_schedstat_waiting_seconds_total", "test"),
                &["cpu"],
            )
            .unwrap(),
            timeslices_total: CounterVec::new(
                Opts::new("test_schedstat_timeslices_total", "test"),
                &["cpu"],
            )
            .unwrap(),
            previous: Mutex::new(HashMap::<String, RawCounters>::new()),
        };
        assert!(
            metrics
                .apply(&[CpuSchedstat {
                    cpu: "test-cpu".to_string(),
                    running_ns: 10,
                    waiting_ns: 20,
                    timeslices: 30,
                }])
                .is_success()
        );
        let count = |families: Vec<prometheus::proto::MetricFamily>| {
            families
                .iter()
                .map(|family| family.get_metric().len())
                .sum::<usize>()
        };
        assert_eq!(
            count(prometheus::core::Collector::collect(
                &metrics.running_seconds_total
            )),
            1
        );
        assert!(metrics.clear_unavailable().is_success());
        assert_eq!(
            count(prometheus::core::Collector::collect(
                &metrics.running_seconds_total
            )),
            0
        );
        assert_eq!(
            count(prometheus::core::Collector::collect(
                &metrics.waiting_seconds_total
            )),
            0
        );
        assert_eq!(
            count(prometheus::core::Collector::collect(
                &metrics.timeslices_total
            )),
            0
        );
    }

    #[test]
    fn monotonic_delta_handles_source_reset() {
        assert_eq!(monotonic_delta(100, 140), 40);
        assert_eq!(monotonic_delta(100, 12), 12);
    }
}
