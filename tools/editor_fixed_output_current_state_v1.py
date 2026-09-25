#!/usr/bin/env python3
"""Current Editor revision -> source-neutral fixed-output packet V1.

This bridge composes two already-owned seams:
- current resolved graph -> Scene (LAYOUT-RESOLVED-SCENE-01);
- merged Rust resolved shaped-flow -> fixed text runs (FIXED-PDF-SHAPED-FLOW-01).

It does not parse PUB, shape text or serialize PDF. Private logical line text is
kept only in the local renderer packet. The public receipt is source-free.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import pathlib
import subprocess
import sys
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
EDITOR_API = ROOT / "services" / "editor-api"
for path in (str(TOOLS), str(EDITOR_API)):
    if path not in sys.path:
        sys.path.insert(0, path)

from cmo_slot_runtime_bridge_v1 import (  # noqa: E402
    CmoSlotRuntimeError,
    build_cmo_slot_runtime_v1,
    merge_cmo_runtime_into_scene_v1,
)
from resolved_graph_scene_bridge_v1 import (  # noqa: E402
    ResolvedGraphSceneError,
    apply_project_to_resolved_graph,
    project_resolved_graph_scene,
    scene_geometry_hash,
    scene_snapshot_id,
    source_hash_from_graph,
)
from story_range_v1 import (  # noqa: E402
    StoryRangeError,
    replay_story_range_operation_v1,
    story_state_id_v1,
    validate_scalar_sequence_v1,
)

RUST_PACKET_VERSION = "chaptera.fixed-flow-adapter-packet.v1"


class EditorFixedOutputError(ValueError):
    pass


def canonical_json(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def hash_id(value: Any) -> str:
    return "sha256:" + hashlib.sha256(canonical_json(value)).hexdigest()


def _require_exact_keys(value: Any, expected: set[str], label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise EditorFixedOutputError(f"{label} must be an object")
    actual = set(value)
    if actual != expected:
        raise EditorFixedOutputError(
            f"{label} fields mismatch: missing={sorted(expected-actual)} "
            f"extra={sorted(actual-expected)}"
        )
    return value


def _require_project(project: Any, source_hash: str) -> dict[str, Any]:
    project = _require_exact_keys(
        project,
        {"schema_version", "source_hash", "operations"},
        "EditorProject",
    )
    if project["schema_version"] not in {
        "pub-editor-v0.2",
        "pub-editor-v0.3",
        "pub-editor-v0.4",
    }:
        raise EditorFixedOutputError("unsupported EditorProject schema_version")
    if project["source_hash"] != source_hash:
        raise EditorFixedOutputError("EditorProject source identity mismatch")
    if not isinstance(project["operations"], list):
        raise EditorFixedOutputError("EditorProject.operations must be an array")
    return copy.deepcopy(project)


def _story_text(graph: dict[str, Any], story_id: str, label: str) -> str:
    stories = graph.get("stories")
    if not isinstance(stories, dict):
        raise EditorFixedOutputError("resolved graph stories must be an object")
    story = stories.get(story_id)
    if not isinstance(story, dict) or story.get("id") != story_id:
        raise EditorFixedOutputError(f"{label} references unknown Story")
    text = story.get("text")
    if not isinstance(text, str):
        raise EditorFixedOutputError(f"{label} Story text missing")
    validate_scalar_sequence_v1(text, f"{label}.story_text")
    return text


def apply_current_editor_project(
    baseline_graph: dict[str, Any],
    project: dict[str, Any],
) -> dict[str, Any]:
    source_hash = source_hash_from_graph(baseline_graph)
    project = _require_project(project, source_hash)
    graph = copy.deepcopy(baseline_graph)

    for index, operation in enumerate(project["operations"]):
        if not isinstance(operation, dict):
            raise EditorFixedOutputError(f"operation[{index}] must be an object")
        kind = operation.get("kind")

        if kind == "move_node":
            graph = apply_project_to_resolved_graph(
                graph,
                {"operations": [copy.deepcopy(operation)]},
            )
            continue

        if kind == "replace_story_range":
            story_id = operation.get("story_id")
            if not isinstance(story_id, str):
                raise EditorFixedOutputError(
                    f"operation[{index}] replace_story_range StoryId missing"
                )
            before = _story_text(graph, story_id, f"operation[{index}]")
            after = replay_story_range_operation_v1(
                story_text=before,
                operation=copy.deepcopy(operation),
                requires_terminal_cr=False,
            )
            graph["stories"][story_id]["text"] = after
            continue

        raise EditorFixedOutputError(
            f"operation[{index}] kind {kind!r} is outside current fixed-output V1"
        )

    if source_hash_from_graph(graph) != source_hash:
        raise EditorFixedOutputError("current project changed immutable source identity")
    return graph


def _normalized_projection_context(context: Any) -> dict[str, Any]:
    if context is None:
        return {
            "schema_version": "chaptera.pub-projection-context.v1",
            "master_relations": [],
            "cmo_relations": [],
        }
    context = _require_exact_keys(
        context,
        {"schema_version", "master_relations", "cmo_relations"},
        "projection_context",
    )
    return copy.deepcopy(context)


def _normalize_shaped_flow(
    shaped_flow: Any,
    *,
    source_hash: str,
    current_graph: dict[str, Any],
    current_scene: dict[str, Any],
) -> dict[str, Any]:
    shaped_flow = _require_exact_keys(
        shaped_flow,
        {
            "schema_version",
            "source_hash",
            "flow_id",
            "environment",
            "lines",
            "diagnostics",
        },
        "shaped_flow",
    )
    if shaped_flow["schema_version"] != "chaptera.shaped-flow-bridge-input.v1":
        raise EditorFixedOutputError("unsupported shaped_flow schema_version")
    if shaped_flow["source_hash"] != source_hash:
        raise EditorFixedOutputError("shaped_flow source identity mismatch")
    if not isinstance(shaped_flow["flow_id"], str) or not shaped_flow["flow_id"].startswith(
        "sha256:"
    ):
        raise EditorFixedOutputError("shaped_flow flow_id must be sha256 identity")

    environment = _require_exact_keys(
        shaped_flow["environment"],
        {"font_size_emu", "line_height_emu"},
        "shaped_flow.environment",
    )
    for key in ("font_size_emu", "line_height_emu"):
        if isinstance(environment[key], bool) or not isinstance(environment[key], int):
            raise EditorFixedOutputError(f"shaped_flow.environment.{key} must be integer")
        if environment[key] <= 0:
            raise EditorFixedOutputError(f"shaped_flow.environment.{key} must be positive")

    lines = shaped_flow["lines"]
    if not isinstance(lines, list):
        raise EditorFixedOutputError("shaped_flow.lines must be an array")
    diagnostics = shaped_flow["diagnostics"]
    if not isinstance(diagnostics, list):
        raise EditorFixedOutputError("shaped_flow.diagnostics must be an array")

    scene_node_origins = {
        node.get("origin")
        for node in current_scene.get("nodes", [])
        if isinstance(node, dict)
    }

    normalized_lines: list[dict[str, Any]] = []
    logical_texts: list[str] = []
    for index, line in enumerate(lines):
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
            f"shaped_flow.lines[{index}]",
        )
        story_id = line["story_id"]
        frame_node_id = line["frame_node_id"]
        start = line["scalar_start"]
        end = line["scalar_end"]
        consumed_end = line["consumed_scalar_end"]
        visible_text = line["text"]

        if not isinstance(story_id, str):
            raise EditorFixedOutputError(f"shaped_flow.lines[{index}] StoryId missing")
        if frame_node_id not in scene_node_origins:
            raise EditorFixedOutputError(
                f"shaped_flow.lines[{index}] frame is absent from current Scene"
            )
        if (
            not isinstance(start, int)
            or isinstance(start, bool)
            or not isinstance(end, int)
            or isinstance(end, bool)
            or not isinstance(consumed_end, int)
            or isinstance(consumed_end, bool)
            or start < 0
            or end < start
            or consumed_end < end
        ):
            raise EditorFixedOutputError(
                f"shaped_flow.lines[{index}] scalar range invalid"
            )
        if not isinstance(visible_text, str) or len(visible_text) != end - start:
            raise EditorFixedOutputError(
                f"shaped_flow.lines[{index}] visible text length mismatch"
            )

        current_text = _story_text(
            current_graph,
            story_id,
            f"shaped_flow.lines[{index}]",
        )
        if end > len(current_text):
            raise EditorFixedOutputError(
                f"shaped_flow.lines[{index}] exceeds current Story"
            )
        if current_text[start:end] != visible_text:
            raise EditorFixedOutputError(
                f"shaped_flow.lines[{index}] text is stale relative to current Story"
            )

        normalized_lines.append(
            {
                "line_index": index,
                "frame_line_index": line["frame_line_index"],
                "frame_node_id": frame_node_id,
                "story_id": story_id,
                "scalar_start": start,
                "scalar_end": end,
                "units_per_em": line["units_per_em"],
                "measured_width": line["measured_width"],
                "glyphs": copy.deepcopy(line["glyphs"]),
            }
        )
        logical_texts.append(visible_text)

    story_overset = any(
        isinstance(item, dict) and item.get("code") == "story_overset"
        for item in diagnostics
    )

    return {
        "native_flow_id": shaped_flow["flow_id"],
        "font_size_emu": environment["font_size_emu"],
        "line_height_emu": environment["line_height_emu"],
        "story_overset": story_overset,
        "lines": normalized_lines,
        "logical_texts": logical_texts,
    }


def _run_rust_fixed_flow_adapter(
    normalized: dict[str, Any],
    *,
    source_hash: str,
    implementation: str,
    commit_or_build: str,
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    adapter_input = {
        "producer": {
            "implementation": implementation,
            "commit_or_build": commit_or_build,
            "core_integration": True,
        },
        "source_hash": source_hash,
        "font_size_emu": normalized["font_size_emu"],
        "line_height_emu": normalized["line_height_emu"],
        "story_overset": normalized["story_overset"],
        "lines": normalized["lines"],
    }
    command = [
        "cargo",
        "run",
        "-q",
        "-p",
        "pub-fixed-flow-adapter",
        "--bin",
        "fixed-flow-packet",
    ]
    completed = subprocess.run(
        command,
        cwd=ROOT,
        input=json.dumps(adapter_input, ensure_ascii=False, separators=(",", ":")),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.strip()
        raise EditorFixedOutputError(
            "Rust fixed-flow adapter failed" + (f": {detail}" if detail else "")
        )
    try:
        packet = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise EditorFixedOutputError("Rust fixed-flow adapter returned invalid JSON") from error
    packet = _require_exact_keys(
        packet,
        {"protocol_version", "runs", "receipt"},
        "Rust fixed-flow packet",
    )
    if packet["protocol_version"] != RUST_PACKET_VERSION:
        raise EditorFixedOutputError("Rust fixed-flow packet protocol mismatch")
    if not isinstance(packet["runs"], list) or not isinstance(packet["receipt"], dict):
        raise EditorFixedOutputError("Rust fixed-flow packet payload invalid")
    return packet["runs"], packet["receipt"]


def _materialize_renderer_runs(
    rust_runs: list[dict[str, Any]],
    logical_texts: list[str],
) -> list[dict[str, Any]]:
    if len(rust_runs) != len(logical_texts):
        raise EditorFixedOutputError("Rust fixed-flow run count differs from visible lines")
    result: list[dict[str, Any]] = []
    for index, (run, logical_text) in enumerate(zip(rust_runs, logical_texts)):
        expected = {
            "run_index",
            "frame_node_id",
            "story_id",
            "scalar_base",
            "scalar_end",
            "units_per_em",
            "measured_width",
            "baseline_x",
            "baseline_y",
            "glyphs",
        }
        run = _require_exact_keys(run, expected, f"Rust fixed-flow runs[{index}]")
        if run["run_index"] != index:
            raise EditorFixedOutputError("Rust fixed-flow run order is not canonical")
        result.append(
            {
                "node_id": run["frame_node_id"],
                "story_id": run["story_id"],
                "scalar_base": run["scalar_base"],
                "scalar_end": run["scalar_end"],
                "logical_text": logical_text,
                "units_per_em": run["units_per_em"],
                "glyphs": copy.deepcopy(run["glyphs"]),
                "total_x_advance": run["measured_width"],
                "baseline_x": run["baseline_x"],
                "baseline_y": run["baseline_y"],
            }
        )
    return result


def _current_story_states(graph: dict[str, Any]) -> list[dict[str, Any]]:
    stories = graph.get("stories")
    if not isinstance(stories, dict):
        raise EditorFixedOutputError("resolved graph stories must be an object")
    result = []
    for story_id in sorted(stories):
        text = _story_text(graph, story_id, f"stories[{story_id}]")
        result.append(
            {
                "story_id": story_id,
                "story_state_id": story_state_id_v1(story_id, text),
                "scalar_count": len(text),
            }
        )
    return result


def build_current_fixed_output(
    *,
    baseline_graph: dict[str, Any],
    editor_project: dict[str, Any],
    projection_context: dict[str, Any] | None,
    shaped_flow: dict[str, Any],
    implementation: str,
    commit_or_build: str,
) -> tuple[dict[str, Any], dict[str, Any]]:
    source_hash = source_hash_from_graph(baseline_graph)
    project = _require_project(editor_project, source_hash)
    context = _normalized_projection_context(projection_context)

    current_graph = apply_current_editor_project(baseline_graph, project)
    current_scene = project_resolved_graph_scene(
        current_graph,
        context=context,
    )
    try:
        cmo_runtime = build_cmo_slot_runtime_v1(
            resolved_graph=current_graph,
            projection_context=context,
            shaped_flow=shaped_flow,
        )
        if cmo_runtime["native_outputs"] or cmo_runtime["scene_instances"]:
            current_scene = merge_cmo_runtime_into_scene_v1(
                current_scene,
                cmo_runtime,
            )
    except CmoSlotRuntimeError as error:
        raise EditorFixedOutputError(
            f"Cmo slot runtime failed: {error}"
        ) from error

    normalized = _normalize_shaped_flow(
        shaped_flow,
        source_hash=source_hash,
        current_graph=current_graph,
        current_scene=current_scene,
    )
    normalized["story_overset"] = (
        normalized["story_overset"] or cmo_runtime["story_overset"]
    )
    rust_runs, rust_receipt = _run_rust_fixed_flow_adapter(
        normalized,
        source_hash=source_hash,
        implementation=implementation,
        commit_or_build=commit_or_build,
    )
    fixed_runs = _materialize_renderer_runs(rust_runs, normalized["logical_texts"])

    if rust_receipt.get("source_hash") != source_hash:
        raise EditorFixedOutputError("Rust fixed-flow receipt source identity mismatch")
    if rust_receipt.get("story_overset") != normalized["story_overset"]:
        raise EditorFixedOutputError("Rust fixed-flow receipt overset mismatch")
    if len(rust_receipt.get("lines", [])) != len(normalized["lines"]):
        raise EditorFixedOutputError("Rust fixed-flow receipt line count mismatch")
    if len(rust_receipt.get("runs", [])) != len(fixed_runs):
        raise EditorFixedOutputError("Rust fixed-flow receipt run count mismatch")

    packet = {
        "schema_version": "chaptera.editor-fixed-output-packet.v1",
        "source_hash": source_hash,
        "project_hash": hash_id(project),
        "projection_context_hash": hash_id(context),
        "scene": current_scene,
        "fixed_text_runs": fixed_runs,
        "flow_id": normalized["native_flow_id"],
        "adapter_flow_id": rust_receipt["flow_id"],
    }

    story_states = _current_story_states(current_graph)
    receipt = {
        "receipt_version": "chaptera.editor-fixed-output-current-state-receipt.v1",
        "producer": {
            "implementation": implementation,
            "commit_or_build": commit_or_build,
            "core_integration": True,
        },
        "source_hash": source_hash,
        "project_hash": packet["project_hash"],
        "projection_context_hash": packet["projection_context_hash"],
        "packet_id": hash_id(packet),
        "scene_snapshot_id": scene_snapshot_id(current_scene),
        "scene_geometry_hash": scene_geometry_hash(current_scene),
        "flow_id": normalized["native_flow_id"],
        "adapter_flow_id": rust_receipt["flow_id"],
        "visible_line_count": len(rust_receipt["lines"]),
        "fixed_run_count": len(rust_receipt["runs"]),
        "story_overset": rust_receipt["story_overset"],
        "cmo_target_count": len(cmo_runtime["native_outputs"]),
        "cmo_visible_slot_count": len(cmo_runtime["scene_instances"]),
        "cmo_overset_story_count": sum(
            1
            for output in cmo_runtime["native_outputs"]
            if output["overset"]["story_overset"]
        ),
        "current_story_states": story_states,
        "invariants": {
            "current_editor_project_authoritative": True,
            "source_reparse_after_edit_count": 0,
            "same_current_graph_feeds_scene_and_story_validation": True,
            "rust_fixed_flow_adapter_authoritative": True,
            "canonical_cmo_slot_flow_authoritative": True,
            "cmo_carrier_reparent_count": sum(
                output["carrier_reparent_count"]
                for output in cmo_runtime["native_outputs"]
            ),
            "cmo_scaling_applied": any(
                output["scaling_applied"]
                for output in cmo_runtime["native_outputs"]
            ),
            "cmo_skip_to_fit": any(
                output["skip_to_fit"]
                for output in cmo_runtime["native_outputs"]
            ),
            "reshaping_calls": rust_receipt["invariants"]["reshaping_calls"],
            "raw_text_emitted": False,
            "native_pub_write_used": False,
            "pdf_renderer_reimplemented": False,
        },
    }
    return packet, receipt


def _load_input(path: pathlib.Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise EditorFixedOutputError("cannot load editor fixed-output input") from error
    value = _require_exact_keys(
        value,
        {
            "schema_version",
            "resolved_graph",
            "editor_project",
            "projection_context",
            "shaped_flow",
        },
        "input",
    )
    if value["schema_version"] != "chaptera.editor-fixed-output-input.v1":
        raise EditorFixedOutputError("unsupported editor fixed-output input schema")
    return value


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, type=pathlib.Path)
    parser.add_argument("--packet-output", required=True, type=pathlib.Path)
    parser.add_argument("--receipt-output", required=True, type=pathlib.Path)
    parser.add_argument(
        "--implementation",
        default="rar-editor-fixed-output-current-state-v1",
    )
    parser.add_argument("--commit-or-build", required=True)
    args = parser.parse_args()

    try:
        value = _load_input(args.input)
        packet, receipt = build_current_fixed_output(
            baseline_graph=value["resolved_graph"],
            editor_project=value["editor_project"],
            projection_context=value["projection_context"],
            shaped_flow=value["shaped_flow"],
            implementation=args.implementation,
            commit_or_build=args.commit_or_build,
        )
        args.packet_output.parent.mkdir(parents=True, exist_ok=True)
        args.receipt_output.parent.mkdir(parents=True, exist_ok=True)
        args.packet_output.write_text(
            json.dumps(packet, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        args.receipt_output.write_text(
            json.dumps(receipt, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        print(
            json.dumps(
                {
                    "status": "valid",
                    "packet_id": receipt["packet_id"],
                    "scene_snapshot_id": receipt["scene_snapshot_id"],
                    "fixed_run_count": receipt["fixed_run_count"],
                    "receipt": str(args.receipt_output),
                },
                sort_keys=True,
            )
        )
        return 0
    except (
        OSError,
        json.JSONDecodeError,
        EditorFixedOutputError,
        ResolvedGraphSceneError,
        StoryRangeError,
    ) as error:
        print(str(error), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
