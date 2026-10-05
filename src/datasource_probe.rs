use crate::collection::CollectionReport;
use crate::config::{AppConfig, ProbeTarget};
use crate::metric_support::RegisterMetricResultExt;
use crate::probe::{PingResult, TraceResult};
use crate::runtime::debug_enabled;
use prometheus::GaugeVec;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
struct TargetSnapshot {
    name: String,
    ping: Option<Result<PingResult, String>>,
    trace: Option<Result<TraceResult, String>>,
}

#[derive(Default)]
struct ProbeCache {
    refreshing: bool,
    refreshed: Option<Instant>,
    refreshed_unix_seconds: Option<f64>,
    snapshot: Vec<TargetSnapshot>,
}

#[derive(Clone)]
struct ProbeSettings {
    targets: Vec<ProbeTarget>,
    ping_binary: String,
    mtr_binary: String,
    ping_timeout: Duration,
    traceroute_timeout: Duration,
    traceroute_max_hops: u32,
}

impl ProbeSettings {
    fn from_config(config: &AppConfig) -> Self {
        Self {
            targets: config.probe_targets.clone(),
            ping_binary: config.probe_ping_binary.clone(),
            mtr_binary: config.probe_mtr_binary.clone(),
            ping_timeout: Duration::from_millis(config.probe_timeout_ms),
            traceroute_timeout: Duration::from_millis(config.probe_traceroute_timeout_ms),
            traceroute_max_hops: config.probe_traceroute_max_hops,
        }
    }
}

struct ProbeMetrics {
    ping_success: GaugeVec,
    ping_rtt_seconds: GaugeVec,
    traceroute_success: GaugeVec,
    traceroute_reached: GaugeVec,
    traceroute_hops: GaugeVec,
    traceroute_hop_rtt_seconds: GaugeVec,
    traceroute_hop_info: GaugeVec,
    last_refresh_timestamp_seconds: GaugeVec,
    refresh_in_progress: prometheus::Gauge,
}

impl ProbeMetrics {
    fn new() -> Self {
        Self {
            ping_success: prometheus::register_gauge_vec!(
                "probe_ping_success",
                "Whether the most recent configured ping probe succeeded",
                &["target"]
            )
            .or_exit("probe_ping_success"),
            ping_rtt_seconds: prometheus::register_gauge_vec!(
                "probe_ping_rtt_seconds",
                "Round-trip time of the most recent successful configured ping probe",
                &["target"]
            )
            .or_exit("probe_ping_rtt_seconds"),
            traceroute_success: prometheus::register_gauge_vec!(
                "probe_traceroute_success",
                "Whether the traceroute command completed and returned parseable results",
                &["target"]
            )
            .or_exit("probe_traceroute_success"),
            traceroute_reached: prometheus::register_gauge_vec!(
                "probe_traceroute_reached",
                "Whether the most recent traceroute reached the configured destination",
                &["target"]
            )
            .or_exit("probe_traceroute_reached"),
            traceroute_hops: prometheus::register_gauge_vec!(
                "probe_traceroute_hops",
                "Number of hop positions reported by the most recent traceroute",
                &["target"]
            )
            .or_exit("probe_traceroute_hops"),
            traceroute_hop_rtt_seconds: prometheus::register_gauge_vec!(
                "probe_traceroute_hop_rtt_seconds",
                "Round-trip time by stable traceroute hop position",
                &["target", "hop"]
            )
            .or_exit("probe_traceroute_hop_rtt_seconds"),
            traceroute_hop_info: prometheus::register_gauge_vec!(
                "probe_traceroute_hop_info",
                "Address observed at a traceroute hop position; this is the intentionally route-churning identity series",
                &["target", "hop", "address"]
            )
            .or_exit("probe_traceroute_hop_info"),
            last_refresh_timestamp_seconds: prometheus::register_gauge_vec!(
                "probe_last_refresh_timestamp_seconds",
                "Unix timestamp when a configured target was last actively probed",
                &["target"]
            )
            .or_exit("probe_last_refresh_timestamp_seconds"),
            refresh_in_progress: prometheus::register_gauge!(
                "probe_refresh_in_progress",
                "Whether a background network-probe refresh is currently running"
            )
            .or_exit("probe_refresh_in_progress"),
        }
    }

    fn reset(&self) {
        self.ping_success.reset();
        self.ping_rtt_seconds.reset();
        self.traceroute_success.reset();
        self.traceroute_reached.reset();
        self.traceroute_hops.reset();
        self.traceroute_hop_rtt_seconds.reset();
        self.traceroute_hop_info.reset();
        self.last_refresh_timestamp_seconds.reset();
    }

    fn apply(&self, snapshot: &[TargetSnapshot], refreshed_at: f64) -> CollectionReport {
        self.reset();
        let mut report = CollectionReport::success();
        for target in snapshot {
            self.last_refresh_timestamp_seconds
                .with_label_values(&[&target.name])
                .set(refreshed_at);
            if let Some(ping) = &target.ping {
                match ping {
                    Ok(ping) => {
                        self.ping_success
                            .with_label_values(&[&target.name])
                            .set(if ping.success { 1.0 } else { 0.0 });
                        if let Some(rtt) = ping.rtt_seconds {
                            self.ping_rtt_seconds
                                .with_label_values(&[&target.name])
                                .set(rtt);
                        }
                    }
                    Err(error) => {
                        report.record_error();
                        if debug_enabled() {
                            eprintln!("probe {} ping: {error}", target.name);
                        }
                    }
                }
            }
            if let Some(trace) = &target.trace {
                match trace {
                    Ok(trace) => {
                        self.traceroute_success
                            .with_label_values(&[&target.name])
                            .set(if trace.success { 1.0 } else { 0.0 });
                        self.traceroute_reached
                            .with_label_values(&[&target.name])
                            .set(if trace.reached { 1.0 } else { 0.0 });
                        self.traceroute_hops
                            .with_label_values(&[&target.name])
                            .set(f64::from(
                                u32::try_from(trace.hops.len()).unwrap_or(u32::MAX),
                            ));
                        for hop in &trace.hops {
                            let hop_label = hop.hop.to_string();
                            if let Some(rtt) = hop.rtt_seconds {
                                self.traceroute_hop_rtt_seconds
                                    .with_label_values(&[&target.name, &hop_label])
                                    .set(rtt);
                            }
                            if let Some(address) = &hop.address {
                                self.traceroute_hop_info
                                    .with_label_values(&[&target.name, &hop_label, address])
                                    .set(1.0);
                            }
                        }
                    }
                    Err(error) => {
                        report.record_error();
                        if debug_enabled() {
                            eprintln!("probe {} traceroute: {error}", target.name);
                        }
                    }
                }
            }
        }
        report
    }
}

static PROBE_METRICS: OnceLock<ProbeMetrics> = OnceLock::new();
static PROBE_CACHE: Mutex<ProbeCache> = Mutex::new(ProbeCache {
    refreshing: false,
    refreshed: None,
    refreshed_unix_seconds: None,
    snapshot: Vec::new(),
});

fn metrics() -> &'static ProbeMetrics {
    PROBE_METRICS.get_or_init(ProbeMetrics::new)
}

fn collect_target(settings: &ProbeSettings, target: &ProbeTarget) -> TargetSnapshot {
    TargetSnapshot {
        name: target.name.clone(),
        ping: target.ping.then(|| {
            crate::probe::run_ping(
                &settings.ping_binary,
                &target.address,
                settings.ping_timeout,
            )
        }),
        trace: target.traceroute.then(|| {
            crate::probe::run_traceroute(
                &settings.mtr_binary,
                &target.address,
                settings.traceroute_timeout,
                settings.traceroute_max_hops,
            )
        }),
    }
}

fn collect(settings: &ProbeSettings) -> Vec<TargetSnapshot> {
    std::thread::scope(|scope| {
        settings
            .targets
            .iter()
            .map(|target| scope.spawn(move || collect_target(settings, target)))
            .filter_map(|handle| handle.join().ok())
            .collect()
    })
}

fn start_refresh(settings: ProbeSettings) {
    std::thread::spawn(move || {
        let snapshot = collect(&settings);
        let refreshed_unix_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |duration| duration.as_secs_f64());
        let mut cache = PROBE_CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.snapshot = snapshot;
        cache.refreshed = Some(Instant::now());
        cache.refreshed_unix_seconds = Some(refreshed_unix_seconds);
        cache.refreshing = false;
    });
}

pub fn update_metrics(config: &AppConfig) -> CollectionReport {
    let interval = Duration::from_secs(config.probe_interval_seconds);
    let (snapshot, refreshed_unix_seconds, refreshing, should_refresh) = {
        let mut cache = PROBE_CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let due = cache
            .refreshed
            .is_none_or(|refreshed| refreshed.elapsed() >= interval);
        let should_refresh = due && !cache.refreshing;
        if should_refresh {
            cache.refreshing = true;
        }
        (
            cache.snapshot.clone(),
            cache.refreshed_unix_seconds,
            cache.refreshing,
            should_refresh,
        )
    };

    if should_refresh {
        start_refresh(ProbeSettings::from_config(config));
    }
    metrics()
        .refresh_in_progress
        .set(if refreshing { 1.0 } else { 0.0 });
    refreshed_unix_seconds.map_or_else(CollectionReport::success, |refreshed| {
        metrics().apply(&snapshot, refreshed)
    })
}
