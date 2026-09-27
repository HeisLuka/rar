#!/usr/bin/env python3
from __future__ import annotations
import argparse, hashlib, json
from pathlib import Path

SCHEMA="chaptera.autonomus-version-header-fingerprint.v1"
CASES=[
"exact_family_magic",
"short_family_marker_fails_closed",
"revision_source_provenance",
"same_revision_cross_family_collision",
"structural_not_marketing_version_guard",
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
      "task":"AUTONOMUS-GH-VERSION-HEADER-FINGERPRINT-01",
      "parent_task":"PUB-T-806",
      "git_commit":a.commit_sha.lower(),
      "parser_surface":"vendor/producer-a/crates/pub-contents::{detect_family,parse_preamble}",
      "case_count":len(CASES),
      "cases":CASES,
      "test_source_sha256":sha256(a.test_source),
      "source_free":True,
      "private_pub_bytes_used":False,
      "marketing_version_mapping_claimed":False,
      "classifier_scope":"Contents family marker + serialization_revision structural coordinates only",
      "limitations":[
        "No producer-generation mapping is inferred.",
        "No local corpus/provenance evidence is consumed.",
        "PUB-T-806 still owns real-world collision and lineage analysis."
      ]
    }
    a.output.parent.mkdir(parents=True,exist_ok=True)
    a.output.write_text(json.dumps(doc,indent=2,sort_keys=True)+"\n",encoding="utf-8")
    return 0

if __name__=="__main__":
    raise SystemExit(main())
