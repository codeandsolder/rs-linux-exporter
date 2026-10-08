use crate::collection::CollectionReport;
use crate::config::AppConfig;
use crate::metric_support::{RegisterMetricResultExt, prometheus_i64, prometheus_u64};
use crate::runtime::debug_enabled;
use prometheus::{CounterVec, Gauge, GaugeVec};
use std::collections::BTreeMap;
use std::fs;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const ARCSTATS_PATH: &str = "/proc/spl/kstat/zfs/arcstats";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KstatValue {
    Signed(i64),
    Unsigned(u64),
}

impl KstatValue {
    const fn as_f64(self) -> f64 {
        match self {
            Self::Signed(value) => prometheus_i64(value),
            Self::Unsigned(value) => prometheus_u64(value),
        }
    }

    fn as_u64(self) -> Option<u64> {
        match self {
            Self::Signed(value) => value.try_into().ok(),
            Self::Unsigned(value) => Some(value),
        }
    }
}

struct ZfsMetrics {
    size_bytes: GaugeVec,
    memory_available_bytes: Gauge,
    accesses_total: CounterVec,
    hit_ratio: GaugeVec,
    previous_accesses: Mutex<BTreeMap<&'static str, u64>>,
}

impl ZfsMetrics {
    fn new() -> Self {
        Self {
            size_bytes: prometheus::register_gauge_vec!(
                "zfs_arc_size_bytes",
                "ZFS ARC sizes in bytes",
                &["kind"]
            )
            .or_exit("zfs_arc_size_bytes"),
            memory_available_bytes: prometheus::register_gauge!(
                "zfs_arc_memory_available_bytes",
                "Memory available to the ZFS ARC according to OpenZFS"
            )
            .or_exit("zfs_arc_memory_available_bytes"),
            accesses_total: prometheus::register_counter_vec!(
                "zfs_arc_accesses_total",
                "Cumulative ZFS ARC access counters",
                &["kind"]
            )
            .or_exit("zfs_arc_accesses_total"),
            hit_ratio: prometheus::register_gauge_vec!(
                "zfs_arc_hit_ratio",
                "ZFS ARC hit ratio since boot",
                &["cache"]
            )
            .or_exit("zfs_arc_hit_ratio"),
            previous_accesses: Mutex::new(BTreeMap::new()),
        }
    }
}

static ZFS_METRICS: OnceLock<ZfsMetrics> = OnceLock::new();

fn metrics() -> &'static ZfsMetrics {
    ZFS_METRICS.get_or_init(ZfsMetrics::new)
}

fn parse_arcstats(content: &str) -> Result<BTreeMap<String, KstatValue>, String> {
    let mut lines = content.lines();
    let Some(header) = lines.next() else {
        return Err("missing kstat header".to_string());
    };
    let Some(columns) = lines.next() else {
        return Err("missing kstat column header".to_string());
    };
    if header.split_whitespace().count() < 4 {
        return Err("malformed kstat header".to_string());
    }
    if columns.split_whitespace().collect::<Vec<_>>() != ["name", "type", "data"] {
        return Err("unexpected kstat column header".to_string());
    }

    let mut values = BTreeMap::new();
    for (line_index, line) in lines.enumerate() {
        let mut parts = line.split_whitespace();
        let Some(name) = parts.next() else { continue };
        let Some(value_type) = parts.next() else {
            return Err(format!("line {}: missing kstat type", line_index + 3));
        };
        let Some(raw_value) = parts.next() else {
            return Err(format!("line {}: missing kstat value", line_index + 3));
        };

        let value = match value_type {
            "3" => KstatValue::Signed(raw_value.parse::<i64>().map_err(|error| {
                format!(
                    "line {}: invalid i64 {raw_value:?}: {error}",
                    line_index + 3
                )
            })?),
            "4" => KstatValue::Unsigned(raw_value.parse::<u64>().map_err(|error| {
                format!(
                    "line {}: invalid u64 {raw_value:?}: {error}",
                    line_index + 3
                )
            })?),
            _ => continue,
        };
        values.insert(name.to_string(), value);
    }

    if values.is_empty() {
        return Err("arcstats contained no numeric kstats".to_string());
    }
    Ok(values)
}

fn get_u64(values: &BTreeMap<String, KstatValue>, key: &str) -> Option<u64> {
    values.get(key).copied().and_then(KstatValue::as_u64)
}

fn set_hit_ratio(metric: &GaugeVec, cache: &str, hits: Option<u64>, misses: Option<u64>) {
    let (Some(hits), Some(misses)) = (hits, misses) else {
        return;
    };
    let total = hits.saturating_add(misses);
    if total == 0 {
        return;
    }
    metric
        .with_label_values(&[cache])
        .set(prometheus_u64(hits) / prometheus_u64(total));
}

fn update_access_counters(
    metrics: &ZfsMetrics,
    values: &BTreeMap<String, KstatValue>,
) -> CollectionReport {
    const ACCESS_FIELDS: &[&str] = &[
        "hits",
        "misses",
        "demand_data_hits",
        "demand_data_misses",
        "demand_metadata_hits",
        "demand_metadata_misses",
        "prefetch_data_hits",
        "prefetch_data_misses",
        "prefetch_metadata_hits",
        "prefetch_metadata_misses",
        "mru_hits",
        "mfu_hits",
        "mru_ghost_hits",
        "mfu_ghost_hits",
        "l2_hits",
        "l2_misses",
    ];

    let Ok(mut previous) = metrics.previous_accesses.lock() else {
        return CollectionReport::error();
    };
    for &field in ACCESS_FIELDS {
        let Some(current) = get_u64(values, field) else {
            continue;
        };
        let old = previous.insert(field, current).unwrap_or(0);
        let delta = if current >= old {
            current - old
        } else {
            current
        };
        metrics
            .accesses_total
            .with_label_values(&[field])
            .inc_by(prometheus_u64(delta));
    }
    CollectionReport::success()
}

fn update_arc_metrics() -> CollectionReport {
    let content = match fs::read_to_string(ARCSTATS_PATH) {
        Ok(content) => content,
        Err(error) => {
            if debug_enabled() {
                eprintln!("zfs: failed to read {ARCSTATS_PATH}: {error}");
            }
            return CollectionReport::error();
        }
    };
    let values = match parse_arcstats(&content) {
        Ok(values) => values,
        Err(error) => {
            if debug_enabled() {
                eprintln!("zfs: failed to parse {ARCSTATS_PATH}: {error}");
            }
            return CollectionReport::error();
        }
    };

    let metrics = metrics();
    metrics.size_bytes.reset();
    for (field, kind) in [
        ("size", "current"),
        ("c", "target"),
        ("c_min", "min"),
        ("c_max", "max"),
        ("compressed_size", "compressed"),
        ("uncompressed_size", "uncompressed"),
        ("l2_size", "l2"),
    ] {
        if let Some(value) = values.get(field) {
            metrics
                .size_bytes
                .with_label_values(&[kind])
                .set(value.as_f64());
        }
    }
    if let Some(value) = values.get("memory_available_bytes") {
        metrics.memory_available_bytes.set(value.as_f64());
    }

    metrics.hit_ratio.reset();
    set_hit_ratio(
        &metrics.hit_ratio,
        "arc",
        get_u64(&values, "hits"),
        get_u64(&values, "misses"),
    );
    set_hit_ratio(
        &metrics.hit_ratio,
        "l2arc",
        get_u64(&values, "l2_hits"),
        get_u64(&values, "l2_misses"),
    );

    update_access_counters(metrics, &values)
}

struct ZpoolMetrics {
    info: GaugeVec,
    size_bytes: GaugeVec,
    fragmentation_ratio: GaugeVec,
    capacity_ratio: GaugeVec,
    iops: GaugeVec,
    bandwidth_bytes_per_second: GaugeVec,
    errors: GaugeVec,
    vdev_errors: GaugeVec,
}

impl ZpoolMetrics {
    fn new() -> Self {
        Self {
            info: prometheus::register_gauge_vec!(
                "zfs_pool_info",
                "ZFS pool identity and health state.",
                &["pool", "health"]
            )
            .or_exit("zfs_pool_info"),
            size_bytes: prometheus::register_gauge_vec!(
                "zfs_pool_size_bytes",
                "ZFS pool size values in bytes.",
                &["pool", "kind"]
            )
            .or_exit("zfs_pool_size_bytes"),
            fragmentation_ratio: prometheus::register_gauge_vec!(
                "zfs_pool_fragmentation_ratio",
                "ZFS pool fragmentation ratio from 0 to 1.",
                &["pool"]
            )
            .or_exit("zfs_pool_fragmentation_ratio"),
            capacity_ratio: prometheus::register_gauge_vec!(
                "zfs_pool_capacity_ratio",
                "ZFS pool allocated capacity ratio from 0 to 1.",
                &["pool"]
            )
            .or_exit("zfs_pool_capacity_ratio"),
            iops: prometheus::register_gauge_vec!(
                "zfs_pool_iops",
                "ZFS pool average operations per second reported by zpool iostat.",
                &["pool", "direction"]
            )
            .or_exit("zfs_pool_iops"),
            bandwidth_bytes_per_second: prometheus::register_gauge_vec!(
                "zfs_pool_bandwidth_bytes_per_second",
                "ZFS pool average bandwidth reported by zpool iostat.",
                &["pool", "direction"]
            )
            .or_exit("zfs_pool_bandwidth_bytes_per_second"),
            errors: prometheus::register_gauge_vec!(
                "zfs_pool_errors",
                "Current ZFS pool read/write/checksum error counters.",
                &["pool", "kind"]
            )
            .or_exit("zfs_pool_errors"),
            vdev_errors: prometheus::register_gauge_vec!(
                "zfs_vdev_errors",
                "Current ZFS vdev read/write/checksum error counters.",
                &["pool", "vdev", "kind"]
            )
            .or_exit("zfs_vdev_errors"),
        }
    }

    fn reset(&self) {
        self.info.reset();
        self.size_bytes.reset();
        self.fragmentation_ratio.reset();
        self.capacity_ratio.reset();
        self.iops.reset();
        self.bandwidth_bytes_per_second.reset();
        self.errors.reset();
        self.vdev_errors.reset();
    }
}

static ZPOOL_METRICS: OnceLock<ZpoolMetrics> = OnceLock::new();
static ZPOOL_LAST_REFRESH: Mutex<Option<Instant>> = Mutex::new(None);

fn zpool_metrics() -> &'static ZpoolMetrics {
    ZPOOL_METRICS.get_or_init(ZpoolMetrics::new)
}

fn zpool_due() -> bool {
    let Ok(mut last) = ZPOOL_LAST_REFRESH.lock() else {
        return true;
    };
    if last.is_some_and(|instant| instant.elapsed() < Duration::from_secs(30)) {
        return false;
    }
    *last = Some(Instant::now());
    true
}

fn run_zpool(args: &[&str], description: &str) -> Result<String, String> {
    let mut command = Command::new("zpool");
    command.args(args);
    crate::subprocess::run_bounded(
        command,
        Duration::from_secs(5),
        2 * 1024 * 1024,
        description,
    )
}

fn parse_pool_list(metrics: &ZpoolMetrics, output: &str) -> Result<(), String> {
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 7 {
            return Err(format!("unexpected zpool list row: {line:?}"));
        }
        let pool = fields[0];
        let size = fields[1]
            .parse::<u64>()
            .map_err(|error| format!("invalid pool size: {error}"))?;
        let alloc = fields[2]
            .parse::<u64>()
            .map_err(|error| format!("invalid allocated size: {error}"))?;
        let free = fields[3]
            .parse::<u64>()
            .map_err(|error| format!("invalid free size: {error}"))?;
        let fragmentation = fields[4]
            .parse::<f64>()
            .map_err(|error| format!("invalid fragmentation: {error}"))?;
        let capacity = fields[5]
            .parse::<f64>()
            .map_err(|error| format!("invalid capacity: {error}"))?;
        let health = fields[6];
        metrics.info.with_label_values(&[pool, health]).set(1.0);
        for (kind, value) in [("total", size), ("allocated", alloc), ("free", free)] {
            metrics
                .size_bytes
                .with_label_values(&[pool, kind])
                .set(prometheus_u64(value));
        }
        metrics
            .fragmentation_ratio
            .with_label_values(&[pool])
            .set(fragmentation / 100.0);
        metrics
            .capacity_ratio
            .with_label_values(&[pool])
            .set(capacity / 100.0);
    }
    Ok(())
}

fn parse_pool_iostat(metrics: &ZpoolMetrics, output: &str) -> Result<(), String> {
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() < 7 {
            return Err(format!("unexpected zpool iostat row: {line:?}"));
        }
        let pool = fields[0];
        let read_iops = fields[3]
            .parse::<f64>()
            .map_err(|error| format!("invalid read iops: {error}"))?;
        let write_iops = fields[4]
            .parse::<f64>()
            .map_err(|error| format!("invalid write iops: {error}"))?;
        let read_bw = fields[5]
            .parse::<f64>()
            .map_err(|error| format!("invalid read bandwidth: {error}"))?;
        let write_bw = fields[6]
            .parse::<f64>()
            .map_err(|error| format!("invalid write bandwidth: {error}"))?;
        metrics
            .iops
            .with_label_values(&[pool, "read"])
            .set(read_iops);
        metrics
            .iops
            .with_label_values(&[pool, "write"])
            .set(write_iops);
        metrics
            .bandwidth_bytes_per_second
            .with_label_values(&[pool, "read"])
            .set(read_bw);
        metrics
            .bandwidth_bytes_per_second
            .with_label_values(&[pool, "write"])
            .set(write_bw);
    }
    Ok(())
}

fn parse_status_row(line: &str) -> Option<(&str, &str, f64, f64, f64)> {
    let fields = line.split_whitespace().collect::<Vec<_>>();
    if fields.len() < 5 {
        return None;
    }
    let len = fields.len();
    let read = fields[len - 3].parse::<f64>().ok()?;
    let write = fields[len - 2].parse::<f64>().ok()?;
    let cksum = fields[len - 1].parse::<f64>().ok()?;
    Some((fields[0], fields[1], read, write, cksum))
}

fn parse_pool_status(metrics: &ZpoolMetrics, output: &str) {
    let mut pool = None::<String>;
    let mut in_config = false;
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix("pool:") {
            pool = Some(name.trim().to_string());
            in_config = false;
            continue;
        }
        if trimmed == "config:" {
            in_config = true;
            continue;
        }
        if !in_config || trimmed.is_empty() || trimmed.starts_with("NAME ") {
            continue;
        }
        if trimmed.starts_with("errors:") || trimmed.starts_with("scan:") {
            in_config = false;
            continue;
        }
        let Some((name, _state, read, write, cksum)) = parse_status_row(line) else {
            continue;
        };
        let Some(pool_name) = pool.as_deref() else {
            continue;
        };
        let target = if name == pool_name {
            &metrics.errors
        } else {
            &metrics.vdev_errors
        };
        if name == pool_name {
            for (kind, value) in [("read", read), ("write", write), ("checksum", cksum)] {
                target.with_label_values(&[pool_name, kind]).set(value);
            }
        } else {
            for (kind, value) in [("read", read), ("write", write), ("checksum", cksum)] {
                metrics
                    .vdev_errors
                    .with_label_values(&[pool_name, name, kind])
                    .set(value);
            }
        }
    }
}

fn update_pool_metrics() -> CollectionReport {
    if !zpool_due() {
        return CollectionReport::success();
    }
    let list = match run_zpool(
        &[
            "list",
            "-Hp",
            "-o",
            "name,size,alloc,free,fragmentation,capacity,health",
        ],
        "zpool list",
    ) {
        Ok(output) => output,
        Err(error) if error.contains("No such file") || error.contains("not found") => {
            return CollectionReport::success();
        }
        Err(error) => {
            if debug_enabled() {
                eprintln!("zfs: {error}");
            }
            return CollectionReport::error();
        }
    };
    let iostat = run_zpool(&["iostat", "-Hp"], "zpool iostat");
    let status = run_zpool(&["status", "-p"], "zpool status");
    let metrics = zpool_metrics();
    metrics.reset();
    let mut report = CollectionReport::success();
    if let Err(error) = parse_pool_list(metrics, &list) {
        if debug_enabled() {
            eprintln!("zfs: {error}");
        }
        report.merge(CollectionReport::error());
    }
    match iostat {
        Ok(output) => {
            if let Err(error) = parse_pool_iostat(metrics, &output) {
                if debug_enabled() {
                    eprintln!("zfs: {error}");
                }
                report.merge(CollectionReport::error());
            }
        }
        Err(error) => {
            if debug_enabled() {
                eprintln!("zfs: {error}");
            }
            report.merge(CollectionReport::error());
        }
    }
    match status {
        Ok(output) => parse_pool_status(metrics, &output),
        Err(error) => {
            if debug_enabled() {
                eprintln!("zfs: {error}");
            }
            report.merge(CollectionReport::error());
        }
    }
    report
}

pub fn update_metrics(_config: &AppConfig) -> CollectionReport {
    let mut report = update_arc_metrics();
    report.merge(update_pool_metrics());
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "21 1 0x01 4 100 1 2\nname type data\nhits 4 90\nmisses 4 10\nsize 4 4096\nmemory_available_bytes 3 -512\n";

    #[test]
    fn parses_signed_and_unsigned_kstats() {
        let parsed = parse_arcstats(SAMPLE).unwrap();
        assert_eq!(parsed.get("hits"), Some(&KstatValue::Unsigned(90)));
        assert_eq!(
            parsed.get("memory_available_bytes"),
            Some(&KstatValue::Signed(-512))
        );
    }

    #[test]
    fn malformed_header_is_rejected() {
        assert!(parse_arcstats("broken\nname type data\nhits 4 1\n").is_err());
    }

    #[test]
    fn hit_ratio_uses_hits_plus_misses() {
        let gauge = GaugeVec::new(
            prometheus::Opts::new("test_zfs_hit_ratio", "test"),
            &["cache"],
        )
        .unwrap();
        set_hit_ratio(&gauge, "arc", Some(90), Some(10));
        assert!((gauge.with_label_values(&["arc"]).get() - 0.9).abs() < f64::EPSILON);
    }
}
