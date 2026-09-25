#!/usr/bin/env python3
import argparse
import json
from pathlib import Path
import sys

from validate_font_environment import (
    FONT,
    assert_delivery_invariants,
    assert_matches_scene,
    assert_source_neutral,
    validate_schema,
)


def build_environment(scene):
    render_text = next(
        (item for item in scene.get("capabilities", []) if item.get("key") == "render.text"),
        None,
    )
    if render_text is None:
        raise AssertionError("real Scene must declare render.text capability")
    if render_text.get("state") not in {"partial", "unsupported"}:
        raise AssertionError(
            "geometry-only real font environment must not claim authoritative browser text"
        )

    environment = scene.get("layout_environment")
    if not isinstance(environment, dict):
        raise AssertionError("real Scene must carry layout_environment")

    return {
        "protocol_version": "chaptera.font-environment.v1",
        "document_id": scene["document_id"],
        "revision_id": scene["revision_id"],
        "scene_snapshot_id": scene["snapshot_id"],
        "layout_environment_id": environment["environment_id"],
        "font_set_fingerprint": environment["font_set_fingerprint"],
        "preview_authority": "server_frame_geometry_only",
        "fonts": [],
        "diagnostics": [],
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("scene", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()

    scene = json.loads(args.scene.read_text(encoding="utf-8"))
    environment = build_environment(scene)
    validate_schema(FONT / "font-environment.schema.json", environment)
    assert_source_neutral(environment)
    assert_matches_scene(scene, environment)
    assert_delivery_invariants(environment)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(environment, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps({
        "protocol_version": environment["protocol_version"],
        "document_id": environment["document_id"],
        "revision_id": environment["revision_id"],
        "scene_snapshot_id": environment["scene_snapshot_id"],
        "layout_environment_id": environment["layout_environment_id"],
        "font_set_fingerprint": environment["font_set_fingerprint"],
        "preview_authority": environment["preview_authority"],
        "font_descriptor_count": len(environment["fonts"]),
        "implicit_host_fallback": False,
    }, indent=2, sort_keys=True))


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"real font environment build failed: {exc}", file=sys.stderr)
        raise
