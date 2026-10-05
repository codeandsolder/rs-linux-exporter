/// Convert a kernel/user-space `u64` counter to Prometheus' sample value type.
///
/// Prometheus samples are `f64`, so integers above 2^53 cannot be represented
/// exactly at the exposition boundary regardless of the in-process metric type.
#[expect(
    clippy::cast_precision_loss,
    reason = "Prometheus sample values are f64; the precision boundary is inherent in exposition"
)]
pub const fn prometheus_u64(value: u64) -> f64 {
    value as f64
}

/// Convert a signed 64-bit sensor/kernel value to Prometheus' sample value type.
#[expect(
    clippy::cast_precision_loss,
    reason = "Prometheus sample values are f64; the precision boundary is inherent in exposition"
)]
pub const fn prometheus_i64(value: i64) -> f64 {
    value as f64
}

/// Convert Prometheus registration failures into an explicit process-level
/// startup/collection failure instead of a hidden panic.
///
/// Registration can fail only when the program defines an invalid descriptor
/// or attempts to register a duplicate descriptor. Continuing would expose an
/// incomplete or ambiguous metric set, so this is a fatal software error.
pub(crate) trait RegisterMetricResultExt<T> {
    fn or_exit(self, metric_name: &'static str) -> T;
}

impl<T> RegisterMetricResultExt<T> for Result<T, prometheus::Error> {
    fn or_exit(self, metric_name: &'static str) -> T {
        match self {
            Ok(metric) => metric,
            Err(error) => {
                eprintln!("failed to register Prometheus metric {metric_name}: {error}");
                std::process::exit(70);
            }
        }
    }
}
