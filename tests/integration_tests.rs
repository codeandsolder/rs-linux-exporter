//! End-to-end tests that drive the real binary over HTTP.
//!
//! The crate has no library target, so collector internals are covered by unit
//! tests inside `src/`. What is left to check from the outside is the behaviour
//! an operator actually depends on: who is allowed to scrape, and that a scrape
//! produces a well-formed exposition.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const EXE: &str = env!("CARGO_BIN_EXE_rs-linux-exporter");

/// Kills the exporter when the test ends, however it ends.
struct Server {
    child: Child,
    port: u16,
    _dir: TempDir,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Asks the OS for a port that is currently free.
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().expect("local addr").port()
}

fn write_config(dir: &Path, port: u16, extra: &str) {
    // conntrack and ipmi need privileges and hardware; leaving them on only
    // makes the tests slower and noisier.
    let config = format!(
        "bind = \"127.0.0.1:{port}\"\n\
         disabled_datasources = [\"ipmi\", \"conntrack\"]\n\
         {extra}"
    );
    std::fs::write(dir.join("config.toml"), config).expect("write config");
}

fn start_server(extra_config: &str) -> Server {
    let dir = TempDir::new().expect("tempdir");
    let port = free_port();
    write_config(dir.path(), port, extra_config);

    let child = Command::new(EXE)
        .current_dir(dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn exporter");

    let server = Server {
        child,
        port,
        _dir: dir,
    };
    wait_until_listening(server.port);
    server
}

fn wait_until_listening(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("exporter did not start listening on port {port}");
}

/// Minimal HTTP/1.1 GET. Returns the status code and the body.
fn get(port: u16, path: &str, headers: &[(&str, &str)]) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .expect("set read timeout");

    let mut request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");

    stream.write_all(request.as_bytes()).expect("write request");
    stream.flush().expect("flush");

    let mut raw = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => raw.extend_from_slice(&chunk[..read]),
            // A short HTTP response may be followed by an RST when the server
            // closes before the client has consumed the FIN. Treat that like
            // EOF; the status/body assertions below still reject truncation.
            Err(err) if err.kind() == ErrorKind::ConnectionReset => break,
            Err(err) => panic!("read response: {err}"),
        }
    }
    let response = String::from_utf8_lossy(&raw).into_owned();

    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("no status line in response: {response:?}"));

    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();

    (status, body)
}

#[test]
fn allowlisted_client_gets_prometheus_exposition() {
    let server = start_server("allowed_ip = [\"127.0.0.0/8\"]\n");
    let (status, body) = get(server.port, "/metrics", &[]);

    assert_eq!(status, 200);
    assert!(body.contains("# HELP"), "missing HELP lines");
    assert!(body.contains("# TYPE"), "missing TYPE lines");
    assert!(body.contains("metrics_requests_total"));
    assert!(body.contains("uptime_seconds"));
}

#[test]
fn client_outside_the_allowlist_is_denied() {
    // Loopback is the only address the test can connect from, so allowlist
    // something else and check the request is refused.
    let server = start_server("allowed_ip = [\"10.0.0.0/8\"]\n");
    let (status, body) = get(server.port, "/metrics", &[]);

    assert_eq!(status, 403);
    assert!(body.contains("access denied"), "body was {body:?}");
}

#[test]
fn x_real_ip_header_cannot_forge_an_allowlisted_address() {
    // Rocket trusts X-Real-IP by default, which would let any client claim to
    // be an allowlisted address. The exporter disables that.
    let server = start_server("allowed_ip = [\"10.0.0.0/8\"]\n");

    for spoofed in ["10.0.0.1", "10.1.2.3"] {
        let (status, _) = get(server.port, "/metrics", &[("X-Real-IP", spoofed)]);
        assert_eq!(status, 403, "X-Real-IP: {spoofed} bypassed the allowlist");

        let (status, _) = get(server.port, "/metrics", &[("X-Forwarded-For", spoofed)]);
        assert_eq!(
            status, 403,
            "X-Forwarded-For: {spoofed} bypassed the allowlist"
        );
    }
}

#[test]
fn auth_token_is_required_when_configured() {
    let server = start_server(
        "allowed_ip = [\"127.0.0.0/8\"]\n\
         auth_token = \"integration-test-token\"\n",
    );

    let (status, _) = get(server.port, "/metrics", &[]);
    assert_eq!(status, 401, "missing token was accepted");

    let (status, _) = get(
        server.port,
        "/metrics",
        &[("Authorization", "Bearer wrong-token")],
    );
    assert_eq!(status, 401, "wrong token was accepted");

    // Differs from the real token only in the last byte.
    let (status, _) = get(
        server.port,
        "/metrics",
        &[("Authorization", "Bearer integration-test-toke")],
    );
    assert_eq!(status, 401, "truncated token was accepted");

    let (status, body) = get(
        server.port,
        "/metrics",
        &[("Authorization", "Bearer integration-test-token")],
    );
    assert_eq!(status, 200);
    assert!(body.contains("# HELP"));
}

#[test]
fn token_is_checked_before_the_address_allowlist() {
    let server = start_server(
        "allowed_ip = [\"10.0.0.0/8\"]\n\
         auth_token = \"integration-test-token\"\n",
    );

    // Denied on both counts; the token check runs first.
    let (status, _) = get(server.port, "/metrics", &[]);
    assert_eq!(status, 401);
}

#[test]
fn json_endpoint_returns_the_same_metrics() {
    let server = start_server("allowed_ip = [\"127.0.0.0/8\"]\n");
    let (status, body) = get(server.port, "/metrics.json", &[]);

    assert_eq!(status, 200);
    assert!(
        body.starts_with('['),
        "body was {:?}",
        &body[..40.min(body.len())]
    );
    assert!(body.contains("\"_name_\""));
    assert!(body.contains("\"_value_\""));
    assert!(body.contains("uptime_seconds"));
}

#[test]
fn repeated_scrapes_do_not_accumulate_series() {
    // Collectors reset their vecs before repopulating, so a stable host should
    // report a stable number of series rather than growing on every scrape.
    let server = start_server("allowed_ip = [\"127.0.0.0/8\"]\n");

    let count_series = |body: &str| body.lines().filter(|l| !l.starts_with('#')).count();

    let (_, first) = get(server.port, "/metrics", &[]);
    let (_, second) = get(server.port, "/metrics", &[]);
    let (_, third) = get(server.port, "/metrics", &[]);

    let counts = [
        count_series(&first),
        count_series(&second),
        count_series(&third),
    ];
    assert!(counts[0] > 0, "no series exposed");
    assert_eq!(
        counts[1], counts[2],
        "series count changed between scrapes: {counts:?}"
    );
}

#[test]
fn unknown_path_is_not_found() {
    let server = start_server("allowed_ip = [\"127.0.0.0/8\"]\n");
    let (status, _) = get(server.port, "/does-not-exist", &[]);
    assert_eq!(status, 404);
}

#[test]
fn unparseable_config_is_fatal() {
    let dir = TempDir::new().expect("tempdir");
    std::fs::write(
        dir.path().join("config.toml"),
        "allowed_ip = [ unterminated\n",
    )
    .expect("write config");

    let output = Command::new(EXE)
        .current_dir(dir.path())
        .output()
        .expect("run exporter");

    assert!(
        !output.status.success(),
        "exporter started despite an unusable config"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Refusing to start"),
        "stderr was {stderr:?}"
    );
}
