#!/usr/bin/env python3
import gzip, html, json, re, urllib.request, urllib.error
from pathlib import Path
from urllib.parse import urljoin

ROWS=[
  {
    "label":"allitems_forms",
    "timestamp":"20170430210052",
    "url":"http://faculty.ksu.edu.sa/14812/DocLib1/Forms/AllItems.aspx?RootFolder=%2F14812%2FDocLib1%2FForms&FolderCTID=0x012001",
    "filename":"crawl-data/CC-MAIN-2017-17/segments/1492917125849.25/warc/CC-MAIN-20170423031205-00370-ip-10-145-167-34.ec2.internal.warc.gz",
    "offset":148701218,
    "length":15619
  },
  {
    "label":"allitems_document",
    "timestamp":"20170427031305",
    "url":"http://faculty.ksu.edu.sa/14812/DocLib1/Forms/AllItems.aspx?RootFolder=%2F14812%2FDocLib1%2FForms%2FDocument&FolderCTID=0x012001",
    "filename":"crawl-data/CC-MAIN-2017-17/segments/1492917121865.67/warc/CC-MAIN-20170423031201-00074-ip-10-145-167-34.ec2.internal.warc.gz",
    "offset":143115356,
    "length":15620
  }
]
TOKENS=["Publication4","DocLib1","14812","Modified","Created","Author","Editor","FileLeafRef","FileRef","_UIVersionString","Publisher","mspublisher"]

def fetch_range(row):
    u="https://data.commoncrawl.org/"+row["filename"]
    start=row["offset"]; end=start+row["length"]-1
    req=urllib.request.Request(u,headers={"User-Agent":"rar-pub-ksu-html-probe/1.0","Range":f"bytes={start}-{end}"})
    with urllib.request.urlopen(req,timeout=30) as r:
        raw=r.read(row["length"]+1024)
        status=int(getattr(r,"status",200))
    return status,u,raw

def extract_http_payload(raw):
    data=gzip.decompress(raw)
    # WARC header, then HTTP header, then payload
    p=data.find(b"\r\n\r\n")
    if p<0: return data,b""
    rest=data[p+4:]
    q=rest.find(b"\r\n\r\n")
    if q<0: return rest,b""
    return rest[:q],rest[q+4:]

def main():
    import argparse
    ap=argparse.ArgumentParser(); ap.add_argument("--out",type=Path,required=True)
    a=ap.parse_args(); a.out.mkdir(parents=True,exist_ok=True)
    out=[]
    for row in ROWS:
        rec={**row}
        try:
            status,src,raw=fetch_range(row)
            http_headers,payload=extract_http_payload(raw)
            text=payload.decode("utf-8","replace")
            plain=re.sub(r"\s+"," ",html.unescape(re.sub(r"<[^>]+>"," ",text))).strip()
            hits=[]
            low=text.lower()
            for tok in TOKENS:
                i=0
                while True:
                    j=low.find(tok.lower(),i)
                    if j<0: break
                    ctx=re.sub(r"\s+"," ",html.unescape(text[max(0,j-240):min(len(text),j+len(tok)+420)]))
                    hits.append({"token":tok,"context":ctx[:900]})
                    i=j+len(tok)
                    if sum(1 for h in hits if h["token"]==tok)>=20: break
            hrefs=[]
            for m in re.finditer(r'''href\s*=\s*["']([^"']+)["']''',text,re.I):
                href=html.unescape(m.group(1))
                if any(x in href.lower() for x in ["doclib1","publication","14812","forms"]):
                    hrefs.append(href)
            rec.update({
              "fetch_status":status,
              "source_url":src,
              "warc_member_bytes":len(raw),
              "http_headers":http_headers.decode("iso-8859-1","replace")[:4000],
              "payload_bytes":len(payload),
              "token_hits":hits,
              "relevant_hrefs":list(dict.fromkeys(hrefs))[:300],
              "plain_preview":plain[:5000],
            })
        except Exception as e:
            rec.update({"error":f"{type(e).__name__}: {e}"})
        out.append(rec)
    summary={
      "schema":"ksu-sharepoint-html-probe.v1",
      "pages":len(out),
      "successful":sum("fetch_status" in x for x in out),
      "publication4_hits":sum(any(h["token"]=="Publication4" for h in x.get("token_hits",[])) for x in out),
      "total_relevant_hrefs":sum(len(x.get("relevant_hrefs",[])) for x in out),
      "boundary":"Only two archived text/html SharePoint list pages were fetched from Common Crawl by exact WARC coordinates. Publication4.pub payload bytes were not fetched."
    }
    (a.out/"report.json").write_text(json.dumps({"summary":summary,"records":out},indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
    (a.out/"summary.json").write_text(json.dumps(summary,indent=2)+"\n",encoding="utf-8")
    print(json.dumps(summary,indent=2))
    for x in out:
        print("PAGE",x["label"],"error=",x.get("error"),"payload=",x.get("payload_bytes"))
        for h in x.get("token_hits",[]):
            if h["token"] in {"Publication4","DocLib1","Modified","Created","Author","Editor","_UIVersionString","Publisher","mspublisher"}:
                print("HIT",x["label"],json.dumps(h,ensure_ascii=False))
        for href in x.get("relevant_hrefs",[]):
            print("HREF",x["label"],href)
if __name__=="__main__": main()
