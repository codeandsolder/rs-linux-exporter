use crate::collection::CollectionReport;
use crate::config::AppConfig;
use crate::metric_support::RegisterMetricResultExt;
use crate::runtime::debug_enabled;
use prometheus::{Gauge, GaugeVec};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

const SOURCE_STATES: &[(&str, &str)] = &[
    ("*", "current"),
    ("+", "combined"),
    ("-", "not_combined"),
    ("x", "error"),
    ("~", "variable"),
    ("?", "unusable"),
];

const SOURCE_MODES: &[(&str, &str)] = &[("^", "server"), ("=", "peer"), ("#", "local")];

#[derive(Debug, Clone, PartialEq)]
struct TrackingSnapshot {
    reference: String,
    stratum: f64,
    reference_time: f64,
    system_offset: f64,
    last_offset: f64,
    rms_offset: f64,
    frequency_ppm: f64,
    residual_frequency_ppm: f64,
    skew_ppm: f64,
    root_delay: f64,
    root_dispersion: f64,
    update_interval: f64,
    leap_status: String,
}

#[derive(Debug, Clone, PartialEq)]
struct SourceSnapshot {
    mode: String,
    state: String,
    source: String,
    stratum: f64,
    poll_seconds: f64,
    reachability: f64,
    last_rx_seconds: f64,
    adjusted_offset_seconds: f64,
    original_offset_seconds: f64,
    error_seconds: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ActivitySnapshot {
    online: f64,
    offline: f64,
    burst_online: f64,
    burst_offline: f64,
    unresolved: f64,
}

struct TrackingMetrics {
    info: GaugeVec,
    stratum: Gauge,
    reference_time_seconds: Gauge,
    system_offset_seconds: Gauge,
    last_offset_seconds: Gauge,
    rms_offset_seconds: Gauge,
    frequency_ppm: Gauge,
    residual_frequency_ppm: Gauge,
    skew_ppm: Gauge,
    root_delay_seconds: Gauge,
    root_dispersion_seconds: Gauge,
    update_interval_seconds: Gauge,
}

struct SourceMetrics {
    info: GaugeVec,
    state: GaugeVec,
    stratum: GaugeVec,
    poll_interval_seconds: GaugeVec,
    reachability: GaugeVec,
    last_rx_seconds: GaugeVec,
    offset_seconds: GaugeVec,
    error_seconds: GaugeVec,
}

struct NtpMetrics {
    tracking: TrackingMetrics,
    activity: GaugeVec,
    source: SourceMetrics,
}

impl TrackingMetrics {
    fn new() -> Self {
        Self {
            info: prometheus::register_gauge_vec!(
                "ntp_tracking_info",
                "Current chrony tracking reference and leap status",
                &["reference", "leap_status"]
            )
            .or_exit("ntp_tracking_info"),
            stratum: prometheus::register_gauge!("ntp_stratum", "Current NTP stratum")
                .or_exit("ntp_stratum"),
            reference_time_seconds: prometheus::register_gauge!(
                "ntp_reference_time_seconds",
                "Unix timestamp of chrony's current reference time"
            )
            .or_exit("ntp_reference_time_seconds"),
            system_offset_seconds: prometheus::register_gauge!(
                "ntp_system_offset_seconds",
                "Current local system-clock offset from NTP time"
            )
            .or_exit("ntp_system_offset_seconds"),
            last_offset_seconds: prometheus::register_gauge!(
                "ntp_last_offset_seconds",
                "Last chrony clock-update offset"
            )
            .or_exit("ntp_last_offset_seconds"),
            rms_offset_seconds: prometheus::register_gauge!(
                "ntp_rms_offset_seconds",
                "Long-term RMS clock offset reported by chrony"
            )
            .or_exit("ntp_rms_offset_seconds"),
            frequency_ppm: prometheus::register_gauge!(
                "ntp_frequency_ppm",
                "System clock frequency error in parts per million"
            )
            .or_exit("ntp_frequency_ppm"),
            residual_frequency_ppm: prometheus::register_gauge!(
                "ntp_residual_frequency_ppm",
                "Residual frequency error in parts per million"
            )
            .or_exit("ntp_residual_frequency_ppm"),
            skew_ppm: prometheus::register_gauge!(
                "ntp_skew_ppm",
                "Estimated frequency error bound in parts per million"
            )
            .or_exit("ntp_skew_ppm"),
            root_delay_seconds: prometheus::register_gauge!(
                "ntp_root_delay_seconds",
                "Network path delay to the NTP stratum-1 reference"
            )
            .or_exit("ntp_root_delay_seconds"),
            root_dispersion_seconds: prometheus::register_gauge!(
                "ntp_root_dispersion_seconds",
                "Estimated maximum clock error accumulated to the stratum-1 reference"
            )
            .or_exit("ntp_root_dispersion_seconds"),
            update_interval_seconds: prometheus::register_gauge!(
                "ntp_update_interval_seconds",
                "Interval between the last two chrony clock updates"
            )
            .or_exit("ntp_update_interval_seconds"),
        }
    }
}

impl SourceMetrics {
    fn new() -> Self {
        Self {
            info: prometheus::register_gauge_vec!(
                "ntp_source_info",
                "Chrony source metadata",
                &["source", "mode"]
            )
            .or_exit("ntp_source_info"),
            state: prometheus::register_gauge_vec!(
                "ntp_source_state",
                "Chrony source selection state; exactly one state is 1 per source",
                &["source", "state"]
            )
            .or_exit("ntp_source_state"),
            stratum: prometheus::register_gauge_vec!(
                "ntp_source_stratum",
                "Reported stratum of each chrony source",
                &["source"]
            )
            .or_exit("ntp_source_stratum"),
            poll_interval_seconds: prometheus::register_gauge_vec!(
                "ntp_source_poll_interval_seconds",
                "Current polling interval for each chrony source",
                &["source"]
            )
            .or_exit("ntp_source_poll_interval_seconds"),
            reachability: prometheus::register_gauge_vec!(
                "ntp_source_reachability",
                "Chrony 8-bit source reachability register as an integer",
                &["source"]
            )
            .or_exit("ntp_source_reachability"),
            last_rx_seconds: prometheus::register_gauge_vec!(
                "ntp_source_last_rx_seconds",
                "Seconds since the last sample from each chrony source",
                &["source"]
            )
            .or_exit("ntp_source_last_rx_seconds"),
            offset_seconds: prometheus::register_gauge_vec!(
                "ntp_source_offset_seconds",
                "Chrony source offset values",
                &["source", "kind"]
            )
            .or_exit("ntp_source_offset_seconds"),
            error_seconds: prometheus::register_gauge_vec!(
                "ntp_source_error_seconds",
                "Estimated source error bound reported by chrony",
                &["source"]
            )
            .or_exit("ntp_source_error_seconds"),
        }
    }
}

impl NtpMetrics {
    fn new() -> Self {
        Self {
            tracking: TrackingMetrics::new(),
            activity: prometheus::register_gauge_vec!(
                "ntp_sources",
                "Number of chrony sources by activity state",
                &["state"]
            )
            .or_exit("ntp_sources"),
            source: SourceMetrics::new(),
        }
    }

    fn clear(&self) {
        self.tracking.info.reset();
        self.activity.reset();
        self.source.info.reset();
        self.source.state.reset();
        self.source.stratum.reset();
        self.source.poll_interval_seconds.reset();
        self.source.reachability.reset();
        self.source.last_rx_seconds.reset();
        self.source.offset_seconds.reset();
        self.source.error_seconds.reset();
    }

    fn apply(
        &self,
        tracking: &TrackingSnapshot,
        sources: &[SourceSnapshot],
        activity: ActivitySnapshot,
    ) {
        self.clear();
        self.tracking
            .info
            .with_label_values(&[&tracking.reference, &tracking.leap_status])
            .set(1.0);
        self.tracking.stratum.set(tracking.stratum);
        self.tracking
            .reference_time_seconds
            .set(tracking.reference_time);
        self.tracking
            .system_offset_seconds
            .set(tracking.system_offset);
        self.tracking.last_offset_seconds.set(tracking.last_offset);
        self.tracking.rms_offset_seconds.set(tracking.rms_offset);
        self.tracking.frequency_ppm.set(tracking.frequency_ppm);
        self.tracking
            .residual_frequency_ppm
            .set(tracking.residual_frequency_ppm);
        self.tracking.skew_ppm.set(tracking.skew_ppm);
        self.tracking.root_delay_seconds.set(tracking.root_delay);
        self.tracking
            .root_dispersion_seconds
            .set(tracking.root_dispersion);
        self.tracking
            .update_interval_seconds
            .set(tracking.update_interval);
        for (state, value) in [
            ("online", activity.online),
            ("offline", activity.offline),
            ("burst_online", activity.burst_online),
            ("burst_offline", activity.burst_offline),
            ("unresolved", activity.unresolved),
        ] {
            self.activity.with_label_values(&[state]).set(value);
        }
        for source in sources {
            self.source
                .info
                .with_label_values(&[source.source.as_str(), source.mode.as_str()])
                .set(1.0);
            for (_, state) in SOURCE_STATES {
                self.source
                    .state
                    .with_label_values(&[source.source.as_str(), state])
                    .set(if *state == source.state { 1.0 } else { 0.0 });
            }
            self.source
                .stratum
                .with_label_values(&[source.source.as_str()])
                .set(source.stratum);
            self.source
                .poll_interval_seconds
                .with_label_values(&[source.source.as_str()])
                .set(source.poll_seconds);
            self.source
                .reachability
                .with_label_values(&[source.source.as_str()])
                .set(source.reachability);
            self.source
                .last_rx_seconds
                .with_label_values(&[source.source.as_str()])
                .set(source.last_rx_seconds);
            self.source
                .offset_seconds
                .with_label_values(&[source.source.as_str(), "adjusted"])
                .set(source.adjusted_offset_seconds);
            self.source
                .offset_seconds
                .with_label_values(&[source.source.as_str(), "original"])
                .set(source.original_offset_seconds);
            self.source
                .error_seconds
                .with_label_values(&[source.source.as_str()])
                .set(source.error_seconds);
        }
    }
}

static NTP_METRICS: OnceLock<NtpMetrics> = OnceLock::new();

fn metrics() -> &'static NtpMetrics {
    NTP_METRICS.get_or_init(NtpMetrics::new)
}

fn parse_f64(value: &str, field: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .map_err(|error| format!("invalid chrony {field} value {value:?}: {error}"))
}

fn parse_tracking(line: &str) -> Result<TrackingSnapshot, String> {
    let fields: Vec<_> = line.trim().split(',').collect();
    if fields.len() != 14 {
        return Err(format!(
            "chrony tracking CSV had {} fields, expected 14",
            fields.len()
        ));
    }
    Ok(TrackingSnapshot {
        reference: fields[1].to_string(),
        stratum: parse_f64(fields[2], "stratum")?,
        reference_time: parse_f64(fields[3], "reference time")?,
        system_offset: parse_f64(fields[4], "system offset")?,
        last_offset: parse_f64(fields[5], "last offset")?,
        rms_offset: parse_f64(fields[6], "RMS offset")?,
        frequency_ppm: parse_f64(fields[7], "frequency")?,
        residual_frequency_ppm: parse_f64(fields[8], "residual frequency")?,
        skew_ppm: parse_f64(fields[9], "skew")?,
        root_delay: parse_f64(fields[10], "root delay")?,
        root_dispersion: parse_f64(fields[11], "root dispersion")?,
        update_interval: parse_f64(fields[12], "update interval")?,
        leap_status: fields[13].to_ascii_lowercase().replace(' ', "_"),
    })
}

fn source_mode(value: &str) -> Result<&'static str, String> {
    SOURCE_MODES
        .iter()
        .find_map(|(symbol, name)| (*symbol == value).then_some(*name))
        .ok_or_else(|| format!("unknown chrony source mode {value:?}"))
}

fn source_state(value: &str) -> Result<&'static str, String> {
    SOURCE_STATES
        .iter()
        .find_map(|(symbol, name)| (*symbol == value).then_some(*name))
        .ok_or_else(|| format!("unknown chrony source state {value:?}"))
}

fn parse_sources(contents: &str) -> Result<Vec<SourceSnapshot>, String> {
    let mut sources = Vec::new();
    for (index, line) in contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
    {
        let fields: Vec<_> = line.trim().split(',').collect();
        if fields.len() != 10 {
            return Err(format!(
                "chrony sources CSV line {} had {} fields, expected 10",
                index + 1,
                fields.len()
            ));
        }
        let poll = fields[4]
            .parse::<i32>()
            .map_err(|error| format!("invalid chrony poll exponent {:?}: {error}", fields[4]))?;
        let reach = u8::from_str_radix(fields[5], 8)
            .map_err(|error| format!("invalid chrony reachability {:?}: {error}", fields[5]))?;
        sources.push(SourceSnapshot {
            mode: source_mode(fields[0])?.to_string(),
            state: source_state(fields[1])?.to_string(),
            source: fields[2].to_string(),
            stratum: parse_f64(fields[3], "source stratum")?,
            poll_seconds: 2_f64.powi(poll),
            reachability: f64::from(reach),
            last_rx_seconds: parse_f64(fields[6], "source last-rx")?,
            adjusted_offset_seconds: parse_f64(fields[7], "source adjusted offset")?,
            original_offset_seconds: parse_f64(fields[8], "source original offset")?,
            error_seconds: parse_f64(fields[9], "source error")?,
        });
    }
    Ok(sources)
}

fn parse_activity(line: &str) -> Result<ActivitySnapshot, String> {
    let fields: Vec<_> = line.trim().split(',').collect();
    if fields.len() != 5 {
        return Err(format!(
            "chrony activity CSV had {} fields, expected 5",
            fields.len()
        ));
    }
    let mut values = [0.0; 5];
    for (index, field) in fields.iter().enumerate() {
        values[index] = parse_f64(field, "activity")?;
    }
    Ok(ActivitySnapshot {
        online: values[0],
        offline: values[1],
        burst_online: values[2],
        burst_offline: values[3],
        unresolved: values[4],
    })
}

fn run_chronyc(config: &AppConfig, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new(&config.ntp_binary);
    command.args(["-n", "-c"]).args(args);
    crate::subprocess::run_bounded(
        command,
        Duration::from_millis(config.ntp_timeout_ms),
        256 * 1024,
        "chronyc command",
    )
}

fn collect(
    config: &AppConfig,
) -> Result<(TrackingSnapshot, Vec<SourceSnapshot>, ActivitySnapshot), String> {
    let tracking = parse_tracking(&run_chronyc(config, &["tracking"])?)?;
    let sources = parse_sources(&run_chronyc(config, &["sources"])?)?;
    let activity = parse_activity(&run_chronyc(config, &["activity"])?)?;
    Ok((tracking, sources, activity))
}

pub fn update_metrics(config: &AppConfig) -> CollectionReport {
    match collect(config) {
        Ok((tracking, sources, activity)) => {
            metrics().apply(&tracking, &sources, activity);
            CollectionReport::success()
        }
        Err(error) => {
            if debug_enabled() {
                eprintln!("ntp: {error}");
            }
            if let Some(metrics) = NTP_METRICS.get() {
                metrics.clear();
            }
            CollectionReport::error()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tracking_csv() {
        let snapshot = parse_tracking("ED11CC5F,2001:638:610:be01::108,2,1791228835.204672024,-0.001176222,0.000139906,0.000839097,-19.968,0.004,0.198,0.036162294,0.007547604,1030.2,Normal").unwrap();
        assert_eq!(snapshot.reference, "2001:638:610:be01::108");
        assert_eq!(snapshot.stratum, 2.0);
        assert_eq!(snapshot.system_offset, -0.001_176_222);
        assert_eq!(snapshot.leap_status, "normal");
    }

    #[test]
    fn parses_sources_and_octal_reachability() {
        let sources = parse_sources(
            "^,*,2001:638:610:be01::108,1,10,377,1027,-0.009238857,-0.009098952,0.040010810\n",
        )
        .unwrap();
        assert_eq!(sources.len(), 1);
        let source = &sources[0];
        assert_eq!(source.mode, "server");
        assert_eq!(source.state, "current");
        assert_eq!(source.reachability, 255.0);
        assert_eq!(source.poll_seconds, 1024.0);
    }

    #[test]
    fn parses_activity_csv() {
        assert_eq!(
            parse_activity("3,0,1,2,4").unwrap(),
            ActivitySnapshot {
                online: 3.0,
                offline: 0.0,
                burst_online: 1.0,
                burst_offline: 2.0,
                unresolved: 4.0,
            }
        );
    }

    #[test]
    fn rejects_unknown_source_states() {
        assert!(parse_sources("^,!,1.2.3.4,1,10,377,1,0,0,0\n").is_err());
    }
}
