use crate::metric_support::RegisterMetricResultExt;
#[macro_use]
extern crate rocket;

mod config;
mod datasource_conntrack;
mod datasource_cpufreq;
mod datasource_edac;
mod datasource_filesystems;
mod datasource_hwmon;
mod datasource_ipmi;
mod datasource_mdraid;
mod datasource_netdev_sysfs;
mod datasource_numa;
mod datasource_nvme;
mod datasource_power_supply;
mod datasource_procfs;
mod datasource_rapl;
mod datasource_softnet;
mod datasource_thermal;
mod metric_support;
mod runtime;
mod sysfs;

use crate::config::{AppConfig, Datasource};
use prometheus::{Encoder, IntCounter, TextEncoder};
use rocket::Config;
use rocket::config::TlsConfig;
use rocket::http::{ContentType, Status};
use rocket::request::{FromRequest, Outcome, Request};
use rocket::response::status;
use rocket::tokio::task::spawn_blocking;
use serde_json::Value as JsonValue;
use std::net::IpAddr;
use std::sync::Mutex;

/// Extracts Bearer token from Authorization header
pub struct BearerToken(Option<String>);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for BearerToken {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let token = request
            .headers()
            .get_one("Authorization")
            .and_then(|header| header.strip_prefix("Bearer "))
            .map(std::string::ToString::to_string);
        Outcome::Success(Self(token))
    }
}
use std::sync::OnceLock;

static METRICS_REQUESTS_TOTAL: OnceLock<IntCounter> = OnceLock::new();
static METRICS_REQUESTS_DENIED_TOTAL: OnceLock<IntCounter> = OnceLock::new();
static APP_CONFIG: OnceLock<AppConfig> = OnceLock::new();

/// Serialises scrapes. Collectors reset their vecs and repopulate them, so two
/// concurrent scrapes would let one observe the other's half-rebuilt state. It
/// also stops N simultaneous requests from doing N times the hardware polling.
static SCRAPE_LOCK: Mutex<()> = Mutex::new(());

fn metrics_requests_total() -> &'static IntCounter {
    METRICS_REQUESTS_TOTAL.get_or_init(|| {
        prometheus::register_int_counter!(
            "metrics_requests_total",
            "Total number of /metrics requests"
        )
        .or_exit("metrics_requests_total")
    })
}

fn metrics_requests_denied_total() -> &'static IntCounter {
    METRICS_REQUESTS_DENIED_TOTAL.get_or_init(|| {
        prometheus::register_int_counter!(
            "metrics_requests_denied_total",
            "Total number of /metrics requests denied by ACL"
        )
        .or_exit("metrics_requests_denied_total")
    })
}

fn app_config() -> &'static AppConfig {
    APP_CONFIG.get_or_init(AppConfig::load)
}

fn update_metrics() {
    let config = app_config();

    if config.is_datasource_enabled(Datasource::Procfs) {
        datasource_procfs::update_metrics(config);
    }
    if config.is_datasource_enabled(Datasource::CpuFreq) {
        datasource_cpufreq::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::Softnet) {
        datasource_softnet::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::Conntrack) {
        datasource_conntrack::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::Filesystems) {
        datasource_filesystems::update_metrics(config);
    }
    if config.is_datasource_enabled(Datasource::Hwmon) {
        datasource_hwmon::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::Ipmi) {
        datasource_ipmi::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::Mdraid) {
        datasource_mdraid::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::Thermal) {
        datasource_thermal::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::Rapl) {
        datasource_rapl::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::PowerSupply) {
        datasource_power_supply::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::Nvme) {
        datasource_nvme::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::Edac) {
        datasource_edac::update_metrics();
    }
    if config.is_datasource_enabled(Datasource::NetdevSysfs) {
        datasource_netdev_sysfs::update_metrics(config);
    }
    if config.is_datasource_enabled(Datasource::Numa) {
        datasource_numa::update_metrics();
    }
    // TODO: Implementation in progress; ethtool netlink stats disabled for now.
}

fn push_json_sample(
    samples: &mut Vec<serde_json::Map<String, JsonValue>>,
    name: &str,
    labels: &[(String, String)],
    value: JsonValue,
) {
    let mut map = serde_json::Map::new();
    map.insert("_name_".to_string(), JsonValue::from(name));
    for (key, value) in labels {
        map.insert(key.clone(), JsonValue::from(value.clone()));
    }
    map.insert("_value_".to_string(), value);
    samples.push(map);
}

fn metrics_json_payload() -> String {
    let families = prometheus::gather();
    let mut samples: Vec<serde_json::Map<String, JsonValue>> = Vec::new();

    for family in families {
        let name = family.name();
        let metric_type = family.get_field_type();
        for metric in family.get_metric() {
            let base_labels: Vec<(String, String)> = metric
                .get_label()
                .iter()
                .map(|label| (label.name().to_string(), label.value().to_string()))
                .collect();

            match metric_type {
                prometheus::proto::MetricType::COUNTER => {
                    let value = JsonValue::from(metric.get_counter().value());
                    push_json_sample(&mut samples, name, &base_labels, value);
                }
                prometheus::proto::MetricType::GAUGE => {
                    let value = JsonValue::from(metric.get_gauge().value());
                    push_json_sample(&mut samples, name, &base_labels, value);
                }
                prometheus::proto::MetricType::UNTYPED => {
                    // UNTYPED metrics are not directly supported, skip
                }
                prometheus::proto::MetricType::HISTOGRAM => {
                    let histogram = metric.get_histogram();
                    for bucket in histogram.get_bucket() {
                        let mut labels = base_labels.clone();
                        labels.push(("le".to_string(), bucket.upper_bound().to_string()));
                        let value = JsonValue::from(bucket.cumulative_count());
                        let bucket_name = format!("{name}_bucket");
                        push_json_sample(&mut samples, &bucket_name, &labels, value);
                    }
                    let sum_name = format!("{name}_sum");
                    let count_name = format!("{name}_count");
                    push_json_sample(
                        &mut samples,
                        &sum_name,
                        &base_labels,
                        JsonValue::from(histogram.sample_sum()),
                    );
                    push_json_sample(
                        &mut samples,
                        &count_name,
                        &base_labels,
                        JsonValue::from(histogram.sample_count()),
                    );
                }
                prometheus::proto::MetricType::SUMMARY => {
                    let summary = metric.get_summary();
                    for quantile in summary.get_quantile() {
                        let mut labels = base_labels.clone();
                        labels.push(("quantile".to_string(), quantile.quantile().to_string()));
                        let value = JsonValue::from(quantile.value());
                        let quantile_name = format!("{name}_quantile");
                        push_json_sample(&mut samples, &quantile_name, &labels, value);
                    }
                    let sum_name = format!("{name}_sum");
                    let count_name = format!("{name}_count");
                    push_json_sample(
                        &mut samples,
                        &sum_name,
                        &base_labels,
                        JsonValue::from(summary.sample_sum()),
                    );
                    push_json_sample(
                        &mut samples,
                        &count_name,
                        &base_labels,
                        JsonValue::from(summary.sample_count()),
                    );
                }
            }
        }
    }

    serde_json::to_string(&samples).unwrap_or_else(|_| "[]".to_string())
}

/// Refreshes every enabled datasource and renders the result.
///
/// Runs on the blocking pool: collectors do synchronous filesystem and netlink
/// I/O, and `statvfs` on an unresponsive network mount or an IPMI controller
/// that is slow to answer can block for seconds. Doing that directly in the
/// handler would tie up a Rocket worker thread for the duration.
fn refresh_and_render<T>(render: impl FnOnce() -> T) -> T {
    let _guard = SCRAPE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    update_metrics();
    render()
}

fn render_text() -> String {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    if let Err(err) = encoder.encode(&metric_families, &mut buffer) {
        eprintln!("Failed to encode metrics: {err}");
        return String::new();
    }
    String::from_utf8(buffer).unwrap_or_default()
}

const fn collection_failed() -> status::Custom<&'static str> {
    status::Custom(Status::InternalServerError, "collection failed")
}

#[get("/metrics")]
async fn metrics(
    client_ip: Option<IpAddr>,
    token: BearerToken,
) -> Result<(ContentType, String), status::Custom<&'static str>> {
    metrics_requests_total().inc();
    let config = app_config();

    // Check token authentication first
    if !config.is_token_valid(token.0.as_deref()) {
        if config.log_denied_requests {
            eprintln!(
                "Denied /metrics request from {} (invalid token)",
                client_ip.map_or_else(|| "<unknown>".to_string(), |ip| ip.to_string())
            );
        }
        metrics_requests_denied_total().inc();
        return Err(status::Custom(Status::Unauthorized, "unauthorized"));
    }

    // Check IP allowlist
    let is_allowed = client_ip.is_some_and(|ip| config.is_metrics_ip_allowed(ip));
    if !is_allowed {
        if config.log_denied_requests {
            eprintln!(
                "Denied /metrics request from {}",
                client_ip.map_or_else(|| "<unknown>".to_string(), |ip| ip.to_string())
            );
        }
        metrics_requests_denied_total().inc();
        return Err(status::Custom(Status::Forbidden, "access denied"));
    }

    match spawn_blocking(|| refresh_and_render(render_text)).await {
        Ok(body) => Ok((ContentType::Plain, body)),
        Err(err) => {
            eprintln!("Metrics collection task failed: {err}");
            Err(collection_failed())
        }
    }
}

#[get("/metrics.json")]
async fn metrics_json(
    client_ip: Option<IpAddr>,
    token: BearerToken,
) -> Result<(ContentType, String), status::Custom<&'static str>> {
    metrics_requests_total().inc();
    let config = app_config();

    // Check token authentication first
    if !config.is_token_valid(token.0.as_deref()) {
        if config.log_denied_requests {
            eprintln!(
                "Denied /metrics.json request from {} (invalid token)",
                client_ip.map_or_else(|| "<unknown>".to_string(), |ip| ip.to_string())
            );
        }
        metrics_requests_denied_total().inc();
        return Err(status::Custom(Status::Unauthorized, "unauthorized"));
    }

    // Check IP allowlist
    let is_allowed = client_ip.is_some_and(|ip| config.is_metrics_ip_allowed(ip));
    if !is_allowed {
        if config.log_denied_requests {
            eprintln!(
                "Denied /metrics.json request from {}",
                client_ip.map_or_else(|| "<unknown>".to_string(), |ip| ip.to_string())
            );
        }
        metrics_requests_denied_total().inc();
        return Err(status::Custom(Status::Forbidden, "access denied"));
    }

    match spawn_blocking(|| refresh_and_render(metrics_json_payload)).await {
        Ok(body) => Ok((ContentType::JSON, body)),
        Err(err) => {
            eprintln!("Metrics collection task failed: {err}");
            Err(collection_failed())
        }
    }
}

#[get("/")]
const fn index() -> &'static str {
    "rs-linux-exporter: /metrics"
}

#[catch(404)]
fn not_found(request: &rocket::Request<'_>) -> &'static str {
    let config = app_config();
    if config.log_404_requests {
        let client_ip = request
            .client_ip()
            .map_or_else(|| "<unknown>".to_string(), |ip| ip.to_string());
        let method = request.method();
        let uri = request.uri();
        eprintln!("404 {method} {uri} from {client_ip}");
    }
    "Not Found"
}

#[launch]
fn rocket() -> _ {
    runtime::init();
    if runtime::debug_enabled() {
        eprintln!("Debug logging enabled.");
    }
    // Initialize config early to run subsystem availability checks and print messages
    let _ = app_config();
    let bind = match app_config().bind_addr() {
        Ok(bind) => bind,
        Err(err) => {
            eprintln!("Invalid runtime configuration: {err}");
            std::process::exit(78);
        }
    };
    let mut figment = Config::figment()
        .merge(("address", bind.ip().to_string()))
        .merge(("port", bind.port()))
        .merge(("ip_header", false));

    match app_config().tls_config() {
        Ok(Some((cert, key))) => {
            figment = figment.merge(("tls", TlsConfig::from_paths(cert, key)));
            eprintln!("TLS enabled with cert: {cert}");
        }
        Ok(None) => {}
        Err(err) => {
            eprintln!("Invalid runtime configuration: {err}");
            std::process::exit(78);
        }
    }

    rocket::custom(figment)
        .mount("/", routes![index, metrics, metrics_json])
        .register("/", catchers![not_found])
}

#[cfg(test)]
mod tests {
    use super::rocket;
    use rocket::http::Status;
    use rocket::local::blocking::Client;
    use std::net::SocketAddr;

    #[test]
    fn index_returns_hint() {
        let client = Client::tracked(rocket()).expect("valid rocket instance");
        let response = client.get("/").dispatch();

        assert_eq!(response.status(), Status::Ok);
        assert_eq!(
            response.into_string().unwrap_or_default(),
            "rs-linux-exporter: /metrics"
        );
    }

    #[test]
    fn metrics_endpoint_returns_ok() {
        let client = Client::tracked(rocket()).expect("valid rocket instance");
        let response = client
            .get("/metrics")
            .remote(metrics_remote_addr())
            .dispatch();

        assert_eq!(response.status(), Status::Ok);
    }

    #[test]
    fn metrics_endpoint_exposes_prometheus_text() {
        let client = Client::tracked(rocket()).expect("valid rocket instance");
        let response = client
            .get("/metrics")
            .remote(metrics_remote_addr())
            .dispatch();

        let body = response.into_string().unwrap_or_default();
        assert!(body.contains("metrics_requests_total"));
    }

    #[test]
    fn metrics_endpoint_contains_help_and_type() {
        let client = Client::tracked(rocket()).expect("valid rocket instance");
        let response = client
            .get("/metrics")
            .remote(metrics_remote_addr())
            .dispatch();

        let body = response.into_string().unwrap_or_default();
        // Prometheus format requires HELP and TYPE lines
        assert!(body.contains("# HELP"));
        assert!(body.contains("# TYPE"));
    }

    #[test]
    fn metrics_endpoint_increments_counter() {
        let client = Client::tracked(rocket()).expect("valid rocket instance");

        // First request
        let response1 = client
            .get("/metrics")
            .remote(metrics_remote_addr())
            .dispatch();
        let body1 = response1.into_string().unwrap_or_default();

        // Find metrics_requests_total value
        let count1 = extract_counter_value(&body1, "metrics_requests_total");

        // Second request
        let response2 = client
            .get("/metrics")
            .remote(metrics_remote_addr())
            .dispatch();
        let body2 = response2.into_string().unwrap_or_default();

        let count2 = extract_counter_value(&body2, "metrics_requests_total");

        // Counter should have incremented
        assert!(
            count2 > count1,
            "Counter should increment: {} -> {}",
            count1,
            count2
        );
    }

    #[test]
    fn metrics_endpoint_has_correct_content_type() {
        let client = Client::tracked(rocket()).expect("valid rocket instance");
        let response = client
            .get("/metrics")
            .remote(metrics_remote_addr())
            .dispatch();

        let content_type = response.content_type();
        assert!(content_type.is_some());
        assert_eq!(
            content_type.unwrap().to_string(),
            "text/plain; charset=utf-8"
        );
    }

    #[test]
    fn unknown_endpoint_returns_404() {
        let client = Client::tracked(rocket()).expect("valid rocket instance");
        let response = client.get("/unknown").dispatch();

        assert_eq!(response.status(), Status::NotFound);
    }

    #[test]
    fn metrics_endpoint_denies_unlisted_ip() {
        let client = Client::tracked(rocket()).expect("valid rocket instance");
        let response = client
            .get("/metrics")
            .remote("10.0.0.1:1234".parse().unwrap())
            .dispatch();

        assert_eq!(response.status(), Status::Forbidden);
        assert_eq!(response.into_string().unwrap_or_default(), "access denied");
    }

    fn metrics_remote_addr() -> SocketAddr {
        "127.0.0.1:1234".parse().expect("parse remote addr")
    }

    /// Helper to extract counter value from Prometheus text format
    fn extract_counter_value(body: &str, metric_name: &str) -> u64 {
        for line in body.lines() {
            if line.starts_with(metric_name) && !line.starts_with('#') {
                // Format: metric_name value
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    return parts[1].parse().unwrap_or(0);
                }
            }
        }
        0
    }
}
