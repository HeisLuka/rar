#!/usr/bin/env python3
"""Targeted Wayback discovery for Publisher training/exercise packs."""
from __future__ import annotations
import argparse, json, re, sys, time
from pathlib import Path
from urllib.parse import quote, urlparse

HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
import wayback_pub_seed as wb

SCOPES={
  "kcls-instruction": ("w3.kcls.org/instruction/", r"original:.*(?:[.]pub|[.]zip)(?:[?#].*)?$"),
  "kcls-techtutor": ("kcls.org/techtutor/", r"original:.*(?:[.]pub|[.]zip)(?:[?#].*)?$"),
  "nottingham-o2016": ("training.nottingham.ac.uk/Public/O2016/", r"original:.*(?:[.]pub|[.]zip)(?:[?#].*)?$"),
  "ezref": ("ezref.com/exercise-files/", r"original:.*(?:[.]pub|[.]zip)(?:[?#].*)?$"),
  "www-ezref": ("www.ezref.com/exercise-files/", r"original:.*(?:[.]pub|[.]zip)(?:[?#].*)?$"),
  "cherylprice": ("cherylprice.co.nz/", r"original:.*(?:2789|[Pp]ublisher).*(?:[.]pub|[.]zip)(?:[?#].*)?$"),
  "www-cherylprice": ("www.cherylprice.co.nz/", r"original:.*(?:2789|[Pp]ublisher).*(?:[.]pub|[.]zip)(?:[?#].*)?$"),
  "simonsezit": ("simonsezit.com/", r"original:.*[Pp]ublisher.*[.]zip(?:[?#].*)?$"),
  "www-simonsezit": ("www.simonsezit.com/", r"original:.*[Pp]ublisher.*[.]zip(?:[?#].*)?$"),
  "sample-courseware-s3": ("sample-courseware-usa.s3.amazonaws.com/", r"original:.*[Pp]ublisher.*(?:[.]pub|[.]zip)(?:[?#].*)?$"),
  "cheltenham": ("cheltenhamcourseware.com.au/", r"original:.*[Pp]ublisher.*(?:[.]pub|[.]zip)(?:[?#].*)?$"),
  "www-cheltenham": ("www.cheltenhamcourseware.com.au/", r"original:.*[Pp]ublisher.*(?:[.]pub|[.]zip)(?:[?#].*)?$"),
}

def query(prefix, filt, limit, timeout, retries):
    return wb.request_json([
      ("url",prefix),("matchType","prefix"),("output","json"),
      ("fl","timestamp,original,mimetype,statuscode,digest,length"),
      ("filter","statuscode:200"),("filter",filt),("collapse","digest"),
      ("limit",str(limit)),("gzip","false"),
    ],timeout,retries)

def convert(row, scope_id, prefix):
    ts=(row.get("timestamp") or "").strip()
    original=(row.get("original") or "").strip()
    if not ts or not original: return None
    digest=(row.get("digest") or "").strip()
    name=Path(urlparse(original).path).name or f"training-{digest[:24]}"
    if not name.lower().endswith((".pub",".zip")):
        name=f"training-{digest[:24]}.bin"
    return {
      "source_page":f"https://web.archive.org/web/{ts}/{quote(original,safe=':/?&=%')}",
      "direct_url":wb.replay_url(ts,original),
      "candidate_filename":name[:180],
      "quarantine":"",
      "source_class":"wayback_training_pack",
      "notes":"Targeted historical Publisher training/exercise pack; archive timestamp is provenance only",
      "wayback_timestamp":ts,
      "wayback_original_url":original,
      "wayback_digest":digest,
      "wayback_mimetype":row.get("mimetype",""),
      "wayback_length":row.get("length",""),
      "wayback_query_kind":"training_prefix",
      "wayback_query_value":prefix,
      "training_scope":scope_id,
    }

def main():
    ap=argparse.ArgumentParser()
    ap.add_argument("--scope",required=True,choices=sorted(SCOPES))
    ap.add_argument("--out",type=Path,required=True)
    ap.add_argument("--summary",type=Path)
    ap.add_argument("--limit",type=int,default=2500)
    ap.add_argument("--timeout",type=float,default=25)
    ap.add_argument("--retries",type=int,default=1)
    args=ap.parse_args()
    prefix,filt=SCOPES[args.scope]
    errors=[]
    try:
        raw=query(prefix,filt,args.limit,args.timeout,args.retries)
    except Exception as exc:
        raw=[]; errors=[f"{type(exc).__name__}: {exc}"]
    rows=[x for x in (convert(r,args.scope,prefix) for r in raw) if x]
    dedup={}
    for r in sorted(rows,key=lambda x:x["wayback_timestamp"]):
        dedup.setdefault((r["wayback_original_url"].casefold(),r["wayback_digest"]),r)
    final=sorted(dedup.values(),key=lambda r:(r["wayback_timestamp"],r["wayback_original_url"]))
    wb.write_csv(final,args.out)
    summary={
      "schema":"rar-publisher-training-wayback-v1",
      "scope":args.scope,"prefix":prefix,"filter":filt,
      "raw_rows":len(rows),"deduplicated_locator_rows":len(final),
      "unique_original_urls":len({r["wayback_original_url"] for r in final}),
      "errors":errors,
    }
    p=args.summary or args.out.with_suffix(".summary.json")
    p.parent.mkdir(parents=True,exist_ok=True)
    p.write_text(json.dumps(summary,indent=2),encoding="utf-8")
    print(json.dumps(summary,indent=2))
    return 0

if __name__=="__main__": raise SystemExit(main())
