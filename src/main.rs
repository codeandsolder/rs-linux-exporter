use crate::metric_support::RegisterMetricResultExt;
#[macro_use]
extern crate rocket;

mod collectors;
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
mod exposition;
mod metric_support;
mod runtime;
mod sysfs;

use crate::config::AppConfig;
use crate::exposition::RenderError;
use prometheus::IntCounter;
use rocket::Config;
use rocket::config::TlsConfig;
use rocket::http::{ContentType, Status};
use rocket::request::{FromRequest, Outcome, Request};
use rocket::response::status;
use rocket::tokio::task::spawn_blocking;
use std::net::IpAddr;
use std::sync::Mutex;

/// Extracts Bearer token from Authorization header
struct BearerToken<'r>(Option<&'r str>);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for BearerToken<'r> {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let token = request
            .headers()
            .get_one("Authorization")
            .and_then(|header| header.strip_prefix("Bearer "));
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
            "Total number of metrics endpoint requests"
        )
        .or_exit("metrics_requests_total")
    })
}

fn metrics_requests_denied_total() -> &'static IntCounter {
    METRICS_REQUESTS_DENIED_TOTAL.get_or_init(|| {
        prometheus::register_int_counter!(
            "metrics_requests_denied_total",
            "Total number of metrics endpoint requests denied by authentication or ACL"
        )
        .or_exit("metrics_requests_denied_total")
    })
}

fn app_config() -> &'static AppConfig {
    APP_CONFIG.get_or_init(AppConfig::load)
}

/// Refreshes every enabled datasource and renders the result.
///
/// Runs on the blocking pool: collectors do synchronous filesystem and netlink
/// I/O, and `statvfs` on an unresponsive network mount or an IPMI controller
/// that is slow to answer can block for seconds. Doing that directly in the
/// handler would tie up a Rocket worker thread for the duration.
fn refresh_and_render(render: fn() -> Result<String, RenderError>) -> Result<String, RenderError> {
    let _guard = SCRAPE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    collectors::update_metrics(app_config());
    render()
}

const fn collection_failed() -> status::Custom<&'static str> {
    status::Custom(Status::InternalServerError, "collection failed")
}

fn authorize_metrics_request(
    endpoint: &'static str,
    client_ip: Option<IpAddr>,
    token: Option<&str>,
) -> Result<(), status::Custom<&'static str>> {
    let config = app_config();
    if !config.is_token_valid(token) {
        if config.log_denied_requests {
            eprintln!(
                "Denied {endpoint} request from {} (invalid token)",
                client_ip.map_or_else(|| "<unknown>".to_string(), |ip| ip.to_string())
            );
        }
        metrics_requests_denied_total().inc();
        return Err(status::Custom(Status::Unauthorized, "unauthorized"));
    }

    if !client_ip.is_some_and(|ip| config.is_metrics_ip_allowed(ip)) {
        if config.log_denied_requests {
            eprintln!(
                "Denied {endpoint} request from {}",
                client_ip.map_or_else(|| "<unknown>".to_string(), |ip| ip.to_string())
            );
        }
        metrics_requests_denied_total().inc();
        return Err(status::Custom(Status::Forbidden, "access denied"));
    }
    Ok(())
}

async fn collect_and_render(
    renderer: fn() -> Result<String, RenderError>,
    content_type: ContentType,
) -> Result<(ContentType, String), status::Custom<&'static str>> {
    match spawn_blocking(move || refresh_and_render(renderer)).await {
        Ok(Ok(body)) => Ok((content_type, body)),
        Ok(Err(err)) => {
            eprintln!("Metrics rendering failed: {err}");
            Err(collection_failed())
        }
        Err(err) => {
            eprintln!("Metrics collection task failed: {err}");
            Err(collection_failed())
        }
    }
}

#[get("/metrics")]
async fn metrics(
    client_ip: Option<IpAddr>,
    token: BearerToken<'_>,
) -> Result<(ContentType, String), status::Custom<&'static str>> {
    metrics_requests_total().inc();
    authorize_metrics_request("/metrics", client_ip, token.0)?;
    collect_and_render(exposition::render_text, ContentType::Plain).await
}

#[get("/metrics.json")]
async fn metrics_json(
    client_ip: Option<IpAddr>,
    token: BearerToken<'_>,
) -> Result<(ContentType, String), status::Custom<&'static str>> {
    metrics_requests_total().inc();
    authorize_metrics_request("/metrics.json", client_ip, token.0)?;
    collect_and_render(exposition::render_json, ContentType::JSON).await
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
    let config = app_config();
    let _ = metrics_requests_total();
    let _ = metrics_requests_denied_total();
    if runtime::debug_enabled() {
        eprintln!("Debug logging enabled.");
    }
    let bind = match config.bind_addr() {
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

    match config.tls_config() {
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
