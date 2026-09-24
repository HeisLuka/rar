#!/usr/bin/env python3
"""Current Editor revision -> source-neutral fixed-output packet V1.

This bridge composes two already-owned seams:
- current resolved graph -> Scene (LAYOUT-RESOLVED-SCENE-01);
- resolved shaped-flow lines -> fixed text runs (FIXED-PDF-SHAPED-FLOW-01).

It does not parse PUB and does not serialize PDF. The output packet is the
bounded current-state input for the existing source-free PDF backend. The
public receipt is sanitized: no Story text, replacement text, source refs or
raw resources are emitted.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import pathlib
import re
import sys
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
EDITOR_API = ROOT / "services" / "editor-api"
for path in (str(TOOLS), str(EDITOR_API)):
    if path not in sys.path:
        sys.path.insert(0, path)

from fixed_pdf_shaped_flow_bridge_v1 import (  # noqa: E402
    FixedPdfShapedFlowBridgeError,
    build_receipt as build_shaped_flow_receipt,
    materialize_fixed_runs,
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

HASH_ID_RE = re.compile(r"^sha256:[0-9a-f]{64}$")


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
            f"{label} fields mismatch: missing={sorted(expected-actual)} extra={sorted(actual-expected)}"
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
    """Apply the bounded current EditorProject over one immutable resolved graph.

    Geometry operations reuse the existing resolved-graph Scene bridge law.
    Story operations reuse the canonical ReplaceStoryRange replay law. No source
    reparse occurs after edit.
    """
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


def _validate_shaped_flow_against_current_state(
    shaped_flow: dict[str, Any],
    *,
    source_hash: str,
    current_graph: dict[str, Any],
    current_scene: dict[str, Any],
) -> list[dict[str, Any]]:
    if not isinstance(shaped_flow, dict):
        raise EditorFixedOutputError("shaped_flow must be an object")
    if shaped_flow.get("source_hash") != source_hash:
        raise EditorFixedOutputError("shaped_flow source identity mismatch")

    # This call enforces the source-neutral line/run contract and all bounded PDF
    # cluster/baseline rules from FIXED-PDF-SHAPED-FLOW-01.
    runs = materialize_fixed_runs(shaped_flow)

    scene_node_origins = {
        node.get("origin")
        for node in current_scene.get("nodes", [])
        if isinstance(node, dict)
    }
    lines = shaped_flow.get("lines")
    if not isinstance(lines, list):
        raise EditorFixedOutputError("shaped_flow.lines must be an array")

    for index, line in enumerate(lines):
        story_id = line.get("story_id")
        frame_node_id = line.get("frame_node_id")
        start = line.get("scalar_start")
        end = line.get("scalar_end")
        visible_text = line.get("text")
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
            or start < 0
            or end < start
        ):
            raise EditorFixedOutputError(
                f"shaped_flow.lines[{index}] scalar range invalid"
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

    return runs


def _current_story_states(graph: dict[str, Any]) -> list[dict[str, Any]]:
    stories = graph.get("stories")
    if not isinstance(stories, dict):
        raise EditorFixedOutputError("resolved graph stories must be an object")
    result = []
    for story_id in sorted(stories):
        text = _story_text(graph, story_id, f"stories[{story_id}]")
        result.append({
            "story_id": story_id,
            "story_state_id": story_state_id_v1(story_id, text),
            "scalar_count": len(text),
        })
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
    fixed_runs = _validate_shaped_flow_against_current_state(
        shaped_flow,
        source_hash=source_hash,
        current_graph=current_graph,
        current_scene=current_scene,
    )

    shaped_receipt = build_shaped_flow_receipt(
        shaped_flow,
        implementation=implementation,
        commit_or_build=commit_or_build,
    )
    packet = {
        "schema_version": "chaptera.editor-fixed-output-packet.v1",
        "source_hash": source_hash,
        "project_hash": hash_id(project),
        "projection_context_hash": hash_id(context),
        "scene": current_scene,
        "fixed_text_runs": fixed_runs,
        "flow_id": shaped_receipt["flow_id"],
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
        "flow_id": shaped_receipt["flow_id"],
        "visible_line_count": len(shaped_receipt["lines"]),
        "fixed_run_count": len(shaped_receipt["runs"]),
        "story_overset": shaped_receipt["story_overset"],
        "current_story_states": story_states,
        "invariants": {
            "current_editor_project_authoritative": True,
            "source_reparse_after_edit_count": 0,
            "same_current_graph_feeds_scene_and_story_validation": True,
            "reshaping_calls": 0,
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
        # Packet is local/private renderer input and may contain logical text.
        args.packet_output.write_text(
            json.dumps(packet, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        # Receipt is public-safe and contains only identities/hashes/counts.
        args.receipt_output.write_text(
            json.dumps(receipt, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        print(json.dumps({
            "status": "valid",
            "packet_id": receipt["packet_id"],
            "scene_snapshot_id": receipt["scene_snapshot_id"],
            "fixed_run_count": receipt["fixed_run_count"],
            "receipt": str(args.receipt_output),
        }, sort_keys=True))
        return 0
    except (
        OSError,
        json.JSONDecodeError,
        EditorFixedOutputError,
        FixedPdfShapedFlowBridgeError,
        ResolvedGraphSceneError,
        StoryRangeError,
    ) as error:
        print(str(error), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
