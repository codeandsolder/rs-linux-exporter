#!/usr/bin/env python3
"""Generate Grafana panel JSON (paste panel format) from `METRICS.schema.json`."""

from __future__ import annotations

import argparse
import json
import re
from collections.abc import Iterable
from pathlib import Path
from typing import Any

PROJECT_ROOT = Path(__file__).resolve().parent.parent
COUNTER_RATE_WINDOW_DEFAULT = "5m"
COUNTER_RATE_METRICS = {
    "conntrack",
    "diskstats",
    "netdev",
    "netstat",
    "rapl_energy_joules",
    "snmp",
    "softnet",
    "vmstat",
}
BYTE_FIELD_TOKENS = ("byte", "bytes", "octet")


def build_panel(
    title: str,
    metrics: Iterable[dict[str, Any]],
    datasource_ref: str,
    instance_var: str,
    datasource_type: str = "prometheus",
    job_var: str | None = None,
    extra_filters: dict[str, str] | None = None,
    rate_window: str = COUNTER_RATE_WINDOW_DEFAULT,
    disable_rate: bool = False,
) -> dict[str, Any]:
    def _metric_labels(metric: dict[str, Any]) -> list[str]:
        labels = metric.get("labels", [])
        if isinstance(labels, list):
            return [
                str(label)
                for label in labels
                if isinstance(label, str) and label and not label[0].isdigit()
            ]
        return []

    def _metric_fields(metric: dict[str, Any]) -> list[str]:
        fields = metric.get("fields")
        if fields is None:
            fields = metric.get("fields_detected")
        if isinstance(fields, list):
            return [str(field) for field in fields if isinstance(field, str)]
        return []

    def _is_rate_metric(metric: dict[str, Any]) -> bool:
        if disable_rate:
            return False
        name = str(metric.get("name", ""))
        metric_type = str(metric.get("type", ""))
        if name.endswith("_total"):
            return True
        if metric_type.startswith("Counter"):
            return True
        return name in COUNTER_RATE_METRICS

    def _byte_fields(fields: list[str]) -> list[str]:
        return sorted(
            {
                field
                for field in fields
                if any(token in field.lower() for token in BYTE_FIELD_TOKENS)
            }
        )

    def _regex(values: list[str]) -> str:
        if not values:
            return ""
        return "^(" + "|".join(re.escape(value) for value in values) + ")$"

    def _selector(metric_name: str, extra_labels: list[str] | None = None) -> str:
        parts = [f'__name__="{metric_name}"']
        if instance_var:
            parts.append(f'instance=~"${{{instance_var}}}"')
        if job_var:
            parts.append(f'job=~"${{{job_var}}}"')
        if extra_filters:
            for key, var_name in sorted(extra_filters.items()):
                parts.append(f'{key}=~"${{{var_name}}}"')
        if extra_labels:
            parts.extend(extra_labels)
        return "{" + ",".join(parts) + "}"

    def _legend(metric: dict[str, Any]) -> str:
        labels = _metric_labels(metric)
        labels_display = ["{{__name__}}"]
        for label in labels:
            labels_display.append("{{" + label + "}}")
        return " ".join(labels_display)

    def _make_target(ref_id: str, expression: str, legend: str) -> dict[str, Any]:
        return {
            "refId": ref_id,
            "datasource": {"type": datasource_type, "uid": datasource_ref},
            "expr": expression,
            "legendFormat": legend,
            "range": True,
            "instant": False,
            "editorMode": "code",
        }

    def _ref_id(index: int) -> str:
        if index < 26:
            return chr(ord("A") + index)
        return f"A{index - 25}"

    targets: list[dict[str, Any]] = []
    ref_index = 0
    for metric in sorted(metrics, key=lambda item: str(item.get("name", ""))):
        name = str(metric.get("name", ""))
        if not name:
            continue
        selector = _selector(name)
        legend = _legend(metric)

        if _is_rate_metric(metric):
            labels = _metric_labels(metric)
            fields = _metric_fields(metric)
            has_field_label = "field" in labels
            if not has_field_label and fields:
                has_field_label = True
            if has_field_label and fields:
                byte_fields = _byte_fields(fields)
                if byte_fields:
                    byte_regex = _regex(byte_fields)
                    byte_selector = selector[:-1] + f',field=~"{byte_regex}"}}'
                    byte_expr = f"rate({byte_selector}[{rate_window}]) * 8"
                    targets.append(
                        _make_target(
                            _ref_id(ref_index), byte_expr, f"{legend} (bits/s)"
                        )
                    )
                    ref_index += 1

                    if set(fields) != set(byte_fields):
                        non_byte_selector = (
                            selector[:-1] + ',field!~"' + byte_regex + '"}'
                        )
                        non_byte_expr = f"rate({non_byte_selector}[{rate_window}])"
                        targets.append(
                            _make_target(_ref_id(ref_index), non_byte_expr, legend)
                        )
                        ref_index += 1
                    continue

            targets.append(
                _make_target(
                    _ref_id(ref_index), f"rate({selector}[{rate_window}])", legend
                )
            )
            ref_index += 1
        else:
            targets.append(_make_target(_ref_id(ref_index), selector, legend))
            ref_index += 1

    return {
        "type": "timeseries",
        "title": title,
        "datasource": {"type": datasource_type, "uid": datasource_ref},
        "targets": targets,
        "fieldConfig": {
            "defaults": {
                "color": {"mode": "palette-classic"},
                "mappings": [],
                "thresholds": {
                    "mode": "absolute",
                    "steps": [{"color": "green", "value": None}],
                },
                "unit": "short",
            },
            "overrides": [],
        },
        "options": {
            "legend": {
                "displayMode": "list",
                "placement": "bottom",
                "calcs": ["lastNotNull", "mean"],
            },
            "tooltip": {"mode": "single"},
        },
    }


def _label_selector(job_var: str | None, metric_selector: str = ".+") -> str:
    labels = [f'__name__=~"{metric_selector}"']
    if job_var:
        labels.append(f'job=~"${{{job_var}}}"')
    return "{" + ",".join(labels) + "}"


def build_panel_template_vars(
    datasource_ref: str,
    datasource_type: str,
    instance_var: str,
    job_var: str | None,
) -> list[dict[str, Any]]:
    vars_list: list[dict[str, Any]] = []
    selector = _label_selector(job_var)

    if job_var:
        vars_list.append(
            {
                "type": "query",
                "name": job_var,
                "label": job_var,
                "description": "",
                "hide": 0,
                "query": 'label_values({__name__=~".+"}, job)',
                "datasource": {"type": datasource_type, "uid": datasource_ref},
                "pluginId": "prometheus",
                "pluginName": "Prometheus",
                "current": {"selected": True, "text": "All", "value": "$__all"},
                "includeAll": True,
                "allValue": "",
                "multi": True,
                "refresh": 1,
                "regex": "",
                "sort": 1,
                "skipUrlSync": False,
            }
        )

    vars_list.append(
        {
            "type": "query",
            "name": instance_var,
            "label": instance_var,
            "description": "",
            "hide": 0,
            "query": f"label_values({selector}, instance)",
            "datasource": {"type": datasource_type, "uid": datasource_ref},
            "pluginId": "prometheus",
            "pluginName": "Prometheus",
            "current": {"selected": True, "text": "All", "value": "$__all"},
            "includeAll": True,
            "allValue": "",
            "multi": True,
            "refresh": 2,
            "regex": "",
            "sort": 1,
            "skipUrlSync": False,
            "definition": f"label_values({selector}, instance)",
            "hideLabel": True,
        }
    )
    return vars_list


def layout_panels(panels: list[dict[str, Any]], width: int, height: int) -> None:
    width = min(width, 24)
    if width <= 0:
        width = 12
    if height <= 0:
        height = 8

    x = 0
    y = 0
    for panel in panels:
        panel["gridPos"] = {"h": height, "w": width, "x": x, "y": y}
        x += width
        if x >= 24:
            x = 0
            y += height


def build_dashboard(
    title: str,
    panels: list[dict[str, Any]],
    datasource_ref: str,
    datasource_type: str,
    instance_var: str,
    job_var: str | None,
    time_from: str,
    time_to: str,
    refresh: str,
    panel_width: int = 12,
    panel_height: int = 8,
) -> dict[str, Any]:
    for idx, panel in enumerate(panels, start=1):
        panel.setdefault("id", idx)

    layout_panels(panels, panel_width, panel_height)

    return {
        "annotations": {
            "list": [
                {
                    "builtIn": 1,
                    "datasource": {"type": "grafana", "uid": "-- Grafana --"},
                    "enable": True,
                    "hide": True,
                    "iconColor": "rgba(0, 211, 255, 1)",
                    "name": "Annotations & Alerts",
                    "type": "dashboard",
                }
            ]
        },
        "editable": True,
        "fiscalYearStartMonth": 0,
        "graphTooltip": 0,
        "id": 1,
        "links": [],
        "panels": panels,
        "preload": False,
        "refresh": refresh,
        "schemaVersion": 42,
        "tags": [],
        "templating": {
            "list": build_panel_template_vars(
                datasource_ref, datasource_type, instance_var, job_var
            )
        },
        "time": {
            "from": time_from,
            "to": time_to,
        },
        "timepicker": {},
        "timezone": "browser",
        "title": title,
        "uid": "",
        "version": 1,
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="python3 scripts/generate_grafana_panel.py",
        description="Generate Grafana panel JSON (paste panel or dashboard) from METRICS.schema.json.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "Examples:\n\n"
            "Most used:\n"
            "python3 scripts/generate_grafana_panel.py --all --dashboard --datasource VictoriaMetrics --datasource-literal --instance-var instance --output dashboard.json --pretty\n\n"
            "1) Print a procfs section panel for Grafana paste:\n"
            "   python3 scripts/generate_grafana_panel.py --section procfs --datasource DS_PROMETHEUS --instance-var instance\n\n"
            "2) Print one panel per section (JSON array):\n"
            "   python3 scripts/generate_grafana_panel.py --all --format array --datasource DS_PROMETHEUS --instance-var instance\n\n"
            "3) Generate a single section panel to a file (for templating):\n"
            "   python3 scripts/generate_grafana_panel.py --section netdev_sysfs --datasource DS_PROMETHEUS --instance-var instance --output out.json\n"
            "4) Generate importable dashboard JSON:\n"
            '   python3 scripts/generate_grafana_panel.py --all --dashboard --datasource DS_PROMETHEUS --instance-var instance --dashboard-title "Linux Exporter" --output dashboard.json\n'
        ),
    )
    parser.add_argument(
        "--schema",
        default=str(PROJECT_ROOT / "METRICS.schema.json"),
        help="Path to schema JSON",
    )
    parser.add_argument(
        "--datasource",
        default="DS_PROMETHEUS",
        help="Grafana datasource UID variable name",
    )
    parser.add_argument(
        "--datasource-type", default="prometheus", help="Grafana datasource type"
    )
    parser.add_argument(
        "--instance-var",
        default="instance",
        help="Grafana variable for dynamic instance selector",
    )
    parser.add_argument(
        "--job-var", default="", help="Optional Grafana variable for job filtering"
    )
    parser.add_argument(
        "--section",
        action="append",
        help="Metric group name from METRICS.md (repeatable)",
    )
    parser.add_argument(
        "--metric", action="append", help="Single metric override (repeatable)"
    )
    parser.add_argument(
        "--metric-regex",
        help="Optional regex to filter metrics from the selected sections",
    )
    parser.add_argument(
        "--list", action="store_true", help="List available sections and metric counts"
    )
    parser.add_argument(
        "--all",
        action="store_true",
        help="Generate panel for each section in one JSON array",
    )
    parser.add_argument(
        "--dashboard",
        action="store_true",
        help="Generate full Grafana dashboard JSON with templating",
    )
    parser.add_argument(
        "--dashboard-title",
        default="Linux Exporter Metrics",
        help="Dashboard title when --dashboard is used",
    )
    parser.add_argument(
        "--time-from", default="now-1h", help="Dashboard default time range start"
    )
    parser.add_argument(
        "--time-to", default="now", help="Dashboard default time range end"
    )
    parser.add_argument(
        "--dashboard-refresh",
        default="",
        help="Dashboard refresh interval, e.g. 30s or 1m",
    )
    parser.add_argument(
        "--rate-window",
        default=COUNTER_RATE_WINDOW_DEFAULT,
        help="PromQL range for auto rate() conversion",
    )
    parser.add_argument(
        "--disable-auto-rate",
        action="store_true",
        help="Disable automatic rate() conversion for counters",
    )
    parser.add_argument(
        "--datasource-literal",
        action="store_true",
        help="Use datasource argument as a literal uid/name (skip wrapping as ${...})",
    )
    parser.add_argument(
        "--panel-width", type=int, default=12, help="Dashboard grid panel width"
    )
    parser.add_argument(
        "--panel-height", type=int, default=8, help="Dashboard grid panel height"
    )
    parser.add_argument(
        "--format",
        default="paste",
        choices=("paste", "array", "ndjson"),
        help="paste=single object, array=JSON array, ndjson=one panel per line",
    )
    parser.add_argument(
        "--title-prefix", default="", help="Optional prefix for panel titles"
    )
    parser.add_argument("--output", help="Write output to file instead of stdout")
    parser.add_argument(
        "--pretty",
        action="store_true",
        help="Pretty print JSON output (no effect on NDJSON mode)",
    )
    return parser


def parse_args() -> argparse.Namespace:
    return build_parser().parse_args()


def load_schema(path: Path) -> dict[str, Any]:
    with open(path, encoding="utf-8") as fp:
        return json.load(fp)


def find_section_metrics(
    schema: dict[str, Any],
    sections: list[str],
    metrics: list[str],
    metric_regex: str | None,
) -> dict[str, list[dict[str, Any]]]:
    selected: dict[str, list[dict[str, Any]]] = {}
    regex = re.compile(metric_regex) if metric_regex else None

    for entry in schema.get("metrics", []):
        group = str(entry.get("group", ""))
        name = str(entry.get("name", ""))

        if sections and group not in sections:
            continue
        if metrics and name not in metrics:
            continue
        if regex and not regex.search(name):
            continue

        selected.setdefault(group, []).append(entry)

    return selected


def section_metrics_map(
    schema: dict[str, Any], group_names: list[str]
) -> dict[str, list[dict[str, Any]]]:
    metrics_map: dict[str, list[dict[str, Any]]] = {}
    for entry in schema.get("metrics", []):
        group = str(entry.get("group", ""))
        if group in group_names:
            metrics_map.setdefault(group, []).append(entry)
    return metrics_map


def list_sections(schema: dict[str, Any]) -> str:
    counts: dict[str, int] = {}
    for entry in schema.get("metrics", []):
        group = str(entry.get("group", ""))
        counts[group] = counts.get(group, 0) + 1

    lines = ["Available groups:"]
    for group in sorted(counts):
        lines.append(f"- {group}: {counts[group]}")
    return "\n".join(lines)


def show_help_if_no_selector(
    args: argparse.Namespace, parser: argparse.ArgumentParser
) -> None:
    if args.list:
        return
    if args.section or args.metric or args.all or args.metric_regex:
        return

    parser.print_help()
    print("\nNo section or metric selector provided.")
    print("Use --help for full options, or:\n")
    print(
        "  python3 scripts/generate_grafana_panel.py --section procfs --datasource DS_PROMETHEUS --instance-var instance"
    )
    print(
        "  python3 scripts/generate_grafana_panel.py --all --format array --datasource DS_PROMETHEUS --instance-var instance"
    )
    raise SystemExit(0)


def render_output(data: Any, fmt: str, pretty: bool) -> str:
    if fmt == "ndjson":
        if isinstance(data, list):
            return "\n".join(json.dumps(panel, separators=(",", ":")) for panel in data)
        return json.dumps(data, separators=(",", ":"))

    if fmt == "array":
        if pretty:
            return json.dumps(data, indent=2)
        return json.dumps(data, separators=(",", ":"))

    if pretty:
        return json.dumps(data, indent=2)
    return json.dumps(data, separators=(",", ":"))


def main() -> None:
    parser = build_parser()
    args = parser.parse_args()
    show_help_if_no_selector(args, parser)

    schema_path = Path(args.schema)
    if not schema_path.exists():
        raise SystemExit(
            f"Schema file not found: {schema_path}\n"
            "Generate it first:\n"
            "python3 scripts/generate_metrics_schema.py --with-runtime-fields --output METRICS.schema.json --pretty"
        )

    schema = load_schema(schema_path)

    if args.list:
        output = list_sections(schema)
        print(output)
        return

    datasource_ref = (
        args.datasource if args.datasource_literal else f"${{{args.datasource}}}"
    )

    schema_sections = [g.get("name") for g in schema.get("groups", [])]

    if args.section:
        sections = args.section
        unknown = [s for s in sections if s not in schema_sections]
        if unknown:
            raise SystemExit(f"Unknown section(s): {', '.join(unknown)}")
        selected = find_section_metrics(
            schema, sections, args.metric or [], args.metric_regex
        )
    else:
        selected = section_metrics_map(schema, schema_sections)
        if args.metric or args.metric_regex:
            selected = find_section_metrics(
                schema, [], args.metric or [], args.metric_regex
            )

    if not selected:
        raise SystemExit("No metrics matched the selection")

    if not args.all and len(selected) > 1 and len(selected.keys()) > 1:
        print(
            "Multiple sections selected; use --all or --section to return one section only."
        )
        raise SystemExit(0)

    panels: list[dict[str, Any]] = []
    for section_name, metric_names in sorted(selected.items()):
        title = section_name
        if args.title_prefix:
            title = f"{args.title_prefix} {title}"
        panels.append(
            build_panel(
                title=title,
                metrics=metric_names,
                datasource_ref=datasource_ref,
                instance_var=args.instance_var,
                job_var=args.job_var or None,
                datasource_type=args.datasource_type,
                rate_window=args.rate_window,
                disable_rate=args.disable_auto_rate,
            )
        )

    if args.dashboard:
        payload = build_dashboard(
            title=args.dashboard_title,
            panels=panels,
            datasource_ref=datasource_ref,
            datasource_type=args.datasource_type,
            instance_var=args.instance_var,
            job_var=args.job_var or None,
            time_from=args.time_from,
            time_to=args.time_to,
            refresh=args.dashboard_refresh,
            panel_width=args.panel_width,
            panel_height=args.panel_height,
        )
    elif args.format == "array":
        payload: Any = panels if args.all else panels[0]
    elif args.format == "ndjson":
        payload = panels if args.all else panels[0]
    else:
        payload = panels[0]

    output = render_output(payload, args.format, args.pretty)
    if args.output:
        Path(args.output).write_text(output, encoding="utf-8")
    else:
        print(output)


if __name__ == "__main__":
    main()
