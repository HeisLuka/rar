#!/usr/bin/env python3
"""Build a source-free ResizeNode receipt from an authorized local producer.

Rar owns the revision/idempotency/history envelope. The local producer owns the
actual canonical editor mutation, replay/export behavior, and target capability
checks. This builder never emits source hash, NodeId, paths, text, or PUB bytes.
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

RESIZE_OPERATION_ID = "resize-op-00000001"
UNDO_OPERATION_ID = "resize-undo-00000001"
REDO_OPERATION_ID = "resize-redo-00000001"

PROBES = {
    "identical_bounds": "identical_bounds_rejected_no_mutation",
    "pure_move": "pure_move_rejected_no_mutation",
    "non_positive_size": "non_positive_size_rejected_no_mutation",
    "overflow": "overflow_rejected_no_mutation",
    "unsupported_target": "unsupported_target_rejected_no_mutation",
}


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
            "ResizeNode producer failed"
            + (f"\nstderr:\n{completed.stderr}" if completed.stderr else "")
        )
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError("ResizeNode producer returned invalid JSON") from error
    if not isinstance(value, dict):
        raise RuntimeError("ResizeNode producer response must be a JSON object")
    return value


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


def validate_rect(rect: Any, label: str) -> None:
    if not isinstance(rect, dict) or set(rect) != {"x", "y", "width", "height"}:
        raise RuntimeError(f"{label} must be exact RectEmu")
    for key in ("x", "y", "width", "height"):
        if not isinstance(rect[key], int) or isinstance(rect[key], bool):
            raise RuntimeError(f"{label}.{key} must be an integer")
    if rect["width"] <= 0 or rect["height"] <= 0:
        raise RuntimeError(f"{label} must have positive size")


def build_receipt(
    producer_command: list[str],
    *,
    source_hash: str,
    document_id: str,
    chaptera_version: str,
    platform: str,
    binary_sha256: str,
    fixture_kind: str,
    projection_instance_admitted: bool = False,
    integration: str = "local_private",
) -> dict[str, Any]:
    if fixture_kind not in {"synthetic_geometry", "real_pub_sanitized"}:
        raise RuntimeError("unsupported fixture_kind")
    if integration not in {"local_private", "hosted_native"}:
        raise RuntimeError("unsupported producer integration")
    if fixture_kind == "real_pub_sanitized" and not projection_instance_admitted:
        raise RuntimeError("projection_instance_gate_unresolved")

    revision_store = load_revision_store()
    kernel = revision_store.RevisionKernel()

    baseline = invoke_producer(
        producer_command,
        {"action": "baseline", "source_hash": source_hash, "fixture_kind": fixture_kind},
    )
    require_exact_keys(
        baseline,
        {"source_hash", "baseline_project", "resize_candidate", "signed_origin_probe_passed"},
        "baseline producer response",
    )
    if baseline["source_hash"] != source_hash:
        raise RuntimeError("baseline producer source hash mismatch")
    if baseline["signed_origin_probe_passed"] is not True:
        raise RuntimeError("signed-origin ResizeNode probe is required")

    candidate = baseline["resize_candidate"]
    require_exact_keys(
        candidate,
        {
            "node_id",
            "before",
            "after",
            "direct_page_owned",
            "identity_transform",
            "original_bounds_valid",
        },
        "resize_candidate",
    )
    validate_rect(candidate["before"], "resize_candidate.before")
    validate_rect(candidate["after"], "resize_candidate.after")
    for gate in ("direct_page_owned", "identity_transform", "original_bounds_valid"):
        if candidate[gate] is not True:
            raise RuntimeError(f"ResizeNode target gate failed: {gate}")

    baseline_project = copy.deepcopy(baseline["baseline_project"])
    baseline_record = kernel.register_baseline(
        document_id=document_id,
        source_hash=source_hash,
        project=baseline_project,
    )

    def target_gate(_command: dict) -> None:
        for gate in ("direct_page_owned", "identity_transform", "original_bounds_valid"):
            if candidate[gate] is not True:
                raise ValueError("unsupported_resize_target")

    commit_output: dict[str, Any] | None = None

    def resize_executor(base_project: dict, command: dict):
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

    after = candidate["after"]
    request = {
        "protocol_version": "chaptera.resize-node-intent.v1",
        "document_id": document_id,
        "source_hash": source_hash,
        "base_revision_id": baseline_record.revision_id,
        "client_operation_id": RESIZE_OPERATION_ID,
        "command": {
            "kind": "resize_node_to",
            "node_id": candidate["node_id"],
            "x_emu": after["x"],
            "y_emu": after["y"],
            "width_emu": after["width"],
            "height_emu": after["height"],
        },
    }
    accepted = kernel.commit_resize(request, resize_executor, pre_execute_validator=target_gate)
    if accepted.get("protocol_version") != "chaptera.commit-accepted.v1":
        raise RuntimeError(f"ResizeNode was not accepted: {accepted}")
    if commit_output is None:
        raise RuntimeError("ResizeNode executor did not run")

    operation = commit_output["canonical_operation"]
    if operation.get("before") != candidate["before"]:
        raise RuntimeError("canonical ResizeNode before differs from producer candidate")
    if operation.get("after") != candidate["after"]:
        raise RuntimeError("canonical ResizeNode after differs from producer candidate")
    accepted_project = copy.deepcopy(commit_output["resulting_project"])

    history_projects: dict[str, dict[str, Any]] = {}

    def history_executor(base_project: dict, transition_kind: str):
        output = invoke_producer(
            producer_command,
            {
                "action": "history",
                "source_hash": source_hash,
                "kind": transition_kind,
                "base_project": base_project,
                "baseline_project": baseline_project,
                "accepted_project": accepted_project,
            },
        )
        require_exact_keys(
            output,
            {"resulting_project", "consequences", "source_hash_after"},
            f"{transition_kind} producer response",
        )
        assert_same_source(output, source_hash, f"{transition_kind} producer")
        history_projects[transition_kind] = copy.deepcopy(output["resulting_project"])
        return copy.deepcopy(output["resulting_project"]), copy.deepcopy(output["consequences"])

    undo = kernel.commit_history_transition(
        {
            "protocol_version": "chaptera.history-transition-intent.v1",
            "document_id": document_id,
            "source_hash": source_hash,
            "base_revision_id": accepted["revision_id"],
            "client_operation_id": UNDO_OPERATION_ID,
            "command": {"kind": "undo"},
        },
        history_executor,
    )
    if undo.get("protocol_version") != "chaptera.history-transition-accepted.v1":
        raise RuntimeError(f"ResizeNode undo was not accepted: {undo}")
    if history_projects["undo"] != baseline_project:
        raise RuntimeError("ResizeNode undo did not restore exact baseline project")

    redo = kernel.commit_history_transition(
        {
            "protocol_version": "chaptera.history-transition-intent.v1",
            "document_id": document_id,
            "source_hash": source_hash,
            "base_revision_id": undo["revision_id"],
            "client_operation_id": REDO_OPERATION_ID,
            "command": {"kind": "redo"},
        },
        history_executor,
    )
    if redo.get("protocol_version") != "chaptera.history-transition-accepted.v1":
        raise RuntimeError(f"ResizeNode redo was not accepted: {redo}")
    if history_projects["redo"] != accepted_project:
        raise RuntimeError("ResizeNode redo did not restore exact accepted project")

    replay = invoke_producer(
        producer_command,
        {"action": "replay", "source_hash": source_hash, "project": accepted_project},
    )
    require_exact_keys(
        replay,
        {
            "replayed_project",
            "legacy_v0_4_rejected",
            "stale_before_rejected_transactionally",
            "source_hash_after",
        },
        "replay producer response",
    )
    assert_same_source(replay, source_hash, "replay producer")
    if replay["replayed_project"] != accepted_project:
        raise RuntimeError("fresh ResizeNode replay differs from accepted project")
    if replay["legacy_v0_4_rejected"] is not True:
        raise RuntimeError("legacy v0.4 ResizeNode project compatibility fence is missing")
    if replay["stale_before_rejected_transactionally"] is not True:
        raise RuntimeError("stale ResizeNode replay did not fail transactionally")

    export = invoke_producer(
        producer_command,
        {"action": "export", "source_hash": source_hash, "project": accepted_project},
    )
    require_exact_keys(
        export,
        {"idml_reflects_resized_bounds", "odg_reflects_resized_bounds", "source_hash_after"},
        "export producer response",
    )
    assert_same_source(export, source_hash, "export producer")
    if not (export["idml_reflects_resized_bounds"] and export["odg_reflects_resized_bounds"]):
        raise RuntimeError("editable export did not preserve resized bounds")

    negative_probes: dict[str, bool] = {}
    for probe_name, receipt_key in PROBES.items():
        output = invoke_producer(
            producer_command,
            {
                "action": "probe",
                "probe": probe_name,
                "source_hash": source_hash,
                "baseline_project": baseline_project,
                "resize_candidate": candidate,
            },
        )
        require_exact_keys(
            output,
            {"rejected_no_mutation", "source_hash_after"},
            f"{probe_name} probe response",
        )
        assert_same_source(output, source_hash, f"{probe_name} probe")
        if output["rejected_no_mutation"] is not True:
            raise RuntimeError(f"ResizeNode negative probe failed: {probe_name}")
        negative_probes[receipt_key] = True

    before = operation["before"]
    after = operation["after"]
    before_count = len(baseline_project["operations"])
    after_count = len(accepted_project["operations"])

    receipt = {
        "receipt_version": "chaptera.resize-node-producer-receipt.v1",
        "operation_contract": "chaptera.resize-node.v1",
        "producer": {
            "kind": "chaptera_desktop_editor",
            "integration": integration,
        },
        "build": {
            "chaptera_version": chaptera_version,
            "platform": platform,
            "binary_sha256": binary_sha256,
        },
        "fixture_kind": fixture_kind,
        "target_gate": {
            "direct_page_owned": True,
            "identity_transform": True,
            "original_bounds_valid": True,
            "node_id_redacted": True,
        },
        "commit": {
            "operation_kind": "ResizeNode",
            "operation_count_before": before_count,
            "operation_count_after": after_count,
            "exact_before_captured": True,
            "exact_after_captured": True,
            "position_may_be_signed": True,
            "after_width_positive": after["width"] > 0,
            "after_height_positive": after["height"] > 0,
            "size_changed": (
                before["width"] != after["width"] or before["height"] != after["height"]
            ),
            "pure_move": (
                before["width"] == after["width"] and before["height"] == after["height"]
            ),
            "bounds_commitments_distinct": before != after,
        },
        "persistence": {
            "project_schema": "pub-editor-v0.5",
            "feature": "node.geometry.bounds",
            "property_path": "node.bounds",
            "format_representability": "lossless",
            "native_pub_writer_state": "writer_blocked",
        },
        "undo_redo": {
            "undo_restores_exact_before": history_projects["undo"] == baseline_project,
            "redo_restores_exact_after": history_projects["redo"] == accepted_project,
        },
        "replay": {
            "fresh_session_reproduces_after": replay["replayed_project"] == accepted_project,
            "operation_count_preserved": (
                len(replay["replayed_project"]["operations"]) == after_count
            ),
            "legacy_v0_4_rejected": replay["legacy_v0_4_rejected"],
            "stale_before_rejected_transactionally": replay[
                "stale_before_rejected_transactionally"
            ],
        },
        "exports": {
            "idml_reflects_resized_bounds": export["idml_reflects_resized_bounds"],
            "odg_reflects_resized_bounds": export["odg_reflects_resized_bounds"],
            "source_pub_unchanged": export["source_hash_after"] == source_hash,
        },
        "negative_probes": negative_probes,
        "privacy": {
            "pub_bytes_in_receipt": False,
            "pub_filename_in_receipt": False,
            "local_path_in_receipt": False,
            "source_hash_in_receipt": False,
            "node_id_in_receipt": False,
            "document_text_in_receipt": False,
            "customer_identity_in_receipt": False,
        },
    }

    sys.path.insert(0, str(TOOLS))
    from validate_resize_node_producer_receipt import validate_receipt

    validate_receipt(receipt)
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-hash", required=True)
    parser.add_argument("--document-id", required=True)
    parser.add_argument("--chaptera-version", required=True)
    parser.add_argument("--platform", choices=["windows", "macos", "linux"], required=True)
    parser.add_argument("--binary-sha256", required=True)
    parser.add_argument(
        "--fixture-kind",
        choices=["synthetic_geometry", "real_pub_sanitized"],
        required=True,
    )
    parser.add_argument("--projection-instance-admitted", action="store_true")
    parser.add_argument(
        "--integration",
        choices=["local_private", "hosted_native"],
        default="local_private",
    )
    parser.add_argument("--output", required=True, type=pathlib.Path)
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
        chaptera_version=args.chaptera_version,
        platform=args.platform,
        binary_sha256=args.binary_sha256,
        fixture_kind=args.fixture_kind,
        projection_instance_admitted=args.projection_instance_admitted,
        integration=args.integration,
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
