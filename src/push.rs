use crate::config::AppConfig;
use flate2::Compression;
use flate2::write::GzEncoder;
use prometheus::{IntCounter, IntGauge};
use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SPOOL_MAGIC: &[u8; 8] = b"RSLPUSH1";

static PUSH_ATTEMPTS_TOTAL: OnceLock<IntCounter> = OnceLock::new();
static PUSH_FAILURES_TOTAL: OnceLock<IntCounter> = OnceLock::new();
static PUSH_DROPPED_BATCHES_TOTAL: OnceLock<IntCounter> = OnceLock::new();
static PUSH_PENDING_BATCHES: OnceLock<IntGauge> = OnceLock::new();
static PUSH_SPOOL_BYTES: OnceLock<IntGauge> = OnceLock::new();
static PUSH_LAST_SUCCESS_UNIX_SECONDS: OnceLock<IntGauge> = OnceLock::new();

fn attempts_total() -> &'static IntCounter {
    PUSH_ATTEMPTS_TOTAL.get_or_init(|| {
        prometheus::register_int_counter!(
            "metrics_push_attempts_total",
            "Total number of active metrics push HTTP attempts"
        )
        .expect("register metrics_push_attempts_total")
    })
}

fn failures_total() -> &'static IntCounter {
    PUSH_FAILURES_TOTAL.get_or_init(|| {
        prometheus::register_int_counter!(
            "metrics_push_failures_total",
            "Total number of failed active metrics push HTTP attempts"
        )
        .expect("register metrics_push_failures_total")
    })
}

fn dropped_batches_total() -> &'static IntCounter {
    PUSH_DROPPED_BATCHES_TOTAL.get_or_init(|| {
        prometheus::register_int_counter!(
            "metrics_push_dropped_batches_total",
            "Total number of in-memory metric batches dropped because the disk spill cap was full"
        )
        .expect("register metrics_push_dropped_batches_total")
    })
}

fn pending_batches() -> &'static IntGauge {
    PUSH_PENDING_BATCHES.get_or_init(|| {
        prometheus::register_int_gauge!(
            "metrics_push_pending_batches",
            "Number of timestamped metric batches currently waiting in RAM"
        )
        .expect("register metrics_push_pending_batches")
    })
}

fn spool_bytes() -> &'static IntGauge {
    PUSH_SPOOL_BYTES.get_or_init(|| {
        prometheus::register_int_gauge!(
            "metrics_push_spool_bytes",
            "Bytes currently occupied by active metrics push spill files"
        )
        .expect("register metrics_push_spool_bytes")
    })
}

fn last_success_unix_seconds() -> &'static IntGauge {
    PUSH_LAST_SUCCESS_UNIX_SECONDS.get_or_init(|| {
        prometheus::register_int_gauge!(
            "metrics_push_last_success_unixtime",
            "Unix timestamp of the most recent successful active metrics push"
        )
        .expect("register metrics_push_last_success_unixtime")
    })
}

#[derive(Debug)]
struct PendingBatch {
    gzip: Vec<u8>,
}

pub fn start(config: &'static AppConfig) -> io::Result<()> {
    let Some(url) = config.push_url.as_deref() else {
        return Ok(());
    };

    let spool_dir = PathBuf::from(&config.push_spool_dir);
    fs::create_dir_all(&spool_dir)?;
    let _ = attempts_total();
    let _ = failures_total();
    let _ = dropped_batches_total();
    let _ = pending_batches();
    let _ = spool_bytes();
    let _ = last_success_unix_seconds();

    let thread_name = "metrics-push".to_string();
    thread::Builder::new()
        .name(thread_name)
        .spawn(move || run(config, url.to_string(), spool_dir))
        .map(|_| ())
}

fn run(config: &'static AppConfig, url: String, spool_dir: PathBuf) {
    let agent_config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(config.push_timeout_ms)))
        .build();
    let agent: ureq::Agent = agent_config.into();
    let interval = Duration::from_millis(config.push_interval_ms);
    let spill_interval = Duration::from_secs(config.push_spill_after_seconds);
    let mut pending = VecDeque::new();
    let mut first_failure: Option<Instant> = None;
    let mut last_spill: Option<Instant> = None;
    let mut next_tick = Instant::now();

    loop {
        next_tick += interval;
        let timestamp_ms = unix_millis();
        match crate::collect_fresh_text() {
            Ok(text) => match timestamp_and_compress(&text, timestamp_ms) {
                Ok(gzip) => pending.push_back(PendingBatch { gzip }),
                Err(error) => eprintln!("active metrics push compression failed: {error}"),
            },
            Err(error) => eprintln!("active metrics collection failed: {error}"),
        }
        pending_batches().set(i64::try_from(pending.len()).unwrap_or(i64::MAX));

        let now = Instant::now();
        let send_ok = if spool_files(&spool_dir).is_ok_and(|files| !files.is_empty()) {
            drain_spool(&agent, &url, &spool_dir) && drain_pending(&agent, &url, &mut pending)
        } else {
            drain_pending(&agent, &url, &mut pending)
        };

        if send_ok {
            first_failure = None;
            last_spill = None;
        } else {
            let failure_start = *first_failure.get_or_insert(now);
            let spill_due = last_spill.map_or_else(
                || now.duration_since(failure_start) >= spill_interval,
                |last| now.duration_since(last) >= spill_interval,
            );
            if spill_due && !pending.is_empty() {
                match spill_pending(config, &spool_dir, &mut pending) {
                    Ok(()) => last_spill = Some(now),
                    Err(error) => eprintln!("active metrics spill failed: {error}"),
                }
            }
        }
        pending_batches().set(i64::try_from(pending.len()).unwrap_or(i64::MAX));
        update_spool_gauge(&spool_dir);

        let after = Instant::now();
        if after < next_tick {
            thread::sleep(next_tick - after);
        } else {
            next_tick = after;
        }
    }
}

fn drain_pending(agent: &ureq::Agent, url: &str, pending: &mut VecDeque<PendingBatch>) -> bool {
    while let Some(batch) = pending.front() {
        if !send_gzip(agent, url, &batch.gzip) {
            return false;
        }
        pending.pop_front();
    }
    true
}

fn drain_spool(agent: &ureq::Agent, url: &str, spool_dir: &Path) -> bool {
    let files = match spool_files(spool_dir) {
        Ok(files) => files,
        Err(error) => {
            eprintln!("cannot enumerate active metrics spool: {error}");
            return false;
        }
    };

    for path in files {
        let contents = match fs::read(&path) {
            Ok(contents) => contents,
            Err(error) => {
                eprintln!(
                    "cannot read active metrics spool {}: {error}",
                    path.display()
                );
                return false;
            }
        };
        let batches = match decode_spool(&contents) {
            Ok(batches) => batches,
            Err(error) => {
                eprintln!(
                    "discarding corrupt active metrics spool {}: {error}",
                    path.display()
                );
                if let Err(remove_error) = fs::remove_file(&path) {
                    eprintln!(
                        "cannot remove corrupt spool {}: {remove_error}",
                        path.display()
                    );
                    return false;
                }
                continue;
            }
        };
        for gzip in batches {
            if !send_gzip(agent, url, gzip) {
                return false;
            }
        }
        if let Err(error) = fs::remove_file(&path) {
            eprintln!(
                "cannot remove delivered active metrics spool {}: {error}",
                path.display()
            );
            return false;
        }
    }
    true
}

fn send_gzip(agent: &ureq::Agent, url: &str, body: &[u8]) -> bool {
    attempts_total().inc();
    match agent
        .post(url)
        .header("Content-Type", "text/plain; version=0.0.4")
        .header("Content-Encoding", "gzip")
        .send(body)
    {
        Ok(_) => {
            last_success_unix_seconds().set(i64::try_from(unix_seconds()).unwrap_or(i64::MAX));
            true
        }
        Err(error) => {
            failures_total().inc();
            eprintln!("active metrics push failed: {error}");
            false
        }
    }
}

fn spill_pending(
    config: &AppConfig,
    spool_dir: &Path,
    pending: &mut VecDeque<PendingBatch>,
) -> io::Result<()> {
    let encoded = encode_spool(pending.iter().map(|batch| batch.gzip.as_slice()));
    let existing = spool_size(spool_dir)?;
    let encoded_len = u64::try_from(encoded.len()).unwrap_or(u64::MAX);
    if existing.saturating_add(encoded_len) > config.push_spool_max_bytes {
        dropped_batches_total().inc_by(u64::try_from(pending.len()).unwrap_or(u64::MAX));
        pending.clear();
        return Ok(());
    }

    let mut suffix = 0_u32;
    loop {
        let filename = if suffix == 0 {
            format!("{:020}.spool", unix_millis())
        } else {
            format!("{:020}-{suffix}.spool", unix_millis())
        };
        let path = spool_dir.join(filename);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(&encoded)?;
                pending.clear();
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                suffix = suffix.saturating_add(1);
            }
            Err(error) => return Err(error),
        }
    }
}

fn timestamp_and_compress(text: &str, timestamp_ms: u64) -> io::Result<Vec<u8>> {
    let timestamp = timestamp_ms.to_string();
    let mut timestamped = Vec::with_capacity(text.len());
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        timestamped.extend_from_slice(line.as_bytes());
        timestamped.push(b' ');
        timestamped.extend_from_slice(timestamp.as_bytes());
        timestamped.push(b'\n');
    }

    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(&timestamped)?;
    encoder.finish()
}

fn encode_spool<'a>(batches: impl IntoIterator<Item = &'a [u8]>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(SPOOL_MAGIC);
    for batch in batches {
        let Ok(len) = u32::try_from(batch.len()) else {
            continue;
        };
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(batch);
    }
    out
}

fn decode_spool(contents: &[u8]) -> io::Result<Vec<&[u8]>> {
    if !contents.starts_with(SPOOL_MAGIC) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bad spool magic",
        ));
    }
    let mut offset = SPOOL_MAGIC.len();
    let mut batches = Vec::new();
    while offset < contents.len() {
        if contents.len() - offset < 4 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated frame length",
            ));
        }
        let len = u32::from_le_bytes(contents[offset..offset + 4].try_into().expect("four bytes"));
        offset += 4;
        let len = usize::try_from(len).expect("u32 fits usize");
        if contents.len() - offset < len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated frame",
            ));
        }
        batches.push(&contents[offset..offset + len]);
        offset += len;
    }
    Ok(batches)
}

fn spool_files(spool_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = fs::read_dir(spool_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "spool"))
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}

fn spool_size(spool_dir: &Path) -> io::Result<u64> {
    spool_files(spool_dir)?
        .into_iter()
        .try_fold(0_u64, |total, path| {
            Ok(total.saturating_add(fs::metadata(path)?.len()))
        })
}

fn update_spool_gauge(spool_dir: &Path) {
    if let Ok(bytes) = spool_size(spool_dir) {
        spool_bytes().set(i64::try_from(bytes).unwrap_or(i64::MAX));
    }
}

fn unix_millis() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    u64::try_from(millis).unwrap_or(u64::MAX)
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::{decode_spool, encode_spool, timestamp_and_compress};
    use flate2::read::GzDecoder;
    use std::io::Read;

    #[test]
    fn timestamped_payload_excludes_metadata_lines() {
        let compressed = timestamp_and_compress(
            "# HELP x demo\n# TYPE x gauge\nx{a=\"b\"} 1.5\ny 2\n",
            1_234,
        )
        .expect("compress");
        let mut decoder = GzDecoder::new(compressed.as_slice());
        let mut body = String::new();
        decoder.read_to_string(&mut body).expect("decode");
        assert_eq!(body, "x{a=\"b\"} 1.5 1234\ny 2 1234\n");
    }

    #[test]
    fn spool_round_trip_preserves_batches() {
        let first = b"one".as_slice();
        let second = b"two-two".as_slice();
        let encoded = encode_spool([first, second]);
        let decoded = decode_spool(&encoded).expect("decode spool");
        assert_eq!(decoded, vec![first, second]);
    }

    #[test]
    fn truncated_spool_is_rejected() {
        let mut encoded = encode_spool([b"payload".as_slice()]);
        encoded.pop();
        assert!(decode_spool(&encoded).is_err());
    }
}
