#!/usr/bin/env python3
"""Safely inspect public archive containers for nested Microsoft Publisher files.

Uses the system 7z implementation as the archive parser. It never executes
archive members. Extraction is bounded by declared size, member count, depth,
wall-clock timeout and path safety checks.
"""
from __future__ import annotations

import argparse,csv,hashlib,json,os,re,subprocess,sys,tempfile,time
from pathlib import Path,PurePosixPath
from urllib.parse import urlparse
from urllib.request import Request,urlopen

sys.path.insert(0,str(Path(__file__).resolve().parent))
import harvest_pub

UA="rar-pub-container/1.0 (public format research)"
CONTAINER_EXT=(".zip",".cab",".7z",".rar",".iso",".tar",".tgz",".tar.gz",".gz")
PUB_EXT=".pub"


def safe_member(name:str)->bool:
    n=name.replace("\\","/")
    p=PurePosixPath(n)
    return bool(n) and not n.startswith("/") and ".." not in p.parts and "\x00" not in n


def ext_kind(name:str)->str:
    low=name.casefold()
    if low.endswith(PUB_EXT): return "pub"
    if low.endswith(CONTAINER_EXT): return "container"
    return ""


def fetch(url:str,path:Path,timeout:float,max_bytes:int)->dict:
    req=Request(url,headers={"User-Agent":UA,"Accept":"*/*"})
    total=0
    h=hashlib.sha256()
    with urlopen(req,timeout=timeout) as r,path.open("wb") as f:
        while True:
            chunk=r.read(1024*1024)
            if not chunk: break
            total+=len(chunk)
            if total>max_bytes: raise ValueError("container exceeds max bytes")
            h.update(chunk); f.write(chunk)
        return {"final_url":r.geturl(),"content_type":r.headers.get("Content-Type",""),"size":total,"sha256":h.hexdigest()}


def list_7z(path:Path,timeout:int):
    p=subprocess.run(["7z","l","-slt","-ba",str(path)],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,timeout=timeout,check=False)
    if p.returncode!=0: raise RuntimeError("7z list failed: "+p.stderr[-1000:])
    rows=[]; cur={}
    for line in p.stdout.splitlines()+[""]:
        if not line.strip():
            if cur.get("Path"): rows.append(cur)
            cur={}
            continue
        if " = " in line:
            k,v=line.split(" = ",1); cur[k.strip()]=v.strip()
    return rows


def extract_member(archive:Path,name:str,out:Path,timeout:int,max_bytes:int):
    with out.open("wb") as fh:
        p=subprocess.run(["7z","x","-so","-y",str(archive),name],stdout=fh,stderr=subprocess.PIPE,timeout=timeout,check=False)
    if p.returncode!=0:
        out.unlink(missing_ok=True)
        raise RuntimeError("7z extract failed: "+p.stderr.decode("utf-8",errors="replace")[-1000:])
    size=out.stat().st_size
    if size>max_bytes:
        out.unlink(missing_ok=True)
        raise ValueError("extracted member exceeds max bytes")
    return size


def inspect_archive(archive:Path,origin:dict,depth:int,args,rows:list,seen:set,td:Path):
    if depth>args.max_depth: return
    entries=list_7z(archive,args.command_timeout)
    candidates=[]
    for e in entries:
        name=e.get("Path","")
        kind=ext_kind(name)
        if not kind or not safe_member(name): continue
        if e.get("Encrypted","-")=="+": continue
        attrs=e.get("Attributes","")
        if "L" in attrs: continue
        try: size=int(e.get("Size","0") or 0)
        except ValueError: continue
        if size<0 or size>args.max_member_bytes: continue
        candidates.append((name,kind,size))
        if len(candidates)>=args.max_members: break
    total_declared=sum(x[2] for x in candidates)
    if total_declared>args.max_total_expanded:
        raise ValueError("declared candidate expansion exceeds total cap")

    for idx,(name,kind,declared) in enumerate(candidates):
        member=td/f"d{depth}_{idx}_{Path(name).name}"
        try:
            actual=extract_member(archive,name,member,args.command_timeout,args.max_member_bytes)
            data=member.read_bytes()
            sha=hashlib.sha256(data).hexdigest()
            lineage=(origin["root_sha256"],depth,name,sha)
            if lineage in seen:
                member.unlink(missing_ok=True); continue
            seen.add(lineage)
            if kind=="pub":
                cls,hints=harvest_pub.classify(data)
                rows.append({
                    **origin,
                    "row_kind":"container_member",
                    "container_depth":depth,
                    "archive_member":name,
                    "declared_size":declared,
                    "size_bytes":actual,
                    "sha256":sha,
                    "classification":cls,
                    "publisher_hints":";".join(hints),
                })
            elif depth<args.max_depth:
                child_origin={**origin,"parent_member":name,"parent_member_sha256":sha}
                inspect_archive(member,child_origin,depth+1,args,rows,seen,td)
        except Exception as exc:
            rows.append({
                **origin,"row_kind":"container_member_error","container_depth":depth,
                "archive_member":name,"error":f"{type(exc).__name__}: {exc}"
            })
        finally:
            member.unlink(missing_ok=True)


def iter_seed(path):
    with Path(path).open("r",encoding="utf-8-sig",newline="") as f:
        yield from csv.DictReader(f)


def write(rows,out:Path):
    out.parent.mkdir(parents=True,exist_ok=True)
    out.write_text(json.dumps(rows,indent=2,ensure_ascii=False),encoding="utf-8")
    keys=[]; seen=set()
    for r in rows:
        for k in r:
            if k not in seen: seen.add(k); keys.append(k)
    with out.with_suffix(".csv").open("w",encoding="utf-8",newline="") as f:
        w=csv.DictWriter(f,fieldnames=keys); w.writeheader(); w.writerows(rows)


def main():
    ap=argparse.ArgumentParser()
    ap.add_argument("--seed",required=True)
    ap.add_argument("--out",type=Path,required=True)
    ap.add_argument("--max-containers",type=int,default=100)
    ap.add_argument("--max-container-bytes",type=int,default=300*1024*1024)
    ap.add_argument("--max-member-bytes",type=int,default=100*1024*1024)
    ap.add_argument("--max-total-expanded",type=int,default=500*1024*1024)
    ap.add_argument("--max-members",type=int,default=300)
    ap.add_argument("--max-depth",type=int,default=2)
    ap.add_argument("--timeout",type=float,default=30)
    ap.add_argument("--command-timeout",type=int,default=45)
    args=ap.parse_args()

    rows=[]; seen=set()
    seeds=[r for r in iter_seed(args.seed) if ext_kind(r.get("candidate_filename",""))=="container"][:args.max_containers]
    with tempfile.TemporaryDirectory(prefix="pub-container-") as tmp:
        td=Path(tmp)
        for i,row in enumerate(seeds):
            url=(row.get("direct_url") or "").strip()
            if not url: continue
            path=td/f"root_{i}{Path(urlparse(url).path).suffix or '.bin'}"
            try:
                meta=fetch(url,path,args.timeout,args.max_container_bytes)
                origin={
                    "source_page":row.get("source_page",""),
                    "container_url":url,
                    "container_final_url":meta["final_url"],
                    "container_filename":row.get("candidate_filename",""),
                    "root_sha256":meta["sha256"],
                    "root_size_bytes":meta["size"],
                    "source_class":row.get("source_class","public_container"),
                }
                inspect_archive(path,origin,0,args,rows,seen,td)
            except Exception as exc:
                rows.append({
                    "row_kind":"container_error","container_url":url,
                    "container_filename":row.get("candidate_filename",""),
                    "error":f"{type(exc).__name__}: {exc}"
                })
            finally:
                path.unlink(missing_ok=True)
    write(rows,args.out)
    summary={
        "schema":"rar-pub-container-v1",
        "containers_attempted":len(seeds),
        "member_rows":sum(r.get("row_kind")=="container_member" for r in rows),
        "publisher_cfb_members":sum(r.get("classification")=="cfb_publisher_hint" for r in rows),
        "errors":sum(str(r.get("row_kind","")).endswith("error") for r in rows),
        "unique_member_sha256":len({r.get("sha256") for r in rows if r.get("sha256")}),
    }
    args.out.with_name(args.out.stem+".summary.json").write_text(json.dumps(summary,indent=2),encoding="utf-8")
    print(json.dumps(summary,indent=2))
    return 0

if __name__=="__main__": raise SystemExit(main())
