use crate::collection::CollectionReport;
use crate::metric_support::{RegisterMetricResultExt, prometheus_u64};
use crate::runtime::debug_enabled;
use prometheus::Gauge;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

const FILE_NR_PATH: &str = "/proc/sys/fs/file-nr";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileFdStats {
    allocated: u64,
    maximum: u64,
}

struct FileFdMetrics {
    allocated: Gauge,
    maximum: Gauge,
}

impl FileFdMetrics {
    fn new() -> Self {
        Self {
            allocated: prometheus::register_gauge!(
                "filefd_allocated",
                "File descriptor statistics: allocated."
            )
            .or_exit("filefd_allocated"),
            maximum: prometheus::register_gauge!(
                "filefd_maximum",
                "File descriptor statistics: maximum."
            )
            .or_exit("filefd_maximum"),
        }
    }

    fn apply(&self, stats: FileFdStats) {
        self.allocated.set(prometheus_u64(stats.allocated));
        self.maximum.set(prometheus_u64(stats.maximum));
    }

    fn mark_unavailable(&self) {
        self.allocated.set(f64::NAN);
        self.maximum.set(f64::NAN);
    }
}

static FILEFD_METRICS: OnceLock<FileFdMetrics> = OnceLock::new();

fn metrics() -> &'static FileFdMetrics {
    FILEFD_METRICS.get_or_init(FileFdMetrics::new)
}

fn parse_file_nr(contents: &str) -> Result<FileFdStats, String> {
    let fields = contents.split_whitespace().collect::<Vec<_>>();
    if fields.len() < 3 {
        return Err(format!(
            "expected at least 3 fields in file-nr, found {}",
            fields.len()
        ));
    }
    let allocated = fields[0].parse::<u64>().map_err(|error| {
        format!(
            "invalid allocated file descriptor count {:?}: {error}",
            fields[0]
        )
    })?;
    let maximum = fields[2].parse::<u64>().map_err(|error| {
        format!(
            "invalid maximum file descriptor count {:?}: {error}",
            fields[2]
        )
    })?;
    Ok(FileFdStats { allocated, maximum })
}

fn update_metrics_from_path(path: &Path) -> CollectionReport {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) => {
            if debug_enabled() {
                eprintln!("filefd: failed to read {}: {error}", path.display());
            }
            metrics().mark_unavailable();
            return CollectionReport::error();
        }
    };
    match parse_file_nr(&contents) {
        Ok(stats) => {
            metrics().apply(stats);
            CollectionReport::success()
        }
        Err(error) => {
            if debug_enabled() {
                eprintln!("filefd: failed to parse {}: {error}", path.display());
            }
            metrics().mark_unavailable();
            CollectionReport::error()
        }
    }
}

pub fn update_metrics() -> CollectionReport {
    update_metrics_from_path(Path::new(FILE_NR_PATH))
}

#[cfg(test)]
mod tests {
    use super::{FileFdStats, parse_file_nr};

    #[test]
    fn parses_current_file_nr_shape() {
        assert_eq!(
            parse_file_nr("2380\t0\t9223372036854775807\n").unwrap(),
            FileFdStats {
                allocated: 2380,
                maximum: 9_223_372_036_854_775_807,
            }
        );
    }

    #[test]
    fn tolerates_future_trailing_fields() {
        assert_eq!(
            parse_file_nr("10 0 20 30\n").unwrap(),
            FileFdStats {
                allocated: 10,
                maximum: 20,
            }
        );
    }

    #[test]
    fn rejects_short_or_invalid_file_nr() {
        assert!(parse_file_nr("1 2\n").is_err());
        assert!(parse_file_nr("nope 0 3\n").is_err());
        assert!(parse_file_nr("1 0 nope\n").is_err());
    }
}
