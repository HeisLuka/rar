#!/usr/bin/env python3
"""Mine bounded public forum/support pages for Publisher attachment locators."""
from __future__ import annotations
import argparse,csv,html,json,re,time
from html.parser import HTMLParser
from pathlib import Path
from urllib.error import HTTPError,URLError
from urllib.parse import urljoin,urlparse
from urllib.request import Request,urlopen

UA="rar-pub-forum-locator/1.0 (public format research)"
MAX=8*1024*1024
FILE_RE=re.compile(r'(?i)([A-Za-z0-9][^\s<>"\'|]{0,140}[.]pub)\b')
URL_RE=re.compile(r'https?://[^\s<>"\']+[.]pub(?:\?[^\s<>"\']*)?',re.I)

class P(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True); self.links=[]; self._h=None; self._t=[]
    def handle_starttag(self,tag,attrs):
        if tag.lower()=="a":
            h=dict(attrs).get("href")
            if h: self._h=h; self._t=[]
    def handle_data(self,data):
        if self._h is not None:self._t.append(data)
    def handle_endtag(self,tag):
        if tag.lower()=="a" and self._h is not None:
            self.links.append((self._h," ".join(self._t).strip())); self._h=None; self._t=[]

def fetch(url,timeout,retries=3,backoff=2.0):
    last=None
    for attempt in range(retries+1):
        try:
            req=Request(url,headers={"User-Agent":UA,"Accept":"text/html,*/*;q=0.1"})
            with urlopen(req,timeout=timeout) as r:
                data=r.read(MAX+1); final=r.geturl()
            if len(data)>MAX: raise ValueError("page too large")
            return data.decode("utf-8",errors="replace"),final
        except HTTPError as exc:
            last=exc
            if attempt<retries and exc.code in {429,500,502,503,504}:
                time.sleep(backoff*(attempt+1)); continue
            raise
        except (URLError,TimeoutError,OSError) as exc:
            last=exc
            if attempt<retries:
                time.sleep(backoff*(attempt+1)); continue
            raise
    raise last

def filename_from(text,url=""):
    name=Path(urlparse(url).path).name
    if name.casefold().endswith(".pub"): return name
    cleaned=re.sub(r"\\s+"," ",html.unescape(text or "")).strip(" .,:;()[]{}")
    if cleaned.casefold().endswith(".pub") and len(cleaned)<=180:
        lowered=cleaned.casefold()
        for prefix in (
            "please see ", "see ", "download ", "download file ",
            "attachment ", "attached ", "file: ", "file ",
        ):
            if lowered.startswith(prefix):
                candidate=cleaned[len(prefix):].strip(" .,:;()[]{}")
                if candidate.casefold().endswith(".pub"):
                    return candidate
        return cleaned
    m=FILE_RE.search(cleaned)
    return m.group(1).strip(".,;:()[]{}") if m else ""

def link_candidate(href,text):
    low=(href+" "+text).casefold()
    return (
        ".pub" in low or ".zip" in low or "attachment" in low or "/files/" in low
        or "download" in low and "publisher" in low
    )

def mine(source_class,page,text,final):
    p=P(); p.feed(text)
    rows=[]
    for href,anchor in p.links:
        u=urljoin(final,href)
        fn=filename_from(anchor,u)
        if not link_candidate(u,anchor) and not fn: continue
        rows.append({
            "source_page":page,"direct_url":u,"candidate_filename":fn or "candidate.pub",
            "quarantine":"","source_class":"forum_attachment",
            "notes":"public forum/support locator; prose is provenance only",
            "forum_source_class":source_class,"forum_final_page":final,
            "forum_anchor_text":re.sub(r"\s+"," ",anchor).strip()[:300],
            "forum_locator_status":"linked",
        })
    for u in URL_RE.findall(text):
        rows.append({
            "source_page":page,"direct_url":html.unescape(u),"candidate_filename":filename_from("",u),
            "quarantine":"","source_class":"forum_attachment","notes":"PUB URL mentioned in public forum/support page",
            "forum_source_class":source_class,"forum_final_page":final,"forum_anchor_text":"","forum_locator_status":"text_url",
        })
    linked_names={r["candidate_filename"].casefold() for r in rows if r.get("candidate_filename")}
    plain=re.sub(r"<[^>]+>"," ",text)
    for m in FILE_RE.finditer(html.unescape(plain)):
        fn=m.group(1).strip(".,;:()[]{}")
        if fn.casefold() in linked_names: continue
        rows.append({
            "source_page":page,"direct_url":"","candidate_filename":fn,
            "quarantine":"","source_class":"forum_filename_only",
            "notes":"filename-only forum provenance; feed source page + filename to live/archive resolver",
            "forum_source_class":source_class,"forum_final_page":final,"forum_anchor_text":"",
            "forum_locator_status":"filename_only",
        })
    return rows

def registry_exact_row(source):
    url=(source.get("exact_url") or "").strip()
    if not url: return None
    p=urlparse(url)
    if p.scheme not in {"http","https"} or not p.hostname: return None
    fn=(source.get("exact_filename") or "").strip() or filename_from("",url) or "candidate.pub"
    return {
        "source_page":(source.get("source_page") or "").strip(),
        "direct_url":url,
        "candidate_filename":fn,
        "quarantine":"",
        "source_class":"forum_attachment",
        "notes":(source.get("notes") or "registry-pinned public attachment locator").strip(),
        "forum_source_class":(source.get("source_class") or "forum").strip(),
        "forum_final_page":(source.get("source_page") or "").strip(),
        "forum_anchor_text":"",
        "forum_locator_status":"registry_exact",
        "forum_claimed_version":(source.get("claimed_version") or "").strip(),
    }


def write(rows,path):
    path.parent.mkdir(parents=True,exist_ok=True); keys=[];seen=set()
    for r in rows:
        for k in r:
            if k not in seen:seen.add(k);keys.append(k)
    with path.open("w",encoding="utf-8",newline="") as f:
        w=csv.DictWriter(f,fieldnames=keys);w.writeheader();w.writerows(rows)

def main():
    ap=argparse.ArgumentParser();ap.add_argument("--registry",required=True);ap.add_argument("--out",type=Path,required=True)
    ap.add_argument("--summary",type=Path);ap.add_argument("--timeout",type=float,default=25);ap.add_argument("--delay",type=float,default=1)
    ap.add_argument("--retries",type=int,default=3);ap.add_argument("--backoff",type=float,default=2.0)
    ap.add_argument("--max-pages",type=int,default=100);args=ap.parse_args()
    with open(args.registry,encoding="utf-8-sig",newline="") as f: sources=list(csv.DictReader(f))[:args.max_pages]
    rows=[];errors=[]
    for i,s in enumerate(sources):
        exact=registry_exact_row(s)
        if exact is not None: rows.append(exact)
        page=(s.get("source_page") or "").strip()
        if page:
            try:
                text,final=fetch(page,args.timeout,args.retries,args.backoff)
                mined=mine(s.get("source_class","forum"),page,text,final)
                claimed=(s.get("claimed_version") or "").strip()
                for row in mined: row["forum_claimed_version"]=claimed
                rows.extend(mined)
            except Exception as exc:
                errors.append({"source_page":page,"error":f"{type(exc).__name__}: {exc}"})
        if i+1<len(sources):time.sleep(args.delay)
    dedup={}
    for r in rows:
        key=(r.get("source_page",""),r.get("direct_url",""),r.get("candidate_filename","").casefold())
        dedup.setdefault(key,r)
    final=sorted(dedup.values(),key=lambda r:(r["source_page"],r.get("candidate_filename",""),r.get("direct_url","")))
    write(final,args.out)
    summary={"schema":"rar-forum-locator-v2","pages_attempted":len(sources),"locator_rows":len(final),
             "registry_exact_rows":sum(r.get("forum_locator_status")=="registry_exact" for r in final),
             "linked_rows":sum(r.get("forum_locator_status") not in {"filename_only","registry_exact"} for r in final),
             "filename_only_rows":sum(r.get("forum_locator_status")=="filename_only" for r in final),"errors":errors}
    sp=args.summary or args.out.with_suffix(".summary.json");sp.write_text(json.dumps(summary,indent=2,ensure_ascii=False),encoding="utf-8")
    print(json.dumps(summary,indent=2,ensure_ascii=False));return 0
if __name__=="__main__":raise SystemExit(main())
