#!/usr/bin/env python3
"""Build a source-free ReplaceImage editable-export receipt.

The authoritative local producer owns actual project/export execution. This
builder verifies the private identity chain locally and emits only the existing
redacted public receipt. It does not implement image export or a second editor.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import subprocess
import sys
from typing import Any, Sequence

ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS = ROOT / "tools"
SHA_RE = re.compile(r"^[0-9a-f]{64}$")


def _invoke(command: Sequence[str], payload: dict[str, Any]) -> dict[str, Any]:
    completed = subprocess.run(
        list(command),
        input=json.dumps(payload, ensure_ascii=False, separators=(",", ":")),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            "ReplaceImage export producer failed"
            + (f"\nstderr:\n{completed.stderr}" if completed.stderr else "")
        )
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError("ReplaceImage export producer returned invalid JSON") from exc
    if not isinstance(value, dict):
        raise RuntimeError("ReplaceImage export producer response must be an object")
    return value


def _sha(value: Any, label: str) -> str:
    if not isinstance(value, str) or not SHA_RE.fullmatch(value):
        raise RuntimeError(f"{label} must be lowercase SHA-256")
    return value


def _exact_keys(value: dict[str, Any], expected: set[str], label: str) -> None:
    actual = set(value)
    if actual != expected:
        raise RuntimeError(
            f"{label} fields mismatch: missing={sorted(expected-actual)} "
            f"extra={sorted(actual-expected)}"
        )


def build_export_receipt(
    ui_receipt: dict[str, Any],
    producer_command: Sequence[str],
    *,
    source_hash: str,
    projection_instance_admitted: bool = False,
) -> dict[str, Any]:
    sys.path.insert(0, str(TOOLS))
    from validate_replace_image_receipt import (
        validate_export_receipt,
        validate_pair_receipts,
        validate_ui_receipt,
    )

    validate_ui_receipt(ui_receipt)
    _sha(source_hash, "source_hash")

    fixture_kind = ui_receipt["fixture_kind"]
    if fixture_kind == "real_pub_sanitized" and not projection_instance_admitted:
        raise RuntimeError("projection_instance_gate_unresolved")

    binding_id = ui_receipt["replacement_binding"]["binding_id"]
    proof = _invoke(
        producer_command,
        {
            "action": "export_replace_image",
            "source_hash": source_hash,
            "replacement_binding_id": binding_id,
            "fixture_kind": fixture_kind,
        },
    )
    _exact_keys(
        proof,
        {
            "replacement_binding_id",
            "source_hash_after",
            "source_asset_sha256",
            "committed_asset_sha256",
            "effective_asset_sha256",
            "idml",
            "odg",
            "unsupported_target",
            "native_pub_writer_promoted",
        },
        "export producer response",
    )

    if proof["replacement_binding_id"] != binding_id:
        raise RuntimeError("producer replacement binding does not match UI receipt")
    if proof["source_hash_after"] != source_hash:
        raise RuntimeError("editable export changed immutable source PUB identity")

    source_asset = _sha(proof["source_asset_sha256"], "source_asset_sha256")
    committed = _sha(proof["committed_asset_sha256"], "committed_asset_sha256")
    effective = _sha(proof["effective_asset_sha256"], "effective_asset_sha256")
    if committed == source_asset:
        raise RuntimeError("ReplaceImage export proof did not change the source asset")
    if effective != committed:
        raise RuntimeError("effective export asset differs from committed replacement")

    expected_target_keys = {
        "can_serialize",
        "embedded_asset_sha256",
        "frame_geometry",
        "content_transform",
        "z_order",
    }
    target_expectations = {
        "idml": ("approximated", "approximated"),
        "odg": ("approximated", "preserved"),
    }
    for target in ("idml", "odg"):
        value = proof[target]
        if not isinstance(value, dict):
            raise RuntimeError(f"{target} proof must be an object")
        _exact_keys(value, expected_target_keys, f"{target} proof")
        if value["can_serialize"] is not True:
            raise RuntimeError(f"{target} did not serialize replacement")
        embedded = _sha(value["embedded_asset_sha256"], f"{target}.embedded_asset_sha256")
        if embedded != effective:
            raise RuntimeError(f"{target} embedded bytes differ from effective replacement")
        expected_transform, expected_z = target_expectations[target]
        if value["frame_geometry"] != "preserved":
            raise RuntimeError(f"{target} frame geometry changed")
        if value["content_transform"] != expected_transform:
            raise RuntimeError(f"{target} content-transform classification changed")
        if value["z_order"] != expected_z:
            raise RuntimeError(f"{target} z-order classification changed")

    unsupported = proof["unsupported_target"]
    if unsupported != {
        "blocked": True,
        "explicit_loss": True,
        "silent_drop": False,
        "silent_source_fallback": False,
    }:
        raise RuntimeError("unsupported target did not fail closed with explicit loss")
    if proof["native_pub_writer_promoted"] is not False:
        raise RuntimeError("ReplaceImage export cannot promote native PUB writer authority")

    receipt = {
        "receipt_version": "chaptera.replace-image-export-producer-receipt.v1",
        "export_contract": "chaptera.replace-image-export.v1",
        "producer": {
            "kind": "chaptera_desktop_editor",
            "integration": "local_private",
        },
        "build": ui_receipt["build"],
        "fixture_kind": fixture_kind,
        "effective_asset": {
            "resolved_by_content_sha256": True,
            "asset_sha_redacted": True,
            "source_asset_silently_reused": False,
        },
        "replacement_binding": {
            "binding_id": binding_id,
            "content_derived": False,
            "effective_asset_matches_committed_replacement": True,
            "idml_matches_effective_asset": True,
            "odg_matches_effective_asset": True,
        },
        "idml": {
            "can_serialize": True,
            "exact_replacement_bytes": True,
            "frame_geometry": "preserved",
            "content_transform": "approximated",
            "z_order": "approximated",
        },
        "odg": {
            "can_serialize": True,
            "exact_replacement_bytes": True,
            "frame_geometry": "preserved",
            "content_transform": "approximated",
            "z_order": "preserved",
        },
        "unsupported_target": {
            "policy": "block_with_explicit_loss",
            "silent_drop": False,
            "silent_source_fallback": False,
        },
        "source_boundary": {
            "source_pub_unchanged": True,
            "native_pub_writer_promoted": False,
        },
        "privacy": {
            "pub_bytes_in_receipt": False,
            "pub_filename_in_receipt": False,
            "local_path_in_receipt": False,
            "source_hash_in_receipt": False,
            "node_id_in_receipt": False,
            "asset_sha_in_receipt": False,
            "replacement_bytes_in_receipt": False,
            "export_bytes_in_receipt": False,
            "customer_identity_in_receipt": False,
        },
    }

    validate_export_receipt(receipt)
    validate_pair_receipts(ui_receipt, receipt)
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--ui-receipt", required=True, type=pathlib.Path)
    parser.add_argument("--source-hash", required=True)
    parser.add_argument("--projection-instance-admitted", action="store_true")
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("producer_command", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    command = list(args.producer_command)
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        parser.error("producer command is required after --")

    ui_receipt = json.loads(args.ui_receipt.read_text(encoding="utf-8"))
    receipt = build_export_receipt(
        ui_receipt,
        command,
        source_hash=args.source_hash,
        projection_instance_admitted=args.projection_instance_admitted,
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
