use crate::collection::CollectionReport;
use crate::metric_support::RegisterMetricResultExt;
use prometheus::GaugeVec;
use std::fs;
use std::sync::OnceLock;

struct WirelessMetrics {
    link_quality: GaugeVec,
    signal_dbm: GaugeVec,
    noise_dbm: GaugeVec,
    discarded_packets: GaugeVec,
    missed_beacons: GaugeVec,
}

impl WirelessMetrics {
    fn new() -> Self {
        Self {
            link_quality: prometheus::register_gauge_vec!(
                "wireless_link_quality",
                "Kernel wireless link quality value from /proc/net/wireless.",
                &["interface"]
            )
            .or_exit("wireless_link_quality"),
            signal_dbm: prometheus::register_gauge_vec!(
                "wireless_signal_dbm",
                "Wireless signal level in dBm.",
                &["interface"]
            )
            .or_exit("wireless_signal_dbm"),
            noise_dbm: prometheus::register_gauge_vec!(
                "wireless_noise_dbm",
                "Wireless noise level in dBm when reported by the driver.",
                &["interface"]
            )
            .or_exit("wireless_noise_dbm"),
            discarded_packets: prometheus::register_gauge_vec!(
                "wireless_discarded_packets",
                "Kernel cumulative wireless discarded-packet counters.",
                &["interface", "reason"]
            )
            .or_exit("wireless_discarded_packets"),
            missed_beacons: prometheus::register_gauge_vec!(
                "wireless_missed_beacons",
                "Kernel cumulative missed wireless beacon count.",
                &["interface"]
            )
            .or_exit("wireless_missed_beacons"),
        }
    }

    fn reset(&self) {
        self.link_quality.reset();
        self.signal_dbm.reset();
        self.noise_dbm.reset();
        self.discarded_packets.reset();
        self.missed_beacons.reset();
    }
}

static METRICS: OnceLock<WirelessMetrics> = OnceLock::new();

fn metrics() -> &'static WirelessMetrics {
    METRICS.get_or_init(WirelessMetrics::new)
}

fn parse_number(value: &str) -> Option<f64> {
    value.trim_end_matches('.').parse::<f64>().ok()
}

fn parse_line(metrics: &WirelessMetrics, line: &str) -> bool {
    let Some((interface, rest)) = line.split_once(':') else {
        return false;
    };
    let interface = interface.trim();
    if interface.is_empty() {
        return false;
    }
    let fields = rest.split_whitespace().collect::<Vec<_>>();
    if fields.len() < 10 {
        return false;
    }
    let Some(link) = parse_number(fields[1]) else {
        return false;
    };
    let Some(signal) = parse_number(fields[2]) else {
        return false;
    };
    metrics
        .link_quality
        .with_label_values(&[interface])
        .set(link);
    metrics
        .signal_dbm
        .with_label_values(&[interface])
        .set(signal);

    if let Some(noise) = parse_number(fields[3])
        && noise > -255.0
    {
        metrics.noise_dbm.with_label_values(&[interface]).set(noise);
    }

    for (reason, index) in [
        ("nwid", 4_usize),
        ("crypt", 5),
        ("frag", 6),
        ("retry", 7),
        ("misc", 8),
    ] {
        if let Some(value) = parse_number(fields[index]) {
            metrics
                .discarded_packets
                .with_label_values(&[interface, reason])
                .set(value);
        }
    }
    if let Some(value) = parse_number(fields[9]) {
        metrics
            .missed_beacons
            .with_label_values(&[interface])
            .set(value);
    }
    true
}

pub fn update_metrics() -> CollectionReport {
    let Ok(contents) = fs::read_to_string("/proc/net/wireless") else {
        return CollectionReport::success();
    };
    let metrics = metrics();
    metrics.reset();
    for line in contents.lines().skip(2) {
        let _ = parse_line(metrics, line);
    }
    CollectionReport::success()
}

#[cfg(test)]
mod tests {
    use super::parse_number;

    #[test]
    fn parses_wireless_decimal_suffix() {
        assert_eq!(parse_number("70."), Some(70.0));
        assert_eq!(parse_number("-36."), Some(-36.0));
    }
}
