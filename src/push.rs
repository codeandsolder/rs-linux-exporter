use crate::config::AppConfig;
use lurkmoar::{Client, ClientBuilder};
use std::io;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

static CLIENT: OnceLock<Client> = OnceLock::new();

pub fn start(config: &'static AppConfig) -> io::Result<()> {
    let Some(url) = config.push_url.as_deref() else {
        return Ok(());
    };

    let client = ClientBuilder::new(url)
        .timeout(Duration::from_millis(config.push_timeout_ms))
        .retry_interval(Duration::from_secs(1))
        .spill_after(Duration::from_secs(config.push_spill_after_seconds))
        .spool(&config.push_spool_dir, config.push_spool_max_bytes)
        .build()
        .map_err(io::Error::other)?;
    let sender = client.clone();
    CLIENT.set(client).map_err(|_| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "active metrics push already started",
        )
    })?;

    let interval = Duration::from_millis(config.push_interval_ms);
    thread::Builder::new()
        .name("metrics-collect".into())
        .spawn(move || run_sampler(&sender, interval))
        .map(|_| ())
}

fn run_sampler(client: &Client, interval: Duration) {
    let mut next_tick = Instant::now();
    loop {
        next_tick += interval;
        match crate::collect_fresh_text() {
            Ok(text) => {
                if let Err(error) = client.submit_prometheus_text(text) {
                    eprintln!("active metrics push enqueue failed: {error}");
                    return;
                }
            }
            Err(error) => eprintln!("active metrics collection failed: {error}"),
        }
        let now = Instant::now();
        if now < next_tick {
            thread::sleep(next_tick - now);
        } else {
            next_tick = now;
        }
    }
}

pub fn append_health(text: &mut String) {
    let Some(client) = CLIENT.get() else {
        return;
    };
    text.push_str(&client.health_prometheus("metrics_push"));
}
