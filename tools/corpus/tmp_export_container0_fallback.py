#!/usr/bin/env python3
from __future__ import annotations
import json, os, sys, tempfile, time
from pathlib import Path
from urllib.parse import quote, urlparse
from urllib.request import Request, urlopen

HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
import structural_novelty as sn
import pub_container_extract as ce
import tmp_export_rar_union as base

ARTIFACT_ID=10789500808
TARGET_INDEX=0
UA="rar-m1-container-fallback/1.0"

def ia_candidates(original:str)->list[str]:
    p=urlparse(original)
    parts=[x for x in p.path.split("/") if x]
    out=[original]
    if len(parts)>=3 and parts[0]=="download":
        ident=parts[1]
        filename="/".join(parts[2:])
        meta_url=f"https://archive.org/metadata/{quote(ident)}"
        req=Request(meta_url,headers={"User-Agent":UA,"Accept":"application/json"})
        try:
            with urlopen(req,timeout=45) as r:
                meta=json.load(r)
            directory=str(meta.get("dir") or "").strip()
            hosts=[]
            for k in ("d1","d2","server"):
                h=str(meta.get(k) or "").strip()
                if h and h not in hosts: hosts.append(h)
            for h in hosts:
                if directory:
                    out.append(f"https://{h}{directory}/{quote(filename,safe='/')}")
                else:
                    out.append(f"https://{h}/download/{quote(ident)}/{quote(filename,safe='/')}")
        except Exception as exc:
            print("metadata fallback unavailable:",type(exc).__name__,exc,file=sys.stderr)
    # dedupe
    seen=set(); result=[]
    for u in out:
        if u not in seen:
            seen.add(u); result.append(u)
    return result

def main()->int:
    out=Path("out/container-0-fallback")
    out.mkdir(parents=True,exist_ok=True)
    token=os.environ.get("GITHUB_TOKEN","")
    records=[]; failures=[]
    with tempfile.TemporaryDirectory(prefix="rar-c0-") as td_raw:
        td=Path(td_raw)
        art=td/"source.zip"
        sn.download_artifact("HeisLuka/rar",ARTIFACT_ID,token,art)
        src=td/"src"; src.mkdir()
        sn.safe_extract_zip(art,src)
        rows=[r for r in sn.complete_cfb_rows(src) if r.get("row_kind")=="container_member"]
        urls=sorted({str(r.get("container_url","")).strip() for r in rows if r.get("container_url")})
        if len(urls)!=3: raise RuntimeError(f"expected 3 container URLs, got {len(urls)}")
        original=urls[TARGET_INDEX]
        selected=[r for r in rows if str(r.get("container_url","")).strip()==original]
        expected_root={str(r.get("root_sha256","")).lower() for r in selected if r.get("root_sha256")}
        if len(expected_root)!=1: raise RuntimeError(f"bad expected roots: {expected_root}")
        archive=td/"container.iso"
        meta=None; used_url=None; errs=[]
        for attempt in range(3):
            for u in ia_candidates(original):
                try:
                    print("FETCH",u)
                    meta=ce.fetch(u,archive,240,700*1024*1024)
                    if meta["sha256"].lower() not in expected_root:
                        raise RuntimeError(f"root SHA mismatch expected={expected_root} actual={meta['sha256']}")
                    used_url=u
                    break
                except Exception as exc:
                    errs.append(f"{u}:{type(exc).__name__}:{exc}")
                    archive.unlink(missing_ok=True)
            if meta is not None: break
            time.sleep(10)
        if meta is None:
            raise RuntimeError("all container fetch candidates failed: "+" | ".join(errs[-12:]))
        seen=set()
        for r in selected:
            expected=str(r["sha256"]).lower()
            if expected in seen: continue
            seen.add(expected)
            member=str(r.get("archive_member",""))
            tmp=td/f"{expected}.pub"
            try:
                ce.extract_member(archive,member,tmp,180,100*1024*1024)
                data=tmp.read_bytes()
                rec=base.emit(out,expected,data,[Path(member).name],"container_first_wave",{
                    "container_url":original,
                    "fetch_url":used_url,
                    "container_sha256":meta["sha256"],
                    "archive_member":member,
                })
                records.append(rec)
                print("OK",expected,member,rec["size_bytes"])
            except Exception as exc:
                failures.append({"sha256":expected,"archive_member":member,"error":f"{type(exc).__name__}:{exc}"})
                print("FAIL",expected,member,exc,file=sys.stderr)
            finally:
                tmp.unlink(missing_ok=True)
    records.sort(key=lambda x:x["sha256"])
    failures.sort(key=lambda x:x["sha256"])
    (out/"index.json").write_text(json.dumps(records,indent=2,ensure_ascii=False),encoding="utf-8")
    summary={"mode":"container-fallback","container_index":0,"selected_unique_sha":len(records)+len(failures),"ok":len(records),"failed":len(failures),"failures":failures}
    (out/"summary.json").write_text(json.dumps(summary,indent=2,ensure_ascii=False),encoding="utf-8")
    print(json.dumps(summary,indent=2))
    return 0 if not failures else 2

if __name__=="__main__": raise SystemExit(main())
