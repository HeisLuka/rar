#!/usr/bin/env python3
from __future__ import annotations
import argparse, json, pathlib, re, sys

SCHEMA_VERSION="chaptera.pub-research-receipt.v1"
SHA256_RE=re.compile(r"^[0-9a-f]{64}$")
SAFE_ID_RE=re.compile(r"^[A-Za-z0-9._:-]{2,128}$")
ABS_PATH_RE=re.compile(r"(?i)(?:[A-Z]:[\\/]|(?:/home|/Users)/[^/\s]+/)")
SECRET_RE=re.compile(r"(?:ghp_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,}|sk-[A-Za-z0-9_-]{20,}|AKIA[0-9A-Z]{16})")

def fail(msg:str)->"NoReturn":
    raise SystemExit("research receipt validation failed: "+msg)

def load(path:pathlib.Path)->dict:
    try: doc=json.loads(path.read_text(encoding="utf-8-sig"))
    except Exception as exc: fail(f"cannot parse {path}: {exc}")
    if not isinstance(doc,dict): fail("root must be object")
    return doc

def validate(doc:dict)->dict:
    required={"receipt_version","task_id","run_id","scope","inputs","outputs","counts","canonical_impact","limitations","privacy"}
    if set(doc)!=required: fail(f"fields must equal {sorted(required)}")
    if doc["receipt_version"]!=SCHEMA_VERSION: fail("wrong receipt_version")
    for field in ("task_id","run_id"):
        if not isinstance(doc[field],str) or not SAFE_ID_RE.fullmatch(doc[field]): fail(f"unsafe {field}")
    scope=doc["scope"]
    if not isinstance(scope,str) or not (3<=len(scope)<=1000): fail("scope must be bounded string")
    if ABS_PATH_RE.search(scope) or SECRET_RE.search(scope): fail("scope contains private path or secret-shaped literal")
    for field in ("inputs","outputs"):
        vals=doc[field]
        if not isinstance(vals,list): fail(f"{field} must be array")
        for item in vals:
            if not isinstance(item,dict) or set(item)!={"kind","sha256"}: fail(f"{field} item shape")
            if not isinstance(item["kind"],str) or not re.fullmatch(r"[a-z0-9._-]{1,64}",item["kind"]): fail(f"{field} kind")
            if not isinstance(item["sha256"],str) or not SHA256_RE.fullmatch(item["sha256"]): fail(f"{field} sha256")
    counts=doc["counts"]
    if not isinstance(counts,dict) or set(counts)!={"observations","supporting","counterexamples"}: fail("counts shape")
    for k,v in counts.items():
        if not isinstance(v,int) or v<0: fail(f"counts.{k}")
    if counts["supporting"]+counts["counterexamples"]>counts["observations"]: fail("supporting+counterexamples exceeds observations")
    if doc["canonical_impact"] not in {"none","reconfirmation","model-narrowing","candidate-change"}: fail("canonical_impact")
    limitations=doc["limitations"]
    if not isinstance(limitations,list) or not limitations: fail("limitations required")
    for value in limitations:
        if not isinstance(value,str) or not value or len(value)>500: fail("invalid limitation")
        if ABS_PATH_RE.search(value) or SECRET_RE.search(value): fail("limitation contains private path or secret-shaped literal")
    privacy=doc["privacy"]
    if not isinstance(privacy,dict) or set(privacy)!={"pub_bytes_in_receipt","document_text_in_receipt","local_path_in_receipt","credentials_in_receipt"}: fail("privacy shape")
    if any(privacy.values()): fail("all privacy flags must be false")
    return {"task_id":doc["task_id"],"run_id":doc["run_id"],"canonical_impact":doc["canonical_impact"]}

def main()->int:
    ap=argparse.ArgumentParser(); ap.add_argument("receipt",type=pathlib.Path); args=ap.parse_args()
    print(json.dumps({"receipt_version":SCHEMA_VERSION,**validate(load(args.receipt))},indent=2)); return 0

if __name__=="__main__": sys.exit(main())
