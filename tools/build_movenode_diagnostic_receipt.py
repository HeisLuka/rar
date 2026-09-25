#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import shlex
import subprocess
import sys
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))

from operation_blast_radius_v1 import build_receipt as build_blast_receipt  # noqa: E402
from validate_movenode_diagnostic_receipt import validate as validate_joined_receipt  # noqa: E402

NATIVE_SCHEMA = "chaptera.publisher-movenode-causal-local.v1"
BLAST_SCHEMA = "chaptera.operation-blast-radius.v1"
JOIN_SCHEMA = "chaptera.movenode-diagnostic-receipt.v1"
EMU_PER_POINT = Decimal(12700)


class BuilderError(RuntimeError):
    pass


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load_json_object(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8-sig"))
    except (OSError, json.JSONDecodeError) as error:
        raise BuilderError(f"{label}: cannot read JSON: {error}") from error
    if not isinstance(value, dict):
        raise BuilderError(f"{label}: expected JSON object")
    return value


def parse_decimal(value: Any, label: str) -> Decimal:
    if not isinstance(value, str):
        raise BuilderError(f"{label}: expected decimal string")
    try:
        return Decimal(value)
    except InvalidOperation as error:
        raise BuilderError(f"{label}: invalid decimal") from error


def require_sha(value: Any, label: str) -> str:
    if not isinstance(value, str) or len(value) != 64 or any(
        ch not in "0123456789abcdef" for ch in value
    ):
        raise BuilderError(f"{label}: invalid lowercase SHA-256")
    return value


def file_identity(path: Path, label: str) -> tuple[bytes, str]:
    try:
        data = path.read_bytes()
    except OSError as error:
        raise BuilderError(f"{label}: cannot read artifact: {error}") from error
    return data, sha256_bytes(data)


def require_file_hash(path: Path, expected: str, label: str) -> bytes:
    data, actual = file_identity(path, label)
    if actual != expected:
        raise BuilderError(
            f"{label}: SHA-256 mismatch: expected {expected}, got {actual}"
        )
    return data


def arm_by_mode(manifest: dict[str, Any], mode: str) -> dict[str, Any]:
    arms = manifest.get("arms")
    if not isinstance(arms, list):
        raise BuilderError("native manifest arms must be an array")
    matches = [arm for arm in arms if isinstance(arm, dict) and arm.get("mode") == mode]
    if len(matches) != 1:
        raise BuilderError(f"native manifest must contain exactly one {mode!r} arm")
    return matches[0]


def geometry(arm: dict[str, Any], field: str, label: str) -> dict[str, Decimal]:
    value = arm.get(field)
    if not isinstance(value, dict):
        raise BuilderError(f"{label}.{field}: expected object")
    expected = {"left", "top", "width", "height"}
    if set(value) != expected:
        raise BuilderError(f"{label}.{field}: geometry fields mismatch")
    return {
        key: parse_decimal(value[key], f"{label}.{field}.{key}")
        for key in ("left", "top", "width", "height")
    }


def geometry_wire(value: dict[str, Decimal]) -> dict[str, str]:
    return {key: str(value[key]) for key in ("left", "top", "width", "height")}


def verify_native_manifest(
    manifest: dict[str, Any],
    *,
    source_path: Path,
    axis: str,
) -> tuple[dict[str, Any], dict[str, Any], dict[str, Any], bytes]:
    if manifest.get("schema") != NATIVE_SCHEMA:
        raise BuilderError("native manifest schema mismatch")

    source = manifest.get("source")
    publisher = manifest.get("publisher")
    target = manifest.get("target")
    boundaries = manifest.get("boundaries")
    if not all(isinstance(value, dict) for value in (source, publisher, target, boundaries)):
        raise BuilderError("native manifest missing source/publisher/target/boundaries")

    source_hash = require_sha(source.get("sha256"), "native source.sha256")
    source_bytes = require_file_hash(source_path, source_hash, "immutable source PUB")
    if source.get("unchanged") is not True:
        raise BuilderError("native manifest does not assert immutable source")
    if boundaries.get("disposable_copies_only") is not True:
        raise BuilderError("native manifest did not use disposable copies")
    if boundaries.get("source_pub_immutable") is not True:
        raise BuilderError("native manifest source immutability missing")
    if boundaries.get("native_writer_capability_granted") is not False:
        raise BuilderError("native manifest improperly grants native writer capability")
    if boundaries.get("oracle_tag_is_identity") is not True:
        raise BuilderError("native manifest target is not oracle-tag identified")

    if manifest.get("emu_per_point") != 12700:
        raise BuilderError("native manifest has unexpected EMU/point conversion")
    delta_points = parse_decimal(manifest.get("delta_points"), "native delta_points")
    if delta_points == 0:
        raise BuilderError("native delta_points must be non-zero")

    if axis not in {"x", "y"}:
        raise BuilderError("axis must be x or y")
    control = arm_by_mode(manifest, "control")
    mutation = arm_by_mode(manifest, axis)

    for label, arm in (("control", control), ("mutation", mutation)):
        if arm.get("baseline_source_sha256") != source_hash:
            raise BuilderError(f"{label} arm baseline source hash mismatch")
        if str(arm.get("publisher_version")) != str(publisher.get("version")):
            raise BuilderError(f"{label} arm Publisher version mismatch")
        if str(arm.get("publisher_build")) != str(publisher.get("build")):
            raise BuilderError(f"{label} arm Publisher build mismatch")
        if arm.get("page_id") != target.get("page_id"):
            raise BuilderError(f"{label} arm PageID mismatch")
        tag = arm.get("oracle_tag")
        if not isinstance(tag, dict):
            raise BuilderError(f"{label} arm oracle_tag missing")
        if tag.get("name") != target.get("tag_name") or tag.get("value") != target.get("tag_value"):
            raise BuilderError(f"{label} arm oracle tag mismatch")

    c_before = geometry(control, "before", "control")
    c_after_set = geometry(control, "after_set", "control")
    c_reopen = geometry(control, "reopen", "control")
    if c_before != c_after_set or c_before != c_reopen:
        raise BuilderError("matched no-op control changed Publisher COM geometry")

    m_before = geometry(mutation, "before", "mutation")
    m_after_set = geometry(mutation, "after_set", "mutation")
    m_reopen = geometry(mutation, "reopen", "mutation")
    if m_before != c_before:
        raise BuilderError("mutation and control do not start from identical COM geometry")
    if m_after_set != m_reopen:
        raise BuilderError("native position mutation did not survive fresh reopen")
    if m_before["width"] != m_reopen["width"] or m_before["height"] != m_reopen["height"]:
        raise BuilderError("native MoveNode arm changed width/height")

    dx = m_reopen["left"] - m_before["left"]
    dy = m_reopen["top"] - m_before["top"]
    if axis == "x":
        if dx != delta_points or dy != 0:
            raise BuilderError("native X arm is not the requested X-only delta")
    else:
        if dy != delta_points or dx != 0:
            raise BuilderError("native Y arm is not the requested Y-only delta")

    return control, mutation, {
        "source_hash": source_hash,
        "publisher_version": str(publisher.get("version")),
        "publisher_build": str(publisher.get("build")),
        "page_id": target.get("page_id"),
        "tag_name": target.get("tag_name"),
        "tag_value": target.get("tag_value"),
        "control_before": c_before,
        "control_after": c_reopen,
        "mutation_before": m_before,
        "mutation_after": m_reopen,
    }, source_bytes


def load_agent_move(trace_path: Path) -> dict[str, Any]:
    commits: list[dict[str, Any]] = []
    try:
        lines = trace_path.read_text(encoding="utf-8-sig").splitlines()
    except OSError as error:
        raise BuilderError(f"agent trace: cannot read: {error}") from error

    for line_number, line in enumerate(lines, 1):
        if not line.strip():
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError as error:
            raise BuilderError(f"agent trace line {line_number}: invalid JSON") from error
        if not isinstance(value, dict):
            continue
        if (
            value.get("message_type") != "trace"
            or value.get("command") != "edit.apply"
            or value.get("event_kind") != "durable_commit"
        ):
            continue
        payload = value.get("payload")
        if not isinstance(payload, dict):
            continue
        operation = payload.get("operation")
        if not isinstance(operation, dict) or operation.get("kind") != "move_node":
            continue
        commits.append(value)

    if len(commits) != 1:
        raise BuilderError(
            f"agent trace must contain exactly one durable MoveNode commit, found {len(commits)}"
        )

    event = commits[0]
    payload = event["payload"]
    operation = payload["operation"]
    scene = payload.get("scene_delta")
    if not isinstance(scene, dict):
        raise BuilderError("agent MoveNode trace missing scene_delta")
    if scene.get("geometry_sync_policy") != "apply_authored_origin_geometry":
        raise BuilderError("agent MoveNode did not pass direct-page-local geometry authority")
    if scene.get("origin_node_id") != operation.get("node_id"):
        raise BuilderError("agent scene origin NodeId differs from canonical MoveNode NodeId")
    if scene.get("geometry_changed") is not True:
        raise BuilderError("agent MoveNode trace does not report geometry change")

    before = operation.get("before")
    after = operation.get("after")
    if not isinstance(before, dict) or not isinstance(after, dict):
        raise BuilderError("agent MoveNode missing before/after RectEmu")
    for label, rect in (("before", before), ("after", after)):
        if set(rect) != {"x", "y", "width", "height"}:
            raise BuilderError(f"agent MoveNode {label} RectEmu fields mismatch")
        if not all(isinstance(rect[key], int) and not isinstance(rect[key], bool) for key in rect):
            raise BuilderError(f"agent MoveNode {label} RectEmu must contain integers")
        if rect["width"] <= 0 or rect["height"] <= 0:
            raise BuilderError(f"agent MoveNode {label} RectEmu must have positive size")

    if before["width"] != after["width"] or before["height"] != after["height"]:
        raise BuilderError("canonical MoveNode changed width/height")

    source_hash = require_sha(event.get("source_hash"), "agent trace source_hash")
    operation_id = payload.get("operation_id")
    instance_id = scene.get("instance_id")
    before_state_id = payload.get("before_state_id")
    after_state_id = payload.get("after_state_id")
    for value, label in (
        (operation_id, "operation_id"),
        (operation.get("node_id"), "node_id"),
        (instance_id, "scene_instance_id"),
        (before_state_id, "before_state_id"),
        (after_state_id, "after_state_id"),
    ):
        if not isinstance(value, str) or not value:
            raise BuilderError(f"agent MoveNode trace missing {label}")

    return {
        "source_hash": source_hash,
        "operation_id": operation_id,
        "node_id": operation["node_id"],
        "scene_instance_id": instance_id,
        "before_state_id": before_state_id,
        "after_state_id": after_state_id,
        "before": before,
        "after": after,
    }


def verify_cross_layer_delta(move: dict[str, Any], native: dict[str, Any], axis: str, tolerance_emu: int) -> None:
    before = move["before"]
    after = move["after"]
    dx_emu = after["x"] - before["x"]
    dy_emu = after["y"] - before["y"]
    n_before = native["mutation_before"]
    n_after = native["mutation_after"]
    dx_points = n_after["left"] - n_before["left"]
    dy_points = n_after["top"] - n_before["top"]

    if axis == "x":
        if dx_emu == 0 or dy_emu != 0:
            raise BuilderError("canonical MoveNode is not X-only")
        predicted = dx_points * EMU_PER_POINT
        error = abs(Decimal(dx_emu) - predicted)
    else:
        if dy_emu == 0 or dx_emu != 0:
            raise BuilderError("canonical MoveNode is not Y-only")
        predicted = dy_points * EMU_PER_POINT
        error = abs(Decimal(dy_emu) - predicted)
    if error > Decimal(tolerance_emu):
        raise BuilderError(
            f"canonical/native movement mismatch: error {error} EMU exceeds tolerance {tolerance_emu}"
        )


def parse_command_json(raw: str) -> list[str]:
    try:
        value = json.loads(raw)
    except json.JSONDecodeError as error:
        raise BuilderError("parser command must be a JSON array of strings") from error
    if not isinstance(value, list) or not value or not all(isinstance(item, str) and item for item in value):
        raise BuilderError("parser command must be a non-empty JSON array of non-empty strings")
    return value


def parser_accepts(command: list[str], path: Path, label: str) -> None:
    completed = subprocess.run(
        [*command, str(path)],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        stderr = completed.stderr.strip()
        raise BuilderError(
            f"{label}: Chaptera parser rejected artifact with exit {completed.returncode}"
            + (f": {stderr}" if stderr else "")
        )


def arm_file(arm: dict[str, Any], field: str, label: str) -> tuple[Path, bytes, str]:
    path_value = arm.get(field)
    if not isinstance(path_value, str) or not path_value:
        raise BuilderError(f"{label}.{field} missing")
    path = Path(path_value)
    sha_field = "first_save_sha256" if field == "first_save_path" else "second_save_sha256"
    expected = require_sha(arm.get(sha_field), f"{label}.{sha_field}")
    data = require_file_hash(path, expected, f"{label}.{field}")
    return path, data, expected


def build(
    *,
    source_path: Path,
    agent_trace_path: Path,
    native_manifest_path: Path,
    axis: str,
    parser_command: list[str],
    blast_out: Path,
    receipt_out: Path,
    tolerance_emu: int,
) -> tuple[dict[str, Any], dict[str, Any]]:
    if tolerance_emu < 0 or tolerance_emu > 127:
        raise BuilderError("tolerance_emu must be between 0 and 127")

    manifest = load_json_object(native_manifest_path, "native manifest")
    control, mutation, native, source_bytes = verify_native_manifest(
        manifest, source_path=source_path, axis=axis
    )
    move = load_agent_move(agent_trace_path)
    if move["source_hash"] != native["source_hash"]:
        raise BuilderError("agent MoveNode source hash differs from native experiment source")

    verify_cross_layer_delta(move, native, axis, tolerance_emu)

    control_first_path, control_first_bytes, control_first_hash = arm_file(
        control, "first_save_path", "control"
    )
    control_second_path, control_second_bytes, control_second_hash = arm_file(
        control, "second_save_path", "control"
    )
    mutation_first_path, mutation_first_bytes, mutation_first_hash = arm_file(
        mutation, "first_save_path", "mutation"
    )
    mutation_second_path, mutation_second_bytes, mutation_second_hash = arm_file(
        mutation, "second_save_path", "mutation"
    )

    for path, label in (
        (source_path, "source"),
        (control_first_path, "control first Save"),
        (control_second_path, "control second Save"),
        (mutation_first_path, "mutation first Save"),
        (mutation_second_path, "mutation second Save"),
    ):
        parser_accepts(parser_command, path, label)

    evidence = {
        "producer": {
            "name": "publisher_movenode_causal",
            "publisher_version": native["publisher_version"],
            "publisher_build": native["publisher_build"],
        },
        "operation": {
            "kind": "MoveNode",
            "operation_id": move["operation_id"],
            "node_id": move["node_id"],
        },
        "requested_streams": [],
        "expected_derived_streams": [],
        "requested_records": [],
        "expected_derived_records": [],
        "requested_entities": [],
        "expected_derived_entities": [],
        "arms": {
            "source": {"parser": {"accepted": True, "diagnostic_codes": []}},
            "control": {"parser": {"accepted": True, "diagnostic_codes": []}},
            "mutation": {"parser": {"accepted": True, "diagnostic_codes": []}},
        },
    }

    blast = build_blast_receipt(
        source_bytes,
        control_first_bytes,
        mutation_first_bytes,
        evidence=evidence,
        second_save=mutation_second_bytes,
    )
    blast_raw = (
        json.dumps(blast, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    ).encode("utf-8")
    shape_identity = (
        f"pageid:{native['page_id']}|tag:{native['tag_name']}={native['tag_value']}"
    )
    receipt = {
        "receipt_version": JOIN_SCHEMA,
        "source_sha256": native["source_hash"],
        "chaptera": {
            "operation_kind": "MoveNode",
            "node_id": move["node_id"],
            "scene_instance_id": move["scene_instance_id"],
            "admission": "direct_page_local",
            "base_revision_id": move["before_state_id"],
            "result_revision_id": move["after_state_id"],
            "before": move["before"],
            "after": move["after"],
        },
        "native_experiment": {
            "publisher_version": native["publisher_version"],
            "publisher_build": native["publisher_build"],
            "shape_identity": shape_identity,
            "axis": axis,
            "emu_per_point": 12700,
            "tolerance_emu": tolerance_emu,
            "control": {
                "baseline_source_sha256": native["source_hash"],
                "first_save_sha256": control_first_hash,
                "second_save_sha256": control_second_hash,
                "before": geometry_wire(native["control_before"]),
                "after": geometry_wire(native["control_after"]),
                "parser_accepted": True,
                "publisher_reopen_accepted": True,
            },
            "mutation": {
                "baseline_source_sha256": native["source_hash"],
                "first_save_sha256": mutation_first_hash,
                "second_save_sha256": mutation_second_hash,
                "before": geometry_wire(native["mutation_before"]),
                "after": geometry_wire(native["mutation_after"]),
                "parser_accepted": True,
                "publisher_reopen_accepted": True,
            },
        },
        "blast_radius": {
            "receipt_sha256": sha256_bytes(blast_raw),
            "schema_version": BLAST_SCHEMA,
            "source_sha256": native["source_hash"],
            "control_sha256": control_first_hash,
            "mutation_sha256": mutation_first_hash,
            "second_save_sha256": mutation_second_hash,
        },
        "invariants": {
            "exactly_one_durable_movenode": True,
            "native_pub_writer_capability_granted": False,
        },
    }

    validate_joined_receipt(receipt, blast, blast_raw)

    encoded = json.dumps(receipt, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    for forbidden in (
        str(source_path),
        str(agent_trace_path),
        str(native_manifest_path),
        str(control_first_path),
        str(control_second_path),
        str(mutation_first_path),
        str(mutation_second_path),
    ):
        if forbidden and forbidden in encoded:
            raise BuilderError("source-free receipt unexpectedly contains a local path")

    blast_out.parent.mkdir(parents=True, exist_ok=True)
    receipt_out.parent.mkdir(parents=True, exist_ok=True)
    blast_out.write_bytes(blast_raw)
    receipt_out.write_text(encoded, encoding="utf-8")
    return blast, receipt


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Build OperationBlastRadiusV1 + MoveNodeDiagnosticReceiptV1 from real local evidence"
    )
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--agent-trace", required=True, type=Path)
    parser.add_argument("--native-manifest", required=True, type=Path)
    parser.add_argument("--axis", choices=("x", "y"), required=True)
    parser.add_argument(
        "--parser-command-json",
        required=True,
        help='JSON argv prefix, e.g. ["C:\\\\...\\\\chaptera-editor.exe","--smoke-check"]',
    )
    parser.add_argument("--blast-out", required=True, type=Path)
    parser.add_argument("--receipt-out", required=True, type=Path)
    parser.add_argument("--tolerance-emu", type=int, default=127)
    args = parser.parse_args()

    try:
        blast, receipt = build(
            source_path=args.source,
            agent_trace_path=args.agent_trace,
            native_manifest_path=args.native_manifest,
            axis=args.axis,
            parser_command=parse_command_json(args.parser_command_json),
            blast_out=args.blast_out,
            receipt_out=args.receipt_out,
            tolerance_emu=args.tolerance_emu,
        )
    except (BuilderError, OSError, ValueError, KeyError) as error:
        print(f"movenode-diagnostic-builder: {error}", file=sys.stderr)
        return 2

    print(
        json.dumps(
            {
                "blast_schema": blast["schema_version"],
                "receipt_version": receipt["receipt_version"],
                "source_sha256": receipt["source_sha256"],
                "axis": receipt["native_experiment"]["axis"],
                "unexplained_collateral": blast["classification_counts"][
                    "unexplained_collateral"
                ],
                "second_save_convergence": blast["second_save_convergence"]["status"],
                "native_pub_write": False,
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
