use crate::collection::CollectionReport;
use crate::metric_support::{RegisterMetricResultExt, prometheus_u64};
use crate::runtime::debug_enabled;
use prometheus::GaugeVec;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const WATCHDOG_PATH: &str = "/sys/class/watchdog";

#[derive(Debug, Default, PartialEq, Eq)]
struct WatchdogSample {
    name: String,
    bootstatus: Option<u64>,
    fw_version: Option<u64>,
    nowayout: Option<u64>,
    timeleft: Option<u64>,
    timeout: Option<u64>,
    pretimeout: Option<u64>,
    access_cs0: Option<u64>,
    options: String,
    identity: String,
    state: String,
    status: String,
    pretimeout_governor: String,
}

struct WatchdogMetrics {
    bootstatus: GaugeVec,
    fw_version: GaugeVec,
    nowayout: GaugeVec,
    timeleft_seconds: GaugeVec,
    timeout_seconds: GaugeVec,
    pretimeout_seconds: GaugeVec,
    access_cs0: GaugeVec,
    info: GaugeVec,
}

impl WatchdogMetrics {
    fn new() -> Self {
        Self {
            bootstatus: gauge("watchdog_bootstatus", "Watchdog boot status."),
            fw_version: gauge("watchdog_fw_version", "Watchdog firmware version."),
            nowayout: gauge("watchdog_nowayout", "Watchdog nowayout setting."),
            timeleft_seconds: gauge(
                "watchdog_timeleft_seconds",
                "Watchdog time left in seconds.",
            ),
            timeout_seconds: gauge("watchdog_timeout_seconds", "Watchdog timeout in seconds."),
            pretimeout_seconds: gauge(
                "watchdog_pretimeout_seconds",
                "Watchdog pretimeout in seconds.",
            ),
            access_cs0: gauge("watchdog_access_cs0", "Watchdog access_cs0 value."),
            info: prometheus::register_gauge_vec!(
                "watchdog_info",
                "Watchdog identity and status information.",
                &[
                    "name",
                    "options",
                    "identity",
                    "state",
                    "status",
                    "pretimeout_governor"
                ]
            )
            .or_exit("watchdog_info"),
        }
    }

    fn reset(&self) {
        self.bootstatus.reset();
        self.fw_version.reset();
        self.nowayout.reset();
        self.timeleft_seconds.reset();
        self.timeout_seconds.reset();
        self.pretimeout_seconds.reset();
        self.access_cs0.reset();
        self.info.reset();
    }

    fn apply(&self, samples: &[WatchdogSample]) {
        self.reset();
        for sample in samples {
            set_optional(&self.bootstatus, &sample.name, sample.bootstatus);
            set_optional(&self.fw_version, &sample.name, sample.fw_version);
            set_optional(&self.nowayout, &sample.name, sample.nowayout);
            set_optional(&self.timeleft_seconds, &sample.name, sample.timeleft);
            set_optional(&self.timeout_seconds, &sample.name, sample.timeout);
            set_optional(&self.pretimeout_seconds, &sample.name, sample.pretimeout);
            set_optional(&self.access_cs0, &sample.name, sample.access_cs0);
            self.info
                .with_label_values(&[
                    &sample.name,
                    &sample.options,
                    &sample.identity,
                    &sample.state,
                    &sample.status,
                    &sample.pretimeout_governor,
                ])
                .set(1.0);
        }
    }
}

fn gauge(name: &'static str, help: &'static str) -> GaugeVec {
    prometheus::register_gauge_vec!(name, help, &["name"]).or_exit(name)
}

fn set_optional(metric: &GaugeVec, name: &str, value: Option<u64>) {
    if let Some(value) = value {
        metric.with_label_values(&[name]).set(prometheus_u64(value));
    }
}

static METRICS: OnceLock<WatchdogMetrics> = OnceLock::new();

fn metrics() -> &'static WatchdogMetrics {
    METRICS.get_or_init(WatchdogMetrics::new)
}

fn read_text(path: &Path) -> Result<String, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).trim().to_string()),
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::PermissionDenied
            ) =>
        {
            Ok(String::new())
        }
        Err(error) => Err(format!("failed to read {}: {error}", path.display())),
    }
}

fn read_u64(path: &Path) -> Result<Option<u64>, String> {
    let text = read_text(path)?;
    if text.is_empty() {
        return Ok(None);
    }
    text.parse::<u64>().map(Some).map_err(|error| {
        format!(
            "invalid numeric watchdog value {text:?} in {}: {error}",
            path.display()
        )
    })
}

fn read_watchdog(path: &Path, name: String) -> Result<WatchdogSample, String> {
    Ok(WatchdogSample {
        name,
        bootstatus: read_u64(&path.join("bootstatus"))?,
        fw_version: read_u64(&path.join("fw_version"))?,
        nowayout: read_u64(&path.join("nowayout"))?,
        timeleft: read_u64(&path.join("timeleft"))?,
        timeout: read_u64(&path.join("timeout"))?,
        pretimeout: read_u64(&path.join("pretimeout"))?,
        access_cs0: read_u64(&path.join("access_cs0"))?,
        options: read_text(&path.join("options"))?,
        identity: read_text(&path.join("identity"))?,
        state: read_text(&path.join("state"))?,
        status: read_text(&path.join("status"))?,
        pretimeout_governor: read_text(&path.join("pretimeout_governor"))?,
    })
}

fn watchdog_paths(root: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("failed to list {}: {error}", root.display())),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("failed to list {}: {error}", root.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("watchdog") {
            paths.push((name, entry.path()));
        }
    }
    paths.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(paths)
}

fn collect_watchdogs(root: &Path) -> Result<Vec<WatchdogSample>, String> {
    watchdog_paths(root)?
        .into_iter()
        .map(|(name, path)| read_watchdog(&path, name))
        .collect()
}

fn update_metrics_from_path(root: &Path) -> CollectionReport {
    match collect_watchdogs(root) {
        Ok(samples) => {
            metrics().apply(&samples);
            CollectionReport::success()
        }
        Err(error) => {
            if debug_enabled() {
                eprintln!("watchdog: {error}");
            }
            metrics().reset();
            CollectionReport::error()
        }
    }
}

pub fn update_metrics() -> CollectionReport {
    update_metrics_from_path(Path::new(WATCHDOG_PATH))
}

#[cfg(test)]
mod tests {
    use super::collect_watchdogs;
    use std::fs;

    #[test]
    fn reads_watchdog_sysfs_shape() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("watchdog0");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("timeout"), "60\n").unwrap();
        fs::write(root.join("identity"), "Example watchdog\n").unwrap();
        fs::write(root.join("options"), "0x8180\n").unwrap();
        let samples = collect_watchdogs(temp.path()).unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].name, "watchdog0");
        assert_eq!(samples[0].timeout, Some(60));
        assert_eq!(samples[0].identity, "Example watchdog");
    }
}
