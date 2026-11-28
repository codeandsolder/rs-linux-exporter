//! Linux host telemetry exporter with Prometheus-compatible exposition.
//!
//! The binary collects bounded host metrics from procfs, sysfs, system services, and
//! optional external tools, then serves them over HTTP for Prometheus-compatible scrapers.

use crate::metric_support::RegisterMetricResultExt;

mod collection;
mod collectors;
mod config;
mod datasource_amdgpu;
mod datasource_cgroup;
mod datasource_conntrack;
mod datasource_cpufreq;
mod datasource_dmi;
mod datasource_edac;
mod datasource_filefd;
mod datasource_filesystems;
mod datasource_hwmon;
mod datasource_ipmi;
mod datasource_journal;
mod datasource_kernel_hung;
mod datasource_laptop;
mod datasource_lifecycle;
mod datasource_mdraid;
mod datasource_netdev_sysfs;
mod datasource_nfs;
mod datasource_ntp;
mod datasource_numa;
mod datasource_nvme;
mod datasource_os;
mod datasource_power_supply;
mod datasource_pressure;
mod datasource_probe;
mod datasource_procfs;
mod datasource_rapl;
mod datasource_sccache;
mod datasource_schedstat;
mod datasource_smart;
mod datasource_softnet;
mod datasource_systemd;
mod datasource_tailscale;
mod datasource_thermal;
mod datasource_time;
mod datasource_timex;
mod datasource_uname;
mod datasource_watchdog;
mod datasource_wireless;
mod datasource_zfs;
mod exposition;
mod metric_support;
mod plugin;
mod probe;
mod push;
mod refresh;
mod runtime;
mod subprocess;
mod sysfs;

use crate::config::AppConfig;
use crate::exposition::RenderError;
use hyper::StatusCode;
use prometheus::IntCounter;
use std::{net::IpAddr, sync::Mutex, time::Instant};
use tokio::task::spawn_blocking;
mod http_server;
use std::sync::OnceLock;

static METRICS_REQUESTS_TOTAL: OnceLock<IntCounter> = OnceLock::new();
static METRICS_REQUESTS_DENIED_TOTAL: OnceLock<IntCounter> = OnceLock::new();
static APP_CONFIG: OnceLock<AppConfig> = OnceLock::new();

/// Serialises collection and exposition. Collectors reset their vecs and repopulate
/// them, so rendering must not overlap a refresh. Overlapping requests are
/// coalesced: waiters reuse the snapshot completed after their request started.
struct ScrapeState {
    last_completed: Option<Instant>,
}

impl ScrapeState {
    fn needs_refresh(&self, requested_at: Instant) -> bool {
        self.last_completed
            .is_none_or(|completed| completed < requested_at)
    }
}

static SCRAPE_STATE: Mutex<ScrapeState> = Mutex::new(ScrapeState {
    last_completed: None,
});

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
/// handler would tie up an async worker thread for the duration.
#[expect(
    clippy::significant_drop_tightening,
    reason = "the scrape mutex must cover rendering so another refresh cannot reset metric vectors mid-encoding"
)]
fn refresh_and_render(render: fn() -> Result<String, RenderError>) -> Result<String, RenderError> {
    let requested_at = Instant::now();
    let mut state = SCRAPE_STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let needs_refresh = state.needs_refresh(requested_at);
    if needs_refresh {
        collectors::update_metrics(app_config());
        state.last_completed = Some(Instant::now());
    }
    render()
}

#[expect(
    clippy::significant_drop_tightening,
    reason = "the scrape mutex must cover rendering so another refresh cannot reset metric vectors mid-encoding"
)]
pub(crate) fn collect_fresh_text() -> Result<String, RenderError> {
    let mut state = SCRAPE_STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    collectors::update_metrics(app_config());
    state.last_completed = Some(Instant::now());
    render_text_with_push_health()
}

fn render_text_with_push_health() -> Result<String, RenderError> {
    let mut text = exposition::render_text()?;
    push::append_health(&mut text);
    Ok(text)
}

const fn collection_failed() -> (StatusCode, &'static str) {
    (StatusCode::INTERNAL_SERVER_ERROR, "collection failed")
}

fn authorize_metrics_request(
    endpoint: &'static str,
    client_ip: Option<IpAddr>,
    token: Option<&str>,
) -> Result<(), (StatusCode, &'static str)> {
    let config = app_config();
    if !config.is_token_valid(token) {
        if config.log_denied_requests {
            eprintln!(
                "Denied {endpoint} request from {} (invalid token)",
                client_ip.map_or_else(|| "<unknown>".to_string(), |ip| ip.to_string())
            );
        }
        metrics_requests_denied_total().inc();
        return Err((StatusCode::UNAUTHORIZED, "unauthorized"));
    }

    if !client_ip.is_some_and(|ip| config.is_metrics_ip_allowed(ip)) {
        if config.log_denied_requests {
            eprintln!(
                "Denied {endpoint} request from {}",
                client_ip.map_or_else(|| "<unknown>".to_string(), |ip| ip.to_string())
            );
        }
        metrics_requests_denied_total().inc();
        return Err((StatusCode::FORBIDDEN, "access denied"));
    }
    Ok(())
}

async fn collect_and_render(
    renderer: fn() -> Result<String, RenderError>,
) -> Result<String, (StatusCode, &'static str)> {
    match spawn_blocking(move || refresh_and_render(renderer)).await {
        Ok(Ok(body)) => Ok(body),
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    runtime::init();
    let config = app_config();
    let _ = metrics_requests_total();
    let _ = metrics_requests_denied_total();
    let bind = config.bind_addr().unwrap_or_else(|err| {
        eprintln!("Invalid runtime configuration: {err}");
        std::process::exit(78);
    });
    let tls = config
        .tls_config()
        .unwrap_or_else(|err| {
            eprintln!("Invalid runtime configuration: {err}");
            std::process::exit(78);
        })
        .map(|(cert, key)| http_server::load_tls(cert, key))
        .transpose()?;
    if let Some(path) = config.plugin_socket.as_deref()
        && let Err(error) = plugin::start_server(std::path::Path::new(path))
    {
        eprintln!("Failed to start plugin socket: {error}");
        std::process::exit(78);
    }
    if let Err(error) = push::start(config) {
        eprintln!("Failed to start active metrics push worker: {error}");
        std::process::exit(78);
    }
    if runtime::debug_enabled() {
        eprintln!("Debug logging enabled.");
    }
    http_server::serve(bind, tls).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ScrapeState, http_server::handle_request};
    use http_body_util::BodyExt as _;
    use hyper::{Request, StatusCode, header};
    use std::net::SocketAddr;
    use std::time::Instant;

    async fn request(path: &str, peer: SocketAddr) -> super::http_server::HttpResponse {
        handle_request(
            Request::builder().uri(path).body(()).expect("request"),
            peer,
        )
        .await
    }

    async fn response_text(response: super::http_server::HttpResponse) -> String {
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("response body")
            .to_bytes();
        String::from_utf8(bytes.to_vec()).expect("UTF-8 response")
    }

    #[tokio::test]
    async fn index_returns_hint() {
        let response = request("/", metrics_remote_addr()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_text(response).await, "rs-linux-exporter: /metrics");
    }

    #[test]
    fn overlapping_scrapes_reuse_the_completed_snapshot() {
        let start = Instant::now();
        let mut state = ScrapeState {
            last_completed: None,
        };
        assert!(state.needs_refresh(start));

        let overlapping_request = start + std::time::Duration::from_millis(1);
        state.last_completed = Some(start + std::time::Duration::from_millis(2));
        assert!(!state.needs_refresh(overlapping_request));

        let later_request = start + std::time::Duration::from_millis(3);
        assert!(state.needs_refresh(later_request));
    }

    #[tokio::test]
    async fn metrics_endpoint_returns_ok() {
        assert_eq!(
            request("/metrics", metrics_remote_addr()).await.status(),
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn metrics_endpoint_exposes_prometheus_text() {
        let body = response_text(request("/metrics", metrics_remote_addr()).await).await;
        assert!(body.contains("metrics_requests_total"));
    }

    #[tokio::test]
    async fn metrics_endpoint_contains_help_and_type() {
        let body = response_text(request("/metrics", metrics_remote_addr()).await).await;
        assert!(body.contains("# HELP"));
        assert!(body.contains("# TYPE"));
    }

    #[tokio::test]
    async fn metrics_endpoint_increments_counter() {
        let body1 = response_text(request("/metrics", metrics_remote_addr()).await).await;
        let count1 = extract_counter_value(&body1, "metrics_requests_total");
        let body2 = response_text(request("/metrics", metrics_remote_addr()).await).await;
        let count2 = extract_counter_value(&body2, "metrics_requests_total");
        assert!(
            count2 > count1,
            "Counter should increment: {count1} -> {count2}"
        );
    }

    #[tokio::test]
    async fn metrics_endpoint_has_correct_content_type() {
        let response = request("/metrics", metrics_remote_addr()).await;
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/plain; charset=utf-8")
        );
    }

    #[tokio::test]
    async fn unknown_endpoint_returns_404() {
        assert_eq!(
            request("/unknown", metrics_remote_addr()).await.status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn metrics_endpoint_denies_unlisted_ip() {
        let response = request("/metrics", "10.0.0.1:1234".parse().unwrap()).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response_text(response).await, "access denied");
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
