//! End-to-end tests that drive the real binary over HTTP and HTTPS.
//!
//! The crate has no library target, so collector internals are covered by unit
//! tests inside `src/`. What is left to check from the outside is the behaviour
//! an operator actually depends on: who is allowed to scrape, and that a scrape
//! produces a well-formed exposition.

use std::fmt::Write as _;
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
    dir: TempDir,
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
    let server = spawn_server(dir, port);
    wait_until_listening(server.port);
    server
}

fn spawn_server(dir: TempDir, port: u16) -> Server {
    let child = Command::new(EXE)
        .current_dir(dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn exporter");

    Server { child, port, dir }
}

fn wait_until_listening(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
            if stream
                .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                .is_ok()
            {
                let mut response = [0_u8; 64];
                if let Ok(read) = stream.read(&mut response)
                    && response[..read].starts_with(b"HTTP/1.1 200")
                {
                    return;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("exporter did not become HTTP-ready on port {port}");
}

/// Minimal HTTP/1.1 GET. Returns the status code and the body.
fn get(port: u16, path: &str, headers: &[(&str, &str)]) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .expect("set read timeout");

    let mut request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    for (name, value) in headers {
        write!(request, "{name}: {value}\r\n").expect("writing to String cannot fail");
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
        .map_or_default(|(_, body)| body.to_string());

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
    assert!(body.contains("node_scrape_collector_duration_seconds"));
    assert!(body.contains("node_scrape_collector_success"));
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
    // The ACL uses the actual transport peer, not attacker-controlled forwarding headers.
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
fn unknown_path_is_not_found() {
    let server = start_server("allowed_ip = [\"127.0.0.0/8\"]\n");
    let (status, _) = get(server.port, "/does-not-exist", &[]);
    assert_eq!(status, 404);
}

fn write_tls_identity(dir: &Path) {
    use btls::{
        asn1::Asn1Time,
        bn::BigNum,
        hash::MessageDigest,
        pkey::PKey,
        rsa::Rsa,
        x509::{X509, X509Name, extension::SubjectAlternativeName},
    };

    let key = PKey::from_rsa(Rsa::generate(2048).expect("generate RSA key"))
        .expect("construct private key");
    let mut name = X509Name::builder().expect("certificate name");
    name.append_entry_by_text("CN", "localhost").expect("CN");
    let name = name.build().expect("build certificate name");
    let mut cert = X509::builder().expect("certificate builder");
    cert.set_version(2).expect("version");
    let serial = BigNum::from_u32(1)
        .expect("serial")
        .to_asn1_integer()
        .expect("ASN.1 serial");
    cert.set_serial_number(&serial).expect("serial");
    cert.set_subject_name(&name).expect("subject");
    cert.set_issuer_name(&name).expect("issuer");
    cert.set_pubkey(&key).expect("public key");
    let start = Asn1Time::days_from_now(0).expect("validity start");
    let end = Asn1Time::days_from_now(1).expect("validity end");
    cert.set_not_before(&start).expect("not before");
    cert.set_not_after(&end).expect("not after");
    let san = SubjectAlternativeName::new()
        .dns("localhost")
        .build(&cert.x509v3_context(None, None))
        .expect("subject alternative name");
    cert.append_extension(&san).expect("append SAN");
    cert.sign(&key, MessageDigest::sha256()).expect("sign");
    let cert = cert.build();
    std::fs::write(
        dir.join("cert.pem"),
        cert.to_pem().expect("certificate PEM"),
    )
    .expect("write certificate");
    std::fs::write(
        dir.join("key.pem"),
        key.private_key_to_pem_pkcs8().expect("private key PEM"),
    )
    .expect("write key");
}

async fn connect_tls(server: &Server, alpn: &[u8]) -> tokio_btls::SslStream<tokio::net::TcpStream> {
    use btls::ssl::{SslConnector, SslMethod};
    use std::pin::Pin;

    let mut builder = SslConnector::builder(SslMethod::tls()).expect("TLS connector");
    builder
        .set_ca_file(server.dir.path().join("cert.pem"))
        .expect("trust test certificate");
    builder.set_alpn_protos(alpn).expect("ALPN");
    let ssl = builder
        .build()
        .configure()
        .expect("TLS configuration")
        .into_ssl("localhost")
        .expect("verified TLS session");
    let deadline = Instant::now() + Duration::from_secs(30);
    let tcp = loop {
        match tokio::net::TcpStream::connect(("127.0.0.1", server.port)).await {
            Ok(stream) => break stream,
            Err(error) => {
                assert!(Instant::now() < deadline, "HTTPS server not ready: {error}");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    };
    let mut tls = tokio_btls::SslStream::new(ssl, tcp).expect("TLS stream");
    Pin::new(&mut tls).connect().await.expect("TLS handshake");
    tls
}

#[tokio::test]
async fn https_negotiates_http1_and_http2_and_enforces_auth() {
    use bytes::Bytes;
    use http_body_util::{BodyExt as _, Empty};
    use hyper::{Request, StatusCode, Version, client::conn};
    use hyper_util::rt::{TokioExecutor, TokioIo};

    tokio::time::timeout(Duration::from_secs(60), async {
        let dir = TempDir::new().expect("tempdir");
        write_tls_identity(dir.path());
        let port = free_port();
        write_config(
            dir.path(),
            port,
            "tls_cert = \"cert.pem\"\ntls_key = \"key.pem\"\nauth_token = \"tls-test-token\"\n",
        );
        let server = spawn_server(dir, port);
        let tls = connect_tls(&server, b"\x08http/1.1").await;
        assert_eq!(
            tls.ssl().selected_alpn_protocol(),
            Some(b"http/1.1".as_slice())
        );
        let (mut h1, connection) = conn::http1::handshake(TokioIo::new(tls))
            .await
            .expect("HTTP/1 handshake");
        let h1_task = tokio::spawn(connection);
        let denied = h1
            .send_request(
                Request::builder()
                    .uri("/metrics")
                    .header("Host", "localhost")
                    .body(Empty::<Bytes>::new())
                    .expect("request"),
            )
            .await
            .expect("HTTP/1 response");
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(denied.version(), Version::HTTP_11);
        let _body = denied.into_body().collect().await.expect("response body");
        drop(h1);
        h1_task.abort();

        let tls = connect_tls(&server, b"\x08http/1.1\x02h2").await;
        assert_eq!(tls.ssl().selected_alpn_protocol(), Some(b"h2".as_slice()));
        let (mut h2, connection) = conn::http2::handshake(TokioExecutor::new(), TokioIo::new(tls))
            .await
            .expect("HTTP/2 handshake");
        let h2_task = tokio::spawn(connection);
        let response = h2
            .send_request(
                Request::builder()
                    .uri("https://localhost/metrics")
                    .header("Authorization", "Bearer tls-test-token")
                    .body(Empty::<Bytes>::new())
                    .expect("request"),
            )
            .await
            .expect("HTTP/2 response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.version(), Version::HTTP_2);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("response body")
            .to_bytes();
        assert!(
            std::str::from_utf8(&body)
                .expect("UTF-8 metrics")
                .contains("uptime_seconds")
        );
        drop(h2);
        h2_task.abort();
    })
    .await
    .expect("HTTPS regression exceeded deadline");
}

#[tokio::test]
async fn push_delivers_timestamped_gzip_without_a_scrape() {
    use bytes::Bytes;
    use http_body_util::{BodyExt as _, Empty};
    use hyper::{Method, Response, StatusCode, server::conn::http1, service::service_fn};
    use hyper_util::rt::TokioIo;

    let receiver = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind push receiver");
    let receiver_port = receiver.local_addr().expect("receiver address").port();
    let (send, mut received) = tokio::sync::mpsc::channel(1);
    let receiver_task = tokio::spawn(async move {
        let (connection, _) = receiver.accept().await.expect("accept push");
        let service = service_fn(|request: hyper::Request<hyper::body::Incoming>| {
            let send = send.clone();
            async move {
                let (parts, body) = request.into_parts();
                let body = body.collect().await?.to_bytes();
                let _ = send.try_send((parts, body));
                let mut response = Response::new(Empty::<Bytes>::new());
                *response.status_mut() = StatusCode::NO_CONTENT;
                Ok::<_, hyper::Error>(response)
            }
        });
        http1::Builder::new()
            .serve_connection(TokioIo::new(connection), service)
            .await
    });
    let dir = TempDir::new().expect("tempdir");
    let port = free_port();
    let spool = dir.path().join("spool");
    write_config(
        dir.path(),
        port,
        &format!(
            "push_url = \"http://127.0.0.1:{receiver_port}/api/v1/import/prometheus\"\n\
             push_interval_ms = 1000\n\
             push_timeout_ms = 10000\n\
             push_spill_after_seconds = 10\n\
             push_spool_dir = {spool:?}\n"
        ),
    );
    let _server = spawn_server(dir, port);
    // No HTTP scrape is made: the push sampler must collect independently.
    let (parts, compressed) = tokio::time::timeout(Duration::from_secs(30), received.recv())
        .await
        .expect("push delivery deadline")
        .expect("receiver closed");
    assert_eq!(parts.method, Method::POST);
    assert_eq!(parts.uri.path(), "/api/v1/import/prometheus");
    assert_eq!(parts.headers["content-encoding"], "gzip");
    let mut decoder = flate2::read::GzDecoder::new(compressed.as_ref());
    let mut text = String::new();
    let _read = decoder.read_to_string(&mut text).expect("decode gzip");
    let uptime = text
        .lines()
        .find(|line| line.starts_with("uptime_seconds "))
        .expect("uptime sample");
    let fields: Vec<_> = uptime.split_ascii_whitespace().collect();
    assert_eq!(fields.len(), 3);
    assert!(fields[1].parse::<f64>().expect("uptime") >= 0.0);
    assert!(fields[2].parse::<u64>().expect("timestamp") > 0);
    assert!(
        !spool.exists()
            || std::fs::read_dir(&spool)
                .expect("read spool directory")
                .next()
                .is_none(),
        "successful delivery must not write spool files"
    );
    receiver_task.abort();
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
