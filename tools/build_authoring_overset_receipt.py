#!/usr/bin/env python3
"""Build AUTHORING-OVERSET-01 receipt from an authorized local producer.

Rar owns revision/idempotency orchestration. The external producer owns canonical
Story mutation, EditorSession history/replay, and authoritative bounded layout.
Only the final source-free receipt is written by this tool.
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

EDIT_OPERATION_ID = "overset-edit-00000001"
UNDO_OPERATION_ID = "overset-undo-00000001"
REDO_OPERATION_ID = "overset-redo-00000001"


def load_revision_store():
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
            "overset producer failed"
            + (f"\nstderr:\n{completed.stderr}" if completed.stderr else "")
        )
    try:
        result = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError("overset producer returned invalid JSON") from error
    if not isinstance(result, dict):
        raise RuntimeError("overset producer response must be a JSON object")
    return result


def require_exact_keys(value: dict[str, Any], expected: set[str], label: str) -> None:
    actual = set(value)
    if actual != expected:
        raise RuntimeError(
            f"{label} fields mismatch: missing={sorted(expected - actual)} "
            f"extra={sorted(actual - expected)}"
        )


def normalize_hash_id(value: str, label: str) -> str:
    if isinstance(value, str) and value.startswith("sha256:"):
        digest = value[7:]
    else:
        digest = value
    if (
        not isinstance(digest, str)
        or len(digest) != 64
        or any(ch not in "0123456789abcdef" for ch in digest)
    ):
        raise RuntimeError(f"{label} must be SHA-256")
    return "sha256:" + digest


def assert_same_source(output: dict[str, Any], source_hash: str, label: str) -> None:
    if output.get("source_hash_after") != source_hash:
        raise RuntimeError(f"{label} changed immutable source identity")


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
            "story_id",
            "frame_node_id",
            "baseline_layout_state",
            "edit_intent",
        },
        "baseline producer response",
    )
    if baseline_output["source_hash"] != source_hash:
        raise RuntimeError("baseline producer source hash mismatch")

    story_id = baseline_output["story_id"]
    frame_node_id = baseline_output["frame_node_id"]
    edit_intent = baseline_output["edit_intent"]
    require_exact_keys(
        edit_intent,
        {"start_scalar", "end_scalar", "replacement_text"},
        "edit_intent",
    )

    baseline_record = kernel.register_baseline(
        document_id=document_id,
        source_hash=source_hash,
        project=copy.deepcopy(baseline_output["baseline_project"]),
    )
    request = {
        "protocol_version": "chaptera.story-range-intent.v1",
        "document_id": document_id,
        "source_hash": source_hash,
        "base_revision_id": baseline_record.revision_id,
        "client_operation_id": EDIT_OPERATION_ID,
        "command": {
            "kind": "replace_story_range",
            "story_id": story_id,
            "start_scalar": edit_intent["start_scalar"],
            "end_scalar": edit_intent["end_scalar"],
            "replacement_text": edit_intent["replacement_text"],
        },
    }

    commit_output: dict[str, Any] | None = None

    def story_executor(base_project: dict, command: dict):
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
                "accepted_layout_state",
                "source_hash_after",
            },
            "commit producer response",
        )
        assert_same_source(commit_output, source_hash, "commit producer")
        return (
            copy.deepcopy(commit_output["canonical_operation"]),
            copy.deepcopy(commit_output["resulting_project"]),
            copy.deepcopy(commit_output["consequences"]),
        )

    accepted = kernel.commit_story_range(copy.deepcopy(request), story_executor)
    if accepted.get("protocol_version") != "chaptera.commit-accepted.v1":
        raise RuntimeError(f"Story edit was not accepted: {accepted}")
    if commit_output is None:
        raise RuntimeError("Story executor did not run")
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
            {"resulting_project", "layout_state", "consequences", "source_hash_after"},
            f"{transition_kind} producer response",
        )
        assert_same_source(output, source_hash, f"{transition_kind} producer")
        history_states[transition_kind] = copy.deepcopy(output["layout_state"])
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
            "layout_state",
            "editable_export_story_hash",
            "fixed_output_outcome",
            "source_hash_after",
        },
        "replay producer response",
    )
    assert_same_source(replay_output, source_hash, "replay producer")
    if replay_output["replayed_project"] != accepted_project:
        raise RuntimeError("fresh replay project differs from accepted project")

    unknown_output = invoke_producer(
        producer_command,
        {
            "action": "layout_unknown_probe",
            "source_hash": source_hash,
            "project": accepted_project,
        },
    )
    require_exact_keys(
        unknown_output,
        {"layout_state", "source_hash_after"},
        "layout_unknown producer response",
    )
    assert_same_source(unknown_output, source_hash, "layout_unknown producer")

    canonical_operation = commit_output["canonical_operation"]
    before_hash = normalize_hash_id(
        canonical_operation.get("before_text_hash"),
        "canonical before_text_hash",
    )
    after_hash = normalize_hash_id(
        canonical_operation.get("after_text_hash"),
        "canonical after_text_hash",
    )
    baseline_layout = copy.deepcopy(baseline_output["baseline_layout_state"])
    accepted_layout = copy.deepcopy(commit_output["accepted_layout_state"])

    receipt = {
        "receipt_version": "chaptera.authoring-overset-receipt.v1",
        "producer": {
            "implementation": implementation,
            "commit_or_build": commit_or_build,
            "core_integration": True,
        },
        "source_hash": source_hash,
        "story_id": story_id,
        "frame_node_id": frame_node_id,
        "canonical_edit": {
            "before_story_hash": before_hash,
            "after_story_hash": after_hash,
            "before_scalar_count": baseline_layout["scalar_count"],
            "after_scalar_count": accepted_layout["scalar_count"],
        },
        "states": {
            "baseline": baseline_layout,
            "accepted": accepted_layout,
            "undo": history_states["undo"],
            "redo": history_states["redo"],
            "replay": copy.deepcopy(replay_output["layout_state"]),
        },
        "layout_unknown_probe": copy.deepcopy(unknown_output["layout_state"]),
        "output_probe": {
            "editable_export_story_hash": normalize_hash_id(
                replay_output["editable_export_story_hash"],
                "editable_export_story_hash",
            ),
            "fixed_output_outcome": replay_output["fixed_output_outcome"],
            "overset_state_explicit": True,
        },
        "invariants": {
            "canonical_story_truncated": False,
            "autofit_mutation_count": 0,
            "source_write_count": 0,
            "linked_frame_flow_used": False,
            "host_font_fallback_used": False,
            "raw_story_text_emitted": False,
        },
    }

    sys.path.insert(0, str(TOOLS))
    from validate_authoring_overset_receipt import validate_schema, validate_semantics

    validate_schema(receipt)
    validate_semantics(receipt)
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-hash", required=True)
    parser.add_argument("--document-id", required=True)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--implementation", default="chaptera-canonical-authoring-layout")
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
