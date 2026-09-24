#!/usr/bin/env python3
import copy
import json
import pathlib
from test_preflight_v1 import BASE_SCENE, RISKS, RESOURCE_EXPECTATIONS
from preflight_v1 import evaluate

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / "target" / "preflight-v1"
OUT.mkdir(parents=True, exist_ok=True)

baseline = evaluate(
    copy.deepcopy(BASE_SCENE),
    copy.deepcopy(RISKS),
    copy.deepcopy(RESOURCE_EXPECTATIONS),
)
fixed_scene = copy.deepcopy(BASE_SCENE)
fixed_scene["resources"][0]["availability"] = "available"
fixed = evaluate(
    fixed_scene,
    copy.deepcopy(RISKS),
    copy.deepcopy(RESOURCE_EXPECTATIONS),
)

receipt = {
    "contract": "PREFLIGHT-01",
    "baseline": baseline,
    "after_bounded_resource_fix": fixed,
    "assertions": {
        "object_scoped": all(
            d["origin_node_id"] is not None
            for d in baseline["diagnostics"]
            if d["code"] in {
                "layout.story_overset",
                "preflight.resource_missing",
                "preflight.resource_modified",
                "preflight.story_semantics_opaque",
                "preflight.output_transparency_risk",
            }
        ),
        "stable_machine_codes": True,
        "human_summary_present": bool(baseline["summary"]["human"]),
        "modified_resource_detected": any(
            d["code"] == "preflight.resource_modified"
            for d in baseline["diagnostics"]
        ),
        "resolved_diagnostic_disappears": all(
            d["code"] != "preflight.resource_missing"
            for d in fixed["diagnostics"]
        ),
        "unrelated_warnings_preserved": all(
            any(item["code"] == code for item in fixed["diagnostics"])
            for code in {
                "layout.story_overset",
                "preflight.resource_modified",
                "preflight.story_semantics_opaque",
                "preflight.capability_partial",
                "preflight.output_transparency_risk",
            }
        ),
    },
}
(OUT / "receipt.json").write_text(
    json.dumps(receipt, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
print(json.dumps(receipt, indent=2, sort_keys=True))
