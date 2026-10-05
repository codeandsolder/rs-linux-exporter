use crate::collection::CollectionReport;
use crate::metric_support::{RegisterMetricResultExt, prometheus_u64};
use crate::runtime::debug_enabled;
use prometheus::{CounterVec, GaugeVec};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::Hash;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

const NFS_PATH: &str = "/proc/net/rpc/nfs";
const NFSD_PATH: &str = "/proc/net/rpc/nfsd";

const V2_METHODS: &[&str] = &[
    "Null", "GetAttr", "SetAttr", "Root", "Lookup", "ReadLink", "Read", "WrCache", "Write",
    "Create", "Remove", "Rename", "Link", "SymLink", "MkDir", "RmDir", "ReadDir", "FsStat",
];
const V3_METHODS: &[&str] = &[
    "Null",
    "GetAttr",
    "SetAttr",
    "Lookup",
    "Access",
    "ReadLink",
    "Read",
    "Write",
    "Create",
    "MkDir",
    "SymLink",
    "MkNod",
    "Remove",
    "RmDir",
    "Rename",
    "Link",
    "ReadDir",
    "ReadDirPlus",
    "FsStat",
    "FsInfo",
    "PathConf",
    "Commit",
];
const CLIENT_V4_METHODS: &[&str] = &[
    "Null",
    "Read",
    "Write",
    "Commit",
    "Open",
    "OpenConfirm",
    "OpenNoattr",
    "OpenDowngrade",
    "Close",
    "Setattr",
    "FsInfo",
    "Renew",
    "SetClientID",
    "SetClientIDConfirm",
    "Lock",
    "Lockt",
    "Locku",
    "Access",
    "Getattr",
    "Lookup",
    "LookupRoot",
    "Remove",
    "Rename",
    "Link",
    "Symlink",
    "Create",
    "Pathconf",
    "StatFs",
    "ReadLink",
    "ReadDir",
    "ServerCaps",
    "DelegReturn",
    "GetACL",
    "SetACL",
    "FsLocations",
    "ReleaseLockowner",
    "Secinfo",
    "FsidPresent",
    "ExchangeID",
    "CreateSession",
    "DestroySession",
    "Sequence",
    "GetLeaseTime",
    "ReclaimComplete",
    "GetDeviceInfo",
    "LayoutGet",
    "LayoutCommit",
    "LayoutReturn",
    "SecinfoNoName",
    "TestStateID",
    "FreeStateID",
    "GetDeviceList",
    "BindConnToSession",
    "DestroyClientID",
    "Seek",
    "Allocate",
    "DeAllocate",
    "LayoutStats",
    "Clone",
];
const SERVER_V4_METHODS: &[&str] = &[
    "Op0Unused",
    "Op1Unused",
    "Op2Future",
    "Access",
    "Close",
    "Commit",
    "Create",
    "DelegPurge",
    "DelegReturn",
    "GetAttr",
    "GetFH",
    "Link",
    "Lock",
    "Lockt",
    "Locku",
    "Lookup",
    "LookupRoot",
    "Nverify",
    "Open",
    "OpenAttr",
    "OpenConfirm",
    "OpenDgrd",
    "PutFH",
    "PutPubFH",
    "PutRootFH",
    "Read",
    "ReadDir",
    "ReadLink",
    "Remove",
    "Rename",
    "Renew",
    "RestoreFH",
    "SaveFH",
    "SecInfo",
    "SetAttr",
    "SetClientID",
    "SetClientIDConfirm",
    "Verify",
    "Write",
    "RelLockOwner",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ClientFamily {
    Packets,
    Connections,
    Rpcs,
    Retransmissions,
    AuthRefreshes,
    Requests,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ServerFamily {
    ReplyCacheHits,
    ReplyCacheMisses,
    ReplyCacheNoCache,
    FileHandlesStale,
    DiskBytesRead,
    DiskBytesWritten,
    ReadAheadNotFound,
    Packets,
    Connections,
    RpcErrors,
    ServerRpcs,
    Requests,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CounterKey<F> {
    family: F,
    labels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CounterSample<F> {
    key: CounterKey<F>,
    value: u64,
}

#[derive(Debug, Default)]
struct ClientSnapshot {
    counters: Vec<CounterSample<ClientFamily>>,
}

#[derive(Debug, Default)]
struct ServerSnapshot {
    counters: Vec<CounterSample<ServerFamily>>,
    server_threads: Option<u64>,
    read_ahead_cache_size_blocks: Option<u64>,
}

fn push_counter<F: Copy>(
    counters: &mut Vec<CounterSample<F>>,
    family: F,
    labels: &[&str],
    value: u64,
) {
    counters.push(CounterSample {
        key: CounterKey {
            family,
            labels: labels.iter().map(|label| (*label).to_string()).collect(),
        },
        value,
    });
}

const fn monotonic_delta(previous: u64, current: u64) -> u64 {
    if current >= previous {
        current - previous
    } else {
        current
    }
}

fn parse_u64_values(line_number: usize, key: &str, fields: &[&str]) -> Result<Vec<u64>, String> {
    fields
        .iter()
        .map(|value| {
            value.parse::<u64>().map_err(|error| {
                format!("line {line_number}: {key} invalid integer {value:?}: {error}")
            })
        })
        .collect()
}

fn require_len(
    line_number: usize,
    key: &str,
    values: &[u64],
    expected: usize,
) -> Result<(), String> {
    if values.len() == expected {
        Ok(())
    } else {
        Err(format!(
            "line {line_number}: {key} has {} values, expected {expected}",
            values.len()
        ))
    }
}

fn method_name(names: &[&str], index: usize) -> String {
    names
        .get(index)
        .map_or_else(|| format!("op_{index}"), |name| (*name).to_string())
}

#[derive(Clone, Copy)]
struct ProcedureSpec<'a> {
    proto: &'a str,
    names: &'a [&'a str],
    minimum: usize,
    pad_known: bool,
}

fn push_counted_procedures<F: Copy>(
    counters: &mut Vec<CounterSample<F>>,
    family: F,
    values: &[u64],
    line_number: usize,
    key: &str,
    spec: ProcedureSpec<'_>,
) -> Result<(), String> {
    let ProcedureSpec {
        proto,
        names,
        minimum,
        pad_known,
    } = spec;
    let Some((&declared, samples)) = values.split_first() else {
        return Err(format!("line {line_number}: {key} has no procedure count"));
    };
    let declared = usize::try_from(declared)
        .map_err(|_| format!("line {line_number}: {key} procedure count is too large"))?;
    if samples.len() != declared || declared < minimum {
        return Err(format!(
            "line {line_number}: {key} declares {declared} procedures but has {} values; minimum is {minimum}",
            samples.len()
        ));
    }
    let emitted = if pad_known {
        names.len().max(samples.len())
    } else {
        samples.len()
    };
    for index in 0..emitted {
        let method = method_name(names, index);
        push_counter(
            counters,
            family,
            &[proto, method.as_str()],
            samples.get(index).copied().unwrap_or(0),
        );
    }
    Ok(())
}

fn parse_client_basic_line(
    snapshot: &mut ClientSnapshot,
    line_number: usize,
    key: &str,
    values: &[u64],
) -> Result<(), String> {
    match key {
        "net" => {
            require_len(line_number, key, values, 4)?;
            push_counter(
                &mut snapshot.counters,
                ClientFamily::Packets,
                &["udp"],
                values[1],
            );
            push_counter(
                &mut snapshot.counters,
                ClientFamily::Packets,
                &["tcp"],
                values[2],
            );
            push_counter(
                &mut snapshot.counters,
                ClientFamily::Connections,
                &[],
                values[3],
            );
        }
        "rpc" => {
            require_len(line_number, key, values, 3)?;
            push_counter(&mut snapshot.counters, ClientFamily::Rpcs, &[], values[0]);
            push_counter(
                &mut snapshot.counters,
                ClientFamily::Retransmissions,
                &[],
                values[1],
            );
            push_counter(
                &mut snapshot.counters,
                ClientFamily::AuthRefreshes,
                &[],
                values[2],
            );
        }
        _ => unreachable!("client basic keys are exhaustively matched"),
    }
    Ok(())
}

fn parse_client_procedure_line(
    snapshot: &mut ClientSnapshot,
    line_number: usize,
    key: &str,
    values: &[u64],
) -> Result<(), String> {
    let spec = match key {
        "proc2" => ProcedureSpec {
            proto: "2",
            names: V2_METHODS,
            minimum: V2_METHODS.len(),
            pad_known: false,
        },
        "proc3" => ProcedureSpec {
            proto: "3",
            names: V3_METHODS,
            minimum: V3_METHODS.len(),
            pad_known: false,
        },
        "proc4" => ProcedureSpec {
            proto: "4",
            names: CLIENT_V4_METHODS,
            minimum: 0,
            pad_known: true,
        },
        _ => unreachable!("client procedure keys are exhaustively matched"),
    };
    push_counted_procedures(
        &mut snapshot.counters,
        ClientFamily::Requests,
        values,
        line_number,
        key,
        spec,
    )
}

fn parse_client(contents: &str) -> Result<ClientSnapshot, String> {
    let mut snapshot = ClientSnapshot::default();
    for (line_index, line) in contents.lines().enumerate() {
        let line_number = line_index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let key = fields
            .next()
            .ok_or_else(|| format!("line {line_number}: missing metric key"))?;
        let raw = fields.collect::<Vec<_>>();
        match key {
            "net" | "rpc" => {
                let values = parse_u64_values(line_number, key, &raw)?;
                parse_client_basic_line(&mut snapshot, line_number, key, &values)?;
            }
            "proc2" | "proc3" | "proc4" => {
                let values = parse_u64_values(line_number, key, &raw)?;
                parse_client_procedure_line(&mut snapshot, line_number, key, &values)?;
            }
            _ => {
                if debug_enabled() {
                    eprintln!("nfs: ignoring unknown metric line {key:?}");
                }
            }
        }
    }
    if snapshot.counters.is_empty() {
        return Err("no recognized NFS client metrics found".to_string());
    }
    Ok(snapshot)
}

fn parse_server_thread(
    snapshot: &mut ServerSnapshot,
    line_number: usize,
    raw: &[&str],
) -> Result<(), String> {
    let Some(threads) = raw.first() else {
        return Err(format!("line {line_number}: th has no thread count"));
    };
    snapshot.server_threads = Some(threads.parse::<u64>().map_err(|error| {
        format!("line {line_number}: invalid nfsd thread count {threads:?}: {error}")
    })?);
    Ok(())
}

fn parse_server_cache_io_line(
    snapshot: &mut ServerSnapshot,
    line_number: usize,
    key: &str,
    values: &[u64],
) -> Result<(), String> {
    match key {
        "rc" => {
            require_len(line_number, key, values, 3)?;
            for (family, value) in [
                (ServerFamily::ReplyCacheHits, values[0]),
                (ServerFamily::ReplyCacheMisses, values[1]),
                (ServerFamily::ReplyCacheNoCache, values[2]),
            ] {
                push_counter(&mut snapshot.counters, family, &[], value);
            }
        }
        "fh" => {
            require_len(line_number, key, values, 5)?;
            push_counter(
                &mut snapshot.counters,
                ServerFamily::FileHandlesStale,
                &[],
                values[0],
            );
        }
        "io" => {
            require_len(line_number, key, values, 2)?;
            push_counter(
                &mut snapshot.counters,
                ServerFamily::DiskBytesRead,
                &[],
                values[0],
            );
            push_counter(
                &mut snapshot.counters,
                ServerFamily::DiskBytesWritten,
                &[],
                values[1],
            );
        }
        "ra" => {
            require_len(line_number, key, values, 12)?;
            snapshot.read_ahead_cache_size_blocks = Some(values[0]);
            push_counter(
                &mut snapshot.counters,
                ServerFamily::ReadAheadNotFound,
                &[],
                values[11],
            );
        }
        _ => unreachable!("server cache/I/O keys are exhaustively matched"),
    }
    Ok(())
}

fn parse_server_network_rpc_line(
    snapshot: &mut ServerSnapshot,
    line_number: usize,
    key: &str,
    values: &[u64],
) -> Result<(), String> {
    match key {
        "net" => {
            require_len(line_number, key, values, 4)?;
            push_counter(
                &mut snapshot.counters,
                ServerFamily::Packets,
                &["udp"],
                values[1],
            );
            push_counter(
                &mut snapshot.counters,
                ServerFamily::Packets,
                &["tcp"],
                values[2],
            );
            push_counter(
                &mut snapshot.counters,
                ServerFamily::Connections,
                &[],
                values[3],
            );
        }
        "rpc" => {
            require_len(line_number, key, values, 5)?;
            push_counter(
                &mut snapshot.counters,
                ServerFamily::ServerRpcs,
                &[],
                values[0],
            );
            for (error, value) in [("fmt", values[2]), ("auth", values[3]), ("cInt", values[4])] {
                push_counter(
                    &mut snapshot.counters,
                    ServerFamily::RpcErrors,
                    &[error],
                    value,
                );
            }
        }
        _ => unreachable!("server network/RPC keys are exhaustively matched"),
    }
    Ok(())
}

fn parse_server_procedure_line(
    snapshot: &mut ServerSnapshot,
    line_number: usize,
    key: &str,
    values: &[u64],
) -> Result<(), String> {
    if key == "wdeleg_getattr" {
        require_len(line_number, key, values, 1)?;
        push_counter(
            &mut snapshot.counters,
            ServerFamily::Requests,
            &["4", "WdelegGetattr"],
            values[0],
        );
        return Ok(());
    }
    let spec = match key {
        "proc2" => ProcedureSpec {
            proto: "2",
            names: V2_METHODS,
            minimum: V2_METHODS.len(),
            pad_known: false,
        },
        "proc3" => ProcedureSpec {
            proto: "3",
            names: V3_METHODS,
            minimum: V3_METHODS.len(),
            pad_known: false,
        },
        "proc4" => ProcedureSpec {
            proto: "4",
            names: &["Null", "Compound"],
            minimum: 2,
            pad_known: false,
        },
        "proc4ops" => ProcedureSpec {
            proto: "4",
            names: SERVER_V4_METHODS,
            minimum: 39,
            pad_known: false,
        },
        _ => unreachable!("server procedure keys are exhaustively matched"),
    };
    push_counted_procedures(
        &mut snapshot.counters,
        ServerFamily::Requests,
        values,
        line_number,
        key,
        spec,
    )
}

fn parse_server(contents: &str) -> Result<ServerSnapshot, String> {
    let mut snapshot = ServerSnapshot::default();
    for (line_index, line) in contents.lines().enumerate() {
        let line_number = line_index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let key = fields
            .next()
            .ok_or_else(|| format!("line {line_number}: missing metric key"))?;
        let raw = fields.collect::<Vec<_>>();
        if key == "th" {
            parse_server_thread(&mut snapshot, line_number, &raw)?;
            continue;
        }
        match key {
            "rc" | "fh" | "io" | "ra" => {
                let values = parse_u64_values(line_number, key, &raw)?;
                parse_server_cache_io_line(&mut snapshot, line_number, key, &values)?;
            }
            "net" | "rpc" => {
                let values = parse_u64_values(line_number, key, &raw)?;
                parse_server_network_rpc_line(&mut snapshot, line_number, key, &values)?;
            }
            "proc2" | "proc3" | "proc4" | "proc4ops" | "wdeleg_getattr" => {
                let values = parse_u64_values(line_number, key, &raw)?;
                parse_server_procedure_line(&mut snapshot, line_number, key, &values)?;
            }
            _ => {
                if debug_enabled() {
                    eprintln!("nfsd: ignoring unknown metric line {key:?}");
                }
            }
        }
    }
    if snapshot.counters.is_empty() && snapshot.server_threads.is_none() {
        return Err("no recognized NFS server metrics found".to_string());
    }
    Ok(snapshot)
}

struct ClientMetrics {
    packets_total: CounterVec,
    connections_total: CounterVec,
    rpcs_total: CounterVec,
    rpc_retransmissions_total: CounterVec,
    rpc_authentication_refreshes_total: CounterVec,
    requests_total: CounterVec,
    previous: Mutex<HashMap<CounterKey<ClientFamily>, u64>>,
}

impl ClientMetrics {
    fn new() -> Self {
        Self {
            packets_total: prometheus::register_counter_vec!(
                "nfs_packets_total",
                "Total NFS network packets by protocol type.",
                &["protocol"]
            )
            .or_exit("nfs_packets_total"),
            connections_total: prometheus::register_counter_vec!(
                "nfs_connections_total",
                "Total number of NFS TCP connections.",
                &[]
            )
            .or_exit("nfs_connections_total"),
            rpcs_total: prometheus::register_counter_vec!(
                "nfs_rpcs_total",
                "Total number of NFS RPCs performed.",
                &[]
            )
            .or_exit("nfs_rpcs_total"),
            rpc_retransmissions_total: prometheus::register_counter_vec!(
                "nfs_rpc_retransmissions_total",
                "Total number of NFS RPC retransmissions performed.",
                &[]
            )
            .or_exit("nfs_rpc_retransmissions_total"),
            rpc_authentication_refreshes_total: prometheus::register_counter_vec!(
                "nfs_rpc_authentication_refreshes_total",
                "Total number of NFS RPC authentication refreshes performed.",
                &[]
            )
            .or_exit("nfs_rpc_authentication_refreshes_total"),
            requests_total: prometheus::register_counter_vec!(
                "nfs_requests_total",
                "Number of NFS procedures invoked.",
                &["proto", "method"]
            )
            .or_exit("nfs_requests_total"),
            previous: Mutex::new(HashMap::new()),
        }
    }

    const fn counter_vec(&self, family: ClientFamily) -> &CounterVec {
        match family {
            ClientFamily::Packets => &self.packets_total,
            ClientFamily::Connections => &self.connections_total,
            ClientFamily::Rpcs => &self.rpcs_total,
            ClientFamily::Retransmissions => &self.rpc_retransmissions_total,
            ClientFamily::AuthRefreshes => &self.rpc_authentication_refreshes_total,
            ClientFamily::Requests => &self.requests_total,
        }
    }

    fn apply(&self, snapshot: ClientSnapshot) -> CollectionReport {
        apply_samples(
            &self.previous,
            snapshot.counters,
            |family| self.counter_vec(family),
            "nfs",
        )
    }

    fn clear(&self) -> CollectionReport {
        clear_samples(&self.previous, |family| self.counter_vec(family), "nfs")
    }
}

struct ServerMetrics {
    reply_cache_hits_total: CounterVec,
    reply_cache_misses_total: CounterVec,
    reply_cache_nocache_total: CounterVec,
    file_handles_stale_total: CounterVec,
    disk_bytes_read_total: CounterVec,
    disk_bytes_written_total: CounterVec,
    read_ahead_cache_not_found_total: CounterVec,
    packets_total: CounterVec,
    connections_total: CounterVec,
    rpc_errors_total: CounterVec,
    server_rpcs_total: CounterVec,
    requests_total: CounterVec,
    server_threads: GaugeVec,
    read_ahead_cache_size_blocks: GaugeVec,
    previous: Mutex<HashMap<CounterKey<ServerFamily>, u64>>,
}

impl ServerMetrics {
    fn new() -> Self {
        Self {
            reply_cache_hits_total: counter_vec(
                "nfsd_reply_cache_hits_total",
                "Total NFSd reply cache hits.",
                &[],
            ),
            reply_cache_misses_total: counter_vec(
                "nfsd_reply_cache_misses_total",
                "Total NFSd reply cache misses.",
                &[],
            ),
            reply_cache_nocache_total: counter_vec(
                "nfsd_reply_cache_nocache_total",
                "Total NFSd reply cache non-cacheable operations.",
                &[],
            ),
            file_handles_stale_total: counter_vec(
                "nfsd_file_handles_stale_total",
                "Total number of NFSd stale file handles.",
                &[],
            ),
            disk_bytes_read_total: counter_vec(
                "nfsd_disk_bytes_read_total",
                "Total NFSd bytes read.",
                &[],
            ),
            disk_bytes_written_total: counter_vec(
                "nfsd_disk_bytes_written_total",
                "Total NFSd bytes written.",
                &[],
            ),
            read_ahead_cache_not_found_total: counter_vec(
                "nfsd_read_ahead_cache_not_found_total",
                "Total NFSd read-ahead cache misses.",
                &[],
            ),
            packets_total: counter_vec(
                "nfsd_packets_total",
                "Total NFSd network packets by protocol type.",
                &["proto"],
            ),
            connections_total: counter_vec(
                "nfsd_connections_total",
                "Total number of NFSd TCP connections.",
                &[],
            ),
            rpc_errors_total: counter_vec(
                "nfsd_rpc_errors_total",
                "Total number of NFSd RPC errors by error type.",
                &["error"],
            ),
            server_rpcs_total: counter_vec(
                "nfsd_server_rpcs_total",
                "Total number of NFSd RPCs.",
                &[],
            ),
            requests_total: counter_vec(
                "nfsd_requests_total",
                "Total number of NFSd requests by protocol and method.",
                &["proto", "method"],
            ),
            server_threads: prometheus::register_gauge_vec!(
                "nfsd_server_threads",
                "Number of NFSd kernel threads currently running.",
                &[]
            )
            .or_exit("nfsd_server_threads"),
            read_ahead_cache_size_blocks: prometheus::register_gauge_vec!(
                "nfsd_read_ahead_cache_size_blocks",
                "NFSd read-ahead cache size in blocks.",
                &[]
            )
            .or_exit("nfsd_read_ahead_cache_size_blocks"),
            previous: Mutex::new(HashMap::new()),
        }
    }

    const fn counter_vec(&self, family: ServerFamily) -> &CounterVec {
        match family {
            ServerFamily::ReplyCacheHits => &self.reply_cache_hits_total,
            ServerFamily::ReplyCacheMisses => &self.reply_cache_misses_total,
            ServerFamily::ReplyCacheNoCache => &self.reply_cache_nocache_total,
            ServerFamily::FileHandlesStale => &self.file_handles_stale_total,
            ServerFamily::DiskBytesRead => &self.disk_bytes_read_total,
            ServerFamily::DiskBytesWritten => &self.disk_bytes_written_total,
            ServerFamily::ReadAheadNotFound => &self.read_ahead_cache_not_found_total,
            ServerFamily::Packets => &self.packets_total,
            ServerFamily::Connections => &self.connections_total,
            ServerFamily::RpcErrors => &self.rpc_errors_total,
            ServerFamily::ServerRpcs => &self.server_rpcs_total,
            ServerFamily::Requests => &self.requests_total,
        }
    }

    fn apply(&self, snapshot: ServerSnapshot) -> CollectionReport {
        self.server_threads.reset();
        if let Some(value) = snapshot.server_threads {
            self.server_threads
                .with_label_values(&[] as &[&str])
                .set(prometheus_u64(value));
        }
        self.read_ahead_cache_size_blocks.reset();
        if let Some(value) = snapshot.read_ahead_cache_size_blocks {
            self.read_ahead_cache_size_blocks
                .with_label_values(&[] as &[&str])
                .set(prometheus_u64(value));
        }
        apply_samples(
            &self.previous,
            snapshot.counters,
            |family| self.counter_vec(family),
            "nfsd",
        )
    }

    fn clear(&self) -> CollectionReport {
        self.server_threads.reset();
        self.read_ahead_cache_size_blocks.reset();
        clear_samples(&self.previous, |family| self.counter_vec(family), "nfsd")
    }
}

fn counter_vec(name: &'static str, help: &'static str, labels: &[&str]) -> CounterVec {
    prometheus::register_counter_vec!(name, help, labels).or_exit(name)
}

fn apply_samples<'a, F>(
    previous: &Mutex<HashMap<CounterKey<F>, u64>>,
    samples: Vec<CounterSample<F>>,
    counter_vec: impl Fn(F) -> &'a CounterVec,
    source: &str,
) -> CollectionReport
where
    F: Copy + Eq + Hash,
{
    let Ok(mut previous) = previous.lock() else {
        return CollectionReport::error();
    };
    let mut seen = HashSet::with_capacity(samples.len());
    for sample in samples {
        let old = previous
            .insert(sample.key.clone(), sample.value)
            .unwrap_or(0);
        let labels = sample
            .key
            .labels
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        counter_vec(sample.key.family)
            .with_label_values(&labels)
            .inc_by(prometheus_u64(monotonic_delta(old, sample.value)));
        seen.insert(sample.key);
    }
    remove_stale(&mut previous, &seen, counter_vec, source)
}

fn clear_samples<'a, F>(
    previous: &Mutex<HashMap<CounterKey<F>, u64>>,
    counter_vec: impl Fn(F) -> &'a CounterVec,
    source: &str,
) -> CollectionReport
where
    F: Copy + Eq + Hash,
{
    let Ok(mut previous) = previous.lock() else {
        return CollectionReport::error();
    };
    let seen = HashSet::new();
    remove_stale(&mut previous, &seen, counter_vec, source)
}

fn remove_stale<'a, F>(
    previous: &mut HashMap<CounterKey<F>, u64>,
    seen: &HashSet<CounterKey<F>>,
    counter_vec: impl Fn(F) -> &'a CounterVec,
    source: &str,
) -> CollectionReport
where
    F: Copy + Eq + Hash,
{
    let stale = previous
        .keys()
        .filter(|key| !seen.contains(*key))
        .cloned()
        .collect::<Vec<_>>();
    let mut report = CollectionReport::success();
    for key in stale {
        let labels = key.labels.iter().map(String::as_str).collect::<Vec<_>>();
        if let Err(error) = counter_vec(key.family).remove_label_values(&labels) {
            report.record_error();
            if debug_enabled() {
                eprintln!("{source}: failed to remove stale labels {labels:?}: {error}");
            }
        }
        previous.remove(&key);
    }
    report
}

static CLIENT_METRICS: OnceLock<ClientMetrics> = OnceLock::new();
static SERVER_METRICS: OnceLock<ServerMetrics> = OnceLock::new();

fn client_metrics() -> &'static ClientMetrics {
    CLIENT_METRICS.get_or_init(ClientMetrics::new)
}

fn server_metrics() -> &'static ServerMetrics {
    SERVER_METRICS.get_or_init(ServerMetrics::new)
}

fn update_client_from_path(path: &Path) -> CollectionReport {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return client_metrics().clear();
        }
        Err(error) => {
            if debug_enabled() {
                eprintln!("nfs: failed to read {}: {error}", path.display());
            }
            let mut report = client_metrics().clear();
            report.record_error();
            return report;
        }
    };
    match parse_client(&contents) {
        Ok(snapshot) => client_metrics().apply(snapshot),
        Err(error) => {
            if debug_enabled() {
                eprintln!("nfs: failed to parse {}: {error}", path.display());
            }
            let mut report = client_metrics().clear();
            report.record_error();
            report
        }
    }
}

fn update_server_from_path(path: &Path) -> CollectionReport {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return server_metrics().clear();
        }
        Err(error) => {
            if debug_enabled() {
                eprintln!("nfsd: failed to read {}: {error}", path.display());
            }
            let mut report = server_metrics().clear();
            report.record_error();
            return report;
        }
    };
    match parse_server(&contents) {
        Ok(snapshot) => server_metrics().apply(snapshot),
        Err(error) => {
            if debug_enabled() {
                eprintln!("nfsd: failed to parse {}: {error}", path.display());
            }
            let mut report = server_metrics().clear();
            report.record_error();
            report
        }
    }
}

pub fn update_client_metrics() -> CollectionReport {
    update_client_from_path(Path::new(NFS_PATH))
}

pub fn update_server_metrics() -> CollectionReport {
    update_server_from_path(Path::new(NFSD_PATH))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENT_FIXTURE: &str = "net 18628 0 18628 6\nrpc 4329785 0 4338291\nproc2 18 2 69 0 0 4410 0 0 0 0 0 0 0 0 0 0 0 99 2\nproc3 22 1 4084749 29200 94754 32580 186 47747 7981 8639 0 6356 0 6962 0 7958 0 0 241 4 4 2 39\nproc4 61 1 0 0 0 0 0 0 0 0 0 0 1 1 0 0 0 0 0 0 0 2 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n";
    const SERVER_FIXTURE: &str = "rc 0 6 18622\nfh 0 0 0 0 0\nio 157286400 0\nth 8 0 0.000 0.000 0.000 0.000 0.000 0.000 0.000 0.000 0.000 0.000\nra 32 0 0 0 0 0 0 0 0 0 0 0\nnet 18628 0 18628 6\nrpc 18628 0 0 0 0\nproc2 18 2 69 0 0 4410 0 0 0 0 0 0 0 0 0 0 0 99 2\nproc3 22 2 112 0 2719 111 0 0 0 0 0 0 0 0 0 0 0 27 216 0 2 1 0\nproc4 2 2 10853\nproc4ops 72 0 0 0 1098 2 0 0 0 0 8179 5896 0 0 0 0 5900 0 0 2 0 2 0 9609 0 2 150 1272 0 0 0 1236 0 0 0 0 3 3 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\nwdeleg_getattr 16\n";

    fn find_client(
        snapshot: &ClientSnapshot,
        family: ClientFamily,
        labels: &[&str],
    ) -> Option<u64> {
        snapshot.counters.iter().find_map(|sample| {
            (sample.key.family == family
                && sample
                    .key
                    .labels
                    .iter()
                    .map(String::as_str)
                    .eq(labels.iter().copied()))
            .then_some(sample.value)
        })
    }

    fn find_server(
        snapshot: &ServerSnapshot,
        family: ServerFamily,
        labels: &[&str],
    ) -> Option<u64> {
        snapshot.counters.iter().find_map(|sample| {
            (sample.key.family == family
                && sample
                    .key
                    .labels
                    .iter()
                    .map(String::as_str)
                    .eq(labels.iter().copied()))
            .then_some(sample.value)
        })
    }

    #[test]
    fn parses_upstream_client_fixture() {
        let snapshot = parse_client(CLIENT_FIXTURE).unwrap();
        assert_eq!(
            find_client(&snapshot, ClientFamily::Packets, &["tcp"]),
            Some(18_628)
        );
        assert_eq!(
            find_client(&snapshot, ClientFamily::Connections, &[]),
            Some(6)
        );
        assert_eq!(
            find_client(&snapshot, ClientFamily::Rpcs, &[]),
            Some(4_329_785)
        );
        assert_eq!(
            find_client(&snapshot, ClientFamily::Requests, &["3", "GetAttr"]),
            Some(4_084_749)
        );
        assert_eq!(
            find_client(&snapshot, ClientFamily::Requests, &["4", "SetClientID"]),
            Some(1)
        );
        assert_eq!(
            find_client(&snapshot, ClientFamily::Requests, &["4", "Clone"]),
            Some(0)
        );
    }

    #[test]
    fn client_v4_short_lists_are_padded_and_future_fields_retained() {
        let short = parse_client("proc4 2 7 8\n").unwrap();
        assert_eq!(
            find_client(&short, ClientFamily::Requests, &["4", "Null"]),
            Some(7)
        );
        assert_eq!(
            find_client(&short, ClientFamily::Requests, &["4", "Write"]),
            Some(0)
        );
        let future = parse_client(&format!(
            "proc4 {} {}\n",
            CLIENT_V4_METHODS.len() + 1,
            std::iter::repeat_n("1", CLIENT_V4_METHODS.len() + 1)
                .collect::<Vec<_>>()
                .join(" ")
        ))
        .unwrap();
        assert_eq!(
            find_client(
                &future,
                ClientFamily::Requests,
                &["4", format!("op_{}", CLIENT_V4_METHODS.len()).as_str()]
            ),
            Some(1)
        );
    }

    #[test]
    fn parses_upstream_server_fixture_and_retains_new_v4_ops() {
        let snapshot = parse_server(SERVER_FIXTURE).unwrap();
        assert_eq!(snapshot.server_threads, Some(8));
        assert_eq!(snapshot.read_ahead_cache_size_blocks, Some(32));
        assert_eq!(
            find_server(&snapshot, ServerFamily::DiskBytesRead, &[]),
            Some(157_286_400)
        );
        assert_eq!(
            find_server(&snapshot, ServerFamily::Packets, &["tcp"]),
            Some(18_628)
        );
        assert_eq!(
            find_server(&snapshot, ServerFamily::Requests, &["4", "GetAttr"]),
            Some(8_179)
        );
        assert_eq!(
            find_server(&snapshot, ServerFamily::Requests, &["4", "WdelegGetattr"]),
            Some(16)
        );
        assert_eq!(
            find_server(&snapshot, ServerFamily::Requests, &["4", "op_40"]),
            Some(0)
        );
        assert_eq!(
            find_server(&snapshot, ServerFamily::Requests, &["4", "op_71"]),
            Some(0)
        );
    }

    #[test]
    fn malformed_known_lines_are_rejected_but_unknown_lines_are_ignored() {
        assert!(parse_client("rpc 1 2\n").is_err());
        assert!(parse_server("rc 1 2\n").is_err());
        assert!(parse_client("future text 2.5 nope\nrpc 1 2 3\n").is_ok());
        assert!(parse_server("future text 2.5 nope\nth 4\n").is_ok());
    }

    #[test]
    fn clearing_unavailable_source_removes_stale_series() {
        let previous = Mutex::new(HashMap::new());
        let counter = CounterVec::new(
            prometheus::Opts::new("test_nfs_requests_total", "test"),
            &["proto", "method"],
        )
        .unwrap();
        let sample = CounterSample {
            key: CounterKey {
                family: ClientFamily::Requests,
                labels: vec!["4".to_string(), "Read".to_string()],
            },
            value: 10,
        };
        assert!(apply_samples(&previous, vec![sample], |_| &counter, "test-nfs").is_success());
        assert_eq!(
            prometheus::core::Collector::collect(&counter)[0]
                .get_metric()
                .len(),
            1
        );
        assert!(clear_samples(&previous, |_| &counter, "test-nfs").is_success());
        let remaining = prometheus::core::Collector::collect(&counter)
            .iter()
            .map(|family| family.get_metric().len())
            .sum::<usize>();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn monotonic_delta_handles_counter_reset() {
        assert_eq!(monotonic_delta(10, 15), 5);
        assert_eq!(monotonic_delta(10, 2), 2);
    }
}
