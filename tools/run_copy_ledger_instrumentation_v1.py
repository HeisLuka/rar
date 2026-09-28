#!/usr/bin/env python3
"""Hosted five-scenario producer for ENGINE-COPY-LEDGER-INSTRUMENT-01."""

from __future__ import annotations

import argparse
import dataclasses
import json
import os
import pathlib
import platform
import subprocess
import sys
import tracemalloc
from typing import Any, Callable

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
EDITOR_API = ROOT / "services" / "editor-api"
PRODUCER = ROOT / "vendor" / "producer-a"

sys.path.insert(0, str(TOOLS))
sys.path.insert(0, str(EDITOR_API))

from copy_ledger_instrument_v1 import FixedCopyCountersV1
from copy_ledger_v1 import identity_hash, validate_receipt, with_summary
from optimization_receipt_v1 import copy_ledger_measurement
from revision_store import RevisionKernel
from test_move_nodes import (
    DOCUMENT_ID as MOVE_DOCUMENT_ID,
    SOURCE_HASH as MOVE_SOURCE_HASH,
    MoveNodesExecutor,
    move_request,
    rect,
)
from test_story_edit_transaction_v1 import (
    DOCUMENT_ID as STORY_DOCUMENT_ID,
    P1,
    SOURCE_HASH as STORY_SOURCE_HASH,
    STORY_ID,
    make_core_state,
    make_project,
)


PUBLIC_FIXTURE_SHA256 = (
    "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf"
)


def _run_json(command: list[str], *, cwd: pathlib.Path = ROOT) -> dict[str, Any]:
    completed = subprocess.run(
        command,
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"scenario command failed rc={completed.returncode}: {' '.join(command)}\n"
            f"{completed.stderr}"
        )
    lines = [line for line in completed.stdout.splitlines() if line.strip().startswith("{")]
    if not lines:
        raise RuntimeError(
            f"scenario command produced no JSON: {' '.join(command)}\n{completed.stdout}"
        )
    value = json.loads(lines[-1])
    if not isinstance(value, dict):
        raise RuntimeError("scenario output must be an object")
    return value


def _counter_dict(counters: FixedCopyCountersV1) -> dict[str, dict[str, int]]:
    return {
        row.site: dataclasses.asdict(row)
        for row in counters.snapshot()
    }


def _measure_python_scenario(
    callback: Callable[[FixedCopyCountersV1], None],
) -> dict[str, Any]:
    counters = FixedCopyCountersV1()
    tracemalloc.start()
    live_before, _ = tracemalloc.get_traced_memory()
    callback(counters)
    live_after, peak = tracemalloc.get_traced_memory()
    tracemalloc.stop()
    return {
        "sites": _counter_dict(counters),
        "allocator": {
            "live_bytes_before": live_before,
            "live_bytes_after": live_after,
            "peak_live_bytes": peak,
        },
    }


def _move_node_once(counters: FixedCopyCountersV1) -> None:
    kernel = RevisionKernel(copy_ledger_counters=counters)
    project = {
        "schema_version": "pub-editor-v0.6",
        "source_hash": MOVE_SOURCE_HASH,
        "operations": [],
        "nodes": {
            "node:a": {
                "page_id": "page:1",
                "provenance": "chaptera-authored",
                "bounds": rect(0, 0),
            },
            "node:b": {
                "page_id": "page:1",
                "provenance": "chaptera-authored",
                "bounds": rect(200, 10, 50, 60),
            },
        },
    }
    baseline = kernel.register_baseline(
        document_id=MOVE_DOCUMENT_ID,
        source_hash=MOVE_SOURCE_HASH,
        project=project,
    )
    counters.reset()
    entries = [
        {
            "node_id": "node:b",
            "expected_before": rect(200, 10, 50, 60),
            "after": rect(180, 40, 50, 60),
        },
        {
            "node_id": "node:a",
            "expected_before": rect(0, 0),
            "after": rect(-20, 30),
        },
    ]
    result = kernel.commit_move_nodes(
        move_request(baseline.revision_id, "copy-ledger-move-1", entries),
        MoveNodesExecutor(),
    )
    if result["protocol_version"] != "chaptera.commit-accepted.v1":
        raise AssertionError("move_node scenario did not commit")


def _small_story_edit_once(counters: FixedCopyCountersV1) -> None:
    kernel = RevisionKernel(copy_ledger_counters=counters)
    core = make_core_state("ABC")
    project = make_project(core)
    baseline = kernel.register_baseline(
        document_id=STORY_DOCUMENT_ID,
        source_hash=STORY_SOURCE_HASH,
        project=project,
    )
    counters.reset()
    request = {
        "protocol_version": "chaptera.story-edit-transaction-intent.v1",
        "document_id": STORY_DOCUMENT_ID,
        "source_hash": STORY_SOURCE_HASH,
        "base_revision_id": baseline.revision_id,
        "client_operation_id": "copy-ledger-story-1",
        "command": {
            "kind": "story_edit_transaction",
            "story_id": STORY_ID,
            "start_scalar": 1,
            "end_scalar": 1,
            "expected_before": "",
            "replacement_text": "X",
            "paragraph_inserted_ids": [],
            "paragraph_inserted_property_presets": [],
            "typing_format": None,
            "fragment_format_runs": [],
            "incoming_semantic_kinds": [],
        },
    }
    result = kernel.commit_story_edit_transaction(request)
    if result["protocol_version"] != "chaptera.commit-accepted.v1":
        raise AssertionError("small_story_edit scenario did not commit")
    if kernel.current_revision(STORY_DOCUMENT_ID).project["stories"][STORY_ID] != "AXBC":
        raise AssertionError("small_story_edit semantic result changed")


def _assert_stable(label: str, first: Any, second: Any) -> None:
    if first != second:
        raise AssertionError(f"{label} fixed-site attribution changed across repeats")


def _python_scenarios() -> tuple[dict[str, Any], dict[str, Any]]:
    move_first = _measure_python_scenario(_move_node_once)
    move_second = _measure_python_scenario(_move_node_once)
    _assert_stable("move_node.sites", move_first["sites"], move_second["sites"])

    story_first = _measure_python_scenario(_small_story_edit_once)
    story_second = _measure_python_scenario(_small_story_edit_once)
    _assert_stable("small_story_edit.sites", story_first["sites"], story_second["sites"])

    if story_first["sites"]["revision.move_nodes_request_normalize"]["materialized_bytes"] != 0:
        raise AssertionError("unrelated move normalization counter fired during story edit")

    idle = FixedCopyCountersV1()
    kernel = RevisionKernel(copy_ledger_counters=idle)
    kernel.register_baseline(
        document_id="copy-ledger-idle",
        source_hash="f" * 64,
        project={
            "schema_version": "pub-editor-v0.6",
            "source_hash": "f" * 64,
            "operations": [],
        },
    )
    if any(row.materialized_bytes for row in idle.snapshot()):
        raise AssertionError("unrelated baseline registration fired commit counters")

    return move_first, story_first


def _rust_scenarios(fixture: pathlib.Path) -> tuple[dict[str, Any], dict[str, Any], dict[str, Any]]:
    open_command = [
        "cargo",
        "run",
        "--quiet",
        "--manifest-path",
        str(PRODUCER / "Cargo.toml"),
        "-p",
        "pub-reader",
        "--bin",
        "copy_ledger_open_parse",
        "--",
        str(fixture),
        PUBLIC_FIXTURE_SHA256,
    ]
    open_first = _run_json(open_command)
    open_second = _run_json(open_command)
    _assert_stable("open_parse", open_first, open_second)

    geometry_command = [
        "cargo",
        "run",
        "--quiet",
        "-p",
        "chaptera-layout-invalidation",
        "--bin",
        "copy_ledger_geometry_probe",
    ]
    geometry_first = _run_json(geometry_command)
    geometry_second = _run_json(geometry_command)
    _assert_stable("geometry_only_reflow", geometry_first, geometry_second)

    replace_command = [
        "cargo",
        "run",
        "--quiet",
        "--manifest-path",
        str(PRODUCER / "Cargo.toml"),
        "-p",
        "pub-editor",
        "--bin",
        "copy_ledger_replace_image_export",
        "--",
        str(fixture),
        PUBLIC_FIXTURE_SHA256,
    ]
    replace_first = _run_json(replace_command)
    replace_second = _run_json(replace_command)
    _assert_stable("replace_image_export", replace_first, replace_second)
    return open_first, geometry_first, replace_first


def _event(
    *,
    event_id: str,
    scenario: str,
    operation: str,
    payload_class: str,
    copy_class: str,
    materialized_bytes: int,
    instances: int,
    reason: str,
    retained_bytes_after: int | None = None,
    allocation_count: int | None = None,
    live_bytes_before: int | None = None,
    live_bytes_after: int | None = None,
    peak_live_bytes: int | None = None,
) -> dict[str, Any]:
    if materialized_bytes < 0 or instances < 0:
        raise AssertionError("negative materialization counter")
    return {
        "event_id": event_id,
        "stage_id": scenario,
        "operation": operation,
        "payload_class": payload_class,
        "payload_identity": f"{scenario}:{event_id}",
        "copy_class": copy_class,
        "reason": reason,
        "logical_bytes": materialized_bytes,
        "materialized_bytes": materialized_bytes,
        "shared_bytes": 0,
        "instances": instances,
        "allocation_count": allocation_count,
        "live_bytes_before": live_bytes_before,
        "live_bytes_after": live_bytes_after,
        "peak_live_bytes": peak_live_bytes,
        "retained_bytes_after": retained_bytes_after,
        "semantic_identity_equal": copy_class != "required_transform",
        "source_identity": f"{scenario}:source",
        "target_identity": f"{scenario}:{event_id}:materialized",
    }


def _python_events(scenario: str, measured: dict[str, Any]) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    allocator = measured["allocator"]
    definitions = {
        "revision.move_nodes_request_normalize": (
            "other",
            "avoidable_duplicate",
            "whole request copied only to canonicalize MoveNodes entry ordering",
        ),
        "revision.executor_project_detach": (
            "authoring_graph_state",
            "required_lifetime_detach",
            "mutable executor receives an isolated project snapshot",
        ),
        "revision.retained_project_snapshot": (
            "authoring_graph_state",
            "required_lifetime_detach",
            "immutable revision history retains the resulting project snapshot",
        ),
        "revision.idempotency_result_snapshot": (
            "serialized_wire_bytes",
            "required_lifetime_detach",
            "idempotency replay retains an isolated accepted-result snapshot",
        ),
    }
    nonzero = [
        row for row in measured["sites"].values()
        if row["materialized_bytes"] > 0
    ]
    for index, row in enumerate(nonzero):
        payload_class, copy_class, reason = definitions[row["site"]]
        events.append(
            _event(
                event_id=f"{scenario}:{row['site']}",
                scenario=scenario,
                operation=row["site"],
                payload_class=payload_class,
                copy_class=copy_class,
                materialized_bytes=row["materialized_bytes"],
                instances=row["instances"],
                reason=reason,
                retained_bytes_after=row["retained_bytes_after"],
                allocation_count=None,
                live_bytes_before=allocator["live_bytes_before"] if index == 0 else None,
                live_bytes_after=allocator["live_bytes_after"] if index == 0 else None,
                peak_live_bytes=allocator["peak_live_bytes"] if index == 0 else None,
            )
        )
    return events


def build_receipt(fixture: pathlib.Path) -> dict[str, Any]:
    move, story = _python_scenarios()
    open_parse, geometry, replace = _rust_scenarios(fixture)

    events = []
    events.extend(
        [
            _event(
                event_id="open_parse:file_buffer",
                scenario="open_parse",
                operation="read_to_end_pub",
                payload_class="source_stream_bytes",
                copy_class="avoidable_duplicate",
                materialized_bytes=open_parse["file_buffer_bytes"],
                instances=open_parse["file_buffer_instances"],
                reason="seekable PUB input is materialized into one whole-file Vec before CFB stream extraction",
                retained_bytes_after=0,
                peak_live_bytes=open_parse["file_buffer_bytes"],
            ),
            _event(
                event_id="open_parse:contents_stream",
                scenario="open_parse",
                operation="read_contents_stream",
                payload_class="source_stream_bytes",
                copy_class="required_lifetime_detach",
                materialized_bytes=open_parse["contents_stream_bytes"],
                instances=open_parse["contents_stream_instances"],
                reason="Contents parser consumes an owned extracted stream",
                retained_bytes_after=0,
            ),
            _event(
                event_id="open_parse:quill_stream",
                scenario="open_parse",
                operation="read_quill_stream",
                payload_class="source_stream_bytes",
                copy_class="required_lifetime_detach",
                materialized_bytes=open_parse["quill_stream_bytes"],
                instances=open_parse["quill_stream_instances"],
                reason="Quill parser consumes an owned extracted stream",
                retained_bytes_after=0,
            ),
            _event(
                event_id="open_parse:escher_stream",
                scenario="open_parse",
                operation="read_escher_stream",
                payload_class="source_stream_bytes",
                copy_class="required_lifetime_detach",
                materialized_bytes=open_parse["escher_stream_bytes"],
                instances=open_parse["escher_stream_instances"],
                reason="OfficeArt parser consumes an owned extracted stream",
                retained_bytes_after=0,
            ),
        ]
    )
    events.extend(_python_events("move_node", move))
    events.extend(_python_events("small_story_edit", story))
    events.append(
        _event(
            event_id="geometry_only_reflow:prepared_units_clone",
            scenario="geometry_only_reflow",
            operation="resolve_story_flow_v1",
            payload_class="shaped_glyph_arrays",
            copy_class="avoidable_duplicate",
            materialized_bytes=geometry["prepared_units_clone_bytes"],
            instances=geometry["prepared_units_clone_instances"],
            reason="prepared typography units are cloned only to sort them before flow",
            retained_bytes_after=0,
            peak_live_bytes=geometry["prepared_units_clone_bytes"],
        )
    )
    events.extend(
        [
            _event(
                event_id="replace_image_export:replacement_asset_clone",
                scenario="replace_image_export",
                operation="idml_replacement_placements",
                payload_class="resource_image_bytes",
                copy_class="avoidable_duplicate",
                materialized_bytes=replace["replacement_image_clone_bytes"],
                instances=replace["replacement_image_clone_instances"],
                reason="replacement image bytes are cloned into IDML placement ownership before package serialization",
                retained_bytes_after=0,
                peak_live_bytes=replace["replacement_image_clone_bytes"],
            ),
            _event(
                event_id="replace_image_export:serialized_output",
                scenario="replace_image_export",
                operation="export_editable_idml",
                payload_class="serialized_wire_bytes",
                copy_class="required_serialization",
                materialized_bytes=replace["editable_serialization_bytes"],
                instances=replace["editable_serialization_instances"],
                reason="editable export boundary requires final serialized package bytes",
                retained_bytes_after=replace["editable_serialization_bytes"],
                peak_live_bytes=replace["editable_serialization_bytes"],
            ),
        ]
    )

    build_sha = os.environ.get("GITHUB_SHA", "local-copy-ledger-instrumentation")
    workload_id = "hosted-five-scenario-copy-ledger-v1"
    identity = identity_hash(
        {
            "workload_id": workload_id,
            "scenarios": [
                "open_parse",
                "move_node",
                "small_story_edit",
                "geometry_only_reflow",
                "replace_image_export",
            ],
        }
    )
    receipt = with_summary(
        {
            "receipt_version": "chaptera.copy-ledger.v1",
            "measurement_class": "synthetic_contract_fixture",
            "producer": {
                "build_sha": build_sha,
                "workload_id": workload_id,
                "runtime_identity": {
                    "runtime": "chaptera-mixed-rust-python-copy-ledger-v1",
                    "platform": platform.system().lower(),
                    "machine": platform.machine() or "unknown",
                    "runner_os": os.environ.get("RUNNER_OS"),
                    "runner_arch": os.environ.get("RUNNER_ARCH"),
                    "cpu": platform.processor() or None,
                    "allocator": "mixed-runtime; fixed-site bytes + explicit unknown allocator counts",
                },
            },
            "equivalence": {
                "semantic_equal": True,
                "baseline_identity": identity,
                "candidate_identity": identity,
            },
            "evidence_authority": {
                "real_pub_runtime": False,
                "technology_decision_allowed": False,
                "blocker": "hosted instrumentation validation only; representative local real-PUB baseline is still required",
            },
            "events": events,
            "summary": None,
            "limitations": [
                "The hosted public fixture validates counter wiring but does not establish current-code baseline bytes for technology decisions.",
                "Python project-copy materialized_bytes use the revision kernel canonical JSON byte-equivalent only while instrumentation is enabled.",
                "Allocator counts that cannot be attributed exactly are null rather than fabricated; Python scenario-level live/peak values come from tracemalloc and Rust fixed-site byte peaks come from known owned buffers.",
                "No source path, PUB bytes, document text, node id, asset digest or customer identity is emitted.",
            ],
        }
    )
    validate_receipt(receipt)

    measurement = copy_ledger_measurement(receipt, {"sha": build_sha})
    if measurement["evidence_authority"]["technology_decision_allowed"] is not False:
        raise AssertionError("hosted instrumentation receipt gained technology authority")
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--fixture", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()

    fixture = args.fixture.resolve()
    if not fixture.is_file():
        raise SystemExit(f"fixture missing: {fixture}")
    receipt = build_receipt(fixture)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(
        json.dumps(
            {
                "receipt": str(args.output),
                "event_count": len(receipt["events"]),
                "scenarios": [
                    "open_parse",
                    "move_node",
                    "small_story_edit",
                    "geometry_only_reflow",
                    "replace_image_export",
                ],
                "technology_decision_allowed": False,
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
