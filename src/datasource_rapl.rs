use crate::metric_support::RegisterMetricResultExt;
use crate::metric_support::prometheus_u64;
use crate::sysfs::{read_trimmed, read_u64};
use prometheus::GaugeVec;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

struct RaplMetrics {
    energy_joules: GaugeVec,
    max_energy_joules: GaugeVec,
}

impl RaplMetrics {
    fn new() -> Self {
        Self {
            energy_joules: prometheus::register_gauge_vec!(
                "rapl_energy_joules",
                "Current energy counter in Joules (wraps at max_energy_joules)",
                &["zone", "name"]
            )
            .or_exit("rapl_energy_joules"),

            max_energy_joules: prometheus::register_gauge_vec!(
                "rapl_max_energy_joules",
                "Maximum energy counter range in Joules before wrap",
                &["zone", "name"]
            )
            .or_exit("rapl_max_energy_joules"),
        }
    }
}

static RAPL_METRICS: OnceLock<RaplMetrics> = OnceLock::new();

fn metrics() -> &'static RaplMetrics {
    RAPL_METRICS.get_or_init(RaplMetrics::new)
}

fn update_rapl_zone(zone_path: &Path, zone_id: &str) {
    let metrics = metrics();

    // Read zone name (e.g., "package-0", "core", "uncore", "dram")
    let name = read_trimmed(&zone_path.join("name")).unwrap_or_else(|| "unknown".to_string());

    // Read energy counter in microjoules, convert to joules
    if let Some(energy_uj) = read_u64(&zone_path.join("energy_uj")) {
        metrics
            .energy_joules
            .with_label_values(&[zone_id, &name])
            .set(prometheus_u64(energy_uj) / 1_000_000.0);
    }

    // Read max energy range in microjoules, convert to joules
    if let Some(max_energy_uj) = read_u64(&zone_path.join("max_energy_range_uj")) {
        metrics
            .max_energy_joules
            .with_label_values(&[zone_id, &name])
            .set(prometheus_u64(max_energy_uj) / 1_000_000.0);
    }

    // Process subzones (e.g., intel-rapl:0:0, intel-rapl:0:1)
    if let Ok(entries) = fs::read_dir(zone_path) {
        for entry in entries.flatten() {
            let Ok(entry_name) = entry.file_name().into_string() else {
                continue;
            };

            // Subzones have names like "intel-rapl:0:0" (contain two colons)
            if entry_name.contains(':')
                && entry.path().is_dir()
                && let Some(subzone_name) = read_trimmed(&entry.path().join("name"))
            {
                // Read subzone energy
                if let Some(energy_uj) = read_u64(&entry.path().join("energy_uj")) {
                    metrics
                        .energy_joules
                        .with_label_values(&[&entry_name, &subzone_name])
                        .set(prometheus_u64(energy_uj) / 1_000_000.0);
                }

                // Read subzone max energy range
                if let Some(max_energy_uj) = read_u64(&entry.path().join("max_energy_range_uj")) {
                    metrics
                        .max_energy_joules
                        .with_label_values(&[&entry_name, &subzone_name])
                        .set(prometheus_u64(max_energy_uj) / 1_000_000.0);
                }
            }
        }
    }
}

pub fn update_metrics() {
    let metrics = metrics();
    metrics.energy_joules.reset();
    metrics.max_energy_joules.reset();

    let base = Path::new("/sys/class/powercap");
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };

    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };

        // Match intel-rapl:N or amd-rapl:N zones (top-level packages)
        if (name.starts_with("intel-rapl:") || name.starts_with("amd-rapl:"))
            && name.matches(':').count() == 1
        {
            let Ok(path) = fs::canonicalize(entry.path()) else {
                continue;
            };
            update_rapl_zone(&path, &name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_rapl_zone(
        dir: &Path,
        name: &str,
        zone_name: &str,
        energy: u64,
        max_energy: u64,
    ) -> std::path::PathBuf {
        let zone_dir = dir.join(name);
        fs::create_dir_all(&zone_dir).unwrap();
        fs::write(zone_dir.join("name"), format!("{}\n", zone_name)).unwrap();
        fs::write(zone_dir.join("energy_uj"), format!("{}\n", energy)).unwrap();
        fs::write(
            zone_dir.join("max_energy_range_uj"),
            format!("{}\n", max_energy),
        )
        .unwrap();
        zone_dir
    }

    #[test]
    fn test_read_string_trims_whitespace() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("name");
        fs::write(&file, "  package-0  \n").unwrap();
        assert_eq!(read_trimmed(&file), Some("package-0".to_string()));
    }

    #[test]
    fn test_read_u64_parses_integer() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("energy_uj");
        fs::write(&file, "123456789\n").unwrap();
        assert_eq!(read_u64(&file), Some(123456789));
    }

    #[test]
    fn test_read_u64_handles_invalid() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("energy_uj");
        fs::write(&file, "not_a_number\n").unwrap();
        assert_eq!(read_u64(&file), None);
    }

    #[test]
    fn test_update_rapl_zone_reads_energy() {
        let dir = TempDir::new().unwrap();
        let zone = create_rapl_zone(
            dir.path(),
            "intel-rapl:0",
            "package-0",
            1000000, // 1 Joule in microjoules
            262143328850,
        );
        update_rapl_zone(&zone, "intel-rapl:0");
    }

    #[test]
    fn test_update_rapl_zone_with_subzones() {
        let dir = TempDir::new().unwrap();
        let zone = create_rapl_zone(
            dir.path(),
            "intel-rapl:0",
            "package-0",
            1000000,
            262143328850,
        );
        // Create subzone
        create_rapl_zone(&zone, "intel-rapl:0:0", "core", 500000, 262143328850);

        update_rapl_zone(&zone, "intel-rapl:0");
    }

    #[test]
    fn test_update_rapl_zone_missing_name() {
        let dir = TempDir::new().unwrap();
        let zone_dir = dir.path().join("intel-rapl:0");
        fs::create_dir_all(&zone_dir).unwrap();
        fs::write(zone_dir.join("energy_uj"), "1000000\n").unwrap();
        // No name file - should use "unknown"

        update_rapl_zone(&zone_dir, "intel-rapl:0");
    }
}
