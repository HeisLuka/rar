#!/usr/bin/env python3
import json
import os
import pathlib

from ci_efficiency_v1 import collect

ROOT=pathlib.Path(__file__).resolve().parents[1]
OUT=ROOT/"target"/"ci-efficiency-v1"
OUT.mkdir(parents=True,exist_ok=True)

token=os.environ.get("GITHUB_TOKEN")
if not token:
    raise SystemExit("GITHUB_TOKEN is required")
repo=os.environ.get("GITHUB_REPOSITORY","HeisLuka/rar")
limit=int(os.environ.get("CI_EFFICIENCY_RUN_LIMIT","30"))
receipt=collect(repo,token,limit)

if receipt["measurement_class"]!="real_public_github_actions_metadata":
    raise AssertionError("unexpected measurement class")
if receipt["bounded_run_count"]<1:
    raise AssertionError("no completed workflow runs collected")
if receipt["summary"]["summed_job_seconds"]<=0:
    raise AssertionError("no job duration measured")
if "billing_dollars" not in receipt["explicit_unknowns"]:
    raise AssertionError("billing uncertainty must stay explicit")

(OUT/"receipt.json").write_text(json.dumps(receipt,indent=2,sort_keys=True)+"\n",encoding="utf-8")
print(json.dumps({
    "receipt_version":receipt["receipt_version"],
    "run_count":receipt["bounded_run_count"],
    "summed_job_minutes":round(receipt["summary"]["summed_job_minutes"],3),
    "summed_run_elapsed_minutes":round(receipt["summary"]["summed_run_elapsed_seconds"]/60.0,3),
    "artifact_mib_first_pages":round(receipt["summary"]["artifact_bytes_first_pages"]/(1024*1024),3),
    "rerun_groups":receipt["summary"]["rerun_group_count"],
    "top_repeated_steps":receipt["candidate_repeated_work"][:10],
},indent=2,sort_keys=True))
