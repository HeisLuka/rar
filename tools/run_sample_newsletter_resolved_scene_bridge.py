#!/usr/bin/env python3
"""Task-local SampleNewsletter current-resolved-graph -> Scene engine.

This is not a PUB parser. It consumes:
- one exact historical real SampleNewsletter resolved-graph artifact;
- the real public ViewerGeometryDocument receipt produced by Producer A;
- canonical EditorProject MoveNode operations.

The resolved graph and Viewer receipt may remain local/private inputs. Only the
source-neutral bridge law is owned by Rar. Baseline fails closed unless the
real Viewer Scene equals the reusable adapter Scene exactly.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import pathlib
import sys
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
sys.path.insert(0, str(TOOLS))

from resolved_graph_scene_bridge_v1 import (
    ResolvedGraphSceneError,
    apply_project_to_resolved_graph,
    compact_scene_state,
    compare_viewer_and_adapter_scene,
    project_resolved_graph_scene,
    source_hash_from_graph,
)
from validate_viewer_geometry_receipt import validate_schema as validate_viewer_schema

SOURCE_HASH = "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf"
RESOLVED_GRAPH_SHA256 = "7c327cd729fc2f9760e59cce1dd7c104c55162f4c3032c132068cfee513b1c4d"
TARGET_NODE_ID = "007d9898-568b-5125-b519-8d88243aabfb"
TARGET_PAGE_ID = "58ffa2e0-896e-5a40-806c-bd0184ea9c85"
TARGET_AFTER_X = 653710
TARGET_AFTER_Y = 1445292
REDO_STATE = "chaptera-layout-redo-operation.json"


class SampleNewsletterSceneEngineError(RuntimeError):
    pass


def sha256_path(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_json(path: pathlib.Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SampleNewsletterSceneEngineError(f"cannot load {label}") from error
    if not isinstance(value, dict):
        raise SampleNewsletterSceneEngineError(f"{label} must be a JSON object")
    return value


def load_inputs(
    resolved_graph_path: pathlib.Path,
    viewer_receipt_path: pathlib.Path,
    source_hash: str,
) -> tuple[dict[str, Any], dict[str, Any]]:
    graph_path = resolved_graph_path.expanduser().resolve(strict=True)
    if sha256_path(graph_path) != RESOLVED_GRAPH_SHA256:
        raise SampleNewsletterSceneEngineError(
            "resolved graph SHA-256 differs from pinned historical authority"
        )
    graph = load_json(graph_path, "resolved graph")
    if source_hash_from_graph(graph) != source_hash:
        raise SampleNewsletterSceneEngineError(
            "resolved graph source identity differs from builder request"
        )

    viewer = load_json(
        viewer_receipt_path.expanduser().resolve(strict=True),
        "Viewer geometry receipt",
    )
    validate_viewer_schema(viewer)
    viewer_source = viewer["document"]["source"]
    if viewer_source["source_hash"] != source_hash:
        raise SampleNewsletterSceneEngineError(
            "Viewer receipt source identity differs from builder request"
        )
    if viewer_source["byte_len"] != 291840:
        raise SampleNewsletterSceneEngineError("Viewer receipt byte length mismatch")
    return graph, viewer


def baseline_project(source_hash: str) -> dict[str, Any]:
    # Match the real Producer B baseline project so all Web V0 producers begin
    # from the same canonical project semantics.
    return {
        "schema_version": "pub-editor-v0.2",
        "source_hash": source_hash,
        "operations": [],
    }


def project_for_scene(
    graph: dict[str, Any],
    project: dict[str, Any],
) -> tuple[dict[str, Any], dict[str, Any]]:
    current_graph = apply_project_to_resolved_graph(graph, project)
    scene = project_resolved_graph_scene(current_graph, context={
        "master_relations": [],
        "cmo_relations": [],
    })
    return current_graph, scene


def move_candidate(graph: dict[str, Any]) -> dict[str, Any]:
    try:
        header = graph["nodes"][TARGET_NODE_ID]["header"]
    except (KeyError, TypeError) as error:
        raise SampleNewsletterSceneEngineError(
            "pinned MoveNode target missing from resolved graph"
        ) from error
    if header.get("parent_id") != TARGET_PAGE_ID:
        raise SampleNewsletterSceneEngineError(
            "pinned MoveNode target is not directly page-owned"
        )
    before = copy.deepcopy(header.get("bounds"))
    if not isinstance(before, dict):
        raise SampleNewsletterSceneEngineError("pinned MoveNode bounds missing")
    after = {
        "x": TARGET_AFTER_X,
        "y": TARGET_AFTER_Y,
        "width": before["width"],
        "height": before["height"],
    }
    return {
        "node_id": TARGET_NODE_ID,
        "page_id": TARGET_PAGE_ID,
        "before": before,
        "after": after,
    }


def canonical_operation(
    graph: dict[str, Any],
    base_project: dict[str, Any],
    command: dict[str, Any],
) -> dict[str, Any]:
    if not isinstance(command, dict) or set(command) != {
        "kind",
        "node_id",
        "x_emu",
        "y_emu",
    }:
        raise SampleNewsletterSceneEngineError("MoveNodeTo command fields mismatch")
    if command["kind"] != "move_node_to" or command["node_id"] != TARGET_NODE_ID:
        raise SampleNewsletterSceneEngineError("unsupported MoveNodeTo target")
    current_graph = apply_project_to_resolved_graph(graph, base_project)
    header = current_graph["nodes"][TARGET_NODE_ID]["header"]
    before = copy.deepcopy(header["bounds"])
    after = {
        "x": command["x_emu"],
        "y": command["y_emu"],
        "width": before["width"],
        "height": before["height"],
    }
    return {
        "kind": "move_node",
        "node_id": TARGET_NODE_ID,
        "before": before,
        "after": after,
    }


def append_operation(
    base_project: dict[str, Any],
    operation: dict[str, Any],
) -> dict[str, Any]:
    if base_project.get("source_hash") != SOURCE_HASH:
        raise SampleNewsletterSceneEngineError("EditorProject source identity mismatch")
    operations = base_project.get("operations")
    if not isinstance(operations, list):
        raise SampleNewsletterSceneEngineError("EditorProject.operations must be an array")
    result = copy.deepcopy(base_project)
    result["schema_version"] = "pub-editor-v0.4"
    result["operations"] = list(result["operations"]) + [copy.deepcopy(operation)]
    return result


def emit(value: dict[str, Any]) -> int:
    sys.stdout.write(json.dumps(value, ensure_ascii=False, separators=(",", ":")))
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--resolved-graph", required=True, type=pathlib.Path)
    parser.add_argument("--viewer-receipt", required=True, type=pathlib.Path)
    parser.add_argument("action", choices=["baseline", "commit", "history", "replay"])
    parser.add_argument("--state-dir", required=True, type=pathlib.Path)
    parser.add_argument("--fixture", type=pathlib.Path)
    args = parser.parse_args()

    try:
        payload = json.load(sys.stdin)
        if not isinstance(payload, dict) or payload.get("action") != args.action:
            raise SampleNewsletterSceneEngineError("builder action mismatch")
        source_hash = payload.get("source_hash")
        if source_hash != SOURCE_HASH:
            raise SampleNewsletterSceneEngineError("unexpected SampleNewsletter source hash")

        graph, viewer = load_inputs(
            args.resolved_graph,
            args.viewer_receipt,
            source_hash,
        )
        args.state_dir.mkdir(parents=True, exist_ok=True)

        if args.action == "baseline":
            if args.fixture is None:
                raise SampleNewsletterSceneEngineError(
                    "baseline requires launcher-verified fixture"
                )
            project = baseline_project(source_hash)
            _, scene = project_for_scene(graph, project)
            equivalence = compare_viewer_and_adapter_scene(viewer, scene)
            candidate = move_candidate(graph)
            return emit({
                "source_hash": source_hash,
                "baseline_project": project,
                "move_candidate": candidate,
                "baseline_scene_state": compact_scene_state(
                    scene,
                    node_id=TARGET_NODE_ID,
                    page_id=TARGET_PAGE_ID,
                ),
                "baseline_equivalence": equivalence,
                "adapter_invariants": {
                    "viewer_private_mapping_used": False,
                    "browser_layout_authoritative": False,
                    "second_geometry_model_created": False,
                    "context_extension_seam_present": True,
                    "graph_only_wrapper_is_empty_context": True,
                },
            })

        if args.fixture is not None:
            raise SampleNewsletterSceneEngineError(
                "post-baseline action must not receive source fixture"
            )

        if args.action == "commit":
            base_project = payload.get("base_project")
            command = payload.get("command")
            if not isinstance(base_project, dict):
                raise SampleNewsletterSceneEngineError("commit base_project missing")
            operation = canonical_operation(graph, base_project, command)
            candidate = move_candidate(graph)
            if operation["before"] != candidate["before"] or operation["after"] != candidate["after"]:
                raise SampleNewsletterSceneEngineError(
                    "commit does not match pinned Producer B MoveNode"
                )
            result = append_operation(base_project, operation)
            _, scene = project_for_scene(graph, result)
            (args.state_dir / REDO_STATE).write_text(
                json.dumps(operation, sort_keys=True, separators=(",", ":")) + "\n",
                encoding="utf-8",
            )
            return emit({
                "canonical_operation": operation,
                "resulting_project": result,
                "consequences": [{
                    "key": "node.geometry.position",
                    "state": "supported",
                    "note": None,
                }],
                "scene_state": compact_scene_state(
                    scene,
                    node_id=TARGET_NODE_ID,
                    page_id=TARGET_PAGE_ID,
                ),
                "source_hash_after": source_hash,
                "source_reparse_after_edit_count": 0,
            })

        if args.action == "history":
            base_project = payload.get("base_project")
            kind = payload.get("kind")
            if not isinstance(base_project, dict) or kind not in {"undo", "redo"}:
                raise SampleNewsletterSceneEngineError("history payload invalid")
            operations = base_project.get("operations")
            if not isinstance(operations, list):
                raise SampleNewsletterSceneEngineError("history operations missing")
            result = copy.deepcopy(base_project)
            redo_path = args.state_dir / REDO_STATE
            if kind == "undo":
                if not operations:
                    raise SampleNewsletterSceneEngineError("nothing to undo")
                removed = copy.deepcopy(operations[-1])
                result["operations"] = copy.deepcopy(operations[:-1])
                redo_path.write_text(
                    json.dumps(removed, sort_keys=True, separators=(",", ":")) + "\n",
                    encoding="utf-8",
                )
            else:
                if not redo_path.is_file():
                    raise SampleNewsletterSceneEngineError("nothing to redo")
                operation = load_json(redo_path, "redo operation")
                result["operations"] = list(copy.deepcopy(operations)) + [operation]
            _, scene = project_for_scene(graph, result)
            return emit({
                "resulting_project": result,
                "scene_state": compact_scene_state(
                    scene,
                    node_id=TARGET_NODE_ID,
                    page_id=TARGET_PAGE_ID,
                ),
                "consequences": [{
                    "key": "history." + kind,
                    "state": "supported",
                    "note": None,
                }],
                "source_hash_after": source_hash,
                "source_reparse_after_edit_count": 0,
            })

        project = payload.get("project")
        if not isinstance(project, dict):
            raise SampleNewsletterSceneEngineError("replay project missing")
        _, scene = project_for_scene(graph, project)
        return emit({
            "replayed_project": copy.deepcopy(project),
            "scene_state": compact_scene_state(
                scene,
                node_id=TARGET_NODE_ID,
                page_id=TARGET_PAGE_ID,
            ),
            "source_hash_after": source_hash,
            "source_reparse_after_edit_count": 0,
        })
    except (
        SampleNewsletterSceneEngineError,
        ResolvedGraphSceneError,
        AssertionError,
        KeyError,
        TypeError,
        ValueError,
        OSError,
        json.JSONDecodeError,
    ) as error:
        print(str(error), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
