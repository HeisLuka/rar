#!/usr/bin/env python3
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "apps" / "web" / "export-preview-v1.schema.json"

LOSS_FIELDS = ("loss_kind", "severity", "reversible", "code")
DISPOSITIONS = ("preserved", "approximated", "flattened", "rasterized", "unsupported")


def validate_schema(preview):
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(
        Draft202012Validator(schema).iter_errors(preview),
        key=lambda error: list(error.path),
    )
    if errors:
        detail = "\n".join(f"{list(error.path)}: {error.message}" for error in errors)
        raise AssertionError("export preview schema validation failed\n" + detail)


def validate_semantics(preview):
    counts = preview["counts"]
    observed = {key: 0 for key in DISPOSITIONS}
    blocking = 0

    for index, item in enumerate(preview["items"]):
        disposition = item["disposition"]
        observed[disposition] += 1
        present_loss_fields = [field for field in LOSS_FIELDS if field in item]

        if disposition == "preserved":
            if present_loss_fields:
                raise AssertionError(
                    f"preserved item {index} carries loss fields: {present_loss_fields}"
                )
        elif len(present_loss_fields) != len(LOSS_FIELDS):
            missing = [field for field in LOSS_FIELDS if field not in item]
            raise AssertionError(
                f"lossy item {index} is missing canonical loss fields: {missing}"
            )

        if item.get("severity") == "blocking":
            blocking += 1

    for disposition in DISPOSITIONS:
        if counts[disposition] != observed[disposition]:
            raise AssertionError(
                f"{disposition} count mismatch: {counts[disposition]} != {observed[disposition]}"
            )

    if counts["blocking"] != blocking:
        raise AssertionError(
            f"blocking count mismatch: {counts['blocking']} != {blocking}"
        )

    expected_can_serialize = blocking == 0
    if preview["can_serialize"] is not expected_can_serialize:
        raise AssertionError(
            "can_serialize must be exactly equivalent to zero blocking losses"
        )

    return {
        "receipt_kind": preview["protocol_version"],
        "document_id": preview["document_id"],
        "source_hash": preview["source_hash"],
        "revision_id": preview["revision_id"],
        "target": preview["target"]["format"],
        "can_serialize": preview["can_serialize"],
        "loss_item_count": sum(
            1 for item in preview["items"] if item["disposition"] != "preserved"
        ),
        "blocking_count": blocking,
        "source_label_present": False,
    }


def validate_scene_binding(preview, scene):
    for field in ("document_id", "source_hash", "revision_id"):
        if preview[field] != scene[field]:
            raise AssertionError(f"export preview {field} does not match current Scene")


def main():
    if len(sys.argv) not in {2, 3}:
        print(
            "usage: validate_export_preview.py PREVIEW.json [SCENE.json]",
            file=sys.stderr,
        )
        return 2

    preview = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
    validate_schema(preview)
    summary = validate_semantics(preview)

    if len(sys.argv) == 3:
        scene = json.loads(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"))
        validate_scene_binding(preview, scene)
        summary["scene_identity_bound"] = True

    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
