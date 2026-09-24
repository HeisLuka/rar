#!/usr/bin/env python3
"""Bounded public crawler for class-templates.com Publisher ZIP downloads."""
from __future__ import annotations
import argparse, csv, json, re, time
from collections import deque
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urljoin, urlparse
from urllib.request import Request, urlopen

HOSTS={"class-templates.com","www.class-templates.com"}
UA="rar-pub-class-templates/1.0 (public corpus research)"
PUB_ZIP_RE=re.compile(r"/support-files/[^?#]*pub[^?#]*[.]zip(?:[?#].*)?$",re.I)

class P(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True); self.links=[]; self.title=[]; self._in_title=False
    def handle_starttag(self,tag,attrs):
        if tag.lower()=="a":
            h=dict(attrs).get("href")
            if h:self.links.append(h)
        if tag.lower()=="title":self._in_title=True
    def handle_endtag(self,tag):
        if tag.lower()=="title":self._in_title=False
    def handle_data(self,data):
        if self._in_title:self.title.append(data)

def fetch(url,timeout,max_bytes=4*1024*1024):
    req=Request(url,headers={"User-Agent":UA,"Accept":"text/html,*/*;q=0.1"})
    with urlopen(req,timeout=timeout) as r:
        ctype=(r.headers.get("Content-Type") or "").lower()
        data=r.read(max_bytes+1)
        if len(data)>max_bytes: raise ValueError("page too large")
        return r.geturl(),ctype,data

def normal_page(url):
    p=urlparse(url)
    if p.scheme not in {"http","https"} or (p.hostname or "").lower() not in HOSTS:return None
    if p.query or p.fragment:return None
    path=p.path or "/"
    if path.startswith("/support-files/"):return None
    if not (path=="/" or path.lower().endswith((".html","/"))):return None
    return f"https://www.class-templates.com{path}"

def main():
    ap=argparse.ArgumentParser()
    ap.add_argument("--out",type=Path,required=True)
    ap.add_argument("--summary",type=Path)
    ap.add_argument("--max-pages",type=int,default=350)
    ap.add_argument("--delay",type=float,default=0.15)
    ap.add_argument("--timeout",type=float,default=20)
    args=ap.parse_args()

    q=deque(["https://www.class-templates.com/"])
    seen=set(); downloads={}; errors=[]
    while q and len(seen)<args.max_pages:
        u=q.popleft()
        if u in seen:continue
        seen.add(u)
        try:
            final,ctype,data=fetch(u,args.timeout)
            if "html" not in ctype and b"<html" not in data[:2048].lower():continue
            text=data.decode("utf-8",errors="replace")
            p=P(); p.feed(text)
            for href in p.links:
                v=urljoin(final,href)
                if PUB_ZIP_RE.search(urlparse(v).path):
                    # Canonicalize host/scheme while preserving server path case.
                    vp=urlparse(v)
                    dl=f"https://www.class-templates.com{vp.path}"
                    downloads.setdefault(dl,{
                        "source_page":final,
                        "direct_url":dl,
                        "candidate_filename":Path(vp.path).name,
                        "quarantine":"",
                        "source_class":"class_templates_public",
                        "notes":"Public class-templates.com Publisher ZIP download; extract .pub member and validate bytes",
                    })
                n=normal_page(v)
                if n and n not in seen:q.append(n)
        except Exception as exc:
            errors.append({"url":u,"error":f"{type(exc).__name__}: {exc}"})
        time.sleep(args.delay)

    rows=sorted(downloads.values(),key=lambda r:r["direct_url"].lower())
    args.out.parent.mkdir(parents=True,exist_ok=True)
    fields=["source_page","direct_url","candidate_filename","quarantine","source_class","notes"]
    with args.out.open("w",encoding="utf-8",newline="") as f:
        w=csv.DictWriter(f,fieldnames=fields);w.writeheader();w.writerows(rows)
    summary={
        "schema":"rar-class-templates-public-v1",
        "pages_visited":len(seen),
        "publisher_zip_downloads":len(rows),
        "downloads":[r["direct_url"] for r in rows],
        "errors":errors[:50],
        "error_count":len(errors),
    }
    sp=args.summary or args.out.with_suffix(".summary.json")
    sp.write_text(json.dumps(summary,indent=2,ensure_ascii=False),encoding="utf-8")
    print(json.dumps(summary,indent=2,ensure_ascii=False))
    return 0

if __name__=="__main__":raise SystemExit(main())
