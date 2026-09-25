#!/usr/bin/env python3
import copy
import json
from pathlib import Path
import sys

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
FONT = ROOT / "packages" / "protocol" / "font-environment" / "v1"
SCENE = ROOT / "packages" / "protocol" / "scene" / "v1"


def validate_schema(schema_path, value):
    schema = json.loads(schema_path.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(Draft202012Validator(schema).iter_errors(value), key=lambda e: list(e.path))
    if errors:
        raise AssertionError("\n".join(f"{list(e.path)}: {e.message}" for e in errors))


def assert_source_neutral(value, at="$"):
    forbidden = {
        "source_path", "filesystem_path", "font_path", "local_path",
        "raw_font_bytes", "font_bytes", "system_font_name", "host_font_path"
    }
    if isinstance(value, dict):
        for key, child in value.items():
            if key in forbidden:
                raise AssertionError(f"forbidden font/source field {key!r} at {at}")
            assert_source_neutral(child, f"{at}.{key}")
    elif isinstance(value, list):
        for i, child in enumerate(value):
            assert_source_neutral(child, f"{at}[{i}]")


def assert_matches_scene(scene, env):
    pairs = (
        ("document_id", scene["document_id"], env["document_id"]),
        ("revision_id", scene["revision_id"], env["revision_id"]),
        ("scene_snapshot_id", scene["snapshot_id"], env["scene_snapshot_id"]),
        (
            "layout_environment_id",
            scene["layout_environment"]["environment_id"],
            env["layout_environment_id"],
        ),
        (
            "font_set_fingerprint",
            scene["layout_environment"]["font_set_fingerprint"],
            env["font_set_fingerprint"],
        ),
    )
    for label, expected, actual in pairs:
        if expected != actual:
            raise AssertionError(f"{label} mismatch")

    if env["preview_authority"] == "server_positioned_glyphs":
        render_text = next(
            (c for c in scene["capabilities"] if c["key"] == "render.text"),
            None,
        )
        if render_text is None or render_text["state"] != "supported":
            raise AssertionError(
                "server_positioned_glyphs requires supported authoritative text projection"
            )


def assert_delivery_invariants(env):
    seen = set()
    for font in env["fonts"]:
        fingerprint = font["font_fingerprint"]
        if fingerprint in seen:
            raise AssertionError("duplicate font fingerprint")
        seen.add(fingerprint)
        delivery = font["delivery"]
        resource_id = font["resource_id"]
        handle = font["fetch_handle"]
        fallback = font["fallback"]

        if delivery in {"deliver_exact", "deliver_subset"}:
            if font["content_hash"] is None or resource_id is None or handle is None:
                raise AssertionError(f"{delivery} requires explicit bytes/resource handle")
            if fallback is not None:
                raise AssertionError(f"{delivery} cannot also declare fallback")
        elif delivery == "substitute_explicit":
            if resource_id is not None or handle is not None:
                raise AssertionError("source font cannot leak a resource when substituting")
            if fallback is None:
                raise AssertionError("explicit substitution requires explicit fallback")
        elif delivery in {"server_render_only", "blocked"}:
            if resource_id is not None or handle is not None or fallback is not None:
                raise AssertionError(f"{delivery} must not expose browser font bytes")
        else:
            raise AssertionError("unknown delivery mode")


def main():
    schema = FONT / "font-environment.schema.json"
    fixture = json.loads((FONT / "fixtures" / "server-only.json").read_text(encoding="utf-8"))
    scene = json.loads((SCENE / "fixtures" / "simple-text.json").read_text(encoding="utf-8"))

    validate_schema(schema, fixture)
    assert_source_neutral(fixture)
    assert_matches_scene(scene, fixture)
    assert_delivery_invariants(fixture)

    mismatch = copy.deepcopy(fixture)
    mismatch["font_set_fingerprint"] = "sha256:" + "9" * 64
    try:
        assert_matches_scene(scene, mismatch)
    except AssertionError:
        pass
    else:
        raise AssertionError("font-set mismatch was accepted")

    false_authority = copy.deepcopy(fixture)
    false_authority["preview_authority"] = "server_positioned_glyphs"
    try:
        assert_matches_scene(scene, false_authority)
    except AssertionError:
        pass
    else:
        raise AssertionError("non-authoritative Scene text was promoted to authoritative")

    deliver = copy.deepcopy(fixture)
    deliver["fonts"][0].update({
        "delivery": "deliver_exact",
        "reason_code": "delivery.allowed",
        "resource_id": "71111111-1111-4111-8111-111111111111",
        "fetch_handle": "font_resource_exact",
    })
    deliver["diagnostics"] = []
    validate_schema(schema, deliver)
    assert_delivery_invariants(deliver)

    substitute = copy.deepcopy(fixture)
    substitute["fonts"][0].update({
        "delivery": "substitute_explicit",
        "reason_code": "source.missing_explicit_fallback",
        "resource_id": None,
        "fetch_handle": None,
        "fallback": {
            "font_fingerprint": "sha256:" + "7" * 64,
            "content_hash": "8" * 64,
            "face_index": 0,
            "family": "Fallback Sans",
            "style": "Regular",
            "resource_id": "81111111-1111-4111-8111-111111111111",
            "fetch_handle": "font_resource_fallback"
        }
    })
    validate_schema(schema, substitute)
    assert_delivery_invariants(substitute)

    print(json.dumps({
        "protocol": fixture["protocol_version"],
        "scene_snapshot_id": fixture["scene_snapshot_id"],
        "layout_environment_id": fixture["layout_environment_id"],
        "font_set_fingerprint": fixture["font_set_fingerprint"],
        "source_neutral": True,
        "implicit_host_fallback": False,
        "scene_identity_fence": "checked"
    }, indent=2, sort_keys=True))


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"font environment validation failed: {exc}", file=sys.stderr)
        raise
