use crate::collection::CollectionReport;
use crate::config::AppConfig;
use crate::metric_support::{RegisterMetricResultExt, prometheus_u64};
use crate::runtime::debug_enabled;
use prometheus::{CounterVec, Gauge, GaugeVec};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum CounterFamily {
    Requests,
    CacheRequests,
    CacheEvents,
    Compilations,
    Durations,
    NotCached,
    NotCachedCrateTypes,
    DistCompiles,
    DistEvents,
    CacheLevelOperations,
    CacheLevelDurations,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CounterKey {
    family: CounterFamily,
    labels: Vec<String>,
}

struct CounterSample {
    key: CounterKey,
    value: u64,
    scale: f64,
}

#[derive(Default)]
struct Snapshot {
    counters: Vec<CounterSample>,
    version: String,
    cache_size: Option<u64>,
    max_cache_size: Option<u64>,
    preprocessor_cache_mode: bool,
    basedirs: u64,
    cache_levels: u64,
}

#[derive(Debug, Deserialize)]
struct ServerInfo {
    stats: ServerStats,
    #[serde(default)]
    cache_size: Option<u64>,
    #[serde(default)]
    max_cache_size: Option<u64>,
    #[serde(default)]
    use_preprocessor_cache_mode: bool,
    version: String,
    #[serde(default)]
    basedirs: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PerLanguageCount {
    counts: BTreeMap<String, u64>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default)]
struct JsonDuration {
    secs: u64,
    nanos: u32,
}

impl JsonDuration {
    fn total_nanos(self) -> Result<u64, String> {
        self.secs
            .checked_mul(1_000_000_000)
            .and_then(|seconds| seconds.checked_add(u64::from(self.nanos)))
            .ok_or_else(|| "sccache cumulative duration overflowed u64 nanoseconds".to_string())
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct CacheLevelStats {
    name: String,
    hits: u64,
    misses: u64,
    writes: u64,
    write_failures: u64,
    backfills_from: u64,
    backfills_to: u64,
    hit_duration: JsonDuration,
    write_duration: JsonDuration,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ServerStats {
    compile_requests: u64,
    requests_unsupported_compiler: u64,
    requests_not_compile: u64,
    requests_not_cacheable: u64,
    requests_executed: u64,
    cache_errors: PerLanguageCount,
    cache_hits: PerLanguageCount,
    cache_misses: PerLanguageCount,
    cache_timeouts: u64,
    cache_read_errors: u64,
    non_cacheable_compilations: u64,
    forced_recaches: u64,
    cache_write_errors: u64,
    cache_writes: u64,
    cache_write_duration: JsonDuration,
    cache_read_hit_duration: JsonDuration,
    compilations: u64,
    compiler_write_duration: JsonDuration,
    compile_fails: u64,
    not_cached: BTreeMap<String, u64>,
    not_cached_crate_types: BTreeMap<String, u64>,
    dist_compiles: BTreeMap<String, u64>,
    dist_errors: u64,
    multi_level: Option<Vec<CacheLevelStats>>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default)]
struct SchedulerStatus {
    num_servers: u64,
    num_cpus: u64,
    in_progress: u64,
}

#[derive(Debug, Deserialize)]
enum DistInfo {
    Disabled(String),
    NotConnected(Option<String>, String),
    SchedulerStatus(Option<String>, SchedulerStatus),
}

#[derive(Clone, Copy, Debug, Default)]
struct DistSnapshot {
    enabled: bool,
    connected: bool,
    scheduler: Option<SchedulerStatus>,
}

struct SccacheMetrics {
    requests: CounterVec,
    cache_requests: CounterVec,
    cache_events: CounterVec,
    compilations: CounterVec,
    durations: CounterVec,
    not_cached: CounterVec,
    not_cached_crate_types: CounterVec,
    dist_compiles: CounterVec,
    dist_events: CounterVec,
    cache_level_operations: CounterVec,
    cache_level_durations: CounterVec,
    info: GaugeVec,
    cache_size_bytes: GaugeVec,
    preprocessor_cache_mode: Gauge,
    basedirs: Gauge,
    cache_levels: Gauge,
    dist_status: GaugeVec,
    previous: Mutex<HashMap<CounterKey, u64>>,
}

fn register_counter_vecs() -> (
    CounterVec,
    CounterVec,
    CounterVec,
    CounterVec,
    CounterVec,
    CounterVec,
    CounterVec,
    CounterVec,
    CounterVec,
    CounterVec,
    CounterVec,
) {
    let requests = prometheus::register_counter_vec!(
        "sccache_requests_total",
        "Cumulative sccache request counters by result",
        &["result"]
    )
    .or_exit("sccache_requests_total");
    let cache_requests = prometheus::register_counter_vec!(
        "sccache_cache_requests_total",
        "Cumulative sccache cache hit/miss/error counters by language",
        &["result", "language"]
    )
    .or_exit("sccache_cache_requests_total");
    let cache_events = prometheus::register_counter_vec!(
        "sccache_cache_events_total",
        "Cumulative sccache cache events",
        &["event"]
    )
    .or_exit("sccache_cache_events_total");
    let compilations = prometheus::register_counter_vec!(
        "sccache_compilations_total",
        "Cumulative sccache compilation counters",
        &["result"]
    )
    .or_exit("sccache_compilations_total");
    let durations = prometheus::register_counter_vec!(
        "sccache_duration_seconds_total",
        "Cumulative time spent in sccache operations",
        &["operation"]
    )
    .or_exit("sccache_duration_seconds_total");
    let not_cached = prometheus::register_counter_vec!(
        "sccache_not_cached_total",
        "Cumulative sccache non-cacheable reasons",
        &["reason"]
    )
    .or_exit("sccache_not_cached_total");
    let not_cached_crate_types = prometheus::register_counter_vec!(
        "sccache_not_cached_crate_types_total",
        "Cumulative Rust crate types behind sccache crate-type non-cacheable results",
        &["crate_type"]
    )
    .or_exit("sccache_not_cached_crate_types_total");
    let dist_compiles = prometheus::register_counter_vec!(
        "sccache_dist_compiles_total",
        "Cumulative successfully distributed sccache compilations by worker",
        &["server"]
    )
    .or_exit("sccache_dist_compiles_total");
    let dist_events = prometheus::register_counter_vec!(
        "sccache_dist_events_total",
        "Cumulative distributed-compilation events",
        &["event"]
    )
    .or_exit("sccache_dist_events_total");
    let cache_level_operations = prometheus::register_counter_vec!(
        "sccache_cache_level_operations_total",
        "Cumulative multi-level cache operations by level",
        &["level", "name", "operation"]
    )
    .or_exit("sccache_cache_level_operations_total");
    let cache_level_durations = prometheus::register_counter_vec!(
        "sccache_cache_level_duration_seconds_total",
        "Cumulative multi-level cache operation duration by level",
        &["level", "name", "operation"]
    )
    .or_exit("sccache_cache_level_duration_seconds_total");
    (
        requests,
        cache_requests,
        cache_events,
        compilations,
        durations,
        not_cached,
        not_cached_crate_types,
        dist_compiles,
        dist_events,
        cache_level_operations,
        cache_level_durations,
    )
}

fn register_gauges() -> (GaugeVec, GaugeVec, Gauge, Gauge, Gauge, GaugeVec) {
    let info = prometheus::register_gauge_vec!(
        "sccache_info",
        "sccache build/version information",
        &["version"]
    )
    .or_exit("sccache_info");
    let cache_size_bytes = prometheus::register_gauge_vec!(
        "sccache_cache_size_bytes",
        "sccache cache size values when the backend reports them",
        &["kind"]
    )
    .or_exit("sccache_cache_size_bytes");
    let preprocessor_cache_mode = prometheus::register_gauge!(
        "sccache_preprocessor_cache_mode",
        "Whether sccache preprocessor-cache mode is enabled"
    )
    .or_exit("sccache_preprocessor_cache_mode");
    let basedirs =
        prometheus::register_gauge!("sccache_basedirs", "Number of configured sccache basedirs")
            .or_exit("sccache_basedirs");
    let cache_levels = prometheus::register_gauge!(
        "sccache_cache_levels",
        "Number of configured sccache multi-level cache levels"
    )
    .or_exit("sccache_cache_levels");
    let dist_status = prometheus::register_gauge_vec!(
        "sccache_dist_status",
        "Current distributed-compilation status values; emitted only when dist-status collection is enabled",
        &["kind"]
    )
    .or_exit("sccache_dist_status");
    (
        info,
        cache_size_bytes,
        preprocessor_cache_mode,
        basedirs,
        cache_levels,
        dist_status,
    )
}

impl SccacheMetrics {
    fn new() -> Self {
        let (
            requests,
            cache_requests,
            cache_events,
            compilations,
            durations,
            not_cached,
            not_cached_crate_types,
            dist_compiles,
            dist_events,
            cache_level_operations,
            cache_level_durations,
        ) = register_counter_vecs();
        let (info, cache_size_bytes, preprocessor_cache_mode, basedirs, cache_levels, dist_status) =
            register_gauges();
        Self {
            requests,
            cache_requests,
            cache_events,
            compilations,
            durations,
            not_cached,
            not_cached_crate_types,
            dist_compiles,
            dist_events,
            cache_level_operations,
            cache_level_durations,
            info,
            cache_size_bytes,
            preprocessor_cache_mode,
            basedirs,
            cache_levels,
            dist_status,
            previous: Mutex::new(HashMap::new()),
        }
    }

    const fn counter_vec(&self, family: CounterFamily) -> &CounterVec {
        match family {
            CounterFamily::Requests => &self.requests,
            CounterFamily::CacheRequests => &self.cache_requests,
            CounterFamily::CacheEvents => &self.cache_events,
            CounterFamily::Compilations => &self.compilations,
            CounterFamily::Durations => &self.durations,
            CounterFamily::NotCached => &self.not_cached,
            CounterFamily::NotCachedCrateTypes => &self.not_cached_crate_types,
            CounterFamily::DistCompiles => &self.dist_compiles,
            CounterFamily::DistEvents => &self.dist_events,
            CounterFamily::CacheLevelOperations => &self.cache_level_operations,
            CounterFamily::CacheLevelDurations => &self.cache_level_durations,
        }
    }

    fn apply(&self, snapshot: Snapshot) -> CollectionReport {
        self.info.reset();
        self.info.with_label_values(&[&snapshot.version]).set(1.0);
        self.cache_size_bytes.reset();
        if let Some(value) = snapshot.cache_size {
            self.cache_size_bytes
                .with_label_values(&["current"])
                .set(prometheus_u64(value));
        }
        if let Some(value) = snapshot.max_cache_size {
            self.cache_size_bytes
                .with_label_values(&["max"])
                .set(prometheus_u64(value));
        }
        self.preprocessor_cache_mode
            .set(if snapshot.preprocessor_cache_mode {
                1.0
            } else {
                0.0
            });
        self.basedirs.set(prometheus_u64(snapshot.basedirs));
        self.cache_levels.set(prometheus_u64(snapshot.cache_levels));

        let Ok(mut previous) = self.previous.lock() else {
            return CollectionReport::error();
        };
        let mut seen = HashSet::with_capacity(snapshot.counters.len());
        for sample in snapshot.counters {
            let old = previous
                .insert(sample.key.clone(), sample.value)
                .unwrap_or(0);
            let delta = monotonic_delta(old, sample.value);
            let labels: Vec<&str> = sample.key.labels.iter().map(String::as_str).collect();
            self.counter_vec(sample.key.family)
                .with_label_values(&labels)
                .inc_by(prometheus_u64(delta) * sample.scale);
            seen.insert(sample.key);
        }

        let stale: Vec<_> = previous
            .keys()
            .filter(|key| !seen.contains(*key))
            .cloned()
            .collect();
        for key in stale {
            let labels: Vec<&str> = key.labels.iter().map(String::as_str).collect();
            if let Err(error) = self.counter_vec(key.family).remove_label_values(&labels)
                && debug_enabled()
            {
                eprintln!("sccache: failed to remove stale counter labels {labels:?}: {error}");
            }
            previous.remove(&key);
        }
        CollectionReport::success()
    }

    fn clear_unavailable(&self) -> CollectionReport {
        self.info.reset();
        self.cache_size_bytes.reset();
        self.dist_status.reset();
        self.preprocessor_cache_mode.set(f64::NAN);
        self.basedirs.set(f64::NAN);
        self.cache_levels.set(f64::NAN);

        let Ok(mut previous) = self.previous.lock() else {
            return CollectionReport::error();
        };
        let stale: Vec<_> = previous.keys().cloned().collect();
        let mut report = CollectionReport::success();
        for key in stale {
            let labels: Vec<&str> = key.labels.iter().map(String::as_str).collect();
            if let Err(error) = self.counter_vec(key.family).remove_label_values(&labels) {
                report.record_error();
                if debug_enabled() {
                    eprintln!(
                        "sccache: failed to remove unavailable counter labels {labels:?}: {error}"
                    );
                }
            }
            previous.remove(&key);
        }
        report
    }

    fn apply_dist(&self, snapshot: DistSnapshot) {
        self.dist_status.reset();
        self.dist_status
            .with_label_values(&["enabled"])
            .set(if snapshot.enabled { 1.0 } else { 0.0 });
        self.dist_status
            .with_label_values(&["connected"])
            .set(if snapshot.connected { 1.0 } else { 0.0 });
        if let Some(status) = snapshot.scheduler {
            self.dist_status
                .with_label_values(&["servers"])
                .set(prometheus_u64(status.num_servers));
            self.dist_status
                .with_label_values(&["cpus"])
                .set(prometheus_u64(status.num_cpus));
            self.dist_status
                .with_label_values(&["in_progress"])
                .set(prometheus_u64(status.in_progress));
        }
    }
}

static SCCACHE_METRICS: OnceLock<SccacheMetrics> = OnceLock::new();

fn metrics() -> &'static SccacheMetrics {
    SCCACHE_METRICS.get_or_init(SccacheMetrics::new)
}

const fn monotonic_delta(previous: u64, current: u64) -> u64 {
    if current >= previous {
        current - previous
    } else {
        current
    }
}

fn push_counter(
    snapshot: &mut Snapshot,
    family: CounterFamily,
    labels: &[&str],
    value: u64,
    scale: f64,
) {
    snapshot.counters.push(CounterSample {
        key: CounterKey {
            family,
            labels: labels.iter().map(|label| (*label).to_string()).collect(),
        },
        value,
        scale,
    });
}

fn push_duration(
    snapshot: &mut Snapshot,
    labels: &[&str],
    value: JsonDuration,
) -> Result<(), String> {
    push_counter(
        snapshot,
        CounterFamily::Durations,
        labels,
        value.total_nanos()?,
        1e-9,
    );
    Ok(())
}

fn push_request_stats(snapshot: &mut Snapshot, stats: &ServerStats) {
    for (result, value) in [
        ("compile", stats.compile_requests),
        ("unsupported_compiler", stats.requests_unsupported_compiler),
        ("not_compile", stats.requests_not_compile),
        ("not_cacheable", stats.requests_not_cacheable),
        ("executed", stats.requests_executed),
    ] {
        push_counter(snapshot, CounterFamily::Requests, &[result], value, 1.0);
    }
}

fn push_cache_stats(snapshot: &mut Snapshot, stats: &ServerStats) {
    for (result, counts) in [
        ("error", &stats.cache_errors.counts),
        ("hit", &stats.cache_hits.counts),
        ("miss", &stats.cache_misses.counts),
    ] {
        for (language, value) in counts {
            push_counter(
                snapshot,
                CounterFamily::CacheRequests,
                &[result, language],
                *value,
                1.0,
            );
        }
    }
    for (event, value) in [
        ("timeout", stats.cache_timeouts),
        ("read_error", stats.cache_read_errors),
        (
            "non_cacheable_compilation",
            stats.non_cacheable_compilations,
        ),
        ("forced_recache", stats.forced_recaches),
        ("write_error", stats.cache_write_errors),
        ("write", stats.cache_writes),
    ] {
        push_counter(snapshot, CounterFamily::CacheEvents, &[event], value, 1.0);
    }
}

fn push_compile_stats(snapshot: &mut Snapshot, stats: &ServerStats) -> Result<(), String> {
    for (result, value) in [
        ("performed", stats.compilations),
        ("failed", stats.compile_fails),
    ] {
        push_counter(snapshot, CounterFamily::Compilations, &[result], value, 1.0);
    }
    push_duration(snapshot, &["cache_write"], stats.cache_write_duration)?;
    push_duration(snapshot, &["cache_read_hit"], stats.cache_read_hit_duration)?;
    push_duration(snapshot, &["compile"], stats.compiler_write_duration)
}

fn push_dynamic_stats(snapshot: &mut Snapshot, stats: &ServerStats) {
    for (reason, value) in &stats.not_cached {
        push_counter(snapshot, CounterFamily::NotCached, &[reason], *value, 1.0);
    }
    for (crate_type, value) in &stats.not_cached_crate_types {
        push_counter(
            snapshot,
            CounterFamily::NotCachedCrateTypes,
            &[crate_type],
            *value,
            1.0,
        );
    }
    for (server, value) in &stats.dist_compiles {
        push_counter(
            snapshot,
            CounterFamily::DistCompiles,
            &[server],
            *value,
            1.0,
        );
    }
    push_counter(
        snapshot,
        CounterFamily::DistEvents,
        &["error"],
        stats.dist_errors,
        1.0,
    );
}

fn push_cache_level_stats(
    snapshot: &mut Snapshot,
    levels: &[CacheLevelStats],
) -> Result<(), String> {
    for (index, level) in levels.iter().enumerate() {
        let index = index.to_string();
        for (operation, value) in [
            ("hit", level.hits),
            ("miss", level.misses),
            ("write", level.writes),
            ("write_failure", level.write_failures),
            ("backfill_from", level.backfills_from),
            ("backfill_to", level.backfills_to),
        ] {
            push_counter(
                snapshot,
                CounterFamily::CacheLevelOperations,
                &[&index, &level.name, operation],
                value,
                1.0,
            );
        }
        for (operation, duration) in [("hit", level.hit_duration), ("write", level.write_duration)]
        {
            push_counter(
                snapshot,
                CounterFamily::CacheLevelDurations,
                &[&index, &level.name, operation],
                duration.total_nanos()?,
                1e-9,
            );
        }
    }
    Ok(())
}

fn snapshot_from_info(info: ServerInfo) -> Result<Snapshot, String> {
    let ServerInfo {
        stats,
        cache_size,
        max_cache_size,
        use_preprocessor_cache_mode,
        version,
        basedirs,
    } = info;
    let levels = stats.multi_level.as_deref().unwrap_or_default();
    let basedirs = u64::try_from(basedirs.len())
        .map_err(|_| "sccache basedir count does not fit u64".to_string())?;
    let cache_levels = u64::try_from(levels.len())
        .map_err(|_| "sccache cache-level count does not fit u64".to_string())?;
    let mut snapshot = Snapshot {
        version,
        cache_size,
        max_cache_size,
        preprocessor_cache_mode: use_preprocessor_cache_mode,
        basedirs,
        cache_levels,
        counters: Vec::new(),
    };
    push_request_stats(&mut snapshot, &stats);
    push_cache_stats(&mut snapshot, &stats);
    push_compile_stats(&mut snapshot, &stats)?;
    push_dynamic_stats(&mut snapshot, &stats);
    push_cache_level_stats(&mut snapshot, levels)?;
    Ok(snapshot)
}

fn dist_snapshot(info: DistInfo) -> DistSnapshot {
    match info {
        DistInfo::Disabled(reason) => {
            let _ = reason;
            DistSnapshot::default()
        }
        DistInfo::NotConnected(url, reason) => {
            let _ = (url, reason);
            DistSnapshot {
                enabled: true,
                connected: false,
                scheduler: None,
            }
        }
        DistInfo::SchedulerStatus(url, scheduler) => {
            let _ = url;
            DistSnapshot {
                enabled: true,
                connected: true,
                scheduler: Some(scheduler),
            }
        }
    }
}

fn run_sccache(config: &AppConfig, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new(&config.sccache_binary);
    command
        .args(args)
        .env(
            "SCCACHE_SERVER_PORT",
            config.sccache_server_port.to_string(),
        )
        .env_remove("SCCACHE_SERVER_UDS");
    crate::subprocess::run_bounded(
        command,
        Duration::from_millis(config.sccache_timeout_ms),
        crate::subprocess::DEFAULT_MAX_OUTPUT_BYTES,
        "sccache command",
    )
}

fn daemon_reachable(config: &AppConfig) -> Result<(), String> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, config.sccache_server_port));
    TcpStream::connect_timeout(
        &addr,
        Duration::from_millis(config.sccache_timeout_ms.min(250)),
    )
    .map(|_| ())
    .map_err(|error| format!("connect to sccache daemon at {addr}: {error}"))
}

fn collect_stats(config: &AppConfig) -> Result<Snapshot, String> {
    daemon_reachable(config)?;
    let output = run_sccache(config, &["--show-stats", "--stats-format", "json"])?;
    let info: ServerInfo = serde_json::from_str(&output)
        .map_err(|error| format!("parse sccache stats JSON: {error}"))?;
    snapshot_from_info(info)
}

fn collect_dist_status(config: &AppConfig) -> Result<DistSnapshot, String> {
    daemon_reachable(config)?;
    let output = run_sccache(config, &["--dist-status"])?;
    let info: DistInfo = serde_json::from_str(&output)
        .map_err(|error| format!("parse sccache dist-status JSON: {error}"))?;
    Ok(dist_snapshot(info))
}

pub fn update_metrics(config: &AppConfig) -> CollectionReport {
    let snapshot = match collect_stats(config) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            if debug_enabled() {
                eprintln!("sccache: {error}");
            }
            let mut report = CollectionReport::error();
            if let Some(metrics) = SCCACHE_METRICS.get() {
                report.merge(metrics.clear_unavailable());
            }
            return report;
        }
    };
    let mut report = metrics().apply(snapshot);
    metrics().dist_status.reset();
    if config.sccache_collect_dist_status {
        match collect_dist_status(config) {
            Ok(snapshot) => metrics().apply_dist(snapshot),
            Err(error) => {
                if debug_enabled() {
                    eprintln!("sccache dist-status: {error}");
                }
                report.record_error();
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{
      "stats": {
        "compile_requests": 100,
        "requests_unsupported_compiler": 1,
        "requests_not_compile": 2,
        "requests_not_cacheable": 3,
        "requests_executed": 94,
        "cache_errors": {"counts":{"Rust":4},"adv_counts":{}},
        "cache_hits": {"counts":{"Rust":40,"C/C++":5},"adv_counts":{}},
        "cache_misses": {"counts":{"Rust":30},"adv_counts":{}},
        "cache_timeouts": 6,
        "cache_read_errors": 7,
        "non_cacheable_compilations": 8,
        "forced_recaches": 9,
        "cache_write_errors": 10,
        "cache_writes": 11,
        "cache_write_duration": {"secs":1,"nanos":500000000},
        "cache_read_hit_duration": {"secs":2,"nanos":250000000},
        "compilations": 12,
        "compiler_write_duration": {"secs":3,"nanos":0},
        "compile_fails": 13,
        "not_cached": {"incremental":14},
        "not_cached_crate_types": {"bin":15},
        "dist_compiles": {"10.0.0.1:10501":16},
        "dist_errors": 17,
        "multi_level": [{
          "name":"L0 (s3)","location":"secret-ish location intentionally ignored",
          "hits":18,"misses":19,"writes":20,"write_failures":21,
          "backfills_from":22,"backfills_to":23,
          "hit_duration":{"secs":4,"nanos":0},
          "write_duration":{"secs":5,"nanos":0}
        }]
      },
      "cache_location":"ignored",
      "cache_size":1234,
      "max_cache_size":5678,
      "use_preprocessor_cache_mode":true,
      "version":"0.18.0",
      "basedirs":["/a","/b"]
    }"#;

    #[test]
    fn parses_current_stats_shape_without_exporting_sensitive_location() {
        let info: ServerInfo = serde_json::from_str(FIXTURE).unwrap();
        let snapshot = snapshot_from_info(info).unwrap();
        assert_eq!(snapshot.version, "0.18.0");
        assert_eq!(snapshot.cache_size, Some(1234));
        assert_eq!(snapshot.max_cache_size, Some(5678));
        assert!(snapshot.preprocessor_cache_mode);
        assert_eq!(snapshot.basedirs, 2);
        assert_eq!(snapshot.cache_levels, 1);
        assert!(snapshot.counters.iter().any(|sample| {
            sample.key.family == CounterFamily::CacheRequests
                && sample.key.labels == ["hit", "Rust"]
                && sample.value == 40
        }));
        assert!(snapshot.counters.iter().all(|sample| {
            !sample
                .key
                .labels
                .iter()
                .any(|label| label.contains("secret-ish"))
        }));
    }

    #[test]
    fn duration_conversion_is_exact_at_nanosecond_resolution() {
        assert_eq!(
            JsonDuration {
                secs: 7,
                nanos: 123_456_789
            }
            .total_nanos()
            .unwrap(),
            7_123_456_789
        );
    }

    #[test]
    fn monotonic_delta_treats_counter_decrease_as_reset() {
        assert_eq!(monotonic_delta(10, 14), 4);
        assert_eq!(monotonic_delta(14, 3), 3);
    }

    #[test]
    fn parses_scheduler_status_and_nonconnected_variants() {
        let connected: DistInfo = serde_json::from_str(
            r#"{"SchedulerStatus":["http://127.0.0.1:10600/",{"num_servers":2,"num_cpus":20,"in_progress":7}]}"#,
        )
        .unwrap();
        let connected = dist_snapshot(connected);
        assert!(connected.enabled);
        assert!(connected.connected);
        assert_eq!(connected.scheduler.unwrap().in_progress, 7);

        let disconnected: DistInfo =
            serde_json::from_str(r#"{"NotConnected":["http://127.0.0.1:10600/","offline"]}"#)
                .unwrap();
        let disconnected = dist_snapshot(disconnected);
        assert!(disconnected.enabled);
        assert!(!disconnected.connected);
    }

    #[test]
    fn unavailable_collection_removes_stale_labeled_series() {
        let mut snapshot = Snapshot {
            version: "test-version".to_string(),
            basedirs: 2,
            cache_levels: 1,
            ..Snapshot::default()
        };
        push_counter(
            &mut snapshot,
            CounterFamily::Requests,
            &["compile"],
            10,
            1.0,
        );
        let metrics = metrics();
        assert!(metrics.apply(snapshot).is_success());
        let series_count = |families: Vec<prometheus::proto::MetricFamily>| {
            families
                .iter()
                .map(|family| family.get_metric().len())
                .sum::<usize>()
        };
        assert_eq!(
            series_count(prometheus::core::Collector::collect(&metrics.requests)),
            1
        );
        assert_eq!(
            series_count(prometheus::core::Collector::collect(&metrics.info)),
            1
        );

        assert!(metrics.clear_unavailable().is_success());
        assert_eq!(
            series_count(prometheus::core::Collector::collect(&metrics.requests)),
            0
        );
        assert_eq!(
            series_count(prometheus::core::Collector::collect(&metrics.info)),
            0
        );
        assert!(metrics.preprocessor_cache_mode.get().is_nan());
        assert!(metrics.basedirs.get().is_nan());
        assert!(metrics.cache_levels.get().is_nan());
    }

    #[test]
    fn unknown_future_stats_fields_are_ignored() {
        let mut value: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        value["stats"]["future_counter"] = serde_json::json!(42);
        let info: ServerInfo = serde_json::from_value(value).unwrap();
        assert_eq!(info.stats.compile_requests, 100);
    }
}
