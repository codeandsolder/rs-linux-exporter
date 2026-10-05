use crate::metric_support::RegisterMetricResultExt;
use prometheus::GaugeVec;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

struct NvmeMetrics {
    info: GaugeVec,
    state: GaugeVec,
}

impl NvmeMetrics {
    fn new() -> Self {
        Self {
            info: prometheus::register_gauge_vec!(
                "nvme_info",
                "NVMe device information",
                &["device", "model", "serial", "firmware_rev"]
            )
            .or_exit("nvme_info"),

            state: prometheus::register_gauge_vec!(
                "nvme_state",
                "NVMe device state (1 = active for given state)",
                &["device", "state"]
            )
            .or_exit("nvme_state"),
        }
    }
}

static NVME_METRICS: OnceLock<NvmeMetrics> = OnceLock::new();

fn metrics() -> &'static NvmeMetrics {
    NVME_METRICS.get_or_init(NvmeMetrics::new)
}

fn read_string(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn update_nvme_device(device_path: &Path, device_name: &str) {
    let metrics = metrics();

    // Read device attributes
    let model = read_string(&device_path.join("model")).unwrap_or_default();
    let serial = read_string(&device_path.join("serial")).unwrap_or_default();
    let firmware_rev = read_string(&device_path.join("firmware_rev")).unwrap_or_default();
    let state = read_string(&device_path.join("state")).unwrap_or_else(|| "unknown".to_string());

    // Set info metric (always 1, labels carry the information)
    metrics
        .info
        .with_label_values(&[device_name, &model, &serial, &firmware_rev])
        .set(1.0);

    // Set state metrics
    for known_state in ["live", "dead", "deleting", "connecting", "resetting"] {
        metrics
            .state
            .with_label_values(&[device_name, known_state])
            .set(if state == known_state { 1.0 } else { 0.0 });
    }
}

pub fn update_metrics() {
    update_metrics_from_path(Path::new("/sys/class/nvme"));
}

fn update_metrics_from_path(base: &Path) {
    let entries = match fs::read_dir(base) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    // Drop controllers that are gone, and stale nvme_info series left behind by
    // a firmware upgrade (the revision is a label).
    let metrics = metrics();
    metrics.info.reset();
    metrics.state.reset();

    for entry in entries.flatten() {
        let name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(_) => continue,
        };

        // Only process nvme controller directories (nvme0, nvme1, etc.)
        if !name.starts_with("nvme") {
            continue;
        }

        let path = match fs::canonicalize(entry.path()) {
            Ok(p) => p,
            Err(_) => continue,
        };

        if path.is_dir() {
            update_nvme_device(&path, &name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_mock_nvme(dir: &Path, name: &str, model: &str, serial: &str, fw: &str, state: &str) {
        let nvme_dir = dir.join(name);
        fs::create_dir_all(&nvme_dir).unwrap();
        fs::write(nvme_dir.join("model"), format!("{}\n", model)).unwrap();
        fs::write(nvme_dir.join("serial"), format!("{}\n", serial)).unwrap();
        fs::write(nvme_dir.join("firmware_rev"), format!("{}\n", fw)).unwrap();
        fs::write(nvme_dir.join("state"), format!("{}\n", state)).unwrap();
    }

    #[test]
    fn test_read_string_trims_whitespace() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test");
        fs::write(&file, "  hello world  \n").unwrap();
        assert_eq!(read_string(&file), Some("hello world".to_string()));
    }

    #[test]
    fn test_read_string_missing_file() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("nonexistent");
        assert_eq!(read_string(&file), None);
    }

    #[test]
    fn test_update_nvme_device_parses_attributes() {
        let dir = TempDir::new().unwrap();
        create_mock_nvme(
            dir.path(),
            "nvme0",
            "Samsung SSD 980 PRO",
            "S5GXNF0N123456",
            "5B2QGXA7",
            "live",
        );

        // Just verify it doesn't panic - metrics are registered globally
        let nvme_path = dir.path().join("nvme0");
        update_nvme_device(&nvme_path, "nvme0");
    }

    #[test]
    fn test_update_metrics_from_path_filters_non_nvme() {
        let dir = TempDir::new().unwrap();
        create_mock_nvme(dir.path(), "nvme0", "Model1", "SN1", "FW1", "live");
        fs::create_dir_all(dir.path().join("not_nvme")).unwrap();

        // Should only process nvme0, not "not_nvme"
        update_metrics_from_path(dir.path());
    }
}
