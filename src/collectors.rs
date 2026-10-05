use crate::collection::CollectionReport;
use crate::config::{AppConfig, Datasource};
use crate::metric_support::RegisterMetricResultExt;
use crate::{
    datasource_cgroup, datasource_conntrack, datasource_cpufreq, datasource_edac,
    datasource_filefd, datasource_filesystems, datasource_hwmon, datasource_ipmi,
    datasource_mdraid, datasource_netdev_sysfs, datasource_numa, datasource_nvme,
    datasource_power_supply, datasource_pressure, datasource_procfs, datasource_rapl,
    datasource_sccache, datasource_schedstat, datasource_softnet, datasource_systemd,
    datasource_thermal, datasource_zfs,
};
use prometheus::GaugeVec;
use std::sync::OnceLock;
use std::time::Instant;

struct CollectorMetrics {
    duration_seconds: GaugeVec,
    success: GaugeVec,
}

impl CollectorMetrics {
    fn new() -> Self {
        Self {
            duration_seconds: prometheus::register_gauge_vec!(
                "node_scrape_collector_duration_seconds",
                "node_exporter: Duration of a collector scrape.",
                &["collector"]
            )
            .or_exit("node_scrape_collector_duration_seconds"),
            success: prometheus::register_gauge_vec!(
                "node_scrape_collector_success",
                "node_exporter: Whether a collector succeeded.",
                &["collector"]
            )
            .or_exit("node_scrape_collector_success"),
        }
    }
}

static COLLECTOR_METRICS: OnceLock<CollectorMetrics> = OnceLock::new();

fn metrics() -> &'static CollectorMetrics {
    COLLECTOR_METRICS.get_or_init(CollectorMetrics::new)
}

fn run_collector(datasource: Datasource, collect: impl FnOnce() -> CollectionReport) {
    let started = Instant::now();
    let report = collect();
    let labels = [datasource.as_str()];

    metrics()
        .duration_seconds
        .with_label_values(&labels)
        .set(started.elapsed().as_secs_f64());
    metrics()
        .success
        .with_label_values(&labels)
        .set(if report.is_success() { 1.0 } else { 0.0 });

    if !report.is_success() {
        eprintln!(
            "collector {} completed with {} collection error(s)",
            datasource.as_str(),
            report.error_count()
        );
    }
}

fn run_if_enabled(
    config: &AppConfig,
    datasource: Datasource,
    collect: impl FnOnce() -> CollectionReport,
) {
    if config.is_datasource_enabled(datasource) {
        run_collector(datasource, collect);
    }
}

pub fn update_metrics(config: &AppConfig) {
    run_if_enabled(config, Datasource::Systemd, || {
        datasource_systemd::update_metrics(config)
    });
    run_if_enabled(config, Datasource::Sccache, || {
        datasource_sccache::update_metrics(config)
    });
    run_if_enabled(config, Datasource::Cgroup, || {
        datasource_cgroup::update_metrics(config)
    });
    run_if_enabled(config, Datasource::Procfs, || {
        datasource_procfs::update_metrics(config)
    });
    run_if_enabled(
        config,
        Datasource::Filefd,
        datasource_filefd::update_metrics,
    );
    run_if_enabled(
        config,
        Datasource::Schedstat,
        datasource_schedstat::update_metrics,
    );
    run_if_enabled(
        config,
        Datasource::CpuFreq,
        datasource_cpufreq::update_metrics,
    );
    run_if_enabled(
        config,
        Datasource::Softnet,
        datasource_softnet::update_metrics,
    );
    run_if_enabled(
        config,
        Datasource::Conntrack,
        datasource_conntrack::update_metrics,
    );
    run_if_enabled(config, Datasource::Filesystems, || {
        datasource_filesystems::update_metrics(config)
    });
    run_if_enabled(config, Datasource::Hwmon, datasource_hwmon::update_metrics);
    run_if_enabled(config, Datasource::Ipmi, datasource_ipmi::update_metrics);
    run_if_enabled(
        config,
        Datasource::Mdraid,
        datasource_mdraid::update_metrics,
    );
    run_if_enabled(
        config,
        Datasource::Thermal,
        datasource_thermal::update_metrics,
    );
    run_if_enabled(config, Datasource::Rapl, datasource_rapl::update_metrics);
    run_if_enabled(
        config,
        Datasource::PowerSupply,
        datasource_power_supply::update_metrics,
    );
    run_if_enabled(config, Datasource::Nvme, datasource_nvme::update_metrics);
    run_if_enabled(config, Datasource::Edac, datasource_edac::update_metrics);
    run_if_enabled(config, Datasource::NetdevSysfs, || {
        datasource_netdev_sysfs::update_metrics(config)
    });
    run_if_enabled(config, Datasource::Numa, datasource_numa::update_metrics);
    run_if_enabled(
        config,
        Datasource::Pressure,
        datasource_pressure::update_metrics,
    );
    run_if_enabled(config, Datasource::Zfs, datasource_zfs::update_metrics);
}
