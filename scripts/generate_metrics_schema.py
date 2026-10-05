#!/usr/bin/env python3
"""Build a machine-readable schema from `METRICS.md`.

The script keeps `METRICS.md` as the single source of truth for metric names,
types and docs while optionally refreshing raw family fields from the running
host.
"""

from __future__ import annotations

import argparse
import json
import re
from collections import OrderedDict
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent


SOFTNET_FIELDS = [
    "softnet_cpu_index",
    "softnet_processed_counter",
    "softnet_dropped_counter",
    "softnet_time_squeeze_counter",
    "softnet_received_rps_counter",
    "softnet_flow_limit_count_counter",
    "softnet_backlog_len_total",
    "softnet_input_qlen",
    "softnet_process_qlen",
]

DISKSTATS_FIELDS = [
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
]

NETDEV_FIELDS = [
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
]


def _parse_table_row(line: str) -> tuple[str, str, str] | None:
    if not line.startswith("|"):
        return None
    cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
    if len(cells) < 3:
        return None
    metric = cells[0].strip().strip("`")
    if re.search(r"\s", metric):
        return None
    metric_type = cells[1].strip()
    description = " | ".join(cells[2:]).strip()
    if not metric or not metric_type:
        return None
    return metric, metric_type, description


def _resolve_metric_name(metric: str, known_names: set[str]) -> str:
    if metric in known_names:
        return metric
    if metric in {"netstat", "ip_ext", "tcp_ext", "mptcp_ext"}:
        return "netstat"
    if metric.startswith(("tcp_ext_", "ip_ext_", "mptcp_ext_")):
        return "netstat"
    return metric


def _parse_markdown(path: Path) -> tuple[OrderedDict[str, list[dict]], dict[str, dict]]:
    text = path.read_text(encoding="utf-8").splitlines()

    groups: OrderedDict[str, list[dict]] = OrderedDict()
    metadata: dict[str, dict] = {}

    in_group = False
    in_labels_section = False
    in_table = False
    capture: dict | None = None
    current_group = ""

    for raw_line in text:
        line = raw_line.rstrip()

        heading2 = re.match(r"^##\s+(.*)$", line)
        if heading2:
            title = heading2.group(1).strip()
            in_table = False
            capture = None

            if title == "Metric labels and field catalogs":
                in_labels_section = True
                in_group = False
                continue

            if title.startswith("TODO"):
                in_labels_section = False
                in_group = False
                current_group = title
                continue

            if title in groups:
                pass
            groups.setdefault(title, [])
            current_group = title
            in_labels_section = False
            in_group = True
            continue

        if not in_labels_section and in_group:
            if re.match(
                r"^\|\s*Metric\s*\|\s*Type\s*\|\s*Description\s*\|$", line.strip()
            ):
                in_table = True
                continue
            if in_table:
                if re.match(r"^\|\s*-+\s*\|", line.strip()):
                    continue
                if line.startswith("|"):
                    parsed = _parse_table_row(line)
                    if parsed:
                        name, metric_type, description = parsed
                        groups[current_group].append(
                            {
                                "name": name,
                                "type": metric_type,
                                "description": description,
                                "group": current_group,
                            }
                        )
                        metadata.setdefault(
                            name, {"labels": [], "label_values": {}, "fields": []}
                        )
                        continue
                in_table = False
            continue

        if not in_labels_section:
            continue

        heading3 = re.match(r"^###\s+(.*)$", line)
        if heading3:
            capture = None
            continue

        direct = re.match(r"^`(?P<metric>[^`]+)`\s*:\s*(?P<body>.+)$", line.strip())
        if direct:
            metric = _resolve_metric_name(direct.group("metric"), set(metadata))
            body = direct.group("body")
            labels = re.findall(r"`([^`]+)`", body)
            if labels:
                metadata.setdefault(
                    metric, {"labels": [], "label_values": {}, "fields": []}
                )["labels"] = labels
            capture = None
            continue

        list_header = re.match(
            r"^`(?P<metric>[^`]+)`\s+label values(?:\s*\(`(?P<label>[^`]+)`\))?:$",
            line.strip(),
        )
        if list_header:
            metric = _resolve_metric_name(list_header.group("metric"), set(metadata))
            label = list_header.group("label") or "value"
            metadata.setdefault(
                metric, {"labels": [], "label_values": {}, "fields": []}
            )
            capture = {
                "metric": metric,
                "kind": "label_values",
                "label": label,
            }
            continue

        field_list_header = re.match(
            r"^`(?P<metric>[^`]+)`\s+field values (?:include|are generated from\s+`/proc`.*):$",
            line.strip(),
        )
        if field_list_header:
            metric = _resolve_metric_name(
                field_list_header.group("metric"), set(metadata)
            )
            metadata.setdefault(
                metric, {"labels": [], "label_values": {}, "fields": []}
            )
            capture = {"metric": metric, "kind": "fields", "label": None}
            continue

        if line.strip() and not line.startswith("-") and capture is not None:
            # Stop capturing list mode at the next non-list content.
            capture = None

        if capture and line.strip().startswith("-"):
            value_match = re.findall(r"`([^`]+)`", line)
            value = value_match[0] if value_match else line.strip("- ").strip()
            if not value:
                continue

            target = metadata.setdefault(
                capture["metric"], {"labels": [], "label_values": {}, "fields": []}
            )
            if capture["kind"] == "label_values":
                values = target.setdefault("label_values", {}).setdefault(
                    capture["label"], []
                )
                if value not in values:
                    values.append(value)
            else:
                if value not in target["fields"]:
                    target["fields"].append(value)

    return groups, metadata


def _fields_from_key_value_file(path: str, prefix: str = "") -> list[str]:
    try:
        lines = Path(path).read_text(encoding="utf-8", errors="ignore").splitlines()
    except FileNotFoundError:
        return []

    fields: list[str] = []
    for line in lines:
        if ":" not in line:
            continue
        name = line.split(":", 1)[0].strip()
        if not name:
            continue
        key = f"{prefix}_{name.lower()}" if prefix else name.lower()
        fields.append(key)
    return fields


def _fields_from_proc_net_pair_file(path: str, prefix_mode: str) -> list[str]:
    try:
        lines = Path(path).read_text(encoding="utf-8", errors="ignore").splitlines()
    except FileNotFoundError:
        return []

    fields: list[str] = []
    for i in range(0, len(lines) - 1, 2):
        line_a = lines[i].strip()
        line_b = lines[i + 1].strip()
        if ":" not in line_a or ":" not in line_b:
            continue
        section_a, rest_a = line_a.split(":", 1)
        section_b, _ = line_b.split(":", 1)
        if section_a != section_b:
            continue
        labels = rest_a.split()
        if not labels:
            continue
        if prefix_mode == "netstat":
            section = section_a.lower()
            if section.endswith("ext"):
                section = section[:-3] + "_ext"
            elif section.startswith("mptcp"):
                section = "mptcp_ext"
        else:
            section = section_a.lower()
        fields.extend([f"{section}_{name.lower()}" for name in labels])
    return fields


def _append_runtime_fields(schema: dict, args: argparse.Namespace) -> None:
    if not args.with_runtime_fields:
        return

    runtime_fields = {
        "meminfo": _fields_from_key_value_file("/proc/meminfo", prefix=""),
        "vmstat": _fields_from_key_value_file("/proc/vmstat", prefix=""),
        "diskstats": DISKSTATS_FIELDS,
        "netdev": NETDEV_FIELDS,
        "snmp": _fields_from_proc_net_pair_file("/proc/net/snmp", "snmp"),
        "netstat": _fields_from_proc_net_pair_file("/proc/net/netstat", "netstat"),
        "softnet": SOFTNET_FIELDS,
    }

    for metric, discovered_fields in runtime_fields.items():
        for item in schema["metrics"]:
            if item["name"] != metric:
                continue
            if discovered_fields:
                item["fields_detected"] = discovered_fields
            break


def _dedupe_fields(metrics: list[dict]) -> None:
    for metric in metrics:
        if "fields" in metric:
            metric["fields"] = list(dict.fromkeys(metric["fields"]))
        if "labels" in metric:
            metric["labels"] = list(dict.fromkeys(metric["labels"]))
        if "fields_detected" in metric:
            seen = dict.fromkeys(metric["fields_detected"])
            metric["fields_detected"] = list(seen.keys())
        if "label_values" in metric:
            cleaned = {}
            for label_name, values in metric["label_values"].items():
                cleaned[label_name] = list(dict.fromkeys(values))
            metric["label_values"] = cleaned


def generate_schema(markdown_path: Path, args: argparse.Namespace) -> dict:
    groups, metadata = _parse_markdown(markdown_path)
    metrics: list[dict] = []

    for group_name, group_metrics in groups.items():
        for metric in group_metrics:
            metric_meta = metadata.get(metric["name"], {})
            metrics.append(
                {
                    "name": metric["name"],
                    "group": group_name,
                    "type": metric["type"],
                    "description": metric["description"],
                    "labels": metric_meta.get("labels", []),
                    **(
                        {"label_values": metric_meta["label_values"]}
                        if metric_meta.get("label_values")
                        else {}
                    ),
                    **(
                        {"fields": metric_meta.get("fields", [])}
                        if metric_meta.get("fields")
                        else {}
                    ),
                }
            )

    schema = {
        "version": "1.0.0",
        "source_file": str(markdown_path),
        "metrics": metrics,
        "groups": [
            {
                "name": group_name,
                "metrics": [metric["name"] for metric in group_metrics],
            }
            for group_name, group_metrics in groups.items()
        ],
    }

    _append_runtime_fields(schema, args)
    _dedupe_fields(schema["metrics"])
    return schema


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate JSON schema from METRICS.md")
    parser.add_argument(
        "--source",
        default=str(PROJECT_ROOT / "METRICS.md"),
        help="Path to METRICS.md",
    )
    parser.add_argument("--output", help="Write schema JSON to file")
    parser.add_argument(
        "--with-runtime-fields",
        action="store_true",
        help="Refresh fields from /proc for raw metric families (vmstat, snmp, netstat, etc)",
    )
    parser.add_argument(
        "--pretty",
        action="store_true",
        help="Pretty-print schema JSON for easier diffing",
    )

    args = parser.parse_args()
    markdown_path = Path(args.source)
    schema = generate_schema(markdown_path, args)
    output_json = json.dumps(schema, indent=2 if args.pretty else None, sort_keys=False)

    if args.output:
        Path(args.output).write_text(output_json, encoding="utf-8")
        return

    print(output_json)


if __name__ == "__main__":
    main()
