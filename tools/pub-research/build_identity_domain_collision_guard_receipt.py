#!/usr/bin/env python3
from __future__ import annotations
import argparse, hashlib, json
from pathlib import Path

SCHEMA="chaptera.autonomus-identity-domain-collision-guard.v1"
CASES=[
"equal_contents_seq_and_quill_syid_do_not_share_canonical_id",
"contents_seq_role_separation",
"equal_oid_scalar_not_join_key",
"semantic_role_changes_identity",
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
      "task":"AUTONOMUS-GH-IDENTITY-DOMAIN-COLLISION-GUARD-01",
      "parent_task":"PUB-T-781",
      "git_commit":a.commit_sha.lower(),
      "identity_domains":["contents_seq_num","quill_syid","oid_physical_pair"],
      "case_count":len(CASES),
      "cases":CASES,
      "test_source_sha256":sha256(a.test_source),
      "source_free":True,
      "private_pub_bytes_used":False,
      "numeric_equality_join_claimed":False,
      "join_rule":"numeric equality alone never establishes cross-domain identity",
      "limitations":[
        "Synthetic equal-value collisions only.",
        "No real corpus collision frequency is measured.",
        "PUB-T-781 retains evidence-backed cross-domain relationship analysis."
      ]
    }
    a.output.parent.mkdir(parents=True,exist_ok=True)
    a.output.write_text(json.dumps(doc,indent=2,sort_keys=True)+"\n",encoding="utf-8")
    return 0

if __name__=="__main__":
    raise SystemExit(main())
