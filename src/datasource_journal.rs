use crate::collection::CollectionReport;
use crate::config::AppConfig;
use crate::metric_support::{RegisterMetricResultExt, prometheus_i64};
use crate::runtime::debug_enabled;
use prometheus::{CounterVec, GaugeVec};
use serde_json::Value;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

struct JournalMetrics {
    events_total: CounterVec,
    critical_total: CounterVec,
    last_critical_timestamp: GaugeVec,
    remote_events_total: CounterVec,
    remote_last_event_timestamp: GaugeVec,
}

impl JournalMetrics {
    fn new() -> Self {
        Self {
            events_total: prometheus::register_counter_vec!(
                "journal_events_total",
                "Incremental journal events collected at warning priority or higher.",
                &["priority", "transport"]
            )
            .or_exit("journal_events_total"),
            critical_total: prometheus::register_counter_vec!(
                "journal_critical_events_total",
                "Classified critical operating-system events observed in the journal.",
                &["kind"]
            )
            .or_exit("journal_critical_events_total"),
            last_critical_timestamp: prometheus::register_gauge_vec!(
                "journal_last_critical_event_timestamp_seconds",
                "Unix timestamp of the most recently observed classified critical event.",
                &["kind"]
            )
            .or_exit("journal_last_critical_event_timestamp_seconds"),
            remote_events_total: prometheus::register_counter_vec!(
                "journal_remote_syslog_events_total",
                "Remote syslog events ingested by udp514-journal.",
                &["sender", "priority"]
            )
            .or_exit("journal_remote_syslog_events_total"),
            remote_last_event_timestamp: prometheus::register_gauge_vec!(
                "journal_remote_syslog_last_event_timestamp_seconds",
                "Unix timestamp of the most recent remote syslog event by sender.",
                &["sender"]
            )
            .or_exit("journal_remote_syslog_last_event_timestamp_seconds"),
        }
    }
}

#[derive(Default)]
struct JournalState {
    cursor: Option<String>,
    remote_cursor: Option<String>,
    last_refresh: Option<Instant>,
}

static METRICS: OnceLock<JournalMetrics> = OnceLock::new();
static STATE: Mutex<JournalState> = Mutex::new(JournalState {
    cursor: None,
    remote_cursor: None,
    last_refresh: None,
});

fn metrics() -> &'static JournalMetrics {
    METRICS.get_or_init(JournalMetrics::new)
}

fn run_journalctl(config: &AppConfig, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new(&config.journalctl_binary);
    command.args(args);
    crate::subprocess::run_bounded(
        command,
        Duration::from_millis(config.journal_timeout_ms),
        4 * 1024 * 1024,
        "journalctl",
    )
}

fn extract_cursor(output: &str) -> Option<String> {
    output.lines().rev().find_map(|line| {
        line.trim()
            .strip_prefix("-- cursor: ")
            .map(str::trim)
            .filter(|cursor| !cursor.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn classify(message: &str) -> Option<&'static str> {
    let message = message.to_ascii_lowercase();
    let patterns: &[(&str, &[&str])] = &[
        ("oom", &["out of memory", "oom-kill", "killed process"]),
        (
            "io_error",
            &[
                "i/o error",
                "buffer i/o error",
                "blk_update_request",
                "end_request: i/o",
            ],
        ),
        (
            "filesystem_error",
            &[
                "filesystem error",
                "ext4-fs error",
                "xfs.*corruption",
                "btrfs error",
                "zfs.*error",
            ],
        ),
        (
            "machine_check",
            &["machine check", "hardware error", "mce:"],
        ),
        ("kernel_panic", &["kernel panic", "kernel oops", "oops:"]),
        ("segfault", &["segfault at", "general protection fault"]),
        ("watchdog", &["watchdog", "soft lockup", "hard lockup"]),
        ("hung_task", &["blocked for more than", "hung task"]),
        ("thermal", &["critical temperature", "thermal thrott"]),
        (
            "service_failure",
            &["failed with result", "failed to start"],
        ),
    ];
    patterns.iter().find_map(|(kind, needles)| {
        needles
            .iter()
            .any(|needle| {
                if needle.contains(".*") {
                    let mut parts = needle.split(".*");
                    let first = parts.next().unwrap_or_default();
                    let second = parts.next().unwrap_or_default();
                    message.contains(first) && message.contains(second)
                } else {
                    message.contains(needle)
                }
            })
            .then_some(*kind)
    })
}

fn event_timestamp_seconds(value: &Value) -> Option<f64> {
    value
        .get("__REALTIME_TIMESTAMP")
        .and_then(Value::as_str)
        .and_then(|raw| raw.parse::<i64>().ok())
        .map(|micros| prometheus_i64(micros) / 1_000_000.0)
}

fn apply_remote_event(value: &Value) {
    let sender = value
        .get("SYSLOG_IDENTIFIER")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let priority = value
        .get("PRIORITY")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    metrics()
        .remote_events_total
        .with_label_values(&[sender, priority])
        .inc();
    if let Some(timestamp) = event_timestamp_seconds(value) {
        metrics()
            .remote_last_event_timestamp
            .with_label_values(&[sender])
            .set(timestamp);
    }
}

fn apply_event(value: &Value) {
    let priority = value
        .get("PRIORITY")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let transport = value
        .get("_TRANSPORT")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    metrics()
        .events_total
        .with_label_values(&[priority, transport])
        .inc();
    let message = value
        .get("MESSAGE")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(kind) = classify(message) else {
        return;
    };
    metrics().critical_total.with_label_values(&[kind]).inc();
    if let Some(timestamp) = event_timestamp_seconds(value) {
        metrics()
            .last_critical_timestamp
            .with_label_values(&[kind])
            .set(timestamp);
    }
}

fn initialize_cursor(config: &AppConfig) -> Result<String, String> {
    let output = run_journalctl(config, &["-n", "0", "--show-cursor", "--no-pager"])?;
    extract_cursor(&output).ok_or_else(|| "journalctl returned no cursor".to_string())
}

pub fn update_metrics(config: &AppConfig) -> CollectionReport {
    let interval = Duration::from_secs(config.journal_interval_seconds);
    let Ok(mut state) = STATE.lock() else {
        return CollectionReport::error();
    };
    if state
        .last_refresh
        .is_some_and(|instant| instant.elapsed() < interval)
    {
        return CollectionReport::success();
    }
    state.last_refresh = Some(Instant::now());

    if state.cursor.is_none() || state.remote_cursor.is_none() {
        match initialize_cursor(config) {
            Ok(cursor) => {
                state.cursor = Some(cursor.clone());
                state.remote_cursor = Some(cursor);
                return CollectionReport::success();
            }
            Err(error) => {
                if debug_enabled() {
                    eprintln!("journal: {error}");
                }
                return CollectionReport::error();
            }
        }
    }

    let cursor = state.cursor.as_deref().unwrap_or_default();
    let cursor_arg = format!("--after-cursor={cursor}");
    let output = match run_journalctl(
        config,
        &[
            &cursor_arg,
            "-p",
            "0..4",
            "-o",
            "json",
            "--show-cursor",
            "--no-pager",
        ],
    ) {
        Ok(output) => output,
        Err(error) => {
            if debug_enabled() {
                eprintln!("journal: incremental read failed: {error}");
            }
            state.cursor = initialize_cursor(config).ok();
            return CollectionReport::error();
        }
    };

    for line in output
        .lines()
        .filter(|line| line.trim_start().starts_with('{'))
    {
        match serde_json::from_str::<Value>(line) {
            Ok(value) => apply_event(&value),
            Err(error) if debug_enabled() => eprintln!("journal: invalid JSON event: {error}"),
            Err(_) => {}
        }
    }
    if let Some(cursor) = extract_cursor(&output) {
        state.cursor = Some(cursor);
    }

    let remote_cursor = state.remote_cursor.as_deref().unwrap_or_default();
    let remote_cursor_arg = format!("--after-cursor={remote_cursor}");
    match run_journalctl(
        config,
        &[
            &remote_cursor_arg,
            "-o",
            "json",
            "--show-cursor",
            "--no-pager",
            "_COMM=udp514-journal",
        ],
    ) {
        Ok(remote_output) => {
            for line in remote_output
                .lines()
                .filter(|line| line.trim_start().starts_with('{'))
            {
                match serde_json::from_str::<Value>(line) {
                    Ok(value) => apply_remote_event(&value),
                    Err(error) if debug_enabled() => {
                        eprintln!("journal: invalid remote syslog JSON event: {error}");
                    }
                    Err(_) => {}
                }
            }
            if let Some(cursor) = extract_cursor(&remote_output) {
                state.remote_cursor = Some(cursor);
            }
        }
        Err(error) => {
            if debug_enabled() {
                eprintln!("journal: remote syslog read failed: {error}");
            }
            state.remote_cursor = initialize_cursor(config).ok();
            return CollectionReport::error();
        }
    }
    CollectionReport::success()
}

#[cfg(test)]
mod tests {
    use super::{classify, event_timestamp_seconds, extract_cursor};

    #[test]
    fn classifies_high_value_failure_messages() {
        assert_eq!(
            classify("kernel: Out of memory: Killed process 42"),
            Some("oom")
        );
        assert_eq!(
            classify("EXT4-fs error (device sda2)"),
            Some("filesystem_error")
        );
        assert_eq!(classify("watchdog: BUG: soft lockup"), Some("watchdog"));
        assert_eq!(classify("ordinary informational message"), None);
    }

    #[test]
    fn extracts_last_cursor() {
        assert_eq!(
            extract_cursor("{}\n-- cursor: s=abc\n"),
            Some("s=abc".to_string())
        );
    }

    #[test]
    fn converts_journal_realtime_timestamp() {
        let value = serde_json::json!({"__REALTIME_TIMESTAMP": "1234567"});
        assert_eq!(event_timestamp_seconds(&value), Some(1.234567));
    }
}
