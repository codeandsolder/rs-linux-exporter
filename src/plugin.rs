use prometheus::proto::{Counter, Gauge, LabelPair, Metric, MetricFamily, MetricType, Untyped};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const PROTOCOL_VERSION: u32 = 1;
const MAX_MESSAGE_BYTES: usize = 1 << 20;
const MAX_PLUGINS: usize = 64;
const MAX_METRICS_PER_PLUGIN: usize = 256;
const MAX_SAMPLES_PER_METRIC: usize = 4_096;
const MAX_LABELS_PER_SAMPLE: usize = 16;
const MAX_TTL_SECONDS: u64 = 3_600;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PluginMetricType {
    Counter,
    Gauge,
    Untyped,
}

#[derive(Clone, Debug, Deserialize)]
struct PluginSample {
    #[serde(default)]
    labels: BTreeMap<String, String>,
    value: f64,
}

#[derive(Clone, Debug, Deserialize)]
struct PluginMetric {
    name: String,
    #[serde(default)]
    help: String,
    #[serde(rename = "type")]
    metric_type: PluginMetricType,
    samples: Vec<PluginSample>,
}

#[derive(Clone, Debug, Deserialize)]
struct PluginSnapshot {
    version: u32,
    plugin: String,
    ttl_seconds: u64,
    metrics: Vec<PluginMetric>,
}

#[derive(Serialize)]
struct Ack<'a> {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

#[derive(Clone)]
struct StoredSnapshot {
    snapshot: PluginSnapshot,
    received: Instant,
}

static SNAPSHOTS: OnceLock<Mutex<HashMap<String, StoredSnapshot>>> = OnceLock::new();

fn snapshots() -> &'static Mutex<HashMap<String, StoredSnapshot>> {
    SNAPSHOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn valid_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn validate_metric(metric: &PluginMetric) -> Result<(), String> {
    if !valid_name(&metric.name) {
        return Err(format!("invalid metric name {:?}", metric.name));
    }
    if metric.help.contains('\n') || metric.help.contains('\r') {
        return Err(format!("metric {:?} help contains a newline", metric.name));
    }
    if metric.samples.len() > MAX_SAMPLES_PER_METRIC {
        return Err(format!(
            "metric {:?} has too many samples ({})",
            metric.name,
            metric.samples.len()
        ));
    }
    let mut label_names: Option<Vec<&str>> = None;
    let mut identities = HashSet::new();
    for sample in &metric.samples {
        if sample.labels.len() > MAX_LABELS_PER_SAMPLE {
            return Err(format!(
                "metric {:?} sample has too many labels",
                metric.name
            ));
        }
        if !sample.value.is_finite() {
            return Err(format!(
                "metric {:?} sample value is not finite",
                metric.name
            ));
        }
        if metric.metric_type == PluginMetricType::Counter && sample.value < 0.0 {
            return Err(format!("counter {:?} has a negative sample", metric.name));
        }
        for label in sample.labels.keys() {
            if !valid_name(label) || label.starts_with("__") {
                return Err(format!(
                    "metric {:?} has invalid label {label:?}",
                    metric.name
                ));
            }
        }
        let current_names: Vec<_> = sample.labels.keys().map(String::as_str).collect();
        if label_names
            .as_ref()
            .is_some_and(|names| *names != current_names)
        {
            return Err(format!(
                "metric {:?} samples do not use one stable label schema",
                metric.name
            ));
        }
        label_names.get_or_insert(current_names);
        let identity: Vec<_> = sample
            .labels
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        if !identities.insert(identity) {
            return Err(format!(
                "metric {:?} has a duplicate label set",
                metric.name
            ));
        }
    }
    Ok(())
}

fn validate_snapshot(snapshot: &PluginSnapshot) -> Result<(), String> {
    if snapshot.version != PROTOCOL_VERSION {
        return Err(format!(
            "unsupported plugin protocol version {}, expected {PROTOCOL_VERSION}",
            snapshot.version
        ));
    }
    if !valid_name(&snapshot.plugin) {
        return Err(format!("invalid plugin name {:?}", snapshot.plugin));
    }
    if snapshot.ttl_seconds == 0 || snapshot.ttl_seconds > MAX_TTL_SECONDS {
        return Err(format!(
            "ttl_seconds must be between 1 and {MAX_TTL_SECONDS}"
        ));
    }
    if snapshot.metrics.len() > MAX_METRICS_PER_PLUGIN {
        return Err(format!(
            "plugin {:?} has too many metrics ({})",
            snapshot.plugin,
            snapshot.metrics.len()
        ));
    }
    let mut names = HashSet::new();
    for metric in &snapshot.metrics {
        if !names.insert(metric.name.as_str()) {
            return Err(format!("duplicate metric name {:?}", metric.name));
        }
        validate_metric(metric)?;
    }
    Ok(())
}

fn store_snapshot(snapshot: PluginSnapshot) -> Result<(), String> {
    validate_snapshot(&snapshot)?;
    let mut stored = snapshots()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !stored.contains_key(&snapshot.plugin) && stored.len() >= MAX_PLUGINS {
        return Err(format!("plugin limit of {MAX_PLUGINS} reached"));
    }
    stored.insert(
        snapshot.plugin.clone(),
        StoredSnapshot {
            snapshot,
            received: Instant::now(),
        },
    );
    drop(stored);
    Ok(())
}

fn label_pair(name: &str, value: &str) -> LabelPair {
    let mut pair = LabelPair::new();
    pair.set_name(name.to_string());
    pair.set_value(value.to_string());
    pair
}

fn proto_metric(sample: &PluginSample, metric_type: PluginMetricType) -> Metric {
    let mut metric = Metric::new();
    metric.label = sample
        .labels
        .iter()
        .map(|(name, value)| label_pair(name, value))
        .collect();
    match metric_type {
        PluginMetricType::Counter => {
            let mut counter = Counter::new();
            counter.set_value(sample.value);
            metric.counter = Some(counter).into();
        }
        PluginMetricType::Gauge => {
            let mut gauge = Gauge::new();
            gauge.set_value(sample.value);
            metric.gauge = Some(gauge).into();
        }
        PluginMetricType::Untyped => {
            let mut untyped = Untyped::new();
            untyped.set_value(sample.value);
            metric.untyped = Some(untyped).into();
        }
    }
    metric
}

fn metric_family(plugin: &str, metric: &PluginMetric) -> MetricFamily {
    let mut family = MetricFamily::new();
    family.set_name(format!("{plugin}_{}", metric.name));
    family.set_help(metric.help.clone());
    family.set_field_type(match metric.metric_type {
        PluginMetricType::Counter => MetricType::COUNTER,
        PluginMetricType::Gauge => MetricType::GAUGE,
        PluginMetricType::Untyped => MetricType::UNTYPED,
    });
    family.metric = metric
        .samples
        .iter()
        .map(|sample| proto_metric(sample, metric.metric_type))
        .collect();
    family
}

fn health_family(name: &str, help: &str, samples: Vec<(&str, f64)>) -> MetricFamily {
    let mut family = MetricFamily::new();
    family.set_name(name.to_string());
    family.set_help(help.to_string());
    family.set_field_type(MetricType::GAUGE);
    family.metric = samples
        .into_iter()
        .map(|(plugin, value)| {
            let mut metric = Metric::new();
            metric.label.push(label_pair("plugin", plugin));
            let mut gauge = Gauge::new();
            gauge.set_value(value);
            metric.gauge = Some(gauge).into();
            metric
        })
        .collect();
    family
}

pub fn metric_families() -> Vec<MetricFamily> {
    let stored: Vec<_> = {
        let guard = snapshots()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.values().cloned().collect()
    };
    let mut families = Vec::new();
    let mut ages = Vec::new();
    let mut valid = Vec::new();
    for stored in &stored {
        let plugin = stored.snapshot.plugin.as_str();
        let age = stored.received.elapsed().as_secs_f64();
        let is_valid =
            stored.received.elapsed() <= Duration::from_secs(stored.snapshot.ttl_seconds);
        ages.push((plugin, age));
        valid.push((plugin, if is_valid { 1.0 } else { 0.0 }));
        if is_valid {
            families.extend(
                stored
                    .snapshot
                    .metrics
                    .iter()
                    .map(|metric| metric_family(plugin, metric)),
            );
        }
    }
    if !ages.is_empty() {
        families.push(health_family(
            "plugin_snapshot_age_seconds",
            "Age of the most recently received plugin snapshot",
            ages,
        ));
        families.push(health_family(
            "plugin_snapshot_valid",
            "Whether the cached plugin snapshot is still within its declared TTL",
            valid,
        ));
    }
    families
}

fn write_ack(stream: &mut UnixStream, ack: &Ack<'_>) -> Result<(), String> {
    let body = serde_json::to_vec(ack).map_err(|error| format!("serialize plugin ACK: {error}"))?;
    let len = u32::try_from(body.len()).map_err(|_| "plugin ACK too large".to_string())?;
    stream
        .write_all(&len.to_be_bytes())
        .and_then(|()| stream.write_all(&body))
        .map_err(|error| format!("write plugin ACK: {error}"))
}

fn handle_connection(mut stream: UnixStream) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| format!("set plugin socket read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| format!("set plugin socket write timeout: {error}"))?;
    let mut header = [0_u8; 4];
    stream
        .read_exact(&mut header)
        .map_err(|error| format!("read plugin message length: {error}"))?;
    let length = usize::try_from(u32::from_be_bytes(header))
        .map_err(|_| "plugin message length does not fit usize".to_string())?;
    if length == 0 || length > MAX_MESSAGE_BYTES {
        let error = format!("plugin message length {length} is outside 1..={MAX_MESSAGE_BYTES}");
        let _ = write_ack(
            &mut stream,
            &Ack {
                ok: false,
                error: Some(&error),
            },
        );
        return Err(error);
    }
    let mut body = vec![0_u8; length];
    stream
        .read_exact(&mut body)
        .map_err(|error| format!("read plugin message: {error}"))?;
    let result = serde_json::from_slice::<PluginSnapshot>(&body)
        .map_err(|error| format!("parse plugin snapshot JSON: {error}"))
        .and_then(store_snapshot);
    match result {
        Ok(()) => write_ack(
            &mut stream,
            &Ack {
                ok: true,
                error: None,
            },
        ),
        Err(error) => {
            let _ = write_ack(
                &mut stream,
                &Ack {
                    ok: false,
                    error: Some(&error),
                },
            );
            Err(error)
        }
    }
}

fn prepare_socket(path: &Path) -> Result<UnixListener, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create plugin socket directory {}: {error}",
                parent.display()
            )
        })?;
    }
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            return Err(format!(
                "plugin socket {} is already accepting connections",
                path.display()
            ));
        }
        fs::remove_file(path)
            .map_err(|error| format!("remove stale plugin socket {}: {error}", path.display()))?;
    }
    let listener = UnixListener::bind(path)
        .map_err(|error| format!("bind plugin socket {}: {error}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o660))
        .map_err(|error| format!("chmod plugin socket {}: {error}", path.display()))?;
    Ok(listener)
}

pub fn start_server(path: &Path) -> Result<(), String> {
    let listener = prepare_socket(path)?;
    let path = path.to_path_buf();
    std::thread::Builder::new()
        .name("rsle-plugin-socket".to_string())
        .spawn(move || {
            for connection in listener.incoming() {
                match connection {
                    Ok(stream) => {
                        if let Err(error) = handle_connection(stream) {
                            eprintln!("plugin socket {}: {error}", path.display());
                        }
                    }
                    Err(error) => {
                        eprintln!("plugin socket {} accept failed: {error}", path.display());
                    }
                }
            }
        })
        .map_err(|error| format!("start plugin socket thread: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> PluginSnapshot {
        PluginSnapshot {
            version: 1,
            plugin: "searxrs2".to_string(),
            ttl_seconds: 30,
            metrics: vec![PluginMetric {
                name: "requests_total".to_string(),
                help: "Requests".to_string(),
                metric_type: PluginMetricType::Counter,
                samples: vec![PluginSample {
                    labels: BTreeMap::from([("engine".to_string(), "brave".to_string())]),
                    value: 42.0,
                }],
            }],
        }
    }

    #[test]
    fn validates_and_namespaces_snapshots() {
        let snapshot = snapshot();
        validate_snapshot(&snapshot).unwrap();
        let family = metric_family(&snapshot.plugin, &snapshot.metrics[0]);
        assert_eq!(family.name(), "searxrs2_requests_total");
        assert_eq!(family.get_metric()[0].get_counter().value(), 42.0);
    }

    #[test]
    fn rejects_duplicate_metric_names_and_negative_counters() {
        let mut snapshot = snapshot();
        snapshot.metrics.push(snapshot.metrics[0].clone());
        assert!(validate_snapshot(&snapshot).is_err());
        snapshot.metrics.pop();
        snapshot.metrics[0].samples[0].value = -1.0;
        assert!(validate_snapshot(&snapshot).is_err());
    }

    #[test]
    fn rejects_label_schema_drift() {
        let mut snapshot = snapshot();
        snapshot.metrics[0].samples.push(PluginSample {
            labels: BTreeMap::from([("provider".to_string(), "qwant".to_string())]),
            value: 1.0,
        });
        assert!(validate_snapshot(&snapshot).is_err());
    }
}
