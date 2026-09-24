#!/usr/bin/env python3
"""Source-free EOT carrier census for the pinned Publisher 2000 X04 corpus."""

from __future__ import annotations
import argparse, hashlib, json, struct, sys, unittest
from pathlib import Path
import olefile

M22=b"\xE8\xAC\x22\x00"; M2C=b"\xE8\xAC\x2C\x00"
EOT_MAGIC=0x504C
VERS={0x00010000:1,0x00020001:2,0x00020002:3}
MIN_META=82

class ParseError(Exception): pass

def sha_bytes(b): return hashlib.sha256(b).hexdigest()
def sha_path(p):
    h=hashlib.sha256()
    with p.open("rb") as f:
        for c in iter(lambda:f.read(1024*1024),b""): h.update(c)
    return h.hexdigest()

class Cur:
    def __init__(self,b,pos,limit): self.b=b; self.pos=pos; self.limit=limit
    def take(self,n):
        end=self.pos+n
        if n<0 or end>self.limit or end>len(self.b): raise ParseError("bounded read exceeded metadata")
        out=self.b[self.pos:end]; self.pos=end; return out
    def skip(self,n): self.take(n)
    def u8(self): return self.take(1)[0]
    def u16(self): return struct.unpack("<H",self.take(2))[0]
    def u32(self): return struct.unpack("<I",self.take(4))[0]
    def s16(self):
        n=self.u16()
        if n%2: raise ParseError("odd UTF-16 byte length")
        return self.take(n).decode("utf-16le",errors="replace").rstrip("\0")
    def barr(self): self.skip(self.u32())

def body(b,mlen,v):
    c=Cur(b,12,mlen)
    flags=c.u32(); c.skip(10); charset=c.u8(); italic=c.u8()!=0
    weight=c.u32(); permissions=c.u16()
    if c.u16()!=EOT_MAGIC: raise ParseError("bad EOT magic")
    c.skip(16); c.skip(8); c.skip(4); c.skip(18)
    family=c.s16(); c.skip(2); style=c.s16(); c.skip(2)
    version=c.s16(); c.skip(2); full=c.s16(); root=""
    if v!=1:
        c.skip(2); root=c.s16()
        if v==3:
            c.u32(); c.u32(); c.skip(2); c.skip(c.u16()); c.u32(); c.barr()
    if c.pos!=mlen: raise ParseError("metadata boundary mismatch")
    return dict(flags=flags,charset=charset,italic=italic,weight=weight,permissions=permissions,
                family_name=family,style_name=style,version_name=version,full_name=full,root_string=root)

def parse_eot(b,off):
    r=b[off:]
    if len(r)<MIN_META: return None
    try: total,fontsz,magic=struct.unpack_from("<III",r,0)
    except struct.error: return None
    cv=VERS.get(magic)
    if cv is None or total>len(r) or fontsz==0 or fontsz>=total: return None
    mlen=total-fontsz
    if mlen<MIN_META or mlen>total: return None
    try:
        if struct.unpack_from("<H",r,34)[0]!=EOT_MAGIC: return None
    except struct.error: return None
    for pv in [cv]+[x for x in (1,2,3) if x!=cv]:
        try: d=body(r,mlen,pv)
        except (ParseError,struct.error): continue
        return dict(total_size=total,font_data_size=fontsz,metadata_size=mlen,
                    coded_version=cv,parsed_version=pv,bad_version=cv!=pv,**d)
    return None

def scan_stream(name,b):
    if len(b)<MIN_META: return []
    offs=set()
    for m in VERS:
        needle=struct.pack("<I",m); pos=b.find(needle)
        while pos>=0:
            if pos>=8: offs.add(pos-8)
            pos=b.find(needle,pos+1)
    sh=sha_bytes(b); out=[]
    for off in sorted(offs):
        if off+36>len(b): continue
        if struct.unpack_from("<H",b,off+34)[0]!=EOT_MAGIC: continue
        p=parse_eot(b,off)
        if p: out.append(dict(stream_path=name,stream_sha256=sh,offset=off,**p))
    return out

def family(contents):
    m=contents[:4]
    if m==M22: return "0x22"
    if m==M2C: return "0x2c"
    raise ParseError("unsupported Contents magic "+m.hex())

def scan_pub(path,rel):
    ph=sha_path(path)
    try: ole=olefile.OleFileIO(str(path))
    except Exception as e: raise ParseError("invalid_cfb:"+type(e).__name__) from e
    try:
        streams=ole.listdir(streams=True,storages=False)
        cp=next((x for x in streams if x==["Contents"]),None)
        if cp is None: raise ParseError("missing_contents")
        contents=ole.openstream(cp).read(); fam=family(contents)
        total=0; hits=[]
        for parts in streams:
            b=ole.openstream(parts).read(); total+=len(b)
            hits.extend(scan_stream("/"+"/".join(parts),b))
    finally: ole.close()
    prefix="legacy_0x22" if fam=="0x22" else "mature_0x2c"
    return dict(relative_path=rel,pub_sha256=ph,contents_family=fam,contents_len=len(contents),
                cfb_stream_count=len(streams),scanned_stream_bytes=total,eot_candidate_count=len(hits),
                eot_candidates=hits,verdict=prefix+("_with_validated_eot" if hits else "_without_validated_eot"))

def classify(root,out):
    pubs=sorted(p for p in root.rglob("*") if p.is_file() and p.suffix.lower()==".pub")
    if not pubs: raise SystemExit("no PUB candidates recovered")
    reports=[]; skipped=[]
    for p in pubs:
        rel=p.relative_to(root).as_posix()
        try:
            r=scan_pub(p,rel); reports.append(r)
            print("scanned",rel,r["contents_family"],r["eot_candidate_count"],r["pub_sha256"])
        except Exception as e:
            skipped.append(dict(relative_path=rel,size=p.stat().st_size,pub_sha256=sha_path(p),error=str(e)[:240]))
            print("skip",rel,e,file=sys.stderr)
    if not reports: raise SystemExit("every recovered PUB was unparseable")
    legacy=[r for r in reports if r["contents_family"]=="0x22"]
    mature=[r for r in reports if r["contents_family"]=="0x2c"]
    with_eot=[r for r in reports if r["eot_candidate_count"]]
    legacy_eot=[r for r in legacy if r["eot_candidate_count"]]
    ev=[]; files=[]
    for r in reports:
        files.append({k:v for k,v in r.items() if k!="eot_candidates"})
        for h in r["eot_candidates"]:
            ev.append(dict(relative_path=r["relative_path"],pub_sha256=r["pub_sha256"],
                contents_family=r["contents_family"],**h))
    result=dict(
        schema="pub-eot-x04-census.v1",input_class="pinned_public_publisher2000_x04_transport",
        raw_payload_published=False,recovered_pub_candidates=len(pubs),scanned_valid_pubs=len(reports),
        skipped_unparseable_pubs=len(skipped),legacy_0x22_reports=len(legacy),mature_0x2c_reports=len(mature),
        reports_with_validated_eot=len(with_eot),legacy_0x22_with_validated_eot=len(legacy_eot),
        files=files,skipped=skipped,validated_eot_evidence=ev,
        interpretation=("Static byte-level census only. A 0x22+validated-EOT hit localizes an embedded-font "
          "carrier candidate but does not identify the owning legacy record. A zero-hit result is bounded "
          "to this exact X04 corpus and is not a family-wide absence claim."),
        guardrails=[
          "Contents family uses only exact E8 AC 22 00 / E8 AC 2C 00 magic.",
          "EOT hits require coherent sizes, fixed LP magic, bounded UTF-16 metadata and exact metadata/font-data boundary.",
          "Malformed/non-CFB PUB members are retained only as hash/size/error receipts and do not abort the census.",
          "No raw PUB, EOT or font bytes are written to public evidence."
        ])
    out.parent.mkdir(parents=True,exist_ok=True)
    out.write_text(json.dumps(result,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
    lines=["# EOT-LEGACY-01 X04 census","",
      f"- recovered PUB candidates: {len(pubs)}",f"- scanned valid PUBs: {len(reports)}",
      f"- skipped malformed/non-CFB PUBs: {len(skipped)}",f"- 0x22 family: {len(legacy)}",
      f"- 0x2C family: {len(mature)}",f"- reports with validated EOT: {len(with_eot)}",
      f"- 0x22 + validated EOT: {len(legacy_eot)}","",
      "| Recovered path | SHA-256 | Family | EOT hits | Verdict |",
      "| --- | --- | --- | ---: | --- |"]
    for r in reports:
        lines.append(f"| `{r['relative_path']}` | `{r['pub_sha256']}` | {r['contents_family']} | {r['eot_candidate_count']} | {r['verdict']} |")
    if skipped:
        lines += ["","## Skipped inputs",""]
        for r in skipped: lines.append(f"- `{r['relative_path']}` - {r['size']} B - `{r['pub_sha256']}` - `{r['error']}`")
    lines += ["","Raw publication/font bytes are intentionally absent.",""]
    out.with_suffix(".md").write_text("\n".join(lines),encoding="utf-8")
    if not legacy: raise SystemExit("no valid 0x22-family PUB in pinned X04 corpus")
    return result

def pu16(o,x): o.extend(struct.pack("<H",x))
def pu32(o,x): o.extend(struct.pack("<I",x))
def pstr(o,s):
    b=s.encode("utf-16le"); pu16(o,len(b)); o.extend(b)

def synth(v):
    o=bytearray(); pu32(o,5); o.extend(b"\0"*10); o.extend(bytes([1,0])); pu32(o,400); pu16(o,8); pu16(o,EOT_MAGIC)
    o.extend(b"\0"*(16+8+4+18)); pstr(o,"Fixture Font"); pu16(o,0); pstr(o,"Regular"); pu16(o,0)
    pstr(o,"Version 1.0"); pu16(o,0); pstr(o,"Fixture Font Regular")
    if v!=1:
        pu16(o,0); pstr(o,"")
        if v==3: pu32(o,0); pu32(o,0); pu16(o,0); pu16(o,0); pu32(o,0); pu32(o,0)
    fd=b"\xA5"*64; magic={1:0x00010000,2:0x00020001,3:0x00020002}[v]
    return struct.pack("<III",12+len(o)+len(fd),len(fd),magic)+o+fd

class Tests(unittest.TestCase):
    def test_versions(self):
        for v in (1,2,3):
            p=parse_eot(synth(v),0); self.assertIsNotNone(p); self.assertEqual(p["parsed_version"],v)
            self.assertEqual(p["family_name"],"Fixture Font"); self.assertEqual(p["permissions"],8)
    def test_nonzero(self):
        h=scan_stream("/Contents",b"\xCC"*137+synth(2)+b"\xDD"*31); self.assertEqual(len(h),1); self.assertEqual(h[0]["offset"],137)
    def test_bad_size(self):
        b=bytearray(b"\0"*(MIN_META+32)); struct.pack_into("<III",b,0,10000,64,0x00020001); struct.pack_into("<H",b,34,EOT_MAGIC)
        self.assertIsNone(parse_eot(bytes(b),0))
    def test_truncated_string(self):
        b=bytearray(synth(1)); struct.pack_into("<H",b,82,0x7FF0); self.assertIsNone(parse_eot(bytes(b),0))

def main():
    a=argparse.ArgumentParser(); a.add_argument("--pub-root",type=Path); a.add_argument("--out",type=Path); a.add_argument("--self-test",action="store_true")
    x=a.parse_args()
    if x.self_test:
        r=unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Tests)); return 0 if r.wasSuccessful() else 1
    if x.pub_root is None or x.out is None: a.error("--pub-root and --out required")
    r=classify(x.pub_root,x.out)
    print(json.dumps(dict(recovered=r["recovered_pub_candidates"],scanned=r["scanned_valid_pubs"],
      skipped=r["skipped_unparseable_pubs"],legacy_0x22=r["legacy_0x22_reports"],
      legacy_0x22_with_eot=r["legacy_0x22_with_validated_eot"])))
    return 0

if __name__=="__main__": raise SystemExit(main())
