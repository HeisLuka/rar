#!/usr/bin/env python3
import json
import pathlib

from copy_ledger_v1 import synthetic_contract_fixture, validate_receipt

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / "target" / "copy-ledger-v1"
OUT.mkdir(parents=True, exist_ok=True)

receipt = synthetic_contract_fixture()
validate_receipt(receipt)

if receipt["measurement_class"] != "synthetic_contract_fixture":
    raise AssertionError("contract fixture must remain explicitly synthetic")
if receipt["evidence_authority"]["technology_decision_allowed"]:
    raise AssertionError("synthetic copy ledger must not authorize product decisions")
if receipt["summary"]["avoidable_duplicate_bytes"] <= 0:
    raise AssertionError("fixture did not exercise avoidable_duplicate classification")

(OUT / "receipt.json").write_text(
    json.dumps(receipt, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)

print(json.dumps({
    "receipt_version": receipt["receipt_version"],
    "measurement_class": receipt["measurement_class"],
    "real_pub_runtime": receipt["evidence_authority"]["real_pub_runtime"],
    "technology_decision_allowed": receipt["evidence_authority"]["technology_decision_allowed"],
    "total_materialized_bytes": receipt["summary"]["total_materialized_bytes"],
    "avoidable_duplicate_bytes": receipt["summary"]["avoidable_duplicate_bytes"],
    "top_materialization_sites": receipt["summary"]["top_materialization_sites"],
}, indent=2, sort_keys=True))
