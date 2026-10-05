//! Conntrack statistics collector via netlink protocol.
//!
//! This module queries per-CPU conntrack statistics using the netfilter netlink
//! protocol, similar to `conntrack -S`.

use prometheus::GaugeVec;
use std::collections::HashMap;
use std::io::{self, Error};
use std::mem;
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
            .expect("register conntrack"),
        }
    }
}

static CONNTRACK_METRICS: OnceLock<ConntrackMetrics> = OnceLock::new();

fn metrics() -> &'static ConntrackMetrics {
    CONNTRACK_METRICS.get_or_init(ConntrackMetrics::new)
}

/// Align to 4-byte boundary (`NLMSG_ALIGN`)
#[inline]
fn nlmsg_align(len: usize) -> usize {
    (len + 3) & !3
}

/// Build the netlink request message for conntrack stats
fn create_stats_request(seq: u32) -> Vec<u8> {
    let nlmsg_type =
        (u16::from(NFNL_SUBSYS_CTNETLINK) << 8) | u16::from(IPCTNL_MSG_CT_GET_STATS_CPU);
    let total_len = mem::size_of::<NlMsgHdr>() + mem::size_of::<NfGenMsg>();

    let mut buf = vec![0u8; total_len];

    // Build nlmsghdr
    let hdr = NlMsgHdr {
        nlmsg_len: total_len as u32,
        nlmsg_type,
        nlmsg_flags: NLM_F_REQUEST | NLM_F_DUMP,
        nlmsg_seq: seq,
        nlmsg_pid: 0,
    };

    // Copy header to buffer
    unsafe {
        std::ptr::copy_nonoverlapping(
            (&raw const hdr).cast::<u8>(),
            buf.as_mut_ptr(),
            mem::size_of::<NlMsgHdr>(),
        );
    }

    // Build nfgenmsg
    let nfmsg = NfGenMsg {
        nfgen_family: libc::AF_UNSPEC as u8,
        version: NFNETLINK_V0,
        res_id: 0,
    };

    // Copy nfgenmsg to buffer
    unsafe {
        std::ptr::copy_nonoverlapping(
            (&raw const nfmsg).cast::<u8>(),
            buf.as_mut_ptr().add(mem::size_of::<NlMsgHdr>()),
            mem::size_of::<NfGenMsg>(),
        );
    }

    buf
}

/// Map `CTA_STATS` attribute type to metric name
fn attr_type_to_name(attr_type: u16) -> Option<&'static str> {
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
    let nfmsg: NfGenMsg = unsafe { std::ptr::read_unaligned(data.as_ptr().cast::<NfGenMsg>()) };
    let cpu_id = u16::from_be(nfmsg.res_id);

    let mut stats = CpuStats {
        cpu_id,
        counters: HashMap::new(),
    };

    // Parse TLV attributes
    let mut offset = mem::size_of::<NfGenMsg>();
    while offset + mem::size_of::<NlAttr>() <= data.len() {
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
fn create_netlink_socket() -> io::Result<i32> {
    let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, NETLINK_NETFILTER) };

    if fd < 0 {
        return Err(Error::last_os_error());
    }

    // Without a receive timeout a dump that never reaches NLMSG_DONE - for
    // instance because a malformed message aborted the parse loop early -
    // would block the scrape forever.
    let timeout = libc::timeval {
        tv_sec: RECV_TIMEOUT_SECS,
        tv_usec: 0,
    };
    let ret = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            (&raw const timeout).cast::<libc::c_void>(),
            mem::size_of::<libc::timeval>() as u32,
        )
    };

    if ret < 0 {
        let err = Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(err);
    }

    // Bind the socket
    let mut addr: libc::sockaddr_nl = unsafe { mem::zeroed() };
    addr.nl_family = libc::AF_NETLINK as u16;
    addr.nl_pid = 0; // Let kernel assign
    addr.nl_groups = 0;

    let ret = unsafe {
        libc::bind(
            fd,
            (&raw const addr).cast::<libc::sockaddr>(),
            mem::size_of::<libc::sockaddr_nl>() as u32,
        )
    };

    if ret < 0 {
        let err = Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(err);
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
    // Create socket
    let fd =
        create_netlink_socket().map_err(|e| format!("Failed to create netlink socket: {e}"))?;

    // Ensure socket is closed on exit
    struct SocketGuard(i32);
    impl Drop for SocketGuard {
        fn drop(&mut self) {
            unsafe { libc::close(self.0) };
        }
    }
    let _guard = SocketGuard(fd);

    // Build and send request
    let request = create_stats_request(1);
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
        let mut addr: libc::sockaddr_nl = unsafe { mem::zeroed() };
        let mut addr_len = mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t;
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

        let len = len as usize;

        // Parse netlink messages in buffer
        let mut offset = 0;
        while offset + mem::size_of::<NlMsgHdr>() <= len {
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
                        .set(value as f64);
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
        let request = create_stats_request(1);
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
