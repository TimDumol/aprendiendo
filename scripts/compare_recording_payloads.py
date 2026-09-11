#!/usr/bin/env python3
"""Compare canonical and compact recording JSON payloads by UTF-8 bytes."""

from __future__ import annotations

import argparse
import csv
import json
import sys
from pathlib import Path
from typing import Any, TextIO


def load_json(path: str) -> Any:
    with Path(path).open("r", encoding="utf-8") as handle:
        return json.load(handle)


def serialized_bytes(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def raw_bytes(path: str) -> int:
    return len(Path(path).read_bytes())


def tools_from_response(value: Any) -> list[dict[str, Any]]:
    if isinstance(value, dict):
        if isinstance(value.get("result"), dict):
            return tools_from_response(value["result"])
        tools = value.get("tools")
        if isinstance(tools, list):
            return [tool for tool in tools if isinstance(tool, dict)]
    raise ValueError("tools/list response must contain a result.tools or tools array")


def tool_measurements(path: str) -> dict[str, Any]:
    response = load_json(path)
    tools = tools_from_response(response)
    by_name = {
        tool.get("name"): tool
        for tool in tools
        if isinstance(tool.get("name"), str)
    }
    recorder_names = ("record_practice_session", "record_tutoring_session")
    schemas: dict[str, Any] = {}
    for name in recorder_names:
        tool = by_name.get(name)
        if tool is None:
            schemas[name] = {"present": False, "schema_bytes": None}
            continue
        schema = tool.get("inputSchema")
        schemas[name] = {
            "present": True,
            "schema_bytes": len(serialized_bytes(schema)) if schema is not None else None,
        }
    return {
        "catalog_tool_count": len(tools),
        "catalog_bytes": len(serialized_bytes(tools)),
        "response_bytes": len(serialized_bytes(response)),
        "recorder_input_schemas": schemas,
    }


def compare(canonical_path: str, compact_path: str, tools_list_path: str | None) -> dict[str, Any]:
    canonical = load_json(canonical_path)
    compact = load_json(compact_path)
    canonical_serialized = serialized_bytes(canonical)
    compact_serialized = serialized_bytes(compact)
    canonical_size = len(canonical_serialized)
    compact_size = len(compact_serialized)
    savings = canonical_size - compact_size
    report: dict[str, Any] = {
        "encoding": "UTF-8 JSON with sorted keys and compact separators",
        "unit": "bytes",
        "canonical": {
            "path": canonical_path,
            "raw_fixture_bytes": raw_bytes(canonical_path),
            "serialized_bytes": canonical_size,
        },
        "compact": {
            "path": compact_path,
            "raw_fixture_bytes": raw_bytes(compact_path),
            "serialized_bytes": compact_size,
        },
        "savings": {
            "bytes": savings,
            "percent": (savings / canonical_size * 100) if canonical_size else None,
        },
    }
    if tools_list_path:
        report["tools_list"] = tool_measurements(tools_list_path)
    return report


def print_text(report: dict[str, Any], output: TextIO) -> None:
    canonical = report["canonical"]
    compact = report["compact"]
    savings = report["savings"]
    print("Recording payload comparison (serialized UTF-8 JSON bytes; not token estimates)", file=output)
    print(f"canonical: {canonical['serialized_bytes']} bytes ({canonical['raw_fixture_bytes']} raw fixture bytes)", file=output)
    print(f"compact:   {compact['serialized_bytes']} bytes ({compact['raw_fixture_bytes']} raw fixture bytes)", file=output)
    print(f"savings:   {savings['bytes']} bytes ({savings['percent']:.2f}%)", file=output)
    if "tools_list" in report:
        catalog = report["tools_list"]
        print(
            f"tools/list: {catalog['catalog_tool_count']} tools, "
            f"{catalog['catalog_bytes']} catalog bytes, "
            f"{catalog['response_bytes']} response bytes",
            file=output,
        )
        for name, values in catalog["recorder_input_schemas"].items():
            print(f"{name} input schema: {values['schema_bytes']} bytes", file=output)


def csv_rows(report: dict[str, Any]):
    yield {"scope": "payload", "name": "canonical", "metric": "serialized_bytes", "value": report["canonical"]["serialized_bytes"], "unit": "bytes"}
    yield {"scope": "payload", "name": "compact", "metric": "serialized_bytes", "value": report["compact"]["serialized_bytes"], "unit": "bytes"}
    yield {"scope": "payload", "name": "comparison", "metric": "savings_bytes", "value": report["savings"]["bytes"], "unit": "bytes"}
    yield {"scope": "payload", "name": "comparison", "metric": "savings_percent", "value": report["savings"]["percent"], "unit": "percent"}
    if "tools_list" in report:
        catalog = report["tools_list"]
        yield {"scope": "catalog", "name": "tools", "metric": "tool_count", "value": catalog["catalog_tool_count"], "unit": "count"}
        yield {"scope": "catalog", "name": "tools", "metric": "catalog_bytes", "value": catalog["catalog_bytes"], "unit": "bytes"}
        yield {"scope": "catalog", "name": "tools", "metric": "response_bytes", "value": catalog["response_bytes"], "unit": "bytes"}
        for name, values in catalog["recorder_input_schemas"].items():
            yield {"scope": "schema", "name": name, "metric": "input_schema_bytes", "value": values["schema_bytes"], "unit": "bytes"}


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("canonical_json")
    parser.add_argument("compact_json")
    parser.add_argument("--tools-list", help="saved tools/list response, if available")
    parser.add_argument("--format", choices=("text", "json", "csv"), default="text")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    report = compare(args.canonical_json, args.compact_json, args.tools_list)
    if args.format == "json":
        json.dump(report, sys.stdout, indent=2, sort_keys=True)
        sys.stdout.write("\n")
    elif args.format == "csv":
        writer = csv.DictWriter(sys.stdout, fieldnames=("scope", "name", "metric", "value", "unit"))
        writer.writeheader()
        writer.writerows(csv_rows(report))
    else:
        print_text(report, sys.stdout)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
