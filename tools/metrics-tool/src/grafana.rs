use super::schema::{Metric, Schema};
use regex::Regex;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io;

const DOLLAR: char = '$';
#[derive(Debug)]
pub struct Options {
    pub schema: String,
    pub datasource: String,
    pub datasource_type: String,
    pub instance_var: String,
    pub job_var: String,
    pub sections: Vec<String>,
    pub metrics: Vec<String>,
    pub metric_regex: Option<String>,
    pub list: bool,
    pub all: bool,
    pub dashboard: bool,
    pub dashboard_title: String,
    pub time_from: String,
    pub time_to: String,
    pub dashboard_refresh: String,
    pub rate_window: String,
    pub disable_auto_rate: bool,
    pub datasource_literal: bool,
    pub panel_width: i64,
    pub panel_height: i64,
    pub format: String,
    pub title_prefix: String,
    pub pretty: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            schema: concat!(env!("CARGO_MANIFEST_DIR"), "/../../METRICS.schema.json").to_owned(),
            datasource: "DS_PROMETHEUS".to_owned(),
            datasource_type: "prometheus".to_owned(),
            instance_var: "instance".to_owned(),
            job_var: String::new(),
            sections: Vec::new(),
            metrics: Vec::new(),
            metric_regex: None,
            list: false,
            all: false,
            dashboard: false,
            dashboard_title: "Linux Exporter Metrics".to_owned(),
            time_from: "now-1h".to_owned(),
            time_to: "now".to_owned(),
            dashboard_refresh: String::new(),
            rate_window: "5m".to_owned(),
            disable_auto_rate: false,
            datasource_literal: false,
            panel_width: 12,
            panel_height: 8,
            format: "paste".to_owned(),
            title_prefix: String::new(),
            pretty: false,
        }
    }
}
fn datasource(kind: &str, uid: &str) -> Value {
    json!({"type": kind, "uid": uid})
}
fn selector(metric: &str, opts: &Options) -> String {
    let mut pairs = vec![format!("__name__=\"{metric}\"")];
    if !opts.instance_var.is_empty() {
        pairs.push(format!("instance=~\"{DOLLAR}{{{}}}\"", opts.instance_var));
    }
    if !opts.job_var.is_empty() {
        pairs.push(format!("job=~\"{DOLLAR}{{{}}}\"", opts.job_var));
    }
    format!("{{{}}}", pairs.join(","))
}
fn legend(metric: &Metric) -> String {
    let mut parts = vec!["{{__name__}}".to_owned()];
    parts.extend(
        metric
            .labels
            .iter()
            .filter(|label| !label.is_empty() && !label.starts_with(|c: char| c.is_ascii_digit()))
            .map(|label| format!("{{{{{label}}}}}")),
    );
    parts.join(" ")
}
fn target(index: usize, expr: String, legend: &str, opts: &Options, uid: &str) -> Value {
    let ref_id = if index < 26 {
        char::from(b'A' + u8::try_from(index).unwrap_or(0)).to_string()
    } else {
        format!("A{}", index - 25)
    };
    json!({
        "refId": ref_id, "datasource": datasource(&opts.datasource_type, uid),
        "expr": expr, "legendFormat": legend,
        "range": true, "instant": false, "editorMode": "code"
    })
}
fn auto_rate(metric: &Metric, opts: &Options) -> bool {
    !opts.disable_auto_rate
        && (metric.name.ends_with("_total")
            || metric.kind.starts_with("Counter")
            || matches!(
                metric.name.as_str(),
                "conntrack"
                    | "diskstats"
                    | "netdev"
                    | "netstat"
                    | "rapl_energy_joules"
                    | "snmp"
                    | "softnet"
                    | "vmstat"
            ))
}
pub fn panel(title: &str, metrics: &[&Metric], opts: &Options, uid: &str) -> Value {
    let mut sorted = metrics.to_vec();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    let mut targets = Vec::new();
    for metric in sorted {
        if metric.name.is_empty() {
            continue;
        }
        let select = selector(&metric.name, opts);
        let legend = legend(metric);
        if auto_rate(metric, opts) {
            let fields = if metric.fields.is_empty() {
                &metric.fields_detected
            } else {
                &metric.fields
            };
            if !fields.is_empty() {
                let bytes = fields
                    .iter()
                    .filter(|field| {
                        let lower = field.to_ascii_lowercase();
                        ["byte", "bytes", "octet"]
                            .iter()
                            .any(|part| lower.contains(part))
                    })
                    .map(|field| field.as_str())
                    .collect::<BTreeSet<_>>();
                if !bytes.is_empty() {
                    let escaped = bytes
                        .into_iter()
                        .map(regex::escape)
                        .collect::<Vec<_>>()
                        .join("|");
                    let matcher = format!("^({escaped})$");
                    let byte_sel =
                        format!("{},field=~\"{matcher}\"}}", select.trim_end_matches('}'));
                    let byte_expr = format!("rate({byte_sel}[{}]) * 8", opts.rate_window);
                    targets.push(target(
                        targets.len(),
                        byte_expr,
                        &format!("{legend} (bits/s)"),
                        opts,
                        uid,
                    ));
                    let non_bytes = fields.iter().any(|f| {
                        !["byte", "bytes", "octet"]
                            .iter()
                            .any(|s| f.to_ascii_lowercase().contains(s))
                    });
                    if non_bytes {
                        let other_sel =
                            format!("{},field!~\"{matcher}\"}}", select.trim_end_matches('}'));
                        targets.push(target(
                            targets.len(),
                            format!("rate({other_sel}[{}])", opts.rate_window),
                            &legend,
                            opts,
                            uid,
                        ));
                    }
                    continue;
                }
            }
            targets.push(target(
                targets.len(),
                format!("rate({select}[{}])", opts.rate_window),
                &legend,
                opts,
                uid,
            ));
        } else {
            targets.push(target(targets.len(), select, &legend, opts, uid));
        }
    }
    json!({
        "type": "timeseries", "title": title,
        "datasource": datasource(&opts.datasource_type, uid),
        "targets": targets,
        "fieldConfig": {
            "defaults": {
                "color": {"mode": "palette-classic"}, "mappings": [],
                "thresholds": {"mode": "absolute", "steps": [{"color": "green", "value": null}]},
                "unit": "short"
            },
            "overrides": []
        },
        "options": {
            "legend": {"displayMode": "list", "placement": "bottom", "calcs": ["lastNotNull", "mean"]},
            "tooltip": {"mode": "single"}
        }
    })
}
fn variables(opts: &Options, uid: &str) -> Vec<Value> {
    let mut vars = Vec::new();
    if !opts.job_var.is_empty() {
        vars.push(json!({
            "type": "query", "name": opts.job_var, "label": opts.job_var,
            "description": "", "hide": 0,
            "query": "label_values({__name__=~\".+\"}, job)",
            "datasource": datasource(&opts.datasource_type, uid),
            "pluginId": "prometheus", "pluginName": "Prometheus",
            "current": {"selected": true, "text": "All", "value": "$__all"},
            "includeAll": true, "allValue": "", "multi": true,
            "refresh": 1, "regex": "", "sort": 1, "skipUrlSync": false
        }));
    }
    let mut label_filter = "__name__=~\".+\"".to_owned();
    if !opts.job_var.is_empty() {
        label_filter.push_str(&format!(",job=~\"{DOLLAR}{{{}}}\"", opts.job_var));
    }
    let query = format!("label_values({{{label_filter}}}, instance)");
    vars.push(json!({
        "type": "query", "name": opts.instance_var, "label": opts.instance_var,
        "description": "", "hide": 0, "query": query,
        "datasource": datasource(&opts.datasource_type, uid),
        "pluginId": "prometheus", "pluginName": "Prometheus",
        "current": {"selected": true, "text": "All", "value": "$__all"},
        "includeAll": true, "allValue": "", "multi": true, "refresh": 2,
        "regex": "", "sort": 1, "skipUrlSync": false,
        "definition": query, "hideLabel": true
    }));
    vars
}
pub fn dashboard(mut panels: Vec<Value>, opts: &Options, uid: &str) -> Value {
    let width = if opts.panel_width <= 0 {
        12
    } else {
        opts.panel_width.min(24)
    };
    let height = if opts.panel_height <= 0 {
        8
    } else {
        opts.panel_height
    };
    let mut x = 0;
    let mut y = 0;
    for (index, panel) in panels.iter_mut().enumerate() {
        panel["id"] = json!(index + 1);
        panel["gridPos"] = json!({"h": height, "w": width, "x": x, "y": y});
        x += width;
        if x >= 24 {
            x = 0;
            y += height;
        }
    }
    json!({
        "annotations": {"list": [{
            "builtIn": 1, "datasource": {"type": "grafana", "uid": "-- Grafana --"},
            "enable": true, "hide": true, "iconColor": "rgba(0, 211, 255, 1)",
            "name": "Annotations & Alerts", "type": "dashboard"
        }]},
        "editable": true, "fiscalYearStartMonth": 0, "graphTooltip": 0,
        "id": 1, "links": [], "panels": panels, "preload": false,
        "refresh": opts.dashboard_refresh, "schemaVersion": 42, "tags": [],
        "templating": {"list": variables(opts, uid)},
        "time": {"from": opts.time_from, "to": opts.time_to}, "timepicker": {},
        "timezone": "browser", "title": opts.dashboard_title, "uid": "",
        "version": 1
    })
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
pub fn generate(schema: &Schema, opts: &Options) -> io::Result<String> {
    if opts.list {
        let mut counts = BTreeMap::new();
        for metric in &schema.metrics {
            *counts.entry(metric.group.as_str()).or_insert(0_usize) += 1;
        }
        let mut result = String::from("Available groups:");
        for (name, count) in counts {
            result.push_str(&format!("\n- {name}: {count}"));
        }
        return Ok(result);
    }
    if opts.sections.is_empty()
        && opts.metrics.is_empty()
        && opts.metric_regex.is_none()
        && !opts.all
    {
        return Err(invalid(
            "No section or metric selector provided (--all, --section, --metric, --metric-regex)",
        ));
    }
    for section in &opts.sections {
        if !schema.groups.iter().any(|g| g.name == *section) {
            return Err(invalid(&format!("Unknown section: {section}")));
        }
    }
    let regex = opts
        .metric_regex
        .as_deref()
        .map(Regex::new)
        .transpose()
        .map_err(io::Error::other)?;
    let mut selected = BTreeMap::<String, Vec<&Metric>>::new();
    for metric in &schema.metrics {
        if !opts.sections.is_empty() && !opts.sections.contains(&metric.group) {
            continue;
        }
        if !opts.metrics.is_empty() && !opts.metrics.contains(&metric.name) {
            continue;
        }
        if regex
            .as_ref()
            .is_some_and(|regex| !regex.is_match(&metric.name))
        {
            continue;
        }
        selected
            .entry(metric.group.clone())
            .or_default()
            .push(metric);
    }
    if selected.is_empty() {
        return Err(invalid("No metrics matched the selection"));
    }
    if selected.len() > 1 && !opts.all {
        return Err(invalid(
            "Multiple sections selected; use --all or --section to return one section only",
        ));
    }
    let uid = if opts.datasource_literal {
        opts.datasource.clone()
    } else {
        format!("{DOLLAR}{{{}}}", opts.datasource)
    };
    let panels = selected
        .into_iter()
        .map(|(group, metrics)| {
            let title = if opts.title_prefix.is_empty() {
                group
            } else {
                format!("{} {group}", opts.title_prefix)
            };
            panel(&title, &metrics, opts, &uid)
        })
        .collect::<Vec<_>>();
    let output = if opts.dashboard {
        dashboard(panels, opts, &uid)
    } else if opts.all && opts.format != "paste" {
        Value::Array(panels)
    } else {
        panels
            .into_iter()
            .next()
            .ok_or_else(|| invalid("No panels generated"))?
    };
    if opts.format == "ndjson" {
        if let Some(panels) = output.as_array() {
            return panels
                .iter()
                .map(serde_json::to_string)
                .collect::<Result<Vec<_>, _>>()
                .map(|lines| lines.join("\n"))
                .map_err(io::Error::other);
        }
    }
    if opts.pretty {
        serde_json::to_string_pretty(&output).map_err(io::Error::other)
    } else {
        serde_json::to_string(&output).map_err(io::Error::other)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema;
    #[test]
    fn byte_fields_split_bits_and_non_bytes() -> io::Result<()> {
        let schema = schema::generate(
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../METRICS.md")),
            true,
        )?;
        let mut opts = Options::default();
        opts.sections.push("procfs".to_owned());
        opts.all = true;
        opts.format = "array".to_owned();
        let generated: Value =
            serde_json::from_str(&generate(&schema, &opts)?).map_err(io::Error::other)?;
        let targets = generated
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|panel| panel["targets"].as_array())
            .ok_or_else(|| invalid("Expected targets"))?;
        assert!(targets.iter().any(|target| {
            target["expr"]
                .as_str()
                .is_some_and(|expr| expr.contains(" * 8"))
        }));
        Ok(())
    }
}
