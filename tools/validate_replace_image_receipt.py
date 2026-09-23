#!/usr/bin/env python3
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
BASE = ROOT / "packages" / "protocol" / "editor-image-replace" / "v1"
UI_SCHEMA = BASE / "ui-producer-receipt.schema.json"
EXPORT_SCHEMA = BASE / "export-producer-receipt.schema.json"


def _validate_schema(receipt, schema_path):
    schema = json.loads(schema_path.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(Draft202012Validator(schema).iter_errors(receipt), key=lambda e: list(e.path))
    if errors:
        detail = "\n".join(f"{list(e.path)}: {e.message}" for e in errors)
        raise AssertionError(f"{schema_path.name} validation failed\n{detail}")


def validate_ui_receipt(receipt):
    _validate_schema(receipt, UI_SCHEMA)

    gate = receipt["target_gate"]
    expected_gate = {
        "image_slot_present": True,
        "explicit_crop_present": False,
        "direct_page_owned": True,
        "valid_bounds": True,
        "target_id_redacted": True,
    }
    if gate != expected_gate:
        raise AssertionError("ReplaceImage target gate widened or leaked identity")

    asset = receipt["asset_import"]
    if asset["mime"] not in {"image/png", "image/jpeg"}:
        raise AssertionError("ReplaceImage asset MIME widened")
    for key in (
        "non_empty", "signature_matches_declared_mime", "content_addressed_sha256",
        "duplicate_same_sha_reused", "mime_conflict_rejected", "asset_sha_redacted"
    ):
        if not asset[key]:
            raise AssertionError(f"asset import proof missing: {key}")
    if asset["filename_is_identity"] or asset["url_is_identity"]:
        raise AssertionError("replacement asset identity must be content-addressed only")

    commit = receipt["commit"]
    if commit["operation_count_after"] != commit["operation_count_before"] + 1:
        raise AssertionError("ReplaceImage commit must append exactly one operation")
    if not commit["registered_asset_required"] or not commit["same_asset_no_change_rejected"]:
        raise AssertionError("ReplaceImage registered-asset/no-op boundary is incomplete")
    if not commit["source_pub_unchanged"]:
        raise AssertionError("ReplaceImage must not mutate source PUB")

    binding = receipt["replacement_binding"]
    if binding["content_derived"]:
        raise AssertionError("replacement binding id must be opaque, not content-derived")
    for key in (
        "import_matches_committed_asset",
        "committed_matches_preview_asset",
        "committed_matches_redo_asset",
        "committed_matches_fresh_replay_asset",
    ):
        if not binding[key]:
            raise AssertionError(f"replacement identity continuity missing: {key}")

    if not all(receipt["project_replay"].values()):
        raise AssertionError("ReplaceImage project/undo/replay proof is incomplete")
    if not all(receipt["preview"].values()):
        raise AssertionError("ReplaceImage preview/source-state proof is incomplete")
    if not all(receipt["negative_probes"].values()):
        raise AssertionError("ReplaceImage negative probes are incomplete")
    if any(receipt["privacy"].values()):
        raise AssertionError("source-free ReplaceImage UI receipt admits private values")

    return {
        "receipt_version": receipt["receipt_version"],
        "operation_contract": receipt["operation_contract"],
        "mime": asset["mime"],
        "crop_free_only": True,
        "content_addressed_asset": True,
        "one_durable_operation": True,
        "replacement_binding_id": binding["binding_id"],
        "undo_redo_replay": True,
        "source_pub_immutable": True,
        "source_free_receipt": True,
    }


def validate_export_receipt(receipt):
    _validate_schema(receipt, EXPORT_SCHEMA)

    if receipt["effective_asset"] != {
        "resolved_by_content_sha256": True,
        "asset_sha_redacted": True,
        "source_asset_silently_reused": False,
    }:
        raise AssertionError("effective replacement asset boundary is incomplete")

    binding = receipt["replacement_binding"]
    if binding["content_derived"]:
        raise AssertionError("replacement binding id must be opaque, not content-derived")
    for key in (
        "effective_asset_matches_committed_replacement",
        "idml_matches_effective_asset",
        "odg_matches_effective_asset",
    ):
        if not binding[key]:
            raise AssertionError(f"replacement export identity continuity missing: {key}")

    idml = receipt["idml"]
    expected_idml = {
        "can_serialize": True,
        "exact_replacement_bytes": True,
        "frame_geometry": "preserved",
        "content_transform": "approximated",
        "z_order": "approximated",
    }
    if idml != expected_idml:
        raise AssertionError("IDML ReplaceImage export contract changed")

    odg = receipt["odg"]
    expected_odg = {
        "can_serialize": True,
        "exact_replacement_bytes": True,
        "frame_geometry": "preserved",
        "content_transform": "approximated",
        "z_order": "preserved",
    }
    if odg != expected_odg:
        raise AssertionError("ODG ReplaceImage export contract changed")

    if receipt["unsupported_target"] != {
        "policy": "block_with_explicit_loss",
        "silent_drop": False,
        "silent_source_fallback": False,
    }:
        raise AssertionError("unsupported export target must block explicitly")

    if receipt["source_boundary"] != {
        "source_pub_unchanged": True,
        "native_pub_writer_promoted": False,
    }:
        raise AssertionError("ReplaceImage export must not widen native PUB writer authority")

    if any(receipt["privacy"].values()):
        raise AssertionError("source-free ReplaceImage export receipt admits private values")

    return {
        "receipt_version": receipt["receipt_version"],
        "export_contract": receipt["export_contract"],
        "replacement_binding_id": binding["binding_id"],
        "idml_exact_bytes": True,
        "odg_exact_bytes": True,
        "content_transform_loss_explicit": True,
        "unsupported_target_fail_closed": True,
        "no_silent_source_fallback": True,
        "source_free_receipt": True,
    }


def validate_pair_receipts(ui_receipt, export_receipt):
    ui_summary = validate_ui_receipt(ui_receipt)
    export_summary = validate_export_receipt(export_receipt)

    if ui_summary["replacement_binding_id"] != export_summary["replacement_binding_id"]:
        raise AssertionError("ReplaceImage UI/export receipts do not bind the same replacement session")
    if ui_receipt["build"] != export_receipt["build"]:
        raise AssertionError("ReplaceImage UI/export receipts must come from the same Chaptera build")
    if ui_receipt["fixture_kind"] != export_receipt["fixture_kind"]:
        raise AssertionError("ReplaceImage UI/export receipts must use the same fixture kind")

    return {
        "replacement_binding_id": ui_summary["replacement_binding_id"],
        "same_build": True,
        "same_fixture_kind": True,
        "ui_validated": True,
        "export_validated": True,
        "replacement_identity_bound_end_to_end": True,
    }


def main():
    if len(sys.argv) == 4 and sys.argv[1] == "pair":
        ui_receipt = json.loads(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"))
        export_receipt = json.loads(pathlib.Path(sys.argv[3]).read_text(encoding="utf-8"))
        summary = validate_pair_receipts(ui_receipt, export_receipt)
    elif len(sys.argv) == 3 and sys.argv[1] in {"ui", "export"}:
        receipt = json.loads(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"))
        summary = validate_ui_receipt(receipt) if sys.argv[1] == "ui" else validate_export_receipt(receipt)
    else:
        print(
            "usage: validate_replace_image_receipt.py ui|export RECEIPT.json\n"
            "   or: validate_replace_image_receipt.py pair UI.json EXPORT.json",
            file=sys.stderr,
        )
        return 2
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
