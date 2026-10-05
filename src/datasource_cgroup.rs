use crate::collection::CollectionReport;
use crate::config::AppConfig;
use crate::metric_support::{RegisterMetricResultExt, prometheus_u64};
use crate::runtime::debug_enabled;
use procfs::{PressureRecord, get_pressure};
use prometheus::{CounterVec, Gauge, GaugeVec};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Cursor, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const CGROUP_ROOT: &str = "/sys/fs/cgroup";
const BYTE_MEMORY_FIELDS: &[&str] = &[
    "anon",
    "file",
    "kernel",
    "kernel_stack",
    "pagetables",
    "sec_pagetables",
    "percpu",
    "sock",
    "vmalloc",
    "shmem",
    "zswap",
    "zswapped",
    "file_mapped",
    "file_dirty",
    "file_writeback",
    "swapcached",
    "anon_thp",
    "file_thp",
    "shmem_thp",
    "inactive_anon",
    "active_anon",
    "inactive_file",
    "active_file",
    "unevictable",
    "slab_reclaimable",
    "slab_unreclaimable",
    "slab",
];

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum CounterFamily {
    CpuSeconds,
    CpuPeriods,
    MemoryEvents,
    IoBytes,
    IoOperations,
    PressureSeconds,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CounterKey {
    family: CounterFamily,
    labels: Vec<String>,
}

const fn monotonic_delta(previous: u64, current: u64) -> u64 {
    if current >= previous {
        current - previous
    } else {
        current
    }
}

struct CounterSample {
    key: CounterKey,
    value: u64,
    scale: f64,
}

struct GaugeSample {
    metric: GaugeFamily,
    labels: Vec<String>,
    value: f64,
}

#[derive(Clone, Copy)]
enum GaugeFamily {
    MemoryBytes,
    MemoryStatBytes,
    PidsCurrent,
    State,
    PressureRatio,
}

#[derive(Default)]
struct Snapshot {
    units: usize,
    counters: Vec<CounterSample>,
    gauges: Vec<GaugeSample>,
}

struct CgroupMetrics {
    cpu_seconds: CounterVec,
    cpu_periods: CounterVec,
    memory_events: CounterVec,
    io_bytes: CounterVec,
    io_operations: CounterVec,
    pressure_seconds: CounterVec,
    memory_bytes: GaugeVec,
    memory_stat_bytes: GaugeVec,
    pids_current: GaugeVec,
    state: GaugeVec,
    pressure_ratio: GaugeVec,
    units: Gauge,
    previous: Mutex<HashMap<CounterKey, u64>>,
}

impl CgroupMetrics {
    fn new() -> Self {
        Self {
            cpu_seconds: prometheus::register_counter_vec!(
                "cgroup_cpu_seconds_total",
                "Cumulative cgroup v2 CPU time in seconds",
                &["cgroup", "kind"]
            )
            .or_exit("cgroup_cpu_seconds_total"),
            cpu_periods: prometheus::register_counter_vec!(
                "cgroup_cpu_periods_total",
                "Cumulative cgroup v2 CPU scheduling period counters",
                &["cgroup", "kind"]
            )
            .or_exit("cgroup_cpu_periods_total"),
            memory_events: prometheus::register_counter_vec!(
                "cgroup_memory_events_total",
                "Cumulative cgroup v2 memory event counters",
                &["cgroup", "event"]
            )
            .or_exit("cgroup_memory_events_total"),
            io_bytes: prometheus::register_counter_vec!(
                "cgroup_io_bytes_total",
                "Cumulative cgroup v2 I/O bytes by device and operation",
                &["cgroup", "device", "operation"]
            )
            .or_exit("cgroup_io_bytes_total"),
            io_operations: prometheus::register_counter_vec!(
                "cgroup_io_operations_total",
                "Cumulative cgroup v2 I/O operations by device and operation",
                &["cgroup", "device", "operation"]
            )
            .or_exit("cgroup_io_operations_total"),
            pressure_seconds: prometheus::register_counter_vec!(
                "cgroup_pressure_seconds_total",
                "Cumulative cgroup v2 pressure stall time in seconds",
                &["cgroup", "resource", "scope"]
            )
            .or_exit("cgroup_pressure_seconds_total"),
            memory_bytes: prometheus::register_gauge_vec!(
                "cgroup_memory_bytes",
                "Current cgroup v2 memory usage values in bytes",
                &["cgroup", "kind"]
            )
            .or_exit("cgroup_memory_bytes"),
            memory_stat_bytes: prometheus::register_gauge_vec!(
                "cgroup_memory_stat_bytes",
                "Selected byte-valued fields from cgroup v2 memory.stat",
                &["cgroup", "field"]
            )
            .or_exit("cgroup_memory_stat_bytes"),
            pids_current: prometheus::register_gauge_vec!(
                "cgroup_pids_current",
                "Current number of processes/threads in the cgroup v2 unit",
                &["cgroup"]
            )
            .or_exit("cgroup_pids_current"),
            state: prometheus::register_gauge_vec!(
                "cgroup_state",
                "Current cgroup v2 state flags",
                &["cgroup", "state"]
            )
            .or_exit("cgroup_state"),
            pressure_ratio: prometheus::register_gauge_vec!(
                "cgroup_pressure_stall_ratio",
                "Fraction of wall time stalled over a cgroup v2 PSI averaging window",
                &["cgroup", "resource", "scope", "window"]
            )
            .or_exit("cgroup_pressure_stall_ratio"),
            units: prometheus::register_gauge!(
                "cgroup_units",
                "Number of systemd service cgroups discovered for collection"
            )
            .or_exit("cgroup_units"),
            previous: Mutex::new(HashMap::new()),
        }
    }

    const fn counter_vec(&self, family: CounterFamily) -> &CounterVec {
        match family {
            CounterFamily::CpuSeconds => &self.cpu_seconds,
            CounterFamily::CpuPeriods => &self.cpu_periods,
            CounterFamily::MemoryEvents => &self.memory_events,
            CounterFamily::IoBytes => &self.io_bytes,
            CounterFamily::IoOperations => &self.io_operations,
            CounterFamily::PressureSeconds => &self.pressure_seconds,
        }
    }

    fn clear_gauges(&self) {
        self.memory_bytes.reset();
        self.memory_stat_bytes.reset();
        self.pids_current.reset();
        self.state.reset();
        self.pressure_ratio.reset();
    }

    fn apply(&self, snapshot: Snapshot) -> CollectionReport {
        self.clear_gauges();
        self.units
            .set(u64::try_from(snapshot.units).map_or(f64::MAX, prometheus_u64));
        for sample in snapshot.gauges {
            let labels: Vec<&str> = sample.labels.iter().map(String::as_str).collect();
            match sample.metric {
                GaugeFamily::MemoryBytes => self
                    .memory_bytes
                    .with_label_values(&labels)
                    .set(sample.value),
                GaugeFamily::MemoryStatBytes => self
                    .memory_stat_bytes
                    .with_label_values(&labels)
                    .set(sample.value),
                GaugeFamily::PidsCurrent => self
                    .pids_current
                    .with_label_values(&labels)
                    .set(sample.value),
                GaugeFamily::State => self.state.with_label_values(&labels).set(sample.value),
                GaugeFamily::PressureRatio => self
                    .pressure_ratio
                    .with_label_values(&labels)
                    .set(sample.value),
            }
        }

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
                eprintln!("cgroup: failed to remove stale counter labels {labels:?}: {error}");
            }
            previous.remove(&key);
        }
        CollectionReport::success()
    }
}

static CGROUP_METRICS: OnceLock<CgroupMetrics> = OnceLock::new();

fn metrics() -> &'static CgroupMetrics {
    CGROUP_METRICS.get_or_init(CgroupMetrics::new)
}

fn discover_service_cgroups(
    base: &Path,
    roots: &[String],
    max_units: usize,
) -> Result<Vec<PathBuf>, String> {
    let mut found = HashSet::new();
    let mut stack = Vec::new();
    for root in roots {
        let path = base.join(root);
        if !path.is_dir() {
            return Err(format!(
                "configured cgroup root {root:?} is not a directory"
            ));
        }
        stack.push(path);
    }

    while let Some(path) = stack.pop() {
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".service"))
        {
            found.insert(path);
            if found.len() > max_units {
                return Err(format!(
                    "discovered more than cgroup_max_units={max_units} service cgroups"
                ));
            }
            continue;
        }

        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("read {}: {error}", path.display())),
        };
        for entry in entries {
            let entry = entry.map_err(|error| format!("read {} entry: {error}", path.display()))?;
            let entry_path = entry.path();
            let is_dir = match entry.file_type() {
                Ok(file_type) => file_type.is_dir(),
                Err(error) if error.kind() == ErrorKind::NotFound => false,
                Err(error) => return Err(format!("stat {}: {error}", entry_path.display())),
            };
            if !is_dir {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".service") || name.ends_with(".slice") {
                stack.push(entry_path);
            }
        }
    }

    let mut found: Vec<_> = found.into_iter().collect();
    found.sort_unstable();
    Ok(found)
}

fn relative_cgroup(base: &Path, path: &Path) -> Result<String, String> {
    path.strip_prefix(base)
        .map_err(|_| format!("{} is outside {}", path.display(), base.display()))?
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("cgroup path {} is not UTF-8", path.display()))
}

fn parse_key_values(content: &str) -> Result<HashMap<&str, u64>, String> {
    let mut values = HashMap::new();
    for (index, line) in content.lines().enumerate() {
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else { continue };
        let Some(raw) = parts.next() else {
            return Err(format!("line {}: missing value", index + 1));
        };
        let value = raw
            .parse::<u64>()
            .map_err(|error| format!("line {}: invalid value {raw:?}: {error}", index + 1))?;
        values.insert(key, value);
    }
    Ok(values)
}

fn read_u64(path: &Path) -> Result<u64, String> {
    let raw =
        fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    raw.trim()
        .parse::<u64>()
        .map_err(|error| format!("parse {}: {error}", path.display()))
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

fn push_gauge(snapshot: &mut Snapshot, metric: GaugeFamily, labels: &[&str], value: u64) {
    snapshot.gauges.push(GaugeSample {
        metric,
        labels: labels.iter().map(|label| (*label).to_string()).collect(),
        value: prometheus_u64(value),
    });
}

fn collect_cpu(path: &Path, cgroup: &str, snapshot: &mut Snapshot) -> Result<(), String> {
    let file = path.join("cpu.stat");
    if !file.exists() {
        return Ok(());
    }
    let content =
        fs::read_to_string(&file).map_err(|error| format!("read {}: {error}", file.display()))?;
    let values = parse_key_values(&content)?;
    for (field, kind) in [
        ("usage_usec", "usage"),
        ("user_usec", "user"),
        ("system_usec", "system"),
        ("nice_usec", "nice"),
        ("core_sched.force_idle_usec", "force_idle"),
        ("throttled_usec", "throttled"),
        ("burst_usec", "burst"),
    ] {
        if let Some(&value) = values.get(field) {
            push_counter(
                snapshot,
                CounterFamily::CpuSeconds,
                &[cgroup, kind],
                value,
                1e-6,
            );
        }
    }
    for (field, kind) in [
        ("nr_periods", "periods"),
        ("nr_throttled", "throttled"),
        ("nr_bursts", "bursts"),
    ] {
        if let Some(&value) = values.get(field) {
            push_counter(
                snapshot,
                CounterFamily::CpuPeriods,
                &[cgroup, kind],
                value,
                1.0,
            );
        }
    }
    Ok(())
}

fn collect_memory(
    path: &Path,
    cgroup: &str,
    detailed: bool,
    snapshot: &mut Snapshot,
) -> Result<(), String> {
    for (file, kind) in [
        ("memory.current", "current"),
        ("memory.peak", "peak"),
        ("memory.swap.current", "swap_current"),
        ("memory.swap.peak", "swap_peak"),
    ] {
        let file_path = path.join(file);
        if file_path.exists() {
            push_gauge(
                snapshot,
                GaugeFamily::MemoryBytes,
                &[cgroup, kind],
                read_u64(&file_path)?,
            );
        }
    }

    let events_path = path.join("memory.events");
    if events_path.exists() {
        let events = fs::read_to_string(&events_path)
            .map_err(|error| format!("read {}: {error}", events_path.display()))?;
        for (event, value) in parse_key_values(&events)? {
            push_counter(
                snapshot,
                CounterFamily::MemoryEvents,
                &[cgroup, event],
                value,
                1.0,
            );
        }
    }

    let stat_path = path.join("memory.stat");
    if detailed && stat_path.exists() {
        let stat = fs::read_to_string(&stat_path)
            .map_err(|error| format!("read {}: {error}", stat_path.display()))?;
        let values = parse_key_values(&stat)?;
        for &field in BYTE_MEMORY_FIELDS {
            if let Some(&value) = values.get(field) {
                push_gauge(
                    snapshot,
                    GaugeFamily::MemoryStatBytes,
                    &[cgroup, field],
                    value,
                );
            }
        }
    }
    Ok(())
}

fn collect_io(path: &Path, cgroup: &str, snapshot: &mut Snapshot) -> Result<(), String> {
    let file = path.join("io.stat");
    if !file.exists() {
        return Ok(());
    }
    let content =
        fs::read_to_string(&file).map_err(|error| format!("read {}: {error}", file.display()))?;
    for (index, line) in content.lines().enumerate() {
        let mut parts = line.split_whitespace();
        let Some(device) = parts.next() else { continue };
        if !device.contains(':') {
            return Err(format!(
                "{} line {}: invalid device {device:?}",
                file.display(),
                index + 1
            ));
        }
        for item in parts {
            let Some((key, raw)) = item.split_once('=') else {
                return Err(format!(
                    "{} line {}: invalid field {item:?}",
                    file.display(),
                    index + 1
                ));
            };
            let value = raw.parse::<u64>().map_err(|error| {
                format!(
                    "{} line {}: invalid value {raw:?}: {error}",
                    file.display(),
                    index + 1
                )
            })?;
            match key {
                "rbytes" => push_counter(
                    snapshot,
                    CounterFamily::IoBytes,
                    &[cgroup, device, "read"],
                    value,
                    1.0,
                ),
                "wbytes" => push_counter(
                    snapshot,
                    CounterFamily::IoBytes,
                    &[cgroup, device, "write"],
                    value,
                    1.0,
                ),
                "dbytes" => push_counter(
                    snapshot,
                    CounterFamily::IoBytes,
                    &[cgroup, device, "discard"],
                    value,
                    1.0,
                ),
                "rios" => push_counter(
                    snapshot,
                    CounterFamily::IoOperations,
                    &[cgroup, device, "read"],
                    value,
                    1.0,
                ),
                "wios" => push_counter(
                    snapshot,
                    CounterFamily::IoOperations,
                    &[cgroup, device, "write"],
                    value,
                    1.0,
                ),
                "dios" => push_counter(
                    snapshot,
                    CounterFamily::IoOperations,
                    &[cgroup, device, "discard"],
                    value,
                    1.0,
                ),
                _ => {}
            }
        }
    }
    Ok(())
}

fn set_pressure_ratios(
    snapshot: &mut Snapshot,
    cgroup: &str,
    resource: &str,
    scope: &str,
    record: &PressureRecord,
) {
    for (window, value) in [
        ("10", record.avg10),
        ("60", record.avg60),
        ("300", record.avg300),
    ] {
        let ratio = (f64::from(value) * 100.0).round() / 10_000.0;
        snapshot.gauges.push(GaugeSample {
            metric: GaugeFamily::PressureRatio,
            labels: vec![
                cgroup.to_string(),
                resource.to_string(),
                scope.to_string(),
                window.to_string(),
            ],
            value: ratio,
        });
    }
    push_counter(
        snapshot,
        CounterFamily::PressureSeconds,
        &[cgroup, resource, scope],
        record.total,
        1e-6,
    );
}

fn collect_pressure(
    path: &Path,
    cgroup: &str,
    detailed: bool,
    snapshot: &mut Snapshot,
) -> Result<(), String> {
    for (file, resource) in [
        ("cpu.pressure", "cpu"),
        ("memory.pressure", "memory"),
        ("io.pressure", "io"),
    ] {
        let pressure_path = path.join(file);
        if !pressure_path.exists() {
            continue;
        }
        let content = match fs::read_to_string(&pressure_path) {
            Ok(content) => content,
            Err(error) if error.kind() == ErrorKind::Unsupported => continue,
            Err(error) => return Err(format!("read {}: {error}", pressure_path.display())),
        };
        let (some, full) = get_pressure(Cursor::new(content.as_bytes()))
            .map_err(|error| format!("parse {}: {error}", pressure_path.display()))?;
        if detailed {
            set_pressure_ratios(snapshot, cgroup, resource, "some", &some);
            set_pressure_ratios(snapshot, cgroup, resource, "full", &full);
        } else {
            push_counter(
                snapshot,
                CounterFamily::PressureSeconds,
                &[cgroup, resource, "some"],
                some.total,
                1e-6,
            );
            push_counter(
                snapshot,
                CounterFamily::PressureSeconds,
                &[cgroup, resource, "full"],
                full.total,
                1e-6,
            );
        }
    }
    Ok(())
}

fn collect_state(path: &Path, cgroup: &str, snapshot: &mut Snapshot) -> Result<(), String> {
    let events_path = path.join("cgroup.events");
    let content = fs::read_to_string(&events_path)
        .map_err(|error| format!("read {}: {error}", events_path.display()))?;
    for (state, value) in parse_key_values(&content)? {
        push_gauge(snapshot, GaugeFamily::State, &[cgroup, state], value);
    }
    let pids_path = path.join("pids.current");
    if pids_path.exists() {
        push_gauge(
            snapshot,
            GaugeFamily::PidsCurrent,
            &[cgroup],
            read_u64(&pids_path)?,
        );
    }
    Ok(())
}

fn collect_unit(
    base: &Path,
    path: &Path,
    detailed: bool,
    snapshot: &mut Snapshot,
) -> Result<(), String> {
    let cgroup = relative_cgroup(base, path)?;
    collect_cpu(path, &cgroup, snapshot)?;
    collect_memory(path, &cgroup, detailed, snapshot)?;
    collect_io(path, &cgroup, snapshot)?;
    collect_pressure(path, &cgroup, detailed, snapshot)?;
    collect_state(path, &cgroup, snapshot)?;
    Ok(())
}

fn update_metrics_from_path(base: &Path, config: &AppConfig) -> CollectionReport {
    let services =
        match discover_service_cgroups(base, &config.cgroup_roots, config.cgroup_max_units) {
            Ok(services) => services,
            Err(error) => {
                if debug_enabled() {
                    eprintln!("cgroup: discovery failed: {error}");
                }
                let mut report = metrics().apply(Snapshot::default());
                report.record_error();
                return report;
            }
        };

    let mut report = CollectionReport::success();
    let mut snapshot = Snapshot {
        units: services.len(),
        ..Snapshot::default()
    };
    for service in services {
        let mut unit_snapshot = Snapshot::default();
        match collect_unit(
            base,
            &service,
            config.cgroup_detailed_metrics,
            &mut unit_snapshot,
        ) {
            Ok(()) => {
                snapshot.counters.extend(unit_snapshot.counters);
                snapshot.gauges.extend(unit_snapshot.gauges);
            }
            Err(error) => {
                // A unit may legitimately disappear between discovery and
                // reading its controller files. Do not turn normal systemd
                // churn into a failed host scrape.
                if service.exists() {
                    report.record_error();
                    if debug_enabled() {
                        eprintln!("cgroup: {}: {error}", service.display());
                    }
                }
            }
        }
    }
    report.merge(metrics().apply(snapshot));
    report
}

pub fn update_metrics(config: &AppConfig) -> CollectionReport {
    update_metrics_from_path(Path::new(CGROUP_ROOT), config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn discovers_only_service_cgroups_recursively() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("system.slice/a.service/internal")).unwrap();
        fs::create_dir_all(dir.path().join("system.slice/nested.slice/b.service")).unwrap();
        fs::create_dir_all(
            dir.path()
                .join("system.slice/transient.scope/hidden.service"),
        )
        .unwrap();
        let found =
            discover_service_cgroups(dir.path(), &["system.slice".to_string()], 10).unwrap();
        let relative: Vec<_> = found
            .iter()
            .map(|path| relative_cgroup(dir.path(), path).unwrap())
            .collect();
        assert_eq!(
            relative,
            vec![
                "system.slice/a.service",
                "system.slice/nested.slice/b.service"
            ]
        );
    }

    #[test]
    fn overlapping_roots_do_not_duplicate_services() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("system.slice/nested.slice/a.service")).unwrap();
        let roots = vec![
            "system.slice".to_string(),
            "system.slice/nested.slice".to_string(),
        ];
        let found = discover_service_cgroups(dir.path(), &roots, 10).unwrap();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn discovery_cap_prevents_label_explosion() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("system.slice/a.service")).unwrap();
        fs::create_dir_all(dir.path().join("system.slice/b.service")).unwrap();
        assert!(discover_service_cgroups(dir.path(), &["system.slice".to_string()], 1).is_err());
    }

    #[test]
    fn missing_optional_controller_files_are_not_errors() {
        let dir = TempDir::new().unwrap();
        let service = dir.path().join("system.slice/minimal.service");
        fs::create_dir_all(&service).unwrap();
        fs::write(service.join("cgroup.events"), "populated 1\nfrozen 0\n").unwrap();

        let mut snapshot = Snapshot::default();
        collect_unit(dir.path(), &service, false, &mut snapshot).unwrap();
        assert!(snapshot.counters.is_empty());
        assert_eq!(snapshot.gauges.len(), 2);
    }

    #[test]
    fn monotonic_delta_handles_counter_reset() {
        assert_eq!(monotonic_delta(10, 15), 5);
        assert_eq!(monotonic_delta(10, 3), 3);
    }

    #[test]
    fn detailed_mode_controls_memory_breakdown() {
        let dir = TempDir::new().unwrap();
        let service = dir.path().join("test.service");
        fs::create_dir_all(&service).unwrap();
        fs::write(service.join("memory.events"), "oom 1\n").unwrap();
        fs::write(service.join("memory.stat"), "anon 123\npgfault 7\n").unwrap();

        let mut summary = Snapshot::default();
        collect_memory(&service, "test.service", false, &mut summary).unwrap();
        assert_eq!(summary.counters.len(), 1);
        assert!(summary.gauges.is_empty());

        let mut detailed = Snapshot::default();
        collect_memory(&service, "test.service", true, &mut detailed).unwrap();
        assert_eq!(detailed.counters.len(), 1);
        assert_eq!(
            detailed
                .gauges
                .iter()
                .filter(|sample| matches!(sample.metric, GaugeFamily::MemoryStatBytes))
                .count(),
            1
        );
    }

    #[test]
    fn detailed_mode_controls_pressure_ratios_not_totals() {
        let dir = TempDir::new().unwrap();
        let service = dir.path().join("test.service");
        fs::create_dir_all(&service).unwrap();
        let pressure = "some avg10=1.00 avg60=2.00 avg300=3.00 total=1000000\nfull avg10=0.50 avg60=1.00 avg300=1.50 total=500000\n";
        for file in ["cpu.pressure", "memory.pressure", "io.pressure"] {
            fs::write(service.join(file), pressure).unwrap();
        }

        let mut summary = Snapshot::default();
        collect_pressure(&service, "test.service", false, &mut summary).unwrap();
        assert_eq!(summary.counters.len(), 6);
        assert!(summary.gauges.is_empty());

        let mut detailed = Snapshot::default();
        collect_pressure(&service, "test.service", true, &mut detailed).unwrap();
        assert_eq!(detailed.counters.len(), 6);
        assert_eq!(detailed.gauges.len(), 18);
    }

    #[test]
    fn parses_cpu_key_value_file() {
        let parsed = parse_key_values("usage_usec 12\nnr_periods 3\n").unwrap();
        assert_eq!(parsed.get("usage_usec"), Some(&12));
        assert_eq!(parsed.get("nr_periods"), Some(&3));
    }
}
