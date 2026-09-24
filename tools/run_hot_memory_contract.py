#!/usr/bin/env python3
import json
import pathlib

from hot_memory_receipt_v1 import synthetic_contract_fixture, validate_receipt

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / "target" / "hot-memory-v1"
OUT.mkdir(parents=True, exist_ok=True)

receipt = synthetic_contract_fixture()
validate_receipt(receipt)

if receipt["measurement_class"] != "synthetic_contract_fixture":
    raise AssertionError("fixture lost synthetic fence")
if receipt["evidence_authority"]["capacity_decision_allowed"]:
    raise AssertionError("synthetic memory fixture cannot authorize capacity")

(OUT / "receipt.json").write_text(
    json.dumps(receipt, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)

print(json.dumps({
    "receipt_version": receipt["receipt_version"],
    "measurement_class": receipt["measurement_class"],
    "capacity_decision_allowed": receipt["evidence_authority"]["capacity_decision_allowed"],
    "modes": [
        {
            "mode": m["mode"],
            "hot_steady_rss_bytes": m["thermal"]["hot_steady_rss"]["bytes"],
            "eviction_order": [x["drop_layer"] for x in m["eviction"]],
            "activation_scenarios": len(m["activation"]),
        }
        for m in receipt["modes"]
    ],
}, indent=2, sort_keys=True))
