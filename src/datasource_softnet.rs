use crate::metric_support::prometheus_u64;
use prometheus::GaugeVec;
use std::fs;
use std::sync::OnceLock;

struct SoftnetMetrics {
    softnet: GaugeVec,
}

impl SoftnetMetrics {
    fn new() -> Self {
        Self {
            softnet: prometheus::register_gauge_vec!(
                "softnet",
                "Per-CPU counters from /proc/net/softnet_stat",
                &["cpu", "field"]
            )
            .expect("register softnet"),
        }
    }
}

static SOFTNET_METRICS: OnceLock<SoftnetMetrics> = OnceLock::new();

fn metrics() -> &'static SoftnetMetrics {
    SOFTNET_METRICS.get_or_init(SoftnetMetrics::new)
}

fn parse_hex_u64(value: &str) -> Option<u64> {
    u64::from_str_radix(value, 16).ok()
}

fn parse_column(columns: &[&str], index: usize) -> Option<u64> {
    columns.get(index).and_then(|value| parse_hex_u64(value))
}

pub fn update_metrics() {
    let contents = match fs::read_to_string("/proc/net/softnet_stat") {
        Ok(contents) => contents,
        Err(_) => return,
    };

    // The number of rows follows CPU hotplug.
    metrics().softnet.reset();

    for (cpu, line) in contents.lines().enumerate() {
        let columns: Vec<&str> = line.split_whitespace().collect();
        if columns.is_empty() {
            continue;
        }

        let cpu_label = cpu.to_string();
        let metric = &metrics().softnet;
        let set_metric = |field: &str, value: u64| {
            metric
                .with_label_values(&[cpu_label.as_str(), field])
                .set(prometheus_u64(value));
        };

        if let Some(value) = parse_column(&columns, 0) {
            set_metric("softnet_processed_counter", value);
        }
        if let Some(value) = parse_column(&columns, 1) {
            set_metric("softnet_dropped_counter", value);
        }
        if let Some(value) = parse_column(&columns, 2) {
            set_metric("softnet_time_squeeze_counter", value);
        }
        if let Some(value) = parse_column(&columns, 9) {
            set_metric("softnet_received_rps_counter", value);
        }
        if let Some(value) = parse_column(&columns, 10) {
            set_metric("softnet_flow_limit_count_counter", value);
        }
        if let Some(value) = parse_column(&columns, 11) {
            set_metric("softnet_backlog_len_total", value);
        }
        set_metric("softnet_cpu_index", cpu as u64);
        if let Some(value) = parse_column(&columns, 13) {
            set_metric("softnet_input_qlen", value);
        }
        if let Some(value) = parse_column(&columns, 14) {
            set_metric("softnet_process_qlen", value);
        }
    }
}
