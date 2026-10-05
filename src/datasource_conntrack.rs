//! Conntrack statistics collector via netlink protocol.
//!
//! This module queries per-CPU conntrack statistics using the netfilter netlink
//! protocol, similar to `conntrack -S`.

use crate::metric_support::RegisterMetricResultExt;
use crate::metric_support::prometheus_u64;
use prometheus::GaugeVec;
use std::collections::HashMap;
use std::io::{self, Error};
use std::mem;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;
use std::sync::OnceLock;

// Netlink protocol constants
const NETLINK_NETFILTER: i32 = 12;

/// Upper bound on how long a single scrape will wait for the kernel.
const RECV_TIMEOUT_SECS: i64 = 2;

// Netlink message flags
const NLM_F_REQUEST: u16 = 0x0001;
const NLM_F_DUMP: u16 = 0x0300;

// Netlink message types
const NLMSG_DONE: u16 = 3;
const NLMSG_ERROR: u16 = 2;

// Netfilter netlink constants
const NFNL_SUBSYS_CTNETLINK: u8 = 1;
const NFNETLINK_V0: u8 = 0;
const IPCTNL_MSG_CT_GET_STATS_CPU: u8 = 4;

// CTA_STATS attribute IDs (from linux/netfilter/nfnetlink_conntrack.h)
const CTA_STATS_FOUND: u16 = 2;
const CTA_STATS_INVALID: u16 = 4;
const CTA_STATS_INSERT: u16 = 8;
const CTA_STATS_INSERT_FAILED: u16 = 9;
const CTA_STATS_DROP: u16 = 10;
const CTA_STATS_EARLY_DROP: u16 = 11;
const CTA_STATS_ERROR: u16 = 12;
const CTA_STATS_SEARCH_RESTART: u16 = 13;
const CTA_STATS_CLASH_RESOLVE: u16 = 14;
const CTA_STATS_CHAIN_TOOLONG: u16 = 15;

/// Netlink message header (16 bytes)
#[repr(C)]
#[expect(
    clippy::struct_field_names,
    reason = "field names intentionally mirror the Linux nlmsghdr UAPI"
)]
struct NlMsgHdr {
    nlmsg_len: u32,
    nlmsg_type: u16,
    nlmsg_flags: u16,
    nlmsg_seq: u32,
    nlmsg_pid: u32,
}

/// Netfilter generic message header (4 bytes)
#[repr(C)]
struct NfGenMsg {
    nfgen_family: u8,
    version: u8,
    res_id: u16, // CPU ID in response (big-endian)
}

/// Netlink attribute header
#[repr(C)]
struct NlAttr {
    nla_len: u16,
    nla_type: u16,
}

/// Per-CPU conntrack statistics
#[derive(Debug, Default)]
pub struct CpuStats {
    pub cpu_id: u16,
    pub counters: HashMap<String, u64>,
}

struct ConntrackMetrics {
    conntrack: GaugeVec,
}

impl ConntrackMetrics {
    fn new() -> Self {
        Self {
            conntrack: prometheus::register_gauge_vec!(
                "conntrack",
                "Per-CPU conntrack counters via netlink",
                &["cpu", "field"]
            )
            .or_exit("conntrack"),
        }
    }
}

static CONNTRACK_METRICS: OnceLock<ConntrackMetrics> = OnceLock::new();

fn metrics() -> &'static ConntrackMetrics {
    CONNTRACK_METRICS.get_or_init(ConntrackMetrics::new)
}

/// Align to 4-byte boundary (`NLMSG_ALIGN`)
#[inline]
const fn nlmsg_align(len: usize) -> usize {
    (len + 3) & !3
}

/// Build the netlink request message for conntrack stats
fn create_stats_request(seq: u32) -> Result<Vec<u8>, String> {
    let nlmsg_type =
        (u16::from(NFNL_SUBSYS_CTNETLINK) << 8) | u16::from(IPCTNL_MSG_CT_GET_STATS_CPU);
    let total_len = mem::size_of::<NlMsgHdr>() + mem::size_of::<NfGenMsg>();
    let nlmsg_len = u32::try_from(total_len)
        .map_err(|_| "netlink request length does not fit nlmsg_len".to_string())?;
    let nfgen_family = u8::try_from(libc::AF_UNSPEC)
        .map_err(|_| "AF_UNSPEC does not fit nfgen_family".to_string())?;

    let mut buf = vec![0u8; total_len];

    let hdr = NlMsgHdr {
        nlmsg_len,
        nlmsg_type,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_DUMP,
        nlmsg_seq: seq,
        nlmsg_pid: 0,
    };

    // SAFETY: `buf` has `total_len` bytes, which includes a full `NlMsgHdr`;
    // source and destination are valid, non-overlapping byte ranges.
    unsafe {
        std::ptr::copy_nonoverlapping(
            (&raw const hdr).cast::<u8>(),
            buf.as_mut_ptr(),
            mem::size_of::<NlMsgHdr>(),
        );
    }

    let nfmsg = NfGenMsg {
        nfgen_family,
        version: NFNETLINK_V0,
        res_id: 0,
    };

    // SAFETY: the destination starts immediately after the netlink header and
    // `total_len` reserves exactly enough space for the complete `NfGenMsg`.
    unsafe {
        std::ptr::copy_nonoverlapping(
            (&raw const nfmsg).cast::<u8>(),
            buf.as_mut_ptr().add(mem::size_of::<NlMsgHdr>()),
            mem::size_of::<NfGenMsg>(),
        );
    }

    Ok(buf)
}

/// Map `CTA_STATS` attribute type to metric name
const fn attr_type_to_name(attr_type: u16) -> Option<&'static str> {
    match attr_type {
        CTA_STATS_FOUND => Some("found"),
        CTA_STATS_INVALID => Some("invalid"),
        CTA_STATS_INSERT => Some("insert"),
        CTA_STATS_INSERT_FAILED => Some("insert_failed"),
        CTA_STATS_DROP => Some("drop"),
        CTA_STATS_EARLY_DROP => Some("early_drop"),
        CTA_STATS_ERROR => Some("error"),
        CTA_STATS_SEARCH_RESTART => Some("search_restart"),
        CTA_STATS_CLASH_RESOLVE => Some("clash_resolve"),
        CTA_STATS_CHAIN_TOOLONG => Some("chain_toolong"),
        _ => None,
    }
}

/// Parse a single netlink message containing per-CPU stats
fn parse_stats_message(data: &[u8]) -> Result<CpuStats, String> {
    if data.len() < mem::size_of::<NfGenMsg>() {
        return Err("Message too short for nfgenmsg".to_string());
    }

    // Parse nfgenmsg to get CPU ID
    // SAFETY: the length check above guarantees a complete `NfGenMsg`; netlink
    // payload alignment is not guaranteed, hence `read_unaligned`.
    let nfmsg: NfGenMsg = unsafe { std::ptr::read_unaligned(data.as_ptr().cast::<NfGenMsg>()) };
    let cpu_id = u16::from_be(nfmsg.res_id);

    let mut stats = CpuStats {
        cpu_id,
        counters: HashMap::new(),
    };

    // Parse TLV attributes
    let mut offset = mem::size_of::<NfGenMsg>();
    while offset + mem::size_of::<NlAttr>() <= data.len() {
        // SAFETY: the loop condition guarantees a complete `NlAttr` remains;
        // netlink TLVs are only 4-byte aligned, so use an unaligned read.
        let attr: NlAttr =
            unsafe { std::ptr::read_unaligned(data.as_ptr().add(offset).cast::<NlAttr>()) };

        let attr_len = attr.nla_len as usize;
        if attr_len < mem::size_of::<NlAttr>() || offset + attr_len > data.len() {
            break;
        }

        let attr_type = attr.nla_type & 0x7FFF; // Mask off NLA_F_* flags
        let payload_offset = offset + mem::size_of::<NlAttr>();
        let payload_len = attr_len - mem::size_of::<NlAttr>();

        // Stats are 32-bit unsigned integers (big-endian from kernel)
        if payload_len >= 4
            && let Some(name) = attr_type_to_name(attr_type)
        {
            let value_bytes: [u8; 4] = data[payload_offset..payload_offset + 4]
                .try_into()
                .unwrap_or([0; 4]);
            let value = u64::from(u32::from_be_bytes(value_bytes));
            stats.counters.insert(name.to_string(), value);
        }

        // Move to next attribute (aligned)
        offset += nlmsg_align(attr_len);
    }

    Ok(stats)
}

/// Create a netlink socket for netfilter
fn create_netlink_socket() -> io::Result<OwnedFd> {
    // SAFETY: arguments are Linux netlink socket constants and no pointers are
    // involved. A nonnegative return value is a newly owned file descriptor.
    let raw_fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, NETLINK_NETFILTER) };
    if raw_fd < 0 {
        return Err(Error::last_os_error());
    }

    // SAFETY: `raw_fd` was just returned by `socket`, is valid, and ownership
    // has not been transferred anywhere else. `OwnedFd` closes it on all exits.
    let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };

    let timeout = libc::timeval {
        tv_sec: RECV_TIMEOUT_SECS,
        tv_usec: 0,
    };
    let timeout_len = libc::socklen_t::try_from(mem::size_of::<libc::timeval>()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "timeval size exceeds socklen_t",
        )
    })?;
    // SAFETY: `fd` is open; `timeout` lives for the duration of the call and
    // `timeout_len` is exactly the size of the pointed-to `timeval` object.
    let ret = unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            (&raw const timeout).cast::<libc::c_void>(),
            timeout_len,
        )
    };
    if ret < 0 {
        return Err(Error::last_os_error());
    }

    // SAFETY: all-zero bytes are a valid baseline for Linux `sockaddr_nl`;
    // every field used by bind is populated immediately below.
    let mut addr: libc::sockaddr_nl = unsafe { mem::zeroed() };
    addr.nl_family = libc::sa_family_t::try_from(libc::AF_NETLINK).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "AF_NETLINK exceeds sa_family_t",
        )
    })?;
    addr.nl_pid = 0;
    addr.nl_groups = 0;
    let addr_len =
        libc::socklen_t::try_from(mem::size_of::<libc::sockaddr_nl>()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "sockaddr_nl size exceeds socklen_t",
            )
        })?;

    // SAFETY: `addr` is a fully initialized `sockaddr_nl`, its pointer remains
    // valid for the call, and `addr_len` describes exactly that object.
    let ret = unsafe {
        libc::bind(
            fd.as_raw_fd(),
            (&raw const addr).cast::<libc::sockaddr>(),
            addr_len,
        )
    };
    if ret < 0 {
        return Err(Error::last_os_error());
    }

    Ok(fd)
}

fn conntrack_module_loaded() -> bool {
    if Path::new("/proc/net/stat/nf_conntrack").exists() {
        return true;
    }

    if let Ok(modules) = procfs::modules() {
        return modules.contains_key("nf_conntrack")
            || modules.contains_key("nf_conntrack_netlink");
    }

    false
}

/// Check if conntrack stats collection is available.
/// Returns true if we can create a netlink socket (requires `CAP_NET_ADMIN` or root).
/// Collect conntrack statistics via netlink.
/// Returns per-CPU statistics or an error.
pub fn collect_stats() -> Result<Vec<CpuStats>, String> {
    let socket =
        create_netlink_socket().map_err(|e| format!("Failed to create netlink socket: {e}"))?;
    let fd = socket.as_raw_fd();

    let request = create_stats_request(1)?;
    // SAFETY: `fd` remains owned by `socket`; `request` is a live contiguous
    // buffer and the pointer/length pair describes its complete contents.
    let sent = unsafe {
        libc::send(
            fd,
            request.as_ptr().cast::<libc::c_void>(),
            request.len(),
            0,
        )
    };

    if sent < 0 {
        return Err(format!(
            "Failed to send netlink request: {}",
            Error::last_os_error()
        ));
    }

    // Receive responses
    let mut all_stats = Vec::new();
    let mut buffer = vec![0u8; 16384];

    loop {
        // Read the sender address so replies that did not come from the kernel
        // can be discarded rather than parsed as statistics.
        // SAFETY: zero initialization is a valid storage state for the kernel
        // to fill as `sockaddr_nl` through `recvfrom`.
        let mut addr: libc::sockaddr_nl = unsafe { mem::zeroed() };
        let mut addr_len = libc::socklen_t::try_from(mem::size_of::<libc::sockaddr_nl>())
            .map_err(|_| "sockaddr_nl size exceeds socklen_t".to_string())?;
        // SAFETY: `fd` is open; `buffer` provides writable storage for its full
        // length; `addr` and `addr_len` are valid output pointers for recvfrom.
        let len = unsafe {
            libc::recvfrom(
                fd,
                buffer.as_mut_ptr().cast::<libc::c_void>(),
                buffer.len(),
                0,
                (&raw mut addr).cast::<libc::sockaddr>(),
                &raw mut addr_len,
            )
        };

        if len < 0 {
            let err = Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock || err.kind() == io::ErrorKind::TimedOut {
                // Timed out waiting for NLMSG_DONE; return what we have.
                return Ok(all_stats);
            }
            return Err(format!("Failed to receive netlink response: {err}"));
        }

        if len == 0 {
            break;
        }

        // The kernel always sends from port id 0.
        if addr.nl_pid != 0 {
            continue;
        }

        let len = usize::try_from(len)
            .map_err(|_| "positive recvfrom length does not fit usize".to_string())?;

        // Parse netlink messages in buffer
        let mut offset = 0;
        while offset + mem::size_of::<NlMsgHdr>() <= len {
            // SAFETY: the loop condition guarantees a complete `NlMsgHdr` is
            // inside the received prefix; the kernel buffer need not be aligned.
            let hdr: NlMsgHdr =
                unsafe { std::ptr::read_unaligned(buffer.as_ptr().add(offset).cast::<NlMsgHdr>()) };

            let msg_len = hdr.nlmsg_len as usize;
            if msg_len < mem::size_of::<NlMsgHdr>() || offset + msg_len > len {
                break;
            }

            // Check message type
            if hdr.nlmsg_type == NLMSG_DONE {
                return Ok(all_stats);
            }

            if hdr.nlmsg_type == NLMSG_ERROR {
                // Parse error code
                if msg_len >= mem::size_of::<NlMsgHdr>() + 4 {
                    let error_offset = offset + mem::size_of::<NlMsgHdr>();
                    // SAFETY: `msg_len` proved that at least four payload bytes
                    // follow the header; an unaligned i32 read is therefore in-bounds.
                    let error_code: i32 = unsafe {
                        std::ptr::read_unaligned(buffer.as_ptr().add(error_offset).cast::<i32>())
                    };
                    if error_code != 0 {
                        return Err(format!(
                            "Netlink error: {}",
                            Error::from_raw_os_error(-error_code)
                        ));
                    }
                }
                offset += nlmsg_align(msg_len);
                continue;
            }

            // Parse stats message
            let payload_offset = offset + mem::size_of::<NlMsgHdr>();
            let payload_len = msg_len - mem::size_of::<NlMsgHdr>();

            if payload_len > 0 {
                let payload = &buffer[payload_offset..payload_offset + payload_len];
                match parse_stats_message(payload) {
                    Ok(stats) => all_stats.push(stats),
                    Err(err) => {
                        eprintln!("Failed to parse conntrack stats message: {err}");
                    }
                }
            }

            offset += nlmsg_align(msg_len);
        }
    }

    Ok(all_stats)
}

pub fn update_metrics() {
    if !conntrack_module_loaded() {
        return;
    }

    let metrics = metrics();
    match collect_stats() {
        Ok(all_stats) => {
            // The kernel reports one message per online CPU.
            metrics.conntrack.reset();
            for cpu_stats in all_stats {
                let cpu_label = cpu_stats.cpu_id.to_string();
                for (name, value) in cpu_stats.counters {
                    metrics
                        .conntrack
                        .with_label_values(&[cpu_label.as_str(), name.as_str()])
                        .set(prometheus_u64(value));
                }
            }
        }
        Err(err) => {
            eprintln!("Failed to collect conntrack stats: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_stats_request() {
        let request = create_stats_request(1).expect("fixed-size netlink request is representable");
        assert_eq!(request.len(), 20); // 16 (nlmsghdr) + 4 (nfgenmsg)

        // Verify nlmsg_type
        let hdr: NlMsgHdr =
            unsafe { std::ptr::read_unaligned(request.as_ptr() as *const NlMsgHdr) };
        let expected_type =
            ((NFNL_SUBSYS_CTNETLINK as u16) << 8) | (IPCTNL_MSG_CT_GET_STATS_CPU as u16);
        assert_eq!(hdr.nlmsg_type, expected_type);
        assert_eq!(hdr.nlmsg_flags, NLM_F_REQUEST | NLM_F_DUMP);
    }

    #[test]
    fn test_attr_type_to_name() {
        assert_eq!(attr_type_to_name(CTA_STATS_FOUND), Some("found"));
        assert_eq!(attr_type_to_name(CTA_STATS_DROP), Some("drop"));
        assert_eq!(attr_type_to_name(0), None);
        assert_eq!(attr_type_to_name(100), None);
    }
}
