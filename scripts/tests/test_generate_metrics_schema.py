from __future__ import annotations

import argparse
import json
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))

import generate_metrics_schema as schema


class GenerateMetricsSchemaTests(unittest.TestCase):
    def test_snake_case_matches_rust_exporter(self) -> None:
        cases = {
            "MemTotal": "mem_total",
            "Active(anon)": "active_anon",
            "SyncookiesSent": "syncookies_sent",
            "IcmpMsg": "icmp_msg",
            "InType0": "in_type_0",
            "MPTcpExt": "mp_tcp_ext",
        }
        for source, expected in cases.items():
            with self.subTest(source=source):
                self.assertEqual(schema._to_snake_case(source), expected)

        self.assertEqual(
            schema._kernel_counter_field_name("Ip", "ReasmOKs"), "ip_reasm_oks"
        )
        self.assertEqual(
            schema._kernel_counter_field_name("Ip", "FragOKs"), "ip_frag_oks"
        )

    def test_pair_file_allows_missing_sections_and_discovers_new_ones(self) -> None:
        contents = (
            "Tcp: MaxConn ActiveOpens\n"
            "Tcp: -1 42\n"
            "IcmpMsg: InType0 OutType3\n"
            "IcmpMsg: 11 22\n"
        )
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "snmp"
            path.write_text(contents, encoding="utf-8")
            self.assertEqual(
                schema._fields_from_proc_net_pair_file(str(path)),
                [
                    "tcp_max_conn",
                    "tcp_active_opens",
                    "icmp_msg_in_type_0",
                    "icmp_msg_out_type_3",
                ],
            )

    def test_documented_heading_label_syntax_is_parsed(self) -> None:
        markdown = """## Core

| Metric | Type | Description |
|---|---|---|
| `sample_metric` | GaugeVec | sample |

## Metric labels and field catalogs

### sample_metric labels: `collector`
"""
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "METRICS.md"
            path.write_text(markdown, encoding="utf-8")
            groups, metadata = schema._parse_markdown(path)
            self.assertIn("Core", groups)
            self.assertEqual(metadata["sample_metric"]["labels"], ["collector"])

    def test_repo_source_path_is_portable(self) -> None:
        args = argparse.Namespace(with_runtime_fields=False)
        generated = schema.generate_schema(schema.PROJECT_ROOT / "METRICS.md", args)
        self.assertEqual(generated["source_file"], "METRICS.md")

    def test_generated_repo_schema_has_collector_labels(self) -> None:
        args = argparse.Namespace(with_runtime_fields=False)
        generated = schema.generate_schema(schema.PROJECT_ROOT / "METRICS.md", args)
        by_name = {metric["name"]: metric for metric in generated["metrics"]}
        self.assertEqual(
            by_name["node_scrape_collector_success"]["labels"], ["collector"]
        )
        # Ensure the result can be serialized without custom encoders.
        json.dumps(generated)


if __name__ == "__main__":
    unittest.main()
