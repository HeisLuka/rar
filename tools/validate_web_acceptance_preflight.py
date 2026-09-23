#!/usr/bin/env python3
import argparse
import hashlib
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "apps" / "web" / "acceptance" / "web-acceptance-inputs.v1.json"
BROWSER_SCHEMA = ROOT / "apps" / "web" / "acceptance" / "browser-acceptance-receipt.schema.json"

sys.path.insert(0, str(ROOT / "tools"))
from adapt_viewer_scene_v1 import adapt_viewer_geometry
from validate_revision_producer_receipt import validate_schema as validate_revision_schema
from validate_revision_producer_receipt import validate_semantics as validate_revision_semantics


def load_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def validate_json_schema(schema_path, value):
    schema = load_json(schema_path)
    Draft202012Validator.check_schema(schema)
    errors = sorted(Draft202012Validator(schema).iter_errors(value), key=lambda e: list(e.path))
    if errors:
        detail = "\n".join(f"{list(e.path)}: {e.message}" for e in errors)
        raise AssertionError(detail)


def sha256_file(path):
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--fixture", type=pathlib.Path, required=True)
    parser.add_argument("--mode", choices=("preflight", "strict"), default="preflight")
    parser.add_argument("--output", type=pathlib.Path)
    args = parser.parse_args()

    manifest = load_json(MANIFEST)
    if manifest.get("manifest_version") != "chaptera.web-acceptance-inputs.v1":
        raise AssertionError("unexpected acceptance manifest version")

    fixture = manifest["fixture"]
    if args.fixture.stat().st_size != fixture["byte_len"]:
        raise AssertionError("fixture byte length mismatch")
    actual_sha = sha256_file(args.fixture)
    if actual_sha != fixture["sha256"]:
        raise AssertionError("fixture SHA-256 mismatch")

    blockers = []
    validated = {
        "fixture": True,
        "revision_producer": False,
        "viewer_geometry": False,
        "browser_acceptance": False,
    }

    required = manifest["required_real_receipts"]
    revision_path = ROOT / required["revision_producer"]
    viewer_path = ROOT / required["viewer_geometry"]
    browser_path = ROOT / required["browser_acceptance"]

    revision = None
    revision_summary = None
    if revision_path.exists():
        revision = load_json(revision_path)
        validate_revision_schema(revision)
        revision_summary = validate_revision_semantics(revision)
        if revision["source_hash"] != fixture["sha256"]:
            raise AssertionError("revision receipt source hash is not the pinned real PUB")
        validated["revision_producer"] = True
    else:
        blockers.append("missing_real_revision_producer_receipt")

    scene_snapshot = None
    if viewer_path.exists():
        if revision is None:
            blockers.append("viewer_receipt_cannot_bind_without_revision_receipt")
        else:
            viewer = load_json(viewer_path)
            scene_snapshot = adapt_viewer_geometry(
                viewer,
                revision["document_id"],
                revision["baseline"]["revision_id"],
            )
            validate_json_schema(
                ROOT / "packages" / "protocol" / "scene" / "v1" / "snapshot.schema.json",
                scene_snapshot,
            )
            if scene_snapshot["source_hash"] != fixture["sha256"]:
                raise AssertionError("Viewer receipt source hash is not the pinned real PUB")
            validated["viewer_geometry"] = True
    else:
        blockers.append("missing_real_viewer_geometry_receipt")

    browser = None
    if browser_path.exists():
        browser = load_json(browser_path)
        validate_json_schema(BROWSER_SCHEMA, browser)
        if browser["fixture"]["sha256"] != fixture["sha256"]:
            raise AssertionError("browser receipt fixture mismatch")
        if revision is None or scene_snapshot is None:
            blockers.append("browser_receipt_cannot_close_without_canonical_receipts")
        else:
            if browser["initial_revision_id"] != revision["baseline"]["revision_id"]:
                raise AssertionError("browser initial revision is not canonical baseline")
            if browser["accepted_revision_id"] != revision["accepted"]["revision_id"]:
                raise AssertionError("browser accepted revision is not canonical accepted revision")
            if browser["scene_snapshot_ids"]["initial"] != scene_snapshot["snapshot_id"]:
                raise AssertionError("browser initial scene snapshot does not match real Viewer adapter")
            validated["browser_acceptance"] = True
    else:
        blockers.append("missing_real_browser_acceptance_receipt")

    acceptance_ready = all(validated.values()) and not blockers
    result = {
        "preflight_version": "chaptera.web-acceptance-preflight.v1",
        "fixture": {
            "name": fixture["name"],
            "git_blob": fixture["upstream_git_blob"],
            "sha256": actual_sha,
            "byte_len": args.fixture.stat().st_size,
            "family": fixture["family"],
        },
        "validated": validated,
        "acceptance_ready": acceptance_ready,
        "blockers": sorted(set(blockers)),
        "claim": (
            "real_pub_browser_acceptance_ready"
            if acceptance_ready
            else "preflight_only_not_product_acceptance"
        ),
    }
    if revision_summary is not None:
        result["canonical_revision"] = {
            "baseline_revision_id": revision_summary["baseline_revision_id"],
            "accepted_revision_id": revision_summary["accepted_revision_id"],
        }
    if scene_snapshot is not None:
        result["canonical_scene"] = {
            "snapshot_id": scene_snapshot["snapshot_id"],
            "node_count": len(scene_snapshot["nodes"]),
            "page_count": len(scene_snapshot["pages"]),
        }

    rendered = json.dumps(result, indent=2, sort_keys=True)
    print(rendered)
    if args.output is not None:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered + "\n", encoding="utf-8")

    if args.mode == "strict" and not acceptance_ready:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
