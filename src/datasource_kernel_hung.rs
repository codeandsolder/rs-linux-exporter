use crate::collection::CollectionReport;
use crate::metric_support::{RegisterMetricResultExt, prometheus_u64};
use crate::runtime::debug_enabled;
use prometheus::Counter;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

const HUNG_TASK_COUNT_PATH: &str = "/proc/sys/kernel/hung_task_detect_count";

struct KernelHungMetrics {
    tasks_total: Counter,
    previous: Mutex<Option<u64>>,
}

impl KernelHungMetrics {
    fn new() -> Self {
        Self {
            tasks_total: prometheus::register_counter!(
                "kernel_hung_tasks_total",
                "Number of tasks detected as hung by the kernel."
            )
            .or_exit("kernel_hung_tasks_total"),
            previous: Mutex::new(None),
        }
    }

    fn apply(&self, current: u64) -> CollectionReport {
        let Ok(mut previous) = self.previous.lock() else {
            return CollectionReport::error();
        };
        let delta = previous.map_or(current, |value| monotonic_delta(value, current));
        self.tasks_total.inc_by(prometheus_u64(delta));
        *previous = Some(current);
        CollectionReport::success()
    }
}

static METRICS: OnceLock<KernelHungMetrics> = OnceLock::new();

fn metrics() -> &'static KernelHungMetrics {
    METRICS.get_or_init(KernelHungMetrics::new)
}

const fn monotonic_delta(previous: u64, current: u64) -> u64 {
    if current >= previous {
        current - previous
    } else {
        current
    }
}

fn parse_count(contents: &str) -> Result<u64, String> {
    contents
        .trim()
        .parse::<u64>()
        .map_err(|error| format!("invalid hung-task count {:?}: {error}", contents.trim()))
}

fn update_metrics_from_path(path: &Path) -> CollectionReport {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => return CollectionReport::success(),
        Err(error) => {
            if debug_enabled() {
                eprintln!("kernel_hung: failed to read {}: {error}", path.display());
            }
            return CollectionReport::error();
        }
    };
    match parse_count(&contents) {
        Ok(count) => metrics().apply(count),
        Err(error) => {
            if debug_enabled() {
                eprintln!("kernel_hung: failed to parse {}: {error}", path.display());
            }
            CollectionReport::error()
        }
    }
}

pub fn update_metrics() -> CollectionReport {
    update_metrics_from_path(Path::new(HUNG_TASK_COUNT_PATH))
}

#[cfg(test)]
mod tests {
    use super::{monotonic_delta, parse_count};

    #[test]
    fn parses_kernel_count() {
        assert_eq!(parse_count("5\n"), Ok(5));
        assert!(parse_count("nope\n").is_err());
    }

    #[test]
    fn counter_reset_starts_a_new_monotonic_segment() {
        assert_eq!(monotonic_delta(5, 8), 3);
        assert_eq!(monotonic_delta(8, 2), 2);
    }
}
