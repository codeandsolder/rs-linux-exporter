use prometheus::proto::{MetricFamily, MetricType};
use prometheus::{Encoder, TextEncoder};
use serde_json::Value as JsonValue;
use std::fmt;
use std::string::FromUtf8Error;

#[derive(Debug)]
pub enum RenderError {
    Prometheus(prometheus::Error),
    Utf8(FromUtf8Error),
    Json(serde_json::Error),
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prometheus(err) => write!(formatter, "Prometheus encoding failed: {err}"),
            Self::Utf8(err) => write!(
                formatter,
                "Prometheus encoding produced invalid UTF-8: {err}"
            ),
            Self::Json(err) => write!(formatter, "JSON encoding failed: {err}"),
        }
    }
}

impl std::error::Error for RenderError {}

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

fn json_from_families(families: Vec<MetricFamily>) -> Result<String, RenderError> {
    let mut samples = Vec::new();

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
                MetricType::COUNTER => push_json_sample(
                    &mut samples,
                    name,
                    &base_labels,
                    JsonValue::from(metric.get_counter().value()),
                ),
                MetricType::GAUGE => push_json_sample(
                    &mut samples,
                    name,
                    &base_labels,
                    JsonValue::from(metric.get_gauge().value()),
                ),
                MetricType::UNTYPED => {
                    if let Some(untyped) = metric.untyped.as_ref() {
                        push_json_sample(
                            &mut samples,
                            name,
                            &base_labels,
                            JsonValue::from(untyped.value()),
                        );
                    }
                }
                MetricType::HISTOGRAM => {
                    let histogram = metric.get_histogram();
                    for bucket in histogram.get_bucket() {
                        let mut labels = base_labels.clone();
                        labels.push(("le".to_string(), bucket.upper_bound().to_string()));
                        push_json_sample(
                            &mut samples,
                            &format!("{name}_bucket"),
                            &labels,
                            JsonValue::from(bucket.cumulative_count()),
                        );
                    }
                    push_json_sample(
                        &mut samples,
                        &format!("{name}_sum"),
                        &base_labels,
                        JsonValue::from(histogram.sample_sum()),
                    );
                    push_json_sample(
                        &mut samples,
                        &format!("{name}_count"),
                        &base_labels,
                        JsonValue::from(histogram.sample_count()),
                    );
                }
                MetricType::SUMMARY => {
                    let summary = metric.get_summary();
                    for quantile in summary.get_quantile() {
                        let mut labels = base_labels.clone();
                        labels.push(("quantile".to_string(), quantile.quantile().to_string()));
                        // Prometheus summaries use the base metric name with a
                        // `quantile` label; only _sum/_count use suffixes.
                        push_json_sample(
                            &mut samples,
                            name,
                            &labels,
                            JsonValue::from(quantile.value()),
                        );
                    }
                    push_json_sample(
                        &mut samples,
                        &format!("{name}_sum"),
                        &base_labels,
                        JsonValue::from(summary.sample_sum()),
                    );
                    push_json_sample(
                        &mut samples,
                        &format!("{name}_count"),
                        &base_labels,
                        JsonValue::from(summary.sample_count()),
                    );
                }
            }
        }
    }

    serde_json::to_string(&samples).map_err(RenderError::Json)
}

pub fn render_json() -> Result<String, RenderError> {
    json_from_families(prometheus::gather())
}

pub fn render_text() -> Result<String, RenderError> {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    encoder
        .encode(&metric_families, &mut buffer)
        .map_err(RenderError::Prometheus)?;
    String::from_utf8(buffer).map_err(RenderError::Utf8)
}

#[cfg(test)]
mod tests {
    use super::json_from_families;
    use prometheus::proto::{Metric, MetricFamily, MetricType, Quantile, Summary, Untyped};
    use serde_json::Value;

    fn family(name: &str, metric_type: MetricType, metric: Metric) -> MetricFamily {
        let mut family = MetricFamily::new();
        family.set_name(name.to_string());
        family.set_field_type(metric_type);
        family.metric.push(metric);
        family
    }

    #[test]
    fn summary_quantiles_use_base_name_and_quantile_label() {
        let mut quantile = Quantile::new();
        quantile.set_quantile(0.5);
        quantile.set_value(12.5);
        let mut summary = Summary::new();
        summary.set_sample_count(4);
        summary.set_sample_sum(50.0);
        summary.quantile.push(quantile);
        let mut metric = Metric::new();
        metric.summary = Some(summary).into();

        let body = json_from_families(vec![family("latency_seconds", MetricType::SUMMARY, metric)])
            .expect("serialize summary");
        let rows: Vec<Value> = serde_json::from_str(&body).expect("parse JSON");

        assert!(rows.iter().any(|row| {
            row["_name_"] == "latency_seconds" && row["quantile"] == "0.5" && row["_value_"] == 12.5
        }));
        assert!(
            !rows
                .iter()
                .any(|row| row["_name_"] == "latency_seconds_quantile")
        );
    }

    #[test]
    fn untyped_metrics_are_not_silently_dropped() {
        let mut untyped = Untyped::new();
        untyped.set_value(7.25);
        let mut metric = Metric::new();
        metric.untyped = Some(untyped).into();

        let body = json_from_families(vec![family("legacy_value", MetricType::UNTYPED, metric)])
            .expect("serialize untyped metric");
        let rows: Vec<Value> = serde_json::from_str(&body).expect("parse JSON");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["_name_"], "legacy_value");
        assert_eq!(rows[0]["_value_"], 7.25);
    }
}
