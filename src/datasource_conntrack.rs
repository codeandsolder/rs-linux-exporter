//! Conntrack statistics collector via generated netlink bindings.
//!
//! This module queries per-CPU conntrack statistics using the kernel's
//! netfilter netlink protocol, similar to `conntrack -S`.

use crate::metric_support::{RegisterMetricResultExt, prometheus_u64};
use netlink_bindings::builtin::Nlmsghdr;
use netlink_bindings::conntrack::{self, ConntrackStatsAttrs, OpGetStatsDump};
use netlink_bindings::traits::{NetlinkRequest, Protocol};
use prometheus::GaugeVec;
use rustix::net::netlink::SocketAddrNetlink;
use rustix::net::sockopt::{Timeout, set_socket_timeout};
use rustix::net::{
    AddressFamily, RecvFlags, SendFlags, SocketFlags, SocketType, bind, netlink, recvfrom, sendto,
    socket_with,
};
use std::io;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

const RECV_TIMEOUT: Duration = Duration::from_secs(2);
const SEQUENCE: u32 = 1;
const RECV_BUFFER_SIZE: usize = 16 * 1024;

#[derive(Debug, PartialEq, Eq)]
struct ConntrackStat {
    cpu_id: u16,
    name: &'static str,
    value: u32,
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

const fn align_netlink_message(len: usize) -> usize {
    (len + 3) & !3
}

const fn stat_name_value(attr: &ConntrackStatsAttrs) -> Option<(&'static str, u32)> {
    match attr {
        ConntrackStatsAttrs::Found(value) => Some(("found", *value)),
        ConntrackStatsAttrs::Invalid(value) => Some(("invalid", *value)),
        ConntrackStatsAttrs::Insert(value) => Some(("insert", *value)),
        ConntrackStatsAttrs::InsertFailed(value) => Some(("insert_failed", *value)),
        ConntrackStatsAttrs::Drop(value) => Some(("drop", *value)),
        ConntrackStatsAttrs::EarlyDrop(value) => Some(("early_drop", *value)),
        ConntrackStatsAttrs::Error(value) => Some(("error", *value)),
        ConntrackStatsAttrs::SearchRestart(value) => Some(("search_restart", *value)),
        ConntrackStatsAttrs::ClashResolve(value) => Some(("clash_resolve", *value)),
        ConntrackStatsAttrs::ChainToolong(value) => Some(("chain_toolong", *value)),
        ConntrackStatsAttrs::Searched(_)
        | ConntrackStatsAttrs::New(_)
        | ConntrackStatsAttrs::Ignore(_)
        | ConntrackStatsAttrs::Delete(_)
        | ConntrackStatsAttrs::DeleteList(_) => None,
    }
}

fn conntrack_module_loaded() -> bool {
    if Path::new("/proc/net/stat/nf_conntrack").exists() {
        return true;
    }

    procfs::modules().is_ok_and(|modules| {
        modules.contains_key("nf_conntrack") || modules.contains_key("nf_conntrack_netlink")
    })
}

fn decode_error_code(payload: &[u8]) -> Result<i32, String> {
    let bytes: [u8; 4] = payload
        .get(..4)
        .ok_or_else(|| "netlink completion/error message is missing its error code".to_string())?
        .try_into()
        .map_err(|_| "netlink error code has an invalid width".to_string())?;
    Ok(i32::from_ne_bytes(bytes))
}

struct EncodedRequest {
    wire: Vec<u8>,
    response_type: u16,
}

enum PacketDisposition {
    Continue,
    Done,
}

fn encode_request() -> Result<EncodedRequest, String> {
    let mut nf_header = conntrack::Nfgenmsg::new();
    nf_header.nfgen_family = u8::try_from(libc::AF_UNSPEC)
        .map_err(|_| "AF_UNSPEC does not fit nfgen_family".to_string())?;
    nf_header.version = 0;
    nf_header.set_res_id(0);

    let request = conntrack::Request::new().op_get_stats_dump(&nf_header);
    let Protocol::Raw {
        protonum,
        request_type,
    } = request.protocol()
    else {
        return Err("conntrack stats unexpectedly used generic netlink".to_string());
    };
    let expected_protocol = u16::try_from(libc::NETLINK_NETFILTER)
        .map_err(|_| "NETLINK_NETFILTER does not fit u16".to_string())?;
    if protonum != expected_protocol {
        return Err(format!("unexpected conntrack netlink protocol {protonum}"));
    }

    let payload = request.payload();
    let message_len = Nlmsghdr::len()
        .checked_add(payload.len())
        .ok_or_else(|| "conntrack request length overflow".to_string())?;
    let message_len = u32::try_from(message_len)
        .map_err(|_| "conntrack request length does not fit nlmsghdr".to_string())?;
    let request_flag = u16::try_from(libc::NLM_F_REQUEST)
        .map_err(|_| "NLM_F_REQUEST does not fit u16".to_string())?;
    let header = Nlmsghdr {
        len: message_len,
        r#type: request_type,
        flags: request.flags() | request_flag,
        seq: SEQUENCE,
        pid: 0,
    };
    let mut wire = Vec::with_capacity(message_len as usize);
    wire.extend_from_slice(header.as_slice());
    wire.extend_from_slice(payload);
    Ok(EncodedRequest {
        wire,
        response_type: request_type,
    })
}

fn open_netlink_socket() -> Result<std::os::fd::OwnedFd, String> {
    let socket = socket_with(
        AddressFamily::NETLINK,
        SocketType::RAW,
        SocketFlags::CLOEXEC,
        Some(netlink::NETFILTER),
    )
    .map_err(|err| format!("failed to create conntrack netlink socket: {err}"))?;
    set_socket_timeout(&socket, Timeout::Recv, Some(RECV_TIMEOUT))
        .map_err(|err| format!("failed to set conntrack receive timeout: {err}"))?;
    bind(&socket, &SocketAddrNetlink::new(0, 0))
        .map_err(|err| format!("failed to bind conntrack netlink socket: {err}"))?;
    Ok(socket)
}

fn decode_stats_payload(payload: &[u8], result: &mut Vec<ConntrackStat>) -> Result<(), String> {
    let (header, attrs) = OpGetStatsDump::decode_reply(payload);
    for attr in attrs {
        let attr =
            attr.map_err(|err| format!("failed to decode conntrack stats attribute: {err}"))?;
        if let Some((name, value)) = stat_name_value(&attr) {
            result.push(ConntrackStat {
                cpu_id: header.res_id(),
                name,
                value,
            });
        }
    }
    Ok(())
}

fn decode_packet(
    packet: &[u8],
    response_type: u16,
    result: &mut Vec<ConntrackStat>,
) -> Result<PacketDisposition, String> {
    let mut offset: usize = 0;
    while offset < packet.len() {
        let header_end = offset
            .checked_add(Nlmsghdr::len())
            .ok_or_else(|| "netlink header offset overflow".to_string())?;
        let header_bytes = packet
            .get(offset..header_end)
            .ok_or_else(|| "truncated conntrack netlink header".to_string())?;
        let header = Nlmsghdr::new_from_slice(header_bytes)
            .ok_or_else(|| "invalid conntrack netlink header width".to_string())?;
        let message_len = usize::try_from(header.len)
            .map_err(|_| "netlink message length does not fit usize".to_string())?;
        if message_len < Nlmsghdr::len() {
            return Err(format!("invalid conntrack netlink length {message_len}"));
        }
        let message_end = offset
            .checked_add(message_len)
            .ok_or_else(|| "netlink message offset overflow".to_string())?;
        let payload = packet
            .get(header_end..message_end)
            .ok_or_else(|| "truncated conntrack netlink message".to_string())?;

        if header.seq == SEQUENCE {
            match decode_message(header.r#type, payload, response_type, result)? {
                PacketDisposition::Done => return Ok(PacketDisposition::Done),
                PacketDisposition::Continue => {}
            }
        }

        offset = offset
            .checked_add(align_netlink_message(message_len))
            .ok_or_else(|| "netlink alignment overflow".to_string())?;
        if offset > packet.len() {
            return Err("conntrack netlink message alignment exceeds datagram".to_string());
        }
    }
    Ok(PacketDisposition::Continue)
}

fn decode_message(
    message_type: u16,
    payload: &[u8],
    response_type: u16,
    result: &mut Vec<ConntrackStat>,
) -> Result<PacketDisposition, String> {
    match i32::from(message_type) {
        libc::NLMSG_DONE => {
            if !payload.is_empty() {
                let code = decode_error_code(payload)?;
                if code != 0 {
                    return Err(format!(
                        "conntrack dump failed: {}",
                        io::Error::from_raw_os_error(-code)
                    ));
                }
            }
            Ok(PacketDisposition::Done)
        }
        libc::NLMSG_ERROR => {
            let code = decode_error_code(payload)?;
            if code != 0 {
                return Err(format!(
                    "conntrack request failed: {}",
                    io::Error::from_raw_os_error(-code)
                ));
            }
            Ok(PacketDisposition::Continue)
        }
        libc::NLMSG_NOOP => Ok(PacketDisposition::Continue),
        libc::NLMSG_OVERRUN => Err("conntrack netlink receive overrun".to_string()),
        kind if kind == i32::from(response_type) => {
            decode_stats_payload(payload, result)?;
            Ok(PacketDisposition::Continue)
        }
        kind => Err(format!("unexpected conntrack netlink message type {kind}")),
    }
}

fn collect_stats() -> Result<Vec<ConntrackStat>, String> {
    let request = encode_request()?;
    let socket = open_netlink_socket()?;
    let kernel = SocketAddrNetlink::new(0, 0);
    let sent = sendto(&socket, &request.wire, SendFlags::empty(), &kernel)
        .map_err(|err| format!("failed to send conntrack request: {err}"))?;
    if sent != request.wire.len() {
        return Err(format!(
            "short conntrack netlink send: sent {sent} of {} bytes",
            request.wire.len()
        ));
    }

    let mut result = Vec::new();
    let mut buffer = [0_u8; RECV_BUFFER_SIZE];
    loop {
        let (read_len, packet_len, sender) = recvfrom(&socket, &mut buffer, RecvFlags::empty())
            .map_err(|err| format!("failed to receive conntrack response: {err}"))?;
        if packet_len > buffer.len() {
            return Err(format!(
                "conntrack netlink datagram was truncated: {packet_len} > {}",
                buffer.len()
            ));
        }
        if let Some(sender) = sender {
            let sender = SocketAddrNetlink::try_from(sender)
                .map_err(|err| format!("unexpected conntrack sender address: {err}"))?;
            if sender.pid() != 0 {
                continue;
            }
        }
        if matches!(
            decode_packet(&buffer[..read_len], request.response_type, &mut result)?,
            PacketDisposition::Done
        ) {
            return Ok(result);
        }
    }
}

pub fn update_metrics() {
    let metrics = metrics();
    metrics.conntrack.reset();

    if !conntrack_module_loaded() {
        return;
    }

    match collect_stats() {
        Ok(stats) => {
            for stat in stats {
                let cpu = stat.cpu_id.to_string();
                metrics
                    .conntrack
                    .with_label_values(&[cpu.as_str(), stat.name])
                    .set(prometheus_u64(u64::from(stat.value)));
            }
        }
        Err(err) => eprintln!("Failed to collect conntrack stats: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{align_netlink_message, decode_error_code, stat_name_value};
    use netlink_bindings::conntrack::ConntrackStatsAttrs;

    #[test]
    fn stat_mapping_preserves_exported_metric_names() {
        assert_eq!(
            stat_name_value(&ConntrackStatsAttrs::Found(3)),
            Some(("found", 3))
        );
        assert_eq!(
            stat_name_value(&ConntrackStatsAttrs::Invalid(4)),
            Some(("invalid", 4))
        );
        assert_eq!(stat_name_value(&ConntrackStatsAttrs::Searched(5)), None);
    }

    #[test]
    fn netlink_alignment_is_four_bytes() {
        assert_eq!(align_netlink_message(16), 16);
        assert_eq!(align_netlink_message(17), 20);
        assert_eq!(align_netlink_message(20), 20);
    }

    #[test]
    fn completion_error_codes_use_native_endian() {
        assert_eq!(decode_error_code(&0_i32.to_ne_bytes()), Ok(0));
        assert_eq!(
            decode_error_code(&(-libc::EPERM).to_ne_bytes()),
            Ok(-libc::EPERM)
        );
        assert!(decode_error_code(&[0, 1, 2]).is_err());
    }
}
