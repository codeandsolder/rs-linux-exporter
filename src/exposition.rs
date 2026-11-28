use prometheus::TextEncoder;
use prometheus::proto::{Histogram, MetricFamily, MetricType, Summary};
use serde_json::Value as JsonValue;
use std::fmt;

#[derive(Debug)]
pub enum RenderError {
    Prometheus(prometheus::Error),
    Json(serde_json::Error),
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prometheus(err) => write!(formatter, "Prometheus encoding failed: {err}"),
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
    let _previous_name = map.insert("_name_".to_string(), JsonValue::from(name));
    for (key, value) in labels {
        let _previous_label = map.insert(key.clone(), JsonValue::from(value.clone()));
    }
    let _previous_value = map.insert("_value_".to_string(), value);
    samples.push(map);
}

fn push_json_histogram(
    samples: &mut Vec<serde_json::Map<String, JsonValue>>,
    name: &str,
    base_labels: &[(String, String)],
    histogram: &Histogram,
) {
    let bucket_name = format!("{name}_bucket");
    let mut has_inf = false;
    for bucket in histogram.get_bucket() {
        let upper_bound = bucket.upper_bound();
        let is_inf = upper_bound.is_infinite() && upper_bound.is_sign_positive();
        has_inf |= is_inf;
        let mut labels = base_labels.to_vec();
        labels.push((
            "le".to_string(),
            if is_inf {
                "+Inf".to_string()
            } else {
                upper_bound.to_string()
            },
        ));
        push_json_sample(
            samples,
            &bucket_name,
            &labels,
            JsonValue::from(bucket.cumulative_count()),
        );
    }
    if !has_inf {
        let mut labels = base_labels.to_vec();
        labels.push(("le".to_string(), "+Inf".to_string()));
        push_json_sample(
            samples,
            &bucket_name,
            &labels,
            JsonValue::from(histogram.get_sample_count()),
        );
    }
    push_json_sample(
        samples,
        &format!("{name}_sum"),
        base_labels,
        JsonValue::from(histogram.get_sample_sum()),
    );
    push_json_sample(
        samples,
        &format!("{name}_count"),
        base_labels,
        JsonValue::from(histogram.get_sample_count()),
    );
}

fn push_json_summary(
    samples: &mut Vec<serde_json::Map<String, JsonValue>>,
    name: &str,
    base_labels: &[(String, String)],
    summary: &Summary,
) {
    for quantile in summary.get_quantile() {
        let mut labels = base_labels.to_vec();
        labels.push(("quantile".to_string(), quantile.quantile().to_string()));
        // Prometheus summaries use the base metric name with a
        // `quantile` label; only _sum/_count use suffixes.
        push_json_sample(samples, name, &labels, JsonValue::from(quantile.value()));
    }
    push_json_sample(
        samples,
        &format!("{name}_sum"),
        base_labels,
        JsonValue::from(summary.sample_sum()),
    );
    push_json_sample(
        samples,
        &format!("{name}_count"),
        base_labels,
        JsonValue::from(summary.sample_count()),
    );
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
                    JsonValue::from(metric.get_counter().get_value()),
                ),
                MetricType::GAUGE => push_json_sample(
                    &mut samples,
                    name,
                    &base_labels,
                    JsonValue::from(metric.get_gauge().get_value()),
                ),
                MetricType::UNTYPED => push_json_sample(
                    &mut samples,
                    name,
                    &base_labels,
                    JsonValue::from(metric.get_untyped().get_value()),
                ),
                MetricType::HISTOGRAM => {
                    push_json_histogram(&mut samples, name, &base_labels, metric.get_histogram());
                }
                MetricType::SUMMARY => {
                    push_json_summary(&mut samples, name, &base_labels, metric.get_summary());
                }
            }
        }
    }

    serde_json::to_string(&samples).map_err(RenderError::Json)
}

fn gather_all() -> Vec<MetricFamily> {
    let mut families = prometheus::gather();
    families.extend(crate::plugin::metric_families());
    families.sort_by(|left, right| left.name().cmp(right.name()));
    families
}

pub fn render_json() -> Result<String, RenderError> {
    json_from_families(gather_all())
}

pub fn render_text() -> Result<String, RenderError> {
    TextEncoder::new()
        .encode_to_string(&gather_all())
        .map_err(RenderError::Prometheus)
}

#[cfg(test)]
mod tests {
    use super::json_from_families;
    use prometheus::proto::{
        Bucket, Histogram, Metric, MetricFamily, MetricType, Quantile, Summary, Untyped,
    };
    use serde_json::Value;

    fn family(name: &str, metric_type: MetricType, metric: Metric) -> MetricFamily {
        let mut family = MetricFamily::default();
        family.set_name(name.to_string());
        family.set_field_type(metric_type);
        family.set_metric(vec![metric]);
        family
    }

    #[test]
    fn summary_quantiles_use_base_name_and_quantile_label() {
        let mut quantile = Quantile::default();
        quantile.set_quantile(0.5);
        quantile.set_value(12.5);
        let mut summary = Summary::default();
        summary.set_sample_count(4);
        summary.set_sample_sum(50.0);
        summary.set_quantile(vec![quantile]);
        let mut metric = Metric::default();
        metric.set_summary(summary);

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
    fn histogram_infinity_bucket_matches_text_exposition() {
        for explicit_infinity in [false, true] {
            let mut finite = Bucket::default();
            finite.set_upper_bound(1.0);
            finite.set_cumulative_count(2);
            let mut buckets = vec![finite];
            if explicit_infinity {
                let mut infinite = Bucket::default();
                infinite.set_upper_bound(f64::INFINITY);
                infinite.set_cumulative_count(4);
                buckets.push(infinite);
            }
            let mut histogram = Histogram::default();
            histogram.set_sample_count(4);
            histogram.set_sample_sum(9.0);
            histogram.set_bucket(buckets);
            let mut metric = Metric::default();
            metric.set_histogram(histogram);
            let body = json_from_families(vec![family(
                "latency_seconds",
                MetricType::HISTOGRAM,
                metric,
            )])
            .expect("serialize histogram");
            let rows: Vec<Value> = serde_json::from_str(&body).expect("parse JSON");
            let infinities: Vec<_> = rows
                .iter()
                .filter(|row| row["_name_"] == "latency_seconds_bucket" && row["le"] == "+Inf")
                .collect();
            assert_eq!(infinities.len(), 1);
            assert_eq!(infinities[0]["_value_"], 4);
        }
    }

    #[test]
    fn untyped_metrics_are_not_silently_dropped() {
        let mut untyped = Untyped::default();
        untyped.set_value(7.25);
        let mut metric = Metric::default();
        metric.set_untyped(untyped);

        let body = json_from_families(vec![family("legacy_value", MetricType::UNTYPED, metric)])
            .expect("serialize untyped metric");
        let rows: Vec<Value> = serde_json::from_str(&body).expect("parse JSON");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["_name_"], "legacy_value");
        assert_eq!(rows[0]["_value_"], 7.25);
    }
}
