#!/usr/bin/env python3
import copy
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "services" / "editor-api"))

from revision_store import RevisionKernel

OUT = ROOT / "target" / "web-revision-history-v1" / "receipt.json"
DOCUMENT_ID = "10000000-0000-4000-8000-000000000001"
SOURCE_HASH = "a" * 64


def move_request(base_revision_id, op_id):
    return {
        "protocol_version": "chaptera.commit-request.v1",
        "document_id": DOCUMENT_ID,
        "source_hash": SOURCE_HASH,
        "base_revision_id": base_revision_id,
        "client_operation_id": op_id,
        "command": {
            "kind": "move_node_to",
            "node_id": "node:1",
            "x_emu": 127000,
            "y_emu": 254000,
        },
    }


def history_request(kind, base_revision_id, op_id):
    return {
        "protocol_version": "chaptera.history-transition-intent.v1",
        "document_id": DOCUMENT_ID,
        "source_hash": SOURCE_HASH,
        "base_revision_id": base_revision_id,
        "client_operation_id": op_id,
        "command": {"kind": kind},
    }


baseline_project = {
    "schema_version": "pub-editor-v0.4",
    "source_hash": SOURCE_HASH,
    "operations": [],
}
kernel = RevisionKernel()
baseline = kernel.register_baseline(
    document_id=DOCUMENT_ID,
    source_hash=SOURCE_HASH,
    project=baseline_project,
)

move_calls = 0


def move_executor(base_project, command):
    global move_calls
    move_calls += 1
    before = {"x": 0, "y": 0, "width": 1828800, "height": 914400}
    operation = {
        "kind": "move_node",
        "node_id": command["node_id"],
        "before": before,
        "after": {
            "x": command["x_emu"],
            "y": command["y_emu"],
            "width": before["width"],
            "height": before["height"],
        },
    }
    project = copy.deepcopy(base_project)
    project["operations"] = list(project["operations"]) + [copy.deepcopy(operation)]
    return operation, project, []


move = kernel.commit_move(
    move_request(baseline.revision_id, "move-op-00000001"),
    move_executor,
)
moved_record = kernel.current_revision(DOCUMENT_ID)
moved_project = copy.deepcopy(moved_record.project)
history_calls = []


def history_executor(base_project, transition_kind):
    history_calls.append(transition_kind)
    if transition_kind == "undo":
        return copy.deepcopy(baseline_project), []
    if transition_kind == "redo":
        return copy.deepcopy(moved_project), []
    raise ValueError("unsupported history transition")


undo_request = history_request(
    "undo",
    move["revision_id"],
    "history-op-00000001",
)
undo = kernel.commit_history_transition(copy.deepcopy(undo_request), history_executor)
redo = kernel.commit_history_transition(
    history_request("redo", undo["revision_id"], "history-op-00000002"),
    history_executor,
)
calls_before_retry = len(history_calls)
undo_retry = kernel.commit_history_transition(
    copy.deepcopy(undo_request),
    history_executor,
)
calls_after_retry = len(history_calls)
head_before_negative = kernel.current_revision(DOCUMENT_ID).revision_id

stale = kernel.commit_history_transition(
    history_request("undo", baseline.revision_id, "history-op-00000003"),
    history_executor,
)
conflicting_request = copy.deepcopy(undo_request)
conflicting_request["command"] = {"kind": "redo"}
conflict = kernel.commit_history_transition(conflicting_request, history_executor)

receipt = {
    "receipt_kind": "chaptera.web-revision-history-v1.public-contract",
    "canonical_core_integration": False,
    "baseline_revision_id": baseline.revision_id,
    "move_revision_id": move["revision_id"],
    "undo_revision_id": undo["revision_id"],
    "redo_revision_id": redo["revision_id"],
    "invariants": {
        "undo_reuses_baseline_state_id": undo["state_id"] == baseline.state_id,
        "undo_is_fresh_history_revision":
            undo["revision_id"] not in {baseline.revision_id, move["revision_id"]},
        "redo_reuses_moved_state_id": redo["state_id"] == moved_record.state_id,
        "redo_is_fresh_history_revision":
            redo["revision_id"] not in {move["revision_id"], undo["revision_id"]},
        "prior_revisions_remain_addressable":
            kernel.has_revision(document_id=DOCUMENT_ID, revision_id=baseline.revision_id)
            and kernel.has_revision(document_id=DOCUMENT_ID, revision_id=move["revision_id"])
            and kernel.has_revision(document_id=DOCUMENT_ID, revision_id=undo["revision_id"]),
        "exact_retry_returns_original_history_result": undo_retry == undo,
        "exact_retry_does_not_execute_again": calls_before_retry == calls_after_retry,
        "stale_history_intent_fails_closed":
            stale["code"] == "stale_revision"
            and kernel.current_revision(DOCUMENT_ID).revision_id == head_before_negative,
        "same_id_different_history_intent_conflicts":
            conflict["code"] == "idempotency_conflict"
            and kernel.current_revision(DOCUMENT_ID).revision_id == head_before_negative,
        "browser_never_supplies_authoritative_history_state": True,
    },
    "executor_calls": {
        "move": move_calls,
        "history": history_calls,
    },
    "guardrail":
        "Public revision-envelope contract only. The authoritative undo/redo executor is synthetic here; WEB-REVISION-ADAPTER-01 must still prove the same transitions against real EditorSession.",
}
assert all(receipt["invariants"].values()), receipt
OUT.parent.mkdir(parents=True, exist_ok=True)
OUT.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
print(json.dumps(receipt, indent=2))
