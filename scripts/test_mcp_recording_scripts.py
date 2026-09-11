#!/usr/bin/env python3
"""Unit tests for the standard-library MCP recording analysis scripts."""

from __future__ import annotations

import importlib.util
import io
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot import {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


analyze = load_module("analyze_mcp_logs", ROOT / "scripts/analyze_mcp_logs.py")
compare = load_module("compare_recording_payloads", ROOT / "scripts/compare_recording_payloads.py")


def event(timestamp: str, process: str, call: str, event_name: str, **fields):
    value = {
        "timestamp": timestamp,
        "event_version": 1,
        "process_instance_id": process,
        "call_id": call,
        "event": event_name,
    }
    value.update(fields)
    return value


class ScriptTests(unittest.TestCase):
    def test_log_analysis_handles_overlap_missing_calls_duplicates_and_filters(self):
        start_one = event(
            "2026-09-09T10:00:00.000Z",
            "p1",
            "c1",
            "mcp_tool_started",
            tool="record_tutoring_session",
            arguments_json_bytes=100,
        )
        complete_one = event(
            "2026-09-09T10:00:00.100Z",
            "p1",
            "c1",
            "mcp_tool_completed",
            tool="record_tutoring_session",
            duration_ms=100,
            outcome="success",
            record_status="created",
            review_applied_count=1,
            review_skipped_count=0,
        )
        start_two = event(
            "2026-09-09T10:00:00.050Z",
            "p1",
            "c2",
            "mcp_tool_started",
            tool="get_practice_brief",
        )
        complete_two = event(
            "2026-09-09T10:00:00.250Z",
            "p1",
            "c2",
            "mcp_tool_completed",
            tool="get_practice_brief",
            duration_ms=200,
            outcome="success",
        )
        start_three = event(
            "2026-09-09T10:00:00.300Z",
            "p1",
            "c3",
            "mcp_tool_started",
            tool="record_practice_session",
            arguments_json_bytes=200,
        )
        completion_only = event(
            "2026-09-09T10:00:00.400Z",
            "p2",
            "c4",
            "mcp_tool_completed",
            tool="unknown",
            outcome="unknown_tool",
        )
        phase = event(
            "2026-09-09T10:00:00.060Z",
            "p1",
            "c1",
            "mcp_recording_phase",
            tool="record_tutoring_session",
            phase="compact_expansion",
            duration_ms=2,
        )
        validation = event(
            "2026-09-09T10:00:00.101Z",
            "p1",
            "c1",
            "mcp_validation_failed",
            tool="record_tutoring_session",
            code="invalid_argument",
            path="turns[0].attempts[0].transcript",
            expected="nonblank text",
            correction="correct the field and retry",
        )
        lines = [
            json.dumps(start_one),
            json.dumps(start_one),
            json.dumps(complete_one),
            json.dumps({**complete_one, "duration_ms": 101}),
            json.dumps(start_two),
            json.dumps(complete_two),
            json.dumps(start_three),
            json.dumps(completion_only),
            json.dumps(phase),
            json.dumps(phase),
            json.dumps(validation),
            "not json",
            json.dumps({"timestamp": "2026-09-09T10:00:00Z", "message": "unrelated"}),
            "",
        ]
        events, stats = analyze.read_events(io.StringIO("\n".join(lines) + "\n"))
        report = analyze.analyze(
            events,
            stats,
            client_start=analyze.parse_timestamp("2026-09-09T09:59:59Z"),
            client_end=analyze.parse_timestamp("2026-09-09T10:00:01Z"),
        )
        self.assertEqual(stats["duplicate_events"], 2)
        self.assertEqual(stats["conflicting_duplicates"], 1)
        self.assertEqual(stats["malformed_lines"], 1)
        self.assertEqual(stats["unrelated_lines"], 1)
        self.assertEqual(stats["empty_lines"], 1)
        self.assertEqual(report["input"]["selected_calls"], 4)
        recorder = report["per_tool"]["record_tutoring_session"]
        self.assertEqual(recorder["calls"], 1)
        self.assertEqual(recorder["record_creations"], 1)
        incomplete_recorder = report["per_tool"]["record_practice_session"]
        self.assertEqual(incomplete_recorder["incomplete_starts"], 1)
        self.assertEqual(recorder["argument_bytes"]["missing"], 0)
        self.assertEqual(report["per_tool"]["get_practice_brief"]["duration_ms"]["p50"], 200)
        self.assertEqual(report["timing"]["external_client_total_ms"], 2000.0)
        self.assertGreater(report["timing"]["sum_action_duration_ms"], report["timing"]["server_busy_union_ms"])
        self.assertEqual(report["validation"]["events"], 1)
        self.assertEqual(report["validation"]["codes"]["invalid_argument"], 1)
        self.assertEqual(report["validation"]["paths"]["turns[0].attempts[0].transcript"], 1)

        filtered = analyze.analyze(
            events,
            stats,
            tool="record_tutoring_session",
            call_ids={"c1"},
        )
        self.assertEqual(filtered["input"]["selected_calls"], 1)
        self.assertEqual(filtered["totals"]["record_creations"], 1)

    def test_payload_comparison_reports_normalized_bytes_and_schema_sizes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            canonical = root / "canonical.json"
            compact = root / "compact.json"
            tools = root / "tools.json"
            canonical.write_text(json.dumps({"turns": [{"prompt": "x"}], "answers": ["x", "x"]}), encoding="utf-8")
            compact.write_text(json.dumps({"turns": [{"prompt": "x"}]}), encoding="utf-8")
            tools.write_text(
                json.dumps(
                    {
                        "result": {
                            "tools": [
                                {"name": "record_practice_session", "inputSchema": {"type": "object"}},
                                {"name": "record_tutoring_session", "inputSchema": {"type": "object", "properties": {}}},
                            ]
                        }
                    }
                ),
                encoding="utf-8",
            )
            report = compare.compare(str(canonical), str(compact), str(tools))
            self.assertEqual(report["canonical"]["serialized_bytes"], len(compare.serialized_bytes({"answers": ["x", "x"], "turns": [{"prompt": "x"}]})))
            self.assertGreater(report["savings"]["bytes"], 0)
            self.assertEqual(report["tools_list"]["catalog_tool_count"], 2)
            self.assertEqual(report["tools_list"]["recorder_input_schemas"]["record_tutoring_session"]["schema_bytes"], len(compare.serialized_bytes({"type": "object", "properties": {}})))

    def test_learner_text_is_filtered_by_default_and_available_on_opt_in(self):
        text_event = event(
            "2026-09-09T10:00:00.010Z",
            "p1",
            "c1",
            "mcp_tool_text",
            tool="record_tutoring_session",
            text_fields_json='[{"path":"turns[0].attempts[0].transcript","text":"learner sentinel"}]',
        )
        lines = json.dumps(text_event) + "\n"
        events, stats = analyze.read_events(io.StringIO(lines))
        self.assertEqual(events, [])
        self.assertEqual(stats["text_events_skipped"], 1)
        events, stats = analyze.read_events(io.StringIO(lines), include_text=True)
        self.assertEqual(len(events), 1)
        self.assertEqual(stats["text_events"], 1)
        report = analyze.analyze(events, stats, include_text=True)
        self.assertEqual(report["input"]["selected_text_events"], 1)
        self.assertIn("learner sentinel", report["text_events"][0]["text_fields_json"])


if __name__ == "__main__":
    unittest.main()
