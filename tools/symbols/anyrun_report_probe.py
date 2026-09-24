#!/usr/bin/env python3
from __future__ import annotations
import hashlib, json, re, time
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen

UA="rar-pub-anyrun-report-probe/1.0 (public format research)"
REPORTS=[
  {
    "label":"Publisher 2010 public report",
    "task_id":"4760c433-3b4a-499d-b8a4-3c905446c6b0",
    "report_object_sha256":"67e887332333a83090edd10aec577145783312c9ea2dcdb9faf96fd425b73962",
    "expected_mspub_version":"14.0.6026.1000",
  },
  {
    "label":"Publisher 2016 public report A",
    "task_id":"5ac51413-d1d7-4981-a515-f683afba6bca",
    "report_object_sha256":"e050ea777d910137fff7c160992ec026ab4f76832b6c96701b114e379abf4ca3",
    "expected_mspub_version":"16.0.16026.20146",
  },
  {
    "label":"Publisher 2016 public report B",
    "task_id":"f4421520-f362-4042-9b75-7c2d1409deae",
    "report_object_sha256":"e050ea777d910137fff7c160992ec026ab4f76832b6c96701b114e379abf4ca3",
    "expected_mspub_version":"16.0.16026.20146",
  },
]
TOKENS=[
  "MSPUB.EXE","MORPH9.DLL","PTXT9.DLL","PUBCONV.DLL","PUBTRAP.DLL","PRTF9.DLL",
  "OplPluo","OplPlcCmob","OhPlccmob","DwNextUniqueOid","ImpositionEngine",
  "OtyOrig","OplLastFmt","CreateAgent","MorphingContextData","ObjectTracking",
  "OplControlling","mspub.pdb","morph9.pdb","ptxt9.pdb","pubconv.pdb",
  "t:\\pub\\","P:\\Target\\","daddev\\office10",
]
ENDPOINTS=[
  ("summary_json","https://api.any.run/report/{task}/summary/json","json"),
  ("summary_html","https://api.any.run/report/{task}/summary/html","text"),
  ("ioc_json","https://api.any.run/report/{task}/ioc/json","json"),
  ("graph","https://content.any.run/tasks/{task}/graph","auto"),
]

def fetch(url, timeout=25, limit=8*1024*1024):
    req=Request(url,headers={"User-Agent":UA,"Accept":"application/json,text/html,text/plain,*/*"})
    try:
        with urlopen(req,timeout=timeout) as r:
            raw=r.read(limit+1)
            status=int(getattr(r,"status",200))
            final=r.geturl()
            ctype=r.headers.get("Content-Type","")
        truncated=len(raw)>limit
        if truncated: raw=raw[:limit]
        return {"status":status,"final_url":final,"content_type":ctype,"raw":raw,"error":None,"truncated":truncated}
    except HTTPError as e:
        try: raw=e.read(512*1024)
        except Exception: raw=b""
        return {"status":int(e.code),"final_url":url,"content_type":e.headers.get("Content-Type","") if e.headers else "","raw":raw,"error":None,"truncated":False}
    except Exception as e:
        return {"status":None,"final_url":url,"content_type":"","raw":b"","error":f"{type(e).__name__}: {e}","truncated":False}

def iter_strings(obj, path="$"):
    if isinstance(obj,dict):
        for k,v in obj.items():
            yield from iter_strings(v, path+"."+str(k))
    elif isinstance(obj,list):
        for i,v in enumerate(obj):
            yield from iter_strings(v, path+f"[{i}]")
    elif isinstance(obj,str):
        yield path,obj

def find_debug_strings(obj):
    out=[]
    if isinstance(obj,dict):
        for k,v in obj.items():
            if str(k).lower()=="debugstrings":
                out.append(v)
            out.extend(find_debug_strings(v))
    elif isinstance(obj,list):
        for v in obj:
            out.extend(find_debug_strings(v))
    return out

def relevant_hits_from_text(text):
    low=text.lower()
    hits=[]
    for tok in TOKENS:
        idx=0
        tl=tok.lower()
        while True:
            i=low.find(tl,idx)
            if i<0: break
            start=max(0,i-100); end=min(len(text),i+len(tok)+180)
            context=re.sub(r"\s+"," ",text[start:end]).strip()
            hits.append({"token":tok,"context":context[:360]})
            idx=i+len(tok)
            if sum(1 for h in hits if h["token"]==tok)>=10:
                break
    return hits

def main():
    import argparse
    ap=argparse.ArgumentParser()
    ap.add_argument("--out",type=Path,required=True)
    ap.add_argument("--timeout",type=float,default=25)
    ap.add_argument("--delay",type=float,default=.35)
    a=ap.parse_args()
    a.out.mkdir(parents=True,exist_ok=True)

    records=[]
    for rep in REPORTS:
        for name,tpl,kind in ENDPOINTS:
            url=tpl.format(task=rep["task_id"])
            r=fetch(url,a.timeout)
            raw=r.pop("raw")
            sha=hashlib.sha256(raw).hexdigest() if raw else None
            text=raw.decode("utf-8","replace") if raw else ""
            parsed=None
            json_error=None
            if raw and (kind=="json" or "json" in r["content_type"].lower() or text.lstrip().startswith(("{","["))):
                try: parsed=json.loads(text)
                except Exception as e: json_error=f"{type(e).__name__}: {e}"
            hits=[]
            string_hits=[]
            debug_summary=[]
            top_keys=[]
            if parsed is not None:
                if isinstance(parsed,dict): top_keys=sorted(str(k) for k in parsed.keys())[:100]
                for path,s in iter_strings(parsed):
                    sl=s.lower()
                    matched=[tok for tok in TOKENS if tok.lower() in sl]
                    if matched:
                        string_hits.append({"path":path,"tokens":matched,"value":re.sub(r"\s+"," ",s)[:500]})
                        if len(string_hits)>=200: break
                for node in find_debug_strings(parsed):
                    if isinstance(node,list):
                        debug_summary.append({
                          "type":"list","count":len(node),
                          "target_hits":sum(any(tok.lower() in str(x).lower() for tok in TOKENS) for x in node),
                        })
                    elif isinstance(node,dict):
                        debug_summary.append({"type":"dict","count":len(node)})
                    elif node is not None:
                        debug_summary.append({"type":type(node).__name__,"preview":str(node)[:200]})
            else:
                hits=relevant_hits_from_text(text)

            expected_version_present=rep["expected_mspub_version"].lower() in text.lower()
            mspub_present="mspub.exe" in text.lower()
            records.append({
              **rep,
              "endpoint":name,
              "url":url,
              **r,
              "response_sha256":sha,
              "response_bytes":len(raw),
              "json_parsed":parsed is not None,
              "json_error":json_error,
              "top_level_keys":top_keys,
              "expected_version_present":expected_version_present,
              "mspub_present":mspub_present,
              "target_string_hits":string_hits,
              "target_text_hits":hits,
              "debug_strings":debug_summary,
            })
            time.sleep(a.delay)

    accessible=[x for x in records if x["status"]==200]
    json_accessible=[x for x in records if x["status"]==200 and x["json_parsed"]]
    all_hits=[]
    for x in records:
        for h in x["target_string_hits"]:
            all_hits.append((x["task_id"],x["endpoint"],h))
        for h in x["target_text_hits"]:
            all_hits.append((x["task_id"],x["endpoint"],h))
    high_value=[h for h in all_hits if any(tok.lower() in json.dumps(h[2]).lower() for tok in [
      "morph9.dll","ptxt9.dll","pubconv.dll","pubtrap.dll","prtf9.dll","oplpluo","oplplccmob",
      "dwnextuniqueoid","impositionengine","otyorig","opllastfmt","createagent","morphingcontextdata",
      "objecttracking","oplcontrolling","mspub.pdb","morph9.pdb","ptxt9.pdb","pubconv.pdb"
    ])]
    summary={
      "schema":"pub-anyrun-report-probe.v1",
      "public_task_ids":len(REPORTS),
      "endpoint_queries":len(records),
      "http_200":len(accessible),
      "json_parsed_200":len(json_accessible),
      "summary_json_200":sum(x["endpoint"]=="summary_json" and x["status"]==200 for x in records),
      "summary_html_200":sum(x["endpoint"]=="summary_html" and x["status"]==200 for x in records),
      "ioc_json_200":sum(x["endpoint"]=="ioc_json" and x["status"]==200 for x in records),
      "graph_200":sum(x["endpoint"]=="graph" and x["status"]==200 for x in records),
      "mspub_positive_control_rows":sum(x["mspub_present"] for x in records),
      "expected_version_rows":sum(x["expected_version_present"] for x in records),
      "high_value_target_hits":len(high_value),
      "boundary":"Public unauthenticated report metadata/summary endpoints only. No sample, PCAP, memory dump, process dump, dropped-file or malware-config payload downloads.",
    }
    (a.out/"records.json").write_text(json.dumps(records,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
    (a.out/"summary.json").write_text(json.dumps(summary,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
    print(json.dumps(summary,indent=2))
    for x in records:
        print(x["task_id"],x["endpoint"],x["status"],x["content_type"],x["response_bytes"],
              "json="+str(x["json_parsed"]),"mspub="+str(x["mspub_present"]),
              "ver="+str(x["expected_version_present"]),"hits="+str(len(x["target_string_hits"])+len(x["target_text_hits"])),
              "debug="+json.dumps(x["debug_strings"],ensure_ascii=False))
    if high_value:
        print("HIGH_VALUE_HITS")
        for task,endpoint,h in high_value[:100]:
            print(task,endpoint,json.dumps(h,ensure_ascii=False))
if __name__=="__main__":
    raise SystemExit(main())
