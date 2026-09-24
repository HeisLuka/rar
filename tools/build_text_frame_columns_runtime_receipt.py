#!/usr/bin/env python3
"""Build AUTHORING-TEXTFRAME-COLUMNS-01 runtime evidence from a local producer.

Rar owns the public RevisionKernel envelope. The authorized local/private
producer owns the real canonical editor state, line-region consumer and
authoritative overflow evaluation. CI may exercise this orchestration with a
fake producer, but only a receipt produced by the real local runtime can close
AUTHORING-TEXTFRAME-COLUMNS-01.
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

COUNT_OPERATION_ID = "columns-count-op-00000001"
GUTTER_OPERATION_ID = "columns-gutter-op-00000001"
UNDO_OPERATION_ID = "columns-runtime-undo-0001"
REDO_OPERATION_ID = "columns-runtime-redo-0001"

RUNTIME_KEYS = {
    "column_count",
    "gutter_emu",
    "sample_band_slot_count",
    "sample_band_total_usable_width_emu",
    "line_region_partition_sha256",
    "overflow_state",
    "overflow_rederived",
    "environment_authoritative",
    "layout_environment_sha256",
}
MUTATION_INVARIANT_KEYS = {
    "story_identity_preserved",
    "story_text_preserved",
    "outer_bounds_preserved",
    "source_bytes_written",
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
            "TextFrame columns producer failed"
            + (f"\nstderr:\n{completed.stderr}" if completed.stderr else "")
        )
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError("TextFrame columns producer returned invalid JSON") from error
    if not isinstance(value, dict):
        raise RuntimeError("TextFrame columns producer response must be a JSON object")
    return value


def require_exact_keys(value: Any, expected: set[str], label: str) -> None:
    if not isinstance(value, dict):
        raise RuntimeError(f"{label} must be an object")
    actual = set(value)
    if actual != expected:
        raise RuntimeError(
            f"{label} fields mismatch: missing={sorted(expected - actual)} "
            f"extra={sorted(actual - expected)}"
        )


def normalize_sha256(value: Any, label: str) -> str:
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


def validate_column_state(state: Any, label: str) -> dict[str, int]:
    require_exact_keys(state, {"column_count", "gutter_emu"}, label)
    count = state["column_count"]
    gutter = state["gutter_emu"]
    if not isinstance(count, int) or isinstance(count, bool) or not 1 <= count <= 1024:
        raise RuntimeError(f"{label}.column_count must be an integer in 1..1024")
    if not isinstance(gutter, int) or isinstance(gutter, bool) or gutter < 0:
        raise RuntimeError(f"{label}.gutter_emu must be a non-negative integer")
    return {"column_count": count, "gutter_emu": gutter}


def validate_runtime_state(
    state: Any,
    *,
    expected_columns: dict[str, int],
    environment_hash: str | None,
    label: str,
) -> tuple[dict[str, Any], str]:
    require_exact_keys(state, RUNTIME_KEYS, label)
    if state["column_count"] != expected_columns["column_count"]:
        raise RuntimeError(f"{label} did not consume canonical column_count")
    if state["gutter_emu"] != expected_columns["gutter_emu"]:
        raise RuntimeError(f"{label} did not consume canonical gutter_emu")
    slot_count = state["sample_band_slot_count"]
    if not isinstance(slot_count, int) or isinstance(slot_count, bool) or slot_count < 1:
        raise RuntimeError(f"{label}.sample_band_slot_count must be positive")
    if slot_count != expected_columns["column_count"]:
        raise RuntimeError(
            f"{label} obstacle-free sample band slot count does not match column_count"
        )
    usable = state["sample_band_total_usable_width_emu"]
    if not isinstance(usable, int) or isinstance(usable, bool) or usable <= 0:
        raise RuntimeError(f"{label}.sample_band_total_usable_width_emu must be positive")
    normalize_sha256(state["line_region_partition_sha256"], f"{label}.line_region_partition_sha256")
    if state["overflow_state"] not in {"fits", "overset"}:
        raise RuntimeError(f"{label}.overflow_state must be fits or overset")
    if state["overflow_rederived"] is not True:
        raise RuntimeError(f"{label} must re-derive overflow")
    if state["environment_authoritative"] is not True:
        raise RuntimeError(f"{label} requires authoritative layout environment")
    current_environment = normalize_sha256(
        state["layout_environment_sha256"],
        f"{label}.layout_environment_sha256",
    )
    if environment_hash is not None and current_environment != environment_hash:
        raise RuntimeError("layout environment changed inside the runtime proof")
    return copy.deepcopy(state), current_environment


def validate_mutation_invariants(value: Any, label: str) -> None:
    require_exact_keys(value, MUTATION_INVARIANT_KEYS, label)
    for key in ("story_identity_preserved", "story_text_preserved", "outer_bounds_preserved"):
        if value[key] is not True:
            raise RuntimeError(f"{label}.{key} must be true")
    if value["source_bytes_written"] is not False:
        raise RuntimeError(f"{label} must not write source PUB bytes")


def require_layout_invalidation(consequences: Any, label: str) -> None:
    if not isinstance(consequences, list):
        raise RuntimeError(f"{label} consequences must be a list")
    states = {
        item.get("key"): item.get("state")
        for item in consequences
        if isinstance(item, dict)
    }
    if states.get("layout.reflow") != "invalidated":
        raise RuntimeError(f"{label} must invalidate layout.reflow")
    if states.get("story.overset") != "invalidated":
        raise RuntimeError(f"{label} must invalidate story.overset")


def build_receipt(
    producer_command: list[str],
    *,
    source_hash: str,
    document_id: str,
    chaptera_version: str,
    platform: str,
    binary_sha256: str,
) -> dict[str, Any]:
    if platform not in {"windows", "macos", "linux"}:
        raise RuntimeError("unsupported platform")
    binary_sha = normalize_sha256(binary_sha256, "binary_sha256")

    revision_store = load_revision_store()
    kernel = revision_store.RevisionKernel()

    baseline = invoke_producer(
        producer_command,
        {"action": "baseline", "source_hash": source_hash},
    )
    require_exact_keys(
        baseline,
        {
            "source_hash",
            "fixture_kind",
            "baseline_project",
            "frame_node_id",
            "before",
            "count_after",
            "gutter_after",
            "target_gate",
        },
        "baseline producer response",
    )
    if baseline["source_hash"] != source_hash:
        raise RuntimeError("baseline producer source hash mismatch")
    if baseline["fixture_kind"] not in {
        "author_created_one_frame",
        "source_backed_sanitized_one_frame",
    }:
        raise RuntimeError("unsupported TextFrame runtime fixture kind")
    target_gate = baseline["target_gate"]
    require_exact_keys(
        target_gate,
        {
            "ordinary_one_frame",
            "unlinked",
            "autofit_off",
            "vertical_text_off",
            "wrap_obstacles_absent",
        },
        "target_gate",
    )
    if not all(target_gate.values()):
        raise RuntimeError("TextFrame runtime target is outside the admitted V1 class")

    before = validate_column_state(baseline["before"], "before")
    count_after = validate_column_state(baseline["count_after"], "count_after")
    gutter_after = validate_column_state(baseline["gutter_after"], "gutter_after")
    if count_after["column_count"] == before["column_count"]:
        raise RuntimeError("count arm must change column_count")
    if count_after["gutter_emu"] != before["gutter_emu"]:
        raise RuntimeError("count arm must keep gutter fixed")
    if gutter_after["column_count"] != count_after["column_count"]:
        raise RuntimeError("gutter arm must keep column_count fixed")
    if gutter_after["gutter_emu"] <= count_after["gutter_emu"]:
        raise RuntimeError("gutter arm must increase gutter_emu")

    baseline_project = copy.deepcopy(baseline["baseline_project"])
    baseline_record = kernel.register_baseline(
        document_id=document_id,
        source_hash=source_hash,
        project=baseline_project,
    )
    frame_node_id = baseline["frame_node_id"]

    environment_hash: str | None = None

    def runtime(project: dict, expected: dict[str, int], label: str) -> dict[str, Any]:
        nonlocal environment_hash
        output = invoke_producer(
            producer_command,
            {
                "action": "runtime",
                "source_hash": source_hash,
                "project": project,
                "frame_node_id": frame_node_id,
            },
        )
        require_exact_keys(output, {"runtime_state", "source_hash_after"}, f"{label} response")
        assert_same_source(output, source_hash, label)
        state, environment_hash = validate_runtime_state(
            output["runtime_state"],
            expected_columns=expected,
            environment_hash=environment_hash,
            label=label,
        )
        return state

    baseline_runtime = runtime(baseline_project, before, "baseline runtime")

    def commit_columns(
        *,
        base_revision_id: str,
        expected_before: dict[str, int],
        after: dict[str, int],
        operation_id: str,
        label: str,
    ) -> tuple[dict[str, Any], dict[str, Any]]:
        commit_output: dict[str, Any] | None = None

        def target_validator(_command: dict) -> None:
            if not all(target_gate.values()):
                raise ValueError("unsupported_text_frame_class")

        def executor(base_project: dict, command: dict):
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
                    "mutation_invariants",
                    "source_hash_after",
                },
                f"{label} producer response",
            )
            assert_same_source(commit_output, source_hash, f"{label} producer")
            validate_mutation_invariants(
                commit_output["mutation_invariants"],
                f"{label} mutation invariants",
            )
            require_layout_invalidation(commit_output["consequences"], label)
            return (
                copy.deepcopy(commit_output["canonical_operation"]),
                copy.deepcopy(commit_output["resulting_project"]),
                copy.deepcopy(commit_output["consequences"]),
            )

        request = {
            "protocol_version": "chaptera.text-frame-columns-intent.v1",
            "document_id": document_id,
            "source_hash": source_hash,
            "base_revision_id": base_revision_id,
            "client_operation_id": operation_id,
            "command": {
                "kind": "set_text_frame_columns",
                "node_id": frame_node_id,
                "expected_before": copy.deepcopy(expected_before),
                "after": copy.deepcopy(after),
            },
        }
        accepted = kernel.commit_text_frame_columns(
            request,
            executor,
            pre_execute_validator=target_validator,
        )
        if accepted.get("protocol_version") != "chaptera.commit-accepted.v1":
            raise RuntimeError(f"{label} was not accepted: {accepted}")
        if commit_output is None:
            raise RuntimeError(f"{label} executor did not run")
        return accepted, copy.deepcopy(commit_output["resulting_project"])

    count_accepted, count_project = commit_columns(
        base_revision_id=baseline_record.revision_id,
        expected_before=before,
        after=count_after,
        operation_id=COUNT_OPERATION_ID,
        label="count arm",
    )
    count_runtime = runtime(count_project, count_after, "count runtime")

    if count_runtime["sample_band_slot_count"] == baseline_runtime["sample_band_slot_count"]:
        raise RuntimeError("count arm did not change obstacle-free line-region slot count")
    if (
        count_runtime["line_region_partition_sha256"]
        == baseline_runtime["line_region_partition_sha256"]
    ):
        raise RuntimeError("count arm did not change line-region partition")

    gutter_accepted, gutter_project = commit_columns(
        base_revision_id=count_accepted["revision_id"],
        expected_before=count_after,
        after=gutter_after,
        operation_id=GUTTER_OPERATION_ID,
        label="gutter arm",
    )
    gutter_runtime = runtime(gutter_project, gutter_after, "gutter runtime")

    if gutter_runtime["sample_band_slot_count"] != count_runtime["sample_band_slot_count"]:
        raise RuntimeError("gutter arm unexpectedly changed column slot count")
    if (
        gutter_runtime["sample_band_total_usable_width_emu"]
        >= count_runtime["sample_band_total_usable_width_emu"]
    ):
        raise RuntimeError("increased gutter did not reduce usable column width")
    if (
        gutter_runtime["line_region_partition_sha256"]
        == count_runtime["line_region_partition_sha256"]
    ):
        raise RuntimeError("gutter arm did not change line-region partition")

    history_projects: dict[str, dict[str, Any]] = {}

    def history_executor(base_project: dict, transition_kind: str):
        output = invoke_producer(
            producer_command,
            {
                "action": "history",
                "source_hash": source_hash,
                "kind": transition_kind,
                "base_project": base_project,
                "count_project": count_project,
                "gutter_project": gutter_project,
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
            "base_revision_id": gutter_accepted["revision_id"],
            "client_operation_id": UNDO_OPERATION_ID,
            "command": {"kind": "undo"},
        },
        history_executor,
    )
    if undo.get("protocol_version") != "chaptera.history-transition-accepted.v1":
        raise RuntimeError(f"columns undo was not accepted: {undo}")
    if history_projects["undo"] != count_project:
        raise RuntimeError("columns undo did not restore exact count-arm project")
    undo_runtime = runtime(history_projects["undo"], count_after, "undo runtime")
    if undo_runtime != count_runtime:
        raise RuntimeError("undo did not re-derive the exact count-arm runtime state")

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
        raise RuntimeError(f"columns redo was not accepted: {redo}")
    if history_projects["redo"] != gutter_project:
        raise RuntimeError("columns redo did not restore exact gutter-arm project")
    redo_runtime = runtime(history_projects["redo"], gutter_after, "redo runtime")
    if redo_runtime != gutter_runtime:
        raise RuntimeError("redo did not re-derive the exact final runtime state")

    replay = invoke_producer(
        producer_command,
        {"action": "replay", "source_hash": source_hash, "project": gutter_project},
    )
    require_exact_keys(
        replay,
        {"replayed_project", "source_hash_after"},
        "replay producer response",
    )
    assert_same_source(replay, source_hash, "replay producer")
    if replay["replayed_project"] != gutter_project:
        raise RuntimeError("fresh replay project differs from accepted gutter project")
    replay_runtime = runtime(replay["replayed_project"], gutter_after, "replay runtime")
    if replay_runtime != gutter_runtime:
        raise RuntimeError("fresh replay did not re-derive exact final runtime state")

    receipt = {
        "receipt_version": "chaptera.text-frame-columns-runtime-receipt.v1",
        "producer": {
            "kind": "chaptera_authoritative_layout_runtime",
            "integration": "local_private",
        },
        "build": {
            "chaptera_version": chaptera_version,
            "platform": platform,
            "binary_sha256": binary_sha,
        },
        "fixture_kind": baseline["fixture_kind"],
        "target_gate": copy.deepcopy(target_gate),
        "layout_environment_sha256": environment_hash,
        "count_arm": {
            "before_column_count": before["column_count"],
            "after_column_count": count_after["column_count"],
            "gutter_emu_held_constant": before["gutter_emu"] == count_after["gutter_emu"],
            "slot_count_before": baseline_runtime["sample_band_slot_count"],
            "slot_count_after": count_runtime["sample_band_slot_count"],
            "line_region_partition_changed": (
                baseline_runtime["line_region_partition_sha256"]
                != count_runtime["line_region_partition_sha256"]
            ),
            "overflow_rederived": count_runtime["overflow_rederived"],
        },
        "gutter_arm": {
            "column_count_held_constant": (
                count_after["column_count"] == gutter_after["column_count"]
            ),
            "gutter_before_emu": count_after["gutter_emu"],
            "gutter_after_emu": gutter_after["gutter_emu"],
            "slot_count_held_constant": (
                count_runtime["sample_band_slot_count"]
                == gutter_runtime["sample_band_slot_count"]
            ),
            "usable_width_before_emu": count_runtime[
                "sample_band_total_usable_width_emu"
            ],
            "usable_width_after_emu": gutter_runtime[
                "sample_band_total_usable_width_emu"
            ],
            "line_region_partition_changed": (
                count_runtime["line_region_partition_sha256"]
                != gutter_runtime["line_region_partition_sha256"]
            ),
            "overflow_rederived": gutter_runtime["overflow_rederived"],
        },
        "history_replay": {
            "undo_restores_count_runtime": undo_runtime == count_runtime,
            "redo_restores_final_runtime": redo_runtime == gutter_runtime,
            "fresh_replay_restores_final_runtime": replay_runtime == gutter_runtime,
        },
        "overflow_states": {
            "baseline": baseline_runtime["overflow_state"],
            "after_count": count_runtime["overflow_state"],
            "after_gutter": gutter_runtime["overflow_state"],
        },
        "invariants": {
            "story_identity_preserved": True,
            "story_text_preserved": True,
            "outer_bounds_preserved": True,
            "source_pub_unchanged": True,
            "native_pub_write_count": 0,
            "mcld_direct_write_count": 0,
            "linked_frame_flow_used": False,
            "autofit_used": False,
            "vertical_text_used": False,
            "wrap_obstacle_count": 0,
        },
        "privacy": {
            "source_hash_in_receipt": False,
            "frame_node_id_in_receipt": False,
            "document_text_in_receipt": False,
            "pub_bytes_in_receipt": False,
            "local_path_in_receipt": False,
        },
    }
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-hash", required=True)
    parser.add_argument("--document-id", required=True)
    parser.add_argument("--chaptera-version", required=True)
    parser.add_argument("--platform", choices=["windows", "macos", "linux"], required=True)
    parser.add_argument("--binary-sha256", required=True)
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
