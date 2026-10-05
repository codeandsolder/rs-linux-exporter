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
