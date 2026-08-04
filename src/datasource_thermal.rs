use prometheus::{Gauge, GaugeVec};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

struct ThermalMetrics {
    zone_temperature_celsius: GaugeVec,
    zone_trip_point_celsius: GaugeVec,
    cooling_device_cur_state: GaugeVec,
    cooling_device_max_state: GaugeVec,
    zone_count: Gauge,
    cooling_device_count: Gauge,
}

impl ThermalMetrics {
    fn new() -> Self {
        Self {
            zone_temperature_celsius: prometheus::register_gauge_vec!(
                "thermal_zone_temperature_celsius",
                "Current temperature of the thermal zone in Celsius",
                &["zone", "type"]
            )
            .expect("register thermal_zone_temperature_celsius"),

            zone_trip_point_celsius: prometheus::register_gauge_vec!(
                "thermal_zone_trip_point_celsius",
                "Trip point temperature threshold in Celsius",
                &["zone", "type", "trip_point", "trip_type"]
            )
            .expect("register thermal_zone_trip_point_celsius"),

            cooling_device_cur_state: prometheus::register_gauge_vec!(
                "thermal_cooling_device_cur_state",
                "Current cooling state of the device",
                &["device", "type"]
            )
            .expect("register thermal_cooling_device_cur_state"),

            cooling_device_max_state: prometheus::register_gauge_vec!(
                "thermal_cooling_device_max_state",
                "Maximum cooling state of the device",
                &["device", "type"]
            )
            .expect("register thermal_cooling_device_max_state"),

            zone_count: prometheus::register_gauge!(
                "thermal_zone_count",
                "Number of thermal zones"
            )
            .expect("register thermal_zone_count"),

            cooling_device_count: prometheus::register_gauge!(
                "thermal_cooling_device_count",
                "Number of cooling devices"
            )
            .expect("register thermal_cooling_device_count"),
        }
    }
}

static THERMAL_METRICS: OnceLock<ThermalMetrics> = OnceLock::new();

fn metrics() -> &'static ThermalMetrics {
    THERMAL_METRICS.get_or_init(ThermalMetrics::new)
}

fn read_string(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn read_i64(path: &Path) -> Option<i64> {
    read_string(path)?.parse::<i64>().ok()
}

/// Extracts N from a `trip_point_N_temp` filename.
///
/// The prefix and suffix can overlap - `trip_point_temp` starts with one and
/// ends with the other - so slicing on fixed offsets can produce an inverted
/// range and panic. Strip both ends instead.
fn trip_point_index(file_name: &str) -> Option<&str> {
    file_name
        .strip_prefix("trip_point_")
        .and_then(|rest| rest.strip_suffix("_temp"))
        .filter(|index| !index.is_empty())
}

fn update_thermal_zone(zone_path: &Path, zone_name: &str) {
    let metrics = metrics();

    // Read zone type
    let zone_type = read_string(&zone_path.join("type")).unwrap_or_else(|| "unknown".to_string());

    // Read current temperature (millidegrees Celsius)
    if let Some(millidegrees) = read_i64(&zone_path.join("temp")) {
        metrics
            .zone_temperature_celsius
            .with_label_values(&[zone_name, &zone_type])
            .set(millidegrees as f64 / 1000.0);
    }

    // Read trip points
    let entries = match fs::read_dir(zone_path) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let file_name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(_) => continue,
        };

        // Match trip_point_N_temp files
        if let Some(index) = trip_point_index(&file_name) {
            if let Some(millidegrees) = read_i64(&entry.path()) {
                // Try to get the trip point type
                let trip_type_path = zone_path.join(format!("trip_point_{}_type", index));
                let trip_type =
                    read_string(&trip_type_path).unwrap_or_else(|| "unknown".to_string());

                metrics
                    .zone_trip_point_celsius
                    .with_label_values(&[zone_name, &zone_type, index, &trip_type])
                    .set(millidegrees as f64 / 1000.0);
            }
        }
    }
}

fn update_cooling_device(device_path: &Path, device_name: &str) {
    let metrics = metrics();

    // Read device type
    let device_type =
        read_string(&device_path.join("type")).unwrap_or_else(|| "unknown".to_string());

    // Read current state
    if let Some(cur_state) = read_i64(&device_path.join("cur_state")) {
        metrics
            .cooling_device_cur_state
            .with_label_values(&[device_name, &device_type])
            .set(cur_state as f64);
    }

    // Read max state
    if let Some(max_state) = read_i64(&device_path.join("max_state")) {
        metrics
            .cooling_device_max_state
            .with_label_values(&[device_name, &device_type])
            .set(max_state as f64);
    }
}

pub fn update_metrics() {
    let base = Path::new("/sys/class/thermal");
    let entries = match fs::read_dir(base) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    let metrics = metrics();

    // Zones and trip points appear and disappear with driver state.
    metrics.zone_temperature_celsius.reset();
    metrics.zone_trip_point_celsius.reset();
    metrics.cooling_device_cur_state.reset();
    metrics.cooling_device_max_state.reset();

    let mut zone_count = 0;
    let mut cooling_count = 0;

    for entry in entries.flatten() {
        let name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(_) => continue,
        };

        let path = match fs::canonicalize(entry.path()) {
            Ok(p) => p,
            Err(_) => continue,
        };

        if name.starts_with("thermal_zone") {
            update_thermal_zone(&path, &name);
            zone_count += 1;
        } else if name.starts_with("cooling_device") {
            update_cooling_device(&path, &name);
            cooling_count += 1;
        }
    }

    metrics.zone_count.set(zone_count as f64);
    metrics.cooling_device_count.set(cooling_count as f64);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_thermal_zone(
        dir: &Path,
        name: &str,
        zone_type: &str,
        temp: i64,
    ) -> std::path::PathBuf {
        let zone_dir = dir.join(name);
        fs::create_dir_all(&zone_dir).unwrap();
        fs::write(zone_dir.join("type"), format!("{}\n", zone_type)).unwrap();
        fs::write(zone_dir.join("temp"), format!("{}\n", temp)).unwrap();
        zone_dir
    }

    fn create_cooling_device(
        dir: &Path,
        name: &str,
        dev_type: &str,
        cur: i64,
        max: i64,
    ) -> std::path::PathBuf {
        let dev_dir = dir.join(name);
        fs::create_dir_all(&dev_dir).unwrap();
        fs::write(dev_dir.join("type"), format!("{}\n", dev_type)).unwrap();
        fs::write(dev_dir.join("cur_state"), format!("{}\n", cur)).unwrap();
        fs::write(dev_dir.join("max_state"), format!("{}\n", max)).unwrap();
        dev_dir
    }

    #[test]
    fn test_read_string_trims_whitespace() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("type");
        fs::write(&file, "  x86_pkg_temp  \n").unwrap();
        assert_eq!(read_string(&file), Some("x86_pkg_temp".to_string()));
    }

    #[test]
    fn test_read_i64_parses_integer() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("temp");
        fs::write(&file, "45000\n").unwrap();
        assert_eq!(read_i64(&file), Some(45000));
    }

    #[test]
    fn test_read_i64_handles_negative() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("temp");
        fs::write(&file, "-5000\n").unwrap();
        assert_eq!(read_i64(&file), Some(-5000));
    }

    #[test]
    fn test_update_thermal_zone_reads_temp() {
        let dir = TempDir::new().unwrap();
        let zone = create_thermal_zone(dir.path(), "thermal_zone0", "x86_pkg_temp", 55000);
        update_thermal_zone(&zone, "thermal_zone0");
    }

    #[test]
    fn test_update_thermal_zone_with_trip_points() {
        let dir = TempDir::new().unwrap();
        let zone = create_thermal_zone(dir.path(), "thermal_zone0", "x86_pkg_temp", 55000);
        fs::write(zone.join("trip_point_0_temp"), "100000\n").unwrap();
        fs::write(zone.join("trip_point_0_type"), "critical\n").unwrap();
        fs::write(zone.join("trip_point_1_temp"), "95000\n").unwrap();
        fs::write(zone.join("trip_point_1_type"), "hot\n").unwrap();

        update_thermal_zone(&zone, "thermal_zone0");
    }

    #[test]
    fn test_update_cooling_device() {
        let dir = TempDir::new().unwrap();
        let dev = create_cooling_device(dir.path(), "cooling_device0", "Processor", 0, 10);
        update_cooling_device(&dev, "cooling_device0");
    }

    #[test]
    fn test_trip_point_index_extracts_number() {
        assert_eq!(trip_point_index("trip_point_0_temp"), Some("0"));
        assert_eq!(trip_point_index("trip_point_12_temp"), Some("12"));
    }

    #[test]
    fn test_trip_point_index_rejects_overlapping_name() {
        // Starts with the prefix and ends with the suffix, but has no index
        // between them; slicing on fixed offsets used to panic here.
        assert_eq!(trip_point_index("trip_point_temp"), None);
        // The prefix and suffix consume the whole name, leaving no index.
        assert_eq!(trip_point_index("trip_point__temp"), None);
        assert_eq!(trip_point_index("trip_point_0_type"), None);
        assert_eq!(trip_point_index("temp"), None);
    }

    #[test]
    fn test_update_thermal_zone_survives_overlapping_filename() {
        let dir = TempDir::new().unwrap();
        let zone = create_thermal_zone(dir.path(), "thermal_zone0", "x86_pkg_temp", 55000);
        fs::write(zone.join("trip_point_temp"), "100000\n").unwrap();

        update_thermal_zone(&zone, "thermal_zone0");
    }

    #[test]
    fn test_update_cooling_device_missing_type() {
        let dir = TempDir::new().unwrap();
        let dev_dir = dir.path().join("cooling_device0");
        fs::create_dir_all(&dev_dir).unwrap();
        fs::write(dev_dir.join("cur_state"), "5\n").unwrap();
        fs::write(dev_dir.join("max_state"), "10\n").unwrap();
        // No type file - should use "unknown"

        update_cooling_device(&dev_dir, "cooling_device0");
    }
}
