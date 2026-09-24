#!/usr/bin/env python3
import copy
import json
import pathlib
import sys

from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

from adapt_viewer_scene_v1 import adapt_viewer_geometry
from scene_v1 import canonical_json, finalize_snapshot
from visreg_v1 import compare_scenes

SCENE_DIR = ROOT / "packages" / "protocol" / "scene" / "v1"
VIEWER_FIXTURE = SCENE_DIR / "viewer-fixtures" / "enriched-overlays.json"
REPORT_SCHEMA = ROOT / "packages" / "validation" / "visreg" / "v1" / "report.schema.json"
OUT = ROOT / "target" / "visreg-v1"

DOC_ID = "90000000-0000-4000-8000-000000000001"
REVISION_ID = "sha256:" + "9" * 64


def validate_report(report):
    schema = json.loads(REPORT_SCHEMA.read_text(encoding="utf-8"))
    Draft202012Validator.check_schema(schema)
    errors = sorted(Draft202012Validator(schema).iter_errors(report), key=lambda e: list(e.path))
    if errors:
        detail = "\n".join(f"{list(error.path)}: {error.message}" for error in errors)
        raise AssertionError("VISREG report schema validation failed\n" + detail)


def render_evidence(scene, fill):
    page_id = scene["pages"][0]["page_id"]
    node_id = scene["nodes"][0]["node_id"]
    return {
        "pages": [
            {
                "page_id": page_id,
                "artifact_sha256": fill * 64,
                "regions": [
                    {
                        "origin_node_id": node_id,
                        "bbox_px": [12, 18, 80, 30],
                        "sha256": fill * 64,
                    }
                ],
            }
        ]
    }


def write_report(name, report):
    validate_report(report)
    path = OUT / f"{name}.report.json"
    path.write_bytes(canonical_json(report) + b"\n")
    return path


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    viewer = json.loads(VIEWER_FIXTURE.read_text(encoding="utf-8"))
    baseline = adapt_viewer_geometry(viewer, DOC_ID, REVISION_ID)

    geometry = copy.deepcopy(baseline)
    geometry["nodes"][0]["bounds"]["x"] += 12700
    geometry = finalize_snapshot(geometry)

    text = copy.deepcopy(baseline)
    if not text["stories"]:
        raise AssertionError("VISREG contract fixture needs a Story")
    text["stories"][0]["text"] += "!"
    text = finalize_snapshot(text)

    geometry_report = compare_scenes(baseline, geometry)
    text_report = compare_scenes(baseline, text)
    render_report = compare_scenes(
        baseline,
        copy.deepcopy(baseline),
        baseline_render=render_evidence(baseline, "b"),
        candidate_render=render_evidence(baseline, "c"),
    )

    write_report("geometry", geometry_report)
    write_report("text-layout", text_report)
    write_report("render-only", render_report)

    if geometry_report["summary"]["stage_counts"]["layout"] < 1:
        raise AssertionError("intentional geometry regression was not classified as layout")
    if any(
        geometry_report["summary"]["stage_counts"][stage]
        for stage in ("text_layout", "render")
    ):
        raise AssertionError("geometry regression leaked into text/render stages")

    if text_report["summary"]["stage_counts"]["text_layout"] < 1:
        raise AssertionError("intentional text regression was not classified as text_layout")
    if text_report["summary"]["stage_counts"]["layout"]:
        raise AssertionError("text regression was misclassified as geometry/layout")

    if not render_report["summary"]["render_only"]:
        raise AssertionError("render-only difference was not kept separate from semantic/layout changes")

    geometry_repeat = compare_scenes(baseline, copy.deepcopy(geometry))
    if canonical_json(geometry_repeat) != canonical_json(geometry_report):
        raise AssertionError("VISREG report is not deterministic for identical inputs")

    receipt = {
        "receipt_version": "chaptera.visreg.contract-receipt.v1",
        "task": "VISREG-01",
        "real_pub_measurement": False,
        "fixture": str(VIEWER_FIXTURE.relative_to(ROOT)),
        "baseline_snapshot_id": baseline["snapshot_id"],
        "assertions": {
            "geometry_localized": True,
            "text_layout_localized": True,
            "render_only_separated": True,
            "deterministic_report": True,
            "pixel_or_render_signal_is_not_sole_oracle": True,
        },
        "reports": {
            "geometry": geometry_report["report_hash"],
            "text_layout": text_report["report_hash"],
            "render_only": render_report["report_hash"],
        },
    }
    (OUT / "receipt.json").write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(receipt, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
