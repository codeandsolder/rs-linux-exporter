use ipnet::IpNet;
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::path::Path;
use std::str::FromStr;

/// Read relative to the working directory; the packaged unit sets
/// WorkingDirectory=/etc/rs-linux-exporter.
const CONFIG_PATH: &str = "config.toml";

/// Subsystem availability checks
struct SubsystemCheck {
    name: &'static str,
    path: &'static str,
    description: &'static str,
    /// If true, check that directory has entries (not just exists)
    require_entries: bool,
}

const SUBSYSTEM_CHECKS: &[SubsystemCheck] = &[
    SubsystemCheck {
        name: "numa",
        path: "/sys/devices/system/node",
        description: "NUMA",
        require_entries: true,
    },
    SubsystemCheck {
        name: "edac",
        path: "/sys/devices/system/edac/mc",
        description: "EDAC (memory error detection)",
        require_entries: true,
    },
    SubsystemCheck {
        name: "rapl",
        path: "/sys/class/powercap",
        description: "RAPL (power monitoring)",
        require_entries: true,
    },
    SubsystemCheck {
        name: "hwmon",
        path: "/sys/class/hwmon",
        description: "Hardware monitoring",
        require_entries: true,
    },
    SubsystemCheck {
        name: "thermal",
        path: "/sys/class/thermal",
        description: "Thermal zones",
        require_entries: true,
    },
    SubsystemCheck {
        name: "power_supply",
        path: "/sys/class/power_supply",
        description: "Power supply",
        require_entries: true,
    },
    SubsystemCheck {
        name: "nvme",
        path: "/sys/class/nvme",
        description: "NVMe devices",
        require_entries: true,
    },
    SubsystemCheck {
        name: "ipmi",
        path: "/dev/ipmi0",
        description: "IPMI device",
        require_entries: false,
    },
    SubsystemCheck {
        name: "mdraid",
        path: "/proc/mdstat",
        description: "MD RAID status",
        require_entries: false,
    },
    SubsystemCheck {
        name: "netdev_sysfs",
        path: "/sys/class/net",
        description: "Network interfaces",
        require_entries: true,
    },
];

/// Compares two byte strings without returning early on the first difference.
///
/// `==` on strings stops at the first mismatching byte, which makes the time it
/// takes depend on how many leading bytes a guess got right. The length is not
/// protected - it is not secret - but the contents are.
#[inline(never)]
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    std::hint::black_box(diff) == 0
}

fn check_path_available(path: &Path, require_entries: bool) -> bool {
    if !path.exists() {
        return false;
    }

    if require_entries {
        match fs::read_dir(path) {
            Ok(mut entries) => entries.next().is_some(),
            Err(_) => false,
        }
    } else {
        true
    }
}

fn check_subsystem_available(check: &SubsystemCheck) -> bool {
    check_path_available(Path::new(check.path), check.require_entries)
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub ignore_loop_devices: bool,
    pub ignore_ramfs_filesystems: bool,
    pub ignore_ppp_interfaces: bool,
    pub ignore_veth_interfaces: bool,
    #[serde(default)]
    pub disabled_datasources: Vec<String>,
    pub allowed_ip: Vec<String>,
    pub bind: String,
    pub log_denied_requests: bool,
    pub log_404_requests: bool,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
    pub auth_token: Option<String>,
    #[serde(skip)]
    disabled_set: HashSet<String>,
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
    pub fn bind_addr(&self) -> SocketAddr {
        self.bind.parse().unwrap_or_else(|err| {
            eprintln!("Invalid bind address '{}': {err}", self.bind);
            "127.0.0.1:9100".parse().expect("default bind")
        })
    }

    pub fn tls_config(&self) -> Option<(&str, &str)> {
        match (&self.tls_cert, &self.tls_key) {
            (Some(cert), Some(key)) => Some((cert, key)),
            _ => None,
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
        self.allowed_metrics_nets
            .iter()
            .any(|net| net.contains(&ip))
    }

    pub fn is_datasource_enabled(&self, name: &str) -> bool {
        !self.disabled_set.contains(name)
    }

    pub fn disable_datasource(&mut self, name: &str) {
        self.disabled_set.insert(name.to_string());
    }

    fn build_disabled_set(&mut self) {
        self.disabled_set = self.disabled_datasources.iter().cloned().collect();
    }

    fn build_allowed_metrics_nets(&mut self) {
        let mut nets = Vec::new();
        for entry in &self.allowed_ip {
            // Try parsing as CIDR first, then as single IP
            if let Ok(net) = IpNet::from_str(entry) {
                nets.push(net);
            } else if let Ok(ip) = entry.parse::<IpAddr>() {
                // Single IP without prefix - convert to /32 (IPv4) or /128 (IPv6)
                nets.push(IpNet::from(ip));
            } else {
                eprintln!("Invalid allowed_ip entry {entry}: not a valid IP or CIDR");
            }
        }
        self.allowed_metrics_nets = nets;
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

        config.build_disabled_set();
        config.build_allowed_metrics_nets();
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
                    check.description, check.path, check.name
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
        assert!(config.is_datasource_enabled("numa"));
        assert!(config.is_datasource_enabled("edac"));
        assert!(config.is_datasource_enabled("procfs"));
    }

    #[test]
    fn test_disable_datasource() {
        let mut config = AppConfig::default();
        assert!(config.is_datasource_enabled("test"));
        config.disable_datasource("test");
        assert!(!config.is_datasource_enabled("test"));
    }

    #[test]
    fn test_build_disabled_set_from_vec() {
        let mut config = AppConfig {
            disabled_datasources: vec!["thermal".to_string(), "numa".to_string()],
            ..Default::default()
        };
        config.build_disabled_set();
        assert!(!config.is_datasource_enabled("thermal"));
        assert!(!config.is_datasource_enabled("numa"));
        assert!(config.is_datasource_enabled("procfs"));
    }

    #[test]
    fn test_allowed_ip_matches_ip() {
        let mut config = AppConfig {
            allowed_ip: vec!["10.0.0.0/8".to_string()],
            ..Default::default()
        };
        config.build_allowed_metrics_nets();

        let allowed_ip: IpAddr = "10.1.2.3".parse().unwrap();
        let denied_ip: IpAddr = "192.168.1.10".parse().unwrap();
        assert!(config.is_metrics_ip_allowed(allowed_ip));
        assert!(!config.is_metrics_ip_allowed(denied_ip));
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
