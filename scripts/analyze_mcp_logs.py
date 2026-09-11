#!/usr/bin/env python3
"""Summarize Aprendiendo MCP telemetry JSONL without external dependencies.

The input is the unprefixed JSON output produced by the server's JSON tracing
subscriber.  The script deliberately treats missing metrics as missing data;
it never turns an absent field into zero.
"""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import json
import math
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any, Iterable, TextIO


UTC = dt.timezone.utc
ACTION_EVENTS = {"mcp_tool_started", "mcp_tool_completed"}
TEXT_EVENT = "mcp_tool_text"
VALIDATION_EVENT = "mcp_validation_failed"


def parse_timestamp(value: str) -> dt.datetime:
    """Parse an RFC 3339 timestamp and normalize it to UTC."""

    text = value.strip()
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    try:
        parsed = dt.datetime.fromisoformat(text)
    except ValueError as exc:
        raise ValueError(f"invalid RFC 3339 timestamp: {value!r}") from exc
    if parsed.tzinfo is None:
        raise ValueError(f"timestamp must include a UTC offset: {value!r}")
    return parsed.astimezone(UTC)


def format_timestamp(value: dt.datetime | None) -> str | None:
    if value is None:
        return None
    return value.isoformat(timespec="milliseconds").replace("+00:00", "Z")


def percentile(values: list[float], fraction: float) -> float | None:
    """Return a linearly interpolated percentile.

    The rank is p * (n - 1), with the two surrounding observations blended
    linearly.  This is stable for one observation and does not require a
    statistics package.
    """

    if not values:
        return None
    ordered = sorted(values)
    rank = fraction * (len(ordered) - 1)
    lower = math.floor(rank)
    upper = math.ceil(rank)
    if lower == upper:
        return ordered[lower]
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (rank - lower)


def integer_percentile(values: list[int], fraction: float) -> float | None:
    value = percentile([float(item) for item in values], fraction)
    if value is None:
        return None
    return round(value, 3)


def duration_distribution(values: list[int], missing: int) -> dict[str, Any]:
    return {
        "observed": len(values),
        "missing": missing,
        "p50": integer_percentile(values, 0.50),
        "p95": integer_percentile(values, 0.95),
        "max": max(values) if values else None,
        "unit": "milliseconds",
    }


def byte_distribution(values: list[int], missing: int) -> dict[str, Any]:
    return {
        "observed": len(values),
        "missing": missing,
        "min": min(values) if values else None,
        "p50": integer_percentile(values, 0.50),
        "p95": integer_percentile(values, 0.95),
        "max": max(values) if values else None,
        "unit": "bytes",
    }


def event_key(event: dict[str, Any]) -> tuple[str, str, str, str]:
    return (
        str(event.get("process_instance_id", "")),
        str(event.get("call_id", "")),
        str(event.get("event", "")),
        str(event.get("phase", "")),
    )


def call_key(event: dict[str, Any]) -> tuple[str, str]:
    return (
        str(event.get("process_instance_id", "")),
        str(event.get("call_id", "")),
    )


def read_events(handle: TextIO, *, include_text: bool = False) -> tuple[list[dict[str, Any]], dict[str, int]]:
    events: list[dict[str, Any]] = []
    stats = {
        "lines": 0,
        "empty_lines": 0,
        "malformed_lines": 0,
        "unrelated_lines": 0,
        "duplicate_events": 0,
        "conflicting_duplicates": 0,
        "text_events": 0,
        "text_events_skipped": 0,
        "validation_events": 0,
    }
    seen: dict[tuple[str, str, str, str], str] = {}
    for line in handle:
        stats["lines"] += 1
        if not line.strip():
            stats["empty_lines"] += 1
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            stats["malformed_lines"] += 1
            continue
        if isinstance(value, dict) and isinstance(value.get("fields"), dict):
            value = {**value, **value["fields"]}
        event_name = value.get("event") if isinstance(value, dict) else None
        if event_name == TEXT_EVENT and not include_text:
            stats["text_events_skipped"] += 1
            continue
        if event_name == TEXT_EVENT:
            stats["text_events"] += 1
        if event_name == VALIDATION_EVENT:
            stats["validation_events"] += 1
        if not isinstance(value, dict) or event_name not in ACTION_EVENTS | {
            "mcp_recording_phase", TEXT_EVENT, VALIDATION_EVENT
        }:
            stats["unrelated_lines"] += 1
            continue
        if not value.get("process_instance_id") or not value.get("call_id"):
            stats["unrelated_lines"] += 1
            continue
        if not value.get("timestamp"):
            stats["malformed_lines"] += 1
            continue
        try:
            value["_timestamp"] = parse_timestamp(str(value["timestamp"]))
        except ValueError:
            stats["malformed_lines"] += 1
            continue
        key = event_key(value)
        serialized = json.dumps(value, sort_keys=True, default=str, separators=(",", ":"))
        previous = seen.get(key)
        if previous is not None:
            if previous == serialized:
                stats["duplicate_events"] += 1
            else:
                stats["conflicting_duplicates"] += 1
            continue
        seen[key] = serialized
        events.append(value)
    return events, stats


def selected_events(
    events: Iterable[dict[str, Any]],
    *,
    from_time: dt.datetime | None,
    to_time: dt.datetime | None,
    tool: str | None,
    call_ids: set[str],
) -> list[dict[str, Any]]:
    selected: list[dict[str, Any]] = []
    for event in events:
        timestamp = event["_timestamp"]
        if from_time is not None and timestamp < from_time:
            continue
        if to_time is not None and timestamp > to_time:
            continue
        if call_ids and str(event.get("call_id")) not in call_ids:
            continue
        if tool is not None and event.get("tool") != tool:
            continue
        selected.append(event)
    return selected


def call_groups(events: Iterable[dict[str, Any]]) -> dict[tuple[str, str], dict[str, Any]]:
    groups: dict[tuple[str, str], dict[str, Any]] = {}
    for event in events:
        if event.get("event") in {TEXT_EVENT, VALIDATION_EVENT}:
            continue
        key = call_key(event)
        group = groups.setdefault(
            key,
            {
                "process_instance_id": key[0],
                "call_id": key[1],
                "starts": [],
                "completions": [],
                "phases": [],
            },
        )
        if event["event"] == "mcp_tool_started":
            group["starts"].append(event)
        elif event["event"] == "mcp_tool_completed":
            group["completions"].append(event)
        else:
            group["phases"].append(event)
    return groups


def group_tool(group: dict[str, Any]) -> str:
    for key in ("starts", "completions"):
        if group[key]:
            return str(group[key][0].get("tool") or "unknown")
    return "unknown"


def paired_call(group: dict[str, Any]) -> tuple[dict[str, Any] | None, dict[str, Any] | None]:
    start = group["starts"][0] if group["starts"] else None
    completion = group["completions"][0] if group["completions"] else None
    return start, completion


def summarize_tool(groups: list[dict[str, Any]]) -> dict[str, Any]:
    completed = [group["completions"][0] for group in groups if group["completions"]]
    starts = [group["starts"][0] for group in groups if group["starts"]]
    durations = [
        int(item["duration_ms"])
        for item in completed
        if isinstance(item.get("duration_ms"), (int, float))
        and not isinstance(item.get("duration_ms"), bool)
    ]
    duration_missing = len(completed) - len(durations)
    argument_bytes = [
        int(item["arguments_json_bytes"])
        for item in starts
        if isinstance(item.get("arguments_json_bytes"), (int, float))
        and not isinstance(item.get("arguments_json_bytes"), bool)
    ]
    argument_missing = len(starts) - len(argument_bytes)
    response_bytes = [
        int(item["response_json_bytes"])
        for item in completed
        if isinstance(item.get("response_json_bytes"), (int, float))
        and not isinstance(item.get("response_json_bytes"), bool)
    ]
    outcomes = [str(item.get("outcome")) for item in completed if item.get("outcome")]
    outcome_missing = len(completed) - len(outcomes)
    created = sum(item.get("record_status") == "created" for item in completed)
    replayed = sum(item.get("record_status") == "replayed" for item in completed)
    status_missing = len(completed) - created - replayed
    applied = [
        int(item["review_applied_count"])
        for item in completed
        if isinstance(item.get("review_applied_count"), (int, float))
        and not isinstance(item.get("review_applied_count"), bool)
    ]
    skipped = [
        int(item["review_skipped_count"])
        for item in completed
        if isinstance(item.get("review_skipped_count"), (int, float))
        and not isinstance(item.get("review_skipped_count"), bool)
    ]
    return {
        "calls": len(groups),
        "started_calls": len(starts),
        "completed_calls": len(completed),
        "successes": sum(outcome == "success" for outcome in outcomes),
        "failures": sum(outcome != "success" for outcome in outcomes),
        "outcome_missing": outcome_missing,
        "outcomes": {outcome: outcomes.count(outcome) for outcome in sorted(set(outcomes))},
        "record_creations": created,
        "record_replays": replayed,
        "record_status_missing": status_missing,
        "duration_ms": duration_distribution(durations, duration_missing),
        "argument_bytes": {
            "observed": len(argument_bytes),
            "missing": argument_missing,
            "min": min(argument_bytes) if argument_bytes else None,
            "p50": integer_percentile(argument_bytes, 0.50),
            "p95": integer_percentile(argument_bytes, 0.95),
            "max": max(argument_bytes) if argument_bytes else None,
            "unit": "bytes",
        },
        "response_bytes": byte_distribution(response_bytes, len(completed) - len(response_bytes)),
        "review_applied": {
            "total": sum(applied) if applied else None,
            "observed_calls": len(applied),
            "missing_calls": len(completed) - len(applied),
        },
        "review_skipped": {
            "total": sum(skipped) if skipped else None,
            "observed_calls": len(skipped),
            "missing_calls": len(completed) - len(skipped),
        },
        "incomplete_starts": sum(not group["completions"] for group in groups),
        "incomplete_completions": sum(not group["starts"] for group in groups),
    }


def union_duration(intervals: list[tuple[float, float]]) -> float:
    if not intervals:
        return 0.0
    total = 0.0
    start, end = sorted(intervals)[0]
    for next_start, next_end in sorted(intervals)[1:]:
        if next_start <= end:
            end = max(end, next_end)
        else:
            total += end - start
            start, end = next_start, next_end
    return total + end - start


def timing_summary(
    groups: list[dict[str, Any]],
    *,
    from_time: dt.datetime | None,
    to_time: dt.datetime | None,
    client_start: dt.datetime | None,
    client_end: dt.datetime | None,
) -> dict[str, Any]:
    intervals: list[tuple[float, float]] = []
    action_duration_values: list[int] = []
    timestamps: list[dt.datetime] = []
    for group in groups:
        start, completion = paired_call(group)
        if start is None:
            if completion is not None:
                timestamps.append(completion["_timestamp"])
            continue
        timestamps.append(start["_timestamp"])
        if completion is None:
            continue
        timestamps.append(completion["_timestamp"])
        start_time = start["_timestamp"]
        end_time = completion["_timestamp"]
        if end_time < start_time:
            continue
        if isinstance(completion.get("duration_ms"), (int, float)) and not isinstance(
            completion.get("duration_ms"), bool
        ):
            action_duration_values.append(int(completion["duration_ms"]))
        clipped_start = max(start_time, from_time) if from_time else start_time
        clipped_end = min(end_time, to_time) if to_time else end_time
        if clipped_end >= clipped_start:
            intervals.append((clipped_start.timestamp(), clipped_end.timestamp()))
    if from_time is not None:
        timestamps.append(from_time)
    if to_time is not None:
        timestamps.append(to_time)
    window_start = min(timestamps) if timestamps else None
    window_end = max(timestamps) if timestamps else None
    if from_time is not None:
        window_start = max(window_start, from_time) if window_start else from_time
    if to_time is not None:
        window_end = min(window_end, to_time) if window_end else to_time
    window_ms = (
        max(0.0, (window_end - window_start).total_seconds() * 1000)
        if window_start is not None and window_end is not None
        else None
    )
    busy_ms = union_duration(intervals) * 1000 if intervals else 0.0
    client_total = None
    if client_start is not None and client_end is not None:
        client_total = max(0.0, (client_end - client_start).total_seconds() * 1000)
    return {
        "window_start": format_timestamp(window_start),
        "window_end": format_timestamp(window_end),
        "window_elapsed_ms": round(window_ms, 3) if window_ms is not None else None,
        "sum_action_duration_ms": sum(action_duration_values) if action_duration_values else None,
        "action_duration_observed": len(action_duration_values),
        "action_duration_missing": sum(
            1 for group in groups if group["starts"] and group["completions"]
        )
        - len(action_duration_values),
        "server_busy_union_ms": round(busy_ms, 3),
        "unattributed_gap_ms": (
            round(max(0.0, window_ms - busy_ms), 3) if window_ms is not None else None
        ),
        "external_client_total_ms": round(client_total, 3) if client_total is not None else None,
        "external_client_bounds_supplied": client_start is not None or client_end is not None,
        "notes": [
            "Action-duration sums may exceed elapsed time when calls overlap.",
            "The window is an observation window; adjacency does not prove one tutoring session.",
        ],
    }


def analyze(
    events: list[dict[str, Any]],
    input_stats: dict[str, int],
    *,
    from_time: dt.datetime | None = None,
    to_time: dt.datetime | None = None,
    tool: str | None = None,
    call_ids: set[str] | None = None,
    client_start: dt.datetime | None = None,
    client_end: dt.datetime | None = None,
    include_text: bool = False,
) -> dict[str, Any]:
    filtered = selected_events(
        events,
        from_time=from_time,
        to_time=to_time,
        tool=tool,
        call_ids=call_ids or set(),
    )
    selected_text = [event for event in filtered if event.get("event") == TEXT_EVENT]
    selected_validation = [
        event for event in filtered if event.get("event") == VALIDATION_EVENT
    ]
    groups = call_groups(filtered)
    by_tool: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for group in groups.values():
        by_tool[group_tool(group)].append(group)
    per_tool = {
        name: summarize_tool(groups_for_tool)
        for name, groups_for_tool in sorted(by_tool.items())
    }
    total = summarize_tool(list(groups.values()))
    total["tool_count"] = len(per_tool)
    return {
        "input": {
            **input_stats,
            "selected_events": len(filtered),
            "selected_calls": len(groups),
            "selected_text_events": len(selected_text),
            "selected_validation_events": len(selected_validation),
            "filters": {
                "from": format_timestamp(from_time),
                "to": format_timestamp(to_time),
                "tool": tool,
                "call_ids": sorted(call_ids or set()),
            },
        },
        "per_tool": per_tool,
        "totals": total,
        "validation": {
            "events": len(selected_validation),
            "codes": {
                code: sum(event.get("code") == code for event in selected_validation)
                for code in sorted({str(event.get("code")) for event in selected_validation})
            },
            "paths": {
                path: sum(event.get("path") == path for event in selected_validation)
                for path in sorted(
                    {
                        str(event.get("path"))
                        for event in selected_validation
                        if event.get("path")
                    }
                )
            },
        },
        "timing": timing_summary(
            list(groups.values()),
            from_time=from_time,
            to_time=to_time,
            client_start=client_start,
            client_end=client_end,
        ),
        **({"text_events": selected_text} if include_text else {}),
    }


def flatten_csv(report: dict[str, Any]) -> Iterable[dict[str, Any]]:
    columns = ("scope", "tool", "metric", "value", "unit", "observed", "missing")

    def row(scope: str, tool: str, metric: str, value: Any, unit: str = "", observed: Any = "", missing: Any = ""):
        return dict(zip(columns, (scope, tool, metric, value, unit, observed, missing)))

    for tool, data in report["per_tool"].items():
        for metric in (
            "calls",
            "started_calls",
            "completed_calls",
            "successes",
            "failures",
            "outcome_missing",
            "record_creations",
            "record_replays",
            "record_status_missing",
            "incomplete_starts",
            "incomplete_completions",
        ):
            yield row("per_tool", tool, metric, data[metric])
        duration = data["duration_ms"]
        for metric in ("p50", "p95", "max"):
            yield row("per_tool", tool, f"duration_{metric}", duration[metric], "milliseconds", duration["observed"], duration["missing"])
        arguments = data["argument_bytes"]
        for metric in ("min", "p50", "p95", "max"):
            yield row("per_tool", tool, f"argument_bytes_{metric}", arguments[metric], "bytes", arguments["observed"], arguments["missing"])
        responses = data["response_bytes"]
        for metric in ("min", "p50", "p95", "max"):
            yield row("per_tool", tool, f"response_bytes_{metric}", responses[metric], "bytes", responses["observed"], responses["missing"])
        for metric in ("review_applied", "review_skipped"):
            value = data[metric]
            yield row("per_tool", tool, f"{metric}_total", value["total"], "count", value["observed_calls"], value["missing_calls"])
    for metric, value in report["timing"].items():
        if isinstance(value, (int, float)) or value is None:
            unit = "milliseconds" if metric.endswith("_ms") else "count"
            yield row("timing", "", metric, value, unit)
    yield row("validation", "", "events", report["validation"]["events"], "count")
    for code, count in report["validation"]["codes"].items():
        yield row("validation", "", f"code_{code}", count, "count")
    for path, count in report["validation"]["paths"].items():
        yield row("validation", "", f"path_{path}", count, "count")


def print_text(report: dict[str, Any], output: TextIO) -> None:
    input_data = report["input"]
    print(
        "Read {lines} lines: {selected_calls} selected calls, {malformed_lines} malformed, "
        "{unrelated_lines} unrelated, {duplicate_events} duplicate events, "
        "{conflicting_duplicates} conflicting duplicates.".format(**input_data),
        file=output,
    )
    for tool, data in report["per_tool"].items():
        duration = data["duration_ms"]
        args = data["argument_bytes"]
        responses = data["response_bytes"]
        print(
            f"{tool}: {data['calls']} calls; {data['successes']} successes; "
            f"{data['failures']} failures; {data['outcome_missing']} outcomes missing; "
            f"{data['record_creations']} created; "
            f"{data['record_replays']} replayed; duration p50/p95/max "
            f"{duration['p50']}/{duration['p95']}/{duration['max']} ms "
            f"({duration['observed']} observed, {duration['missing']} missing); "
            f"arguments p50 {args['p50']} bytes ({args['observed']} observed, {args['missing']} missing).",
            file=output,
        )
        print(
            f"{tool}: responses p50 {responses['p50']} bytes "
            f"({responses['observed']} observed, {responses['missing']} missing).",
            file=output,
        )
    timing = report["timing"]
    print(
        "Timing: window {window_elapsed_ms} ms; action sum {sum_action_duration_ms} ms; "
        "server-busy union {server_busy_union_ms} ms; unattributed gap "
        "{unattributed_gap_ms} ms; external client total {external_client_total_ms} ms.".format(
            **timing
        ),
        file=output,
    )
    validation = report["validation"]
    print(
        f"Validation diagnostics: {validation['events']} events; "
        f"codes {validation['codes']}; paths {validation['paths']}.",
        file=output,
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", nargs="?", help="JSONL log path; omit or use - for stdin")
    parser.add_argument("--format", choices=("text", "json", "csv"), default="text")
    parser.add_argument("--from", dest="from_time", type=parse_timestamp)
    parser.add_argument("--to", dest="to_time", type=parse_timestamp)
    parser.add_argument("--tool")
    parser.add_argument("--call-id", action="append", default=[])
    parser.add_argument("--client-start", type=parse_timestamp)
    parser.add_argument("--client-end", type=parse_timestamp)
    parser.add_argument(
        "--include-text",
        action="store_true",
        help="include separately logged learner text events; omitted by default",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if (args.client_start is None) != (args.client_end is None):
        raise SystemExit("--client-start and --client-end must be supplied together")
    if args.from_time and args.to_time and args.from_time > args.to_time:
        raise SystemExit("--from must not be after --to")
    if args.client_start and args.client_end and args.client_start > args.client_end:
        raise SystemExit("--client-start must not be after --client-end")
    if not args.path or args.path == "-":
        events, stats = read_events(sys.stdin, include_text=args.include_text)
    else:
        with Path(args.path).open("r", encoding="utf-8") as handle:
            events, stats = read_events(handle, include_text=args.include_text)
    report = analyze(
        events,
        stats,
        from_time=args.from_time,
        to_time=args.to_time,
        tool=args.tool,
        call_ids=set(args.call_id),
        client_start=args.client_start,
        client_end=args.client_end,
        include_text=args.include_text,
    )
    if args.format == "json":
        json.dump(report, sys.stdout, indent=2, sort_keys=True)
        sys.stdout.write("\n")
    elif args.format == "csv":
        writer = csv.DictWriter(
            sys.stdout,
            fieldnames=("scope", "tool", "metric", "value", "unit", "observed", "missing"),
        )
        writer.writeheader()
        writer.writerows(flatten_csv(report))
    else:
        print_text(report, sys.stdout)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
