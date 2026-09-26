#!/usr/bin/env python3
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "apps" / "web" / "acceptance" / "viewer-geometry-receipt.schema.json"


def validate_schema(receipt):
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(
        Draft202012Validator(schema).iter_errors(receipt),
        key=lambda error: list(error.path),
    )
    if errors:
        detail = "\n".join(f"{list(error.path)}: {error.message}" for error in errors)
        raise AssertionError("Viewer geometry receipt schema validation failed\n" + detail)


def main():
    if len(sys.argv) != 2:
        print("usage: validate_viewer_geometry_receipt.py RECEIPT.json", file=sys.stderr)
        return 2
    path = pathlib.Path(sys.argv[1])
    receipt = json.loads(path.read_text(encoding="utf-8"))
    validate_schema(receipt)
    source = receipt["document"]["source"]
    summary = {
        "receipt_kind": "chaptera.viewer-geometry-receipt.v0.1",
        "source_hash": source["source_hash"],
        "byte_len": source["byte_len"],
        "page_count": len(receipt["document"]["pages"]),
        "node_count": len(receipt["scene"]["nodes"]),
        "story_count": len(receipt["document"]["stories"]),
        "text_fragment_count": len(receipt.get("text_fragments", [])),
        "image_descriptor_count": len(receipt.get("images", [])),
        "allowlist_valid": True,
    }
    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
