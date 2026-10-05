use crate::collection::CollectionReport;
use crate::config::AppConfig;
use crate::metric_support::{RegisterMetricResultExt, prometheus_u64};
use crate::runtime::debug_enabled;
use prometheus::{CounterVec, Gauge, GaugeVec};
use regex::Regex;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::thread;
use systemd_zbus::{
    ManagerProxyBlocking, MountProxyBlocking, ServiceProxyBlocking, SocketProxyBlocking,
    TimerProxyBlocking, UnitProxyBlocking,
};
use zbus::blocking::Connection;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;

const PROPERTY_WORKERS: usize = 16;
const KNOWN_ACTIVE_STATES: &[&str] =
    &["active", "activating", "deactivating", "inactive", "failed"];

type RawUnit = (
    String,
    String,
    String,
    String,
    String,
    String,
    OwnedObjectPath,
    u32,
    String,
    OwnedObjectPath,
);

#[derive(Debug, Clone, PartialEq, Eq)]
struct UnitInfo {
    name: String,
    active_state: String,
    unit_class: String,
    state_type: String,
    path: OwnedObjectPath,
}

#[derive(Debug, Default)]
struct Snapshot {
    version: String,
    virtualization: String,
    system_state: String,
    units: Vec<UnitInfo>,
    services: Vec<ServiceStatus>,
    service_runtime: Vec<ServiceRuntime>,
    timers: Vec<TimerDetails>,
    sockets: Vec<SocketDetails>,
}

#[derive(Debug, Default)]
struct ServiceStatus {
    name: String,
    restarts: u64,
    service_type: Option<String>,
}

#[derive(Debug, Default)]
struct MountStatus {
    name: String,
    mount_type: Option<String>,
}

#[derive(Debug, Default)]
struct ServiceRuntime {
    name: String,
    start_time_seconds: Option<f64>,
    tasks_current: Option<u64>,
    tasks_max: Option<u64>,
}

#[derive(Debug, Default)]
struct TimerDetails {
    name: String,
    last_trigger_seconds: f64,
}

#[derive(Debug, Default)]
struct SocketDetails {
    name: String,
    accepted: u64,
    current: u64,
    refused: u64,
}

struct SystemdMetrics {
    unit_state: GaugeVec,
    units: GaugeVec,
    system_running: Gauge,
    version: GaugeVec,
    virtualization_info: GaugeVec,
    service_restarts: CounterVec,
    unit_start_time_seconds: GaugeVec,
    service_tasks_current: GaugeVec,
    service_tasks_max: GaugeVec,
    timer_last_trigger_seconds: GaugeVec,
    socket_accepted_connections: CounterVec,
    socket_current_connections: GaugeVec,
    socket_refused_connections: CounterVec,
}

impl SystemdMetrics {
    fn new() -> Self {
        Self {
            unit_state: prometheus::register_gauge_vec!(
                "systemd_unit_state",
                "Systemd unit state (1 for the current state).",
                &["name", "state", "type"]
            )
            .or_exit("systemd_unit_state"),
            units: prometheus::register_gauge_vec!(
                "systemd_units",
                "Number of systemd units by active state.",
                &["state"]
            )
            .or_exit("systemd_units"),
            system_running: prometheus::register_gauge!(
                "systemd_system_running",
                "Whether systemd reports the system state as running (1 = running)."
            )
            .or_exit("systemd_system_running"),
            version: prometheus::register_gauge_vec!(
                "systemd_version",
                "Detected systemd version; sample value is the numeric systemd version.",
                &["version"]
            )
            .or_exit("systemd_version"),
            virtualization_info: prometheus::register_gauge_vec!(
                "systemd_virtualization_info",
                "Systemd-detected virtualization technology.",
                &["virtualization_type"]
            )
            .or_exit("systemd_virtualization_info"),
            service_restarts: prometheus::register_counter_vec!(
                "systemd_service_restart_total",
                "Service unit count of automatic restart triggers.",
                &["name"]
            )
            .or_exit("systemd_service_restart_total"),
            unit_start_time_seconds: prometheus::register_gauge_vec!(
                "systemd_unit_start_time_seconds",
                "Start time of a service unit since the Unix epoch in seconds.",
                &["name"]
            )
            .or_exit("systemd_unit_start_time_seconds"),
            service_tasks_current: prometheus::register_gauge_vec!(
                "systemd_unit_tasks_current",
                "Current number of tasks in a systemd service when known.",
                &["name"]
            )
            .or_exit("systemd_unit_tasks_current"),
            service_tasks_max: prometheus::register_gauge_vec!(
                "systemd_unit_tasks_max",
                "Configured task limit for a systemd service when finite/known.",
                &["name"]
            )
            .or_exit("systemd_unit_tasks_max"),
            timer_last_trigger_seconds: prometheus::register_gauge_vec!(
                "systemd_timer_last_trigger_seconds",
                "Seconds since the Unix epoch of the last systemd timer trigger.",
                &["name"]
            )
            .or_exit("systemd_timer_last_trigger_seconds"),
            socket_accepted_connections: prometheus::register_counter_vec!(
                "systemd_socket_accepted_connections_total",
                "Total number of accepted socket connections.",
                &["name"]
            )
            .or_exit("systemd_socket_accepted_connections_total"),
            socket_current_connections: prometheus::register_gauge_vec!(
                "systemd_socket_current_connections",
                "Current number of socket connections.",
                &["name"]
            )
            .or_exit("systemd_socket_current_connections"),
            socket_refused_connections: prometheus::register_counter_vec!(
                "systemd_socket_refused_connections_total",
                "Total number of refused socket connections.",
                &["name"]
            )
            .or_exit("systemd_socket_refused_connections_total"),
        }
    }

    fn clear_dynamic(&self) {
        self.unit_state.reset();
        self.units.reset();
        self.version.reset();
        self.virtualization_info.reset();
        self.service_restarts.reset();
        self.unit_start_time_seconds.reset();
        self.service_tasks_current.reset();
        self.service_tasks_max.reset();
        self.timer_last_trigger_seconds.reset();
        self.socket_accepted_connections.reset();
        self.socket_current_connections.reset();
        self.socket_refused_connections.reset();
    }

    fn mark_unavailable(&self) {
        self.clear_dynamic();
        self.system_running.set(f64::NAN);
    }

    fn apply(&self, snapshot: &Snapshot) {
        self.clear_dynamic();
        self.system_running
            .set(if snapshot.system_state == "running" {
                1.0
            } else {
                0.0
            });
        self.version
            .with_label_values(&[&snapshot.version])
            .set(systemd_version_value(&snapshot.version));
        self.virtualization_info
            .with_label_values(&[virtualization_label(&snapshot.virtualization)])
            .set(1.0);

        let mut state_counts = KNOWN_ACTIVE_STATES
            .iter()
            .map(|state| ((*state).to_string(), 0_u64))
            .collect::<BTreeMap<_, _>>();
        for unit in &snapshot.units {
            *state_counts.entry(unit.active_state.clone()).or_default() += 1;
            for state in KNOWN_ACTIVE_STATES {
                self.unit_state
                    .with_label_values(&[unit.name.as_str(), state, unit.state_type.as_str()])
                    .set(if unit.active_state == *state {
                        1.0
                    } else {
                        0.0
                    });
            }
            if !KNOWN_ACTIVE_STATES.contains(&unit.active_state.as_str()) {
                self.unit_state
                    .with_label_values(&[
                        unit.name.as_str(),
                        unit.active_state.as_str(),
                        unit.state_type.as_str(),
                    ])
                    .set(1.0);
            }
        }
        for (state, count) in state_counts {
            self.units
                .with_label_values(&[state.as_str()])
                .set(prometheus_u64(count));
        }

        for status in &snapshot.services {
            self.service_restarts
                .with_label_values(&[&status.name])
                .inc_by(prometheus_u64(status.restarts));
        }
        for detail in &snapshot.service_runtime {
            if let Some(value) = detail.start_time_seconds {
                self.unit_start_time_seconds
                    .with_label_values(&[&detail.name])
                    .set(value);
            }
            if let Some(value) = detail.tasks_current {
                self.service_tasks_current
                    .with_label_values(&[&detail.name])
                    .set(prometheus_u64(value));
            }
            if let Some(value) = detail.tasks_max {
                self.service_tasks_max
                    .with_label_values(&[&detail.name])
                    .set(prometheus_u64(value));
            }
        }
        for detail in &snapshot.timers {
            self.timer_last_trigger_seconds
                .with_label_values(&[&detail.name])
                .set(detail.last_trigger_seconds);
        }
        for detail in &snapshot.sockets {
            self.socket_accepted_connections
                .with_label_values(&[&detail.name])
                .inc_by(prometheus_u64(detail.accepted));
            self.socket_current_connections
                .with_label_values(&[&detail.name])
                .set(prometheus_u64(detail.current));
            self.socket_refused_connections
                .with_label_values(&[&detail.name])
                .inc_by(prometheus_u64(detail.refused));
        }
    }
}

static SYSTEMD_METRICS: OnceLock<SystemdMetrics> = OnceLock::new();

fn metrics() -> &'static SystemdMetrics {
    SYSTEMD_METRICS.get_or_init(SystemdMetrics::new)
}

fn unit_class(name: &str) -> &str {
    name.rsplit_once('.')
        .map_or("unknown", |(_, suffix)| suffix)
}

fn known_u64(value: u64) -> Option<u64> {
    (value != u64::MAX).then_some(value)
}

fn usec_to_seconds(value: u64) -> Option<f64> {
    (value != u64::MAX).then_some(prometheus_u64(value) / 1_000_000.0)
}

fn systemd_version_value(version: &str) -> f64 {
    let Ok(regex) = Regex::new(r"[0-9]{3,}(?:\.[0-9]+)?") else {
        return 0.0;
    };
    regex
        .find(version)
        .and_then(|matched| matched.as_str().parse::<f64>().ok())
        .unwrap_or(0.0)
}

const fn virtualization_label(value: &str) -> &str {
    if value.is_empty() { "none" } else { value }
}

fn compile_filter(pattern: &str) -> Result<Regex, regex::Error> {
    Regex::new(&format!(r"^(?:{pattern})$"))
}

fn filter_units(
    raw: Vec<RawUnit>,
    include: &Regex,
    exclude: &Regex,
    max_units: usize,
) -> Result<Vec<UnitInfo>, String> {
    let mut units = raw
        .into_iter()
        .filter(|unit| {
            unit.2 == "loaded" && include.is_match(&unit.0) && !exclude.is_match(&unit.0)
        })
        .map(|unit| UnitInfo {
            unit_class: unit_class(&unit.0).to_string(),
            state_type: String::new(),
            name: unit.0,
            active_state: unit.3,
            path: unit.6,
        })
        .collect::<Vec<_>>();
    units.sort_unstable_by(|a, b| a.name.cmp(&b.name));
    if units.len() > max_units {
        return Err(format!(
            "systemd matched {} loaded units, exceeding systemd_max_units={max_units}",
            units.len()
        ));
    }
    Ok(units)
}

fn optional_property_error(error: &zbus::Error) -> bool {
    let message = error.to_string();
    message.contains("UnknownObject")
        || message.contains("UnknownInterface")
        || message.contains("UnknownProperty")
        || message.contains("NoSuchUnit")
}

fn service_proxy<'a>(
    conn: &'a Connection,
    unit: &'a UnitInfo,
) -> Result<ServiceProxyBlocking<'a>, zbus::Error> {
    ServiceProxyBlocking::builder(conn)
        .path(unit.path.clone())?
        .cache_properties(CacheProperties::No)
        .build()
}

fn build_service_status(conn: &Connection, unit: &UnitInfo) -> Result<ServiceStatus, zbus::Error> {
    let proxy = service_proxy(conn, unit)?;
    Ok(ServiceStatus {
        name: unit.name.clone(),
        restarts: u64::from(proxy.n_restarts()?),
        service_type: proxy.kind().ok(),
    })
}

fn build_mount_status(conn: &Connection, unit: &UnitInfo) -> Result<MountStatus, zbus::Error> {
    let proxy = MountProxyBlocking::builder(conn)
        .path(unit.path.clone())?
        .cache_properties(CacheProperties::No)
        .build()?;
    Ok(MountStatus {
        name: unit.name.clone(),
        mount_type: proxy.kind().ok(),
    })
}

fn build_service_runtime(
    conn: &Connection,
    unit: &UnitInfo,
) -> Result<ServiceRuntime, zbus::Error> {
    let proxy = service_proxy(conn, unit)?;
    let base = UnitProxyBlocking::builder(conn)
        .path(unit.path.clone())?
        .cache_properties(CacheProperties::No)
        .build()?;
    Ok(ServiceRuntime {
        name: unit.name.clone(),
        start_time_seconds: usec_to_seconds(base.active_enter_timestamp()?),
        tasks_current: known_u64(proxy.tasks_current()?),
        tasks_max: known_u64(proxy.tasks_max()?),
    })
}

fn build_timer_detail(conn: &Connection, unit: &UnitInfo) -> Result<TimerDetails, zbus::Error> {
    let proxy = TimerProxyBlocking::builder(conn)
        .path(unit.path.clone())?
        .cache_properties(CacheProperties::No)
        .build()?;
    Ok(TimerDetails {
        name: unit.name.clone(),
        last_trigger_seconds: usec_to_seconds(proxy.last_trigger_usec()?).unwrap_or(0.0),
    })
}

fn build_socket_detail(conn: &Connection, unit: &UnitInfo) -> Result<SocketDetails, zbus::Error> {
    let proxy = SocketProxyBlocking::builder(conn)
        .path(unit.path.clone())?
        .cache_properties(CacheProperties::No)
        .build()?;
    Ok(SocketDetails {
        name: unit.name.clone(),
        accepted: u64::from(proxy.n_accepted()?),
        current: u64::from(proxy.n_connections()?),
        refused: u64::from(proxy.n_refused()?),
    })
}

fn parallel_collect<T, F>(
    conn: &Connection,
    units: &[UnitInfo],
    collect: F,
) -> (Vec<T>, CollectionReport)
where
    T: Send,
    F: Fn(&Connection, &UnitInfo) -> Result<T, zbus::Error> + Sync,
{
    if units.is_empty() {
        return (Vec::new(), CollectionReport::success());
    }
    let next = AtomicUsize::new(0);
    let output = Mutex::new(Vec::with_capacity(units.len()));
    let errors = Mutex::new(Vec::<String>::new());
    let workers = PROPERTY_WORKERS.min(units.len());
    thread::scope(|scope| {
        for _ in 0..workers {
            let conn = conn.clone();
            let next = &next;
            let output = &output;
            let errors = &errors;
            let collect = &collect;
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(unit) = units.get(index) else {
                        break;
                    };
                    match collect(&conn, unit) {
                        Ok(value) => output
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push(value),
                        Err(error) if optional_property_error(&error) => {}
                        Err(error) => errors
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .push(format!("{}: {error}", unit.name)),
                    }
                }
            });
        }
    });

    let errors = errors.into_inner().unwrap_or_else(PoisonError::into_inner);
    let mut report = CollectionReport::success();
    for error in &errors {
        report.record_error();
        if debug_enabled() {
            eprintln!("systemd: property collection failed: {error}");
        }
    }
    (
        output.into_inner().unwrap_or_else(PoisonError::into_inner),
        report,
    )
}

fn collect_snapshot(config: &AppConfig) -> Result<(Snapshot, CollectionReport), String> {
    let include = compile_filter(&config.systemd_unit_include)
        .map_err(|error| format!("invalid systemd include regex: {error}"))?;
    let exclude = compile_filter(&config.systemd_unit_exclude)
        .map_err(|error| format!("invalid systemd exclude regex: {error}"))?;
    let conn = Connection::system().map_err(|error| format!("connect system bus: {error}"))?;
    let manager = ManagerProxyBlocking::builder(&conn)
        .cache_properties(CacheProperties::No)
        .build()
        .map_err(|error| format!("create systemd manager proxy: {error}"))?;

    let version = manager
        .version()
        .map_err(|error| format!("read systemd version: {error}"))?;
    let virtualization = manager
        .virtualization()
        .map_err(|error| format!("read systemd virtualization: {error}"))?;
    let system_state = manager
        .system_state()
        .map_err(|error| format!("read systemd system state: {error}"))?;
    let raw: Vec<RawUnit> = manager
        .inner()
        .call("ListUnits", &())
        .map_err(|error| format!("ListUnits: {error}"))?;
    let mut units = filter_units(raw, &include, &exclude, config.systemd_max_units)?;

    let service_units = units
        .iter()
        .filter(|unit| unit.unit_class == "service")
        .cloned()
        .collect::<Vec<_>>();
    let mount_units = units
        .iter()
        .filter(|unit| unit.unit_class == "mount")
        .cloned()
        .collect::<Vec<_>>();
    let timer_units = units
        .iter()
        .filter(|unit| unit.unit_class == "timer")
        .cloned()
        .collect::<Vec<_>>();
    let socket_units = units
        .iter()
        .filter(|unit| unit.unit_class == "socket")
        .cloned()
        .collect::<Vec<_>>();

    let (services, mut report) = parallel_collect(&conn, &service_units, build_service_status);
    let (mounts, mount_report) = parallel_collect(&conn, &mount_units, build_mount_status);
    report.merge(mount_report);
    let mut state_types = services
        .iter()
        .filter_map(|status| {
            status
                .service_type
                .as_deref()
                .map(|kind| (status.name.as_str(), kind))
        })
        .collect::<HashMap<_, _>>();
    state_types.extend(mounts.iter().filter_map(|status| {
        status
            .mount_type
            .as_deref()
            .map(|kind| (status.name.as_str(), kind))
    }));
    for unit in &mut units {
        if let Some(kind) = state_types.get(unit.name.as_str()) {
            unit.state_type = (*kind).to_string();
        }
    }

    let (timers, timer_report) = parallel_collect(&conn, &timer_units, build_timer_detail);
    let (sockets, socket_report) = parallel_collect(&conn, &socket_units, build_socket_detail);
    report.merge(timer_report);
    report.merge(socket_report);

    let service_runtime = if config.systemd_detailed_metrics {
        let (details, detail_report) =
            parallel_collect(&conn, &service_units, build_service_runtime);
        report.merge(detail_report);
        details
    } else {
        Vec::new()
    };

    Ok((
        Snapshot {
            version,
            virtualization,
            system_state,
            units,
            services,
            service_runtime,
            timers,
            sockets,
        },
        report,
    ))
}

pub fn update_metrics(config: &AppConfig) -> CollectionReport {
    match collect_snapshot(config) {
        Ok((snapshot, report)) => {
            metrics().apply(&snapshot);
            report
        }
        Err(error) => {
            if debug_enabled() {
                eprintln!("systemd: collection failed: {error}");
            }
            metrics().mark_unavailable();
            CollectionReport::error()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        KNOWN_ACTIVE_STATES, RawUnit, Snapshot, UnitInfo, compile_filter, filter_units, known_u64,
        metrics, systemd_version_value, unit_class, usec_to_seconds, virtualization_label,
    };
    use zbus::zvariant::OwnedObjectPath;

    fn raw_unit(name: &str, load: &str, state: &str) -> RawUnit {
        (
            name.to_string(),
            "desc".to_string(),
            load.to_string(),
            state.to_string(),
            "running".to_string(),
            String::new(),
            OwnedObjectPath::try_from("/org/freedesktop/systemd1/unit/test_2eservice").unwrap(),
            0,
            String::new(),
            OwnedObjectPath::try_from("/").unwrap(),
        )
    }

    #[test]
    fn unit_class_uses_suffix() {
        assert_eq!(unit_class("sshd.service"), "service");
        assert_eq!(unit_class("foo.timer"), "timer");
        assert_eq!(unit_class("unexpected"), "unknown");
    }

    #[test]
    fn systemd_unknown_sentinels_are_not_exported() {
        assert_eq!(known_u64(u64::MAX), None);
        assert_eq!(known_u64(42), Some(42));
        assert_eq!(usec_to_seconds(0), Some(0.0));
        assert_eq!(usec_to_seconds(u64::MAX), None);
        assert_eq!(usec_to_seconds(2_500_000), Some(2.5));
    }

    #[test]
    fn default_style_filter_excludes_noisy_and_unloaded_units() {
        let include = compile_filter(r".+").unwrap();
        let exclude = compile_filter(r".+\.(automount|device|mount|scope|slice)").unwrap();
        let units = vec![
            raw_unit("a.service", "loaded", "active"),
            raw_unit("b.timer", "loaded", "inactive"),
            raw_unit("dev-sda.device", "loaded", "active"),
            raw_unit("system.slice", "loaded", "active"),
            raw_unit("missing.service", "not-found", "inactive"),
        ];
        let filtered = filter_units(units, &include, &exclude, 10).unwrap();
        assert_eq!(
            filtered
                .iter()
                .map(|unit| unit.name.as_str())
                .collect::<Vec<_>>(),
            vec!["a.service", "b.timer"]
        );
    }

    #[test]
    fn filter_patterns_are_anchored_like_node_exporter() {
        let include = compile_filter("foo").unwrap();
        assert!(include.is_match("foo"));
        assert!(!include.is_match("foo.service"));
        assert!(!include.is_match("prefix-foo"));
    }

    #[test]
    fn unit_cap_fails_before_exporting_partial_set() {
        let include = compile_filter(r".+").unwrap();
        let exclude = compile_filter(r"$^").unwrap();
        let error = filter_units(
            vec![
                raw_unit("a.service", "loaded", "active"),
                raw_unit("b.service", "loaded", "active"),
            ],
            &include,
            &exclude,
            1,
        )
        .unwrap_err();
        assert!(error.contains("systemd_max_units=1"));
    }

    #[test]
    fn unavailable_collection_removes_stale_systemd_series() {
        let snapshot = Snapshot {
            version: "262-test".to_string(),
            virtualization: String::new(),
            system_state: "running".to_string(),
            units: vec![UnitInfo {
                name: "example.service".to_string(),
                active_state: "active".to_string(),
                unit_class: "service".to_string(),
                state_type: "simple".to_string(),
                path: OwnedObjectPath::try_from("/org/freedesktop/systemd1/unit/example_2eservice")
                    .unwrap(),
            }],
            ..Snapshot::default()
        };
        let metrics = metrics();
        metrics.apply(&snapshot);
        let series_count = |families: Vec<prometheus::proto::MetricFamily>| {
            families
                .iter()
                .map(|family| family.get_metric().len())
                .sum::<usize>()
        };
        assert_eq!(
            series_count(prometheus::core::Collector::collect(&metrics.unit_state)),
            KNOWN_ACTIVE_STATES.len()
        );
        assert_eq!(
            series_count(prometheus::core::Collector::collect(&metrics.version)),
            1
        );
        assert_eq!(
            series_count(prometheus::core::Collector::collect(
                &metrics.virtualization_info
            )),
            1
        );
        assert_eq!(metrics.system_running.get(), 1.0);

        metrics.mark_unavailable();
        assert_eq!(
            series_count(prometheus::core::Collector::collect(&metrics.unit_state)),
            0
        );
        assert_eq!(
            series_count(prometheus::core::Collector::collect(&metrics.version)),
            0
        );
        assert_eq!(
            series_count(prometheus::core::Collector::collect(
                &metrics.virtualization_info
            )),
            0
        );
        assert!(metrics.system_running.get().is_nan());
    }

    #[test]
    fn active_state_catalog_matches_node_exporter_baseline() {
        assert_eq!(
            KNOWN_ACTIVE_STATES,
            &["active", "activating", "deactivating", "inactive", "failed"]
        );
    }

    #[test]
    fn version_and_virtualization_match_node_exporter_semantics() {
        assert_eq!(systemd_version_value("262-1-arch"), 262.0);
        assert_eq!(systemd_version_value("255.4 (255.4-1)"), 255.4);
        assert_eq!(systemd_version_value("unknown"), 0.0);
        assert_eq!(virtualization_label(""), "none");
        assert_eq!(virtualization_label("kvm"), "kvm");
    }
}
