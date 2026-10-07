use ipnet::IpNet;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path};
use std::str::FromStr;
use subtle::ConstantTimeEq;

/// Read relative to the working directory; the packaged unit sets
/// WorkingDirectory=/etc/rs-linux-exporter.
const CONFIG_PATH: &str = "config.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Datasource {
    Procfs,
    Cgroup,
    #[serde(rename = "cpufreq")]
    CpuFreq,
    Softnet,
    Conntrack,
    Filefd,
    KernelHung,
    Watchdog,
    Uname,
    Os,
    Dmi,
    Timex,
    Time,
    Filesystems,
    Hwmon,
    Ipmi,
    Mdraid,
    Thermal,
    Rapl,
    PowerSupply,
    Pressure,
    Nvme,
    Edac,
    NetdevSysfs,
    Nfs,
    Ntp,
    Probe,
    Nfsd,
    Numa,
    Zfs,
    Sccache,
    Schedstat,
    Systemd,
    Tailscale,
}

impl Datasource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Procfs => "procfs",
            Self::Cgroup => "cgroup",
            Self::CpuFreq => "cpufreq",
            Self::Softnet => "softnet",
            Self::Conntrack => "conntrack",
            Self::Filefd => "filefd",
            Self::KernelHung => "kernel_hung",
            Self::Watchdog => "watchdog",
            Self::Uname => "uname",
            Self::Os => "os",
            Self::Dmi => "dmi",
            Self::Timex => "timex",
            Self::Time => "time",
            Self::Filesystems => "filesystems",
            Self::Hwmon => "hwmon",
            Self::Ipmi => "ipmi",
            Self::Mdraid => "mdraid",
            Self::Thermal => "thermal",
            Self::Rapl => "rapl",
            Self::PowerSupply => "power_supply",
            Self::Pressure => "pressure",
            Self::Nvme => "nvme",
            Self::Edac => "edac",
            Self::NetdevSysfs => "netdev_sysfs",
            Self::Nfs => "nfs",
            Self::Ntp => "ntp",
            Self::Probe => "probe",
            Self::Nfsd => "nfsd",
            Self::Numa => "numa",
            Self::Zfs => "zfs",
            Self::Sccache => "sccache",
            Self::Schedstat => "schedstat",
            Self::Systemd => "systemd",
            Self::Tailscale => "tailscale",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    InvalidAllowedIp(String),
    InvalidBind(String),
    InvalidPluginSocket(String),
    IncompleteTls,
    InvalidCgroupRoot(String),
    InvalidCgroupMaxUnits,
    InvalidSccacheTimeout,
    InvalidNtpTimeout,
    InvalidProbeTarget(String),
    InvalidProbeInterval,
    InvalidProbeTimeout,
    InvalidProbeTracerouteTimeout,
    InvalidProbeMaxHops,
    InvalidSccachePort,
    InvalidSystemdUnitInclude(String),
    InvalidSystemdUnitExclude(String),
    InvalidSystemdMaxUnits,
    InvalidTailscaleInterval,
    InvalidTailscaleTimeout,
    InvalidTailscalePingTimeout,
    InvalidPushInterval,
    InvalidPushTimeout,
    InvalidPushSpillInterval,
    InvalidPushSpoolDir(String),
    InvalidPushSpoolMaxBytes,
    InvalidPushUrl(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAllowedIp(value) => {
                write!(
                    formatter,
                    "invalid allowed_ip entry {value:?}: expected an IP or CIDR"
                )
            }
            Self::InvalidBind(value) => {
                write!(
                    formatter,
                    "invalid bind address {value:?}: expected IP:port"
                )
            }
            Self::InvalidPluginSocket(value) => write!(
                formatter,
                "invalid plugin_socket {value:?}: expected an absolute Unix socket path"
            ),
            Self::IncompleteTls => formatter.write_str(
                "tls_cert and tls_key must either both be configured or both be omitted",
            ),
            Self::InvalidCgroupRoot(value) => write!(
                formatter,
                "invalid cgroup_roots entry {value:?}: expected a relative path below /sys/fs/cgroup without '..'",
            ),
            Self::InvalidCgroupMaxUnits => {
                formatter.write_str("cgroup_max_units must be greater than zero")
            }
            Self::InvalidNtpTimeout => {
                formatter.write_str("ntp_timeout_ms must be between 10 and 60000 milliseconds")
            }
            Self::InvalidProbeTarget(value) => {
                write!(formatter, "invalid probe target: {value}")
            }
            Self::InvalidProbeInterval => {
                formatter.write_str("probe_interval_seconds must be between 1 and 3600")
            }
            Self::InvalidProbeTimeout => {
                formatter.write_str("probe_timeout_ms must be between 10 and 60000")
            }
            Self::InvalidProbeTracerouteTimeout => {
                formatter.write_str("probe_traceroute_timeout_ms must be between 100 and 120000")
            }
            Self::InvalidProbeMaxHops => {
                formatter.write_str("probe_traceroute_max_hops must be between 1 and 64")
            }
            Self::InvalidSccacheTimeout => {
                formatter.write_str("sccache_timeout_ms must be between 10 and 60000 milliseconds")
            }
            Self::InvalidSccachePort => {
                formatter.write_str("sccache_server_port must be greater than zero")
            }
            Self::InvalidSystemdUnitInclude(value) => {
                write!(formatter, "invalid systemd_unit_include regex {value:?}")
            }
            Self::InvalidSystemdUnitExclude(value) => {
                write!(formatter, "invalid systemd_unit_exclude regex {value:?}")
            }
            Self::InvalidSystemdMaxUnits => {
                formatter.write_str("systemd_max_units must be greater than zero")
            }
            Self::InvalidTailscaleInterval => {
                formatter.write_str("tailscale_interval_seconds must be between 1 and 3600")
            }
            Self::InvalidTailscaleTimeout => {
                formatter.write_str("tailscale_timeout_ms must be between 100 and 120000")
            }
            Self::InvalidTailscalePingTimeout => {
                formatter.write_str("tailscale_ping_timeout_ms must be between 10 and 60000")
            }
            Self::InvalidPushInterval => {
                formatter.write_str("push_interval_ms must be between 100 and 60000")
            }
            Self::InvalidPushTimeout => {
                formatter.write_str("push_timeout_ms must be between 100 and 60000")
            }
            Self::InvalidPushSpillInterval => {
                formatter.write_str("push_spill_after_seconds must be between 1 and 3600")
            }
            Self::InvalidPushSpoolDir(value) => write!(
                formatter,
                "invalid push_spool_dir {value:?}: expected an absolute path"
            ),
            Self::InvalidPushSpoolMaxBytes => {
                formatter.write_str("push_spool_max_bytes must be greater than zero")
            }
            Self::InvalidPushUrl(value) => write!(
                formatter,
                "invalid push_url {value:?}: expected an http:// or https:// URL"
            ),
        }
    }
}

/// Subsystem availability checks
struct SubsystemCheck {
    name: Datasource,
    path: &'static str,
    description: &'static str,
    /// If true, check that directory has entries (not just exists)
    require_entries: bool,
}

const SUBSYSTEM_CHECKS: &[SubsystemCheck] = &[
    SubsystemCheck {
        name: Datasource::Schedstat,
        path: "/proc/schedstat",
        description: "kernel scheduler statistics",
        require_entries: false,
    },
    SubsystemCheck {
        name: Datasource::Filefd,
        path: "/proc/sys/fs/file-nr",
        description: "file descriptor statistics",
        require_entries: false,
    },
    SubsystemCheck {
        name: Datasource::Systemd,
        path: "/run/systemd/system",
        description: "systemd",
        require_entries: false,
    },
    SubsystemCheck {
        name: Datasource::Pressure,
        path: "/proc/pressure",
        description: "pressure stall information (PSI)",
        require_entries: true,
    },
    SubsystemCheck {
        name: Datasource::Zfs,
        path: "/proc/spl/kstat/zfs/arcstats",
        description: "ZFS ARC statistics",
        require_entries: false,
    },
    SubsystemCheck {
        name: Datasource::Numa,
        path: "/sys/devices/system/node",
        description: "NUMA",
        require_entries: true,
    },
    SubsystemCheck {
        name: Datasource::Edac,
        path: "/sys/devices/system/edac/mc",
        description: "EDAC (memory error detection)",
        require_entries: true,
    },
    SubsystemCheck {
        name: Datasource::Rapl,
        path: "/sys/class/powercap",
        description: "RAPL (power monitoring)",
        require_entries: true,
    },
    SubsystemCheck {
        name: Datasource::Hwmon,
        path: "/sys/class/hwmon",
        description: "Hardware monitoring",
        require_entries: true,
    },
    SubsystemCheck {
        name: Datasource::Thermal,
        path: "/sys/class/thermal",
        description: "Thermal zones",
        require_entries: true,
    },
    SubsystemCheck {
        name: Datasource::PowerSupply,
        path: "/sys/class/power_supply",
        description: "Power supply",
        require_entries: true,
    },
    SubsystemCheck {
        name: Datasource::Nvme,
        path: "/sys/class/nvme",
        description: "NVMe devices",
        require_entries: true,
    },
    SubsystemCheck {
        name: Datasource::Ipmi,
        path: "/dev/ipmi0",
        description: "IPMI device",
        require_entries: false,
    },
    SubsystemCheck {
        name: Datasource::Mdraid,
        path: "/proc/mdstat",
        description: "MD RAID status",
        require_entries: false,
    },
    SubsystemCheck {
        name: Datasource::NetdevSysfs,
        path: "/sys/class/net",
        description: "Network interfaces",
        require_entries: true,
    },
];

/// Converts an IPv4-mapped IPv6 address back to plain IPv4.
///
/// A dual-stack listener reports IPv4 clients as `::ffff:a.b.c.d`, which never
/// matches an IPv4 CIDR in `allowed_ip` and would deny legitimate scrapes.
fn unmap_ipv4(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
        IpAddr::V4(v4) => IpAddr::V4(v4),
    }
}

/// Compares equal-length secret byte strings in constant time.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

fn check_path_available(path: &Path, require_entries: bool) -> bool {
    if !path.exists() {
        return false;
    }

    if require_entries {
        fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_some())
    } else {
        true
    }
}

fn check_subsystem_available(check: &SubsystemCheck) -> bool {
    check_path_available(Path::new(check.path), check.require_entries)
}

fn cgroup_roots_available_at(base: &Path, roots: &[String]) -> bool {
    base.join("cgroup.controllers").is_file() && roots.iter().all(|root| base.join(root).is_dir())
}

fn executable_file(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ProbeTarget {
    pub name: String,
    pub address: String,
    pub ping: bool,
    pub traceroute: bool,
}

impl Default for ProbeTarget {
    fn default() -> Self {
        Self {
            name: String::new(),
            address: String::new(),
            ping: true,
            traceroute: true,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent boolean switches map directly to stable flat TOML keys"
)]
pub struct AppConfig {
    pub ignore_loop_devices: bool,
    pub ignore_ramfs_filesystems: bool,
    pub ignore_ppp_interfaces: bool,
    pub ignore_veth_interfaces: bool,
    pub cgroup_roots: Vec<String>,
    pub cgroup_max_units: usize,
    pub cgroup_detailed_metrics: bool,
    pub sccache_binary: String,
    pub ntp_binary: String,
    pub ntp_timeout_ms: u64,
    pub probe_targets: Vec<ProbeTarget>,
    pub probe_interval_seconds: u64,
    pub probe_timeout_ms: u64,
    pub probe_traceroute_timeout_ms: u64,
    pub probe_traceroute_max_hops: u32,
    pub probe_ping_binary: String,
    pub probe_mtr_binary: String,
    pub sccache_server_port: u16,
    pub sccache_timeout_ms: u64,
    pub sccache_collect_dist_status: bool,
    pub systemd_unit_include: String,
    pub systemd_unit_exclude: String,
    pub systemd_max_units: usize,
    pub systemd_detailed_metrics: bool,
    pub tailscale_binary: String,
    pub tailscale_interval_seconds: u64,
    pub tailscale_timeout_ms: u64,
    pub tailscale_ping_timeout_ms: u64,
    pub tailscale_probe_tsmp: bool,
    pub tailscale_probe_icmp: bool,
    pub tailscale_external_traceroute: bool,
    #[serde(default)]
    pub disabled_datasources: Vec<Datasource>,
    pub allowed_ip: Vec<String>,
    pub bind: String,
    pub plugin_socket: Option<String>,
    pub log_denied_requests: bool,
    pub log_404_requests: bool,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
    pub auth_token: Option<String>,
    pub push_url: Option<String>,
    pub push_interval_ms: u64,
    pub push_timeout_ms: u64,
    pub push_spill_after_seconds: u64,
    pub push_spool_dir: String,
    pub push_spool_max_bytes: u64,
    #[serde(skip)]
    disabled_set: HashSet<Datasource>,
    #[serde(skip)]
    allowed_metrics_nets: Vec<IpNet>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ignore_loop_devices: true,
            ignore_ramfs_filesystems: true,
            ignore_ppp_interfaces: true,
            ignore_veth_interfaces: true,
            cgroup_roots: vec!["system.slice".to_string()],
            cgroup_max_units: 256,
            cgroup_detailed_metrics: false,
            sccache_binary: "sccache".to_string(),
            ntp_binary: "chronyc".to_string(),
            ntp_timeout_ms: 1_000,
            probe_targets: Vec::new(),
            probe_interval_seconds: 30,
            probe_timeout_ms: 1_000,
            probe_traceroute_timeout_ms: 10_000,
            probe_traceroute_max_hops: 30,
            probe_ping_binary: "ping".to_string(),
            probe_mtr_binary: "mtr".to_string(),
            sccache_server_port: 4226,
            sccache_timeout_ms: 1_000,
            sccache_collect_dist_status: false,
            systemd_unit_include: ".+".to_string(),
            systemd_unit_exclude: r".+\.(automount|device|mount|scope|slice)".to_string(),
            systemd_max_units: 512,
            systemd_detailed_metrics: false,
            tailscale_binary: "tailscale".to_string(),
            tailscale_interval_seconds: 30,
            tailscale_timeout_ms: 5_000,
            tailscale_ping_timeout_ms: 1_000,
            tailscale_probe_tsmp: true,
            tailscale_probe_icmp: true,
            tailscale_external_traceroute: true,
            disabled_datasources: Vec::new(),
            allowed_ip: vec!["127.0.0.0/8".to_string()],
            bind: "127.0.0.1:9100".to_string(),
            plugin_socket: None,
            log_denied_requests: true,
            log_404_requests: false,
            tls_cert: None,
            tls_key: None,
            auth_token: None,
            push_url: None,
            push_interval_ms: 1_000,
            push_timeout_ms: 1_000,
            push_spill_after_seconds: 10,
            push_spool_dir: "/var/lib/rs-linux-exporter/spool".to_string(),
            push_spool_max_bytes: 10 * 1024 * 1024,
            disabled_set: HashSet::new(),
            allowed_metrics_nets: Vec::new(),
        }
    }
}

impl AppConfig {
    pub fn bind_addr(&self) -> Result<SocketAddr, ConfigError> {
        self.bind
            .parse()
            .map_err(|_| ConfigError::InvalidBind(self.bind.clone()))
    }

    pub fn tls_config(&self) -> Result<Option<(&str, &str)>, ConfigError> {
        match (&self.tls_cert, &self.tls_key) {
            (Some(cert), Some(key)) => Ok(Some((cert, key))),
            (None, None) => Ok(None),
            _ => Err(ConfigError::IncompleteTls),
        }
    }

    pub fn is_token_valid(&self, token: Option<&str>) -> bool {
        match (&self.auth_token, token) {
            (Some(expected), Some(provided)) => {
                constant_time_eq(expected.as_bytes(), provided.as_bytes())
            }
            (Some(_), None) => false,
            (None, _) => true, // No token configured, allow all
        }
    }

    pub fn is_metrics_ip_allowed(&self, ip: IpAddr) -> bool {
        let ip = unmap_ipv4(ip);
        self.allowed_metrics_nets
            .iter()
            .any(|net| net.contains(&ip))
    }

    pub fn is_datasource_enabled(&self, datasource: Datasource) -> bool {
        !self.disabled_set.contains(&datasource)
    }

    pub fn disable_datasource(&mut self, datasource: Datasource) {
        self.disabled_set.insert(datasource);
    }

    fn build_disabled_set(&mut self) {
        self.disabled_set = self.disabled_datasources.iter().copied().collect();
    }

    fn validate_cgroup_settings(&self) -> Result<(), ConfigError> {
        if self.cgroup_max_units == 0 {
            return Err(ConfigError::InvalidCgroupMaxUnits);
        }
        if self.cgroup_roots.is_empty() {
            return Err(ConfigError::InvalidCgroupRoot("<empty list>".to_string()));
        }
        for root in &self.cgroup_roots {
            let path = Path::new(root);
            if root.is_empty()
                || path.is_absolute()
                || path.components().any(|component| {
                    matches!(
                        component,
                        Component::ParentDir | Component::RootDir | Component::Prefix(_)
                    )
                })
            {
                return Err(ConfigError::InvalidCgroupRoot(root.clone()));
            }
        }
        Ok(())
    }

    fn validate_ntp_settings(&self) -> Result<(), ConfigError> {
        if !(10..=60_000).contains(&self.ntp_timeout_ms) {
            return Err(ConfigError::InvalidNtpTimeout);
        }
        Ok(())
    }

    fn validate_probe_settings(&self) -> Result<(), ConfigError> {
        if !(1..=3_600).contains(&self.probe_interval_seconds) {
            return Err(ConfigError::InvalidProbeInterval);
        }
        if !(10..=60_000).contains(&self.probe_timeout_ms) {
            return Err(ConfigError::InvalidProbeTimeout);
        }
        if !(100..=120_000).contains(&self.probe_traceroute_timeout_ms) {
            return Err(ConfigError::InvalidProbeTracerouteTimeout);
        }
        if !(1..=64).contains(&self.probe_traceroute_max_hops) {
            return Err(ConfigError::InvalidProbeMaxHops);
        }
        let mut names = HashSet::new();
        for target in &self.probe_targets {
            if target.name.trim().is_empty() || target.address.trim().is_empty() {
                return Err(ConfigError::InvalidProbeTarget(
                    "name and address must be non-empty".to_string(),
                ));
            }
            if !target.ping && !target.traceroute {
                return Err(ConfigError::InvalidProbeTarget(format!(
                    "target {:?} enables neither ping nor traceroute",
                    target.name
                )));
            }
            if !names.insert(target.name.clone()) {
                return Err(ConfigError::InvalidProbeTarget(format!(
                    "duplicate target name {:?}",
                    target.name
                )));
            }
        }
        Ok(())
    }

    fn validate_sccache_settings(&self) -> Result<(), ConfigError> {
        if self.sccache_server_port == 0 {
            return Err(ConfigError::InvalidSccachePort);
        }
        if !(10..=60_000).contains(&self.sccache_timeout_ms) {
            return Err(ConfigError::InvalidSccacheTimeout);
        }
        Ok(())
    }

    fn validate_tailscale_settings(&self) -> Result<(), ConfigError> {
        if !(1..=3_600).contains(&self.tailscale_interval_seconds) {
            return Err(ConfigError::InvalidTailscaleInterval);
        }
        if !(100..=120_000).contains(&self.tailscale_timeout_ms) {
            return Err(ConfigError::InvalidTailscaleTimeout);
        }
        if !(10..=60_000).contains(&self.tailscale_ping_timeout_ms) {
            return Err(ConfigError::InvalidTailscalePingTimeout);
        }
        Ok(())
    }

    fn validate_systemd_settings(&self) -> Result<(), ConfigError> {
        if self.systemd_max_units == 0 {
            return Err(ConfigError::InvalidSystemdMaxUnits);
        }
        Regex::new(&self.systemd_unit_include).map_err(|_| {
            ConfigError::InvalidSystemdUnitInclude(self.systemd_unit_include.clone())
        })?;
        Regex::new(&self.systemd_unit_exclude).map_err(|_| {
            ConfigError::InvalidSystemdUnitExclude(self.systemd_unit_exclude.clone())
        })?;
        Ok(())
    }

    fn validate_push_settings(&self) -> Result<(), ConfigError> {
        let Some(url) = self.push_url.as_deref() else {
            return Ok(());
        };
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(ConfigError::InvalidPushUrl(url.to_string()));
        }
        if !(100..=60_000).contains(&self.push_interval_ms) {
            return Err(ConfigError::InvalidPushInterval);
        }
        if !(100..=60_000).contains(&self.push_timeout_ms) {
            return Err(ConfigError::InvalidPushTimeout);
        }
        if !(1..=3_600).contains(&self.push_spill_after_seconds) {
            return Err(ConfigError::InvalidPushSpillInterval);
        }
        let spool_dir = Path::new(&self.push_spool_dir);
        if !spool_dir.is_absolute() || spool_dir.as_os_str().is_empty() {
            return Err(ConfigError::InvalidPushSpoolDir(
                self.push_spool_dir.clone(),
            ));
        }
        if self.push_spool_max_bytes == 0 {
            return Err(ConfigError::InvalidPushSpoolMaxBytes);
        }
        Ok(())
    }

    fn build_allowed_metrics_nets(&mut self) -> Result<(), ConfigError> {
        let mut nets = Vec::new();
        for entry in &self.allowed_ip {
            // Try parsing as CIDR first, then as single IP
            if let Ok(net) = IpNet::from_str(entry) {
                nets.push(net);
            } else if let Ok(ip) = entry.parse::<IpAddr>() {
                // Single IP without prefix - convert to /32 (IPv4) or /128 (IPv6)
                nets.push(IpNet::from(ip));
            } else {
                return Err(ConfigError::InvalidAllowedIp(entry.clone()));
            }
        }
        self.allowed_metrics_nets = nets;
        Ok(())
    }

    fn validate(&mut self) -> Result<(), ConfigError> {
        self.build_disabled_set();
        self.build_allowed_metrics_nets()?;
        self.validate_cgroup_settings()?;
        self.validate_sccache_settings()?;
        self.validate_ntp_settings()?;
        self.validate_probe_settings()?;
        self.validate_systemd_settings()?;
        self.validate_tailscale_settings()?;
        self.validate_push_settings()?;
        if let Some(path) = self.plugin_socket.as_deref() {
            let path = Path::new(path);
            if !path.is_absolute() || path.as_os_str().is_empty() {
                return Err(ConfigError::InvalidPluginSocket(path.display().to_string()));
            }
        }
        let _ = self.bind_addr()?;
        let _ = self.tls_config()?;
        Ok(())
    }

    pub fn load() -> Self {
        // A config that exists but cannot be used must not be silently replaced
        // by the defaults: that would drop auth_token and the operator's
        // allowed_ip list while the exporter carried on serving metrics.
        let mut config = match fs::read_to_string(CONFIG_PATH) {
            Ok(contents) => match toml::from_str(&contents) {
                Ok(config) => config,
                Err(err) => {
                    eprintln!("Failed to parse {CONFIG_PATH}: {err}");
                    eprintln!("Refusing to start with default settings; fix the config file.");
                    std::process::exit(1);
                }
            },
            Err(err) if err.kind() == ErrorKind::NotFound => {
                eprintln!(
                    "No {CONFIG_PATH} in the working directory, using defaults \
                     (bind 127.0.0.1:9100, allowed_ip 127.0.0.0/8, no auth token)."
                );
                Self::default()
            }
            Err(err) => {
                eprintln!("Failed to read {CONFIG_PATH}: {err}");
                eprintln!("Refusing to start with default settings; fix the config file.");
                std::process::exit(1);
            }
        };

        if let Err(err) = config.validate() {
            eprintln!("Invalid {CONFIG_PATH}: {err}");
            eprintln!("Refusing to start with an invalid configuration.");
            std::process::exit(78);
        }
        config.check_subsystems();
        config
    }

    fn cgroup_roots_available(&self) -> bool {
        cgroup_roots_available_at(Path::new("/sys/fs/cgroup"), &self.cgroup_roots)
    }

    fn command_available(command: &str) -> bool {
        let binary = Path::new(command);
        if binary.components().count() > 1 {
            return executable_file(binary);
        }
        std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|dir| executable_file(&dir.join(binary)))
        })
    }

    fn check_subsystems(&mut self) {
        if self.is_datasource_enabled(Datasource::Tailscale)
            && !Self::command_available(&self.tailscale_binary)
        {
            eprintln!(
                "Tailscale binary {:?} not available, disabling tailscale datasource.",
                self.tailscale_binary
            );
            self.disable_datasource(Datasource::Tailscale);
        }
        if self.is_datasource_enabled(Datasource::Tailscale)
            && self.tailscale_external_traceroute
            && !Self::command_available(&self.probe_mtr_binary)
        {
            eprintln!(
                "mtr binary {:?} not available, disabling Tailscale external traceroutes.",
                self.probe_mtr_binary
            );
            self.tailscale_external_traceroute = false;
        }
        if self.is_datasource_enabled(Datasource::Probe) && self.probe_targets.is_empty() {
            self.disable_datasource(Datasource::Probe);
        }
        if self.is_datasource_enabled(Datasource::Probe)
            && self.probe_targets.iter().any(|target| target.ping)
            && !Self::command_available(&self.probe_ping_binary)
        {
            eprintln!(
                "ping binary {:?} not available, disabling probe datasource.",
                self.probe_ping_binary
            );
            self.disable_datasource(Datasource::Probe);
        }
        if self.is_datasource_enabled(Datasource::Probe)
            && self.probe_targets.iter().any(|target| target.traceroute)
            && !Self::command_available(&self.probe_mtr_binary)
        {
            eprintln!(
                "mtr binary {:?} not available, disabling probe datasource.",
                self.probe_mtr_binary
            );
            self.disable_datasource(Datasource::Probe);
        }
        if self.is_datasource_enabled(Datasource::Ntp) && !Self::command_available(&self.ntp_binary)
        {
            eprintln!(
                "NTP client binary {:?} not available, disabling ntp datasource.",
                self.ntp_binary
            );
            self.disable_datasource(Datasource::Ntp);
        }

        if self.is_datasource_enabled(Datasource::Sccache)
            && !Self::command_available(&self.sccache_binary)
        {
            eprintln!(
                "sccache binary {:?} not available, disabling sccache datasource.",
                self.sccache_binary
            );
            self.disable_datasource(Datasource::Sccache);
        }

        if self.is_datasource_enabled(Datasource::Cgroup) && !self.cgroup_roots_available() {
            eprintln!(
                "cgroup v2 service roots not available below /sys/fs/cgroup, disabling cgroup datasource."
            );
            self.disable_datasource(Datasource::Cgroup);
        }

        for check in SUBSYSTEM_CHECKS {
            if !self.is_datasource_enabled(check.name) {
                // Already disabled by config, skip check
                continue;
            }

            if !check_subsystem_available(check) {
                eprintln!(
                    "{} subsystem not available ({}), disabling {} datasource.",
                    check.description,
                    check.path,
                    check.name.as_str()
                );
                self.disable_datasource(check.name);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn cgroup_root_availability_requires_v2_and_all_roots() {
        let dir = TempDir::new().unwrap();
        let roots = vec!["system.slice".to_string(), "custom.slice".to_string()];
        assert!(!cgroup_roots_available_at(dir.path(), &roots));

        fs::write(dir.path().join("cgroup.controllers"), "cpu memory io").unwrap();
        fs::create_dir(dir.path().join("system.slice")).unwrap();
        assert!(!cgroup_roots_available_at(dir.path(), &roots));

        fs::create_dir(dir.path().join("custom.slice")).unwrap();
        assert!(cgroup_roots_available_at(dir.path(), &roots));
    }

    #[test]
    fn test_check_path_available_missing_path() {
        let path = Path::new("/nonexistent/path/that/does/not/exist");
        assert!(!check_path_available(path, true));
    }

    #[test]
    fn test_check_path_available_empty_dir() {
        let dir = TempDir::new().unwrap();
        assert!(!check_path_available(dir.path(), true));
    }

    #[test]
    fn test_check_path_available_with_entries() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("entry"), "data").unwrap();
        assert!(check_path_available(dir.path(), true));
    }

    #[test]
    fn test_check_path_available_no_entries_required() {
        let dir = TempDir::new().unwrap();
        // Should return true even if empty when require_entries is false
        assert!(check_path_available(dir.path(), false));
    }

    #[test]
    fn test_default_config_all_enabled() {
        let config = AppConfig::default();
        assert!(config.is_datasource_enabled(Datasource::Numa));
        assert!(config.is_datasource_enabled(Datasource::Edac));
        assert!(config.is_datasource_enabled(Datasource::Procfs));
        assert!(config.is_datasource_enabled(Datasource::Cgroup));
    }

    #[test]
    fn invalid_cgroup_roots_are_rejected() {
        for root in [
            "",
            "/system.slice",
            "../system.slice",
            "system.slice/../user.slice",
        ] {
            let config = AppConfig {
                cgroup_roots: vec![root.to_string()],
                ..Default::default()
            };
            assert!(matches!(
                config.validate_cgroup_settings(),
                Err(ConfigError::InvalidCgroupRoot(_))
            ));
        }
    }

    #[test]
    fn empty_cgroup_root_list_is_rejected() {
        let config = AppConfig {
            cgroup_roots: Vec::new(),
            ..Default::default()
        };
        assert!(matches!(
            config.validate_cgroup_settings(),
            Err(ConfigError::InvalidCgroupRoot(_))
        ));
    }

    #[test]
    fn zero_cgroup_unit_cap_is_rejected() {
        let config = AppConfig {
            cgroup_max_units: 0,
            ..Default::default()
        };
        assert_eq!(
            config.validate_cgroup_settings(),
            Err(ConfigError::InvalidCgroupMaxUnits)
        );
    }

    #[test]
    fn default_sccache_settings_are_bounded_and_side_effect_free() {
        let config = AppConfig::default();
        assert_eq!(config.sccache_binary, "sccache");
        assert_eq!(config.sccache_server_port, 4226);
        assert_eq!(config.sccache_timeout_ms, 1_000);
        assert!(!config.sccache_collect_dist_status);
        assert_eq!(config.validate_sccache_settings(), Ok(()));
    }

    #[test]
    fn invalid_systemd_regexes_are_rejected() {
        let include = AppConfig {
            systemd_unit_include: "[".to_string(),
            ..Default::default()
        };
        assert!(matches!(
            include.validate_systemd_settings(),
            Err(ConfigError::InvalidSystemdUnitInclude(_))
        ));

        let exclude = AppConfig {
            systemd_unit_exclude: "[".to_string(),
            ..Default::default()
        };
        assert!(matches!(
            exclude.validate_systemd_settings(),
            Err(ConfigError::InvalidSystemdUnitExclude(_))
        ));
    }

    #[test]
    fn zero_systemd_unit_cap_is_rejected() {
        let config = AppConfig {
            systemd_max_units: 0,
            ..Default::default()
        };
        assert_eq!(
            config.validate_systemd_settings(),
            Err(ConfigError::InvalidSystemdMaxUnits)
        );
    }

    #[test]
    fn default_systemd_filter_matches_node_exporter_baseline() {
        let config = AppConfig::default();
        assert_eq!(config.systemd_unit_include, ".+");
        assert_eq!(
            config.systemd_unit_exclude,
            r".+\.(automount|device|mount|scope|slice)"
        );
        assert_eq!(config.systemd_max_units, 512);
        assert!(!config.systemd_detailed_metrics);
        assert!(config.validate_systemd_settings().is_ok());
    }

    #[test]
    fn network_probe_settings_are_bounded_and_targets_are_unique() {
        let valid = AppConfig {
            probe_targets: vec![ProbeTarget {
                name: "dns".to_string(),
                address: "1.1.1.1".to_string(),
                ping: true,
                traceroute: false,
            }],
            ..Default::default()
        };
        assert_eq!(valid.validate_probe_settings(), Ok(()));

        let duplicate = AppConfig {
            probe_targets: vec![
                ProbeTarget {
                    name: "dns".to_string(),
                    address: "1.1.1.1".to_string(),
                    ping: true,
                    traceroute: false,
                },
                ProbeTarget {
                    name: "dns".to_string(),
                    address: "8.8.8.8".to_string(),
                    ping: true,
                    traceroute: false,
                },
            ],
            ..Default::default()
        };
        assert!(matches!(
            duplicate.validate_probe_settings(),
            Err(ConfigError::InvalidProbeTarget(_))
        ));

        let disabled = AppConfig {
            probe_targets: vec![ProbeTarget {
                name: "dns".to_string(),
                address: "1.1.1.1".to_string(),
                ping: false,
                traceroute: false,
            }],
            ..Default::default()
        };
        assert!(matches!(
            disabled.validate_probe_settings(),
            Err(ConfigError::InvalidProbeTarget(_))
        ));
    }

    #[test]
    fn ntp_and_tailscale_timeouts_are_bounded() {
        let ntp = AppConfig {
            ntp_timeout_ms: 9,
            ..Default::default()
        };
        assert_eq!(
            ntp.validate_ntp_settings(),
            Err(ConfigError::InvalidNtpTimeout)
        );

        let tailscale = AppConfig {
            tailscale_interval_seconds: 0,
            ..Default::default()
        };
        assert_eq!(
            tailscale.validate_tailscale_settings(),
            Err(ConfigError::InvalidTailscaleInterval)
        );
    }

    #[test]
    fn plugin_socket_must_be_absolute() {
        let mut invalid = AppConfig {
            plugin_socket: Some("plugins.sock".to_string()),
            ..Default::default()
        };
        assert!(matches!(
            invalid.validate(),
            Err(ConfigError::InvalidPluginSocket(_))
        ));

        invalid.plugin_socket = Some("/run/rs-linux-exporter/plugins.sock".to_string());
        assert!(invalid.validate().is_ok());
    }

    #[test]
    fn invalid_sccache_port_is_rejected() {
        let config = AppConfig {
            sccache_server_port: 0,
            ..Default::default()
        };
        assert_eq!(
            config.validate_sccache_settings(),
            Err(ConfigError::InvalidSccachePort)
        );
    }

    #[test]
    fn invalid_sccache_timeouts_are_rejected() {
        for timeout in [0, 9, 60_001, u64::MAX] {
            let config = AppConfig {
                sccache_timeout_ms: timeout,
                ..Default::default()
            };
            assert_eq!(
                config.validate_sccache_settings(),
                Err(ConfigError::InvalidSccacheTimeout)
            );
        }
    }

    #[test]
    fn sccache_binary_must_be_executable() {
        let dir = TempDir::new().unwrap();
        let binary = dir.path().join("sccache-test");
        fs::write(&binary, b"#!/bin/sh\nexit 0\n").unwrap();
        assert!(!executable_file(&binary));
        let mut permissions = fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&binary, permissions).unwrap();
        assert!(executable_file(&binary));
    }

    #[test]
    fn test_disable_datasource() {
        let mut config = AppConfig::default();
        assert!(config.is_datasource_enabled(Datasource::Thermal));
        config.disable_datasource(Datasource::Thermal);
        assert!(!config.is_datasource_enabled(Datasource::Thermal));
    }

    #[test]
    fn test_build_disabled_set_from_vec() {
        let mut config = AppConfig {
            disabled_datasources: vec![Datasource::Thermal, Datasource::Numa],
            ..Default::default()
        };
        config.build_disabled_set();
        assert!(!config.is_datasource_enabled(Datasource::Thermal));
        assert!(!config.is_datasource_enabled(Datasource::Numa));
        assert!(config.is_datasource_enabled(Datasource::Procfs));
    }

    #[test]
    fn test_allowed_ip_matches_ip() {
        let mut config = AppConfig {
            allowed_ip: vec!["10.0.0.0/8".to_string()],
            ..Default::default()
        };
        config.build_allowed_metrics_nets().expect("valid test ACL");

        let allowed_ip: IpAddr = "10.1.2.3".parse().unwrap();
        let denied_ip: IpAddr = "192.168.1.10".parse().unwrap();
        assert!(config.is_metrics_ip_allowed(allowed_ip));
        assert!(!config.is_metrics_ip_allowed(denied_ip));
    }

    #[test]
    fn test_allowed_ip_matches_ipv4_mapped_ipv6() {
        let mut config = AppConfig {
            allowed_ip: vec!["127.0.0.0/8".to_string()],
            ..Default::default()
        };
        config.build_allowed_metrics_nets().expect("valid test ACL");

        // What a dual-stack listener reports for a loopback IPv4 client.
        let mapped: IpAddr = "::ffff:127.0.0.1".parse().unwrap();
        assert!(config.is_metrics_ip_allowed(mapped));

        let mapped_denied: IpAddr = "::ffff:10.1.2.3".parse().unwrap();
        assert!(!config.is_metrics_ip_allowed(mapped_denied));
    }

    #[test]
    fn test_allowed_ip_still_matches_real_ipv6() {
        let mut config = AppConfig {
            allowed_ip: vec!["fd00::/8".to_string()],
            ..Default::default()
        };
        config.build_allowed_metrics_nets().expect("valid test ACL");

        assert!(config.is_metrics_ip_allowed("fd00::1".parse().unwrap()));
        assert!(!config.is_metrics_ip_allowed("2001:db8::1".parse().unwrap()));
    }

    #[test]
    fn legacy_cpufreq_datasource_name_remains_accepted() {
        let mut config = toml::from_str::<AppConfig>(r#"disabled_datasources = ["cpufreq"]"#)
            .expect("existing cpufreq spelling must remain valid");
        config.build_disabled_set();
        assert!(!config.is_datasource_enabled(Datasource::CpuFreq));
    }

    #[test]
    fn unknown_datasource_name_is_rejected_by_toml() {
        let parsed = toml::from_str::<AppConfig>(r#"disabled_datasources = ["theraml"]"#);
        assert!(parsed.is_err());
    }

    #[test]
    fn invalid_allowed_ip_is_rejected() {
        let mut config = AppConfig {
            allowed_ip: vec!["not-an-ip".to_string()],
            ..Default::default()
        };
        assert_eq!(
            config.build_allowed_metrics_nets(),
            Err(ConfigError::InvalidAllowedIp("not-an-ip".to_string()))
        );
    }

    #[test]
    fn invalid_bind_is_rejected() {
        let config = AppConfig {
            bind: "localhost:9100".to_string(),
            ..Default::default()
        };
        assert_eq!(
            config.bind_addr(),
            Err(ConfigError::InvalidBind("localhost:9100".to_string()))
        );
    }

    #[test]
    fn incomplete_tls_is_rejected() {
        let cert_only = AppConfig {
            tls_cert: Some("cert.pem".to_string()),
            ..Default::default()
        };
        assert_eq!(cert_only.tls_config(), Err(ConfigError::IncompleteTls));

        let key_only = AppConfig {
            tls_key: Some("key.pem".to_string()),
            ..Default::default()
        };
        assert_eq!(key_only.tls_config(), Err(ConfigError::IncompleteTls));
    }

    #[test]
    fn complete_tls_pair_is_accepted() {
        let config = AppConfig {
            tls_cert: Some("cert.pem".to_string()),
            tls_key: Some("key.pem".to_string()),
            ..Default::default()
        };
        assert_eq!(config.tls_config(), Ok(Some(("cert.pem", "key.pem"))));
    }

    #[test]
    fn test_token_validation_no_token_configured() {
        let config = AppConfig::default();
        // When no token is configured, all requests should be allowed
        assert!(config.is_token_valid(None));
        assert!(config.is_token_valid(Some("any-token")));
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"", b""));
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreT"));
        // Differs only in the first byte, and only in the last.
        assert!(!constant_time_eq(b"secret", b"Secret"));
        assert!(!constant_time_eq(b"secret", b"secrets"));
        assert!(!constant_time_eq(b"secret", b""));
    }

    #[test]
    fn test_token_validation_with_token_configured() {
        let config = AppConfig {
            auth_token: Some("secret-token".to_string()),
            ..Default::default()
        };
        // Correct token should be allowed
        assert!(config.is_token_valid(Some("secret-token")));
        // Wrong token should be denied
        assert!(!config.is_token_valid(Some("wrong-token")));
        // No token should be denied
        assert!(!config.is_token_valid(None));
    }
}
