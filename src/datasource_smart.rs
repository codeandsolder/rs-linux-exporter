use crate::collection::CollectionReport;
use crate::config::AppConfig;
use crate::metric_support::{RegisterMetricResultExt, prometheus_u64};
use crate::runtime::debug_enabled;
use prometheus::GaugeVec;
use serde::Deserialize;
use serde_json::Value;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Deserialize)]
struct ScanOutput {
    #[serde(default)]
    devices: Vec<ScanDevice>,
}

#[derive(Debug, Deserialize)]
struct ScanDevice {
    name: String,
    #[serde(rename = "type")]
    device_type: String,
    #[serde(default)]
    protocol: String,
}

struct SmartMetrics {
    info: GaugeVec,
    passed: GaugeVec,
    temperature_celsius: GaugeVec,
    power_on_hours: GaugeVec,
    power_cycles: GaugeVec,
    percentage_used: GaugeVec,
    available_spare_ratio: GaugeVec,
    unsafe_shutdowns: GaugeVec,
    media_errors: GaugeVec,
    attribute_raw: GaugeVec,
    attribute_value: GaugeVec,
    attribute_worst: GaugeVec,
    attribute_threshold: GaugeVec,
    attribute_failed: GaugeVec,
}

impl SmartMetrics {
    fn new() -> Self {
        Self {
            info: prometheus::register_gauge_vec!(
                "smart_device_info",
                "SMART device identity.",
                &["device", "model", "serial", "protocol"]
            )
            .or_exit("smart_device_info"),
            passed: prometheus::register_gauge_vec!(
                "smart_status_passed",
                "Whether the device SMART health assessment passed.",
                &["device"]
            )
            .or_exit("smart_status_passed"),
            temperature_celsius: prometheus::register_gauge_vec!(
                "smart_temperature_celsius",
                "Current SMART-reported device temperature.",
                &["device"]
            )
            .or_exit("smart_temperature_celsius"),
            power_on_hours: prometheus::register_gauge_vec!(
                "smart_power_on_hours",
                "SMART-reported power-on hours.",
                &["device"]
            )
            .or_exit("smart_power_on_hours"),
            power_cycles: prometheus::register_gauge_vec!(
                "smart_power_cycles_total",
                "SMART-reported power-cycle count.",
                &["device"]
            )
            .or_exit("smart_power_cycles_total"),
            percentage_used: prometheus::register_gauge_vec!(
                "smart_percentage_used",
                "NVMe endurance percentage used.",
                &["device"]
            )
            .or_exit("smart_percentage_used"),
            available_spare_ratio: prometheus::register_gauge_vec!(
                "smart_available_spare_ratio",
                "NVMe available spare as a ratio from 0 to 1.",
                &["device"]
            )
            .or_exit("smart_available_spare_ratio"),
            unsafe_shutdowns: prometheus::register_gauge_vec!(
                "smart_unsafe_shutdowns_total",
                "NVMe unsafe shutdown count.",
                &["device"]
            )
            .or_exit("smart_unsafe_shutdowns_total"),
            media_errors: prometheus::register_gauge_vec!(
                "smart_media_errors_total",
                "NVMe media/data integrity error count.",
                &["device"]
            )
            .or_exit("smart_media_errors_total"),
            attribute_raw: prometheus::register_gauge_vec!(
                "smart_attribute_raw",
                "Raw ATA SMART attribute value.",
                &["device", "id", "name"]
            )
            .or_exit("smart_attribute_raw"),
            attribute_value: prometheus::register_gauge_vec!(
                "smart_attribute_value",
                "Normalized ATA SMART attribute value.",
                &["device", "id", "name"]
            )
            .or_exit("smart_attribute_value"),
            attribute_worst: prometheus::register_gauge_vec!(
                "smart_attribute_worst",
                "Worst normalized ATA SMART attribute value.",
                &["device", "id", "name"]
            )
            .or_exit("smart_attribute_worst"),
            attribute_threshold: prometheus::register_gauge_vec!(
                "smart_attribute_threshold",
                "ATA SMART attribute failure threshold.",
                &["device", "id", "name"]
            )
            .or_exit("smart_attribute_threshold"),
            attribute_failed: prometheus::register_gauge_vec!(
                "smart_attribute_failed",
                "Whether an ATA SMART attribute is currently or previously failed.",
                &["device", "id", "name"]
            )
            .or_exit("smart_attribute_failed"),
        }
    }

    fn reset(&self) {
        self.info.reset();
        self.passed.reset();
        self.temperature_celsius.reset();
        self.power_on_hours.reset();
        self.power_cycles.reset();
        self.percentage_used.reset();
        self.available_spare_ratio.reset();
        self.unsafe_shutdowns.reset();
        self.media_errors.reset();
        self.attribute_raw.reset();
        self.attribute_value.reset();
        self.attribute_worst.reset();
        self.attribute_threshold.reset();
        self.attribute_failed.reset();
    }
}

static METRICS: OnceLock<SmartMetrics> = OnceLock::new();
static LAST_REFRESH: Mutex<Option<Instant>> = Mutex::new(None);

fn metrics() -> &'static SmartMetrics {
    METRICS.get_or_init(SmartMetrics::new)
}

fn due(interval: Duration) -> bool {
    let Ok(mut last) = LAST_REFRESH.lock() else {
        return true;
    };
    if last.is_some_and(|instant| instant.elapsed() < interval) {
        return false;
    }
    *last = Some(Instant::now());
    true
}

fn run_smartctl(config: &AppConfig, args: &[&str], description: &str) -> Result<String, String> {
    let mut command = Command::new(&config.smartctl_binary);
    command.args(args);
    crate::subprocess::run_bounded_allow_failure(
        command,
        Duration::from_millis(config.smartctl_timeout_ms),
        4 * 1024 * 1024,
        description,
    )
}

fn number(value: &Value, path: &[&str]) -> Option<f64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_f64()
}

fn integer(value: &Value, path: &[&str]) -> Option<u64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_u64()
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn apply_ata_attributes(metrics: &SmartMetrics, device: &str, value: &Value) {
    let Some(attributes) = value
        .pointer("/ata_smart_attributes/table")
        .and_then(Value::as_array)
    else {
        return;
    };
    for attribute in attributes {
        let Some(id) = attribute.get("id").and_then(Value::as_u64) else {
            continue;
        };
        let name = text(attribute, "name");
        let id_text = id.to_string();
        let labels = [device, id_text.as_str(), name];
        if let Some(raw) = integer(attribute, &["raw", "value"]) {
            metrics
                .attribute_raw
                .with_label_values(&labels)
                .set(prometheus_u64(raw));
        }
        if let Some(current) = attribute.get("value").and_then(Value::as_f64) {
            metrics
                .attribute_value
                .with_label_values(&labels)
                .set(current);
        }
        if let Some(worst) = attribute.get("worst").and_then(Value::as_f64) {
            metrics
                .attribute_worst
                .with_label_values(&labels)
                .set(worst);
        }
        if let Some(threshold) = attribute.get("thresh").and_then(Value::as_f64) {
            metrics
                .attribute_threshold
                .with_label_values(&labels)
                .set(threshold);
        }
        let failed = !text(attribute, "when_failed").is_empty();
        metrics
            .attribute_failed
            .with_label_values(&labels)
            .set(f64::from(failed));
    }
}

fn apply_device(metrics: &SmartMetrics, scan: &ScanDevice, value: &Value) {
    let device = value
        .pointer("/device/name")
        .and_then(Value::as_str)
        .unwrap_or(&scan.name);
    let model = text(value, "model_name");
    let serial = text(value, "serial_number");
    let protocol = value
        .pointer("/device/protocol")
        .and_then(Value::as_str)
        .unwrap_or(&scan.protocol);
    metrics
        .info
        .with_label_values(&[device, model, serial, protocol])
        .set(1.0);

    if let Some(passed) = value
        .pointer("/smart_status/passed")
        .and_then(Value::as_bool)
    {
        metrics
            .passed
            .with_label_values(&[device])
            .set(f64::from(passed));
    }
    if let Some(temp) = number(value, &["temperature", "current"]) {
        metrics
            .temperature_celsius
            .with_label_values(&[device])
            .set(temp);
    }
    if let Some(hours) = number(value, &["power_on_time", "hours"]) {
        metrics
            .power_on_hours
            .with_label_values(&[device])
            .set(hours);
    }
    if let Some(cycles) = integer(value, &["power_cycle_count"]) {
        metrics
            .power_cycles
            .with_label_values(&[device])
            .set(prometheus_u64(cycles));
    }

    if let Some(nvme) = value.get("nvme_smart_health_information_log") {
        if let Some(percentage) = nvme.get("percentage_used").and_then(Value::as_f64) {
            metrics
                .percentage_used
                .with_label_values(&[device])
                .set(percentage);
        }
        if let Some(spare) = nvme.get("available_spare").and_then(Value::as_f64) {
            metrics
                .available_spare_ratio
                .with_label_values(&[device])
                .set(spare / 100.0);
        }
        if let Some(unsafe_shutdowns) = nvme.get("unsafe_shutdowns").and_then(Value::as_u64) {
            metrics
                .unsafe_shutdowns
                .with_label_values(&[device])
                .set(prometheus_u64(unsafe_shutdowns));
        }
        if let Some(media_errors) = nvme.get("media_errors").and_then(Value::as_u64) {
            metrics
                .media_errors
                .with_label_values(&[device])
                .set(prometheus_u64(media_errors));
        }
        if let Some(cycles) = nvme.get("power_cycles").and_then(Value::as_u64) {
            metrics
                .power_cycles
                .with_label_values(&[device])
                .set(prometheus_u64(cycles));
        }
        if let Some(hours) = nvme.get("power_on_hours").and_then(Value::as_u64) {
            metrics
                .power_on_hours
                .with_label_values(&[device])
                .set(prometheus_u64(hours));
        }
    }
    apply_ata_attributes(metrics, device, value);
}

pub fn update_metrics(config: &AppConfig) -> CollectionReport {
    if !due(Duration::from_secs(config.smart_interval_seconds)) {
        return CollectionReport::success();
    }
    let scan =
        match run_smartctl(config, &["--scan-open", "-j"], "smartctl scan").and_then(|output| {
            serde_json::from_str::<ScanOutput>(&output)
                .map_err(|error| format!("parse smartctl scan JSON: {error}"))
        }) {
            Ok(scan) => scan,
            Err(error) => {
                if debug_enabled() {
                    eprintln!("smart: {error}");
                }
                return CollectionReport::error();
            }
        };

    let metrics = metrics();
    metrics.reset();
    let mut report = CollectionReport::success();
    for device in scan.devices {
        let args = [
            "-a",
            "-j",
            "-d",
            device.device_type.as_str(),
            device.name.as_str(),
        ];
        match run_smartctl(config, &args, "smartctl device").and_then(|output| {
            serde_json::from_str::<Value>(&output)
                .map_err(|error| format!("parse smartctl device JSON: {error}"))
        }) {
            Ok(value) => apply_device(metrics, &device, &value),
            Err(error) => {
                if debug_enabled() {
                    eprintln!("smart {}: {error}", device.name);
                }
                report.merge(CollectionReport::error());
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scan_json() {
        let parsed: ScanOutput = serde_json::from_str(
            r#"{"devices":[{"name":"/dev/sda","type":"sat","protocol":"ATA"}]}"#,
        )
        .unwrap();
        assert_eq!(parsed.devices.len(), 1);
        assert_eq!(parsed.devices[0].device_type, "sat");
    }

    #[test]
    fn nested_numeric_lookup_is_tolerant() {
        let value: Value = serde_json::from_str(r#"{"temperature":{"current":42}}"#).unwrap();
        assert_eq!(number(&value, &["temperature", "current"]), Some(42.0));
        assert_eq!(number(&value, &["missing"]), None);
    }
}
