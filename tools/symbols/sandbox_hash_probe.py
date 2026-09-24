#!/usr/bin/env python3
from __future__ import annotations
import argparse, html, json, re, time
from pathlib import Path
from urllib.error import HTTPError
from urllib.parse import quote
from urllib.request import Request, urlopen

UA="rar-pub-symbol-sandbox/1.1 (public format research)"

RECORDS=[
("2002","MSPUB.EXE","02603be3cc900d167aa4842f64320f8550c44d36cb149e0c824830f502d73fdc","MSPUBO.pdb","3F8AF3342"),
("2002","MORPH9.DLL","ef4da9d11e0d54934f72189e37855ac5dfa99fb245ca492be10f13a430dd7f11","MORPH9O.pdb","3F8AF2EE2"),
("2002","PTXT9.DLL","1210ce3347769194536750f9e1f619cba97f2c256e99d045f68345ed41d97c50","PTXT9O.pdb","3F8AF3752"),
("2002","PUBCONV.DLL","4025b0022e6bd64928b5b3117c146c92bf4bd2939764960eae372c07919c949c","PUBCONVO.pdb","3F8AF3842"),
("2007","MSPUB.EXE","3d482ddb0e07c7232c903c888724c1d332a7cb5e029f964688606352a04a5683","mspub.pdb","7BC6715E6CA54F4D919A854C04FC50272"),
("2007","MORPH9.DLL","61cef76f0f1aad665b437483c2b77229f7f97ba1dfc0fb413c31947c40d4617d","morph9.pdb","9BADA37C83444F8F9BF6DE5D2D333C412"),
("2007","PTXT9.DLL","847276b2b803a318cf86196153602959b9c7831aca81aa9bad8ea59acbacf3ab","ptxt9.pdb","9DF153BF514D498088B2AF1A8195F27E2"),
("2007","PUBCONV.DLL","9d9c3dd583eb136eed5fd19627178af672409ef379ba5b767066da615d182c16","pubconv.pdb","BB1E1E0A09C1486FB9C8D46B8F5B0BA62"),
("2013","MSPUB.EXE","578937a126bb5f7e5c2e0c9ec80f52abe74a6846bc9f5fdde286b28e95538344","mspub.pdb","E9B57F7F314C4F3AA6DEC419D691F7582"),
("2013","MORPH9.DLL","0661f16aba0f23da2b2cfc7e6516bbc690e8eb83f6760fb36a73e030a10a54f0","morph9.pdb","B3679184181A41A3A0D38AF8345AA06C2"),
("2013","PTXT9.DLL","d0350be20326b71e6fd9529d0debc79e388451b987a8feb3a8ae62972b35e24d","ptxt9.pdb","72032A2A6C7E44E8B28BCB4A11F5A09A2"),
("2013","PUBCONV.DLL","b1eff9290146bd5f140dfa8ef5013a8b5332a36946ebce7bbfc13ebad3369d58","pubconv.pdb","92FFE1DDF9E744059E938256D15FACE22"),
("2016","MSPUB.EXE","7bca60abd07655518f0797d4cb55cbc0f588389626ebc0c7035f2b1c9efaaa5b","mspub.pdb","44CB6B28DB744E3CB07C49BEF3E770322"),
("2016","MORPH9.DLL","b155b07ffe4e48710828d18833d615df022f09108a531f6249c9c2c543b88a03","morph9.pdb","C612D21409CD42759BFF296A215935DC2"),
("2016","PTXT9.DLL","599e2272a73a8a4594e3c46f0312f99e7faf0e249b908f211665be9504b4f624","ptxt9.pdb","65217074C1424306A6D6FC57EED1FB5B2"),
("2016","PUBCONV.DLL","404b5d21087b16eac74670f87b792799c33ecdbc1d1ebc5be9429c5db54777b8","pubconv.pdb","7B68ADABF0034D079D0C7003D36390E62"),
]

TRIAGE_POSITIVE_CONTROL="a0a02694788266de5797199d0011ce6fcb21a8b65cb13595a760ddfc6ca13ca3"
HYBRID_POSITIVE_CONTROL="2a84f2d82a4ddc30f3a16e2a93ed7f374119768d60f98bbd67a6d9a4f7377d79"
SAMPLE_LINK_RE=re.compile(r'href=["\'](/(?:[0-9]{6}-[a-z0-9]+)(?:/[^"\']*)?)["\']',re.I)
TITLE_RE=re.compile(r"<title[^>]*>(.*?)</title>",re.I|re.S)

def fetch(url,timeout=20):
    q=Request(url,headers={"User-Agent":UA,"Accept":"text/html,application/xhtml+xml"})
    try:
        with urlopen(q,timeout=timeout) as r:
            raw=r.read(4*1024*1024+1)
            status=int(getattr(r,"status",200))
            final=r.geturl()
            ctype=r.headers.get("Content-Type","")
        if len(raw)>4*1024*1024: raw=raw[:4*1024*1024]
        body=raw.decode("utf-8","replace")
        return {"status":status,"final_url":final,"content_type":ctype,"body":body,"error":None}
    except HTTPError as e:
        try: raw=e.read(512*1024)
        except Exception: raw=b""
        return {"status":int(e.code),"final_url":url,"content_type":e.headers.get("Content-Type","") if e.headers else "","body":raw.decode("utf-8","replace"),"error":None}
    except Exception as e:
        return {"status":None,"final_url":url,"content_type":"","body":"","error":f"{type(e).__name__}: {e}"}

def title(body):
    m=TITLE_RE.search(body)
    return re.sub(r"\s+"," ",html.unescape(m.group(1))).strip() if m else ""

def triage_probe(sha,timeout):
    url="https://tria.ge/s?q="+quote("sha256:"+sha,safe="")
    r=fetch(url,timeout); body=html.unescape(r["body"])
    links=sorted(set(SAMPLE_LINK_RE.findall(body)))
    markers={
        "exact_hash_in_body":sha.lower() in body.lower(),
        "sample_links":links[:20],
        "reported_token":"Reported" in body,
        "sample_id_header":"Sample ID" in body,
        "no_results_token":any(x in body.lower() for x in ["no results","no reports","nothing found"]),
        "title":title(body),"body_length":len(body),
    }
    return url,r,markers

def hybrid_probe(sha,timeout):
    url="https://www.hybrid-analysis.com/sample/"+sha
    r=fetch(url,timeout); body=html.unescape(r["body"]); low=body.lower()
    markers={
        "exact_hash_in_body":sha.lower() in low,
        "title":title(body),"body_length":len(body),
        "antibot_marker":any(x in low for x in ["cloudflare","captcha","access denied","just a moment","cf-chl"]),
        "analysis_title":"viewing online file analysis results for" in low,
        "file_details":"file details" in low,
        "sha256_label":"sha256" in low,
        "pdb_pathway":"pdb pathway" in low,
        "no_result_marker":any(x in low for x in ["not found","no analysis","no result"]),
    }
    return url,r,markers

def main():
    ap=argparse.ArgumentParser()
    ap.add_argument("--out",type=Path,required=True)
    ap.add_argument("--delay",type=float,default=.35)
    ap.add_argument("--timeout",type=float,default=20)
    a=ap.parse_args(); a.out.mkdir(parents=True,exist_ok=True)

    # Controls prove whether the public HTML response can actually distinguish a known-positive item.
    tu,tr,tm=triage_probe(TRIAGE_POSITIVE_CONTROL,a.timeout)
    hu,hr,hm=hybrid_probe(HYBRID_POSITIVE_CONTROL,a.timeout)
    triage_control_ok=bool(tm["sample_links"] and tm["reported_token"])
    hybrid_control_ok=bool(hm["exact_hash_in_body"] and "score " in hm["title"].lower() and "/100" in hm["title"].lower())

    controls={
        "tria_ge":{"sha256":TRIAGE_POSITIVE_CONTROL,"url":tu,"status":tr["status"],"final_url":tr["final_url"],"error":tr["error"],**tm,"control_ok":triage_control_ok},
        "hybrid_analysis":{"sha256":HYBRID_POSITIVE_CONTROL,"url":hu,"status":hr["status"],"final_url":hr["final_url"],"error":hr["error"],**hm,"control_ok":hybrid_control_ok},
    }
    time.sleep(a.delay)

    rows=[]
    for gen,module,sha,pdb,identity in RECORDS:
        tu,tr,tm=triage_probe(sha,a.timeout)
        if not triage_control_ok:
            tres="SURFACE_UNCALIBRATED"
        elif tm["sample_links"] and tm["reported_token"]:
            tres="MATCH_PAGE"
        elif tr["status"]==200:
            tres="NO_MATCH_VISIBLE"
        else:
            tres="ERROR_OR_BLOCKED"
        rows.append({"generation":gen,"module":module,"sha256":sha,"pdb":pdb,"codeview_identity":identity,
                     "surface":"tria.ge","query_url":tu,"http_status":tr["status"],"final_url":tr["final_url"],
                     "result":tres,"body_has_pdb_name":pdb.lower() in tr["body"].lower(),
                     "body_has_codeview_identity":identity.lower() in tr["body"].lower(),"error":tr["error"],**tm})
        time.sleep(a.delay)

        hu,hr,hm=hybrid_probe(sha,a.timeout)
        if not hybrid_control_ok:
            hres="SURFACE_UNCALIBRATED"
        elif hm["analysis_title"] and hm["file_details"] and hm["sha256_label"] and hm["exact_hash_in_body"]:
            hres="MATCH_PAGE"
        elif hr["status"] in (404,410) or hm["no_result_marker"]:
            hres="NO_PUBLIC_SAMPLE_PAGE"
        elif hm["antibot_marker"]:
            hres="ANTIBOT_OR_GATE"
        else:
            hres="NO_MATCH_VISIBLE"
        rows.append({"generation":gen,"module":module,"sha256":sha,"pdb":pdb,"codeview_identity":identity,
                     "surface":"hybrid-analysis.com","query_url":hu,"http_status":hr["status"],"final_url":hr["final_url"],
                     "result":hres,"body_has_pdb_name":pdb.lower() in hr["body"].lower(),
                     "body_has_codeview_identity":identity.lower() in hr["body"].lower(),"error":hr["error"],**hm})
        time.sleep(a.delay)

    positives=[r for r in rows if r["result"]=="MATCH_PAGE"]
    summary={
        "schema":"pub-symbol-sandbox-hash-probe.v2",
        "exact_module_builds":len(RECORDS),
        "surface_queries":len(rows),
        "controls":controls,
        "calibrated_surfaces":{"tria.ge":triage_control_ok,"hybrid-analysis.com":hybrid_control_ok},
        "positive_rows":len(positives),
        "positive_builds":len({r["sha256"] for r in positives}),
        "tria_ge_matches":sum(r["surface"]=="tria.ge" and r["result"]=="MATCH_PAGE" for r in rows),
        "hybrid_analysis_matches":sum(r["surface"]=="hybrid-analysis.com" and r["result"]=="MATCH_PAGE" for r in rows),
        "no_match_visible":sum(r["result"] in ("NO_MATCH_VISIBLE","NO_PUBLIC_SAMPLE_PAGE") for r in rows),
        "uncalibrated_rows":sum(r["result"]=="SURFACE_UNCALIBRATED" for r in rows),
        "errors_or_blocked":sum(r["result"] in ("ERROR_OR_BLOCKED","ANTIBOT_OR_GATE") for r in rows),
        "boundary":"Public locator/metadata probe only; positive controls required before any surface result is treated as evidence. No sample or memory-dump bytes downloaded.",
    }
    (a.out/"rows.json").write_text(json.dumps(rows,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
    (a.out/"summary.json").write_text(json.dumps(summary,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
    print(json.dumps(summary,indent=2))
    for r in rows:
        print(r["surface"],r["generation"],r["module"],r["sha256"][:16],r["http_status"],r["result"],
              ",".join(r.get("sample_links",[])[:2]),r.get("title","")[:80])
if __name__=="__main__":
    raise SystemExit(main())
