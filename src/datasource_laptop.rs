use crate::collection::CollectionReport;
use crate::metric_support::RegisterMetricResultExt;
use crate::sysfs::{read_i64, read_trimmed};
use prometheus::{Gauge, GaugeVec};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

const LID_ROOT: &str = "/proc/acpi/button/lid";
const DRM_ROOT: &str = "/sys/class/drm";
const ACER_HEALTH_MODE: &str = "/sys/bus/wmi/drivers/acer-wmi-battery/health_mode";

struct LaptopMetrics {
    lid_closed: GaugeVec,
    external_display_connected: Gauge,
    charge_limit_enabled: GaugeVec,
}

impl LaptopMetrics {
    fn new() -> Self {
        Self {
            lid_closed: prometheus::register_gauge_vec!(
                "laptop_lid_closed",
                "Laptop lid state (1 = closed, 0 = open).",
                &["lid"]
            )
            .or_exit("laptop_lid_closed"),
            external_display_connected: prometheus::register_gauge!(
                "laptop_external_display_connected",
                "Whether at least one external DRM display connector is connected."
            )
            .or_exit("laptop_external_display_connected"),
            charge_limit_enabled: prometheus::register_gauge_vec!(
                "laptop_charge_limit_enabled",
                "Whether a battery charge-limit backend is enabled.",
                &["backend"]
            )
            .or_exit("laptop_charge_limit_enabled"),
        }
    }
}

static METRICS: OnceLock<LaptopMetrics> = OnceLock::new();

fn metrics() -> &'static LaptopMetrics {
    METRICS.get_or_init(LaptopMetrics::new)
}

fn parse_lid_state(value: &str) -> Option<bool> {
    if value.contains("closed") {
        Some(true)
    } else if value.contains("open") {
        Some(false)
    } else {
        None
    }
}

fn is_internal_connector(name: &str) -> bool {
    let uppercase = name.to_ascii_uppercase();
    ["-EDP-", "-LVDS-", "-DSI-"]
        .iter()
        .any(|marker| uppercase.contains(marker))
}

fn update_lids(root: &Path, metrics: &LaptopMetrics) -> CollectionReport {
    metrics.lid_closed.reset();
    let Ok(entries) = fs::read_dir(root) else {
        return CollectionReport::error();
    };
    let mut report = CollectionReport::success();
    for entry in entries {
        let Ok(entry) = entry else {
            report.record_error();
            continue;
        };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(state) =
            read_trimmed(&entry.path().join("state")).and_then(|value| parse_lid_state(&value))
        else {
            continue;
        };
        metrics
            .lid_closed
            .with_label_values(&[&name])
            .set(f64::from(u8::from(state)));
    }
    report
}

fn update_external_display(root: &Path, metrics: &LaptopMetrics) -> CollectionReport {
    metrics.external_display_connected.set(f64::NAN);
    let Ok(entries) = fs::read_dir(root) else {
        return CollectionReport::error();
    };
    let mut connected = false;
    let mut report = CollectionReport::success();
    for entry in entries {
        let Ok(entry) = entry else {
            report.record_error();
            continue;
        };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if is_internal_connector(&name) || name.contains("Writeback") {
            continue;
        }
        let status = entry.path().join("status");
        if !status.exists() {
            continue;
        }
        if matches!(read_trimmed(&status).as_deref(), Some("connected")) {
            connected = true;
        }
    }
    metrics
        .external_display_connected
        .set(f64::from(u8::from(connected)));
    report
}

fn update_charge_limit(metrics: &LaptopMetrics, acer_health_mode: &Path) {
    metrics.charge_limit_enabled.reset();
    if let Some(value) = read_i64(acer_health_mode) {
        metrics
            .charge_limit_enabled
            .with_label_values(&["acer_health_mode"])
            .set(f64::from(u8::from(value != 0)));
    }
}

fn update_metrics_from_paths(
    lid_root: &Path,
    drm_root: &Path,
    acer_health_mode: &Path,
) -> CollectionReport {
    let metrics = metrics();
    let mut report = update_lids(lid_root, metrics);
    report.merge(update_external_display(drm_root, metrics));
    update_charge_limit(metrics, acer_health_mode);
    report
}

pub fn update_metrics() -> CollectionReport {
    update_metrics_from_paths(
        Path::new(LID_ROOT),
        Path::new(DRM_ROOT),
        Path::new(ACER_HEALTH_MODE),
    )
}

#[cfg(test)]
mod tests {
    use super::{is_internal_connector, metrics, parse_lid_state, update_metrics_from_paths};
    use std::error::Error;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn parses_common_lid_state_strings() {
        assert_eq!(parse_lid_state("state:      open"), Some(false));
        assert_eq!(parse_lid_state("state:      closed"), Some(true));
        assert_eq!(parse_lid_state("unknown"), None);
    }

    #[test]
    fn detects_internal_drm_connectors_case_insensitively() {
        assert!(is_internal_connector("card1-eDP-1"));
        assert!(is_internal_connector("card0-LVDS-1"));
        assert!(is_internal_connector("card0-dsi-1"));
        assert!(!is_internal_connector("card1-HDMI-A-1"));
        assert!(!is_internal_connector("card0-DP-2"));
    }
    #[test]
    fn collects_platform_state_from_kernel_files() -> Result<(), Box<dyn Error>> {
        let root = TempDir::new()?;
        let lids = root.path().join("lid");
        let drm = root.path().join("drm");
        let lid = lids.join("LIDTEST");
        let internal = drm.join("card0-eDP-1");
        let external = drm.join("card0-HDMI-A-1");
        fs::create_dir_all(&lid)?;
        fs::create_dir_all(&internal)?;
        fs::create_dir_all(&external)?;
        fs::write(lid.join("state"), "state:      closed\n")?;
        fs::write(internal.join("status"), "connected\n")?;
        fs::write(external.join("status"), "connected\n")?;
        let health_mode = root.path().join("health_mode");
        fs::write(&health_mode, "1\n")?;

        assert!(update_metrics_from_paths(&lids, &drm, &health_mode).is_success());
        let metrics = metrics();
        assert_eq!(
            metrics.lid_closed.with_label_values(&["LIDTEST"]).get(),
            1.0
        );
        assert_eq!(metrics.external_display_connected.get(), 1.0);
        assert_eq!(
            metrics
                .charge_limit_enabled
                .with_label_values(&["acer_health_mode"])
                .get(),
            1.0
        );
        Ok(())
    }
}
