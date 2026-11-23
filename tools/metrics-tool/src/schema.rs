use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::Path;

const TICK: char = '\x60';

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Metric {
    pub name: String,
    pub group: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub description: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub label_values: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields_detected: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Group {
    pub name: String,
    pub metrics: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Schema {
    pub version: String,
    pub source_file: String,
    pub metrics: Vec<Metric>,
    pub groups: Vec<Group>,
}

#[derive(Default)]
struct Metadata {
    labels: Vec<String>,
    fields: Vec<String>,
    label_values: BTreeMap<String, Vec<String>>,
}

enum Capture {
    Fields(String),
    LabelValues(String, String),
}

fn dedupe(values: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

pub fn to_snake_case(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::new();
    let mut previous = None;
    for (index, &character) in chars.iter().enumerate() {
        if !character.is_ascii_alphanumeric() {
            if !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            previous = Some(character);
            continue;
        }
        if let Some(prev) = previous {
            let next_lower = chars.get(index + 1).is_some_and(char::is_ascii_lowercase);
            let upper_boundary = character.is_ascii_uppercase()
                && (prev.is_ascii_lowercase()
                    || prev.is_ascii_digit()
                    || (prev.is_ascii_uppercase() && next_lower));
            let digit_boundary = character.is_ascii_digit() && prev.is_ascii_alphabetic();
            let lower_boundary = character.is_ascii_lowercase() && prev.is_ascii_digit();
            if (upper_boundary || digit_boundary || lower_boundary)
                && !out.is_empty()
                && !out.ends_with('_')
            {
                out.push('_');
            }
        }
        out.push(character.to_ascii_lowercase());
        previous = Some(character);
    }
    let result = out.trim_matches('_');
    if result.is_empty() {
        "unknown".to_owned()
    } else {
        result.to_owned()
    }
}

pub fn kernel_counter_field_name(section: &str, field: &str) -> String {
    let field_name = match (section, field) {
        ("Ip", "ReasmOKs") => "reasm_oks".to_owned(),
        ("Ip", "FragOKs") => "frag_oks".to_owned(),
        _ => to_snake_case(field),
    };
    format!("{}_{}", to_snake_case(section), field_name)
}

fn known_metric_name(name: &str, meta: &HashMap<String, Metadata>) -> String {
    if meta.contains_key(name) {
        return name.to_owned();
    }
    if matches!(name, "netstat" | "ip_ext" | "tcp_ext" | "mptcp_ext")
        || ["tcp_ext_", "ip_ext_", "mptcp_ext_"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
    {
        "netstat".to_owned()
    } else {
        name.to_owned()
    }
}

fn inline_codes(text: &str) -> Vec<String> {
    text.split(TICK)
        .skip(1)
        .step_by(2)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn table_row(line: &str) -> Option<(&str, &str, String)> {
    let inner = line.strip_prefix('|')?.trim_end_matches('|');
    let mut cells = inner.split('|');
    let metric = cells.next()?.trim().trim_matches(TICK);
    let kind = cells.next()?.trim();
    let description = cells.collect::<Vec<_>>().join(" | ").trim().to_owned();
    if metric.is_empty() || metric.chars().any(char::is_whitespace) || kind.is_empty() {
        return None;
    }
    Some((metric, kind, description))
}

fn parse_markdown(contents: &str) -> Schema {
    let mut groups = Vec::<Group>::new();
    let mut metrics = Vec::<Metric>::new();
    let mut metadata = HashMap::<String, Metadata>::new();
    let mut current_group = None;
    let mut in_labels = false;
    let mut in_table = false;
    let mut capture = None;

    for raw in contents.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim();
        if let Some(title) = line.strip_prefix("## ") {
            let title = title.trim();
            in_labels = title == "Metric labels and field catalogs";
            in_table = false;
            capture = None;
            if in_labels || title.starts_with("TODO") {
                current_group = None;
            } else {
                if !groups.iter().any(|g| g.name == title) {
                    groups.push(Group {
                        name: title.to_owned(),
                        metrics: Vec::new(),
                    });
                }
                current_group = Some(title.to_owned());
            }
            continue;
        }

        if !in_labels {
            if let Some(group) = current_group.as_ref() {
                if trimmed == "| Metric | Type | Description |" {
                    in_table = true;
                    continue;
                }
                if in_table {
                    if trimmed.starts_with('|')
                        && trimmed
                            .trim_start_matches('|')
                            .trim_start()
                            .starts_with('-')
                    {
                        continue;
                    }
                    if let Some((name, kind, description)) = table_row(line) {
                        if let Some(entry) = groups.iter_mut().find(|entry| entry.name == *group) {
                            entry.metrics.push(name.to_owned());
                        }
                        let _ = metadata.entry(name.to_owned()).or_default();
                        metrics.push(Metric {
                            name: name.to_owned(),
                            group: group.clone(),
                            kind: kind.to_owned(),
                            description,
                            labels: Vec::new(),
                            label_values: BTreeMap::new(),
                            fields: Vec::new(),
                            fields_detected: Vec::new(),
                        });
                        continue;
                    }
                    in_table = false;
                }
            }
            continue;
        }

        if let Some(heading) = line.strip_prefix("### ") {
            capture = None;
            if let Some((raw_metric, body)) = heading.trim().split_once(" labels:") {
                let name = known_metric_name(raw_metric.trim_matches(TICK), &metadata);
                let labels = inline_codes(body);
                if !labels.is_empty() {
                    metadata.entry(name).or_default().labels = labels;
                }
            }
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix(TICK) {
            if let Some((raw_metric, body)) = rest.split_once("\x60:") {
                let name = known_metric_name(raw_metric, &metadata);
                let labels = inline_codes(body);
                if !labels.is_empty() {
                    metadata.entry(name).or_default().labels = labels;
                }
                capture = None;
                continue;
            }
            if let Some((raw_metric, suffix)) = rest.split_once("\x60 label values") {
                if suffix.ends_with(':') {
                    let name = known_metric_name(raw_metric, &metadata);
                    let label = inline_codes(suffix)
                        .into_iter()
                        .last()
                        .unwrap_or_else(|| "value".to_owned());
                    let _ = metadata.entry(name.clone()).or_default();
                    capture = Some(Capture::LabelValues(name, label));
                    continue;
                }
            }
            if let Some((raw_metric, suffix)) = rest.split_once("\x60 field values ") {
                if suffix.ends_with(':')
                    && (suffix.starts_with("include") || suffix.starts_with("are generated from"))
                {
                    let name = known_metric_name(raw_metric, &metadata);
                    let _ = metadata.entry(name.clone()).or_default();
                    capture = Some(Capture::Fields(name));
                    continue;
                }
            }
        }

        if !trimmed.is_empty() && !line.starts_with('-') {
            capture = None;
        }
        if let (Some(kind), Some(item)) = (&capture, trimmed.strip_prefix('-')) {
            let value = inline_codes(item)
                .into_iter()
                .next()
                .unwrap_or_else(|| item.trim_matches('-').trim().to_owned());
            if value.is_empty() {
                continue;
            }
            match kind {
                Capture::Fields(name) => {
                    metadata.entry(name.clone()).or_default().fields.push(value)
                }
                Capture::LabelValues(name, label) => {
                    metadata
                        .entry(name.clone())
                        .or_default()
                        .label_values
                        .entry(label.clone())
                        .or_default()
                        .push(value);
                }
            }
        }
    }

    for metric in &mut metrics {
        if let Some(meta) = metadata.get(&metric.name) {
            metric.labels.clone_from(&meta.labels);
            metric.fields.clone_from(&meta.fields);
            metric.label_values.clone_from(&meta.label_values);
            dedupe(&mut metric.labels);
            dedupe(&mut metric.fields);
            for values in metric.label_values.values_mut() {
                dedupe(values);
            }
        }
    }
    Schema {
        version: "1.0.0".to_owned(),
        source_file: String::new(),
        metrics,
        groups,
    }
}

fn key_value_fields(input: &str) -> Vec<String> {
    input
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter_map(|(key, _)| (!key.trim().is_empty()).then(|| to_snake_case(key.trim())))
        .collect()
}

pub fn proc_pair_fields(input: &str) -> Vec<String> {
    let mut pending = HashMap::<&str, Vec<&str>>::new();
    let mut fields = Vec::new();
    for line in input.lines() {
        let Some((section, rest)) = line.trim().split_once(':') else {
            continue;
        };
        let columns = rest.split_whitespace().collect::<Vec<_>>();
        if section.is_empty() || columns.is_empty() {
            continue;
        }
        if let Some(labels) = pending.remove(section) {
            if labels.len() == columns.len() {
                fields.extend(
                    labels
                        .into_iter()
                        .map(|label| kernel_counter_field_name(section, label)),
                );
            }
        } else if !columns.iter().all(|value| value.parse::<i128>().is_ok()) {
            let _ = pending.insert(section, columns);
        }
    }
    fields
}

fn read_or_empty(path: &str) -> io::Result<String> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(value),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error),
    }
}

const SOFTNET_FIELDS: &[&str] = &[
    "softnet_cpu_index",
    "softnet_processed_counter",
    "softnet_dropped_counter",
    "softnet_time_squeeze_counter",
    "softnet_received_rps_counter",
    "softnet_flow_limit_count_counter",
    "softnet_backlog_len_total",
    "softnet_input_qlen",
    "softnet_process_qlen",
];
const DISKSTATS_FIELDS: &[&str] = &[
    "reads",
    "reads_merged",
    "sectors_read",
    "time_reading_ms",
    "writes",
    "writes_merged",
    "sectors_written",
    "time_writing_ms",
    "in_progress",
    "time_in_progress_ms",
    "weighted_time_in_progress_ms",
    "discards",
    "discards_merged",
    "sectors_discarded",
    "time_discarding_ms",
    "flushes",
    "time_flushing_ms",
];
const NETDEV_FIELDS: &[&str] = &[
    "recv_bytes",
    "recv_packets",
    "recv_errs",
    "recv_drop",
    "recv_fifo",
    "recv_frame",
    "recv_compressed",
    "recv_multicast",
    "sent_bytes",
    "sent_packets",
    "sent_errs",
    "sent_drop",
    "sent_fifo",
    "sent_colls",
    "sent_carrier",
    "sent_compressed",
];

fn append_runtime_fields(schema: &mut Schema) -> io::Result<()> {
    let runtime = [
        (
            "meminfo",
            key_value_fields(&read_or_empty("/proc/meminfo")?),
        ),
        ("vmstat", key_value_fields(&read_or_empty("/proc/vmstat")?)),
        (
            "diskstats",
            DISKSTATS_FIELDS.iter().map(|s| (*s).to_owned()).collect(),
        ),
        (
            "netdev",
            NETDEV_FIELDS.iter().map(|s| (*s).to_owned()).collect(),
        ),
        ("snmp", proc_pair_fields(&read_or_empty("/proc/net/snmp")?)),
        (
            "netstat",
            proc_pair_fields(&read_or_empty("/proc/net/netstat")?),
        ),
        (
            "softnet",
            SOFTNET_FIELDS.iter().map(|s| (*s).to_owned()).collect(),
        ),
    ];
    for (name, fields) in runtime {
        if let Some(metric) = schema.metrics.iter_mut().find(|metric| metric.name == name) {
            if !fields.is_empty() {
                metric.fields_detected = fields;
                dedupe(&mut metric.fields_detected);
            }
        }
    }
    Ok(())
}

pub fn generate(source: &Path, runtime: bool) -> io::Result<Schema> {
    let mut schema = parse_markdown(&fs::read_to_string(source)?);
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let abs = source.canonicalize()?;
    schema.source_file = abs.strip_prefix(root).map_or_else(
        |_| abs.display().to_string(),
        |path| path.display().to_string(),
    );
    if runtime {
        append_runtime_fields(&mut schema)?;
    }
    Ok(schema)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_matches_exporter() {
        let cases = [
            ("MemTotal", "mem_total"),
            ("Active(anon)", "active_anon"),
            ("SyncookiesSent", "syncookies_sent"),
            ("IcmpMsg", "icmp_msg"),
            ("InType0", "in_type_0"),
            ("MPTcpExt", "mp_tcp_ext"),
        ];
        for (source, expected) in cases {
            assert_eq!(to_snake_case(source), expected);
        }
        assert_eq!(kernel_counter_field_name("Ip", "ReasmOKs"), "ip_reasm_oks");
        assert_eq!(kernel_counter_field_name("Ip", "FragOKs"), "ip_frag_oks");
    }

    #[test]
    fn pair_rows_handle_missing_and_new_sections() {
        let text =
            "Tcp: MaxConn ActiveOpens\nTcp: -1 42\nIcmpMsg: InType0 OutType3\nIcmpMsg: 11 22\n";
        assert_eq!(
            proc_pair_fields(text),
            [
                "tcp_max_conn",
                "tcp_active_opens",
                "icmp_msg_in_type_0",
                "icmp_msg_out_type_3"
            ]
        );
    }

    #[test]
    fn documented_label_headings() {
        let text = "## Core\n\n| Metric | Type | Description |\n|---|---|---|\n| \x60sample_metric\x60 | GaugeVec | sample |\n\n## Metric labels and field catalogs\n\n### sample_metric labels: \x60collector\x60\n";
        let schema = parse_markdown(text);
        assert_eq!(schema.metrics[0].labels, ["collector"]);
    }

    #[test]
    fn documented_kernel_field_labels_use_the_field_label() {
        let text = "## procfs\n\n| Metric | Type | Description |\n|---|---|---|\n| \x60meminfo\x60 | GaugeVec | memory |\n\n## Metric labels and field catalogs\n\n\x60meminfo\x60 label values (\x60meminfo\x60 metric \x60field\x60):\n- \x60mem_total\x60\n";
        let schema = parse_markdown(text);
        assert_eq!(
            schema.metrics[0]
                .label_values
                .get("field")
                .and_then(|v| v.first())
                .map(String::as_str),
            Some("mem_total")
        );
        assert!(!schema.metrics[0].label_values.contains_key("meminfo"));
    }

    #[test]
    fn complete_repository_schema() -> io::Result<()> {
        let schema = generate(
            Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../METRICS.md")),
            false,
        )?;
        assert_eq!(schema.source_file, "METRICS.md");
        assert_eq!(
            schema
                .metrics
                .iter()
                .find(|m| m.name == "node_scrape_collector_success")
                .and_then(|m| m.labels.first())
                .map(String::as_str),
            Some("collector")
        );
        assert!(schema.metrics.len() > 200);
        Ok(())
    }
}
