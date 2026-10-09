use crate::collection::CollectionReport;
use crate::config::AppConfig;
use crate::metric_support::{RegisterMetricResultExt, prometheus_u64};
use crate::probe::TraceResult;
use crate::runtime::debug_enabled;
use prometheus::{Gauge, GaugeVec};
use serde::Deserialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::process::Command;
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PATH_STATES: &[&str] = &["direct", "peer_relay", "derp", "offline", "unknown"];

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct StatusPeer {
    #[serde(rename = "HostName")]
    host_name: String,
    #[serde(rename = "DNSName")]
    dns_name: String,
    #[serde(rename = "OS")]
    os: String,
    #[serde(rename = "TailscaleIPs")]
    tailscale_ips: Vec<String>,
    #[serde(rename = "CurAddr")]
    cur_addr: String,
    #[serde(rename = "Relay")]
    relay: String,
    #[serde(rename = "PeerRelay")]
    peer_relay: String,
    #[serde(rename = "Online")]
    online: bool,
    #[serde(rename = "Active")]
    active: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct TailscaleStatus {
    #[serde(rename = "BackendState")]
    backend_state: String,
    #[serde(rename = "Self")]
    self_node: StatusPeer,
    #[serde(rename = "Peer")]
    peers: HashMap<String, StatusPeer>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct Netcheck {
    #[serde(rename = "PreferredDERP")]
    preferred_derp: u64,
    #[serde(rename = "RegionLatency")]
    region_latency: HashMap<String, u64>,
    #[serde(rename = "RegionV4Latency")]
    region_v4_latency: HashMap<String, u64>,
    #[serde(rename = "RegionV6Latency")]
    region_v6_latency: HashMap<String, u64>,
    #[serde(rename = "GlobalV4")]
    global_v4: String,
    #[serde(rename = "GlobalV6")]
    global_v6: String,
}

#[derive(Clone, Debug)]
struct PingSnapshot {
    kind: &'static str,
    success: bool,
    rtt_seconds: Option<f64>,
}

#[derive(Clone, Debug)]
struct PeerSnapshot {
    identity: String,
    peer: StatusPeer,
    pings: Vec<PingSnapshot>,
    trace: Option<Result<TraceResult, String>>,
}

#[derive(Clone, Debug)]
struct TailscaleSnapshot {
    status: TailscaleStatus,
    netcheck: Result<Netcheck, String>,
    peers: Vec<PeerSnapshot>,
}

#[derive(Default)]
struct TailscaleCache {
    refreshing: bool,
    refreshed: Option<Instant>,
    refreshed_unix_seconds: Option<f64>,
    snapshot: Option<Result<TailscaleSnapshot, String>>,
}

#[derive(Clone)]
struct Settings {
    tailscale_binary: String,
    mtr_binary: String,
    timeout: Duration,
    ping_timeout: Duration,
    traceroute_timeout: Duration,
    traceroute_max_hops: u32,
    probe_tsmp: bool,
    probe_icmp: bool,
    external_traceroute: bool,
}

impl Settings {
    fn from_config(config: &AppConfig) -> Self {
        Self {
            tailscale_binary: config.tailscale_binary.clone(),
            mtr_binary: config.probe_mtr_binary.clone(),
            timeout: Duration::from_millis(config.tailscale_timeout_ms),
            ping_timeout: Duration::from_millis(config.tailscale_ping_timeout_ms),
            traceroute_timeout: Duration::from_millis(config.probe_traceroute_timeout_ms),
            traceroute_max_hops: config.probe_traceroute_max_hops,
            probe_tsmp: config.tailscale_probe_tsmp,
            probe_icmp: config.tailscale_probe_icmp,
            external_traceroute: config.tailscale_external_traceroute,
        }
    }
}

struct TailscaleMetrics {
    backend_state_info: GaugeVec,
    self_info: GaugeVec,
    public_endpoint_info: GaugeVec,
    public_endpoint_port: GaugeVec,
    preferred_derp: GaugeVec,
    derp_latency_seconds: GaugeVec,
    peer_info: GaugeVec,
    peer_online: GaugeVec,
    peer_active: GaugeVec,
    peer_path_state: GaugeVec,
    peer_endpoint_info: GaugeVec,
    peer_endpoint_port: GaugeVec,
    peer_ping_success: GaugeVec,
    peer_ping_rtt_seconds: GaugeVec,
    peer_external_traceroute_success: GaugeVec,
    peer_external_traceroute_reached: GaugeVec,
    peer_external_traceroute_hops: GaugeVec,
    peer_external_traceroute_hop_rtt_seconds: GaugeVec,
    peer_external_traceroute_hop_info: GaugeVec,
    last_refresh_timestamp_seconds: Gauge,
    refresh_in_progress: Gauge,
}

fn register_status_metrics() -> (GaugeVec, GaugeVec, GaugeVec, GaugeVec, GaugeVec, GaugeVec) {
    (
        prometheus::register_gauge_vec!(
            "tailscale_backend_state_info",
            "Current tailscaled backend state",
            &["state"]
        )
        .or_exit("tailscale_backend_state_info"),
        prometheus::register_gauge_vec!(
            "tailscale_self_info",
            "Local Tailscale node identity",
            &["hostname", "dns_name", "relay", "tailscale_ip"]
        )
        .or_exit("tailscale_self_info"),
        prometheus::register_gauge_vec!(
            "tailscale_public_endpoint_info",
            "Public endpoint address discovered by Tailscale netcheck",
            &["family", "address"]
        )
        .or_exit("tailscale_public_endpoint_info"),
        prometheus::register_gauge_vec!(
            "tailscale_public_endpoint_port",
            "Public endpoint UDP port discovered by Tailscale netcheck",
            &["family"]
        )
        .or_exit("tailscale_public_endpoint_port"),
        prometheus::register_gauge_vec!(
            "tailscale_preferred_derp",
            "Preferred DERP region ID",
            &["region"]
        )
        .or_exit("tailscale_preferred_derp"),
        prometheus::register_gauge_vec!(
            "tailscale_derp_latency_seconds",
            "Tailscale netcheck latency to DERP regions",
            &["region", "family"]
        )
        .or_exit("tailscale_derp_latency_seconds"),
    )
}

fn register_peer_metrics() -> (
    GaugeVec,
    GaugeVec,
    GaugeVec,
    GaugeVec,
    GaugeVec,
    GaugeVec,
    GaugeVec,
    GaugeVec,
) {
    (
        prometheus::register_gauge_vec!(
            "tailscale_peer_info",
            "Tailscale peer identity",
            &[
                "peer",
                "hostname",
                "dns_name",
                "os",
                "relay",
                "tailscale_ip"
            ]
        )
        .or_exit("tailscale_peer_info"),
        prometheus::register_gauge_vec!(
            "tailscale_peer_online",
            "Whether a Tailscale peer is online",
            &["peer"]
        )
        .or_exit("tailscale_peer_online"),
        prometheus::register_gauge_vec!(
            "tailscale_peer_active",
            "Whether a Tailscale peer currently has an active session",
            &["peer"]
        )
        .or_exit("tailscale_peer_active"),
        prometheus::register_gauge_vec!(
            "tailscale_peer_path_state",
            "Current Tailscale peer path classification; exactly one state is 1",
            &["peer", "path"]
        )
        .or_exit("tailscale_peer_path_state"),
        prometheus::register_gauge_vec!(
            "tailscale_peer_endpoint_info",
            "Current direct peer endpoint address when known",
            &["peer", "address"]
        )
        .or_exit("tailscale_peer_endpoint_info"),
        prometheus::register_gauge_vec!(
            "tailscale_peer_endpoint_port",
            "Current direct peer endpoint UDP port when known",
            &["peer"]
        )
        .or_exit("tailscale_peer_endpoint_port"),
        prometheus::register_gauge_vec!(
            "tailscale_peer_ping_success",
            "Whether the most recent Tailscale peer ping succeeded",
            &["peer", "type"]
        )
        .or_exit("tailscale_peer_ping_success"),
        prometheus::register_gauge_vec!(
            "tailscale_peer_ping_rtt_seconds",
            "Round-trip time of the most recent successful Tailscale peer ping",
            &["peer", "type"]
        )
        .or_exit("tailscale_peer_ping_rtt_seconds"),
    )
}

fn register_trace_metrics() -> (GaugeVec, GaugeVec, GaugeVec, GaugeVec, GaugeVec) {
    (
        prometheus::register_gauge_vec!(
            "tailscale_peer_external_traceroute_success",
            "Whether external traceroute to the peer direct endpoint completed successfully",
            &["peer"]
        )
        .or_exit("tailscale_peer_external_traceroute_success"),
        prometheus::register_gauge_vec!(
            "tailscale_peer_external_traceroute_reached",
            "Whether external traceroute reached the peer direct endpoint address",
            &["peer"]
        )
        .or_exit("tailscale_peer_external_traceroute_reached"),
        prometheus::register_gauge_vec!(
            "tailscale_peer_external_traceroute_hops",
            "Number of hop positions reported by external traceroute to a peer",
            &["peer"]
        )
        .or_exit("tailscale_peer_external_traceroute_hops"),
        prometheus::register_gauge_vec!(
            "tailscale_peer_external_traceroute_hop_rtt_seconds",
            "External traceroute RTT by stable hop position",
            &["peer", "hop"]
        )
        .or_exit("tailscale_peer_external_traceroute_hop_rtt_seconds"),
        prometheus::register_gauge_vec!(
            "tailscale_peer_external_traceroute_hop_info",
            "Address observed at an external traceroute hop; intentionally route-churning identity series",
            &["peer", "hop", "address"]
        )
        .or_exit("tailscale_peer_external_traceroute_hop_info"),
    )
}

impl TailscaleMetrics {
    fn new() -> Self {
        let (
            backend_state_info,
            self_info,
            public_endpoint_info,
            public_endpoint_port,
            preferred_derp,
            derp_latency_seconds,
        ) = register_status_metrics();
        let (
            peer_info,
            peer_online,
            peer_active,
            peer_path_state,
            peer_endpoint_info,
            peer_endpoint_port,
            peer_ping_success,
            peer_ping_rtt_seconds,
        ) = register_peer_metrics();
        let (
            peer_external_traceroute_success,
            peer_external_traceroute_reached,
            peer_external_traceroute_hops,
            peer_external_traceroute_hop_rtt_seconds,
            peer_external_traceroute_hop_info,
        ) = register_trace_metrics();
        Self {
            backend_state_info,
            self_info,
            public_endpoint_info,
            public_endpoint_port,
            preferred_derp,
            derp_latency_seconds,
            peer_info,
            peer_online,
            peer_active,
            peer_path_state,
            peer_endpoint_info,
            peer_endpoint_port,
            peer_ping_success,
            peer_ping_rtt_seconds,
            peer_external_traceroute_success,
            peer_external_traceroute_reached,
            peer_external_traceroute_hops,
            peer_external_traceroute_hop_rtt_seconds,
            peer_external_traceroute_hop_info,
            last_refresh_timestamp_seconds: prometheus::register_gauge!(
                "tailscale_last_refresh_timestamp_seconds",
                "Unix timestamp when Tailscale telemetry last completed a background refresh"
            )
            .or_exit("tailscale_last_refresh_timestamp_seconds"),
            refresh_in_progress: prometheus::register_gauge!(
                "tailscale_refresh_in_progress",
                "Whether a Tailscale telemetry background refresh is currently running"
            )
            .or_exit("tailscale_refresh_in_progress"),
        }
    }

    fn reset(&self) {
        self.backend_state_info.reset();
        self.self_info.reset();
        self.public_endpoint_info.reset();
        self.public_endpoint_port.reset();
        self.preferred_derp.reset();
        self.derp_latency_seconds.reset();
        self.peer_info.reset();
        self.peer_online.reset();
        self.peer_active.reset();
        self.peer_path_state.reset();
        self.peer_endpoint_info.reset();
        self.peer_endpoint_port.reset();
        self.peer_ping_success.reset();
        self.peer_ping_rtt_seconds.reset();
        self.peer_external_traceroute_success.reset();
        self.peer_external_traceroute_reached.reset();
        self.peer_external_traceroute_hops.reset();
        self.peer_external_traceroute_hop_rtt_seconds.reset();
        self.peer_external_traceroute_hop_info.reset();
    }
}

static TAILSCALE_METRICS: OnceLock<TailscaleMetrics> = OnceLock::new();
static TAILSCALE_CACHE: Mutex<TailscaleCache> = Mutex::new(TailscaleCache {
    refreshing: false,
    refreshed: None,
    refreshed_unix_seconds: None,
    snapshot: None,
});

fn metrics() -> &'static TailscaleMetrics {
    TAILSCALE_METRICS.get_or_init(TailscaleMetrics::new)
}

fn run_tailscale(settings: &Settings, args: &[&str], description: &str) -> Result<String, String> {
    let mut command = Command::new(&settings.tailscale_binary);
    command.args(args);
    crate::subprocess::run_bounded(command, settings.timeout, 1024 * 1024, description)
}

fn parse_ping_rtt(output: &str) -> Option<f64> {
    let (_, tail) = output.rsplit_once(" in ")?;
    let value = tail.trim().strip_suffix("ms")?;
    if value == "<1" {
        return Some(0.001);
    }
    value.parse::<f64>().ok().map(|ms| ms / 1_000.0)
}

fn run_peer_ping(settings: &Settings, target: &str, kind: &'static str) -> PingSnapshot {
    let flag = match kind {
        "tsmp" => "--tsmp",
        "icmp" => "--icmp",
        _ => unreachable!("fixed ping kind"),
    };
    let timeout = format!("{}ms", settings.ping_timeout.as_millis());
    let mut command = Command::new(&settings.tailscale_binary);
    command.args(["ping", flag, "--c", "1", "--timeout", &timeout, target]);
    match crate::subprocess::run_bounded(
        command,
        settings.ping_timeout + Duration::from_secs(1),
        64 * 1024,
        "tailscale peer ping",
    ) {
        Ok(output) => PingSnapshot {
            kind,
            success: true,
            rtt_seconds: parse_ping_rtt(&output),
        },
        Err(error) if error.contains("exited with") || error.contains("timed out") => {
            if debug_enabled() {
                eprintln!("tailscale {kind} ping {target}: {error}");
            }
            PingSnapshot {
                kind,
                success: false,
                rtt_seconds: None,
            }
        }
        Err(error) => {
            if debug_enabled() {
                eprintln!("tailscale {kind} ping {target}: {error}");
            }
            PingSnapshot {
                kind,
                success: false,
                rtt_seconds: None,
            }
        }
    }
}

fn peer_probe_target(peer: &StatusPeer) -> Option<&str> {
    peer.tailscale_ips
        .iter()
        .find(|address| address.parse::<std::net::Ipv4Addr>().is_ok())
        .or_else(|| peer.tailscale_ips.first())
        .map(String::as_str)
}

fn peer_identity(peer: &StatusPeer) -> String {
    let dns = peer.dns_name.trim_end_matches('.');
    if dns.is_empty() {
        peer.host_name.clone()
    } else {
        dns.to_string()
    }
}

fn endpoint(endpoint: &str) -> Option<(String, u16)> {
    SocketAddr::from_str(endpoint)
        .ok()
        .map(|socket| (socket.ip().to_string(), socket.port()))
}

fn collect_peer(settings: &Settings, peer: StatusPeer) -> PeerSnapshot {
    let identity = peer_identity(&peer);
    let mut pings = Vec::new();
    if let Some(target) = peer.online.then(|| peer_probe_target(&peer)).flatten() {
        if settings.probe_tsmp {
            pings.push(run_peer_ping(settings, target, "tsmp"));
        }
        if settings.probe_icmp {
            pings.push(run_peer_ping(settings, target, "icmp"));
        }
    }
    let trace = (settings.external_traceroute && peer.online)
        .then(|| endpoint(&peer.cur_addr))
        .flatten()
        .map(|(address, _)| {
            crate::probe::run_traceroute(
                &settings.mtr_binary,
                &address,
                settings.traceroute_timeout,
                settings.traceroute_max_hops,
            )
        });
    PeerSnapshot {
        identity,
        peer,
        pings,
        trace,
    }
}

fn collect(settings: &Settings) -> Result<TailscaleSnapshot, String> {
    let status_output = run_tailscale(settings, &["status", "--json"], "tailscale status")?;
    let status: TailscaleStatus = serde_json::from_str(&status_output)
        .map_err(|error| format!("parse tailscale status JSON: {error}"))?;
    let netcheck = run_tailscale(
        settings,
        &["netcheck", "--format=json"],
        "tailscale netcheck",
    )
    .and_then(|output| {
        serde_json::from_str(&output)
            .map_err(|error| format!("parse tailscale netcheck JSON: {error}"))
    });
    let peers = std::thread::scope(|scope| {
        status
            .peers
            .values()
            .cloned()
            .map(|peer| scope.spawn(move || collect_peer(settings, peer)))
            .filter_map(|handle| handle.join().ok())
            .collect()
    });
    Ok(TailscaleSnapshot {
        status,
        netcheck,
        peers,
    })
}

const fn peer_path(peer: &StatusPeer) -> &'static str {
    if !peer.online {
        "offline"
    } else if !peer.cur_addr.is_empty() {
        "direct"
    } else if !peer.peer_relay.is_empty() {
        "peer_relay"
    } else if peer.active && !peer.relay.is_empty() {
        "derp"
    } else {
        "unknown"
    }
}

fn apply_netcheck(metrics: &TailscaleMetrics, netcheck: &Netcheck) {
    for (family, value) in [("v4", &netcheck.global_v4), ("v6", &netcheck.global_v6)] {
        if let Some((address, port)) = endpoint(value) {
            metrics
                .public_endpoint_info
                .with_label_values(&[family, &address])
                .set(1.0);
            metrics
                .public_endpoint_port
                .with_label_values(&[family])
                .set(f64::from(port));
        }
    }
    if netcheck.preferred_derp != 0 {
        let region = netcheck.preferred_derp.to_string();
        metrics
            .preferred_derp
            .with_label_values(&[&region])
            .set(1.0);
    }
    for (family, values) in [
        ("any", &netcheck.region_latency),
        ("v4", &netcheck.region_v4_latency),
        ("v6", &netcheck.region_v6_latency),
    ] {
        for (region, nanos) in values {
            metrics
                .derp_latency_seconds
                .with_label_values(&[region, family])
                .set(prometheus_u64(*nanos) / 1_000_000_000.0);
        }
    }
}

fn apply_trace(metrics: &TailscaleMetrics, peer: &str, trace: &TraceResult) {
    metrics
        .peer_external_traceroute_success
        .with_label_values(&[peer])
        .set(if trace.success { 1.0 } else { 0.0 });
    metrics
        .peer_external_traceroute_reached
        .with_label_values(&[peer])
        .set(if trace.reached { 1.0 } else { 0.0 });
    metrics
        .peer_external_traceroute_hops
        .with_label_values(&[peer])
        .set(f64::from(
            u32::try_from(trace.hops.len()).unwrap_or(u32::MAX),
        ));
    for hop in &trace.hops {
        let hop_label = hop.hop.to_string();
        if let Some(rtt) = hop.rtt_seconds {
            metrics
                .peer_external_traceroute_hop_rtt_seconds
                .with_label_values(&[peer, &hop_label])
                .set(rtt);
        }
        if let Some(address) = &hop.address {
            metrics
                .peer_external_traceroute_hop_info
                .with_label_values(&[peer, &hop_label, address])
                .set(1.0);
        }
    }
}

fn apply(snapshot: &TailscaleSnapshot, refreshed_unix_seconds: f64) -> CollectionReport {
    let metrics = metrics();
    metrics.reset();
    metrics
        .backend_state_info
        .with_label_values(&[&snapshot.status.backend_state])
        .set(1.0);
    let self_node = &snapshot.status.self_node;
    metrics
        .self_info
        .with_label_values(&[
            &self_node.host_name,
            self_node.dns_name.trim_end_matches('.'),
            &self_node.relay,
            peer_probe_target(self_node).unwrap_or(""),
        ])
        .set(1.0);
    let mut report = CollectionReport::success();
    match &snapshot.netcheck {
        Ok(netcheck) => apply_netcheck(metrics, netcheck),
        Err(error) => {
            report.record_error();
            if debug_enabled() {
                eprintln!("tailscale netcheck: {error}");
            }
        }
    }
    for snapshot in &snapshot.peers {
        let peer = &snapshot.peer;
        let identity = snapshot.identity.as_str();
        metrics
            .peer_info
            .with_label_values(&[
                identity,
                &peer.host_name,
                peer.dns_name.trim_end_matches('.'),
                &peer.os,
                &peer.relay,
                peer_probe_target(peer).unwrap_or(""),
            ])
            .set(1.0);
        metrics
            .peer_online
            .with_label_values(&[identity])
            .set(if peer.online { 1.0 } else { 0.0 });
        metrics
            .peer_active
            .with_label_values(&[identity])
            .set(if peer.active { 1.0 } else { 0.0 });
        let selected_path = peer_path(peer);
        for path in PATH_STATES {
            metrics
                .peer_path_state
                .with_label_values(&[identity, path])
                .set(if *path == selected_path { 1.0 } else { 0.0 });
        }
        if let Some((address, port)) = endpoint(&peer.cur_addr) {
            metrics
                .peer_endpoint_info
                .with_label_values(&[identity, &address])
                .set(1.0);
            metrics
                .peer_endpoint_port
                .with_label_values(&[identity])
                .set(f64::from(port));
        }
        for ping in &snapshot.pings {
            metrics
                .peer_ping_success
                .with_label_values(&[identity, ping.kind])
                .set(if ping.success { 1.0 } else { 0.0 });
            if let Some(rtt) = ping.rtt_seconds {
                metrics
                    .peer_ping_rtt_seconds
                    .with_label_values(&[identity, ping.kind])
                    .set(rtt);
            }
        }
        if let Some(trace) = &snapshot.trace {
            match trace {
                Ok(trace) => apply_trace(metrics, identity, trace),
                Err(error) => {
                    report.record_error();
                    if debug_enabled() {
                        eprintln!("tailscale external traceroute {identity}: {error}");
                    }
                }
            }
        }
    }
    metrics
        .last_refresh_timestamp_seconds
        .set(refreshed_unix_seconds);
    report
}

fn start_refresh(settings: Settings) {
    std::thread::spawn(move || {
        let snapshot = collect(&settings);
        let refreshed_unix_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |duration| duration.as_secs_f64());
        let mut cache = TAILSCALE_CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.snapshot = Some(snapshot);
        cache.refreshed = Some(Instant::now());
        cache.refreshed_unix_seconds = Some(refreshed_unix_seconds);
        cache.refreshing = false;
    });
}

pub fn update_metrics(config: &AppConfig) -> CollectionReport {
    let interval = Duration::from_secs(config.tailscale_interval_seconds);
    let (snapshot, refreshed_unix_seconds, refreshing, should_refresh) = {
        let mut cache = TAILSCALE_CACHE
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
        start_refresh(Settings::from_config(config));
    }
    metrics()
        .refresh_in_progress
        .set(if refreshing { 1.0 } else { 0.0 });
    match (snapshot, refreshed_unix_seconds) {
        (Some(Ok(snapshot)), Some(refreshed)) => apply(&snapshot, refreshed),
        (Some(Err(error)), _) => {
            if debug_enabled() {
                eprintln!("tailscale: {error}");
            }
            CollectionReport::error()
        }
        _ => CollectionReport::success(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ping_rtt() {
        assert_eq!(
            parse_ping_rtt("pong from host (100.64.0.1) via TSMP in 40ms\n"),
            Some(0.04)
        );
    }

    #[test]
    fn prefers_tailscale_ipv4_for_peer_identity_join() {
        let peer = StatusPeer {
            tailscale_ips: vec![
                "fd7a:115c:a1e0::bd01:835a".to_string(),
                "100.101.0.1".to_string(),
            ],
            ..Default::default()
        };
        assert_eq!(peer_probe_target(&peer), Some("100.101.0.1"));
    }

    #[test]
    fn classifies_peer_paths() {
        let direct = StatusPeer {
            online: true,
            cur_addr: "1.2.3.4:41641".to_string(),
            relay: "waw".to_string(),
            ..Default::default()
        };
        assert_eq!(peer_path(&direct), "direct");
        let peer_relay = StatusPeer {
            online: true,
            peer_relay: "relay".to_string(),
            relay: "waw".to_string(),
            ..Default::default()
        };
        assert_eq!(peer_path(&peer_relay), "peer_relay");
        let derp = StatusPeer {
            online: true,
            active: true,
            relay: "waw".to_string(),
            ..Default::default()
        };
        assert_eq!(peer_path(&derp), "derp");
        let idle = StatusPeer {
            online: true,
            relay: "waw".to_string(),
            ..Default::default()
        };
        assert_eq!(peer_path(&idle), "unknown");
        let offline = StatusPeer {
            relay: "waw".to_string(),
            ..Default::default()
        };
        assert_eq!(peer_path(&offline), "offline");
    }

    #[test]
    fn parses_ipv4_and_ipv6_endpoints_without_port_churn_in_identity() {
        assert_eq!(
            endpoint("77.236.10.63:33669"),
            Some(("77.236.10.63".to_string(), 33_669))
        );
        assert_eq!(
            endpoint("[2a02:a317:e3c7::1]:54959"),
            Some(("2a02:a317:e3c7::1".to_string(), 54_959))
        );
    }
}
