#!/usr/bin/env python3
"""Exact MSPUB14 descriptor-table pointer topology probe for PUB-T-451 / U-OBJ-01.

Fail-closed route:
1) recover the exact 169-class Microsoft OPL descriptor registry;
2) treat class-header/table VAs as calibration anchors;
3) find data structures that actually point to many class tables;
4) localize code references to those registry containers and to OplPluo/OplUo entries.

No semantic expansion from class/property names.
"""
from __future__ import annotations
import argparse, hashlib, json, re, struct
from collections import defaultdict, Counter
from pathlib import Path
import capstone, pefile
from capstone.x86 import *

EXPECTED="27c00f7f06957f24d392f9c61fbd3b40282f15dc595caf66785b561f559d7b97"
ROW=18
SENT=0x07FF

def sha(p):
    h=hashlib.sha256()
    with open(p,"rb") as f:
        for b in iter(lambda:f.read(1<<20),b""):h.update(b)
    return h.hexdigest()

class V:
    def __init__(self,p):
        self.path=Path(p);self.b=self.path.read_bytes();self.pe=pefile.PE(str(p));self.base=self.pe.OPTIONAL_HEADER.ImageBase
        self.s=[]
        for x in self.pe.sections:
            n=x.Name.rstrip(b"\0").decode("ascii","replace")
            self.s.append(dict(name=n,rva=x.VirtualAddress,va=self.base+x.VirtualAddress,raw=x.PointerToRawData,
                               rs=x.SizeOfRawData,vs=x.Misc_VirtualSize,data=self.b[x.PointerToRawData:x.PointerToRawData+x.SizeOfRawData]))
    def u16(self,o):return struct.unpack_from("<H",self.b,o)[0]
    def u32(self,o):return struct.unpack_from("<I",self.b,o)[0]
    def va2off(self,va):
        r=va-self.base
        for s in self.s:
            if s["rva"]<=r<s["rva"]+max(s["rs"],s["vs"]):
                d=r-s["rva"];return None if d>=s["rs"] else s["raw"]+d
    def off2va(self,o):
        for s in self.s:
            if s["raw"]<=o<s["raw"]+s["rs"]:return s["va"]+(o-s["raw"])
    def sec(self,va):
        if va is None:return None
        r=va-self.base
        for s in self.s:
            if s["rva"]<=r<s["rva"]+max(s["rs"],s["vs"]):return s["name"]
    def ascii(self,va,lim=160):
        o=self.va2off(va)
        if o is None:return None
        z=self.b[o:o+lim].split(b"\0",1)[0]
        try:s=z.decode("ascii")
        except:return None
        return s if len(s)>1 and all(0x20<=ord(c)<=0x7e for c in s) else None
    def utf16(self,va,lim=160):
        o=self.va2off(va)
        if o is None:return None
        z=bytearray()
        for p in range(o,min(len(self.b)-1,o+lim*2),2):
            q=self.b[p:p+2]
            if q==b"\0\0":break
            z.extend(q)
        try:s=z.decode("utf-16le")
        except:return None
        return s if len(s)>1 and all(0x20<=ord(c)<=0x7e for c in s) else None

IDENT=re.compile(r"^[A-Za-z_?][A-Za-z0-9_.$?@:+-]{1,126}$")
def name_at(v,va):
    for s in (v.utf16(va),v.ascii(va)):
        if s and IDENT.fullmatch(s):return s
    return None

def scan_tables(v):
    cand=[]
    for o in range(0,len(v.b)-ROW):
        if v.u16(o+4)!=SENT:continue
        cn=name_at(v,v.u32(o))
        if not cn or not cn.startswith("Opl"):continue
        rows=[];p=o+ROW;prev=-1
        while p+ROW<=len(v.b) and len(rows)<2048:
            fid=v.u16(p+4);nm=name_at(v,v.u32(p))
            if fid==SENT and nm and nm.startswith("Opl"):break
            if fid>0x7fe or fid<=prev or not nm:break
            rows.append(dict(field_id=fid,field=nm,row_va=v.off2va(p),aux=v.u32(p+8),meta=v.u16(p+12),flags=v.u16(p+16)))
            prev=fid;p+=ROW
        if rows:cand.append(dict(class_name=cn,header_va=v.off2va(o),header_off=o,header_aux=v.u32(o+8),header_meta=v.u16(o+12),header_flags=v.u16(o+16),rows=rows))
    best={}
    for t in cand:
        if t["class_name"] not in best or len(t["rows"])>len(best[t["class_name"]]["rows"]):best[t["class_name"]]=t
    return best

def disasm(v):
    md=capstone.Cs(capstone.CS_ARCH_X86,capstone.CS_MODE_32);md.detail=True;md.skipdata=True
    out=[]
    for s in v.s:
        if s["name"]!=".text":continue
        for x in md.disasm(s["data"],s["va"]):
            if x.mnemonic==".byte":continue
            refs=set();call=None
            for op in x.operands:
                if op.type==X86_OP_IMM:
                    q=op.imm&0xffffffff;refs.add(q)
                    if x.mnemonic=="call":call=q
                elif op.type==X86_OP_MEM and op.mem.base==0 and op.mem.index==0:
                    refs.add(op.mem.disp&0xffffffff)
            out.append(dict(a=x.address,m=x.mnemonic,s=x.op_str,refs=refs,call=call))
    return out

def slim(x):return dict(address=f"0x{x['a']:08X}",mnemonic=x["m"],op_str=x["s"])

def find_data_refs(v,header_map):
    by_va={t["header_va"]:cn for cn,t in header_map.items()}
    hits=[]
    # only aligned dwords in mapped raw sections, excluding .text
    for s in v.s:
        if s["name"]==".text":continue
        start=s["raw"];end=start+s["rs"]
        for o in range(start,end-3,4):
            q=v.u32(o)
            cn=by_va.get(q)
            if cn:
                va=v.off2va(o)
                hits.append(dict(site_va=va,site_off=o,site_section=s["name"],target_va=q,class_name=cn))
    return hits

def cluster_hits(hits,window=0x200):
    hs=sorted(hits,key=lambda x:x["site_va"])
    out=[]
    for i,h in enumerate(hs):
        lo=h["site_va"];hi=lo+window
        rows=[]
        j=i
        while j<len(hs) and hs[j]["site_va"]<hi:
            rows.append(hs[j]);j+=1
        uniq=sorted({x["class_name"] for x in rows})
        if len(uniq)>=4:
            out.append(dict(start_va=lo,end_va=hi,pointer_count=len(rows),unique_classes=len(uniq),
                            classes=uniq[:200],sites=[dict(site_va=f"0x{x['site_va']:08X}",class_name=x["class_name"],target_va=f"0x{x['target_va']:08X}") for x in rows]))
    # de-duplicate near-identical sliding windows by keeping local maxima
    out.sort(key=lambda x:(-x["unique_classes"],-x["pointer_count"],x["start_va"]))
    kept=[]
    for x in out:
        if any(abs(x["start_va"]-y["start_va"])<0x80 and x["classes"]==y["classes"] for y in kept):continue
        kept.append(x)
        if len(kept)>=100:break
    return kept

def context_dwords(v,site_va,radius=0x40):
    o=v.va2off(site_va)
    if o is None:return []
    a=max(0,(o-radius)&~3);b=min(len(v.b)-3,o+radius)
    rows=[]
    for p in range(a,b+1,4):
        q=v.u32(p);va=v.off2va(p)
        rows.append(dict(site_va=None if va is None else f"0x{va:08X}",value=f"0x{q:08X}",section=v.sec(q),ascii=v.ascii(q),utf16=v.utf16(q)))
    return rows

def main():
    ap=argparse.ArgumentParser();ap.add_argument("exe",type=Path);ap.add_argument("--out",type=Path,required=True)
    a=ap.parse_args()
    h=sha(a.exe)
    if h!=EXPECTED:raise SystemExit(f"hash mismatch {h}")
    v=V(a.exe);T=scan_tables(v)
    print("classes",len(T))
    if len(T)!=169:raise SystemExit(f"expected 169 classes, got {len(T)}")
    for req in ("OplPluo","OplUo","OplMocd"):
        if req not in T:raise SystemExit(f"missing {req}")
    headers={cn:t["header_va"] for cn,t in T.items()}
    rev={va:cn for cn,va in headers.items()}
    print("OplPluo",hex(headers["OplPluo"]),"OplUo",hex(headers["OplUo"]),"OplMocd",hex(headers["OplMocd"]))

    D=find_data_refs(v,T)
    print("data refs",len(D),"unique targets",len({x["class_name"] for x in D}))
    C=cluster_hits(D)
    print("clusters",len(C))
    for x in C[:20]:print("CLUSTER",hex(x["start_va"]),x["unique_classes"],x["pointer_count"],x["classes"][:15])

    I=disasm(v)
    # direct code refs to individual class headers
    code_header_refs=[]
    for x in I:
        m=x["refs"]&set(rev)
        if m:
            code_header_refs.append(dict(ins=slim(x),targets=[dict(class_name=rev[q],header_va=f"0x{q:08X}") for q in sorted(m)]))
    print("code header refs",len(code_header_refs))
    for x in code_header_refs[:80]:print("CODEHDR",x)

    # OplPluo/Uo data pointer sites + context
    exact={}
    for cn in ("OplPluo","OplUo","OplMocd"):
        rows=[x for x in D if x["class_name"]==cn]
        exact[cn]=[]
        for r in rows:
            item=dict(site_va=f"0x{r['site_va']:08X}",site_section=r["site_section"],target_va=f"0x{r['target_va']:08X}",
                      dwords=context_dwords(v,r["site_va"]))
            # code refs to pointer site itself and nearby aligned bases (-0x40..0)
            refs=[]
            near_bases=[r["site_va"]-d for d in range(0,0x44,4)]
            for ins in I:
                m=ins["refs"]&set(near_bases)
                if m:refs.append(dict(ins=slim(ins),matched=[f"0x{q:08X}" for q in sorted(m)]))
            item["code_refs_to_site_or_near_base"]=refs[:200]
            exact[cn].append(item)
        print(cn,"data pointer sites",len(rows))
        for r in exact[cn]:print("EXACT",cn,r["site_va"],"coderefs",len(r["code_refs_to_site_or_near_base"]))

    # Registry cluster code refs: scan exact cluster starts and every pointer site in top clusters.
    cluster_code=[]
    for c in C[:40]:
        addrs={int(x["site_va"],16) for x in c["sites"]}
        # plus candidate aligned bases in 0x40 before first site
        addrs|={c["start_va"]-d for d in range(0,0x44,4)}
        refs=[]
        for ins in I:
            m=ins["refs"]&addrs
            if m:refs.append(dict(ins=slim(ins),matched=[f"0x{q:08X}" for q in sorted(m)]))
        cluster_code.append(dict(start_va=f"0x{c['start_va']:08X}",unique_classes=c["unique_classes"],pointer_count=c["pointer_count"],code_refs=refs[:300],classes=c["classes"]))
    cluster_code.sort(key=lambda x:(-len(x["code_refs"]),-x["unique_classes"]))

    # Header aux topology itself may be the root linkage.
    aux_counter=Counter()
    aux_classes=defaultdict(list)
    for cn,t in T.items():
        aux=t["header_aux"]
        aux_counter[aux]+=1;aux_classes[aux].append(cn)
    top_aux=[dict(aux_va=f"0x{k:08X}",count=n,section=v.sec(k),classes=aux_classes[k][:200]) for k,n in aux_counter.most_common(50)]
    print("top header aux")
    for x in top_aux[:20]:print(x)

    result={
      "schema":"mspub14-opl-registry-pointer-topology.v1",
      "sha256":h,
      "class_count":len(T),
      "headers":{cn:f"0x{va:08X}" for cn,va in headers.items()},
      "data_pointer_hits":D,
      "clusters":C,
      "code_header_refs":code_header_refs,
      "exact_targets":exact,
      "cluster_code_refs":cluster_code,
      "header_aux_histogram":top_aux,
      "boundary":"Static pointer topology only. A pointer/consumer relation is implementation evidence, not a user-facing semantic role."
    }
    a.out.parent.mkdir(parents=True,exist_ok=True)
    a.out.write_text(json.dumps(result,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
    summary={
      "class_count":len(T),
      "data_pointer_hits":len(D),
      "unique_header_targets_with_data_refs":len({x["class_name"] for x in D}),
      "cluster_count":len(C),
      "code_header_refs":len(code_header_refs),
      "exact_pointer_sites":{cn:len(exact[cn]) for cn in exact},
      "exact_sites_with_code_refs":{cn:sum(bool(x["code_refs_to_site_or_near_base"]) for x in exact[cn]) for cn in exact},
      "top_clusters":[{k:x[k] for k in ("start_va","unique_classes","pointer_count","classes")} for x in C[:20]],
      "top_cluster_code_refs":[{"start_va":x["start_va"],"unique_classes":x["unique_classes"],"pointer_count":x["pointer_count"],"code_ref_count":len(x["code_refs"]),"classes":x["classes"][:30]} for x in cluster_code[:30]],
      "top_header_aux":top_aux[:30],
    }
    a.out.with_name("summary.json").write_text(json.dumps(summary,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
    lines=[
      "PUB-T-451 exact OPL registry pointer topology",
      f"MSPUB SHA256: {h}",
      f"class tables: {len(T)}",
      f"data pointers to class headers: {len(D)} / unique class targets {summary['unique_header_targets_with_data_refs']}",
      f"dense pointer clusters: {len(C)}",
      f"direct code refs to class headers: {len(code_header_refs)}",
      f"OplPluo header={hex(headers['OplPluo'])} data_sites={len(exact['OplPluo'])} sites_with_code_refs={summary['exact_sites_with_code_refs']['OplPluo']}",
      f"OplUo header={hex(headers['OplUo'])} data_sites={len(exact['OplUo'])} sites_with_code_refs={summary['exact_sites_with_code_refs']['OplUo']}",
      f"OplMocd header={hex(headers['OplMocd'])} data_sites={len(exact['OplMocd'])} sites_with_code_refs={summary['exact_sites_with_code_refs']['OplMocd']}",
      "",
      "Top dense clusters:"
    ]
    for x in C[:20]:lines.append(f"  0x{x['start_va']:08X} classes={x['unique_classes']} ptrs={x['pointer_count']} sample={','.join(x['classes'][:10])}")
    lines.append("")
    lines.append("Top cluster code refs:")
    for x in cluster_code[:20]:lines.append(f"  {x['start_va']} classes={x['unique_classes']} ptrs={x['pointer_count']} code_refs={len(x['code_refs'])}")
    a.out.with_name("summary.txt").write_text("\n".join(lines)+"\n",encoding="utf-8")
    print("\n".join(lines))
if __name__=="__main__":main()
