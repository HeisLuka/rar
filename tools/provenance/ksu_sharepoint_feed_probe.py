#!/usr/bin/env python3
import gzip, json, re, urllib.parse, urllib.request, urllib.error
from pathlib import Path

CC="https://index.commoncrawl.org/CC-MAIN-2017-17-index"
PATTERNS=[
  ("owssvr","http://faculty.ksu.edu.sa/14812/_vti_bin/owssvr.dll*"),
  ("listfeed","http://faculty.ksu.edu.sa/14812/_layouts/listfeed.aspx*"),
  ("home","http://faculty.ksu.edu.sa/14812/Pages/Home.aspx"),
  ("publications","http://faculty.ksu.edu.sa/14812/Publications/Publications.aspx"),
]
TOKENS=["Publication4.pub","Publication4","DocLib1","Modified","Editor","Author","FileRef","FileLeafRef","ows_","محمد مفلح","الدوسري"]

def get(url,timeout=30,headers=None):
    h={"User-Agent":"rar-pub-ksu-feed-probe/1.0"}
    if headers: h.update(headers)
    req=urllib.request.Request(url,headers=h)
    try:
      with urllib.request.urlopen(req,timeout=timeout) as r:
        return int(getattr(r,"status",200)),r.read(6*1024*1024),r.geturl(),r.headers.get("Content-Type","")
    except urllib.error.HTTPError as e:
      try:b=e.read(512*1024)
      except Exception:b=b""
      return int(e.code),b,url,e.headers.get("Content-Type","") if e.headers else ""
    except Exception as e:
      return None,b"",url,f"ERROR {type(e).__name__}: {e}"

def cc_query(pattern):
    u=CC+"?"+urllib.parse.urlencode({"url":pattern,"output":"json"})
    st,body,final,ct=get(u,40)
    rows=[]
    if st==200:
      for line in body.decode("utf-8","replace").splitlines():
        try: rows.append(json.loads(line))
        except Exception: pass
    return {"pattern":pattern,"request":u,"status":st,"rows":rows}

def fetch_warc(row):
    u="https://data.commoncrawl.org/"+row["filename"]
    start=int(row["offset"]); ln=int(row["length"])
    st,raw,final,ct=get(u,30,{"Range":f"bytes={start}-{start+ln-1}"})
    if st not in (200,206): return {"status":st,"error":ct}
    try:data=gzip.decompress(raw)
    except Exception as e:return {"status":st,"error":f"gzip {e}"}
    p=data.find(b"\r\n\r\n"); rest=data[p+4:] if p>=0 else data
    q=rest.find(b"\r\n\r\n"); payload=rest[q+4:] if q>=0 else rest
    return {"status":st,"payload":payload.decode("utf-8","replace"),"bytes":len(payload)}

def scan(text):
    low=text.lower(); hits=[]
    for tok in TOKENS:
      i=0
      while True:
        j=low.find(tok.lower(),i)
        if j<0:break
        ctx=re.sub(r"\s+"," ",text[max(0,j-350):min(len(text),j+len(tok)+700)])
        hits.append({"token":tok,"context":ctx[:1400]})
        i=j+len(tok)
        if sum(1 for h in hits if h["token"]==tok)>=30:break
    return hits

def main():
    import argparse
    ap=argparse.ArgumentParser(); ap.add_argument("--out",type=Path,required=True)
    a=ap.parse_args(); a.out.mkdir(parents=True,exist_ok=True)
    results=[]
    for label,pat in PATTERNS:
      q=cc_query(pat)
      rec={"label":label,**q,"captures":[]}
      for row in q["rows"][:50]:
        if row.get("status")!="200" or row.get("mime","").lower() not in {"text/html","application/xml","text/xml","application/rss+xml","text/plain"}:
          continue
        f=fetch_warc(row)
        cap={"row":row,"fetch_status":f.get("status"),"error":f.get("error"),"payload_bytes":f.get("bytes")}
        if "payload" in f:
          hits=scan(f["payload"])
          cap["hits"]=hits
          cap["publication4_present"]="publication4" in f["payload"].lower()
          cap["preview"]=re.sub(r"\s+"," ",f["payload"])[:5000]
        rec["captures"].append(cap)
      results.append(rec)
    summary={
      "schema":"ksu-sharepoint-feed-probe.v1",
      "patterns":len(results),
      "index_rows":sum(len(x["rows"]) for x in results),
      "text_captures_fetched":sum(len(x["captures"]) for x in results),
      "publication4_positive_captures":sum(sum(1 for c in x["captures"] if c.get("publication4_present")) for x in results),
      "boundary":"Common Crawl index + archived text/XML/HTML SharePoint metadata surfaces only. No Publication4.pub payload retrieval."
    }
    (a.out/"report.json").write_text(json.dumps({"summary":summary,"results":results},indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
    (a.out/"summary.json").write_text(json.dumps(summary,indent=2)+"\n",encoding="utf-8")
    print(json.dumps(summary,indent=2))
    for x in results:
      print("QUERY",x["label"],"status",x["status"],"rows",len(x["rows"]))
      for c in x["captures"]:
        print("CAP",x["label"],c["row"].get("timestamp"),c["row"].get("url"),"publication4",c.get("publication4_present"),"bytes",c.get("payload_bytes"),"error",c.get("error"))
        for h in c.get("hits",[]):
          if h["token"] in {"Publication4.pub","Publication4","FileRef","FileLeafRef","Modified","Editor","Author","محمد مفلح","الدوسري"}:
            print("HIT",x["label"],json.dumps(h,ensure_ascii=False))
if __name__=="__main__":main()
