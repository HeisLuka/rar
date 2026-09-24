#!/usr/bin/env python3
"""Source-neutral shaped-flow -> fixed-text-run bridge for FIXED-PDF-SHAPED-FLOW-01.

The bridge consumes already-resolved shaped-flow lines. It does not shape text,
does not parse PUB and does not serialize PDF. Its job is narrower:

- preserve exact resolved glyph sequences;
- preserve Story-global scalar provenance via scalar_base;
- derive deterministic bounded baselines;
- materialize FixedTextRun-like records suitable for the existing PDF seam;
- emit only a sanitized public receipt.

Raw line text may exist in the local/private input because the downstream PDF
serializer needs ActualText. It is never copied into the public receipt.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
from typing import Any

UUID_RE = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
SHA_RE = re.compile(r"^[0-9a-f]{64}$")
HASH_ID_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
I64_MIN = -(1 << 63)
I64_MAX = (1 << 63) - 1


class FixedPdfShapedFlowBridgeError(ValueError):
    pass


def _canonical_json(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def _hash_id(value: Any) -> str:
    return "sha256:" + hashlib.sha256(_canonical_json(value)).hexdigest()


def _require_exact_keys(value: Any, expected: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise FixedPdfShapedFlowBridgeError(f"{label} must be an object")
    actual = set(value)
    if actual != expected:
        raise FixedPdfShapedFlowBridgeError(
            f"{label} fields mismatch: missing={sorted(expected-actual)} extra={sorted(actual-expected)}"
        )
    return value


def _require_int(value: Any, label: str, *, minimum: int | None = None) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise FixedPdfShapedFlowBridgeError(f"{label} must be an integer")
    if minimum is not None and value < minimum:
        raise FixedPdfShapedFlowBridgeError(f"{label} must be >= {minimum}")
    return value


def _require_i64(value: Any, label: str) -> int:
    value = _require_int(value, label)
    if value < I64_MIN or value > I64_MAX:
        raise FixedPdfShapedFlowBridgeError(f"{label} is outside signed 64-bit range")
    return value


def _checked_i64_add(left: int, right: int, label: str) -> int:
    value = left + right
    if value < I64_MIN or value > I64_MAX:
        raise FixedPdfShapedFlowBridgeError(f"{label} overflow")
    return value


def _checked_i64_mul(left: int, right: int, label: str) -> int:
    value = left * right
    if value < I64_MIN or value > I64_MAX:
        raise FixedPdfShapedFlowBridgeError(f"{label} overflow")
    return value


def _require_uuid(value: Any, label: str) -> str:
    if not isinstance(value, str) or not UUID_RE.fullmatch(value):
        raise FixedPdfShapedFlowBridgeError(f"{label} must be canonical lowercase UUID")
    return value


def _require_source_hash(value: Any) -> str:
    if not isinstance(value, str) or not SHA_RE.fullmatch(value):
        raise FixedPdfShapedFlowBridgeError("source_hash must be lowercase SHA-256")
    return value


def _require_hash_id(value: Any, label: str) -> str:
    if not isinstance(value, str) or not HASH_ID_RE.fullmatch(value):
        raise FixedPdfShapedFlowBridgeError(f"{label} must be sha256:<hex>")
    return value


def _validate_glyph(glyph: Any, label: str) -> dict[str, int]:
    glyph = _require_exact_keys(
        glyph,
        {"glyph_id", "cluster", "x_advance", "y_advance", "x_offset", "y_offset"},
        label,
    )
    return {
        "glyph_id": _require_int(glyph["glyph_id"], f"{label}.glyph_id", minimum=0),
        "cluster": _require_int(glyph["cluster"], f"{label}.cluster", minimum=0),
        "x_advance": _require_i64(glyph["x_advance"], f"{label}.x_advance"),
        "y_advance": _require_i64(glyph["y_advance"], f"{label}.y_advance"),
        "x_offset": _require_i64(glyph["x_offset"], f"{label}.x_offset"),
        "y_offset": _require_i64(glyph["y_offset"], f"{label}.y_offset"),
    }


def _glyph_sequence_hash(glyphs: list[dict[str, int]]) -> str:
    return _hash_id(glyphs)


def _validate_line(line: Any, index: int) -> dict[str, Any]:
    label = f"lines[{index}]"
    line = _require_exact_keys(
        line,
        {
            "frame_node_id",
            "story_id",
            "frame_line_index",
            "scalar_start",
            "scalar_end",
            "consumed_scalar_end",
            "text",
            "units_per_em",
            "measured_width",
            "glyphs",
        },
        label,
    )

    frame_node_id = _require_uuid(line["frame_node_id"], f"{label}.frame_node_id")
    story_id = _require_uuid(line["story_id"], f"{label}.story_id")
    frame_line_index = _require_int(line["frame_line_index"], f"{label}.frame_line_index", minimum=0)
    scalar_start = _require_int(line["scalar_start"], f"{label}.scalar_start", minimum=0)
    scalar_end = _require_int(line["scalar_end"], f"{label}.scalar_end", minimum=0)
    consumed_scalar_end = _require_int(
        line["consumed_scalar_end"],
        f"{label}.consumed_scalar_end",
        minimum=0,
    )
    if scalar_end < scalar_start:
        raise FixedPdfShapedFlowBridgeError(f"{label} scalar range is inverted")
    if consumed_scalar_end < scalar_end:
        raise FixedPdfShapedFlowBridgeError(
            f"{label}.consumed_scalar_end cannot precede scalar_end"
        )

    text = line["text"]
    if not isinstance(text, str):
        raise FixedPdfShapedFlowBridgeError(f"{label}.text must be a string")
    if len(text) != scalar_end - scalar_start:
        raise FixedPdfShapedFlowBridgeError(
            f"{label}.text scalar length must equal scalar_end - scalar_start"
        )

    units_per_em = _require_int(line["units_per_em"], f"{label}.units_per_em", minimum=1)
    measured_width = _require_i64(line["measured_width"], f"{label}.measured_width")
    if measured_width < 0:
        raise FixedPdfShapedFlowBridgeError(f"{label}.measured_width must be >= 0")

    glyph_values = line["glyphs"]
    if not isinstance(glyph_values, list):
        raise FixedPdfShapedFlowBridgeError(f"{label}.glyphs must be an array")
    glyphs = [_validate_glyph(glyph, f"{label}.glyphs[{i}]") for i, glyph in enumerate(glyph_values)]

    for i, glyph in enumerate(glyphs):
        cluster = glyph["cluster"]
        if cluster < scalar_start or cluster >= scalar_end:
            raise FixedPdfShapedFlowBridgeError(
                f"{label}.glyphs[{i}].cluster must stay inside Story-global visible scalar range"
            )

    return {
        "frame_node_id": frame_node_id,
        "story_id": story_id,
        "frame_line_index": frame_line_index,
        "scalar_start": scalar_start,
        "scalar_end": scalar_end,
        "consumed_scalar_end": consumed_scalar_end,
        "text": text,
        "units_per_em": units_per_em,
        "measured_width": measured_width,
        "glyphs": glyphs,
    }


def _prepare_run_mapping(run: dict[str, Any]) -> dict[int, str]:
    """Mirror the bounded PDF cluster mapping using scalar_base for local slicing."""
    chars = list(run["logical_text"])
    glyphs = run["glyphs"]
    if not glyphs:
        return {}

    local_clusters: list[int] = []
    for glyph in glyphs:
        global_cluster = glyph["cluster"]
        local = global_cluster - run["scalar_base"]
        if local < 0 or local >= len(chars):
            raise FixedPdfShapedFlowBridgeError(
                "Story-global glyph cluster cannot be mapped into run-local logical_text"
            )
        local_clusters.append(local)

    unique = sorted(set(local_clusters))
    if len(unique) != len(local_clusters):
        raise FixedPdfShapedFlowBridgeError(
            "bounded PDF one-CID mapping rejects duplicate glyph clusters"
        )

    next_cluster: dict[int, int] = {}
    for i, cluster in enumerate(unique):
        next_cluster[cluster] = unique[i + 1] if i + 1 < len(unique) else len(chars)

    mappings: dict[int, str] = {}
    for glyph, cluster in zip(glyphs, local_clusters):
        end = next_cluster[cluster]
        if end <= cluster or end > len(chars):
            raise FixedPdfShapedFlowBridgeError("invalid local glyph cluster span")
        text = "".join(chars[cluster:end])
        glyph_id = glyph["glyph_id"]
        prior = mappings.get(glyph_id)
        if prior is not None and prior != text:
            raise FixedPdfShapedFlowBridgeError(
                "glyph id has conflicting bounded ToUnicode mapping"
            )
        mappings[glyph_id] = text
    return mappings


def _validate_scene(scene: Any) -> dict[str, Any]:
    scene = _require_exact_keys(
        scene,
        {
            "schema_version",
            "source_hash",
            "flow_id",
            "environment",
            "lines",
            "diagnostics",
        },
        "scene",
    )
    if scene["schema_version"] != "chaptera.shaped-flow-bridge-input.v1":
        raise FixedPdfShapedFlowBridgeError("unsupported shaped-flow bridge input schema")

    source_hash = _require_source_hash(scene["source_hash"])
    flow_id = _require_hash_id(scene["flow_id"], "flow_id")

    environment = _require_exact_keys(
        scene["environment"],
        {"font_size_emu", "line_height_emu"},
        "environment",
    )
    font_size_emu = _require_i64(environment["font_size_emu"], "environment.font_size_emu")
    line_height_emu = _require_i64(environment["line_height_emu"], "environment.line_height_emu")
    if font_size_emu <= 0 or line_height_emu <= 0:
        raise FixedPdfShapedFlowBridgeError("font_size_emu and line_height_emu must be positive")

    raw_lines = scene["lines"]
    if not isinstance(raw_lines, list):
        raise FixedPdfShapedFlowBridgeError("lines must be an array")
    lines = [_validate_line(line, index) for index, line in enumerate(raw_lines)]

    keys = [(line["story_id"], line["scalar_start"], line["frame_node_id"]) for line in lines]
    if keys != sorted(keys):
        raise FixedPdfShapedFlowBridgeError(
            "resolved shaped-flow lines are not in canonical Story/scalar/frame order"
        )
    if len(keys) != len(set(keys)):
        raise FixedPdfShapedFlowBridgeError("duplicate resolved shaped-flow line identity")

    diagnostics = scene["diagnostics"]
    if not isinstance(diagnostics, list):
        raise FixedPdfShapedFlowBridgeError("diagnostics must be an array")
    for index, diagnostic in enumerate(diagnostics):
        if not isinstance(diagnostic, dict) or not isinstance(diagnostic.get("code"), str):
            raise FixedPdfShapedFlowBridgeError(
                f"diagnostics[{index}] must contain string code"
            )

    return {
        "source_hash": source_hash,
        "flow_id": flow_id,
        "font_size_emu": font_size_emu,
        "line_height_emu": line_height_emu,
        "lines": lines,
        "diagnostics": diagnostics,
    }


def materialize_fixed_runs(scene: Any) -> list[dict[str, Any]]:
    normalized = _validate_scene(scene)
    runs: list[dict[str, Any]] = []

    for line in normalized["lines"]:
        row_offset = _checked_i64_mul(
            line["frame_line_index"],
            normalized["line_height_emu"],
            "baseline row offset",
        )
        baseline_y = _checked_i64_add(
            row_offset,
            normalized["font_size_emu"],
            "baseline_y",
        )
        run = {
            "node_id": line["frame_node_id"],
            "story_id": line["story_id"],
            "scalar_base": line["scalar_start"],
            "scalar_end": line["scalar_end"],
            "logical_text": line["text"],
            "units_per_em": line["units_per_em"],
            "glyphs": [dict(glyph) for glyph in line["glyphs"]],
            "total_x_advance": line["measured_width"],
            "baseline_x": 0,
            "baseline_y": baseline_y,
        }
        _prepare_run_mapping(run)
        runs.append(run)

    return runs


def build_receipt(
    scene: Any,
    *,
    implementation: str,
    commit_or_build: str,
) -> dict[str, Any]:
    normalized = _validate_scene(scene)
    runs = materialize_fixed_runs(scene)

    if not re.fullmatch(r"[A-Za-z0-9._-]{1,128}", implementation):
        raise FixedPdfShapedFlowBridgeError("invalid implementation identifier")
    if not re.fullmatch(r"[A-Za-z0-9._:-]{1,160}", commit_or_build):
        raise FixedPdfShapedFlowBridgeError("invalid commit_or_build identifier")

    receipt_lines = []
    receipt_runs = []
    for index, (line, run) in enumerate(zip(normalized["lines"], runs)):
        glyph_hash = _glyph_sequence_hash(line["glyphs"])
        receipt_lines.append({
            "line_index": index,
            "frame_node_id": line["frame_node_id"],
            "story_id": line["story_id"],
            "scalar_start": line["scalar_start"],
            "scalar_end": line["scalar_end"],
            "glyph_count": len(line["glyphs"]),
            "glyph_sequence_hash": glyph_hash,
            "units_per_em": line["units_per_em"],
            "measured_width": line["measured_width"],
        })
        receipt_runs.append({
            "run_index": index,
            "frame_node_id": run["node_id"],
            "story_id": run["story_id"],
            "scalar_base": run["scalar_base"],
            "scalar_end": run["scalar_end"],
            "glyph_count": len(run["glyphs"]),
            "glyph_sequence_hash": _glyph_sequence_hash(run["glyphs"]),
            "baseline_x": run["baseline_x"],
            "baseline_y": run["baseline_y"],
        })

    return {
        "receipt_version": "chaptera.fixed-pdf-shaped-flow-receipt.v1",
        "producer": {
            "implementation": implementation,
            "commit_or_build": commit_or_build,
            "core_integration": True,
        },
        "source_hash": normalized["source_hash"],
        "flow_id": normalized["flow_id"],
        "lines": receipt_lines,
        "runs": receipt_runs,
        "story_overset": any(
            diagnostic.get("code") == "story_overset"
            for diagnostic in normalized["diagnostics"]
        ),
        "invariants": {
            "reshaping_calls": 0,
            "raw_text_emitted": False,
            "ascii_gate_applied": False,
            "overset_tail_painted": False,
            "line_order_preserved": True,
            "story_global_clusters_preserved": True,
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=pathlib.Path)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--implementation", default="rar-fixed-pdf-shaped-flow-bridge-v1")
    parser.add_argument("--commit-or-build", required=True)
    args = parser.parse_args()

    try:
        scene = json.loads(args.input.read_text(encoding="utf-8"))
        receipt = build_receipt(
            scene,
            implementation=args.implementation,
            commit_or_build=args.commit_or_build,
        )
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(
            json.dumps(receipt, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        print(json.dumps({
            "status": "valid",
            "receipt": str(args.output),
            "visible_line_count": len(receipt["lines"]),
            "fixed_run_count": len(receipt["runs"]),
            "story_overset": receipt["story_overset"],
        }, sort_keys=True))
        return 0
    except (OSError, json.JSONDecodeError, FixedPdfShapedFlowBridgeError) as error:
        print(str(error), file=__import__("sys").stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
