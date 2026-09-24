#!/usr/bin/env python3
"""Bounded same-origin crawler for public domains that already yielded PUB evidence."""
from __future__ import annotations

import argparse, csv, html, json, re, sys, time
from http.client import InvalidURL
from collections import deque
from html.parser import HTMLParser
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import parse_qsl, unquote, urljoin, urlparse, urlunparse
from urllib.request import Request, urlopen
from urllib.robotparser import RobotFileParser

UA="rar-pub-domain-vacuum/1.0 (public format research)"
MAX_HTML=8*1024*1024
PUB_RE=re.compile(r"[.]pub(?:$|[?#])",re.I)
ZIP_RE=re.compile(r"[.]zip(?:$|[?#])",re.I)
PAGE_EXT={".html",".htm",".php",".asp",".aspx",".jsp",".cfm",".shtml",""}

SENSITIVE_QUERY_KEYS={
    "x-amz-signature","x-amz-security-token","signature","access_token",
    "auth_token","session_token","oauth_token",
}


class LinkParser(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.links=[]
        self._href=None
        self._text=[]
    def handle_starttag(self,tag,attrs):
        if tag.lower()=="a":
            href=dict(attrs).get("href")
            if href:
                self._href=href; self._text=[]
    def handle_data(self,data):
        if self._href is not None: self._text.append(data)
    def handle_endtag(self,tag):
        if tag.lower()=="a" and self._href is not None:
            self.links.append((self._href," ".join(self._text).strip()))
            self._href=None; self._text=[]


def hostname(url):
    h=(urlparse(url).hostname or "").casefold().strip(".")
    return h[4:] if h.startswith("www.") else h


def canonical_page(url):
    p=urlparse(url)
    scheme="https" if p.scheme=="https" else "http"
    host=(p.hostname or "").casefold()
    port=f":{p.port}" if p.port and p.port not in {80,443} else ""
    path=p.path or "/"
    return urlunparse((scheme,host+port,path,"",p.query,""))


def safe_public_url(url):
    p=urlparse(url)
    if p.scheme not in {"http","https"} or not p.hostname: return False
    keys={k.casefold() for k,_ in parse_qsl(p.query,keep_blank_values=True)}
    if keys & SENSITIVE_QUERY_KEYS: return False
    if any(k.startswith("x-amz-") for k in keys): return False
    return True


def page_like(url):
    path=urlparse(url).path.casefold()
    suffix=Path(path).suffix
    return suffix in PAGE_EXT or path.endswith("/")


def filename_from(link,text):
    name=unquote(Path(urlparse(link).path).name)
    if name.casefold().endswith((".pub",".zip")): return name[:180]
    m=re.search(r"([^/\\<>|]{1,160}[.]pub)\b",html.unescape(text or ""),re.I)
    return m.group(1).strip() if m else "candidate.pub"


def candidate(link,text):
    low=(text or "").casefold()
    return bool(PUB_RE.search(link) or ZIP_RE.search(link) or ".pub" in low or "pub file" in low or "publisher file" in low)


def fetch_html(url,timeout):
    req=Request(url,headers={"User-Agent":UA,"Accept":"text/html,application/xhtml+xml;q=0.9,*/*;q=0.1"})
    with urlopen(req,timeout=timeout) as r:
        ctype=(r.headers.get("Content-Type") or "").casefold()
        data=r.read(MAX_HTML+1)
        final=r.geturl()
    if len(data)>MAX_HTML: raise ValueError("page exceeds HTML cap")
    if "html" not in ctype and b"<html" not in data[:4096].lower():
        raise ValueError("not html")
    return data,final


def robots_for(base,timeout):
    p=urlparse(base)
    robots=urlunparse((p.scheme,p.netloc,"/robots.txt","","",""))
    rp=RobotFileParser(); rp.set_url(robots)
    try:
        req=Request(robots,headers={"User-Agent":UA})
        with urlopen(req,timeout=timeout) as r:
            text=r.read(1024*1024).decode("utf-8",errors="replace")
        rp.parse(text.splitlines())
        return rp
    except Exception:
        return None


def load_domains(paths):
    starts={}
    for path in paths:
        with path.open("r",encoding="utf-8-sig",newline="") as f:
            for row in csv.DictReader(f):
                for key in ("source_page","direct_url"):
                    u=(row.get(key) or "").strip()
                    h=hostname(u)
                    if not h or not safe_public_url(u): continue
                    starts.setdefault(h,set())
                    if key=="source_page" and page_like(u): starts[h].add(canonical_page(u))
                sp=(row.get("source_page") or "").strip()
                h=hostname(sp)
                if h and safe_public_url(sp):
                    starts.setdefault(h,set()).add(canonical_page(sp))
    return starts


def crawl_domain(domain,starts,max_pages,max_depth,max_candidates,delay,timeout):
    seeds=sorted(starts) or [f"https://{domain}/"]
    # Root is useful even when the original positive page was deep.
    seeds.append(f"https://{domain}/")
    q=deque((u,0,"seed") for u in dict.fromkeys(seeds))
    seen=set(); out=[]; errors=[]
    rp=robots_for(seeds[0],timeout)
    while q and len(seen)<max_pages and len(out)<max_candidates:
        url,depth,discovered_from=q.popleft()
        key=canonical_page(url)
        if key in seen: continue
        if hostname(url)!=domain: continue
        if rp is not None and not rp.can_fetch(UA,url):
            seen.add(key); continue
        seen.add(key)
        try:
            raw,final=fetch_html(url,timeout)
            if hostname(final)!=domain: continue
            parser=LinkParser(); parser.feed(raw.decode("utf-8",errors="replace"))
            for href,text in parser.links:
                link=urljoin(final,href)
                if not safe_public_url(link): continue
                if hostname(link)!=domain: continue
                if candidate(link,text):
                    out.append({
                        "source_page":final,
                        "direct_url":link,
                        "candidate_filename":filename_from(link,text),
                        "quarantine":"",
                        "source_class":"positive_domain_crawl",
                        "notes":"bounded same-origin crawl from a previously PUB-positive domain",
                        "crawl_domain":domain,
                        "crawl_depth":str(depth),
                        "crawl_discovered_from":discovered_from,
                        "crawl_anchor_text":re.sub(r"\s+"," ",text).strip()[:300],
                    })
                    if len(out)>=max_candidates: break
                elif depth<max_depth and page_like(link):
                    q.append((canonical_page(link),depth+1,final))
        except (HTTPError,URLError,TimeoutError,OSError,ValueError,InvalidURL) as exc:
            errors.append({"url":url,"error":f"{type(exc).__name__}: {exc}"})
        time.sleep(delay)
    return out,{"domain":domain,"pages_seen":len(seen),"candidate_rows":len(out),"errors":errors}


def shard_values(values,index,count):
    if count < 1:
        raise ValueError("shard_count must be >= 1")
    if index < 0 or index >= count:
        raise ValueError("shard_index must satisfy 0 <= index < shard_count")
    return [value for i,value in enumerate(values) if i % count == index]


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
    ap.add_argument("--seed",type=Path,action="append",default=[])
    ap.add_argument("--max-domains",type=int,default=40)
    ap.add_argument("--max-pages-per-domain",type=int,default=150)
    ap.add_argument("--max-depth",type=int,default=2)
    ap.add_argument("--max-candidates-per-domain",type=int,default=200)
    ap.add_argument("--delay",type=float,default=0.5)
    ap.add_argument("--timeout",type=float,default=20)
    ap.add_argument("--shard-index",type=int,default=0)
    ap.add_argument("--shard-count",type=int,default=1)
    ap.add_argument("--out",type=Path,required=True)
    ap.add_argument("--summary",type=Path)
    args=ap.parse_args()

    starts=load_domains(args.seed)
    all_domains=sorted(starts)[:args.max_domains]
    domains=shard_values(all_domains,args.shard_index,args.shard_count)
    rows=[]; stats=[]
    for d in domains:
        found,stat=crawl_domain(d,starts[d],args.max_pages_per_domain,args.max_depth,args.max_candidates_per_domain,args.delay,args.timeout)
        rows.extend(found); stats.append(stat)
        print(f"{d}: pages={stat['pages_seen']} candidates={stat['candidate_rows']}",file=sys.stderr)

    # same locator may be linked from many pages; keep first provenance edge here.
    dedup={}
    for r in rows: dedup.setdefault(r["direct_url"],r)
    final=sorted(dedup.values(),key=lambda r:(r["crawl_domain"],r["direct_url"]))
    write_csv(final,args.out)
    summary={
        "schema":"rar-positive-domain-crawl-v2",
        "total_domains_considered":len(all_domains),
        "shard_index":args.shard_index,
        "shard_count":args.shard_count,
        "domains_attempted":len(domains),
        "raw_candidate_edges":len(rows),
        "deduplicated_locators":len(final),
        "domain_stats":stats,
    }
    p=args.summary or args.out.with_suffix(".summary.json")
    p.write_text(json.dumps(summary,indent=2,ensure_ascii=False),encoding="utf-8")
    print(json.dumps(summary,indent=2,ensure_ascii=False))
    return 0

if __name__=="__main__":
    raise SystemExit(main())
