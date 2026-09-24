#!/usr/bin/env python3
"""Source-neutral continuous fidelity/loss diagnostics for PREFLIGHT-01."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

INPUT_SCHEMA = "chaptera.preflight.input.v1"
REPORT_SCHEMA = "chaptera.preflight.report.v1"

CODE_TEXT_OVERFLOW = "TEXT_OVERFLOW"
CODE_RESOURCE_MISSING = "RESOURCE_MISSING"
CODE_RESOURCE_MODIFIED = "RESOURCE_MODIFIED"
CODE_SEMANTIC_UNSUPPORTED = "SEMANTIC_UNSUPPORTED"
CODE_SEMANTIC_OPAQUE = "SEMANTIC_OPAQUE"
CODE_OUTPUT_RISK = "OUTPUT_RISK"

SEVERITY = {
    CODE_TEXT_OVERFLOW: "error",
    CODE_RESOURCE_MISSING: "error",
    CODE_RESOURCE_MODIFIED: "warning",
    CODE_SEMANTIC_UNSUPPORTED: "warning",
    CODE_SEMANTIC_OPAQUE: "warning",
    CODE_OUTPUT_RISK: "warning",
}

MESSAGE = {
    CODE_TEXT_OVERFLOW: "Text exceeds the resolved frame/story capacity.",
    CODE_RESOURCE_MISSING: "A required referenced resource is missing.",
    CODE_RESOURCE_MODIFIED: "A referenced resource differs from its pinned identity.",
    CODE_SEMANTIC_UNSUPPORTED: "The object contains semantics not currently supported by this capability set.",
    CODE_SEMANTIC_OPAQUE: "The object contains semantics that are present but not yet interpreted.",
    CODE_OUTPUT_RISK: "Known state may cause fidelity loss in a downstream output path.",
}


def stable(value: Any) -> Any:
    if isinstance(value, dict):
        return {k: stable(value[k]) for k in sorted(value)}
    if isinstance(value, list):
        return [stable(v) for v in value]
    return value


def load(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("schema") != INPUT_SCHEMA:
        raise ValueError(f"{path}: expected schema {INPUT_SCHEMA!r}")
    return data


def diag(code: str, obj: dict[str, Any], detail: dict[str, Any] | None = None) -> dict[str, Any]:
    origin = {"page_id": obj.get("page_id"), "node_id": obj.get("node_id")}
    if obj.get("story_id") is not None:
        origin["story_id"] = obj.get("story_id")
    result = {
        "code": code,
        "severity": SEVERITY[code],
        "message": MESSAGE[code],
        "origin": stable(origin),
    }
    if detail:
        result["detail"] = stable(detail)
    return result


def evaluate(doc: dict[str, Any]) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for obj in doc.get("objects", []):
        node_id = obj.get("node_id")
        page_id = obj.get("page_id")
        if not isinstance(node_id, str) or not node_id:
            raise ValueError("every object requires non-empty node_id")
        if not isinstance(page_id, str) or not page_id:
            raise ValueError(f"{node_id}: every object requires non-empty page_id")

        if obj.get("text_overflow") is True:
            out.append(diag(CODE_TEXT_OVERFLOW, obj))

        for resource in obj.get("resources", []):
            state = resource.get("state")
            detail = {
                "resource_id": resource.get("resource_id"),
                "kind": resource.get("kind"),
            }
            if state == "missing":
                out.append(diag(CODE_RESOURCE_MISSING, obj, detail))
            elif state == "modified":
                detail["expected_sha256"] = resource.get("expected_sha256")
                detail["actual_sha256"] = resource.get("actual_sha256")
                out.append(diag(CODE_RESOURCE_MODIFIED, obj, detail))

        semantic_state = obj.get("semantic_state")
        if semantic_state == "unsupported":
            out.append(diag(CODE_SEMANTIC_UNSUPPORTED, obj, {"feature": obj.get("semantic_feature")}))
        elif semantic_state == "opaque":
            out.append(diag(CODE_SEMANTIC_OPAQUE, obj, {"feature": obj.get("semantic_feature")}))

        for risk in obj.get("output_risks", []):
            out.append(
                diag(
                    CODE_OUTPUT_RISK,
                    obj,
                    {
                        "target": risk.get("target"),
                        "risk": risk.get("risk"),
                    },
                )
            )

    return sorted(
        out,
        key=lambda d: (
            d["origin"].get("page_id") or "",
            d["origin"].get("node_id") or "",
            d["code"],
            json.dumps(d.get("detail", {}), sort_keys=True),
        ),
    )


def build_report(doc: dict[str, Any]) -> dict[str, Any]:
    diagnostics = evaluate(doc)
    counts_by_code: dict[str, int] = {}
    counts_by_severity: dict[str, int] = {}
    for item in diagnostics:
        counts_by_code[item["code"]] = counts_by_code.get(item["code"], 0) + 1
        counts_by_severity[item["severity"]] = counts_by_severity.get(item["severity"], 0) + 1

    summary_lines = [
        f"{code}: {counts_by_code[code]}"
        for code in sorted(counts_by_code)
    ]
    return {
        "schema": REPORT_SCHEMA,
        "fixture_id": doc.get("fixture_id"),
        "summary": {
            "diagnostic_count": len(diagnostics),
            "counts_by_code": {k: counts_by_code[k] for k in sorted(counts_by_code)},
            "counts_by_severity": {k: counts_by_severity[k] for k in sorted(counts_by_severity)},
            "human": "No diagnostics." if not diagnostics else "; ".join(summary_lines),
        },
        "diagnostics": diagnostics,
    }


def write(report: dict[str, Any], path: Path | None) -> str:
    rendered = json.dumps(report, sort_keys=True, ensure_ascii=False, indent=2) + "\n"
    if path:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(rendered, encoding="utf-8", newline="\n")
    else:
        sys.stdout.write(rendered)
    return rendered


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    try:
        report = build_report(load(args.input))
        write(report, args.out)
        return 0
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"preflight: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
