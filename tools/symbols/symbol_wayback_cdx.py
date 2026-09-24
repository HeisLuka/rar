#!/usr/bin/env python3
from __future__ import annotations
import argparse, csv, json, time
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode
from urllib.request import Request, urlopen

CDX="https://web.archive.org/cdx/search/cdx"
UA="rar-pub-symbol-archive/1.0 (public format research)"

RECORDS=[
("2002","MORPH9.DLL","MORPH9O.pdb","3F8AF2EE2"),
("2002","MSPUB.EXE","MSPUBO.pdb","3F8AF3342"),
("2002","PRTF9.DLL","PRTF9O.PDB","3F8AF3592"),
("2002","PTXT9.DLL","PTXT9O.pdb","3F8AF3752"),
("2002","PUBCONV.DLL","PUBCONVO.pdb","3F8AF3842"),
("2007","MSPUB.EXE","mspub.pdb","7BC6715E6CA54F4D919A854C04FC50272"),
("2007","MORPH9.DLL","morph9.pdb","9BADA37C83444F8F9BF6DE5D2D333C412"),
("2007","PTXT9.DLL","ptxt9.pdb","9DF153BF514D498088B2AF1A8195F27E2"),
("2007","PUBCONV.DLL","pubconv.pdb","BB1E1E0A09C1486FB9C8D46B8F5B0BA62"),
("2007","PRTF9.DLL","prtf9.pdb","392A7333A8204196AB9D35DD963F745C2"),
("2007","PUBTRAP.DLL","pubtrap.pdb","1A5FA7772552459EBC189CBE9BD98EE42"),
("2010","MORPH9.DLL","morph9.pdb","B303D59B819D4611BCC69B5879EF9CCF2"),
("2010","MSPUB.EXE","mspub.pdb","522366D4280345B3A2EA541F6319BDF92"),
("2010","PRTF9.DLL","prtf9.pdb","D830EA40E51C413E89AF70CC3AD977CB2"),
("2010","PTXT9.DLL","ptxt9.pdb","65AD9C1BB315400B9A532ACEAB33D08F2"),
("2010","PUBCONV.DLL","pubconv.pdb","8B1478BAE1EC4C708402D366487128C12"),
("2010","PUBTRAP.DLL","pubtrap.pdb","CD69A68F28E64CA58B5B6B1EEE8545362"),
("2013","MSPUB.EXE","mspub.pdb","E9B57F7F314C4F3AA6DEC419D691F7582"),
("2013","MORPH9.DLL","morph9.pdb","B3679184181A41A3A0D38AF8345AA06C2"),
("2013","PTXT9.DLL","ptxt9.pdb","72032A2A6C7E44E8B28BCB4A11F5A09A2"),
("2013","PUBCONV.DLL","pubconv.pdb","92FFE1DDF9E744059E938256D15FACE22"),
("2013","PRTF9.DLL","prtf9.pdb","0869E260B8E54108AD377B8C645E65F62"),
("2013","PUBTRAP.DLL","pubtrap.pdb","D6E975F58E5F47EF871E7E02B39B103B2"),
("2016","MSPUB.EXE","mspub.pdb","44CB6B28DB744E3CB07C49BEF3E770322"),
("2016","MORPH9.DLL","morph9.pdb","C612D21409CD42759BFF296A215935DC2"),
("2016","PTXT9.DLL","ptxt9.pdb","65217074C1424306A6D6FC57EED1FB5B2"),
("2016","PUBCONV.DLL","pubconv.pdb","7B68ADABF0034D079D0C7003D36390E62"),
("2016","PRTF9.DLL","prtf9.pdb","8A0C3C03022A4827B2951F3AD87B54082"),
]

RETRY_IDS={
"8B1478BAE1EC4C708402D366487128C12",
"CD69A68F28E64CA58B5B6B1EEE8545362",
"E9B57F7F314C4F3AA6DEC419D691F7582",
"B3679184181A41A3A0D38AF8345AA06C2",
"72032A2A6C7E44E8B28BCB4A11F5A09A2",
"92FFE1DDF9E744059E938256D15FACE22",
"0869E260B8E54108AD377B8C645E65F62",
"D6E975F58E5F47EF871E7E02B39B103B2",
"44CB6B28DB744E3CB07C49BEF3E770322",
"C612D21409CD42759BFF296A215935DC2",
}

def req(params, timeout=12, retries=0):
    u=CDX+"?"+urlencode(params)
    last=None
    for i in range(retries+1):
        try:
            q=Request(u,headers={"User-Agent":UA,"Accept":"application/json"})
            with urlopen(q,timeout=timeout) as r:
                b=r.read(16*1024*1024+1)
            if len(b)>16*1024*1024: raise ValueError("response>16MiB")
            d=json.loads(b.decode("utf-8"))
            if not d: return [],u
            hdr=d[0]
            return [{str(k):str(v) for k,v in zip(hdr,row)} for row in d[1:] if isinstance(row,list)],u
        except (HTTPError,URLError,TimeoutError,OSError,ValueError,json.JSONDecodeError) as e:
            last=e
            if i<retries:
                time.sleep(1.5*(i+1)); continue
            raise
    raise last

def variants(pdb):
    vals=[]
    for x in (pdb,pdb.lower(),pdb.upper()):
        if x not in vals: vals.append(x)
    return vals

def main():
    ap=argparse.ArgumentParser()
    ap.add_argument("--out",type=Path,required=True)
    ap.add_argument("--delay",type=float,default=.25)
    ap.add_argument("--limit",type=int,default=50)
    ap.add_argument("--timeout",type=float,default=12)
    ap.add_argument("--retries",type=int,default=0)
    ap.add_argument("--retry-errors-only",action="store_true")
    a=ap.parse_args()
    records=[r for r in RECORDS if (not a.retry_errors_only or r[3] in RETRY_IDS)]
    rows=[]; receipts=[]; errors=[]
    for gen,module,pdb,index in records:
        for scheme in ("http",):
            for pdbname in [pdb]:
                prefix=f"{scheme}://msdl.microsoft.com/download/symbols/{pdbname}/{index}/"
                params=[
                    ("url",prefix),("matchType","prefix"),("output","json"),
                    ("fl","timestamp,original,mimetype,statuscode,digest,length"),
                    ("collapse","digest"),("limit",str(a.limit)),("gzip","false")
                ]
                try:
                    found,qurl=req(params,a.timeout,a.retries)
                    receipts.append({"generation":gen,"module":module,"pdb":pdb,"identity":index,"query_prefix":prefix,"query_url":qurl,"status":"ok","captures":len(found)})
                    for r in found:
                        rows.append({"generation":gen,"module":module,"pdb":pdb,"identity":index,"query_prefix":prefix,**r})
                except Exception as e:
                    errors.append({"generation":gen,"module":module,"pdb":pdb,"identity":index,"query_prefix":prefix,"error":f"{type(e).__name__}: {e}"})
                time.sleep(a.delay)
    # Dedup capture rows by identity+original+timestamp+digest.
    ded={}
    for r in rows:
        k=(r["identity"],r.get("original",""),r.get("timestamp",""),r.get("digest",""))
        ded[k]=r
    rows=sorted(ded.values(),key=lambda r:(r["generation"],r["module"],r.get("timestamp",""),r.get("original","")))
    a.out.mkdir(parents=True,exist_ok=True)
    (a.out/"captures.json").write_text(json.dumps(rows,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
    (a.out/"query-receipts.json").write_text(json.dumps(receipts,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
    (a.out/"errors.json").write_text(json.dumps(errors,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
    with (a.out/"captures.tsv").open("w",encoding="utf-8",newline="") as f:
        w=csv.writer(f,delimiter="\t")
        w.writerow(["generation","module","pdb","identity","timestamp","statuscode","mimetype","length","digest","original","query_prefix"])
        for r in rows:
            w.writerow([r.get(k,"") for k in ("generation","module","pdb","identity","timestamp","statuscode","mimetype","length","digest","original","query_prefix")])
    hitids=sorted({r["identity"] for r in rows})
    summary={
        "schema":"pub-symbol-wayback-cdx.v1",
        "exact_codeview_identities":len(records),
        "query_count":len(receipts)+len(errors),
        "successful_queries":len(receipts),
        "error_queries":len(errors),
        "capture_rows":len(rows),
        "identities_with_capture":len(hitids),
        "identities_without_capture":len(records)-len(hitids),
        "hit_identities":hitids,
        "boundary":"CDX locator census only; no PDB bytes downloaded or published",
    }
    (a.out/"summary.json").write_text(json.dumps(summary,indent=2)+"\n",encoding="utf-8")
    print(json.dumps(summary,indent=2))
    if rows:
        print("CAPTURES")
        for r in rows[:100]:
            print(r["generation"],r["module"],r["identity"],r.get("timestamp"),r.get("statuscode"),r.get("original"))
    return 0
if __name__=="__main__": raise SystemExit(main())
