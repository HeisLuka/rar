#!/usr/bin/env python3
"""Discover public Microsoft Publisher payloads in Internet Archive items."""
from __future__ import annotations

import argparse, csv, json, re, time
from pathlib import Path
from urllib.parse import quote, urlencode
from urllib.request import Request, urlopen

SEARCH="https://archive.org/advancedsearch.php"
META="https://archive.org/metadata/"
UA="rar-pub-internet-archive/1.0 (public format research)"
PUBLIKE=re.compile(r"[.]pub$",re.I)
ZIPLIKE=re.compile(r"[.]zip$",re.I)
CONTAINER=re.compile(r"(?:[.]iso|[.]cab|[.]7z|[.]rar|[.]tar(?:[.]gz)?|[.]tgz)$",re.I)
IA_DERIVATIVE_ZIP=re.compile(r"(?:_jp2|_thumb|_text|_hocr|_djvu|_scandata|_page_numbers|_files)[^/]*[.]zip$",re.I)
IA_GENERATED_CONTAINER=re.compile(r"(?:_jp2|_thumb|_text|_hocr|_djvu|_scandata|_page_numbers|_files)[^/]*(?:[.]tar(?:[.]gz)?|[.]tgz|[.]7z|[.]rar|[.]iso|[.]cab)$",re.I)

DEFAULT_QUERIES=[
    'title:"Microsoft Publisher" OR subject:"Microsoft Publisher" OR description:"Microsoft Publisher"',
    'title:"Publisher 2000" OR title:"Publisher 2002" OR title:"Publisher 2003"',
    'description:"Publisher template" AND mediatype:(software OR data)',
]


def get_json(url: str, timeout: float):
    req=Request(url,headers={"User-Agent":UA,"Accept":"application/json"})
    with urlopen(req,timeout=timeout) as r:
        raw=r.read(64*1024*1024+1)
    if len(raw)>64*1024*1024: raise ValueError("response too large")
    return json.loads(raw.decode("utf-8"))


def search_items(query: str, rows: int, pages: int, timeout: float):
    out=[]
    for page in range(1,pages+1):
        params=[
            ("q",query),("fl[]","identifier"),("fl[]","title"),("fl[]","date"),
            ("fl[]","mediatype"),("rows",str(rows)),("page",str(page)),("output","json")
        ]
        data=get_json(SEARCH+"?"+urlencode(params),timeout)
        docs=data.get("response",{}).get("docs",[])
        if not docs: break
        out.extend(docs)
        if len(docs)<rows: break
    return out


def download_url(identifier: str, name: str) -> str:
    return f"https://archive.org/download/{quote(identifier,safe='')}/{quote(name,safe='/')}"


def seed_row(identifier: str, item: dict, file: dict) -> dict[str,str]:
    name=str(file.get("name") or "")
    return {
        "source_page":f"https://archive.org/details/{quote(identifier,safe='')}",
        "direct_url":download_url(identifier,name),
        "candidate_filename":Path(name).name,
        "quarantine":"",
        "source_class":"internet_archive_item",
        "notes":"Internet Archive public item member; item date is provenance, not Publisher writer-version proof",
        "ia_identifier":identifier,
        "ia_member_path":name,
        "ia_item_title":str(item.get("title") or ""),
        "ia_item_date":str(item.get("date") or ""),
        "ia_mediatype":str(item.get("mediatype") or ""),
        "ia_member_size":str(file.get("size") or ""),
        "ia_member_md5":str(file.get("md5") or ""),
        "ia_member_sha1":str(file.get("sha1") or ""),
        "ia_member_format":str(file.get("format") or ""),
        "ia_member_source":str(file.get("source") or ""),
        "ia_member_mtime":str(file.get("mtime") or ""),
    }


def is_pub_or_zip_seed(file: dict) -> bool:
    name=str(file.get("name") or "")
    if PUBLIKE.search(name):
        return True
    if not ZIPLIKE.search(name):
        return False
    if IA_DERIVATIVE_ZIP.search(name):
        return False
    source=str(file.get("source") or "").strip().lower()
    # IA generated/derivative ZIPs are not Publisher payload containers.
    # Missing source is tolerated for older metadata, but an explicit
    # non-original source fails closed.
    if source and source != "original":
        return False
    return True


def is_nonzip_container_seed(item: dict, file: dict) -> bool:
    name=str(file.get("name") or "")
    if not CONTAINER.search(name):
        return False
    if IA_GENERATED_CONTAINER.search(name):
        return False
    source=str(file.get("source") or "").strip().lower()
    if source and source != "original":
        return False
    title=str(item.get("title") or "").casefold()
    if "publisher" not in title and "office" not in title:
        return False
    return True


def write_csv(rows,path):
    path.parent.mkdir(parents=True,exist_ok=True)
    keys=[]; seen=set()
    for r in rows:
        for k in r:
            if k not in seen: seen.add(k); keys.append(k)
    with path.open("w",encoding="utf-8",newline="") as f:
        w=csv.DictWriter(f,fieldnames=keys); w.writeheader(); w.writerows(rows)


def main():
    ap=argparse.ArgumentParser()
    ap.add_argument("--query",action="append",default=[])
    ap.add_argument("--rows",type=int,default=50)
    ap.add_argument("--pages",type=int,default=4)
    ap.add_argument("--max-items",type=int,default=200)
    ap.add_argument("--delay",type=float,default=0.25)
    ap.add_argument("--timeout",type=float,default=45)
    ap.add_argument("--out",type=Path,required=True)
    ap.add_argument("--containers",type=Path)
    ap.add_argument("--summary",type=Path)
    args=ap.parse_args()

    queries=args.query or DEFAULT_QUERIES
    items={}
    for q in queries:
        for d in search_items(q,args.rows,args.pages,args.timeout):
            ident=str(d.get("identifier") or "").strip()
            if ident: items.setdefault(ident,d)
    chosen=list(items.items())[:args.max_items]

    seeds=[]; containers=[]; errors=[]
    for idx,(ident,item) in enumerate(chosen,1):
        try:
            meta=get_json(META+quote(ident,safe=""),args.timeout)
            for f in meta.get("files",[]) or []:
                name=str(f.get("name") or "")
                if is_pub_or_zip_seed(f):
                    seeds.append(seed_row(ident,item,f))
                elif is_nonzip_container_seed(item,f):
                    containers.append(seed_row(ident,item,f))
        except Exception as exc:
            errors.append({"identifier":ident,"error":f"{type(exc).__name__}: {exc}"})
        if idx<len(chosen): time.sleep(args.delay)

    # provider identity + member path are stable; remove duplicated search hits.
    def dedup(rows):
        out={}
        for r in rows:
            out.setdefault((r["ia_identifier"],r["ia_member_path"]),r)
        return sorted(out.values(),key=lambda r:(r["ia_identifier"],r["ia_member_path"]))
    seeds=dedup(seeds); containers=dedup(containers)
    write_csv(seeds,args.out)
    cp=args.containers or args.out.with_name(args.out.stem+".containers.csv")
    write_csv(containers,cp)
    summary={
        "schema":"rar-internet-archive-v1",
        "queries":queries,
        "items_examined":len(chosen),
        "pub_or_zip_seed_rows":len(seeds),
        "nonzip_container_rows":len(containers),
        "unique_items_with_pub_or_zip":len({r["ia_identifier"] for r in seeds}),
        "errors":errors,
    }
    sp=args.summary or args.out.with_suffix(".summary.json")
    sp.write_text(json.dumps(summary,indent=2,ensure_ascii=False),encoding="utf-8")
    print(json.dumps(summary,indent=2,ensure_ascii=False))
    return 0

if __name__=="__main__":
    raise SystemExit(main())
