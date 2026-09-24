#!/usr/bin/env python3
"""Build the real WEB-REVISION-ADAPTER-01 receipt from a local canonical-core executor.

This tool deliberately reuses Rar's existing RevisionKernel for revision ids,
stale-base rejection and idempotency semantics. The private/local producer owns
only canonical EditorSession execution.

Producer protocol (JSON over stdin/stdout):

1. Baseline request:
   {"action":"baseline","source_hash":"<pinned sha256>"}

   Response must contain exactly:
   {
     "source_hash":"<same sha256>",
     "baseline_project":{...},
     "move_candidate":{
       "node_id":"<canonical uuid>",
       "before":{"x":0,"y":0,"width":1,"height":1}
     }
   }

2. Commit request:
   {
     "action":"commit",
     "source_hash":"<pinned sha256>",
     "base_project":{...},
     "command":{"kind":"move_node_to",...}
   }

   Response must contain exactly:
   {
     "canonical_operation":{...},
     "resulting_project":{...},
     "replayed_project":{...},
     "consequences":[...],
     "source_hash_after":"<same sha256>",
     "source_hash_replay":"<same sha256>"
   }

The commit action must call the real EditorSession::move_node_to exactly once,
then prove replay by opening a fresh EditorSession from the same immutable PUB
and apply_project(resulting_project).
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

PINNED_SOURCE_HASH = "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf"
PINNED_DOCUMENT_ID = "9c2f8d5e-3f1b-4a57-9d40-6a7e2c11b001"
PRIMARY_OPERATION_ID = "9c2f8d5e-3f1b-4a57-9d40-6a7e2c11b002"
STALE_OPERATION_ID = "9c2f8d5e-3f1b-4a57-9d40-6a7e2c11b003"
X_DELTA_EMU = 127_000
Y_DELTA_EMU = 254_000


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
            "canonical producer failed"
            + (f"\nstderr:\n{completed.stderr}" if completed.stderr else "")
        )
    try:
        result = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError("canonical producer returned invalid JSON") from error
    if not isinstance(result, dict):
        raise RuntimeError("canonical producer response must be a JSON object")
    return result


def require_exact_keys(value: dict[str, Any], expected: set[str], label: str) -> None:
    actual = set(value)
    if actual != expected:
        missing = sorted(expected - actual)
        extra = sorted(actual - expected)
        raise RuntimeError(f"{label} fields mismatch: missing={missing} extra={extra}")


def build_receipt(
    producer_command: list[str],
    *,
    implementation: str,
    commit_or_build: str,
) -> dict[str, Any]:
    revision_store = load_revision_store()
    kernel = revision_store.RevisionKernel()

    baseline_output = invoke_producer(
        producer_command,
        {"action": "baseline", "source_hash": PINNED_SOURCE_HASH},
    )
    require_exact_keys(
        baseline_output,
        {"source_hash", "baseline_project", "move_candidate"},
        "baseline producer response",
    )
    if baseline_output["source_hash"] != PINNED_SOURCE_HASH:
        raise RuntimeError("baseline producer source hash mismatch")

    baseline_project = baseline_output["baseline_project"]
    move_candidate = baseline_output["move_candidate"]
    require_exact_keys(move_candidate, {"node_id", "before"}, "move_candidate")
    before = move_candidate["before"]
    if not isinstance(before, dict):
        raise RuntimeError("move_candidate.before must be an object")
    require_exact_keys(before, {"x", "y", "width", "height"}, "move_candidate.before")
    for field in ("x", "y", "width", "height"):
        if not isinstance(before[field], int) or isinstance(before[field], bool):
            raise RuntimeError(f"move_candidate.before.{field} must be an integer")

    baseline = kernel.register_baseline(
        document_id=PINNED_DOCUMENT_ID,
        source_hash=PINNED_SOURCE_HASH,
        project=copy.deepcopy(baseline_project),
    )

    request = {
        "protocol_version": "chaptera.commit-request.v1",
        "document_id": PINNED_DOCUMENT_ID,
        "source_hash": PINNED_SOURCE_HASH,
        "base_revision_id": baseline.revision_id,
        "client_operation_id": PRIMARY_OPERATION_ID,
        "command": {
            "kind": "move_node_to",
            "node_id": move_candidate["node_id"],
            "x_emu": before["x"] + X_DELTA_EMU,
            "y_emu": before["y"] + Y_DELTA_EMU,
        },
    }

    executor_calls = 0
    commit_output: dict[str, Any] | None = None

    def authoritative_executor(base_project: dict, command: dict):
        nonlocal executor_calls, commit_output
        executor_calls += 1
        if executor_calls != 1:
            raise RuntimeError("canonical executor was invoked more than once")

        commit_output = invoke_producer(
            producer_command,
            {
                "action": "commit",
                "source_hash": PINNED_SOURCE_HASH,
                "base_project": base_project,
                "command": command,
            },
        )
        require_exact_keys(
            commit_output,
            {
                "canonical_operation",
                "resulting_project",
                "replayed_project",
                "consequences",
                "source_hash_after",
                "source_hash_replay",
            },
            "commit producer response",
        )
        if commit_output["source_hash_after"] != PINNED_SOURCE_HASH:
            raise RuntimeError("canonical producer changed source hash")
        if commit_output["source_hash_replay"] != PINNED_SOURCE_HASH:
            raise RuntimeError("canonical replay changed source hash")

        return (
            copy.deepcopy(commit_output["canonical_operation"]),
            copy.deepcopy(commit_output["resulting_project"]),
            copy.deepcopy(commit_output["consequences"]),
        )

    accepted = kernel.commit_move(copy.deepcopy(request), authoritative_executor)
    if accepted.get("protocol_version") != "chaptera.commit-accepted.v1":
        raise RuntimeError(f"primary commit was not accepted: {accepted}")
    if executor_calls != 1 or commit_output is None:
        raise RuntimeError("canonical executor did not run exactly once")

    # Exact retry must be served from the kernel's idempotency cache.
    retry = kernel.commit_move(copy.deepcopy(request), authoritative_executor)
    if retry != accepted or executor_calls != 1:
        raise RuntimeError("exact retry was not idempotent")

    current_before_stale = kernel.current_revision(PINNED_DOCUMENT_ID).revision_id
    stale_request = copy.deepcopy(request)
    stale_request["client_operation_id"] = STALE_OPERATION_ID
    stale_request["base_revision_id"] = baseline.revision_id
    calls_before_stale = executor_calls
    stale = kernel.commit_move(stale_request, authoritative_executor)
    current_after_stale = kernel.current_revision(PINNED_DOCUMENT_ID).revision_id
    if stale.get("code") != "stale_revision":
        raise RuntimeError(f"stale probe returned {stale}")
    if executor_calls != calls_before_stale:
        raise RuntimeError("stale probe reached canonical executor")

    current_before_conflict = kernel.current_revision(PINNED_DOCUMENT_ID).revision_id
    conflict_request = copy.deepcopy(request)
    conflict_request["command"]["x_emu"] += 1
    calls_before_conflict = executor_calls
    conflict = kernel.commit_move(conflict_request, authoritative_executor)
    current_after_conflict = kernel.current_revision(PINNED_DOCUMENT_ID).revision_id
    if conflict.get("code") != "idempotency_conflict":
        raise RuntimeError(f"idempotency conflict probe returned {conflict}")
    if executor_calls != calls_before_conflict:
        raise RuntimeError("idempotency conflict reached canonical executor")

    receipt = {
        "receipt_version": "chaptera.revision-producer-receipt.v1",
        "producer": {
            "implementation": implementation,
            "commit_or_build": commit_or_build,
            "core_integration": True,
        },
        "document_id": PINNED_DOCUMENT_ID,
        "source_hash": PINNED_SOURCE_HASH,
        "baseline": {
            "project": copy.deepcopy(baseline.project),
            "project_hash": baseline.project_hash,
            "state_id": baseline.state_id,
            "revision_id": baseline.revision_id,
        },
        "request": request,
        "accepted": accepted,
        "resulting_project": copy.deepcopy(commit_output["resulting_project"]),
        "replayed_project": copy.deepcopy(commit_output["replayed_project"]),
        "probes": {
            "stale_base": {
                "code": stale["code"],
                "before_revision_id": current_before_stale,
                "after_revision_id": current_after_stale,
                "executor_calls_delta": executor_calls - calls_before_stale,
            },
            "exact_retry": {
                "same_result": retry == accepted,
                "executor_calls_total": executor_calls,
            },
            "idempotency_conflict": {
                "code": conflict["code"],
                "before_revision_id": current_before_conflict,
                "after_revision_id": current_after_conflict,
                "executor_calls_delta": executor_calls - calls_before_conflict,
            },
            "source_identity": {
                "before": baseline_output["source_hash"],
                "after": commit_output["source_hash_after"],
                "replay": commit_output["source_hash_replay"],
            },
        },
    }
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--implementation", default="chaptera-canonical-editor")
    parser.add_argument("--commit-or-build", required=True)
    parser.add_argument(
        "producer_command",
        nargs=argparse.REMAINDER,
        help="local canonical producer command; prefix it with --",
    )
    args = parser.parse_args()

    producer_command = list(args.producer_command)
    if producer_command and producer_command[0] == "--":
        producer_command = producer_command[1:]
    if not producer_command:
        parser.error("producer_command is required after --")

    receipt = build_receipt(
        producer_command,
        implementation=args.implementation,
        commit_or_build=args.commit_or_build,
    )

    sys.path.insert(0, str(TOOLS))
    from validate_revision_producer_receipt import validate_schema, validate_semantics

    validate_schema(receipt)
    summary = validate_semantics(receipt)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(receipt, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
