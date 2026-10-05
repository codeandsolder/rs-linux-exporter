use crate::metric_support::prometheus_i64;
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
}

impl PowerSupplyMetrics {
    fn new() -> Self {
        Self {
            info: prometheus::register_gauge_vec!(
                "power_supply_info",
                "Power supply information",
                &["name", "type"]
            )
            .expect("register power_supply_info"),

            online: prometheus::register_gauge_vec!(
                "power_supply_online",
                "Power supply online status (1 = online, 0 = offline)",
                &["name", "type"]
            )
            .expect("register power_supply_online"),

            status: prometheus::register_gauge_vec!(
                "power_supply_status",
                "Battery status (1 = active for given state)",
                &["name", "status"]
            )
            .expect("register power_supply_status"),

            capacity_percent: prometheus::register_gauge_vec!(
                "power_supply_capacity_percent",
                "Battery capacity in percent",
                &["name"]
            )
            .expect("register power_supply_capacity_percent"),

            voltage_volts: prometheus::register_gauge_vec!(
                "power_supply_voltage_volts",
                "Power supply voltage in Volts",
                &["name", "measurement"]
            )
            .expect("register power_supply_voltage_volts"),

            current_amps: prometheus::register_gauge_vec!(
                "power_supply_current_amps",
                "Power supply current in Amps",
                &["name", "measurement"]
            )
            .expect("register power_supply_current_amps"),

            power_watts: prometheus::register_gauge_vec!(
                "power_supply_power_watts",
                "Power supply power in Watts",
                &["name"]
            )
            .expect("register power_supply_power_watts"),

            energy_wh: prometheus::register_gauge_vec!(
                "power_supply_energy_wh",
                "Battery energy in Watt-hours",
                &["name", "measurement"]
            )
            .expect("register power_supply_energy_wh"),

            charge_ah: prometheus::register_gauge_vec!(
                "power_supply_charge_ah",
                "Battery charge in Amp-hours",
                &["name", "measurement"]
            )
            .expect("register power_supply_charge_ah"),

            temperature_celsius: prometheus::register_gauge_vec!(
                "power_supply_temperature_celsius",
                "Power supply temperature in Celsius",
                &["name"]
            )
            .expect("register power_supply_temperature_celsius"),
        }
    }
}

static POWER_SUPPLY_METRICS: OnceLock<PowerSupplyMetrics> = OnceLock::new();

fn metrics() -> &'static PowerSupplyMetrics {
    POWER_SUPPLY_METRICS.get_or_init(PowerSupplyMetrics::new)
}

fn read_string(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn read_i64(path: &Path) -> Option<i64> {
    read_string(path)?.parse::<i64>().ok()
}

fn update_power_supply(supply_path: &Path, supply_name: &str) {
    let metrics = metrics();

    // Read supply type (Battery, Mains, UPS, USB)
    let supply_type =
        read_string(&supply_path.join("type")).unwrap_or_else(|| "Unknown".to_string());

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
    if let Some(status) = read_string(&supply_path.join("status")) {
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

    // Voltage (microvolts -> volts)
    if let Some(voltage) = read_i64(&supply_path.join("voltage_now")) {
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

    // Current (microamps -> amps)
    if let Some(current) = read_i64(&supply_path.join("current_now")) {
        metrics
            .current_amps
            .with_label_values(&[supply_name, "now"])
            .set(prometheus_i64(current) / 1_000_000.0);
    }

    // Power (microwatts -> watts)
    if let Some(power) = read_i64(&supply_path.join("power_now")) {
        metrics
            .power_watts
            .with_label_values(&[supply_name])
            .set(prometheus_i64(power) / 1_000_000.0);
    }

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

    // Temperature (tenths of degree Celsius -> Celsius)
    if let Some(temp) = read_i64(&supply_path.join("temp")) {
        metrics
            .temperature_celsius
            .with_label_values(&[supply_name])
            .set(prometheus_i64(temp) / 10.0);
    }
}

pub fn update_metrics() {
    let base = Path::new("/sys/class/power_supply");
    let entries = match fs::read_dir(base) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    // Batteries and USB supplies are hot-pluggable.
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

    for entry in entries.flatten() {
        let name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(_) => continue,
        };

        let path = match fs::canonicalize(entry.path()) {
            Ok(p) => p,
            Err(_) => continue,
        };

        update_power_supply(&path, &name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_battery(dir: &Path, name: &str, capacity: i64, status: &str) -> std::path::PathBuf {
        let supply_dir = dir.join(name);
        fs::create_dir_all(&supply_dir).unwrap();
        fs::write(supply_dir.join("type"), "Battery\n").unwrap();
        fs::write(supply_dir.join("capacity"), format!("{}\n", capacity)).unwrap();
        fs::write(supply_dir.join("status"), format!("{}\n", status)).unwrap();
        supply_dir
    }

    fn create_ac_adapter(dir: &Path, name: &str, online: i64) -> std::path::PathBuf {
        let supply_dir = dir.join(name);
        fs::create_dir_all(&supply_dir).unwrap();
        fs::write(supply_dir.join("type"), "Mains\n").unwrap();
        fs::write(supply_dir.join("online"), format!("{}\n", online)).unwrap();
        supply_dir
    }

    #[test]
    fn test_read_string_trims_whitespace() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("type");
        fs::write(&file, "  Battery  \n").unwrap();
        assert_eq!(read_string(&file), Some("Battery".to_string()));
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
        assert_eq!(read_i64(&file), Some(-500000));
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
}
