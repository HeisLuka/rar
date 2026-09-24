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
from projection_context_sidecar_v1 import (
    ProjectionContextSidecarError,
    empty_sidecar,
    normalize_sidecar,
    scene_supported_context,
    sidecar_state,
)
from validate_viewer_geometry_receipt import validate_schema as validate_viewer_schema

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
    *,
    expected_resolved_graph_sha256: str,
) -> tuple[dict[str, Any], dict[str, Any]]:
    graph_path = resolved_graph_path.expanduser().resolve(strict=True)
    if sha256_path(graph_path) != expected_resolved_graph_sha256:
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
    projection_context: dict[str, Any],
) -> tuple[dict[str, Any], dict[str, Any]]:
    current_graph = apply_project_to_resolved_graph(graph, project)
    scene = project_resolved_graph_scene(
        current_graph,
        context=scene_supported_context(projection_context),
    )
    return current_graph, scene


def move_candidate(
    graph: dict[str, Any],
    *,
    target_node_id: str,
    target_page_id: str,
    after_x_emu: int,
    after_y_emu: int,
) -> dict[str, Any]:
    try:
        header = graph["nodes"][target_node_id]["header"]
    except (KeyError, TypeError) as error:
        raise SampleNewsletterSceneEngineError(
            "pinned MoveNode target missing from resolved graph"
        ) from error
    if header.get("parent_id") != target_page_id:
        raise SampleNewsletterSceneEngineError(
            "pinned MoveNode target is not directly page-owned"
        )
    before = copy.deepcopy(header.get("bounds"))
    if not isinstance(before, dict):
        raise SampleNewsletterSceneEngineError("pinned MoveNode bounds missing")
    after = {
        "x": after_x_emu,
        "y": after_y_emu,
        "width": before["width"],
        "height": before["height"],
    }
    return {
        "node_id": target_node_id,
        "page_id": target_page_id,
        "before": before,
        "after": after,
    }


def canonical_operation(
    graph: dict[str, Any],
    base_project: dict[str, Any],
    command: dict[str, Any],
    *,
    target_node_id: str,
) -> dict[str, Any]:
    if not isinstance(command, dict) or set(command) != {
        "kind",
        "node_id",
        "x_emu",
        "y_emu",
    }:
        raise SampleNewsletterSceneEngineError("MoveNodeTo command fields mismatch")
    if command["kind"] != "move_node_to" or command["node_id"] != target_node_id:
        raise SampleNewsletterSceneEngineError("unsupported MoveNodeTo target")
    current_graph = apply_project_to_resolved_graph(graph, base_project)
    header = current_graph["nodes"][target_node_id]["header"]
    before = copy.deepcopy(header["bounds"])
    after = {
        "x": command["x_emu"],
        "y": command["y_emu"],
        "width": before["width"],
        "height": before["height"],
    }
    return {
        "kind": "move_node",
        "node_id": target_node_id,
        "before": before,
        "after": after,
    }


def append_operation(
    base_project: dict[str, Any],
    operation: dict[str, Any],
) -> dict[str, Any]:
    if not isinstance(base_project.get("source_hash"), str):
        raise SampleNewsletterSceneEngineError("EditorProject source identity missing")
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
    parser.add_argument("--expected-resolved-graph-sha256", required=True)
    parser.add_argument("--target-node-id", required=True)
    parser.add_argument("--target-page-id", required=True)
    parser.add_argument("--after-x-emu", required=True, type=int)
    parser.add_argument("--after-y-emu", required=True, type=int)
    parser.add_argument("action", choices=["baseline", "commit", "history", "replay"])
    parser.add_argument("--state-dir", required=True, type=pathlib.Path)
    parser.add_argument("--fixture", type=pathlib.Path)
    parser.add_argument("--projection-context-sidecar", type=pathlib.Path)
    args = parser.parse_args()

    try:
        payload = json.load(sys.stdin)
        if not isinstance(payload, dict) or payload.get("action") != args.action:
            raise SampleNewsletterSceneEngineError("builder action mismatch")
        source_hash = payload.get("source_hash")
        if not isinstance(source_hash, str):
            raise SampleNewsletterSceneEngineError("builder source hash missing")

        graph, viewer = load_inputs(
            args.resolved_graph,
            args.viewer_receipt,
            source_hash,
            expected_resolved_graph_sha256=args.expected_resolved_graph_sha256,
        )
        if args.projection_context_sidecar is None:
            sidecar = normalize_sidecar(
                empty_sidecar(source_hash),
                expected_source_hash=source_hash,
            )
        else:
            sidecar = normalize_sidecar(
                load_json(
                    args.projection_context_sidecar.expanduser().resolve(strict=True),
                    "projection context sidecar",
                ),
                expected_source_hash=source_hash,
            )
        projection_context = sidecar["context"]
        projection_context_state = sidecar_state(sidecar)
        args.state_dir.mkdir(parents=True, exist_ok=True)

        if args.action == "baseline":
            if args.fixture is None:
                raise SampleNewsletterSceneEngineError(
                    "baseline requires launcher-verified fixture"
                )
            project = baseline_project(source_hash)
            _, scene = project_for_scene(graph, project, projection_context)
            equivalence = compare_viewer_and_adapter_scene(viewer, scene)
            candidate = move_candidate(
                graph,
                target_node_id=args.target_node_id,
                target_page_id=args.target_page_id,
                after_x_emu=args.after_x_emu,
                after_y_emu=args.after_y_emu,
            )
            return emit({
                "source_hash": source_hash,
                "baseline_project": project,
                "move_candidate": candidate,
                "baseline_scene_state": compact_scene_state(
                    scene,
                    node_id=args.target_node_id,
                    page_id=args.target_page_id,
                ),
                "baseline_equivalence": equivalence,
                "projection_context_state": copy.deepcopy(projection_context_state),
                "adapter_invariants": {
                    "viewer_private_mapping_used": False,
                    "browser_layout_authoritative": False,
                    "second_geometry_model_created": False,
                    "context_extension_seam_present": True,
                    "graph_only_wrapper_is_empty_context": (
                        projection_context_state["master_relation_count"] == 0
                        and projection_context_state["cmo_relation_count"] == 0
                    ),
                    "projection_context_carried_outside_editor_project": True,
                    "unsupported_cmo_layout_deferred": True,
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
            operation = canonical_operation(
                graph,
                base_project,
                command,
                target_node_id=args.target_node_id,
            )
            candidate = move_candidate(
                graph,
                target_node_id=args.target_node_id,
                target_page_id=args.target_page_id,
                after_x_emu=args.after_x_emu,
                after_y_emu=args.after_y_emu,
            )
            if operation["before"] != candidate["before"] or operation["after"] != candidate["after"]:
                raise SampleNewsletterSceneEngineError(
                    "commit does not match pinned Producer B MoveNode"
                )
            result = append_operation(base_project, operation)
            _, scene = project_for_scene(graph, result, projection_context)
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
                    node_id=args.target_node_id,
                    page_id=args.target_page_id,
                ),
                "source_hash_after": source_hash,
                "source_reparse_after_edit_count": 0,
                "projection_context_state": copy.deepcopy(projection_context_state),
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
            _, scene = project_for_scene(graph, result, projection_context)
            return emit({
                "resulting_project": result,
                "scene_state": compact_scene_state(
                    scene,
                    node_id=args.target_node_id,
                    page_id=args.target_page_id,
                ),
                "consequences": [{
                    "key": "history." + kind,
                    "state": "supported",
                    "note": None,
                }],
                "source_hash_after": source_hash,
                "source_reparse_after_edit_count": 0,
                "projection_context_state": copy.deepcopy(projection_context_state),
            })

        project = payload.get("project")
        if not isinstance(project, dict):
            raise SampleNewsletterSceneEngineError("replay project missing")
        _, scene = project_for_scene(graph, project, projection_context)
        return emit({
            "replayed_project": copy.deepcopy(project),
            "scene_state": compact_scene_state(
                scene,
                node_id=args.target_node_id,
                page_id=args.target_page_id,
            ),
            "source_hash_after": source_hash,
            "source_reparse_after_edit_count": 0,
            "projection_context_state": copy.deepcopy(projection_context_state),
        })
    except (
        SampleNewsletterSceneEngineError,
        ProjectionContextSidecarError,
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
