#!/usr/bin/env python3
from __future__ import annotations
import argparse, hashlib, json
from pathlib import Path

SCHEMA="chaptera.autonomus-crossgen-family-fence.v1"
CASES=[
"mature_rejects_legacy_family",
"legacy_formatting_rejects_mature_family",
"legacy_table_rejects_mature_family",
"same_numeric_marker_does_not_bypass_family_fence",
]

def sha256(path:Path)->str:
    return hashlib.sha256(path.read_bytes()).hexdigest()

def main()->int:
    p=argparse.ArgumentParser()
    p.add_argument("--test-source",required=True,type=Path)
    p.add_argument("--commit-sha",required=True)
    p.add_argument("--output",required=True,type=Path)
    a=p.parse_args()
    if len(a.commit_sha)!=40 or any(c not in "0123456789abcdef" for c in a.commit_sha.lower()):
        raise SystemExit("invalid commit sha")
    doc={
      "schema":SCHEMA,
      "task":"AUTONOMUS-GH-CROSSGEN-FAMILY-FENCE-01",
      "parent_task":"PUB-T-808",
      "git_commit":a.commit_sha.lower(),
      "parser_surface":[
        "parse_0x2c_header",
        "parse_legacy_0x22_formatting_descriptor",
        "parse_legacy_0x22_table_catalog"
      ],
      "case_count":len(CASES),
      "cases":CASES,
      "test_source_sha256":sha256(a.test_source),
      "source_free":True,
      "private_pub_bytes_used":False,
      "marker_semantics_claimed":False,
      "dispatch_rule":"family gate must be proven before family-specific interpretation; raw marker equality alone is insufficient",
      "limitations":[
        "No real cross-generation parent/context corpus is consumed.",
        "No semantic meaning is assigned to the colliding synthetic marker.",
        "PUB-T-808 retains real evidence-table and safe-dispatch conclusions."
      ]
    }
    a.output.parent.mkdir(parents=True,exist_ok=True)
    a.output.write_text(json.dumps(doc,indent=2,sort_keys=True)+"\n",encoding="utf-8")
    return 0

if __name__=="__main__":
    raise SystemExit(main())
