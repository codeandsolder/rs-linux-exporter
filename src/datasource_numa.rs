use crate::metric_support::prometheus_u64;
use prometheus::{Gauge, GaugeVec};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

struct NumaMetrics {
    node_count: Gauge,
    meminfo: GaugeVec,
    numastat: GaugeVec,
}

impl NumaMetrics {
    fn new() -> Self {
        Self {
            node_count: prometheus::register_gauge!("numa_node_count", "Number of NUMA nodes")
                .expect("register numa_node_count"),

            meminfo: prometheus::register_gauge_vec!(
                "numa_node_memory_bytes",
                "NUMA node memory information in bytes",
                &["node", "type"]
            )
            .expect("register numa_node_memory_bytes"),

            numastat: prometheus::register_gauge_vec!(
                "numa_node_stat_pages",
                "NUMA node hit/miss statistics in pages",
                &["node", "type"]
            )
            .expect("register numa_node_stat_pages"),
        }
    }
}

static NUMA_METRICS: OnceLock<NumaMetrics> = OnceLock::new();

fn metrics() -> &'static NumaMetrics {
    NUMA_METRICS.get_or_init(NumaMetrics::new)
}

fn read_string(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn parse_meminfo(content: &str, node_name: &str) {
    let metrics = metrics();

    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }

        // Format: "Node X FieldName: VALUE kB"
        let field_name = parts[2].trim_end_matches(':');
        let value: u64 = match parts[3].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Convert kB to bytes. A corrupt or absurd value would otherwise panic
        // a debug build and wrap silently in a release build.
        let bytes = value.saturating_mul(1024);

        metrics
            .meminfo
            .with_label_values(&[node_name, field_name])
            .set(prometheus_u64(bytes));
    }
}

fn parse_numastat(content: &str, node_name: &str) {
    let metrics = metrics();

    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }

        let stat_name = parts[0];
        let value: u64 = match parts[1].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };

        metrics
            .numastat
            .with_label_values(&[node_name, stat_name])
            .set(prometheus_u64(value));
    }
}

fn update_numa_node(node_path: &Path, node_name: &str) {
    // Read meminfo
    if let Some(meminfo) = read_string(&node_path.join("meminfo")) {
        parse_meminfo(&meminfo, node_name);
    }

    // Read numastat
    if let Some(numastat) = read_string(&node_path.join("numastat")) {
        parse_numastat(&numastat, node_name);
    }
}

pub fn update_metrics() {
    update_metrics_from_path(Path::new("/sys/devices/system/node"));
}

fn update_metrics_from_path(base: &Path) {
    let entries = match fs::read_dir(base) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    let metrics = metrics();
    let mut node_count = 0;

    for entry in entries.flatten() {
        let name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(_) => continue,
        };

        // Match node0, node1, etc.
        if name.starts_with("node") && name[4..].chars().all(|c| c.is_ascii_digit()) {
            let path = match fs::canonicalize(entry.path()) {
                Ok(p) => p,
                Err(_) => continue,
            };
            update_numa_node(&path, &name);
            node_count += 1;
        }
    }

    metrics.node_count.set(f64::from(node_count));
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const MOCK_MEMINFO: &str = r#"Node 0 MemTotal:       16384000 kB
Node 0 MemFree:         8192000 kB
Node 0 MemUsed:         8192000 kB
Node 0 Active:          4096000 kB
Node 0 Inactive:        2048000 kB
Node 0 Active(anon):    1024000 kB
Node 0 Inactive(anon):   512000 kB
Node 0 Active(file):    3072000 kB
Node 0 Inactive(file):  1536000 kB
Node 0 Unevictable:           0 kB
Node 0 Mlocked:               0 kB
Node 0 Dirty:              1024 kB
Node 0 Writeback:             0 kB
Node 0 FilePages:       4608000 kB
Node 0 Mapped:           256000 kB
Node 0 AnonPages:       1536000 kB
Node 0 Shmem:            128000 kB
Node 0 KernelStack:       16384 kB
Node 0 SReclaimable:     512000 kB
Node 0 SUnreclaim:       128000 kB
"#;

    const MOCK_NUMASTAT: &str = r#"numa_hit 123456789
numa_miss 1234
numa_foreign 5678
interleave_hit 9012
local_node 123456000
other_node 789
"#;

    fn create_mock_node(dir: &Path, name: &str) -> std::path::PathBuf {
        let node_dir = dir.join(name);
        fs::create_dir_all(&node_dir).unwrap();
        fs::write(node_dir.join("meminfo"), MOCK_MEMINFO).unwrap();
        fs::write(node_dir.join("numastat"), MOCK_NUMASTAT).unwrap();
        node_dir
    }

    #[test]
    fn test_read_string_trims_whitespace() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test");
        fs::write(&file, "  content  \n").unwrap();
        assert_eq!(read_string(&file), Some("content".to_string()));
    }

    #[test]
    fn test_parse_meminfo() {
        parse_meminfo(MOCK_MEMINFO, "node0");
        // If we get here without panic, parsing worked
    }

    #[test]
    fn test_parse_meminfo_handles_empty() {
        parse_meminfo("", "node0");
    }

    #[test]
    fn test_parse_meminfo_handles_malformed() {
        parse_meminfo("invalid line\nno data here", "node0");
    }

    #[test]
    fn test_parse_meminfo_handles_absurd_value() {
        // u64::MAX kB overflows the conversion to bytes.
        let line = format!("Node 0 MemTotal: {} kB", u64::MAX);
        parse_meminfo(&line, "node0");
    }

    #[test]
    fn test_parse_numastat() {
        parse_numastat(MOCK_NUMASTAT, "node0");
    }

    #[test]
    fn test_parse_numastat_handles_empty() {
        parse_numastat("", "node0");
    }

    #[test]
    fn test_parse_numastat_handles_malformed() {
        parse_numastat("invalid\nno_value", "node0");
    }

    #[test]
    fn test_update_numa_node() {
        let dir = TempDir::new().unwrap();
        let node = create_mock_node(dir.path(), "node0");
        update_numa_node(&node, "node0");
    }

    #[test]
    fn test_update_numa_node_missing_files() {
        let dir = TempDir::new().unwrap();
        let node_dir = dir.path().join("node0");
        fs::create_dir_all(&node_dir).unwrap();
        // No meminfo or numastat files
        update_numa_node(&node_dir, "node0");
    }

    #[test]
    fn test_update_metrics_from_path() {
        let dir = TempDir::new().unwrap();
        create_mock_node(dir.path(), "node0");
        create_mock_node(dir.path(), "node1");
        update_metrics_from_path(dir.path());
    }

    #[test]
    fn test_update_metrics_from_path_filters_non_nodes() {
        let dir = TempDir::new().unwrap();
        create_mock_node(dir.path(), "node0");
        fs::create_dir_all(dir.path().join("not_node")).unwrap();
        fs::create_dir_all(dir.path().join("possible")).unwrap();
        update_metrics_from_path(dir.path());
    }

    #[test]
    fn test_update_metrics_from_path_handles_empty_dir() {
        let dir = TempDir::new().unwrap();
        update_metrics_from_path(dir.path());
    }
}
