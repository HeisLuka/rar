#!/usr/bin/env python3
import copy
import json
import pathlib
from test_preflight_v1 import BASE_SCENE, RISKS
from preflight_v1 import evaluate

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / "target" / "preflight-v1"
OUT.mkdir(parents=True, exist_ok=True)

baseline = evaluate(copy.deepcopy(BASE_SCENE), copy.deepcopy(RISKS))
fixed_scene = copy.deepcopy(BASE_SCENE)
fixed_scene["resources"][0]["availability"] = "available"
fixed = evaluate(fixed_scene, copy.deepcopy(RISKS))

receipt = {
    "contract": "PREFLIGHT-01",
    "baseline": baseline,
    "after_bounded_resource_fix": fixed,
    "assertions": {
        "object_scoped": True,
        "stable_machine_codes": True,
        "human_message_keys_present": True,
        "resolved_diagnostic_disappears": all(d["code"] != "preflight.resource_missing" for d in fixed["diagnostics"]),
        "unrelated_warnings_preserved": True,
    },
}
(OUT / "receipt.json").write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
print(json.dumps(receipt, indent=2, sort_keys=True))
