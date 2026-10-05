use crate::collection::CollectionReport;
use crate::metric_support::RegisterMetricResultExt;
use crate::metric_support::prometheus_u64;
use crate::sysfs::read_u64;
use prometheus::GaugeVec;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

struct CpuFreqMetrics {
    cpu_frequency_hz: GaugeVec,
}

impl CpuFreqMetrics {
    fn new() -> Self {
        Self {
            cpu_frequency_hz: prometheus::register_gauge_vec!(
                "cpu_frequency_hz",
                "Current CPU frequency per core",
                &["cpu", "source"]
            )
            .or_exit("cpu_frequency_hz"),
        }
    }
}

static CPUFREQ_METRICS: OnceLock<CpuFreqMetrics> = OnceLock::new();

fn metrics() -> &'static CpuFreqMetrics {
    CPUFREQ_METRICS.get_or_init(CpuFreqMetrics::new)
}

fn update_cpu(cpu_name: &str, cpufreq_dir: &Path) {
    let metrics = metrics();
    let scaling_path = cpufreq_dir.join("scaling_cur_freq");
    if let Some(khz) = read_u64(&scaling_path) {
        metrics
            .cpu_frequency_hz
            .with_label_values(&[cpu_name, "scaling_cur_freq"])
            .set(prometheus_u64(khz * 1000));
        return;
    }

    let info_path = cpufreq_dir.join("cpuinfo_cur_freq");
    if let Some(khz) = read_u64(&info_path) {
        metrics
            .cpu_frequency_hz
            .with_label_values(&[cpu_name, "cpuinfo_cur_freq"])
            .set(prometheus_u64(khz * 1000));
    }
}

pub fn update_metrics() -> CollectionReport {
    // CPUs can be taken offline, which removes their cpufreq directory.
    metrics().cpu_frequency_hz.reset();

    let base = Path::new("/sys/devices/system/cpu");
    let Ok(entries) = fs::read_dir(base) else {
        return CollectionReport::error();
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with("cpu") || name == "cpufreq" || name == "cpuidle" {
            continue;
        }
        if !name[3..].chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }

        let cpufreq_dir = entry.path().join("cpufreq");
        if cpufreq_dir.is_dir() {
            update_cpu(name, &cpufreq_dir);
        }
    }

    CollectionReport::success()
}
