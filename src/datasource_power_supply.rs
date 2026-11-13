use crate::collection::CollectionReport;
use crate::metric_support::RegisterMetricResultExt;
use crate::metric_support::prometheus_i64;
use crate::sysfs::{read_i64, read_trimmed};
use prometheus::GaugeVec;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

struct PowerSupplyMetrics {
    info: GaugeVec,
    online: GaugeVec,
    status: GaugeVec,
    capacity_percent: GaugeVec,
    voltage_volts: GaugeVec,
    current_amps: GaugeVec,
    power_watts: GaugeVec,
    energy_wh: GaugeVec,
    charge_ah: GaugeVec,
    temperature_celsius: GaugeVec,
    cycle_count: GaugeVec,
    charge_limit_percent: GaugeVec,
}

impl PowerSupplyMetrics {
    fn new() -> Self {
        Self {
            info: prometheus::register_gauge_vec!(
                "power_supply_info",
                "Power supply information",
                &["name", "type"]
            )
            .or_exit("power_supply_info"),

            online: prometheus::register_gauge_vec!(
                "power_supply_online",
                "Power supply online status (1 = online, 0 = offline)",
                &["name", "type"]
            )
            .or_exit("power_supply_online"),

            status: prometheus::register_gauge_vec!(
                "power_supply_status",
                "Battery status (1 = active for given state)",
                &["name", "status"]
            )
            .or_exit("power_supply_status"),

            capacity_percent: prometheus::register_gauge_vec!(
                "power_supply_capacity_percent",
                "Battery capacity in percent",
                &["name"]
            )
            .or_exit("power_supply_capacity_percent"),

            voltage_volts: prometheus::register_gauge_vec!(
                "power_supply_voltage_volts",
                "Power supply voltage in Volts",
                &["name", "measurement"]
            )
            .or_exit("power_supply_voltage_volts"),

            current_amps: prometheus::register_gauge_vec!(
                "power_supply_current_amps",
                "Power supply current in Amps (batteries: positive charging, negative discharging)",
                &["name", "measurement"]
            )
            .or_exit("power_supply_current_amps"),

            power_watts: prometheus::register_gauge_vec!(
                "power_supply_power_watts",
                "Power supply power in Watts",
                &["name"]
            )
            .or_exit("power_supply_power_watts"),

            energy_wh: prometheus::register_gauge_vec!(
                "power_supply_energy_wh",
                "Battery energy in Watt-hours",
                &["name", "measurement"]
            )
            .or_exit("power_supply_energy_wh"),

            charge_ah: prometheus::register_gauge_vec!(
                "power_supply_charge_ah",
                "Battery charge in Amp-hours",
                &["name", "measurement"]
            )
            .or_exit("power_supply_charge_ah"),

            temperature_celsius: prometheus::register_gauge_vec!(
                "power_supply_temperature_celsius",
                "Power supply temperature in Celsius",
                &["name"]
            )
            .or_exit("power_supply_temperature_celsius"),

            cycle_count: prometheus::register_gauge_vec!(
                "power_supply_cycle_count",
                "Battery charge/discharge cycle count when reported by the kernel",
                &["name"]
            )
            .or_exit("power_supply_cycle_count"),
            charge_limit_percent: prometheus::register_gauge_vec!(
                "power_supply_charge_limit_percent",
                "Configured battery charge ceiling when exposed by power_supply sysfs",
                &["name"]
            )
            .or_exit("power_supply_charge_limit_percent"),
        }
    }
}

static POWER_SUPPLY_METRICS: OnceLock<PowerSupplyMetrics> = OnceLock::new();

fn metrics() -> &'static PowerSupplyMetrics {
    POWER_SUPPLY_METRICS.get_or_init(PowerSupplyMetrics::new)
}

fn supply_current_amps(
    supply_type: &str,
    status: Option<&str>,
    current_microamps: Option<i64>,
    power_microwatts: Option<i64>,
    voltage_microvolts: Option<i64>,
) -> Option<f64> {
    if supply_type != "Battery" {
        return current_microamps.map(|current| prometheus_i64(current) / 1_000_000.0);
    }

    // Some batteries (including the ASUS G14) provide power_now and
    // voltage_now but no current_now. µW / µV already gives amperes.
    let current = if let Some(current) = current_microamps {
        prometheus_i64(current) / 1_000_000.0
    } else if matches!(status, Some("Charging" | "Discharging")) {
        let voltage = voltage_microvolts?;
        if voltage <= 0 {
            return None;
        }
        prometheus_i64(power_microwatts?) / prometheus_i64(voltage)
    } else {
        // An unsigned power_now cannot reveal direction by itself.
        return None;
    };

    // Different battery drivers expose current_now with different polarities.
    // The battery's status supplies the direction; retain raw readings when
    // the kernel does not report a charging/discharging direction.
    match status {
        Some("Charging") => Some(current.abs()),
        Some("Discharging") => Some(-current.abs()),
        _ => Some(current),
    }
}

fn update_battery_policy(metrics: &PowerSupplyMetrics, supply_path: &Path, supply_name: &str) {
    if let Some(cycles) = read_i64(&supply_path.join("cycle_count")) {
        metrics
            .cycle_count
            .with_label_values(&[supply_name])
            .set(prometheus_i64(cycles));
    }
    if let Some(limit) = read_i64(&supply_path.join("charge_control_end_threshold")) {
        metrics
            .charge_limit_percent
            .with_label_values(&[supply_name])
            .set(prometheus_i64(limit));
    }
}

fn update_electrical_measurements(
    metrics: &PowerSupplyMetrics,
    supply_path: &Path,
    supply_name: &str,
    supply_type: &str,
    status: Option<&str>,
) {
    // Voltage (microvolts -> volts)
    let voltage_now = read_i64(&supply_path.join("voltage_now"));
    if let Some(voltage) = voltage_now {
        metrics
            .voltage_volts
            .with_label_values(&[supply_name, "now"])
            .set(prometheus_i64(voltage) / 1_000_000.0);
    }
    if let Some(voltage) = read_i64(&supply_path.join("voltage_min_design")) {
        metrics
            .voltage_volts
            .with_label_values(&[supply_name, "min_design"])
            .set(prometheus_i64(voltage) / 1_000_000.0);
    }

    let power_now = read_i64(&supply_path.join("power_now"));
    // Battery current is positive when charging and negative when discharging;
    // other supplies retain the kernel's raw sign and measurement semantics.
    if let Some(current) = supply_current_amps(
        supply_type,
        status,
        read_i64(&supply_path.join("current_now")),
        power_now,
        voltage_now,
    ) {
        metrics
            .current_amps
            .with_label_values(&[supply_name, "now"])
            .set(current);
    }

    // Power (microwatts -> watts)
    if let Some(power) = power_now {
        metrics
            .power_watts
            .with_label_values(&[supply_name])
            .set(prometheus_i64(power) / 1_000_000.0);
    }
}

fn update_power_supply(supply_path: &Path, supply_name: &str) {
    let metrics = metrics();

    // Read supply type (Battery, Mains, UPS, USB)
    let supply_type =
        read_trimmed(&supply_path.join("type")).unwrap_or_else(|| "Unknown".to_string());

    // Set info metric
    metrics
        .info
        .with_label_values(&[supply_name, &supply_type])
        .set(1.0);

    // Online status (for AC/Mains)
    if let Some(online) = read_i64(&supply_path.join("online")) {
        metrics
            .online
            .with_label_values(&[supply_name, &supply_type])
            .set(prometheus_i64(online));
    }

    // Battery status (Charging, Discharging, Not charging, Full)
    let status = read_trimmed(&supply_path.join("status"));
    if let Some(status) = status.as_deref() {
        for state in ["Charging", "Discharging", "Not charging", "Full", "Unknown"] {
            metrics
                .status
                .with_label_values(&[supply_name, state])
                .set(if status == state { 1.0 } else { 0.0 });
        }
    }

    // Capacity (0-100%)
    if let Some(capacity) = read_i64(&supply_path.join("capacity")) {
        metrics
            .capacity_percent
            .with_label_values(&[supply_name])
            .set(prometheus_i64(capacity));
    }

    update_electrical_measurements(
        metrics,
        supply_path,
        supply_name,
        &supply_type,
        status.as_deref(),
    );

    // Energy (microwatt-hours -> watt-hours)
    if let Some(energy) = read_i64(&supply_path.join("energy_now")) {
        metrics
            .energy_wh
            .with_label_values(&[supply_name, "now"])
            .set(prometheus_i64(energy) / 1_000_000.0);
    }
    if let Some(energy) = read_i64(&supply_path.join("energy_full")) {
        metrics
            .energy_wh
            .with_label_values(&[supply_name, "full"])
            .set(prometheus_i64(energy) / 1_000_000.0);
    }
    if let Some(energy) = read_i64(&supply_path.join("energy_full_design")) {
        metrics
            .energy_wh
            .with_label_values(&[supply_name, "full_design"])
            .set(prometheus_i64(energy) / 1_000_000.0);
    }

    // Charge (microamp-hours -> amp-hours)
    if let Some(charge) = read_i64(&supply_path.join("charge_now")) {
        metrics
            .charge_ah
            .with_label_values(&[supply_name, "now"])
            .set(prometheus_i64(charge) / 1_000_000.0);
    }
    if let Some(charge) = read_i64(&supply_path.join("charge_full")) {
        metrics
            .charge_ah
            .with_label_values(&[supply_name, "full"])
            .set(prometheus_i64(charge) / 1_000_000.0);
    }
    if let Some(charge) = read_i64(&supply_path.join("charge_full_design")) {
        metrics
            .charge_ah
            .with_label_values(&[supply_name, "full_design"])
            .set(prometheus_i64(charge) / 1_000_000.0);
    }

    update_battery_policy(metrics, supply_path, supply_name);

    // Temperature (tenths of degree Celsius -> Celsius)
    if let Some(temp) = read_i64(&supply_path.join("temp")) {
        metrics
            .temperature_celsius
            .with_label_values(&[supply_name])
            .set(prometheus_i64(temp) / 10.0);
    }
}

pub fn update_metrics() -> CollectionReport {
    // Batteries and USB supplies are hot-pluggable. Clear first so a sysfs
    // failure cannot keep a removed supply alive in the registry.
    let metrics = metrics();
    metrics.info.reset();
    metrics.online.reset();
    metrics.status.reset();
    metrics.capacity_percent.reset();
    metrics.voltage_volts.reset();
    metrics.current_amps.reset();
    metrics.power_watts.reset();
    metrics.energy_wh.reset();
    metrics.charge_ah.reset();
    metrics.temperature_celsius.reset();
    metrics.cycle_count.reset();
    metrics.charge_limit_percent.reset();

    let base = Path::new("/sys/class/power_supply");
    let Ok(entries) = fs::read_dir(base) else {
        return CollectionReport::error();
    };

    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };

        let Ok(path) = fs::canonicalize(entry.path()) else {
            continue;
        };

        update_power_supply(&path, &name);
    }

    CollectionReport::success()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_battery(dir: &Path, name: &str, capacity: i64, status: &str) -> std::path::PathBuf {
        let supply_dir = dir.join(name);
        fs::create_dir_all(&supply_dir).unwrap();
        fs::write(supply_dir.join("type"), "Battery\n").unwrap();
        fs::write(supply_dir.join("capacity"), format!("{capacity}\n")).unwrap();
        fs::write(supply_dir.join("status"), format!("{status}\n")).unwrap();
        supply_dir
    }

    fn create_ac_adapter(dir: &Path, name: &str, online: i64) -> std::path::PathBuf {
        let supply_dir = dir.join(name);
        fs::create_dir_all(&supply_dir).unwrap();
        fs::write(supply_dir.join("type"), "Mains\n").unwrap();
        fs::write(supply_dir.join("online"), format!("{online}\n")).unwrap();
        supply_dir
    }

    #[test]
    fn test_read_string_trims_whitespace() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("type");
        fs::write(&file, "  Battery  \n").unwrap();
        assert_eq!(read_trimmed(&file), Some("Battery".to_string()));
    }

    #[test]
    fn test_read_i64_parses_integer() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("capacity");
        fs::write(&file, "85\n").unwrap();
        assert_eq!(read_i64(&file), Some(85));
    }

    #[test]
    fn test_read_i64_handles_negative() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("current_now");
        fs::write(&file, "-500000\n").unwrap();
        assert_eq!(read_i64(&file), Some(-500_000));
    }

    #[test]
    fn test_battery_current_direction_normalizes_both_raw_polarities() {
        for (status, expected) in [("Charging", 1.5), ("Discharging", -1.5)] {
            for raw in [-1_500_000, 1_500_000] {
                assert_eq!(
                    supply_current_amps("Battery", Some(status), Some(raw), None, None),
                    Some(expected)
                );
            }
        }
    }

    #[test]
    fn test_battery_current_from_power_and_voltage() {
        assert_eq!(
            supply_current_amps(
                "Battery",
                Some("Charging"),
                None,
                Some(12_000_000),
                Some(12_000_000)
            ),
            Some(1.0)
        );
        assert_eq!(
            supply_current_amps(
                "Battery",
                Some("Discharging"),
                None,
                Some(12_000_000),
                Some(12_000_000)
            ),
            Some(-1.0)
        );
        assert_eq!(
            supply_current_amps(
                "Battery",
                Some("Discharging"),
                None,
                Some(12_000_000),
                Some(0)
            ),
            None
        );
        assert_eq!(
            supply_current_amps("Battery", None, None, Some(12_000_000), Some(12_000_000)),
            None
        );
    }

    #[test]
    fn test_current_fallback_does_not_change_other_supply_semantics() {
        assert_eq!(
            supply_current_amps(
                "USB",
                Some("Discharging"),
                Some(3_250_000),
                Some(12_000_000),
                Some(12_000_000)
            ),
            Some(3.25)
        );
        assert_eq!(
            supply_current_amps("USB", None, None, Some(12_000_000), Some(12_000_000)),
            None
        );
        assert_eq!(
            supply_current_amps("Battery", None, Some(-123_000), None, None),
            Some(-0.123)
        );
    }

    #[test]
    fn test_battery_current_emitted_from_sysfs_power_without_current_file() {
        let dir = TempDir::new().unwrap();
        let supply = create_battery(dir.path(), "BAT_SIGNED", 85, "Discharging");
        fs::write(supply.join("voltage_now"), "12000000\n").unwrap();
        fs::write(supply.join("power_now"), "12000000\n").unwrap();
        update_power_supply(&supply, "BAT_SIGNED");
        assert_eq!(
            metrics()
                .current_amps
                .with_label_values(&["BAT_SIGNED", "now"])
                .get(),
            -1.0
        );
    }

    #[test]
    fn test_update_power_supply_battery() {
        let dir = TempDir::new().unwrap();
        let supply = create_battery(dir.path(), "BAT0", 85, "Discharging");
        update_power_supply(&supply, "BAT0");
    }

    #[test]
    fn test_update_power_supply_ac_adapter() {
        let dir = TempDir::new().unwrap();
        let supply = create_ac_adapter(dir.path(), "AC0", 1);
        update_power_supply(&supply, "AC0");
    }

    #[test]
    fn test_update_power_supply_with_voltage() {
        let dir = TempDir::new().unwrap();
        let supply = create_battery(dir.path(), "BAT0", 85, "Charging");
        fs::write(supply.join("voltage_now"), "12500000\n").unwrap(); // 12.5V
        fs::write(supply.join("current_now"), "1500000\n").unwrap(); // 1.5A

        update_power_supply(&supply, "BAT0");
    }

    #[test]
    fn test_update_power_supply_with_energy() {
        let dir = TempDir::new().unwrap();
        let supply = create_battery(dir.path(), "BAT0", 75, "Discharging");
        fs::write(supply.join("energy_now"), "30000000\n").unwrap(); // 30 Wh
        fs::write(supply.join("energy_full"), "40000000\n").unwrap(); // 40 Wh
        fs::write(supply.join("energy_full_design"), "45000000\n").unwrap(); // 45 Wh

        update_power_supply(&supply, "BAT0");
    }

    #[test]
    fn test_update_power_supply_missing_type() {
        let dir = TempDir::new().unwrap();
        let supply_dir = dir.path().join("BAT0");
        fs::create_dir_all(&supply_dir).unwrap();
        // No type file - should use "Unknown"
        fs::write(supply_dir.join("capacity"), "50\n").unwrap();

        update_power_supply(&supply_dir, "BAT0");
    }
    #[test]
    fn test_update_power_supply_with_charge_limit() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new()?;
        let supply = create_battery(dir.path(), "BAT_LIMIT", 80, "Not charging");
        fs::write(supply.join("charge_control_end_threshold"), "80\n")?;
        update_power_supply(&supply, "BAT_LIMIT");
        assert_eq!(
            metrics()
                .charge_limit_percent
                .with_label_values(&["BAT_LIMIT"])
                .get(),
            80.0
        );
        Ok(())
    }
}
