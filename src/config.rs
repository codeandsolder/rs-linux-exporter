use ipnet::IpNet;
use serde::Deserialize;
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::path::Path;
use std::str::FromStr;
use subtle::ConstantTimeEq;

/// Read relative to the working directory; the packaged unit sets
/// WorkingDirectory=/etc/rs-linux-exporter.
const CONFIG_PATH: &str = "config.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Datasource {
    Procfs,
    #[serde(rename = "cpufreq")]
    CpuFreq,
    Softnet,
    Conntrack,
    Filesystems,
    Hwmon,
    Ipmi,
    Mdraid,
    Thermal,
    Rapl,
    PowerSupply,
    Nvme,
    Edac,
    NetdevSysfs,
    Numa,
}

impl Datasource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Procfs => "procfs",
            Self::CpuFreq => "cpufreq",
            Self::Softnet => "softnet",
            Self::Conntrack => "conntrack",
            Self::Filesystems => "filesystems",
            Self::Hwmon => "hwmon",
            Self::Ipmi => "ipmi",
            Self::Mdraid => "mdraid",
            Self::Thermal => "thermal",
            Self::Rapl => "rapl",
            Self::PowerSupply => "power_supply",
            Self::Nvme => "nvme",
            Self::Edac => "edac",
            Self::NetdevSysfs => "netdev_sysfs",
            Self::Numa => "numa",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    InvalidAllowedIp(String),
    InvalidBind(String),
    IncompleteTls,
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
            Self::IncompleteTls => formatter.write_str(
                "tls_cert and tls_key must either both be configured or both be omitted",
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
    #[serde(default)]
    pub disabled_datasources: Vec<Datasource>,
    pub allowed_ip: Vec<String>,
    pub bind: String,
    pub log_denied_requests: bool,
    pub log_404_requests: bool,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
    pub auth_token: Option<String>,
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
            disabled_datasources: Vec::new(),
            allowed_ip: vec!["127.0.0.0/8".to_string()],
            bind: "127.0.0.1:9100".to_string(),
            log_denied_requests: true,
            log_404_requests: false,
            tls_cert: None,
            tls_key: None,
            auth_token: None,
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

    fn check_subsystems(&mut self) {
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
