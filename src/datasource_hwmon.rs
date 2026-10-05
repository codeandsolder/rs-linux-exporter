use crate::metric_support::RegisterMetricResultExt;
use crate::metric_support::prometheus_i64;
use prometheus::GaugeVec;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

struct HwmonMetrics {
    temperature_celsius: GaugeVec,
    fan_rpm: GaugeVec,
    voltage_volts: GaugeVec,
    power_watts: GaugeVec,
    current_amps: GaugeVec,
}

impl HwmonMetrics {
    fn new() -> Self {
        Self {
            temperature_celsius: prometheus::register_gauge_vec!(
                "hwmon_temperature_celsius",
                "Hardware monitor temperature sensor reading in Celsius",
                &["chip", "sensor"]
            )
            .or_exit("hwmon_temperature_celsius"),

            fan_rpm: prometheus::register_gauge_vec!(
                "hwmon_fan_rpm",
                "Hardware monitor fan speed in RPM",
                &["chip", "sensor"]
            )
            .or_exit("hwmon_fan_rpm"),

            voltage_volts: prometheus::register_gauge_vec!(
                "hwmon_voltage_volts",
                "Hardware monitor voltage reading in Volts",
                &["chip", "sensor"]
            )
            .or_exit("hwmon_voltage_volts"),

            power_watts: prometheus::register_gauge_vec!(
                "hwmon_power_watts",
                "Hardware monitor power reading in Watts",
                &["chip", "sensor"]
            )
            .or_exit("hwmon_power_watts"),

            current_amps: prometheus::register_gauge_vec!(
                "hwmon_current_amps",
                "Hardware monitor current reading in Amps",
                &["chip", "sensor"]
            )
            .or_exit("hwmon_current_amps"),
        }
    }
}

static HWMON_METRICS: OnceLock<HwmonMetrics> = OnceLock::new();

fn metrics() -> &'static HwmonMetrics {
    HWMON_METRICS.get_or_init(HwmonMetrics::new)
}

fn read_value(path: &Path) -> Option<i64> {
    let contents = fs::read_to_string(path).ok()?;
    contents.trim().parse::<i64>().ok()
}

fn read_string(path: &Path) -> Option<String> {
    let contents = fs::read_to_string(path).ok()?;
    Some(contents.trim().to_string())
}

fn get_sensor_label(hwmon_dir: &Path, sensor_type: &str, index: &str) -> String {
    let label_path = hwmon_dir.join(format!("{sensor_type}_{index}_label"));
    read_string(&label_path).unwrap_or_else(|| format!("{sensor_type}_{index}"))
}

fn update_hwmon_device(hwmon_dir: &Path) {
    let chip_name = match read_string(&hwmon_dir.join("name")) {
        Some(name) => name,
        None => return,
    };

    let entries = match fs::read_dir(hwmon_dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    let metrics = metrics();

    for entry in entries.flatten() {
        let file_name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(_) => continue,
        };

        // Temperature sensors: temp[1-*]_input (millidegrees Celsius)
        if file_name.starts_with("temp") && file_name.ends_with("_input") {
            let index = &file_name[4..file_name.len() - 6];
            if let Some(millidegrees) = read_value(&entry.path()) {
                let label = get_sensor_label(hwmon_dir, "temp", index);
                metrics
                    .temperature_celsius
                    .with_label_values(&[&chip_name, &label])
                    .set(prometheus_i64(millidegrees) / 1000.0);
            }
        }
        // Fan sensors: fan[1-*]_input (RPM)
        else if file_name.starts_with("fan") && file_name.ends_with("_input") {
            let index = &file_name[3..file_name.len() - 6];
            if let Some(rpm) = read_value(&entry.path()) {
                let label = get_sensor_label(hwmon_dir, "fan", index);
                metrics
                    .fan_rpm
                    .with_label_values(&[&chip_name, &label])
                    .set(prometheus_i64(rpm));
            }
        }
        // Voltage sensors: in[0-*]_input (millivolts)
        else if file_name.starts_with("in") && file_name.ends_with("_input") {
            let index = &file_name[2..file_name.len() - 6];
            if index.chars().all(|c| c.is_ascii_digit())
                && let Some(millivolts) = read_value(&entry.path())
            {
                let label = get_sensor_label(hwmon_dir, "in", index);
                metrics
                    .voltage_volts
                    .with_label_values(&[&chip_name, &label])
                    .set(prometheus_i64(millivolts) / 1000.0);
            }
        }
        // Power sensors: power[1-*]_input (microwatts)
        else if file_name.starts_with("power") && file_name.ends_with("_input") {
            let index = &file_name[5..file_name.len() - 6];
            if let Some(microwatts) = read_value(&entry.path()) {
                let label = get_sensor_label(hwmon_dir, "power", index);
                metrics
                    .power_watts
                    .with_label_values(&[&chip_name, &label])
                    .set(prometheus_i64(microwatts) / 1_000_000.0);
            }
        }
        // Current sensors: curr[1-*]_input (milliamps)
        else if file_name.starts_with("curr") && file_name.ends_with("_input") {
            let index = &file_name[4..file_name.len() - 6];
            if let Some(milliamps) = read_value(&entry.path()) {
                let label = get_sensor_label(hwmon_dir, "curr", index);
                metrics
                    .current_amps
                    .with_label_values(&[&chip_name, &label])
                    .set(prometheus_i64(milliamps) / 1000.0);
            }
        }
    }
}

pub fn update_metrics() {
    update_metrics_from_path(Path::new("/sys/class/hwmon"));
}

fn update_metrics_from_path(base: &Path) {
    let entries = match fs::read_dir(base) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    // Chips and sensors disappear when a module is unloaded or a device is
    // unplugged; rebuild so their readings do not stay frozen.
    let metrics = metrics();
    metrics.temperature_celsius.reset();
    metrics.fan_rpm.reset();
    metrics.voltage_volts.reset();
    metrics.power_watts.reset();
    metrics.current_amps.reset();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() || path.is_symlink() {
            // Resolve symlinks to get the actual hwmon directory
            let resolved = match fs::canonicalize(&path) {
                Ok(p) => p,
                Err(_) => continue,
            };
            update_hwmon_device(&resolved);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_mock_hwmon(dir: &Path, name: &str, chip_name: &str) -> std::path::PathBuf {
        let hwmon_dir = dir.join(name);
        fs::create_dir_all(&hwmon_dir).unwrap();
        fs::write(hwmon_dir.join("name"), format!("{}\n", chip_name)).unwrap();
        hwmon_dir
    }

    #[test]
    fn test_read_value_parses_integer() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("temp1_input");
        fs::write(&file, "45000\n").unwrap();
        assert_eq!(read_value(&file), Some(45000));
    }

    #[test]
    fn test_read_value_handles_invalid() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("temp1_input");
        fs::write(&file, "not_a_number\n").unwrap();
        assert_eq!(read_value(&file), None);
    }

    #[test]
    fn test_read_string_trims_whitespace() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("name");
        fs::write(&file, "  coretemp  \n").unwrap();
        assert_eq!(read_string(&file), Some("coretemp".to_string()));
    }

    #[test]
    fn test_get_sensor_label_with_label_file() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("temp_1_label"), "Core 0\n").unwrap();
        assert_eq!(get_sensor_label(dir.path(), "temp", "1"), "Core 0");
    }

    #[test]
    fn test_get_sensor_label_fallback() {
        let dir = TempDir::new().unwrap();
        // No label file exists
        assert_eq!(get_sensor_label(dir.path(), "temp", "1"), "temp_1");
    }

    #[test]
    fn test_update_hwmon_device_with_temp_sensor() {
        let dir = TempDir::new().unwrap();
        let hwmon = create_mock_hwmon(dir.path(), "hwmon0", "coretemp");
        fs::write(hwmon.join("temp1_input"), "45000\n").unwrap();

        // Should not panic
        update_hwmon_device(&hwmon);
    }

    #[test]
    fn test_update_hwmon_device_with_fan_sensor() {
        let dir = TempDir::new().unwrap();
        let hwmon = create_mock_hwmon(dir.path(), "hwmon0", "nct6775");
        fs::write(hwmon.join("fan1_input"), "1200\n").unwrap();

        update_hwmon_device(&hwmon);
    }

    #[test]
    fn test_update_hwmon_device_with_voltage_sensor() {
        let dir = TempDir::new().unwrap();
        let hwmon = create_mock_hwmon(dir.path(), "hwmon0", "nct6775");
        fs::write(hwmon.join("in0_input"), "1200\n").unwrap();

        update_hwmon_device(&hwmon);
    }

    #[test]
    fn test_update_hwmon_device_skips_without_name() {
        let dir = TempDir::new().unwrap();
        let hwmon = dir.path().join("hwmon0");
        fs::create_dir_all(&hwmon).unwrap();
        // No "name" file
        fs::write(hwmon.join("temp1_input"), "45000\n").unwrap();

        // Should return early without panicking
        update_hwmon_device(&hwmon);
    }

    #[test]
    fn test_update_metrics_from_path_handles_empty_dir() {
        let dir = TempDir::new().unwrap();
        // Empty directory - should not panic
        update_metrics_from_path(dir.path());
    }
}
