use crate::collection::CollectionReport;
use crate::metric_support::RegisterMetricResultExt;
use prometheus::{GaugeVec, IntGaugeVec};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::OnceLock;

struct LifecycleMetrics {
    boot_id: GaugeVec,
    build_info: GaugeVec,
    package_db_timestamp: IntGaugeVec,
}

impl LifecycleMetrics {
    fn new() -> Self {
        Self {
            boot_id: prometheus::register_gauge_vec!(
                "host_boot_id_info",
                "Current Linux boot ID.",
                &["boot_id"]
            )
            .or_exit("host_boot_id_info"),
            build_info: prometheus::register_gauge_vec!(
                "rs_linux_exporter_build_info",
                "rs-linux-exporter build information.",
                &["version"]
            )
            .or_exit("rs_linux_exporter_build_info"),
            package_db_timestamp: prometheus::register_int_gauge_vec!(
                "host_package_database_timestamp_seconds",
                "Modification timestamp of the host package database.",
                &["backend"]
            )
            .or_exit("host_package_database_timestamp_seconds"),
        }
    }
}

static METRICS: OnceLock<LifecycleMetrics> = OnceLock::new();

fn metrics() -> &'static LifecycleMetrics {
    METRICS.get_or_init(LifecycleMetrics::new)
}

fn update_package_database(metrics: &LifecycleMetrics) {
    const DATABASES: &[(&str, &str)] = &[
        ("pacman", "/var/lib/pacman/local"),
        ("dpkg", "/var/lib/dpkg/status"),
        ("rpm", "/usr/lib/sysimage/rpm/rpmdb.sqlite"),
        ("rpm", "/var/lib/rpm/rpmdb.sqlite"),
        ("apk", "/lib/apk/db/installed"),
    ];
    metrics.package_db_timestamp.reset();
    for &(backend, path) in DATABASES {
        let Ok(metadata) = fs::metadata(path) else {
            continue;
        };
        metrics
            .package_db_timestamp
            .with_label_values(&[backend])
            .set(metadata.mtime());
        break;
    }
}

pub fn update_metrics() -> CollectionReport {
    let metrics = metrics();
    metrics.build_info.reset();
    metrics
        .build_info
        .with_label_values(&[env!("CARGO_PKG_VERSION")])
        .set(1.0);

    metrics.boot_id.reset();
    if let Ok(boot_id) = fs::read_to_string(Path::new("/proc/sys/kernel/random/boot_id")) {
        let boot_id = boot_id.trim();
        if !boot_id.is_empty() {
            metrics.boot_id.with_label_values(&[boot_id]).set(1.0);
        }
    }
    update_package_database(metrics);
    CollectionReport::success()
}

#[cfg(test)]
mod tests {

    #[test]
    fn build_version_is_non_empty() {
        assert!(!env!("CARGO_PKG_VERSION").is_empty());
    }
}
