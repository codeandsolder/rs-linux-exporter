use crate::collection::CollectionReport;
use crate::metric_support::RegisterMetricResultExt;
use crate::runtime::debug_enabled;
use prometheus::GaugeVec;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::OnceLock;

const DMI_PATH: &str = "/sys/class/dmi/id";
const FIELDS: [(&str, &str); 20] = [
    ("bios_date", "bios_date"),
    ("bios_release", "bios_release"),
    ("bios_vendor", "bios_vendor"),
    ("bios_version", "bios_version"),
    ("board_asset_tag", "board_asset_tag"),
    ("board_name", "board_name"),
    ("board_serial", "board_serial"),
    ("board_vendor", "board_vendor"),
    ("board_version", "board_version"),
    ("chassis_asset_tag", "chassis_asset_tag"),
    ("chassis_serial", "chassis_serial"),
    ("chassis_vendor", "chassis_vendor"),
    ("chassis_version", "chassis_version"),
    ("product_family", "product_family"),
    ("product_name", "product_name"),
    ("product_serial", "product_serial"),
    ("product_sku", "product_sku"),
    ("product_uuid", "product_uuid"),
    ("product_version", "product_version"),
    ("sys_vendor", "system_vendor"),
];

struct DmiMetrics {
    info: GaugeVec,
}

impl DmiMetrics {
    fn new() -> Self {
        Self {
            info: prometheus::register_gauge_vec!(
                "dmi_info",
                "DMI information exposed by /sys/class/dmi/id.",
                &[
                    "bios_date",
                    "bios_release",
                    "bios_vendor",
                    "bios_version",
                    "board_asset_tag",
                    "board_name",
                    "board_serial",
                    "board_vendor",
                    "board_version",
                    "chassis_asset_tag",
                    "chassis_serial",
                    "chassis_vendor",
                    "chassis_version",
                    "product_family",
                    "product_name",
                    "product_serial",
                    "product_sku",
                    "product_uuid",
                    "product_version",
                    "system_vendor"
                ]
            )
            .or_exit("dmi_info"),
        }
    }
}

static METRICS: OnceLock<DmiMetrics> = OnceLock::new();

fn metrics() -> &'static DmiMetrics {
    METRICS.get_or_init(DmiMetrics::new)
}

fn read_optional(path: &Path) -> Result<String, String> {
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

fn read_labels(root: &Path) -> Result<Vec<String>, String> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    FIELDS
        .iter()
        .map(|(filename, _)| read_optional(&root.join(filename)))
        .collect()
}

fn update_metrics_from_path(root: &Path) -> CollectionReport {
    let values = match read_labels(root) {
        Ok(values) => values,
        Err(error) => {
            if debug_enabled() {
                eprintln!("dmi: {error}");
            }
            metrics().info.reset();
            return CollectionReport::error();
        }
    };
    metrics().info.reset();
    if values.is_empty() || values.iter().all(String::is_empty) {
        return CollectionReport::success();
    }
    let labels = values.iter().map(String::as_str).collect::<Vec<_>>();
    metrics().info.with_label_values(&labels).set(1.0);
    CollectionReport::success()
}

pub fn update_metrics() -> CollectionReport {
    update_metrics_from_path(Path::new(DMI_PATH))
}

#[cfg(test)]
mod tests {
    use super::{FIELDS, read_labels};
    use std::fs;

    #[test]
    fn reads_present_fields_and_leaves_missing_fields_empty() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("product_name"), "Example Box\n").unwrap();
        let values = read_labels(temp.path()).unwrap();
        assert_eq!(values.len(), FIELDS.len());
        let product_name = FIELDS
            .iter()
            .position(|(filename, _)| *filename == "product_name")
            .unwrap();
        assert_eq!(values[product_name], "Example Box");
    }
}
