use crate::config::{AppConfig, Datasource};
use crate::{
    datasource_conntrack, datasource_cpufreq, datasource_edac, datasource_filesystems,
    datasource_hwmon, datasource_ipmi, datasource_mdraid, datasource_netdev_sysfs, datasource_numa,
    datasource_nvme, datasource_power_supply, datasource_procfs, datasource_rapl,
    datasource_softnet, datasource_thermal,
};

pub fn update_metrics(config: &AppConfig) {
    if config.is_datasource_enabled(Datasource::Procfs) {
        datasource_procfs::update_metrics(config);
    }
    if config.is_datasource_enabled(Datasource::CpuFreq) {
        datasource_cpufreq::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::Softnet) {
        datasource_softnet::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::Conntrack) {
        datasource_conntrack::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::Filesystems) {
        datasource_filesystems::update_metrics(config);
    }
    if config.is_datasource_enabled(Datasource::Hwmon) {
        datasource_hwmon::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::Ipmi) {
        datasource_ipmi::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::Mdraid) {
        datasource_mdraid::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::Thermal) {
        datasource_thermal::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::Rapl) {
        datasource_rapl::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::PowerSupply) {
        datasource_power_supply::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::Nvme) {
        datasource_nvme::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::Edac) {
        datasource_edac::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::NetdevSysfs) {
        datasource_netdev_sysfs::update_metrics(config);
    }
    if config.is_datasource_enabled(Datasource::Numa) {
        datasource_numa::update_metrics();
    }
}
