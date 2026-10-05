use crate::collection::CollectionReport;
use crate::metric_support::{RegisterMetricResultExt, prometheus_i64};
use crate::runtime::debug_enabled;
use etc_os_release::OsRelease;
use prometheus::GaugeVec;
use std::sync::OnceLock;

#[derive(Debug, PartialEq)]
struct OsSnapshot {
    labels: [String; 12],
    version: Option<f64>,
    version_labels: [String; 3],
    support_end: Option<i64>,
}

struct OsMetrics {
    info: GaugeVec,
    version: GaugeVec,
    support_end: GaugeVec,
}

impl OsMetrics {
    fn new() -> Self {
        Self {
            info: prometheus::register_gauge_vec!(
                "os_info",
                "Operating system information from os-release.",
                &[
                    "build_id",
                    "id",
                    "id_like",
                    "image_id",
                    "image_version",
                    "name",
                    "pretty_name",
                    "variant",
                    "variant_id",
                    "version",
                    "version_codename",
                    "version_id"
                ]
            )
            .or_exit("os_info"),
            version: prometheus::register_gauge_vec!(
                "os_version",
                "Major.minor part of the operating-system version.",
                &["id", "id_like", "name"]
            )
            .or_exit("os_version"),
            support_end: prometheus::register_gauge_vec!(
                "os_support_end_timestamp_seconds",
                "End-of-life date of the operating system as a Unix timestamp.",
                &[]
            )
            .or_exit("os_support_end_timestamp_seconds"),
        }
    }

    fn reset(&self) {
        self.info.reset();
        self.version.reset();
        self.support_end.reset();
    }

    fn apply(&self, snapshot: &OsSnapshot) {
        self.reset();
        let labels = snapshot
            .labels
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        self.info.with_label_values(&labels).set(1.0);
        if let Some(version) = snapshot.version {
            let labels = snapshot
                .version_labels
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            self.version.with_label_values(&labels).set(version);
        }
        if let Some(support_end) = snapshot.support_end {
            self.support_end
                .with_label_values(&[] as &[&str])
                .set(prometheus_i64(support_end));
        }
    }
}

static METRICS: OnceLock<OsMetrics> = OnceLock::new();

fn metrics() -> &'static OsMetrics {
    METRICS.get_or_init(OsMetrics::new)
}

fn value(release: &OsRelease, key: &str) -> String {
    release.get_value(key).unwrap_or_default().to_string()
}

fn parse_major_minor(version: &str) -> Option<f64> {
    let mut end = 0;
    let mut saw_dot = false;
    for (index, byte) in version.bytes().enumerate() {
        match byte {
            b'0'..=b'9' => end = index + 1,
            b'.' if !saw_dot && end > 0 => {
                saw_dot = true;
                end = index + 1;
            }
            _ => break,
        }
    }
    let candidate = version.get(..end)?.trim_end_matches('.');
    (!candidate.is_empty())
        .then(|| candidate.parse::<f64>().ok())
        .flatten()
}

fn snapshot(release: &OsRelease) -> OsSnapshot {
    let id = value(release, "ID");
    let id_like = value(release, "ID_LIKE");
    let name = value(release, "NAME");
    let version_id = value(release, "VERSION_ID");
    let support_end = release
        .get_value_as_date("SUPPORT_END")
        .ok()
        .flatten()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|datetime| datetime.and_utc().timestamp());
    OsSnapshot {
        labels: [
            value(release, "BUILD_ID"),
            id.clone(),
            id_like.clone(),
            value(release, "IMAGE_ID"),
            value(release, "IMAGE_VERSION"),
            name.clone(),
            value(release, "PRETTY_NAME"),
            value(release, "VARIANT"),
            value(release, "VARIANT_ID"),
            value(release, "VERSION"),
            value(release, "VERSION_CODENAME"),
            version_id.clone(),
        ],
        version: parse_major_minor(&version_id),
        version_labels: [id, id_like, name],
        support_end,
    }
}

pub fn update_metrics() -> CollectionReport {
    match OsRelease::open() {
        Ok(release) => {
            metrics().apply(&snapshot(&release));
            CollectionReport::success()
        }
        Err(etc_os_release::Error::NoOsRelease) => {
            metrics().reset();
            CollectionReport::success()
        }
        Err(error) => {
            if debug_enabled() {
                eprintln!("os: failed to read os-release: {error}");
            }
            metrics().reset();
            CollectionReport::error()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_major_minor, snapshot};
    use etc_os_release::OsRelease;
    use std::str::FromStr;

    #[test]
    fn extracts_node_exporter_style_os_fields() {
        let release = OsRelease::from_str(
            "NAME=Example\nID=example\nID_LIKE=debian\nVERSION_ID=24.04\nSUPPORT_END=2030-01-02\n",
        )
        .unwrap();
        let snapshot = snapshot(&release);
        assert_eq!(snapshot.labels[1], "example");
        assert_eq!(snapshot.labels[2], "debian");
        assert_eq!(snapshot.version, Some(24.04));
        assert_eq!(snapshot.support_end, Some(1_893_542_400));
    }

    #[test]
    fn major_minor_requires_a_numeric_prefix() {
        assert_eq!(parse_major_minor("12.3.4"), Some(12.3));
        assert_eq!(parse_major_minor("12"), Some(12.0));
        assert_eq!(parse_major_minor("rolling"), None);
    }
}
