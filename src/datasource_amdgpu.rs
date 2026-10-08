use crate::collection::CollectionReport;
use crate::metric_support::{RegisterMetricResultExt, prometheus_u64};
use crate::sysfs::{read_trimmed, read_u64};
use prometheus::GaugeVec;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

struct AmdgpuMetrics {
    busy_ratio: GaugeVec,
    memory_bytes: GaugeVec,
    performance_level: GaugeVec,
    dpm_clock_hz: GaugeVec,
}

impl AmdgpuMetrics {
    fn new() -> Self {
        Self {
            busy_ratio: prometheus::register_gauge_vec!(
                "amdgpu_busy_ratio",
                "AMDGPU-reported graphics-engine busy ratio from 0 to 1.",
                &["card"]
            )
            .or_exit("amdgpu_busy_ratio"),
            memory_bytes: prometheus::register_gauge_vec!(
                "amdgpu_memory_bytes",
                "AMDGPU memory allocation state in bytes.",
                &["card", "kind"]
            )
            .or_exit("amdgpu_memory_bytes"),
            performance_level: prometheus::register_gauge_vec!(
                "amdgpu_performance_level_info",
                "Current AMDGPU forced performance level.",
                &["card", "level"]
            )
            .or_exit("amdgpu_performance_level_info"),
            dpm_clock_hz: prometheus::register_gauge_vec!(
                "amdgpu_dpm_clock_hz",
                "Current active AMDGPU DPM clock in hertz.",
                &["card", "domain"]
            )
            .or_exit("amdgpu_dpm_clock_hz"),
        }
    }

    fn reset(&self) {
        self.busy_ratio.reset();
        self.memory_bytes.reset();
        self.performance_level.reset();
        self.dpm_clock_hz.reset();
    }
}

static METRICS: OnceLock<AmdgpuMetrics> = OnceLock::new();

fn metrics() -> &'static AmdgpuMetrics {
    METRICS.get_or_init(AmdgpuMetrics::new)
}

fn active_clock_hz(path: &Path) -> Option<f64> {
    let contents = fs::read_to_string(path).ok()?;
    contents
        .lines()
        .filter(|line| line.contains('*'))
        .find_map(|line| {
            let value = line.split(':').nth(1)?.split_whitespace().next()?;
            let mhz = value
                .strip_suffix("Mhz")
                .or_else(|| value.strip_suffix("MHz"))?
                .parse::<f64>()
                .ok()?;
            Some(mhz * 1_000_000.0)
        })
}

fn update_card(metrics: &AmdgpuMetrics, card: &str, device: &Path) {
    let Some(vendor) = read_trimmed(&device.join("vendor")) else {
        return;
    };
    if vendor != "0x1002" {
        return;
    }

    if let Some(busy) = read_u64(&device.join("gpu_busy_percent")) {
        metrics
            .busy_ratio
            .with_label_values(&[card])
            .set(prometheus_u64(busy) / 100.0);
    }
    for (kind, file) in [
        ("vram_total", "mem_info_vram_total"),
        ("vram_used", "mem_info_vram_used"),
        ("gtt_total", "mem_info_gtt_total"),
        ("gtt_used", "mem_info_gtt_used"),
    ] {
        if let Some(value) = read_u64(&device.join(file)) {
            metrics
                .memory_bytes
                .with_label_values(&[card, kind])
                .set(prometheus_u64(value));
        }
    }
    if let Some(level) = read_trimmed(&device.join("power_dpm_force_performance_level")) {
        metrics
            .performance_level
            .with_label_values(&[card, &level])
            .set(1.0);
    }
    for (domain, file) in [("sclk", "pp_dpm_sclk"), ("mclk", "pp_dpm_mclk")] {
        if let Some(value) = active_clock_hz(&device.join(file)) {
            metrics
                .dpm_clock_hz
                .with_label_values(&[card, domain])
                .set(value);
        }
    }
}

pub fn update_metrics() -> CollectionReport {
    let metrics = metrics();
    metrics.reset();
    let Ok(entries) = fs::read_dir("/sys/class/drm") else {
        return CollectionReport::success();
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(card) = name.to_str() else {
            continue;
        };
        if !card.starts_with("card") || card.contains('-') {
            continue;
        }
        update_card(metrics, card, &entry.path().join("device"));
    }
    CollectionReport::success()
}

#[cfg(test)]
mod tests {
    use super::active_clock_hz;
    use std::fs;

    #[test]
    fn parses_active_dpm_clock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pp_dpm_sclk");
        fs::write(&path, "0: 200Mhz\n1: 800Mhz *\n2: 1600Mhz\n").unwrap();
        assert_eq!(active_clock_hz(&path), Some(800_000_000.0));
    }
}
