#!/usr/bin/env python3
import copy
import json
import pathlib
import sys

from scene_v1 import finalize_snapshot
from visreg_v1 import compare_scenes, RENDER_MANIFEST_VERSION

ROOT = pathlib.Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "packages" / "protocol" / "scene" / "v1" / "fixtures"
TARGET = ROOT / "target" / "visreg"


def load(name):
    return json.loads((FIXTURES / name).read_text(encoding="utf-8"))


def assert_true(value, message):
    if not value:
        raise AssertionError(message)


def render_manifest(snapshot, suffix):
    return {
        "manifest_version": RENDER_MANIFEST_VERSION,
        "snapshot_id": snapshot["snapshot_id"],
        "pages": [
            {
                "page_id": page["page_id"],
                "artifact_sha256": (suffix * 64)[:64],
            }
            for page in snapshot["pages"]
        ],
    }


def main():
    TARGET.mkdir(parents=True, exist_ok=True)
    base = load("simple-text.json")

    equal_a = compare_scenes(base, base)
    equal_b = compare_scenes(base, base)
    assert_true(equal_a == equal_b, "identical reruns must produce identical normalized reports")
    assert_true(equal_a["summary"]["classification"] == "equal", "identical scene must classify equal")

    geometry = copy.deepcopy(base)
    geometry["nodes"][0]["bounds"]["x"] += 12700
    geometry["revision_id"] = "sha256:" + "5" * 64
    geometry = finalize_snapshot(geometry)
    geometry_report = compare_scenes(base, geometry)
    assert_true(geometry_report["summary"]["stage_counts"]["geometry"] == 1, "geometry regression not localized")
    assert_true(geometry_report["summary"]["stage_counts"]["text_layout"] == 0, "geometry regression leaked into text stage")
    assert_true(
        any(
            item["kind"] == "node_geometry_changed"
            and item["node_id"] == base["nodes"][0]["node_id"]
            and item["page_id"] == base["pages"][0]["page_id"]
            for item in geometry_report["differences"]
        ),
        "geometry regression lost node/page origin",
    )

    text = copy.deepcopy(base)
    text["stories"][0]["text"] = "Hello, changed Publisher"
    text["revision_id"] = "sha256:" + "6" * 64
    text = finalize_snapshot(text)
    text_report = compare_scenes(base, text)
    assert_true(text_report["summary"]["stage_counts"]["text_layout"] == 1, "text regression not localized")
    assert_true(
        any(
            item["kind"] == "story_changed"
            and item["story_id"] == base["stories"][0]["story_id"]
            and item["page_id"] == base["pages"][0]["page_id"]
            for item in text_report["differences"]
        ),
        "text regression lost Story/page origin",
    )

    render_before = render_manifest(base, "a")
    render_after = render_manifest(base, "b")
    render_report = compare_scenes(base, base, render_before, render_after)
    assert_true(render_report["summary"]["classification"] == "render_only", "artifact-only change must classify render_only")
    assert_true(render_report["summary"]["semantic_difference_count"] == 0, "render-only arm invented semantic difference")
    assert_true(render_report["summary"]["stage_counts"]["render"] == 1, "render-only arm not localized")

    mixed_report = compare_scenes(base, geometry, render_before, render_manifest(geometry, "c"))
    assert_true(mixed_report["summary"]["classification"] == "mixed", "semantic + artifact change must classify mixed")

    receipt = {
        "receipt_kind": "visreg_01_acceptance",
        "report_version": "chaptera.visreg.v1",
        "deterministic_rerun": equal_a == equal_b,
        "arms": {
            "equal": equal_a["summary"],
            "geometry": geometry_report["summary"],
            "text_layout": text_report["summary"],
            "render_only": render_report["summary"],
            "mixed": mixed_report["summary"],
        },
        "origin_assertions": {
            "geometry_node_id": base["nodes"][0]["node_id"],
            "text_story_id": base["stories"][0]["story_id"],
            "page_id": base["pages"][0]["page_id"],
        },
    }

    for name, report in (
        ("equal", equal_a),
        ("geometry", geometry_report),
        ("text-layout", text_report),
        ("render-only", render_report),
        ("mixed", mixed_report),
    ):
        (TARGET / f"{name}.report.json").write_text(
            json.dumps(report, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )

    (TARGET / "receipt.json").write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(receipt, indent=2, sort_keys=True))


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"VISREG-01 validation failed: {exc}", file=sys.stderr)
        raise
