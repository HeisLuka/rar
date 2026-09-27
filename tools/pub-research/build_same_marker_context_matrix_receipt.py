#!/usr/bin/env python3
from __future__ import annotations
import argparse, hashlib, json
from pathlib import Path

SCHEMA="chaptera.autonomus-same-marker-context-matrix.v1"
CASES=[
"generic_same_id_is_physical_only",
"raw_type_requires_reference_plus_id02_wire18",
"chunk_offset_requires_reference_plus_id04_wireb8",
"parent_seq_requires_reference_plus_id05_wire68",
"right_wire_wrong_id_stays_unpromoted",
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
      "task":"AUTONOMUS-GH-SAME-MARKER-CONTEXT-MATRIX-01",
      "parent_task":"PUB-T-774",
      "git_commit":a.commit_sha.lower(),
      "parser_surface":"pub-contents generic block + chunk-reference context parser",
      "dispatch_tuple":["family","parent_context","field_id","wire_type"],
      "case_count":len(CASES),
      "cases":CASES,
      "test_source_sha256":sha256(a.test_source),
      "source_free":True,
      "private_pub_bytes_used":False,
      "numeric_id_semantics_claimed":False,
      "limitations":[
        "Synthetic mature chunk-reference context only.",
        "No real cross-parent or cross-generation corpus evidence is consumed.",
        "PUB-T-774 retains real grammar/semantic conclusions."
      ]
    }
    a.output.parent.mkdir(parents=True,exist_ok=True)
    a.output.write_text(json.dumps(doc,indent=2,sort_keys=True)+"\n",encoding="utf-8")
    return 0

if __name__=="__main__":
    raise SystemExit(main())
