use crate::collection::CollectionReport;
use crate::metric_support::RegisterMetricResultExt;
use crate::runtime::debug_enabled;
use prometheus::{Gauge, GaugeVec};
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

const CLOCKSOURCE_PATH: &str = "/sys/devices/system/clocksource";

#[derive(Debug, PartialEq, Eq)]
struct ClocksourceSample {
    device: String,
    current: String,
    available: Vec<String>,
}

struct TimeMetrics {
    seconds: Gauge,
    clocksource_available: GaugeVec,
    clocksource_current: GaugeVec,
}

impl TimeMetrics {
    fn new() -> Self {
        Self {
            seconds: prometheus::register_gauge!(
                "time_seconds",
                "System time in seconds since the Unix epoch."
            )
            .or_exit("time_seconds"),
            clocksource_available: prometheus::register_gauge_vec!(
                "time_clocksource_available_info",
                "Available kernel clocksources.",
                &["device", "clocksource"]
            )
            .or_exit("time_clocksource_available_info"),
            clocksource_current: prometheus::register_gauge_vec!(
                "time_clocksource_current_info",
                "Current kernel clocksource.",
                &["device", "clocksource"]
            )
            .or_exit("time_clocksource_current_info"),
        }
    }

    fn apply(&self, now: f64, clocksources: &[ClocksourceSample]) {
        self.seconds.set(now);
        self.clocksource_available.reset();
        self.clocksource_current.reset();
        for sample in clocksources {
            for clocksource in &sample.available {
                self.clocksource_available
                    .with_label_values(&[&sample.device, clocksource])
                    .set(1.0);
            }
            if !sample.current.is_empty() {
                self.clocksource_current
                    .with_label_values(&[&sample.device, &sample.current])
                    .set(1.0);
            }
        }
    }
}

static METRICS: OnceLock<TimeMetrics> = OnceLock::new();

fn metrics() -> &'static TimeMetrics {
    METRICS.get_or_init(TimeMetrics::new)
}

fn read_trimmed(path: &Path) -> Result<String, String> {
    fs::read_to_string(path)
        .map(|value| value.trim().to_string())
        .map_err(|error| format!("failed to read {}: {error}", path.display()))
}

fn collect_clocksources(root: &Path) -> Result<Vec<ClocksourceSample>, String> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("failed to list {}: {error}", root.display())),
    };
    let mut samples = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("failed to list {}: {error}", root.display()))?;
        let device = entry.file_name().to_string_lossy().into_owned();
        if !device.starts_with("clocksource") || !entry.path().is_dir() {
            continue;
        }
        let current = read_trimmed(&entry.path().join("current_clocksource"))?;
        let available = read_trimmed(&entry.path().join("available_clocksource"))?
            .split_whitespace()
            .map(str::to_string)
            .collect();
        samples.push(ClocksourceSample {
            device,
            current,
            available,
        });
    }
    samples.sort_by(|left, right| left.device.cmp(&right.device));
    Ok(samples)
}

pub fn update_metrics() -> CollectionReport {
    let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs_f64(),
        Err(error) => {
            if debug_enabled() {
                eprintln!("time: system clock predates Unix epoch: {error}");
            }
            return CollectionReport::error();
        }
    };
    match collect_clocksources(Path::new(CLOCKSOURCE_PATH)) {
        Ok(clocksources) => {
            metrics().apply(now, &clocksources);
            CollectionReport::success()
        }
        Err(error) => {
            if debug_enabled() {
                eprintln!("time: {error}");
            }
            metrics().seconds.set(now);
            metrics().clocksource_available.reset();
            metrics().clocksource_current.reset();
            CollectionReport::error()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::collect_clocksources;
    use std::fs;

    #[test]
    fn reads_linux_clocksource_shape() {
        let temp = tempfile::tempdir().unwrap();
        let device = temp.path().join("clocksource0");
        fs::create_dir(&device).unwrap();
        fs::write(device.join("current_clocksource"), "tsc\n").unwrap();
        fs::write(device.join("available_clocksource"), "tsc hpet acpi_pm\n").unwrap();
        let samples = collect_clocksources(temp.path()).unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].device, "clocksource0");
        assert_eq!(samples[0].current, "tsc");
        assert_eq!(samples[0].available, ["tsc", "hpet", "acpi_pm"]);
    }
}
