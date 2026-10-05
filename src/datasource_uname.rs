use crate::collection::CollectionReport;
use crate::metric_support::RegisterMetricResultExt;
use prometheus::GaugeVec;
use std::sync::OnceLock;

struct UnameMetrics {
    info: GaugeVec,
}

impl UnameMetrics {
    fn new() -> Self {
        Self {
            info: prometheus::register_gauge_vec!(
                "uname_info",
                "Labeled system information as provided by the uname system call.",
                &[
                    "sysname",
                    "release",
                    "version",
                    "machine",
                    "nodename",
                    "domainname"
                ]
            )
            .or_exit("uname_info"),
        }
    }
}

static METRICS: OnceLock<UnameMetrics> = OnceLock::new();

fn metrics() -> &'static UnameMetrics {
    METRICS.get_or_init(UnameMetrics::new)
}

pub fn update_metrics() -> CollectionReport {
    let uname = rustix::system::uname();
    let sysname = uname.sysname().to_string_lossy();
    let release = uname.release().to_string_lossy();
    let version = uname.version().to_string_lossy();
    let machine = uname.machine().to_string_lossy();
    let nodename = uname.nodename().to_string_lossy();
    let domainname = uname.domainname().to_string_lossy();
    let labels = [
        sysname.as_ref(),
        release.as_ref(),
        version.as_ref(),
        machine.as_ref(),
        nodename.as_ref(),
        domainname.as_ref(),
    ];
    metrics().info.reset();
    metrics().info.with_label_values(&labels).set(1.0);
    CollectionReport::success()
}
