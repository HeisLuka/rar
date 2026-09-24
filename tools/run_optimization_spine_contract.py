#!/usr/bin/env python3
import json
import os
import pathlib

from optimization_receipt_v1 import render_bench_measurement

ROOT=pathlib.Path(__file__).resolve().parents[1]
SOURCE=ROOT/"target"/"render-bench-v1"/"receipt.json"
OUT=ROOT/"target"/"optimization-spine-v1"
OUT.mkdir(parents=True,exist_ok=True)

receipt=json.loads(SOURCE.read_text(encoding="utf-8"))
build={
    "repository":os.environ.get("GITHUB_REPOSITORY","HeisLuka/rar"),
    "sha":os.environ.get("GITHUB_SHA") or receipt.get("runtime",{}).get("github_sha") or "local-unpinned",
}
snapshot=render_bench_measurement(receipt,build)

if snapshot["producer"]["receipt_version"]!="chaptera.render-bench.v1":
    raise AssertionError("Render benchmark producer was not integrated")
if snapshot["evidence_authority"]["technology_decision_allowed"] is not False:
    raise AssertionError("synthetic public benchmark must not authorize technology choice")
if snapshot["metrics"]["gpu.draw_latency"]["state"]!="unknown":
    raise AssertionError("unmeasured GPU latency must remain unknown")
if snapshot["metrics"]["cost.usd_per_1000_edits"]["state"]!="unknown":
    raise AssertionError("unmeasured cost must remain unknown")
if not all(snapshot["correctness"].values()):
    raise AssertionError("render benchmark correctness fence failed")

(OUT/"render-bench-measurement.json").write_text(
    json.dumps(snapshot,indent=2,sort_keys=True)+"\n",
    encoding="utf-8",
)
print(json.dumps({
    "schema":snapshot["schema"],
    "producer":snapshot["producer"],
    "workload_identity_hash":snapshot["workload_identity_hash"],
    "observed_metric_count":sum(1 for m in snapshot["metrics"].values() if m["state"]=="observed"),
    "unknown_metric_count":sum(1 for m in snapshot["metrics"].values() if m["state"]=="unknown"),
    "technology_decision_allowed":snapshot["evidence_authority"]["technology_decision_allowed"],
},indent=2,sort_keys=True))
