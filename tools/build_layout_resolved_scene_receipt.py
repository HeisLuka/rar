#!/usr/bin/env python3
"""Build LAYOUT-RESOLVED-SCENE-01 receipt from an authorized local producer.

Rar owns revision/history orchestration. The local producer owns canonical
EditorSession execution and projection of the current resolved graph into the
existing Scene contract. Only source-free scene state hashes/geometry enter the
public receipt.
"""

from __future__ import annotations

import argparse
import copy
import importlib.util
import json
import pathlib
import subprocess
import sys
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
REVISION_STORE = ROOT / "services" / "editor-api" / "revision_store.py"
TOOLS = ROOT / "tools"

MOVE_OPERATION_ID = "layout-move-00000001"
UNDO_OPERATION_ID = "layout-undo-00000001"
REDO_OPERATION_ID = "layout-redo-00000001"


def load_revision_store():
    # revision_store.py has source-neutral sibling modules (for example
    # story_range_v1.py). Direct importlib loading must preserve that package-local
    # import boundary instead of forcing revision_store's fallback loaders.
    editor_api_dir = str(REVISION_STORE.parent)
    if editor_api_dir not in sys.path:
        sys.path.insert(0, editor_api_dir)

    spec = importlib.util.spec_from_file_location("chaptera_revision_store", REVISION_STORE)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load RevisionKernel")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def invoke_producer(command: list[str], payload: dict[str, Any]) -> dict[str, Any]:
    completed = subprocess.run(
        command,
        input=json.dumps(payload, ensure_ascii=False, separators=(",", ":")),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            "resolved-scene producer failed"
            + (f"\nstderr:\n{completed.stderr}" if completed.stderr else "")
        )
    try:
        result = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError("resolved-scene producer returned invalid JSON") from error
    if not isinstance(result, dict):
        raise RuntimeError("resolved-scene producer response must be a JSON object")
    return result


def require_exact_keys(value: dict[str, Any], expected: set[str], label: str) -> None:
    actual = set(value)
    if actual != expected:
        raise RuntimeError(
            f"{label} fields mismatch: missing={sorted(expected - actual)} "
            f"extra={sorted(actual - expected)}"
        )


def assert_same_source(output: dict[str, Any], source_hash: str, label: str) -> None:
    if output.get("source_hash_after") != source_hash:
        raise RuntimeError(f"{label} changed immutable source identity")


def assert_no_reparse(output: dict[str, Any], label: str) -> None:
    if output.get("source_reparse_after_edit_count") != 0:
        raise RuntimeError(f"{label} reparsed immutable source after edit")


def build_receipt(
    producer_command: list[str],
    *,
    source_hash: str,
    document_id: str,
    implementation: str,
    commit_or_build: str,
) -> dict[str, Any]:
    revision_store = load_revision_store()
    kernel = revision_store.RevisionKernel()

    baseline_output = invoke_producer(
        producer_command,
        {"action": "baseline", "source_hash": source_hash},
    )
    require_exact_keys(
        baseline_output,
        {
            "source_hash",
            "baseline_project",
            "move_candidate",
            "baseline_scene_state",
            "baseline_equivalence",
            "adapter_invariants",
        },
        "baseline producer response",
    )
    if baseline_output["source_hash"] != source_hash:
        raise RuntimeError("baseline producer source hash mismatch")

    move_candidate = baseline_output["move_candidate"]
    require_exact_keys(
        move_candidate,
        {"node_id", "page_id", "before", "after"},
        "move_candidate",
    )
    baseline_record = kernel.register_baseline(
        document_id=document_id,
        source_hash=source_hash,
        project=copy.deepcopy(baseline_output["baseline_project"]),
    )

    move_request = {
        "protocol_version": "chaptera.commit-request.v1",
        "document_id": document_id,
        "source_hash": source_hash,
        "base_revision_id": baseline_record.revision_id,
        "client_operation_id": MOVE_OPERATION_ID,
        "command": {
            "kind": "move_node_to",
            "node_id": move_candidate["node_id"],
            "x_emu": move_candidate["after"]["x"],
            "y_emu": move_candidate["after"]["y"],
        },
    }

    commit_output: dict[str, Any] | None = None

    def move_executor(base_project: dict, command: dict):
        nonlocal commit_output
        commit_output = invoke_producer(
            producer_command,
            {
                "action": "commit",
                "source_hash": source_hash,
                "base_project": base_project,
                "command": command,
            },
        )
        require_exact_keys(
            commit_output,
            {
                "canonical_operation",
                "resulting_project",
                "consequences",
                "scene_state",
                "source_hash_after",
                "source_reparse_after_edit_count",
            },
            "commit producer response",
        )
        assert_same_source(commit_output, source_hash, "commit producer")
        assert_no_reparse(commit_output, "commit producer")
        return (
            copy.deepcopy(commit_output["canonical_operation"]),
            copy.deepcopy(commit_output["resulting_project"]),
            copy.deepcopy(commit_output["consequences"]),
        )

    accepted = kernel.commit_move(copy.deepcopy(move_request), move_executor)
    if accepted.get("protocol_version") != "chaptera.commit-accepted.v1":
        raise RuntimeError(f"MoveNode was not accepted: {accepted}")
    if commit_output is None:
        raise RuntimeError("MoveNode executor did not run")
    accepted_project = copy.deepcopy(commit_output["resulting_project"])

    history_states: dict[str, dict[str, Any]] = {}

    def history_executor(base_project: dict, transition_kind: str):
        output = invoke_producer(
            producer_command,
            {
                "action": "history",
                "source_hash": source_hash,
                "kind": transition_kind,
                "base_project": base_project,
            },
        )
        require_exact_keys(
            output,
            {
                "resulting_project",
                "scene_state",
                "consequences",
                "source_hash_after",
                "source_reparse_after_edit_count",
            },
            f"{transition_kind} producer response",
        )
        assert_same_source(output, source_hash, f"{transition_kind} producer")
        assert_no_reparse(output, f"{transition_kind} producer")
        history_states[transition_kind] = copy.deepcopy(output["scene_state"])
        return copy.deepcopy(output["resulting_project"]), copy.deepcopy(output["consequences"])

    undo_request = {
        "protocol_version": "chaptera.history-transition-intent.v1",
        "document_id": document_id,
        "source_hash": source_hash,
        "base_revision_id": accepted["revision_id"],
        "client_operation_id": UNDO_OPERATION_ID,
        "command": {"kind": "undo"},
    }
    undo = kernel.commit_history_transition(undo_request, history_executor)
    if undo.get("protocol_version") != "chaptera.history-transition-accepted.v1":
        raise RuntimeError(f"undo was not accepted: {undo}")
    if kernel.current_revision(document_id).project != baseline_record.project:
        raise RuntimeError("authoritative undo did not restore baseline project")

    redo_request = {
        "protocol_version": "chaptera.history-transition-intent.v1",
        "document_id": document_id,
        "source_hash": source_hash,
        "base_revision_id": undo["revision_id"],
        "client_operation_id": REDO_OPERATION_ID,
        "command": {"kind": "redo"},
    }
    redo = kernel.commit_history_transition(redo_request, history_executor)
    if redo.get("protocol_version") != "chaptera.history-transition-accepted.v1":
        raise RuntimeError(f"redo was not accepted: {redo}")
    if kernel.current_revision(document_id).project != accepted_project:
        raise RuntimeError("authoritative redo did not restore accepted project")

    replay_output = invoke_producer(
        producer_command,
        {
            "action": "replay",
            "source_hash": source_hash,
            "project": accepted_project,
        },
    )
    require_exact_keys(
        replay_output,
        {
            "replayed_project",
            "scene_state",
            "source_hash_after",
            "source_reparse_after_edit_count",
        },
        "replay producer response",
    )
    assert_same_source(replay_output, source_hash, "replay producer")
    assert_no_reparse(replay_output, "replay producer")
    if replay_output["replayed_project"] != accepted_project:
        raise RuntimeError("fresh replay project differs from accepted project")

    canonical_operation = commit_output["canonical_operation"]
    before = canonical_operation.get("before")
    after = canonical_operation.get("after")
    if before != move_candidate["before"]:
        raise RuntimeError("canonical MoveNode before-state differs from baseline candidate")
    if after != move_candidate["after"]:
        raise RuntimeError("canonical MoveNode after-state differs from accepted candidate")

    invariants = copy.deepcopy(baseline_output["adapter_invariants"])
    require_exact_keys(
        invariants,
        {
            "viewer_private_mapping_used",
            "browser_layout_authoritative",
            "second_geometry_model_created",
            "context_extension_seam_present",
            "graph_only_wrapper_is_empty_context",
        },
        "adapter_invariants",
    )

    receipt = {
        "receipt_version": "chaptera.layout-resolved-scene-receipt.v1",
        "producer": {
            "implementation": implementation,
            "commit_or_build": commit_or_build,
            "core_integration": True,
        },
        "source_hash": source_hash,
        "scene_protocol_version": "chaptera.scene.v1",
        "projection_api": "resolved_graph_adapter",
        "canonical_move": {
            "node_id": move_candidate["node_id"],
            "page_id": move_candidate["page_id"],
            "before": copy.deepcopy(before),
            "after": copy.deepcopy(after),
        },
        "states": {
            "baseline": copy.deepcopy(baseline_output["baseline_scene_state"]),
            "accepted": copy.deepcopy(commit_output["scene_state"]),
            "undo": history_states["undo"],
            "redo": history_states["redo"],
            "replay": copy.deepcopy(replay_output["scene_state"]),
        },
        "baseline_equivalence": copy.deepcopy(baseline_output["baseline_equivalence"]),
        "invariants": {
            "source_reparse_after_edit_count": 0,
            **invariants,
            "raw_source_bytes_emitted": False,
        },
    }

    sys.path.insert(0, str(TOOLS))
    from validate_layout_resolved_scene_receipt import validate_schema, validate_semantics

    validate_schema(receipt)
    validate_semantics(receipt)
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-hash", required=True)
    parser.add_argument("--document-id", required=True)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--implementation", default="chaptera-canonical-layout-adapter")
    parser.add_argument("--commit-or-build", required=True)
    parser.add_argument("producer_command", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    producer_command = list(args.producer_command)
    if producer_command and producer_command[0] == "--":
        producer_command = producer_command[1:]
    if not producer_command:
        parser.error("producer_command is required after --")

    receipt = build_receipt(
        producer_command,
        source_hash=args.source_hash,
        document_id=args.document_id,
        implementation=args.implementation,
        commit_or_build=args.commit_or_build,
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(receipt, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps({"receipt": str(args.output), "status": "valid"}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
