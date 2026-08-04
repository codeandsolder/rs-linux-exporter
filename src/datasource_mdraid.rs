use prometheus::GaugeVec;
use std::fs;
use std::sync::OnceLock;

const MDSTAT_PATH: &str = "/proc/mdstat";

struct MdraidMetrics {
    array_state: GaugeVec,
    array_disks: GaugeVec,
    array_degraded: GaugeVec,
    array_sync_progress: GaugeVec,
}

impl MdraidMetrics {
    fn new() -> Self {
        Self {
            array_state: prometheus::register_gauge_vec!(
                "mdraid_array_state",
                "MD RAID array state (1 for current state label)",
                &["array", "state", "level"]
            )
            .expect("register mdraid_array_state"),
            array_disks: prometheus::register_gauge_vec!(
                "mdraid_array_disks",
                "MD RAID array disk counts by role",
                &["array", "role"]
            )
            .expect("register mdraid_array_disks"),
            array_degraded: prometheus::register_gauge_vec!(
                "mdraid_array_degraded",
                "MD RAID array degraded state (1 if degraded)",
                &["array"]
            )
            .expect("register mdraid_array_degraded"),
            array_sync_progress: prometheus::register_gauge_vec!(
                "mdraid_array_sync_progress",
                "MD RAID array sync action progress (0-1)",
                &["array", "action"]
            )
            .expect("register mdraid_array_sync_progress"),
        }
    }
}

static MDRAID_METRICS: OnceLock<MdraidMetrics> = OnceLock::new();

fn metrics() -> &'static MdraidMetrics {
    MDRAID_METRICS.get_or_init(MdraidMetrics::new)
}

fn parse_level(tokens: &[&str]) -> String {
    for token in tokens {
        if token.starts_with("raid")
            || *token == "linear"
            || *token == "multipath"
            || *token == "faulty"
        {
            return (*token).to_string();
        }
    }
    "unknown".to_string()
}

fn parse_counts_token(token: &str) -> Option<(u64, u64)> {
    if !(token.starts_with('[') && token.ends_with(']')) {
        return None;
    }

    let inner = &token[1..token.len() - 1];
    let (left, right) = inner.split_once('/')?;
    let total = left.parse::<u64>().ok()?;
    let active = right.parse::<u64>().ok()?;
    Some((total, active))
}

fn parse_working_token(token: &str) -> Option<(u64, u64)> {
    if !(token.starts_with('[') && token.ends_with(']')) {
        return None;
    }

    let inner = &token[1..token.len() - 1];
    if !inner.chars().all(|c| c == 'U' || c == '_') {
        return None;
    }

    let working = inner.chars().filter(|c| *c == 'U').count() as u64;
    let total = inner.len() as u64;
    Some((total, working))
}

fn parse_sync_progress(line: &str) -> Option<(String, f64)> {
    let actions = ["resync", "recovery", "reshape", "check"];
    let action = actions.iter().find(|a| line.contains(*a))?;

    let percent_token = line.split_whitespace().find(|token| token.ends_with('%'))?;

    let raw = percent_token.trim_end_matches('%');
    let value = raw.parse::<f64>().ok()?;
    Some(((*action).to_string(), value / 100.0))
}

pub fn update_metrics() {
    let contents = match fs::read_to_string(MDSTAT_PATH) {
        Ok(contents) => contents,
        Err(_) => return,
    };

    let metrics = metrics();

    // Arrays, their states and their sync actions all come and go. Drop the
    // previous scrape's series so a stale array or a state the array has since
    // left does not keep reporting 1.
    metrics.array_state.reset();
    metrics.array_disks.reset();
    metrics.array_degraded.reset();
    metrics.array_sync_progress.reset();

    let mut lines = contents.lines().peekable();

    while let Some(line) = lines.next() {
        if !line.starts_with("md") {
            continue;
        }

        let mut parts = line.split_whitespace();
        let name = match parts.next() {
            Some(name) => name.to_string(),
            None => continue,
        };
        let colon = parts.next().unwrap_or("");
        if colon != ":" {
            continue;
        }

        let state = parts.next().unwrap_or("unknown").to_string();
        let remainder: Vec<&str> = parts.collect();
        let level = parse_level(&remainder);

        let mut total: Option<u64> = None;
        let mut active: Option<u64> = None;
        let mut working: Option<u64> = None;
        let mut sync_action: Option<String> = None;
        let mut sync_progress: Option<f64> = None;

        while let Some(next_line) = lines.peek() {
            if next_line.starts_with("md") {
                break;
            }
            let detail = lines.next().unwrap_or_default();
            if detail.trim().is_empty() {
                continue;
            }

            for token in detail.split_whitespace() {
                if active.is_none() {
                    if let Some((t, a)) = parse_counts_token(token) {
                        total = Some(t);
                        active = Some(a);
                        continue;
                    }
                }

                if working.is_none() {
                    if let Some((t, w)) = parse_working_token(token) {
                        working = Some(w);
                        if total.is_none() {
                            total = Some(t);
                        }
                    }
                }
            }

            if sync_action.is_none() {
                if let Some((action, progress)) = parse_sync_progress(detail) {
                    sync_action = Some(action);
                    sync_progress = Some(progress);
                }
            }
        }

        metrics
            .array_state
            .with_label_values(&[&name, &state, &level])
            .set(1.0);

        if let Some(total) = total {
            let role = "total".to_string();
            metrics
                .array_disks
                .with_label_values(&[&name, &role])
                .set(total as f64);
        }
        if let Some(active) = active {
            let role = "active".to_string();
            metrics
                .array_disks
                .with_label_values(&[&name, &role])
                .set(active as f64);
        }
        if let Some(working) = working {
            let role = "working".to_string();
            metrics
                .array_disks
                .with_label_values(&[&name, &role])
                .set(working as f64);
        }

        let degraded = match (total, active.or(working)) {
            (Some(total), Some(active)) => (active < total) as i32,
            _ => 0,
        };
        metrics
            .array_degraded
            .with_label_values(&[&name])
            .set(degraded as f64);

        if let (Some(action), Some(progress)) = (sync_action, sync_progress) {
            metrics
                .array_sync_progress
                .with_label_values(&[&name, &action])
                .set(progress);
        }
    }
}
