use crate::config::AppConfig;
use crate::metric_support::RegisterMetricResultExt;
use crate::metric_support::prometheus_i64;
use prometheus::GaugeVec;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

const SYS_CLASS_NET: &str = "/sys/class/net";
const OPERSTATES: [&str; 7] = [
    "unknown",
    "notpresent",
    "down",
    "lowerlayerdown",
    "testing",
    "dormant",
    "up",
];
const DUPLEX_STATES: [&str; 3] = ["unknown", "half", "full"];
const AUTONEG_STATES: [&str; 3] = ["unknown", "off", "on"];

struct NetdevSysfsMetrics {
    operstate: GaugeVec,
    carrier: GaugeVec,
    carrier_changes: GaugeVec,
    dormant: GaugeVec,
    speed_mbps: GaugeVec,
    duplex: GaugeVec,
    autoneg: GaugeVec,
}

impl NetdevSysfsMetrics {
    fn new() -> Self {
        Self {
            operstate: prometheus::register_gauge_vec!(
                "netdev_operstate",
                "Network interface operational state (1 for current state)",
                &["interface", "state"]
            )
            .or_exit("netdev_operstate"),
            carrier: prometheus::register_gauge_vec!(
                "netdev_carrier",
                "Network interface carrier status (1 = link detected)",
                &["interface"]
            )
            .or_exit("netdev_carrier"),
            carrier_changes: prometheus::register_gauge_vec!(
                "netdev_carrier_changes",
                "Network interface carrier change count",
                &["interface"]
            )
            .or_exit("netdev_carrier_changes"),
            dormant: prometheus::register_gauge_vec!(
                "netdev_dormant",
                "Network interface dormant flag (1 = dormant)",
                &["interface"]
            )
            .or_exit("netdev_dormant"),
            speed_mbps: prometheus::register_gauge_vec!(
                "netdev_speed_mbps",
                "Network interface speed in Mbps",
                &["interface"]
            )
            .or_exit("netdev_speed_mbps"),
            duplex: prometheus::register_gauge_vec!(
                "netdev_duplex",
                "Network interface duplex (1 for current duplex)",
                &["interface", "duplex"]
            )
            .or_exit("netdev_duplex"),
            autoneg: prometheus::register_gauge_vec!(
                "netdev_autoneg",
                "Network interface autonegotiation (1 for current state)",
                &["interface", "state"]
            )
            .or_exit("netdev_autoneg"),
        }
    }
}

static NETDEV_SYSFS_METRICS: OnceLock<NetdevSysfsMetrics> = OnceLock::new();

fn metrics() -> &'static NetdevSysfsMetrics {
    NETDEV_SYSFS_METRICS.get_or_init(NetdevSysfsMetrics::new)
}

fn read_string(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn read_i64(path: &Path) -> Option<i64> {
    read_string(path)?.parse::<i64>().ok()
}

/// Maps the numeric autonegotiation flag onto the exported label values.
///
/// Where sysfs exposes `autoneg` at all it holds 0 or 1, so comparing against
/// "off"/"on" always fell through to "unknown". Drivers that report the words
/// directly are passed through unchanged.
fn normalized_autoneg(value: &str) -> &str {
    match value {
        "0" => "off",
        "1" => "on",
        other => other,
    }
}

fn normalized_state<'a>(value: &'a str, known: &[&'a str]) -> &'a str {
    if known.contains(&value) {
        value
    } else {
        "unknown"
    }
}

fn set_state_metric(metric: &GaugeVec, iface: &str, value: &str, known: &[&str]) {
    let state = normalized_state(value, known);
    for known_state in known {
        metric
            .with_label_values(&[iface, known_state])
            .set(if state == *known_state { 1.0 } else { 0.0 });
    }
}

fn should_skip_interface(name: &str, config: &AppConfig) -> bool {
    if config.ignore_ppp_interfaces && name.starts_with("ppp") {
        return true;
    }
    if config.ignore_veth_interfaces && (name.starts_with("veth") || name.starts_with("br-")) {
        return true;
    }
    false
}

fn update_interface(metrics: &NetdevSysfsMetrics, iface_path: &Path, iface: &str) {
    if let Some(state) =
        read_string(&iface_path.join("operstate")).map(|value| value.to_lowercase())
    {
        set_state_metric(&metrics.operstate, iface, &state, &OPERSTATES);
    }

    if let Some(carrier) = read_i64(&iface_path.join("carrier"))
        && carrier >= 0
    {
        metrics
            .carrier
            .with_label_values(&[iface])
            .set(prometheus_i64(carrier));
    }

    if let Some(changes) = read_i64(&iface_path.join("carrier_changes"))
        && changes >= 0
    {
        metrics
            .carrier_changes
            .with_label_values(&[iface])
            .set(prometheus_i64(changes));
    }

    if let Some(dormant) = read_i64(&iface_path.join("dormant"))
        && dormant >= 0
    {
        metrics
            .dormant
            .with_label_values(&[iface])
            .set(prometheus_i64(dormant));
    }

    if let Some(speed) = read_i64(&iface_path.join("speed"))
        && speed >= 0
    {
        metrics
            .speed_mbps
            .with_label_values(&[iface])
            .set(prometheus_i64(speed));
    }

    if let Some(duplex) = read_string(&iface_path.join("duplex")).map(|value| value.to_lowercase())
    {
        set_state_metric(&metrics.duplex, iface, &duplex, &DUPLEX_STATES);
    }

    if let Some(autoneg) =
        read_string(&iface_path.join("autoneg")).map(|value| value.to_lowercase())
    {
        set_state_metric(
            &metrics.autoneg,
            iface,
            normalized_autoneg(&autoneg),
            &AUTONEG_STATES,
        );
    }
}

pub fn update_metrics(config: &AppConfig) {
    let metrics = metrics();
    // Interfaces are created and destroyed constantly on container hosts.
    metrics.operstate.reset();
    metrics.carrier.reset();
    metrics.carrier_changes.reset();
    metrics.dormant.reset();
    metrics.speed_mbps.reset();
    metrics.duplex.reset();
    metrics.autoneg.reset();

    let Ok(entries) = fs::read_dir(SYS_CLASS_NET) else {
        return;
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if should_skip_interface(&name, config) {
            continue;
        }
        update_interface(metrics, &entry.path(), &name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn autoneg_numeric_flag_maps_to_a_known_state() {
        assert_eq!(normalized_autoneg("0"), "off");
        assert_eq!(normalized_autoneg("1"), "on");
        // Anything else is left for normalized_state to classify.
        assert_eq!(normalized_autoneg("on"), "on");
        assert_eq!(normalized_autoneg("garbage"), "garbage");
    }

    #[test]
    fn autoneg_numeric_flag_is_no_longer_unknown() {
        assert_eq!(
            normalized_state(normalized_autoneg("1"), &AUTONEG_STATES),
            "on"
        );
        assert_eq!(
            normalized_state(normalized_autoneg("garbage"), &AUTONEG_STATES),
            "unknown"
        );
    }

    #[test]
    fn operstate_and_duplex_are_classified() {
        assert_eq!(normalized_state("up", &OPERSTATES), "up");
        assert_eq!(normalized_state("weird", &OPERSTATES), "unknown");
        assert_eq!(normalized_state("full", &DUPLEX_STATES), "full");
    }

    #[test]
    fn update_interface_reads_sysfs_attributes() {
        let dir = TempDir::new().unwrap();
        let iface = dir.path().join("eth0");
        fs::create_dir_all(&iface).unwrap();
        fs::write(iface.join("operstate"), "up\n").unwrap();
        fs::write(iface.join("carrier"), "1\n").unwrap();
        fs::write(iface.join("speed"), "1000\n").unwrap();
        fs::write(iface.join("duplex"), "full\n").unwrap();
        fs::write(iface.join("autoneg"), "1\n").unwrap();

        let metrics = metrics();
        update_interface(metrics, &iface, "eth0");

        assert_eq!(
            metrics.autoneg.with_label_values(&["eth0", "on"]).get(),
            1.0
        );
        assert_eq!(
            metrics.autoneg.with_label_values(&["eth0", "off"]).get(),
            0.0
        );
        assert_eq!(
            metrics.speed_mbps.with_label_values(&["eth0"]).get(),
            1000.0
        );
        assert_eq!(
            metrics.operstate.with_label_values(&["eth0", "up"]).get(),
            1.0
        );
        assert_eq!(
            metrics.operstate.with_label_values(&["eth0", "down"]).get(),
            0.0
        );
    }
}
