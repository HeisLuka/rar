#!/usr/bin/env python3
from __future__ import annotations
import argparse, hashlib, json
from pathlib import Path

SCHEMA="chaptera.autonomus-parser-warning-contract.v1"
CASES=[
"unsupported_block_wire",
"truncated_u32",
"invalid_container_length",
"chunk_declared_range_oob",
"directory_wrong_slot_id",
"directory_wrong_supported_wire",
"reference_context_unsupported_wire",
"reference_seqnum_oob",
]

def digest(path:Path)->str:
    return hashlib.sha256(path.read_bytes()).hexdigest()

def main()->int:
    p=argparse.ArgumentParser()
    p.add_argument("--test-source",required=True,type=Path)
    p.add_argument("--commit-sha",required=True)
    p.add_argument("--output",required=True,type=Path)
    a=p.parse_args()
    if len(a.commit_sha)!=40 or any(c not in "0123456789abcdef" for c in a.commit_sha.lower()):
        raise SystemExit("invalid commit sha")
    r={
      "schema":SCHEMA,
      "task":"AUTONOMUS-GH-PARSER-WARNING-REGRESSION-01",
      "git_commit":a.commit_sha.lower(),
      "parser_surface":"vendor/producer-a/crates/pub-contents",
      "case_count":len(CASES),
      "cases":CASES,
      "test_source_sha256":digest(a.test_source),
      "source_free":True,
      "private_pub_bytes_used":False,
      "taxonomy_claim":"bounded parser diagnostic categories only",
      "limitations":[
        "No private warning logs or corpus bytes are inputs.",
        "Frequency of real warnings is not measured here.",
        "This does not assign PUB feature semantics."
      ]
    }
    a.output.parent.mkdir(parents=True,exist_ok=True)
    a.output.write_text(json.dumps(r,indent=2,sort_keys=True)+"\n",encoding="utf-8")
    return 0

if __name__=="__main__":
    raise SystemExit(main())
