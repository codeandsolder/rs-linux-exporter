use crate::collection::CollectionReport;
use crate::config::AppConfig;
use crate::metric_support::RegisterMetricResultExt;
use crate::metric_support::{prometheus_i64, prometheus_u64};
use crate::runtime::debug_enabled;
use procfs::net::{TcpState, UdpState};
use procfs::prelude::{Current, CurrentSI};
use procfs::{CpuTime, KernelStats, LoadAverage, Meminfo, Uptime};
use prometheus::{Gauge, GaugeVec};
use std::collections::HashMap;
use std::fs;
use std::sync::OnceLock;

struct ProcfsMetrics {
    uptime_seconds: Gauge,
    uptime_idle_seconds: Gauge,
    load_average: GaugeVec,
    load_processes: GaugeVec,
    cpu_seconds_total: GaugeVec,
    cpu_context_switches_total: Gauge,
    cpu_boot_time_seconds: Gauge,
    processes_forked_total: Gauge,
    processes_running: Gauge,
    processes_blocked: Gauge,
    meminfo: GaugeVec,
    vmstat: GaugeVec,
    diskstats: GaugeVec,
    netdev: GaugeVec,
    tcp_sockets: GaugeVec,
    udp_sockets: GaugeVec,
    arp_entries: GaugeVec,
    snmp: GaugeVec,
    netstat: GaugeVec,
}

impl ProcfsMetrics {
    #[expect(
        clippy::too_many_lines,
        reason = "flat Prometheus descriptor registration is easier to audit as one table"
    )]
    fn new() -> Self {
        Self {
            uptime_seconds: prometheus::register_gauge!(
                "uptime_seconds",
                "System uptime in seconds"
            )
            .or_exit("uptime_seconds"),
            uptime_idle_seconds: prometheus::register_gauge!(
                "uptime_idle_seconds",
                "Sum of idle time across all CPUs in seconds"
            )
            .or_exit("uptime_idle_seconds"),
            load_average: prometheus::register_gauge_vec!(
                "load_average",
                "System load averages",
                &["interval"]
            )
            .or_exit("load_average"),
            load_processes: prometheus::register_gauge_vec!(
                "load_processes",
                "Runnable and total scheduling entities from /proc/loadavg",
                &["kind"]
            )
            .or_exit("load_processes"),
            cpu_seconds_total: prometheus::register_gauge_vec!(
                "cpu_seconds_total",
                "CPU time spent in seconds",
                &["cpu", "mode"]
            )
            .or_exit("cpu_seconds_total"),
            cpu_context_switches_total: prometheus::register_gauge!(
                "cpu_context_switches_total",
                "Number of context switches since boot"
            )
            .or_exit("cpu_context_switches_total"),
            cpu_boot_time_seconds: prometheus::register_gauge!(
                "cpu_boot_time_seconds",
                "Boot time, in seconds since the epoch"
            )
            .or_exit("cpu_boot_time_seconds"),
            processes_forked_total: prometheus::register_gauge!(
                "processes_forked_total",
                "Number of forks since boot"
            )
            .or_exit("processes_forked_total"),
            processes_running: prometheus::register_gauge!(
                "processes_running",
                "Number of processes currently runnable"
            )
            .or_exit("processes_running"),
            processes_blocked: prometheus::register_gauge!(
                "processes_blocked",
                "Number of processes blocked waiting for I/O"
            )
            .or_exit("processes_blocked"),
            meminfo: prometheus::register_gauge_vec!(
                "meminfo",
                "Raw values from /proc/meminfo (bytes unless otherwise noted)",
                &["field"]
            )
            .or_exit("meminfo"),
            vmstat: prometheus::register_gauge_vec!(
                "vmstat",
                "Raw values from /proc/vmstat",
                &["field"]
            )
            .or_exit("vmstat"),
            diskstats: prometheus::register_gauge_vec!(
                "diskstats",
                "Raw disk statistics from /proc/diskstats",
                &["device", "field"]
            )
            .or_exit("diskstats"),
            netdev: prometheus::register_gauge_vec!(
                "netdev",
                "Raw network device stats from /proc/net/dev",
                &["interface", "field"]
            )
            .or_exit("netdev"),
            tcp_sockets: prometheus::register_gauge_vec!(
                "tcp_sockets",
                "TCP socket counts by state from /proc/net/tcp",
                &["state"]
            )
            .or_exit("tcp_sockets"),
            udp_sockets: prometheus::register_gauge_vec!(
                "udp_sockets",
                "UDP socket counts by state from /proc/net/udp",
                &["state"]
            )
            .or_exit("udp_sockets"),
            arp_entries: prometheus::register_gauge_vec!(
                "arp_entries",
                "ARP table entries by device from /proc/net/arp",
                &["device"]
            )
            .or_exit("arp_entries"),
            snmp: prometheus::register_gauge_vec!(
                "snmp",
                "SNMP counters from /proc/net/snmp",
                &["field"]
            )
            .or_exit("snmp"),
            netstat: prometheus::register_gauge_vec!(
                "netstat",
                "Extended netstat counters from /proc/net/netstat",
                &["field"]
            )
            .or_exit("netstat"),
        }
    }

    fn reset_dynamic(&self) {
        self.load_average.reset();
        self.load_processes.reset();
        self.cpu_seconds_total.reset();
        self.meminfo.reset();
        self.vmstat.reset();
        self.diskstats.reset();
        self.netdev.reset();
        self.tcp_sockets.reset();
        self.udp_sockets.reset();
        self.arp_entries.reset();
        self.snmp.reset();
        self.netstat.reset();
    }
}

static PROCFS_METRICS: OnceLock<ProcfsMetrics> = OnceLock::new();

fn metrics() -> &'static ProcfsMetrics {
    PROCFS_METRICS.get_or_init(ProcfsMetrics::new)
}

fn set_cpu_time(metrics: &GaugeVec, cpu_label: &str, cpu_time: &CpuTime) {
    metrics
        .with_label_values(&[cpu_label, "user"])
        .set(prometheus_u64(cpu_time.user_ms()) / 1000.0);
    metrics
        .with_label_values(&[cpu_label, "nice"])
        .set(prometheus_u64(cpu_time.nice_ms()) / 1000.0);
    metrics
        .with_label_values(&[cpu_label, "system"])
        .set(prometheus_u64(cpu_time.system_ms()) / 1000.0);
    metrics
        .with_label_values(&[cpu_label, "idle"])
        .set(prometheus_u64(cpu_time.idle_ms()) / 1000.0);

    if let Some(value) = cpu_time.iowait_ms() {
        metrics
            .with_label_values(&[cpu_label, "iowait"])
            .set(prometheus_u64(value) / 1000.0);
    }
    if let Some(value) = cpu_time.irq_ms() {
        metrics
            .with_label_values(&[cpu_label, "irq"])
            .set(prometheus_u64(value) / 1000.0);
    }
    if let Some(value) = cpu_time.softirq_ms() {
        metrics
            .with_label_values(&[cpu_label, "softirq"])
            .set(prometheus_u64(value) / 1000.0);
    }
    if let Some(value) = cpu_time.steal_ms() {
        metrics
            .with_label_values(&[cpu_label, "steal"])
            .set(prometheus_u64(value) / 1000.0);
    }
    if let Some(value) = cpu_time.guest_ms() {
        metrics
            .with_label_values(&[cpu_label, "guest"])
            .set(prometheus_u64(value) / 1000.0);
    }
    if let Some(value) = cpu_time.guest_nice_ms() {
        metrics
            .with_label_values(&[cpu_label, "guest_nice"])
            .set(prometheus_u64(value) / 1000.0);
    }
}

fn set_meminfo_value(metrics: &GaugeVec, name: &str, value: u64) {
    metrics
        .with_label_values(&[name])
        .set(prometheus_u64(value));
}

fn set_meminfo_optional(metrics: &GaugeVec, name: &str, value: Option<u64>) {
    if let Some(value) = value {
        set_meminfo_value(metrics, name, value);
    }
}

fn update_meminfo(metrics: &ProcfsMetrics, meminfo: &Meminfo) {
    set_meminfo_value(&metrics.meminfo, "mem_total", meminfo.mem_total);
    set_meminfo_value(&metrics.meminfo, "mem_free", meminfo.mem_free);
    set_meminfo_optional(&metrics.meminfo, "mem_available", meminfo.mem_available);
    set_meminfo_value(&metrics.meminfo, "buffers", meminfo.buffers);
    set_meminfo_value(&metrics.meminfo, "cached", meminfo.cached);
    set_meminfo_value(&metrics.meminfo, "swap_cached", meminfo.swap_cached);
    set_meminfo_value(&metrics.meminfo, "active", meminfo.active);
    set_meminfo_value(&metrics.meminfo, "inactive", meminfo.inactive);
    set_meminfo_optional(&metrics.meminfo, "active_anon", meminfo.active_anon);
    set_meminfo_optional(&metrics.meminfo, "inactive_anon", meminfo.inactive_anon);
    set_meminfo_optional(&metrics.meminfo, "active_file", meminfo.active_file);
    set_meminfo_optional(&metrics.meminfo, "inactive_file", meminfo.inactive_file);
    set_meminfo_optional(&metrics.meminfo, "unevictable", meminfo.unevictable);
    set_meminfo_optional(&metrics.meminfo, "mlocked", meminfo.mlocked);
    set_meminfo_optional(&metrics.meminfo, "high_total", meminfo.high_total);
    set_meminfo_optional(&metrics.meminfo, "high_free", meminfo.high_free);
    set_meminfo_optional(&metrics.meminfo, "low_total", meminfo.low_total);
    set_meminfo_optional(&metrics.meminfo, "low_free", meminfo.low_free);
    set_meminfo_optional(&metrics.meminfo, "mmap_copy", meminfo.mmap_copy);
    set_meminfo_value(&metrics.meminfo, "swap_total", meminfo.swap_total);
    set_meminfo_value(&metrics.meminfo, "swap_free", meminfo.swap_free);
    set_meminfo_value(&metrics.meminfo, "dirty", meminfo.dirty);
    set_meminfo_value(&metrics.meminfo, "writeback", meminfo.writeback);
    set_meminfo_optional(&metrics.meminfo, "anon_pages", meminfo.anon_pages);
    set_meminfo_value(&metrics.meminfo, "mapped", meminfo.mapped);
    set_meminfo_optional(&metrics.meminfo, "shmem", meminfo.shmem);
    set_meminfo_value(&metrics.meminfo, "slab", meminfo.slab);
    set_meminfo_optional(&metrics.meminfo, "s_reclaimable", meminfo.s_reclaimable);
    set_meminfo_optional(&metrics.meminfo, "s_unreclaim", meminfo.s_unreclaim);
    set_meminfo_optional(&metrics.meminfo, "kernel_stack", meminfo.kernel_stack);
    set_meminfo_optional(&metrics.meminfo, "page_tables", meminfo.page_tables);
    set_meminfo_optional(
        &metrics.meminfo,
        "secondary_page_tables",
        meminfo.secondary_page_tables,
    );
    set_meminfo_optional(&metrics.meminfo, "quicklists", meminfo.quicklists);
    set_meminfo_optional(&metrics.meminfo, "nfs_unstable", meminfo.nfs_unstable);
    set_meminfo_optional(&metrics.meminfo, "bounce", meminfo.bounce);
    set_meminfo_optional(&metrics.meminfo, "writeback_tmp", meminfo.writeback_tmp);
    set_meminfo_optional(&metrics.meminfo, "commit_limit", meminfo.commit_limit);
    set_meminfo_value(&metrics.meminfo, "committed_as", meminfo.committed_as);
    set_meminfo_value(&metrics.meminfo, "vmalloc_total", meminfo.vmalloc_total);
    set_meminfo_value(&metrics.meminfo, "vmalloc_used", meminfo.vmalloc_used);
    set_meminfo_value(&metrics.meminfo, "vmalloc_chunk", meminfo.vmalloc_chunk);
    set_meminfo_optional(
        &metrics.meminfo,
        "hardware_corrupted",
        meminfo.hardware_corrupted,
    );
    set_meminfo_optional(&metrics.meminfo, "anon_hugepages", meminfo.anon_hugepages);
    set_meminfo_optional(&metrics.meminfo, "shmem_hugepages", meminfo.shmem_hugepages);
    set_meminfo_optional(
        &metrics.meminfo,
        "shmem_pmd_mapped",
        meminfo.shmem_pmd_mapped,
    );
    set_meminfo_optional(&metrics.meminfo, "cma_total", meminfo.cma_total);
    set_meminfo_optional(&metrics.meminfo, "cma_free", meminfo.cma_free);
    set_meminfo_optional(&metrics.meminfo, "hugepages_total", meminfo.hugepages_total);
    set_meminfo_optional(&metrics.meminfo, "hugepages_free", meminfo.hugepages_free);
    set_meminfo_optional(&metrics.meminfo, "hugepages_rsvd", meminfo.hugepages_rsvd);
    set_meminfo_optional(&metrics.meminfo, "hugepages_surp", meminfo.hugepages_surp);
    set_meminfo_optional(&metrics.meminfo, "hugepagesize", meminfo.hugepagesize);
    set_meminfo_optional(&metrics.meminfo, "direct_map_4k", meminfo.direct_map_4k);
    set_meminfo_optional(&metrics.meminfo, "direct_map_4M", meminfo.direct_map_4M);
    set_meminfo_optional(&metrics.meminfo, "direct_map_2M", meminfo.direct_map_2M);
    set_meminfo_optional(&metrics.meminfo, "direct_map_1G", meminfo.direct_map_1G);
    set_meminfo_optional(&metrics.meminfo, "hugetlb", meminfo.hugetlb);
    set_meminfo_optional(&metrics.meminfo, "per_cpu", meminfo.per_cpu);
    set_meminfo_optional(&metrics.meminfo, "k_reclaimable", meminfo.k_reclaimable);
    set_meminfo_optional(&metrics.meminfo, "file_pmd_mapped", meminfo.file_pmd_mapped);
    set_meminfo_optional(&metrics.meminfo, "file_huge_pages", meminfo.file_huge_pages);
    set_meminfo_optional(&metrics.meminfo, "z_swap", meminfo.z_swap);
    set_meminfo_optional(&metrics.meminfo, "z_swapped", meminfo.z_swapped);
}

fn update_kernel_stats(metrics: &ProcfsMetrics, stats: &KernelStats) {
    set_cpu_time(&metrics.cpu_seconds_total, "total", &stats.total);
    for (idx, cpu) in stats.cpu_time.iter().enumerate() {
        let label = format!("cpu{idx}");
        set_cpu_time(&metrics.cpu_seconds_total, &label, cpu);
    }

    metrics
        .cpu_context_switches_total
        .set(prometheus_u64(stats.ctxt));
    metrics
        .cpu_boot_time_seconds
        .set(prometheus_u64(stats.btime));
    metrics
        .processes_forked_total
        .set(prometheus_u64(stats.processes));

    if let Some(value) = stats.procs_running {
        metrics.processes_running.set(f64::from(value));
    }
    if let Some(value) = stats.procs_blocked {
        metrics.processes_blocked.set(f64::from(value));
    }
}

fn update_diskstats(metrics: &ProcfsMetrics, stats: &[procfs::DiskStat], config: &AppConfig) {
    for stat in stats {
        let device = stat.name.as_str();
        if config.ignore_loop_devices && device.starts_with("loop") {
            continue;
        }
        let diskstats = &metrics.diskstats;
        diskstats
            .with_label_values(&[device, "reads"])
            .set(prometheus_u64(stat.reads));
        diskstats
            .with_label_values(&[device, "reads_merged"])
            .set(prometheus_u64(stat.merged));
        diskstats
            .with_label_values(&[device, "sectors_read"])
            .set(prometheus_u64(stat.sectors_read));
        diskstats
            .with_label_values(&[device, "time_reading_ms"])
            .set(prometheus_u64(stat.time_reading));
        diskstats
            .with_label_values(&[device, "writes"])
            .set(prometheus_u64(stat.writes));
        diskstats
            .with_label_values(&[device, "writes_merged"])
            .set(prometheus_u64(stat.writes_merged));
        diskstats
            .with_label_values(&[device, "sectors_written"])
            .set(prometheus_u64(stat.sectors_written));
        diskstats
            .with_label_values(&[device, "time_writing_ms"])
            .set(prometheus_u64(stat.time_writing));
        diskstats
            .with_label_values(&[device, "in_progress"])
            .set(prometheus_u64(stat.in_progress));
        diskstats
            .with_label_values(&[device, "time_in_progress_ms"])
            .set(prometheus_u64(stat.time_in_progress));
        diskstats
            .with_label_values(&[device, "weighted_time_in_progress_ms"])
            .set(prometheus_u64(stat.weighted_time_in_progress));

        if let Some(value) = stat.discards {
            diskstats
                .with_label_values(&[device, "discards"])
                .set(prometheus_u64(value));
        }
        if let Some(value) = stat.discards_merged {
            diskstats
                .with_label_values(&[device, "discards_merged"])
                .set(prometheus_u64(value));
        }
        if let Some(value) = stat.sectors_discarded {
            diskstats
                .with_label_values(&[device, "sectors_discarded"])
                .set(prometheus_u64(value));
        }
        if let Some(value) = stat.time_discarding {
            diskstats
                .with_label_values(&[device, "time_discarding_ms"])
                .set(prometheus_u64(value));
        }
        if let Some(value) = stat.flushes {
            diskstats
                .with_label_values(&[device, "flushes"])
                .set(prometheus_u64(value));
        }
        if let Some(value) = stat.time_flushing {
            diskstats
                .with_label_values(&[device, "time_flushing_ms"])
                .set(prometheus_u64(value));
        }
    }
}

fn update_netdev(
    metrics: &ProcfsMetrics,
    devs: &std::collections::HashMap<String, procfs::net::DeviceStatus>,
    config: &AppConfig,
) {
    for (name, dev) in devs {
        if config.ignore_ppp_interfaces && name.starts_with("ppp") {
            continue;
        }
        if config.ignore_veth_interfaces && (name.starts_with("veth") || name.starts_with("br-")) {
            continue;
        }
        let netdev = &metrics.netdev;
        let iface = name.as_str();
        netdev
            .with_label_values(&[iface, "recv_bytes"])
            .set(prometheus_u64(dev.recv_bytes));
        netdev
            .with_label_values(&[iface, "recv_packets"])
            .set(prometheus_u64(dev.recv_packets));
        netdev
            .with_label_values(&[iface, "recv_errs"])
            .set(prometheus_u64(dev.recv_errs));
        netdev
            .with_label_values(&[iface, "recv_drop"])
            .set(prometheus_u64(dev.recv_drop));
        netdev
            .with_label_values(&[iface, "recv_fifo"])
            .set(prometheus_u64(dev.recv_fifo));
        netdev
            .with_label_values(&[iface, "recv_frame"])
            .set(prometheus_u64(dev.recv_frame));
        netdev
            .with_label_values(&[iface, "recv_compressed"])
            .set(prometheus_u64(dev.recv_compressed));
        netdev
            .with_label_values(&[iface, "recv_multicast"])
            .set(prometheus_u64(dev.recv_multicast));
        netdev
            .with_label_values(&[iface, "sent_bytes"])
            .set(prometheus_u64(dev.sent_bytes));
        netdev
            .with_label_values(&[iface, "sent_packets"])
            .set(prometheus_u64(dev.sent_packets));
        netdev
            .with_label_values(&[iface, "sent_errs"])
            .set(prometheus_u64(dev.sent_errs));
        netdev
            .with_label_values(&[iface, "sent_drop"])
            .set(prometheus_u64(dev.sent_drop));
        netdev
            .with_label_values(&[iface, "sent_fifo"])
            .set(prometheus_u64(dev.sent_fifo));
        netdev
            .with_label_values(&[iface, "sent_colls"])
            .set(prometheus_u64(dev.sent_colls));
        netdev
            .with_label_values(&[iface, "sent_carrier"])
            .set(prometheus_u64(dev.sent_carrier));
        netdev
            .with_label_values(&[iface, "sent_compressed"])
            .set(prometheus_u64(dev.sent_compressed));
    }
}

/// Every label value `tcp_state_label` can return. Counts are seeded from this
/// list so a state that drops to zero reports zero instead of keeping its last
/// value.
const TCP_STATE_LABELS: [&str; 12] = [
    "established",
    "syn_sent",
    "syn_recv",
    "fin_wait_1",
    "fin_wait_2",
    "time_wait",
    "close",
    "close_wait",
    "last_ack",
    "listen",
    "closing",
    "new_syn_recv",
];

/// Every label value `udp_state_label` can return.
const UDP_STATE_LABELS: [&str; 2] = ["established", "close"];

const fn tcp_state_label(state: &TcpState) -> &'static str {
    match state {
        TcpState::Established => "established",
        TcpState::SynSent => "syn_sent",
        TcpState::SynRecv => "syn_recv",
        TcpState::FinWait1 => "fin_wait_1",
        TcpState::FinWait2 => "fin_wait_2",
        TcpState::TimeWait => "time_wait",
        TcpState::Close => "close",
        TcpState::CloseWait => "close_wait",
        TcpState::LastAck => "last_ack",
        TcpState::Listen => "listen",
        TcpState::Closing => "closing",
        TcpState::NewSynRecv => "new_syn_recv",
    }
}

const fn udp_state_label(state: &UdpState) -> &'static str {
    match state {
        UdpState::Established => "established",
        UdpState::Close => "close",
    }
}

fn update_tcp(metrics: &ProcfsMetrics, entries: &[procfs::net::TcpNetEntry]) {
    let mut counts: HashMap<&'static str, u64> =
        TCP_STATE_LABELS.iter().map(|state| (*state, 0)).collect();
    for entry in entries {
        *counts.entry(tcp_state_label(&entry.state)).or_insert(0) += 1;
    }

    for (state, count) in counts {
        metrics
            .tcp_sockets
            .with_label_values(&[state])
            .set(prometheus_u64(count));
    }
}

fn update_udp(metrics: &ProcfsMetrics, entries: &[procfs::net::UdpNetEntry]) {
    let mut counts: HashMap<&'static str, u64> =
        UDP_STATE_LABELS.iter().map(|state| (*state, 0)).collect();
    for entry in entries {
        *counts.entry(udp_state_label(&entry.state)).or_insert(0) += 1;
    }

    for (state, count) in counts {
        metrics
            .udp_sockets
            .with_label_values(&[state])
            .set(prometheus_u64(count));
    }
}

fn update_arp(metrics: &ProcfsMetrics, entries: &[procfs::net::ARPEntry]) {
    let mut counts: HashMap<&str, u64> = HashMap::new();
    for entry in entries {
        *counts.entry(entry.device.as_str()).or_insert(0) += 1;
    }

    for (device, count) in counts {
        metrics
            .arp_entries
            .with_label_values(&[device])
            .set(prometheus_u64(count));
    }
}

fn to_snake_case(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut prev: Option<char> = None;

    while let Some(ch) = chars.next() {
        if !ch.is_ascii_alphanumeric() {
            if !out.ends_with('_') {
                out.push('_');
            }
            prev = Some(ch);
            continue;
        }

        let next = chars.peek().copied();
        if let Some(prev_ch) = prev {
            let prev_is_lower = prev_ch.is_ascii_lowercase();
            let prev_is_upper = prev_ch.is_ascii_uppercase();
            let prev_is_digit = prev_ch.is_ascii_digit();
            let is_upper = ch.is_ascii_uppercase();
            let is_lower = ch.is_ascii_lowercase();
            let is_digit = ch.is_ascii_digit();

            let starts_new_word = (is_upper
                && (prev_is_lower
                    || prev_is_digit
                    || (prev_is_upper && next.is_some_and(|n| n.is_ascii_lowercase()))))
                || (is_digit && (prev_is_lower || prev_is_upper))
                || (is_lower && prev_is_digit);
            if !out.is_empty() && starts_new_word && !out.ends_with('_') {
                out.push('_');
            }
        }

        out.push(ch.to_ascii_lowercase());
        prev = Some(ch);
    }

    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "unknown".to_string()
    } else {
        out
    }
}

fn kernel_counter_field_name(section: &str, field: &str) -> String {
    let field_key = match (section, field) {
        ("Ip", "ReasmOKs") => "reasm_oks".to_string(),
        ("Ip", "FragOKs") => "frag_oks".to_string(),
        _ => to_snake_case(field),
    };
    format!("{}_{field_key}", to_snake_case(section))
}

fn parse_sectioned_integer_table(contents: &str) -> Result<Vec<(String, i64)>, String> {
    let mut headers: HashMap<String, Vec<String>> = HashMap::new();
    let mut parsed = Vec::new();

    for (line_index, line) in contents.lines().enumerate() {
        let line_number = line_index + 1;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let mut parts = line.split_whitespace();
        let section_raw = parts
            .next()
            .ok_or_else(|| format!("line {line_number}: missing section"))?;
        let Some(section) = section_raw.strip_suffix(':') else {
            return Err(format!(
                "line {line_number}: section {section_raw:?} is missing ':'"
            ));
        };
        if section.is_empty() {
            return Err(format!("line {line_number}: empty section name"));
        }

        let columns: Vec<&str> = parts.collect();
        if columns.is_empty() {
            return Err(format!(
                "line {line_number}: section {section:?} has no columns"
            ));
        }

        if let Some(fields) = headers.remove(section) {
            if fields.len() != columns.len() {
                return Err(format!(
                    "line {line_number}: section {section:?} has {} values for {} fields",
                    columns.len(),
                    fields.len()
                ));
            }

            for (field, value_text) in fields.iter().zip(columns) {
                let value = value_text.parse::<i64>().map_err(|error| {
                    format!(
                        "line {line_number}: section {section:?} field {field:?} has invalid integer {value_text:?}: {error}"
                    )
                })?;
                parsed.push((kernel_counter_field_name(section, field), value));
            }
        } else {
            if columns.iter().all(|value| value.parse::<i64>().is_ok()) {
                return Err(format!(
                    "line {line_number}: section {section:?} has values without a preceding header"
                ));
            }
            headers.insert(
                section.to_string(),
                columns
                    .into_iter()
                    .map(std::string::ToString::to_string)
                    .collect(),
            );
        }
    }

    if !headers.is_empty() {
        let mut sections: Vec<_> = headers.into_keys().collect();
        sections.sort_unstable();
        return Err(format!(
            "missing value rows for section header(s): {}",
            sections.join(", ")
        ));
    }

    Ok(parsed)
}

fn update_sectioned_integer_file(
    path: &'static str,
    source: &'static str,
    metric: &GaugeVec,
) -> CollectionReport {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) => {
            if debug_enabled() {
                eprintln!("procfs: failed to collect {source}: {error}");
            }
            return CollectionReport::error();
        }
    };

    let parsed = match parse_sectioned_integer_table(&contents) {
        Ok(parsed) => parsed,
        Err(error) => {
            if debug_enabled() {
                eprintln!("procfs: failed to collect {source}: {error}");
            }
            return CollectionReport::error();
        }
    };

    for (field, value) in parsed {
        metric
            .with_label_values(&[field.as_str()])
            .set(prometheus_i64(value));
    }
    CollectionReport::success()
}

fn update_snmp(metrics: &ProcfsMetrics) -> CollectionReport {
    update_sectioned_integer_file("/proc/net/snmp", "snmp", &metrics.snmp)
}

fn update_netstat(metrics: &ProcfsMetrics) -> CollectionReport {
    update_sectioned_integer_file("/proc/net/netstat", "netstat", &metrics.netstat)
}

fn update_loadavg(metrics: &ProcfsMetrics, loadavg: &LoadAverage) {
    metrics
        .load_average
        .with_label_values(&["1"])
        .set(f64::from(loadavg.one));
    metrics
        .load_average
        .with_label_values(&["5"])
        .set(f64::from(loadavg.five));
    metrics
        .load_average
        .with_label_values(&["15"])
        .set(f64::from(loadavg.fifteen));

    metrics
        .load_processes
        .with_label_values(&["running"])
        .set(f64::from(loadavg.cur));
    metrics
        .load_processes
        .with_label_values(&["total"])
        .set(f64::from(loadavg.max));
    metrics
        .load_processes
        .with_label_values(&["latest_pid"])
        .set(f64::from(loadavg.latest_pid));
}

fn update_uptime(metrics: &ProcfsMetrics, uptime: &Uptime) {
    metrics.uptime_seconds.set(uptime.uptime);
    metrics.uptime_idle_seconds.set(uptime.idle);
}

fn record_collection_error(
    report: &mut CollectionReport,
    source: &'static str,
    error: &impl std::fmt::Display,
) {
    report.record_error();
    if debug_enabled() {
        eprintln!("procfs: failed to collect {source}: {error}");
    }
}

pub fn update_metrics(config: &AppConfig) -> CollectionReport {
    let metrics = metrics();
    let mut report = CollectionReport::success();

    // Dynamic label sets must describe this scrape only. Clear them before
    // touching /proc so a read failure cannot preserve last scrape's series.
    metrics.reset_dynamic();

    match Uptime::current() {
        Ok(uptime) => update_uptime(metrics, &uptime),
        Err(error) => record_collection_error(&mut report, "uptime", &error),
    }
    match LoadAverage::current() {
        Ok(loadavg) => update_loadavg(metrics, &loadavg),
        Err(error) => record_collection_error(&mut report, "loadavg", &error),
    }
    match Meminfo::current() {
        Ok(meminfo) => update_meminfo(metrics, &meminfo),
        Err(error) => record_collection_error(&mut report, "meminfo", &error),
    }
    match KernelStats::current() {
        Ok(stats) => update_kernel_stats(metrics, &stats),
        Err(error) => record_collection_error(&mut report, "stat", &error),
    }
    match procfs::vmstat() {
        Ok(vmstat) => {
            for (key, value) in vmstat {
                metrics
                    .vmstat
                    .with_label_values(&[key.as_str()])
                    .set(prometheus_i64(value));
            }
        }
        Err(error) => record_collection_error(&mut report, "vmstat", &error),
    }
    match procfs::diskstats() {
        Ok(stats) => update_diskstats(metrics, &stats, config),
        Err(error) => record_collection_error(&mut report, "diskstats", &error),
    }
    match procfs::net::dev_status() {
        Ok(devs) => update_netdev(metrics, &devs, config),
        Err(error) => record_collection_error(&mut report, "netdev", &error),
    }
    match procfs::net::tcp() {
        Ok(entries) => update_tcp(metrics, &entries),
        Err(error) => record_collection_error(&mut report, "tcp", &error),
    }
    match procfs::net::udp() {
        Ok(entries) => update_udp(metrics, &entries),
        Err(error) => record_collection_error(&mut report, "udp", &error),
    }
    match procfs::net::arp() {
        Ok(entries) => update_arp(metrics, &entries),
        Err(error) => record_collection_error(&mut report, "arp", &error),
    }
    report.merge(update_snmp(metrics));
    report.merge(update_netstat(metrics));
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seeded label lists must stay in sync with the label functions,
    /// otherwise a state that drops to zero would keep its previous value.
    #[test]
    fn tcp_state_labels_cover_every_variant() {
        let all = [
            TcpState::Established,
            TcpState::SynSent,
            TcpState::SynRecv,
            TcpState::FinWait1,
            TcpState::FinWait2,
            TcpState::TimeWait,
            TcpState::Close,
            TcpState::CloseWait,
            TcpState::LastAck,
            TcpState::Listen,
            TcpState::Closing,
            TcpState::NewSynRecv,
        ];
        assert_eq!(all.len(), TCP_STATE_LABELS.len());
        for state in &all {
            let label = tcp_state_label(state);
            assert!(
                TCP_STATE_LABELS.contains(&label),
                "{label} missing from TCP_STATE_LABELS"
            );
        }
    }

    #[test]
    fn snake_case_matches_exported_kernel_field_names() {
        assert_eq!(to_snake_case("MemTotal"), "mem_total");
        assert_eq!(to_snake_case("SyncookiesSent"), "syncookies_sent");
        assert_eq!(to_snake_case("IcmpMsg"), "icmp_msg");
        assert_eq!(to_snake_case("InType0"), "in_type_0");
        assert_eq!(to_snake_case("MPTcpExt"), "mp_tcp_ext");
        assert_eq!(kernel_counter_field_name("Ip", "ReasmOKs"), "ip_reasm_oks");
        assert_eq!(kernel_counter_field_name("Ip", "FragOKs"), "ip_frag_oks");
    }

    #[test]
    fn sectioned_integer_table_allows_missing_optional_sections() {
        let input = "Tcp: MaxConn ActiveOpens\nTcp: -1 42\nUdp: InDatagrams NoPorts\nUdp: 7 3\n";
        let parsed = parse_sectioned_integer_table(input).expect("valid table");

        assert!(parsed.contains(&("tcp_max_conn".to_string(), -1)));
        assert!(parsed.contains(&("tcp_active_opens".to_string(), 42)));
        assert!(parsed.contains(&("udp_in_datagrams".to_string(), 7)));
        assert!(
            !parsed
                .iter()
                .any(|(field, _)| field.starts_with("udp_lite_"))
        );
    }

    #[test]
    fn sectioned_integer_table_includes_new_sections_automatically() {
        let input = "IcmpMsg: InType0 OutType3\nIcmpMsg: 11 22\n";
        let parsed = parse_sectioned_integer_table(input).expect("valid table");

        assert_eq!(
            parsed,
            vec![
                ("icmp_msg_in_type_0".to_string(), 11),
                ("icmp_msg_out_type_3".to_string(), 22),
            ]
        );
    }

    #[test]
    fn sectioned_integer_table_rejects_cardinality_mismatch() {
        let input = "Udp: InDatagrams NoPorts\nUdp: 7\n";
        let error = parse_sectioned_integer_table(input).expect_err("mismatch must fail");
        assert!(error.contains("1 values for 2 fields"), "{error}");
    }

    #[test]
    fn sectioned_integer_table_rejects_orphan_values() {
        let error =
            parse_sectioned_integer_table("Udp: 1 2\n").expect_err("orphan values must fail");
        assert!(error.contains("without a preceding header"), "{error}");
    }

    #[test]
    fn sectioned_integer_table_rejects_incomplete_header() {
        let error = parse_sectioned_integer_table("Udp: InDatagrams NoPorts\n")
            .expect_err("header without values must fail");
        assert!(error.contains("missing value rows"), "{error}");
    }

    #[test]
    fn udp_state_labels_cover_every_variant() {
        let all = [UdpState::Established, UdpState::Close];
        assert_eq!(all.len(), UDP_STATE_LABELS.len());
        for state in &all {
            let label = udp_state_label(state);
            assert!(
                UDP_STATE_LABELS.contains(&label),
                "{label} missing from UDP_STATE_LABELS"
            );
        }
    }
}
