use crate::config::AppConfig;
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
    fn new() -> Self {
        Self {
            uptime_seconds: prometheus::register_gauge!(
                "uptime_seconds",
                "System uptime in seconds"
            )
            .expect("register uptime_seconds"),
            uptime_idle_seconds: prometheus::register_gauge!(
                "uptime_idle_seconds",
                "Sum of idle time across all CPUs in seconds"
            )
            .expect("register uptime_idle_seconds"),
            load_average: prometheus::register_gauge_vec!(
                "load_average",
                "System load averages",
                &["interval"]
            )
            .expect("register load_average"),
            load_processes: prometheus::register_gauge_vec!(
                "load_processes",
                "Runnable and total scheduling entities from /proc/loadavg",
                &["kind"]
            )
            .expect("register load_processes"),
            cpu_seconds_total: prometheus::register_gauge_vec!(
                "cpu_seconds_total",
                "CPU time spent in seconds",
                &["cpu", "mode"]
            )
            .expect("register cpu_seconds_total"),
            cpu_context_switches_total: prometheus::register_gauge!(
                "cpu_context_switches_total",
                "Number of context switches since boot"
            )
            .expect("register cpu_context_switches_total"),
            cpu_boot_time_seconds: prometheus::register_gauge!(
                "cpu_boot_time_seconds",
                "Boot time, in seconds since the epoch"
            )
            .expect("register cpu_boot_time_seconds"),
            processes_forked_total: prometheus::register_gauge!(
                "processes_forked_total",
                "Number of forks since boot"
            )
            .expect("register processes_forked_total"),
            processes_running: prometheus::register_gauge!(
                "processes_running",
                "Number of processes currently runnable"
            )
            .expect("register processes_running"),
            processes_blocked: prometheus::register_gauge!(
                "processes_blocked",
                "Number of processes blocked waiting for I/O"
            )
            .expect("register processes_blocked"),
            meminfo: prometheus::register_gauge_vec!(
                "meminfo",
                "Raw values from /proc/meminfo (bytes unless otherwise noted)",
                &["field"]
            )
            .expect("register meminfo"),
            vmstat: prometheus::register_gauge_vec!(
                "vmstat",
                "Raw values from /proc/vmstat",
                &["field"]
            )
            .expect("register vmstat"),
            diskstats: prometheus::register_gauge_vec!(
                "diskstats",
                "Raw disk statistics from /proc/diskstats",
                &["device", "field"]
            )
            .expect("register diskstats"),
            netdev: prometheus::register_gauge_vec!(
                "netdev",
                "Raw network device stats from /proc/net/dev",
                &["interface", "field"]
            )
            .expect("register netdev"),
            tcp_sockets: prometheus::register_gauge_vec!(
                "tcp_sockets",
                "TCP socket counts by state from /proc/net/tcp",
                &["state"]
            )
            .expect("register tcp_sockets"),
            udp_sockets: prometheus::register_gauge_vec!(
                "udp_sockets",
                "UDP socket counts by state from /proc/net/udp",
                &["state"]
            )
            .expect("register udp_sockets"),
            arp_entries: prometheus::register_gauge_vec!(
                "arp_entries",
                "ARP table entries by device from /proc/net/arp",
                &["device"]
            )
            .expect("register arp_entries"),
            snmp: prometheus::register_gauge_vec!(
                "snmp",
                "SNMP counters from /proc/net/snmp",
                &["field"]
            )
            .expect("register snmp"),
            netstat: prometheus::register_gauge_vec!(
                "netstat",
                "Extended netstat counters from /proc/net/netstat",
                &["field"]
            )
            .expect("register netstat"),
        }
    }
}

static PROCFS_METRICS: OnceLock<ProcfsMetrics> = OnceLock::new();

fn metrics() -> &'static ProcfsMetrics {
    PROCFS_METRICS.get_or_init(ProcfsMetrics::new)
}

fn set_cpu_time(metrics: &GaugeVec, cpu_label: &str, cpu_time: &CpuTime) {
    metrics
        .with_label_values(&[cpu_label, "user"])
        .set(cpu_time.user_ms() as f64 / 1000.0);
    metrics
        .with_label_values(&[cpu_label, "nice"])
        .set(cpu_time.nice_ms() as f64 / 1000.0);
    metrics
        .with_label_values(&[cpu_label, "system"])
        .set(cpu_time.system_ms() as f64 / 1000.0);
    metrics
        .with_label_values(&[cpu_label, "idle"])
        .set(cpu_time.idle_ms() as f64 / 1000.0);

    if let Some(value) = cpu_time.iowait_ms() {
        metrics
            .with_label_values(&[cpu_label, "iowait"])
            .set(value as f64 / 1000.0);
    }
    if let Some(value) = cpu_time.irq_ms() {
        metrics
            .with_label_values(&[cpu_label, "irq"])
            .set(value as f64 / 1000.0);
    }
    if let Some(value) = cpu_time.softirq_ms() {
        metrics
            .with_label_values(&[cpu_label, "softirq"])
            .set(value as f64 / 1000.0);
    }
    if let Some(value) = cpu_time.steal_ms() {
        metrics
            .with_label_values(&[cpu_label, "steal"])
            .set(value as f64 / 1000.0);
    }
    if let Some(value) = cpu_time.guest_ms() {
        metrics
            .with_label_values(&[cpu_label, "guest"])
            .set(value as f64 / 1000.0);
    }
    if let Some(value) = cpu_time.guest_nice_ms() {
        metrics
            .with_label_values(&[cpu_label, "guest_nice"])
            .set(value as f64 / 1000.0);
    }
}

fn set_meminfo_value(metrics: &GaugeVec, name: &str, value: u64) {
    metrics.with_label_values(&[name]).set(value as f64);
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
        let label = format!("cpu{}", idx);
        set_cpu_time(&metrics.cpu_seconds_total, &label, cpu);
    }

    metrics.cpu_context_switches_total.set(stats.ctxt as f64);
    metrics.cpu_boot_time_seconds.set(stats.btime as f64);
    metrics.processes_forked_total.set(stats.processes as f64);

    if let Some(value) = stats.procs_running {
        metrics.processes_running.set(value as f64);
    }
    if let Some(value) = stats.procs_blocked {
        metrics.processes_blocked.set(value as f64);
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
            .set(stat.reads as f64);
        diskstats
            .with_label_values(&[device, "reads_merged"])
            .set(stat.merged as f64);
        diskstats
            .with_label_values(&[device, "sectors_read"])
            .set(stat.sectors_read as f64);
        diskstats
            .with_label_values(&[device, "time_reading_ms"])
            .set(stat.time_reading as f64);
        diskstats
            .with_label_values(&[device, "writes"])
            .set(stat.writes as f64);
        diskstats
            .with_label_values(&[device, "writes_merged"])
            .set(stat.writes_merged as f64);
        diskstats
            .with_label_values(&[device, "sectors_written"])
            .set(stat.sectors_written as f64);
        diskstats
            .with_label_values(&[device, "time_writing_ms"])
            .set(stat.time_writing as f64);
        diskstats
            .with_label_values(&[device, "in_progress"])
            .set(stat.in_progress as f64);
        diskstats
            .with_label_values(&[device, "time_in_progress_ms"])
            .set(stat.time_in_progress as f64);
        diskstats
            .with_label_values(&[device, "weighted_time_in_progress_ms"])
            .set(stat.weighted_time_in_progress as f64);

        if let Some(value) = stat.discards {
            diskstats
                .with_label_values(&[device, "discards"])
                .set(value as f64);
        }
        if let Some(value) = stat.discards_merged {
            diskstats
                .with_label_values(&[device, "discards_merged"])
                .set(value as f64);
        }
        if let Some(value) = stat.sectors_discarded {
            diskstats
                .with_label_values(&[device, "sectors_discarded"])
                .set(value as f64);
        }
        if let Some(value) = stat.time_discarding {
            diskstats
                .with_label_values(&[device, "time_discarding_ms"])
                .set(value as f64);
        }
        if let Some(value) = stat.flushes {
            diskstats
                .with_label_values(&[device, "flushes"])
                .set(value as f64);
        }
        if let Some(value) = stat.time_flushing {
            diskstats
                .with_label_values(&[device, "time_flushing_ms"])
                .set(value as f64);
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
            .set(dev.recv_bytes as f64);
        netdev
            .with_label_values(&[iface, "recv_packets"])
            .set(dev.recv_packets as f64);
        netdev
            .with_label_values(&[iface, "recv_errs"])
            .set(dev.recv_errs as f64);
        netdev
            .with_label_values(&[iface, "recv_drop"])
            .set(dev.recv_drop as f64);
        netdev
            .with_label_values(&[iface, "recv_fifo"])
            .set(dev.recv_fifo as f64);
        netdev
            .with_label_values(&[iface, "recv_frame"])
            .set(dev.recv_frame as f64);
        netdev
            .with_label_values(&[iface, "recv_compressed"])
            .set(dev.recv_compressed as f64);
        netdev
            .with_label_values(&[iface, "recv_multicast"])
            .set(dev.recv_multicast as f64);
        netdev
            .with_label_values(&[iface, "sent_bytes"])
            .set(dev.sent_bytes as f64);
        netdev
            .with_label_values(&[iface, "sent_packets"])
            .set(dev.sent_packets as f64);
        netdev
            .with_label_values(&[iface, "sent_errs"])
            .set(dev.sent_errs as f64);
        netdev
            .with_label_values(&[iface, "sent_drop"])
            .set(dev.sent_drop as f64);
        netdev
            .with_label_values(&[iface, "sent_fifo"])
            .set(dev.sent_fifo as f64);
        netdev
            .with_label_values(&[iface, "sent_colls"])
            .set(dev.sent_colls as f64);
        netdev
            .with_label_values(&[iface, "sent_carrier"])
            .set(dev.sent_carrier as f64);
        netdev
            .with_label_values(&[iface, "sent_compressed"])
            .set(dev.sent_compressed as f64);
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

fn tcp_state_label(state: &TcpState) -> &'static str {
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

fn udp_state_label(state: &UdpState) -> &'static str {
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
            .set(count as f64);
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
            .set(count as f64);
    }
}

fn update_arp(metrics: &ProcfsMetrics, entries: &[procfs::net::ARPEntry]) {
    let mut counts: HashMap<&str, u64> = HashMap::new();
    for entry in entries {
        *counts.entry(entry.device.as_str()).or_insert(0) += 1;
    }

    // The set of devices is dynamic, so drop the previous scrape's series
    // rather than leaving entries for devices that no longer exist.
    metrics.arp_entries.reset();
    for (device, count) in counts {
        metrics
            .arp_entries
            .with_label_values(&[device])
            .set(count as f64);
    }
}

fn update_snmp(metrics: &ProcfsMetrics, snmp: &procfs::net::Snmp) {
    let set = |field: &str, value: u64| {
        metrics.snmp.with_label_values(&[field]).set(value as f64);
    };
    let set_i64 = |field: &str, value: i64| {
        metrics.snmp.with_label_values(&[field]).set(value as f64);
    };

    set("ip_forwarding", snmp.ip_forwarding.to_u8() as u64);
    set("ip_default_ttl", snmp.ip_default_ttl as u64);
    set("ip_in_receives", snmp.ip_in_receives);
    set("ip_in_hdr_errors", snmp.ip_in_hdr_errors);
    set("ip_in_addr_errors", snmp.ip_in_addr_errors);
    set("ip_forw_datagrams", snmp.ip_forw_datagrams);
    set("ip_in_unknown_protos", snmp.ip_in_unknown_protos);
    set("ip_in_discards", snmp.ip_in_discards);
    set("ip_in_delivers", snmp.ip_in_delivers);
    set("ip_out_requests", snmp.ip_out_requests);
    set("ip_out_discards", snmp.ip_out_discards);
    set("ip_out_no_routes", snmp.ip_out_no_routes);
    set("ip_reasm_timeout", snmp.ip_reasm_timeout);
    set("ip_reasm_reqds", snmp.ip_reasm_reqds);
    set("ip_reasm_oks", snmp.ip_reasm_oks);
    set("ip_reasm_fails", snmp.ip_reasm_fails);
    set("ip_frag_oks", snmp.ip_frag_oks);
    set("ip_frag_fails", snmp.ip_frag_fails);
    set("ip_frag_creates", snmp.ip_frag_creates);

    set("icmp_in_msgs", snmp.icmp_in_msgs);
    set("icmp_in_errors", snmp.icmp_in_errors);
    set("icmp_in_csum_errors", snmp.icmp_in_csum_errors);
    set("icmp_in_dest_unreachs", snmp.icmp_in_dest_unreachs);
    set("icmp_in_time_excds", snmp.icmp_in_time_excds);
    set("icmp_in_parm_probs", snmp.icmp_in_parm_probs);
    set("icmp_in_src_quenchs", snmp.icmp_in_src_quenchs);
    set("icmp_in_redirects", snmp.icmp_in_redirects);
    set("icmp_in_echos", snmp.icmp_in_echos);
    set("icmp_in_echo_reps", snmp.icmp_in_echo_reps);
    set("icmp_in_timestamps", snmp.icmp_in_timestamps);
    set("icmp_in_timestamp_reps", snmp.icmp_in_timestamp_reps);
    set("icmp_in_addr_masks", snmp.icmp_in_addr_masks);
    set("icmp_in_addr_mask_reps", snmp.icmp_in_addr_mask_reps);
    set("icmp_out_msgs", snmp.icmp_out_msgs);
    set("icmp_out_errors", snmp.icmp_out_errors);
    set("icmp_out_dest_unreachs", snmp.icmp_out_dest_unreachs);
    set("icmp_out_time_excds", snmp.icmp_out_time_excds);
    set("icmp_out_parm_probs", snmp.icmp_out_parm_probs);
    set("icmp_out_src_quenchs", snmp.icmp_out_src_quenchs);
    set("icmp_out_redirects", snmp.icmp_out_redirects);
    set("icmp_out_echos", snmp.icmp_out_echos);
    set("icmp_out_echo_reps", snmp.icmp_out_echo_reps);
    set("icmp_out_timestamps", snmp.icmp_out_timestamps);
    set("icmp_out_timestamp_reps", snmp.icmp_out_timestamp_reps);
    set("icmp_out_addr_masks", snmp.icmp_out_addr_masks);
    set("icmp_out_addr_mask_reps", snmp.icmp_out_addr_mask_reps);

    set("tcp_rto_algorithm", snmp.tcp_rto_algorithm.to_u8() as u64);
    set("tcp_rto_min", snmp.tcp_rto_min);
    set("tcp_rto_max", snmp.tcp_rto_max);
    set_i64("tcp_max_conn", snmp.tcp_max_conn);
    set("tcp_active_opens", snmp.tcp_active_opens);
    set("tcp_passive_opens", snmp.tcp_passive_opens);
    set("tcp_attempt_fails", snmp.tcp_attempt_fails);
    set("tcp_estab_resets", snmp.tcp_estab_resets);
    set("tcp_curr_estab", snmp.tcp_curr_estab);
    set("tcp_in_segs", snmp.tcp_in_segs);
    set("tcp_out_segs", snmp.tcp_out_segs);
    set("tcp_retrans_segs", snmp.tcp_retrans_segs);
    set("tcp_in_errs", snmp.tcp_in_errs);
    set("tcp_out_rsts", snmp.tcp_out_rsts);
    set("tcp_in_csum_errors", snmp.tcp_in_csum_errors);

    set("udp_in_datagrams", snmp.udp_in_datagrams);
    set("udp_no_ports", snmp.udp_no_ports);
    set("udp_in_errors", snmp.udp_in_errors);
    set("udp_out_datagrams", snmp.udp_out_datagrams);
    set("udp_rcvbuf_errors", snmp.udp_rcvbuf_errors);
    set("udp_sndbuf_errors", snmp.udp_sndbuf_errors);
    set("udp_in_csum_errors", snmp.udp_in_csum_errors);
    set("udp_ignored_multi", snmp.udp_ignored_multi);

    set("udp_lite_in_datagrams", snmp.udp_lite_in_datagrams);
    set("udp_lite_no_ports", snmp.udp_lite_no_ports);
    set("udp_lite_in_errors", snmp.udp_lite_in_errors);
    set("udp_lite_out_datagrams", snmp.udp_lite_out_datagrams);
    set("udp_lite_rcvbuf_errors", snmp.udp_lite_rcvbuf_errors);
    set("udp_lite_sndbuf_errors", snmp.udp_lite_sndbuf_errors);
    set("udp_lite_in_csum_errors", snmp.udp_lite_in_csum_errors);
    set("udp_lite_ignored_multi", snmp.udp_lite_ignored_multi);
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

            if !out.is_empty() {
                if is_upper
                    && (prev_is_lower
                        || prev_is_digit
                        || (prev_is_upper && next.map(|n| n.is_ascii_lowercase()).unwrap_or(false)))
                {
                    if !out.ends_with('_') {
                        out.push('_');
                    }
                } else if is_digit && (prev_is_lower || prev_is_upper) {
                    if !out.ends_with('_') {
                        out.push('_');
                    }
                } else if is_lower && prev_is_digit {
                    if !out.ends_with('_') {
                        out.push('_');
                    }
                }
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

fn update_netstat(metrics: &ProcfsMetrics) {
    let Ok(contents) = fs::read_to_string("/proc/net/netstat") else {
        return;
    };

    let mut headers: HashMap<String, Vec<String>> = HashMap::new();

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let mut parts = line.split_whitespace();
        let Some(section_raw) = parts.next() else {
            continue;
        };
        let section = section_raw.trim_end_matches(':').to_string();
        let rest: Vec<&str> = parts.collect();
        if rest.is_empty() {
            continue;
        }

        let is_values = rest.iter().all(|value| value.parse::<i64>().is_ok());
        if is_values {
            let Some(fields) = headers.get(&section) else {
                continue;
            };
            if fields.len() != rest.len() {
                continue;
            }

            let section_key = to_snake_case(&section);
            for (field, value_str) in fields.iter().zip(rest.iter()) {
                if let Ok(value) = value_str.parse::<i64>() {
                    let field_key = format!("{}_{}", section_key, to_snake_case(field));
                    metrics
                        .netstat
                        .with_label_values(&[field_key.as_str()])
                        .set(value as f64);
                }
            }
        } else {
            headers.insert(section, rest.iter().map(|s| s.to_string()).collect());
        }
    }
}

fn update_loadavg(metrics: &ProcfsMetrics, loadavg: &LoadAverage) {
    metrics
        .load_average
        .with_label_values(&["1"])
        .set(loadavg.one as f64);
    metrics
        .load_average
        .with_label_values(&["5"])
        .set(loadavg.five as f64);
    metrics
        .load_average
        .with_label_values(&["15"])
        .set(loadavg.fifteen as f64);

    metrics
        .load_processes
        .with_label_values(&["running"])
        .set(loadavg.cur as f64);
    metrics
        .load_processes
        .with_label_values(&["total"])
        .set(loadavg.max as f64);
    metrics
        .load_processes
        .with_label_values(&["latest_pid"])
        .set(loadavg.latest_pid as f64);
}

fn update_uptime(metrics: &ProcfsMetrics, uptime: &Uptime) {
    metrics.uptime_seconds.set(uptime.uptime);
    metrics.uptime_idle_seconds.set(uptime.idle);
}

pub fn update_metrics(config: &AppConfig) {
    let metrics = metrics();

    if let Ok(uptime) = Uptime::current() {
        update_uptime(metrics, &uptime);
    }

    if let Ok(loadavg) = LoadAverage::current() {
        update_loadavg(metrics, &loadavg);
    }

    if let Ok(meminfo) = Meminfo::current() {
        update_meminfo(metrics, &meminfo);
    }

    if let Ok(stats) = KernelStats::current() {
        update_kernel_stats(metrics, &stats);
    }

    if let Ok(vmstat) = procfs::vmstat() {
        for (key, value) in vmstat {
            metrics
                .vmstat
                .with_label_values(&[key.as_str()])
                .set(value as f64);
        }
    }

    if let Ok(stats) = procfs::diskstats() {
        update_diskstats(metrics, &stats, config);
    }

    if let Ok(devs) = procfs::net::dev_status() {
        update_netdev(metrics, &devs, config);
    }

    if let Ok(entries) = procfs::net::tcp() {
        update_tcp(metrics, &entries);
    }

    if let Ok(entries) = procfs::net::udp() {
        update_udp(metrics, &entries);
    }

    if let Ok(entries) = procfs::net::arp() {
        update_arp(metrics, &entries);
    }

    if let Ok(snmp) = procfs::net::snmp() {
        update_snmp(metrics, &snmp);
    }

    update_netstat(metrics);
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
