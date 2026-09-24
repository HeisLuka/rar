#!/usr/bin/env python3
import json, urllib.parse, urllib.request, urllib.error, time
from pathlib import Path

BASE="http://faculty.ksu.edu.sa/14812/DocLib1/"
TARGET=BASE+"Publication4.pub"
CC_INDEX="https://index.commoncrawl.org/CC-MAIN-2017-17-index"

def get(url, timeout=40):
    req=urllib.request.Request(url,headers={"User-Agent":"rar-pub-provenance/1.0"})
    try:
        with urllib.request.urlopen(req,timeout=timeout) as r:
            body=r.read(4*1024*1024)
            return {"status":int(getattr(r,"status",200)),"url":r.geturl(),"content_type":r.headers.get("Content-Type",""),"body":body.decode("utf-8","replace"),"error":None}
    except urllib.error.HTTPError as e:
        try: body=e.read(512*1024).decode("utf-8","replace")
        except Exception: body=""
        return {"status":int(e.code),"url":url,"content_type":e.headers.get("Content-Type","") if e.headers else "","body":body,"error":None}
    except Exception as e:
        return {"status":None,"url":url,"content_type":"","body":"","error":f"{type(e).__name__}: {e}"}

def cc_query(pattern):
    u=CC_INDEX+"?"+urllib.parse.urlencode({"url":pattern,"output":"json"})
    r=get(u)
    rows=[]
    if r["status"]==200:
        for line in r["body"].splitlines():
            line=line.strip()
            if not line: continue
            try: rows.append(json.loads(line))
            except Exception: pass
    return {"query":pattern,"request":u,"http_status":r["status"],"error":r["error"],"rows":rows}

def wayback(pattern):
    params={
      "url":pattern,"output":"json","fl":"timestamp,original,mimetype,statuscode,digest,length",
      "filter":"statuscode:200","collapse":"digest","limit":"2000"
    }
    u="https://web.archive.org/cdx/search/cdx?"+urllib.parse.urlencode(params)
    r=get(u,60)
    rows=[]
    if r["status"]==200:
        try:
            j=json.loads(r["body"])
            if isinstance(j,list) and j:
                hdr=j[0]
                for vals in j[1:]:
                    rows.append(dict(zip(hdr,vals)))
        except Exception:
            pass
    return {"query":pattern,"request":u,"http_status":r["status"],"error":r["error"],"rows":rows}

def main():
    import argparse
    ap=argparse.ArgumentParser(); ap.add_argument("--out",type=Path,required=True)
    a=ap.parse_args(); a.out.mkdir(parents=True,exist_ok=True)

    cc_exact=cc_query(TARGET)
    time.sleep(.5)
    cc_dir=cc_query(BASE+"*")
    time.sleep(.5)
    wb_exact=wayback(TARGET)
    time.sleep(.5)
    wb_dir=wayback(BASE+"*")

    # relevant sibling names: same stem or web-export-ish extensions/assets
    candidates={}
    def add(source,row):
        url=row.get("url") or row.get("original") or ""
        if not url: return
        low=url.lower()
        relevant=("publication4" in low or any(x in low for x in [".htm",".html",".mht",".mhtml",".xml","_files/","filelist.xml","pubmaster"]))
        if relevant:
            candidates.setdefault(url,{"url":url,"sources":[]})
            candidates[url]["sources"].append({"source":source,"row":row})
    for x in cc_dir["rows"]: add("commoncrawl",x)
    for x in wb_dir["rows"]: add("wayback",x)

    target_cc=[x for x in cc_exact["rows"] if (x.get("url") or "").lower()==TARGET.lower()]
    target_wb=[x for x in wb_exact["rows"] if (x.get("original") or "").lower()==TARGET.lower()]

    out={
      "schema":"ksu-publication4-provenance.v1",
      "target_url":TARGET,
      "target_sha256":"116f6aaad41dd5d3d49731b5d8363dbf6503cc13df2aadb7ea7afecf2e34f79f",
      "known_size":67584,
      "commoncrawl_exact":cc_exact,
      "commoncrawl_directory":cc_dir,
      "wayback_exact":wb_exact,
      "wayback_directory":wb_dir,
      "exact_target_cc_rows":target_cc,
      "exact_target_wayback_rows":target_wb,
      "relevant_sibling_candidates":list(candidates.values()),
      "boundary":"Metadata/index queries only. No PUB or sibling payload bytes downloaded."
    }
    (a.out/"provenance.json").write_text(json.dumps(out,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
    summary={
      "target_url":TARGET,
      "cc_exact_http":cc_exact["http_status"],
      "cc_exact_rows":len(cc_exact["rows"]),
      "cc_dir_http":cc_dir["http_status"],
      "cc_dir_rows":len(cc_dir["rows"]),
      "wayback_exact_http":wb_exact["http_status"],
      "wayback_exact_rows":len(wb_exact["rows"]),
      "wayback_dir_http":wb_dir["http_status"],
      "wayback_dir_rows":len(wb_dir["rows"]),
      "relevant_sibling_candidates":len(candidates),
    }
    (a.out/"summary.json").write_text(json.dumps(summary,indent=2)+"\n",encoding="utf-8")
    print(json.dumps(summary,indent=2))
    print("EXACT_CC")
    for x in target_cc: print(json.dumps(x,ensure_ascii=False))
    print("EXACT_WAYBACK")
    for x in target_wb: print(json.dumps(x,ensure_ascii=False))
    print("SIBLINGS")
    for x in candidates.values(): print(json.dumps(x,ensure_ascii=False)[:2500])

if __name__=="__main__": main()
