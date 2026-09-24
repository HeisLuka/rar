#!/usr/bin/env python3
import json
import pathlib

from open_first_useful_page_v1 import synthetic_contract_fixture, validate_receipt

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / "target" / "open-first-useful-page-v1"
OUT.mkdir(parents=True, exist_ok=True)

receipt = synthetic_contract_fixture()
validate_receipt(receipt)

if receipt["measurement_class"] != "synthetic_contract_fixture":
    raise AssertionError("fixture lost synthetic fence")
if receipt["evidence_authority"]["architecture_decision_allowed"]:
    raise AssertionError("synthetic open receipt cannot authorize architecture")

(OUT / "receipt.json").write_text(
    json.dumps(receipt, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)

print(json.dumps({
    "receipt_version": receipt["receipt_version"],
    "measurement_class": receipt["measurement_class"],
    "architecture_decision_allowed": receipt["evidence_authority"]["architecture_decision_allowed"],
    "summary": receipt["summary"],
    "synthetic_global_pre_first": sorted({
        phase
        for run in receipt["runs"]
        for phase in run["document_global_before_first_useful_page"]
    }),
}, indent=2, sort_keys=True))
