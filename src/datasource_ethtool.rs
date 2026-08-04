#![allow(dead_code)]

use crate::runtime::debug_enabled;
use prometheus::GaugeVec;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::mem;
use std::path::Path;
use std::sync::OnceLock;

const NETLINK_GENERIC: i32 = 16;

const NLM_F_REQUEST: u16 = 0x0001;
const NLM_F_DUMP: u16 = 0x0300;
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;

const GENL_ID_CTRL: u16 = 0x10;
const CTRL_CMD_GETFAMILY: u8 = 3;
const CTRL_ATTR_FAMILY_ID: u16 = 1;
const CTRL_ATTR_FAMILY_NAME: u16 = 2;

const ETHTOOL_GENL_NAME: &str = "ethtool";
const ETHTOOL_GENL_VERSION: u8 = 1;

const ETHTOOL_MSG_STRSET_GET: u8 = 1;
const ETHTOOL_MSG_STATS_GET: u8 = 32;

const ETHTOOL_A_HEADER_DEV_NAME: u16 = 2;
const ETHTOOL_A_STRSET_HEADER: u16 = 1;
const ETHTOOL_A_STRSET_STRINGSETS: u16 = 2;
const ETHTOOL_A_STRINGSETS_STRINGSET: u16 = 1;
const ETHTOOL_A_STRINGSET_ID: u16 = 1;
const ETHTOOL_A_STRINGSET_STRINGS: u16 = 3;
const ETHTOOL_A_STRINGS_STRING: u16 = 1;
const ETHTOOL_A_STRING_INDEX: u16 = 1;
const ETHTOOL_A_STRING_VALUE: u16 = 2;

const ETHTOOL_A_STATS_HEADER: u16 = 2;
const ETHTOOL_A_STATS_GROUPS: u16 = 3;
const ETHTOOL_A_STATS_GRP: u16 = 4;

const ETHTOOL_A_STATS_GRP_ID: u16 = 2;
const ETHTOOL_A_STATS_GRP_SS_ID: u16 = 3;
const ETHTOOL_A_STATS_GRP_STAT: u16 = 4;

const ETHTOOL_A_BITSET_NOMASK: u16 = 1;
const ETHTOOL_A_BITSET_BITS: u16 = 3;
const ETHTOOL_A_BITSET_BIT: u16 = 1;
const ETHTOOL_A_BITSET_BIT_NAME: u16 = 2;
const ETHTOOL_A_BITSET_BIT_VALUE: u16 = 3;

const NLA_F_NESTED: u16 = 0x8000;

const ETH_SS_STATS_ETH_PHY: u32 = 17;
const ETH_SS_STATS_ETH_MAC: u32 = 18;
const ETH_SS_STATS_ETH_CTRL: u32 = 19;
const ETH_SS_STATS_RMON: u32 = 20;
const ETH_SS_STATS_PHY: u32 = 21;

type StatsGroup = (u32, u32, Vec<(u32, u64)>);

#[repr(C)]
struct NlMsgHdr {
    nlmsg_len: u32,
    nlmsg_type: u16,
    nlmsg_flags: u16,
    nlmsg_seq: u32,
    nlmsg_pid: u32,
}

#[repr(C)]
struct GenlMsgHdr {
    cmd: u8,
    version: u8,
    reserved: u16,
}

#[repr(C)]
struct NlAttr {
    nla_len: u16,
    nla_type: u16,
}

#[repr(C)]
struct NlMsgErr {
    error: i32,
    msg: NlMsgHdr,
}

struct EthtoolMetrics {
    ethtool_stats: GaugeVec,
}

impl EthtoolMetrics {
    fn new() -> Self {
        Self {
            ethtool_stats: prometheus::register_gauge_vec!(
                "ethtool_stats",
                "Ethernet statistics via ethtool netlink",
                &["interface", "stat"]
            )
            .expect("register ethtool_stats"),
        }
    }
}

static ETHTOOL_METRICS: OnceLock<EthtoolMetrics> = OnceLock::new();

fn metrics() -> &'static EthtoolMetrics {
    ETHTOOL_METRICS.get_or_init(EthtoolMetrics::new)
}

fn nlmsg_align(len: usize) -> usize {
    (len + 3) & !3
}

fn nla_align(len: usize) -> usize {
    (len + 3) & !3
}

fn add_attr(buf: &mut Vec<u8>, attr_type: u16, payload: &[u8]) {
    let len = mem::size_of::<NlAttr>() + payload.len();
    let aligned_len = nla_align(len);
    let header = NlAttr {
        nla_len: len as u16,
        nla_type: attr_type,
    };
    buf.extend_from_slice(unsafe {
        std::slice::from_raw_parts(
            &header as *const NlAttr as *const u8,
            mem::size_of::<NlAttr>(),
        )
    });
    buf.extend_from_slice(payload);
    if aligned_len > len {
        buf.resize(buf.len() + (aligned_len - len), 0);
    }
}

fn add_attr_u32(buf: &mut Vec<u8>, attr_type: u16, value: u32) {
    add_attr(buf, attr_type, &value.to_ne_bytes());
}

fn add_attr_string(buf: &mut Vec<u8>, attr_type: u16, value: &str) {
    let mut bytes = value.as_bytes().to_vec();
    bytes.push(0);
    add_attr(buf, attr_type, &bytes);
}

fn start_nested(buf: &mut Vec<u8>, attr_type: u16) -> usize {
    let start = buf.len();
    let header = NlAttr {
        nla_len: 0,
        nla_type: attr_type | NLA_F_NESTED,
    };
    buf.extend_from_slice(unsafe {
        std::slice::from_raw_parts(
            &header as *const NlAttr as *const u8,
            mem::size_of::<NlAttr>(),
        )
    });
    start
}

fn end_nested(buf: &mut Vec<u8>, start: usize) {
    let len = buf.len() - start;
    let aligned_len = nla_align(len);
    buf[start..start + 2].copy_from_slice(&(len as u16).to_ne_bytes());
    if aligned_len > len {
        buf.resize(buf.len() + (aligned_len - len), 0);
    }
}

fn build_message(nlmsg_type: u16, flags: u16, seq: u32, cmd: u8, version: u8) -> Vec<u8> {
    let mut buf = vec![0u8; mem::size_of::<NlMsgHdr>() + mem::size_of::<GenlMsgHdr>()];
    let hdr = NlMsgHdr {
        nlmsg_len: buf.len() as u32,
        nlmsg_type,
        nlmsg_flags: flags,
        nlmsg_seq: seq,
        nlmsg_pid: 0,
    };
    let genl = GenlMsgHdr {
        cmd,
        version,
        reserved: 0,
    };
    unsafe {
        std::ptr::copy_nonoverlapping(
            &hdr as *const NlMsgHdr as *const u8,
            buf.as_mut_ptr(),
            mem::size_of::<NlMsgHdr>(),
        );
        std::ptr::copy_nonoverlapping(
            &genl as *const GenlMsgHdr as *const u8,
            buf.as_mut_ptr().add(mem::size_of::<NlMsgHdr>()),
            mem::size_of::<GenlMsgHdr>(),
        );
    }
    buf
}

fn finalize_message(buf: &mut [u8]) {
    let len = buf.len() as u32;
    buf[..4].copy_from_slice(&len.to_ne_bytes());
}

fn parse_attrs(mut data: &[u8]) -> Vec<(u16, &[u8])> {
    let mut attrs = Vec::new();
    while data.len() >= mem::size_of::<NlAttr>() {
        let header = unsafe { std::ptr::read_unaligned(data.as_ptr() as *const NlAttr) };
        let len = header.nla_len as usize;
        if len < mem::size_of::<NlAttr>() || len > data.len() {
            break;
        }
        let payload = &data[mem::size_of::<NlAttr>()..len];
        let attr_type = header.nla_type & !NLA_F_NESTED;
        attrs.push((attr_type, payload));
        // The final attribute of a message need not be padded out, so the
        // aligned length can run past the end of the buffer.
        let advance = nla_align(len);
        if advance >= data.len() {
            break;
        }
        data = &data[advance..];
    }
    attrs
}

fn parse_u32(data: &[u8]) -> Option<u32> {
    if data.len() < 4 {
        return None;
    }
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&data[..4]);
    Some(u32::from_ne_bytes(buf))
}

fn parse_u16(data: &[u8]) -> Option<u16> {
    if data.len() < 2 {
        return None;
    }
    let mut buf = [0u8; 2];
    buf.copy_from_slice(&data[..2]);
    Some(u16::from_ne_bytes(buf))
}

fn parse_u64(data: &[u8]) -> Option<u64> {
    if data.len() < 8 {
        return None;
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[..8]);
    Some(u64::from_ne_bytes(buf))
}

fn parse_string(data: &[u8]) -> Option<String> {
    let nul = data.iter().position(|b| *b == 0).unwrap_or(data.len());
    String::from_utf8(data[..nul].to_vec()).ok()
}

fn create_netlink_socket() -> io::Result<i32> {
    let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_RAW, NETLINK_GENERIC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let timeout = libc::timeval {
        tv_sec: 1,
        tv_usec: 0,
    };
    let ret = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            &timeout as *const libc::timeval as *const libc::c_void,
            mem::size_of::<libc::timeval>() as u32,
        )
    };
    if ret < 0 {
        let err = io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(err);
    }
    let mut addr: libc::sockaddr_nl = unsafe { mem::zeroed() };
    addr.nl_family = libc::AF_NETLINK as u16;
    addr.nl_pid = unsafe { libc::getpid() as u32 };
    addr.nl_groups = 0;
    let ret = unsafe {
        libc::bind(
            fd,
            &addr as *const libc::sockaddr_nl as *const libc::sockaddr,
            mem::size_of::<libc::sockaddr_nl>() as u32,
        )
    };
    if ret < 0 {
        let err = io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(err);
    }
    Ok(fd)
}

fn send_message(fd: i32, buf: &[u8]) -> io::Result<()> {
    let sent = unsafe { libc::send(fd, buf.as_ptr() as *const libc::c_void, buf.len(), 0) };
    if sent < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn recv_messages(fd: i32, seq: u32) -> io::Result<Vec<Vec<u8>>> {
    let mut responses = Vec::new();
    let mut buffer = vec![0u8; 16384];
    loop {
        let len = unsafe {
            libc::recv(
                fd,
                buffer.as_mut_ptr() as *mut libc::c_void,
                buffer.len(),
                0,
            )
        };
        if len < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock || err.kind() == io::ErrorKind::TimedOut {
                break;
            }
            return Err(err);
        }
        if len == 0 {
            break;
        }
        let len = len as usize;
        let mut offset = 0;
        while offset + mem::size_of::<NlMsgHdr>() <= len {
            let hdr: NlMsgHdr =
                unsafe { std::ptr::read_unaligned(buffer.as_ptr().add(offset) as *const NlMsgHdr) };
            // Validate the length before using it to advance, otherwise a
            // message claiming nlmsg_len == 0 never moves the cursor and the
            // loop spins forever.
            let msg_len = hdr.nlmsg_len as usize;
            if msg_len < mem::size_of::<NlMsgHdr>() || offset + msg_len > len {
                break;
            }
            if hdr.nlmsg_seq != seq {
                offset += nlmsg_align(msg_len);
                continue;
            }
            if hdr.nlmsg_type == NLMSG_DONE {
                return Ok(responses);
            }
            if hdr.nlmsg_type == NLMSG_ERROR {
                let err_offset = offset + mem::size_of::<NlMsgHdr>();
                if err_offset + mem::size_of::<NlMsgErr>() <= len {
                    let err: NlMsgErr = unsafe {
                        std::ptr::read_unaligned(buffer.as_ptr().add(err_offset) as *const NlMsgErr)
                    };
                    if err.error != 0 {
                        return Err(io::Error::from_raw_os_error(-err.error));
                    }
                }
                offset += nlmsg_align(msg_len);
                continue;
            }
            let payload_offset = offset + mem::size_of::<NlMsgHdr>();
            let payload_len = msg_len - mem::size_of::<NlMsgHdr>();
            if payload_len > 0 {
                responses.push(buffer[payload_offset..payload_offset + payload_len].to_vec());
            }
            offset += nlmsg_align(msg_len);
        }
    }
    Ok(responses)
}

fn get_ethtool_family_id(fd: i32, seq: &mut u32) -> io::Result<u16> {
    *seq += 1;
    let mut msg = build_message(GENL_ID_CTRL, NLM_F_REQUEST, *seq, CTRL_CMD_GETFAMILY, 1);
    add_attr_string(&mut msg, CTRL_ATTR_FAMILY_NAME, ETHTOOL_GENL_NAME);
    finalize_message(&mut msg);
    send_message(fd, &msg)?;
    let replies = recv_messages(fd, *seq)?;
    if debug_enabled() {
        eprintln!("ethtool: ctrl getfamily replies={}", replies.len());
    }
    for reply in replies {
        if reply.len() < mem::size_of::<GenlMsgHdr>() {
            continue;
        }
        let attr_start = mem::size_of::<GenlMsgHdr>();
        let attrs = parse_attrs(&reply[attr_start..]);
        if debug_enabled() {
            let mut summary = Vec::new();
            for (attr_type, payload) in &attrs {
                if *attr_type == CTRL_ATTR_FAMILY_NAME {
                    if let Some(name) = parse_string(payload) {
                        summary.push(format!("name={name}"));
                    }
                } else {
                    summary.push(format!("attr={attr_type}"));
                }
            }
            eprintln!("ethtool: ctrl attrs {}", summary.join(", "));
        }
        for (attr_type, payload) in attrs {
            if attr_type == CTRL_ATTR_FAMILY_ID
                && let Some(id) = parse_u16(payload)
            {
                return Ok(id);
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "ethtool family id not found",
    ))
}

fn extract_header_name(header_payload: &[u8]) -> Option<String> {
    for (attr_type, payload) in parse_attrs(header_payload) {
        if attr_type == ETHTOOL_A_HEADER_DEV_NAME {
            return parse_string(payload);
        }
    }
    None
}

fn request_stringsets(
    fd: i32,
    family_id: u16,
    seq: &mut u32,
    dev: &str,
) -> io::Result<HashMap<u32, Vec<String>>> {
    *seq += 1;
    let mut msg = build_message(
        family_id,
        NLM_F_REQUEST | NLM_F_DUMP,
        *seq,
        ETHTOOL_MSG_STRSET_GET,
        ETHTOOL_GENL_VERSION,
    );

    let header_start = start_nested(&mut msg, ETHTOOL_A_STRSET_HEADER);
    end_nested(&mut msg, header_start);

    let sets_start = start_nested(&mut msg, ETHTOOL_A_STRSET_STRINGSETS);
    for id in [
        ETH_SS_STATS_ETH_PHY,
        ETH_SS_STATS_ETH_MAC,
        ETH_SS_STATS_ETH_CTRL,
        ETH_SS_STATS_RMON,
        ETH_SS_STATS_PHY,
    ] {
        let set_start = start_nested(&mut msg, ETHTOOL_A_STRINGSETS_STRINGSET);
        add_attr_u32(&mut msg, ETHTOOL_A_STRINGSET_ID, id);
        end_nested(&mut msg, set_start);
    }
    end_nested(&mut msg, sets_start);

    finalize_message(&mut msg);
    send_message(fd, &msg)?;
    let replies = recv_messages(fd, *seq)?;

    let mut stringsets = HashMap::new();
    for reply in replies {
        if reply.len() < mem::size_of::<GenlMsgHdr>() {
            continue;
        }
        let attrs = parse_attrs(&reply[mem::size_of::<GenlMsgHdr>()..]);
        let mut matched = false;
        for (attr_type, payload) in attrs {
            if attr_type == ETHTOOL_A_STRSET_HEADER {
                if let Some(name) = extract_header_name(payload) {
                    matched = name == dev;
                }
                continue;
            }
            if attr_type != ETHTOOL_A_STRSET_STRINGSETS {
                continue;
            }
            if !matched {
                continue;
            }
            for (set_type, set_payload) in parse_attrs(payload) {
                if set_type != ETHTOOL_A_STRINGSETS_STRINGSET {
                    continue;
                }
                let mut set_id = None;
                let mut strings = Vec::new();
                for (set_attr, set_value) in parse_attrs(set_payload) {
                    if set_attr == ETHTOOL_A_STRINGSET_ID {
                        set_id = parse_u32(set_value);
                    } else if set_attr == ETHTOOL_A_STRINGSET_STRINGS {
                        for (strings_attr, strings_payload) in parse_attrs(set_value) {
                            if strings_attr != ETHTOOL_A_STRINGS_STRING {
                                continue;
                            }
                            let mut index = None;
                            let mut value = None;
                            for (str_attr, str_payload) in parse_attrs(strings_payload) {
                                if str_attr == ETHTOOL_A_STRING_INDEX {
                                    index = parse_u32(str_payload);
                                } else if str_attr == ETHTOOL_A_STRING_VALUE {
                                    value = parse_string(str_payload);
                                }
                            }
                            if let (Some(idx), Some(val)) = (index, value) {
                                if strings.len() <= idx as usize {
                                    strings.resize(idx as usize + 1, String::new());
                                }
                                strings[idx as usize] = val;
                            }
                        }
                    }
                }
                if let Some(id) = set_id {
                    stringsets.insert(id, strings);
                }
            }
        }
    }

    Ok(stringsets)
}

fn request_stats(fd: i32, family_id: u16, seq: &mut u32, dev: &str) -> io::Result<Vec<StatsGroup>> {
    *seq += 1;
    let mut msg = build_message(
        family_id,
        NLM_F_REQUEST | NLM_F_DUMP,
        *seq,
        ETHTOOL_MSG_STATS_GET,
        ETHTOOL_GENL_VERSION,
    );

    let header_start = start_nested(&mut msg, ETHTOOL_A_STATS_HEADER);
    end_nested(&mut msg, header_start);

    let groups_start = start_nested(&mut msg, ETHTOOL_A_STATS_GROUPS);
    add_attr(&mut msg, ETHTOOL_A_BITSET_NOMASK, &[]);
    let bits_start = start_nested(&mut msg, ETHTOOL_A_BITSET_BITS);
    for name in ["eth-phy", "eth-mac", "eth-ctrl", "rmon", "phy"] {
        let bit_start = start_nested(&mut msg, ETHTOOL_A_BITSET_BIT);
        add_attr_string(&mut msg, ETHTOOL_A_BITSET_BIT_NAME, name);
        add_attr(&mut msg, ETHTOOL_A_BITSET_BIT_VALUE, &[]);
        end_nested(&mut msg, bit_start);
    }
    end_nested(&mut msg, bits_start);
    end_nested(&mut msg, groups_start);

    finalize_message(&mut msg);
    send_message(fd, &msg)?;
    let replies = recv_messages(fd, *seq)?;

    let mut groups = Vec::new();
    for reply in replies {
        if reply.len() < mem::size_of::<GenlMsgHdr>() {
            continue;
        }
        let attrs = parse_attrs(&reply[mem::size_of::<GenlMsgHdr>()..]);
        let mut matched = false;
        for (attr_type, payload) in attrs {
            if attr_type == ETHTOOL_A_STATS_HEADER {
                if let Some(name) = extract_header_name(payload) {
                    matched = name == dev;
                }
                continue;
            }
            if attr_type != ETHTOOL_A_STATS_GRP {
                continue;
            }
            if !matched {
                continue;
            }
            let mut grp_id = None;
            let mut ss_id = None;
            let mut stats = Vec::new();
            if debug_enabled() {
                let attr_types: Vec<String> = parse_attrs(payload)
                    .iter()
                    .map(|(t, v)| format!("{t}:{len}", len = v.len()))
                    .collect();
                eprintln!("ethtool: grp attrs {dev}: {}", attr_types.join(", "));
            }
            for (grp_attr, grp_payload) in parse_attrs(payload) {
                if grp_attr == ETHTOOL_A_STATS_GRP_ID {
                    grp_id = parse_u32(grp_payload);
                } else if grp_attr == ETHTOOL_A_STATS_GRP_SS_ID {
                    ss_id = parse_u32(grp_payload);
                } else if grp_attr == ETHTOOL_A_STATS_GRP_STAT {
                    if debug_enabled() {
                        let inner: Vec<String> = parse_attrs(grp_payload)
                            .iter()
                            .map(|(t, v)| format!("{t}:{len}", len = v.len()))
                            .collect();
                        eprintln!("ethtool: grp stat inner {dev}: {}", inner.join(", "));
                    }
                    for (stat_attr, stat_payload) in parse_attrs(grp_payload) {
                        if let Some(value) = parse_u64(stat_payload) {
                            stats.push((stat_attr as u32, value));
                        }
                    }
                }
            }
            if let (Some(group_id), Some(stringset_id)) = (grp_id, ss_id) {
                groups.push((group_id, stringset_id, stats));
            }
        }
    }

    Ok(groups)
}

fn stringset_name(stringsets: &HashMap<u32, Vec<String>>, ss_id: u32, stat_id: u32) -> String {
    if let Some(strings) = stringsets.get(&ss_id)
        && let Some(name) = strings.get(stat_id as usize)
        && !name.is_empty()
    {
        return name.clone();
    }
    format!("stat_{}", stat_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attr_bytes(attr_type: u16, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        add_attr(&mut buf, attr_type, payload);
        buf
    }

    #[test]
    fn parse_attrs_reads_a_well_formed_attribute() {
        let buf = attr_bytes(7, &[1, 2, 3, 4]);
        let attrs = parse_attrs(&buf);
        assert_eq!(attrs.len(), 1);
        assert_eq!(attrs[0].0, 7);
        assert_eq!(attrs[0].1, &[1, 2, 3, 4]);
    }

    #[test]
    fn parse_attrs_handles_unpadded_trailing_attribute() {
        // A 3-byte payload makes nla_len 7, whose aligned length is 8 - past
        // the end of the buffer. This used to panic on the slice.
        let mut buf = attr_bytes(7, &[1, 2, 3]);
        buf.truncate(7);
        let attrs = parse_attrs(&buf);
        assert_eq!(attrs.len(), 1);
        assert_eq!(attrs[0].1, &[1, 2, 3]);
    }

    #[test]
    fn parse_attrs_stops_on_a_truncated_header() {
        assert!(parse_attrs(&[0, 0, 0]).is_empty());
        // nla_len smaller than the header itself must not loop.
        assert!(parse_attrs(&[1, 0, 0, 0, 0, 0, 0, 0]).is_empty());
    }
}

fn list_ethernet_interfaces() -> Vec<String> {
    let mut ifaces = Vec::new();
    let base = Path::new("/sys/class/net");
    let entries = match fs::read_dir(base) {
        Ok(entries) => entries,
        Err(_) => return ifaces,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = match name.to_str() {
            Some(name) => name.to_string(),
            None => continue,
        };
        let iface_path = entry.path();
        if is_ethernet_interface(&iface_path) {
            ifaces.push(name);
        }
    }
    ifaces
}

fn is_ethernet_interface(iface_path: &Path) -> bool {
    let type_path = iface_path.join("type");
    let Ok(contents) = fs::read_to_string(type_path) else {
        return false;
    };
    let Ok(value) = contents.trim().parse::<u32>() else {
        return false;
    };
    if value != 1 {
        return false;
    }
    iface_path.join("device").exists()
}

pub fn update_metrics() {
    let fd = match create_netlink_socket() {
        Ok(fd) => fd,
        Err(_) => return,
    };

    struct SocketGuard(i32);
    impl Drop for SocketGuard {
        fn drop(&mut self) {
            unsafe { libc::close(self.0) };
        }
    }
    let _guard = SocketGuard(fd);

    let mut seq = 0;
    let family_id = match get_ethtool_family_id(fd, &mut seq) {
        Ok(id) => id,
        Err(err) => {
            if debug_enabled() {
                eprintln!("ethtool: failed to resolve family id: {err}");
            }
            return;
        }
    };

    let ifaces = list_ethernet_interfaces();
    if debug_enabled() {
        eprintln!("ethtool: ethernet interfaces {}", ifaces.len());
    }
    for iface in ifaces {
        let stringsets = match request_stringsets(fd, family_id, &mut seq, &iface) {
            Ok(stringsets) => stringsets,
            Err(err) => {
                if debug_enabled() {
                    eprintln!("ethtool: stringset request failed for {iface}: {err}");
                }
                continue;
            }
        };
        if debug_enabled() {
            let summary: Vec<String> = stringsets
                .iter()
                .map(|(id, strings)| format!("{id}:{len}", len = strings.len()))
                .collect();
            eprintln!("ethtool: stringsets for {iface}: {}", summary.join(", "));
        }
        let groups = match request_stats(fd, family_id, &mut seq, &iface) {
            Ok(groups) => groups,
            Err(err) => {
                if debug_enabled() {
                    eprintln!("ethtool: stats request failed for {iface}: {err}");
                }
                continue;
            }
        };
        if debug_enabled() {
            eprintln!("ethtool: stats groups for {iface}: {}", groups.len());
        }
        let metric = &metrics().ethtool_stats;
        let mut emitted = 0usize;
        for (_grp_id, ss_id, stats) in groups {
            for (stat_id, value) in stats {
                let name = stringset_name(&stringsets, ss_id, stat_id);
                metric
                    .with_label_values(&[iface.as_str(), name.as_str()])
                    .set(value as f64);
                emitted += 1;
            }
        }
        if debug_enabled() {
            eprintln!("ethtool: emitted {emitted} metrics for {iface}");
        }
    }
}
