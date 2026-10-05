use serde::Deserialize;
use std::net::{IpAddr, ToSocketAddrs};
use std::process::Command;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub struct PingResult {
    pub success: bool,
    pub rtt_seconds: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TraceHop {
    pub hop: u32,
    pub address: Option<String>,
    pub rtt_seconds: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TraceResult {
    pub success: bool,
    pub reached: bool,
    pub hops: Vec<TraceHop>,
}

#[derive(Debug, Deserialize)]
struct MtrDocument {
    report: MtrReport,
}

#[derive(Debug, Deserialize)]
struct MtrReport {
    mtr: MtrMetadata,
    hubs: Vec<MtrHub>,
}

#[derive(Debug, Deserialize)]
struct MtrMetadata {
    dst: String,
}

#[derive(Debug, Deserialize)]
struct MtrHub {
    count: u32,
    host: String,
    #[serde(rename = "Loss%")]
    loss_percent: f64,
    #[serde(rename = "Last")]
    last_ms: f64,
}

fn parse_ping_rtt(output: &str) -> Option<f64> {
    output.lines().find_map(|line| {
        let marker = " time=";
        let start = line.find(marker)? + marker.len();
        let tail = &line[start..];
        let end = tail.find(" ms")?;
        tail[..end].parse::<f64>().ok().map(|ms| ms / 1_000.0)
    })
}

pub fn run_ping(binary: &str, address: &str, timeout: Duration) -> Result<PingResult, String> {
    let timeout_secs = timeout.as_secs_f64().ceil().max(1.0);
    let mut command = Command::new(binary);
    command
        .env("LC_ALL", "C")
        .args(["-n", "-c", "1", "-W", &timeout_secs.to_string(), address]);
    match crate::subprocess::run_bounded(
        command,
        timeout + Duration::from_secs(1),
        64 * 1024,
        "ping",
    ) {
        Ok(output) => Ok(PingResult {
            success: true,
            rtt_seconds: parse_ping_rtt(&output),
        }),
        Err(error) if error.contains("exited with") => Ok(PingResult {
            success: false,
            rtt_seconds: None,
        }),
        Err(error) => Err(error),
    }
}

fn parse_mtr(output: &str) -> Result<TraceResult, String> {
    let document: MtrDocument =
        serde_json::from_str(output).map_err(|error| format!("parse mtr JSON: {error}"))?;
    let destination = document.report.mtr.dst;
    let mut reached = false;
    let hops = document
        .report
        .hubs
        .into_iter()
        .map(|hub| {
            let responsive = hub.loss_percent < 100.0 && hub.host != "???";
            if responsive && hub.host == destination {
                reached = true;
            }
            TraceHop {
                hop: hub.count,
                address: responsive.then_some(hub.host),
                rtt_seconds: responsive.then_some(hub.last_ms / 1_000.0),
            }
        })
        .collect();
    Ok(TraceResult {
        success: true,
        reached,
        hops,
    })
}

fn traceroute_target(address: &str) -> Result<String, String> {
    if let Ok(ip) = address.parse::<IpAddr>() {
        return Ok(ip.to_string());
    }
    (address, 0_u16)
        .to_socket_addrs()
        .map_err(|error| format!("resolve traceroute target {address:?}: {error}"))?
        .next()
        .map(|socket| socket.ip().to_string())
        .ok_or_else(|| format!("traceroute target {address:?} resolved to no addresses"))
}

pub fn run_traceroute(
    binary: &str,
    address: &str,
    timeout: Duration,
    max_hops: u32,
) -> Result<TraceResult, String> {
    let target = traceroute_target(address)?;
    let mut command = Command::new(binary);
    command.env("LC_ALL", "C").args([
        "--json",
        "-n",
        "-c",
        "1",
        "-m",
        &max_hops.to_string(),
        &target,
    ]);
    let output = crate::subprocess::run_bounded(command, timeout, 512 * 1024, "mtr traceroute")?;
    parse_mtr(&output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ping_rtt() {
        let output = "64 bytes from 1.1.1.1: icmp_seq=1 ttl=55 time=12.345 ms\n";
        assert_eq!(parse_ping_rtt(output), Some(0.012_345));
    }

    #[test]
    fn traceroute_target_preserves_numeric_addresses() {
        assert_eq!(traceroute_target("1.1.1.1"), Ok("1.1.1.1".to_string()));
        assert_eq!(
            traceroute_target("2001:db8::1"),
            Ok("2001:db8::1".to_string())
        );
    }

    #[test]
    fn parses_mtr_hops_without_using_address_in_rtt_identity() {
        let output = r#"{
          "report": {
            "mtr": {"dst":"1.1.1.1"},
            "hubs": [
              {"count":1,"host":"10.0.0.1","Loss%":0.0,"Last":1.5},
              {"count":2,"host":"???","Loss%":100.0,"Last":0.0},
              {"count":3,"host":"1.1.1.1","Loss%":0.0,"Last":20.25}
            ]
          }
        }"#;
        let trace = parse_mtr(output).unwrap();
        assert!(trace.reached);
        assert_eq!(trace.hops[0].rtt_seconds, Some(0.001_5));
        assert_eq!(trace.hops[1].address, None);
        assert_eq!(trace.hops[2].address.as_deref(), Some("1.1.1.1"));
    }
}
