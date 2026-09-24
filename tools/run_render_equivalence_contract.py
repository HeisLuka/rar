#!/usr/bin/env python3
import json
from pathlib import Path

from render_equivalence_v1 import compare_render_paths

scene = json.loads(Path("packages/protocol/scene/v1/fixtures/group-table.json").read_text())
page_id = scene["pages"][0]["page_id"]
env = {
    "viewport_width_px": 1200,
    "viewport_height_px": 800,
    "dpr": 1,
    "output_width_px": 1200,
    "output_height_px": 800,
    "color_profile": "srgb",
    "resource_state": "settled",
}
reference = {"backend_family": "reference", "build": "synthetic"}
candidate = {"backend_family": "candidate", "build": "synthetic"}
settled_render = {"pages": [{"page_id": page_id, "artifact_sha256": "a" * 64, "regions": []}]}
transient_render = {"pages": [{"page_id": page_id, "artifact_sha256": "b" * 64, "regions": []}]}

settled = compare_render_paths(
    baseline_scene=scene,
    candidate_scene=scene,
    baseline_render=settled_render,
    candidate_render=settled_render,
    baseline_environment=env,
    candidate_environment=env,
    reference_identity=reference,
    candidate_identity=candidate,
)
transient = compare_render_paths(
    baseline_scene=scene,
    candidate_scene=scene,
    baseline_render=settled_render,
    candidate_render=transient_render,
    baseline_environment=env,
    candidate_environment=env,
    reference_identity=reference,
    candidate_identity=candidate,
    mode="transient",
    admitted_transient_render_codes={"render.page_artifact_changed"},
    settled_baseline_render=settled_render,
    settled_candidate_render=settled_render,
)
receipt = {
    "schema": "chaptera.render-equivalence-contract-suite.v1",
    "measurement_class": "synthetic_source_neutral_fixture",
    "real_pub": False,
    "representative": False,
    "cases": {"settled": settled, "transient_convergent": transient},
}
print(json.dumps(receipt, indent=2, sort_keys=True))
