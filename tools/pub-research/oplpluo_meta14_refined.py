#!/usr/bin/env python3
"""Refined exact MSPUB14 descriptor-row dispatch probe for PUB-T-451 / U-OBJ-01.

Looks for actual 18-byte descriptor-row access patterns, not generic immediate 0x14 noise:
- metadata at row+0x0C,
- aux descriptor at row+0x08,
- field id at row+0x04,
- flags/tail at row+0x10,
- stride 0x12,
- meta 0x14 dispatch.

Static implementation evidence only.
"""
from __future__ import annotations
import argparse, hashlib, json, re, struct
from collections import defaultdict
from pathlib import Path
import capstone, pefile
from capstone.x86 import *

EXPECTED="27c00f7f06957f24d392f9c61fbd3b40282f15dc595caf66785b561f559d7b97"
BASE=0x2E000000
ROW=0x12
META=0x14
RGAUX=0x2E694104
UOTABLE=0x2E694278
SHARED=0x2E5AA766

def sha(p):
    h=hashlib.sha256()
    with open(p,"rb") as f:
        for b in iter(lambda:f.read(1<<20),b""):h.update(b)
    return h.hexdigest()

class V:
    def __init__(self,p):
        self.b=Path(p).read_bytes(); self.pe=pefile.PE(str(p)); self.base=self.pe.OPTIONAL_HEADER.ImageBase; self.s=[]
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
            if s["raw"]<=o<s["raw"]+s["rs"]:return s["va"]+o-s["raw"]
    def sec(self,va):
        if va is None:return None
        r=va-self.base
        for s in self.s:
            if s["rva"]<=r<s["rva"]+max(s["rs"],s["vs"]):return s["name"]
    def utf16(self,va,limit=150):
        o=self.va2off(va)
        if o is None:return None
        z=bytearray()
        for p in range(o,min(len(self.b)-1,o+2*limit),2):
            q=self.b[p:p+2]
            if q==b"\0\0":break
            z.extend(q)
        try:s=z.decode("utf-16le")
        except:return None
        return s if len(s)>1 and all(0x20<=ord(c)<=0x7e for c in s) else None

def insns(v):
    md=capstone.Cs(capstone.CS_ARCH_X86,capstone.CS_MODE_32);md.detail=True;md.skipdata=True
    out=[]
    for s in v.s:
        if s["name"]!=".text":continue
        for x in md.disasm(s["data"],s["va"]):
            if x.mnemonic==".byte":continue
            ops=[]
            for op in x.operands:
                if op.type==X86_OP_IMM:
                    ops.append({"t":"i","v":op.imm&0xffffffff,"size":op.size})
                elif op.type==X86_OP_REG:
                    ops.append({"t":"r","v":op.reg,"name":x.reg_name(op.reg),"size":op.size})
                elif op.type==X86_OP_MEM:
                    ops.append({"t":"m","base":op.mem.base,"base_name":x.reg_name(op.mem.base) if op.mem.base else "",
                                "index":op.mem.index,"index_name":x.reg_name(op.mem.index) if op.mem.index else "",
                                "scale":op.mem.scale,"disp":op.mem.disp,"size":op.size})
            out.append({"a":x.address,"m":x.mnemonic,"s":x.op_str,"ops":ops})
    return out,{x["a"]:i for i,x in enumerate(out)}

def slim(x):return {"address":f"0x{x['a']:08X}","mnemonic":x["m"],"op_str":x["s"]}

def mems(x):return [o for o in x["ops"] if o["t"]=="m"]
def imms(x):return [o["v"] for o in x["ops"] if o["t"]=="i"]
def regs(x):return [o for o in x["ops"] if o["t"]=="r"]

def callto(x):
    if x["m"]!="call":return None
    z=imms(x);return z[0] if z else None

def start(I,i,back=0x600):
    here=I[i]["a"]
    for j in range(i,max(-1,i-500),-1):
        if here-I[j]["a"]>back:break
        if I[j]["m"]=="push" and I[j]["s"]=="ebp" and j+1<len(I) and I[j+1]["m"]=="mov" and I[j+1]["s"].replace(" ","")=="ebp,esp":
            return I[j]["a"]
    return None

def body(I,A,s,limit=500):
    i=A.get(s)
    if i is None:return []
    z=[]
    for x in I[i:i+limit]:
        if x["a"]-s>0xa00:break
        z.append(x)
        if x["m"].startswith("ret") and len(z)>5:break
    return z

def parse_class_tables(v):
    # Enough to establish actual descriptor addresses and meta14 rows.
    def name(va):return v.utf16(va)
    best={}
    for o in range(0,len(v.b)-18):
        if v.u16(o+4)!=0x7ff:continue
        cn=name(v.u32(o))
        if not cn or not cn.startswith("Opl"):continue
        rows=[];p=o+18;prev=-1
        while p+18<=len(v.b) and len(rows)<2048:
            fid=v.u16(p+4); nm=name(v.u32(p))
            if fid==0x7ff and nm and nm.startswith("Opl"):break
            if fid>0x7fe or fid<=prev or not nm:break
            rows.append({"class":cn,"field_id":fid,"field":nm,"row_va":v.off2va(p),"aux":v.u32(p+8),"meta":v.u16(p+12),"flags":v.u16(p+16)})
            prev=fid;p+=18
        if rows and (cn not in best or len(rows)>len(best[cn]["rows"])):best[cn]={"header_va":v.off2va(o),"rows":rows}
    return best

def meta_read_dispatches(I,A):
    hits=[]
    stack={X86_REG_EBP,X86_REG_ESP}
    for i,x in enumerate(I):
        # candidate read/compare of row+0x0C from non-stack base
        cands=[o for o in mems(x) if o["disp"]==0x0c and o["base"] not in stack and o["base"]!=0]
        if not cands:continue
        for mo in cands:
            base=mo["base"]; direct_cmp=(x["m"]=="cmp" and META in imms(x))
            dst=None
            if x["m"] in ("mov","movzx","movsx") and x["ops"] and x["ops"][0]["t"]=="r":dst=x["ops"][0]["v"]
            cmp_ins=None
            if direct_cmp:cmp_ins=x
            elif dst:
                for y in I[i+1:min(len(I),i+14)]:
                    if y["m"]=="cmp" and META in imms(y) and any(o["t"]=="r" and o["v"]==dst for o in y["ops"]):
                        cmp_ins=y;break
            if not cmp_ins:continue
            # same row-base aux read after/before dispatch
            local=I[max(0,i-12):min(len(I),i+55)]
            aux=[y for y in local if any(o["t"]=="m" and o["base"]==base and o["disp"]==8 for o in y["ops"])]
            fid=[y for y in local if any(o["t"]=="m" and o["base"]==base and o["disp"]==4 for o in y["ops"])]
            flg=[y for y in local if any(o["t"]=="m" and o["base"]==base and o["disp"]==0x10 for o in y["ops"])]
            stride=[y for y in local if ROW in imms(y)]
            fn=start(I,i)
            hits.append({"function":None if fn is None else f"0x{fn:08X}","meta_read":slim(x),"meta_size":mo["size"],"row_base":mo["base_name"],
                         "cmp14":slim(cmp_ins),"aux_same_base":[slim(y) for y in aux[:12]],"fieldid_same_base":[slim(y) for y in fid[:12]],
                         "flags_same_base":[slim(y) for y in flg[:12]],"stride18_near":[slim(y) for y in stride[:12]],
                         "context":[slim(y) for y in local]})
    return hits

def row_layout_functions(I,A):
    stack={X86_REG_EBP,X86_REG_ESP}
    seed=set()
    for i,x in enumerate(I):
        for o in mems(x):
            if o["base"] and o["base"] not in stack and o["disp"] in (0,4,8,12,16):
                s=start(I,i)
                if s:seed.add(s)
    out=[]
    for s in seed:
        B=body(I,A,s)
        disps=defaultdict(list); stride=[]; sent=[]; meta14=[]; calls=[]
        for x in B:
            for o in mems(x):
                if o["base"] and o["base"] not in stack and o["disp"] in (0,4,8,12,16):
                    disps[o["disp"]].append(slim(x))
            if ROW in imms(x):stride.append(slim(x))
            if 0x7ff in imms(x):sent.append(slim(x))
            if META in imms(x):meta14.append(slim(x))
            ct=callto(x)
            if ct is not None:calls.append(ct)
        coverage=sum(1 for d in (0,4,8,12,16) if disps[d])
        if coverage<3:continue
        score=coverage*8+min(len(stride),4)*10+min(len(sent),3)*8+min(len(meta14),5)*3
        if disps[8] and disps[12]:score+=12
        if coverage==5:score+=20
        out.append({"start":f"0x{s:08X}","score":score,"coverage":coverage,
                    "disp0":disps[0][:12],"disp4":disps[4][:12],"disp8":disps[8][:12],"disp12":disps[12][:12],"disp16":disps[16][:12],
                    "stride18":stride[:12],"sentinel7ff":sent[:12],"imm14":meta14[:12],
                    "calls":[f"0x{x:08X}" for x in sorted(set(calls))[:150]],"body":[slim(x) for x in B[:280]]})
    out.sort(key=lambda x:(-x["score"],x["start"]))
    return out

def main():
    ap=argparse.ArgumentParser();ap.add_argument("exe",type=Path);ap.add_argument("--out",type=Path,required=True)
    a=ap.parse_args()
    h=sha(a.exe)
    if h!=EXPECTED:raise SystemExit(f"hash mismatch {h}")
    v=V(a.exe)
    if v.base!=BASE:raise SystemExit("base mismatch")
    T=parse_class_tables(v)
    meta14=[r for t in T.values() for r in t["rows"] if r["meta"]==META]
    if len(meta14)!=43:raise SystemExit(f"meta14 count {len(meta14)}")
    pluo=T["OplPluo"];rguo=next(r for r in pluo["rows"] if r["field"]=="Rguo")
    if rguo["aux"]!=RGAUX:raise SystemExit("Rguo aux mismatch")
    I,A=insns(v)
    cmap=defaultdict(list)
    for x in I:
        ct=callto(x)
        if ct is not None:cmap[ct].append(x["a"])

    dispatch=meta_read_dispatches(I,A)
    layouts=row_layout_functions(I,A)
    layout_starts={int(x["start"],16) for x in layouts[:300]}

    # Helper parents and whether they call any high-scoring descriptor-layout function.
    helper_parents=defaultdict(list)
    for cs in cmap.get(SHARED,[]):
        i=A.get(cs)
        if i is None:continue
        s=start(I,i)
        if s:helper_parents[s].append(cs)
    profiles=[]
    for s,sites in helper_parents.items():
        B=body(I,A,s)
        calls={callto(x) for x in B if callto(x) is not None}
        overlaps=sorted(calls & layout_starts)
        # strong descriptor-ish accesses inside helper parent itself
        nonstack={X86_REG_EBP,X86_REG_ESP}
        disps=set()
        for x in B:
            for o in mems(x):
                if o["base"] and o["base"] not in nonstack and o["disp"] in (0,4,8,12,16):disps.add(o["disp"])
        rank=len(overlaps)*30+len(disps)*4+(10 if 0x59 in {z for x in B for z in imms(x)} else 0)
        profiles.append({"start":f"0x{s:08X}","rank":rank,"helper_callsites":[f"0x{x:08X}" for x in sites],
                         "row_layout_disps":sorted(disps),"calls_layout_candidates":[f"0x{x:08X}" for x in overlaps],
                         "body":[slim(x) for x in B[:300]]})
    profiles.sort(key=lambda x:(-x["rank"],x["start"]))

    # direct absolute code refs remain useful negative control
    direct=[]
    for x in I:
        vals=set(imms(x))
        for o in mems(x):
            if o["base"]==0 and o["index"]==0:vals.add(o["disp"]&0xffffffff)
        m=vals&{RGAUX,UOTABLE}
        if m:direct.append({"ins":slim(x),"targets":[f"0x{z:08X}" for z in sorted(m)]})

    out={"schema":"oplpluo-meta14-refined.v1","sha256":h,
         "calibration":{"meta14_rows":len(meta14),"rguo_aux":f"0x{RGAUX:08X}","opluo_table":f"0x{UOTABLE:08X}"},
         "meta14_dispatch_hits":dispatch,"row_layout_functions":layouts[:120],
         "shared_helper":{"direct_callsites":len(cmap.get(SHARED,[])),"unique_parent_functions":len(helper_parents),"profiles":profiles[:120]},
         "direct_rguo_aux_or_table_code_refs":direct,
         "boundary":"Static implementation evidence only; no feature/semantic expansion from names or metadata type."}
    a.out.parent.mkdir(parents=True,exist_ok=True)
    a.out.write_text(json.dumps(out,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
    summary={"meta14_rows":len(meta14),"meta14_dispatch_hits":len(dispatch),"row_layout_functions":len(layouts),
             "top_row_layouts":[{"start":x["start"],"score":x["score"],"coverage":x["coverage"],"stride18":len(x["stride18"]),"sentinel7ff":len(x["sentinel7ff"]),"imm14":len(x["imm14"])} for x in layouts[:30]],
             "shared_helper_direct_callsites":len(cmap.get(SHARED,[])),"shared_helper_unique_parent_functions":len(helper_parents),
             "helper_parents_calling_top_layouts":sum(bool(x["calls_layout_candidates"]) for x in profiles),
             "top_helper_profiles":[{k:x[k] for k in ("start","rank","row_layout_disps","calls_layout_candidates")} for x in profiles[:40]],
             "direct_rguo_aux_or_table_code_refs":len(direct)}
    a.out.with_name("summary.json").write_text(json.dumps(summary,indent=2)+"\n",encoding="utf-8")
    lines=[
      "PUB-T-451 refined descriptor-row dispatch probe",
      f"MSPUB SHA256: {h}",
      f"meta14 rows: {len(meta14)}",
      f"true row+0x0C -> cmp 0x14 dispatch hits: {len(dispatch)}",
      f"row-layout candidate functions: {len(layouts)}",
      f"direct Rguo aux/table code refs: {len(direct)}",
      f"shared helper: {len(cmap.get(SHARED,[]))} callsites / {len(helper_parents)} parent functions",
      f"helper parents calling top descriptor-layout functions: {summary['helper_parents_calling_top_layouts']}",
      "",
      "Meta14 dispatch hits:"
    ]
    for x in dispatch[:40]:
        lines.append(f"  fn={x['function']} meta={x['meta_read']['address']} {x['meta_read']['mnemonic']} {x['meta_read']['op_str']} cmp={x['cmp14']['address']} aux_same_base={len(x['aux_same_base'])} fid={len(x['fieldid_same_base'])} flags={len(x['flags_same_base'])} stride={len(x['stride18_near'])}")
    lines.append("")
    lines.append("Top row-layout functions:")
    for x in layouts[:30]:
        lines.append(f"  {x['start']} score={x['score']} coverage={x['coverage']} stride={len(x['stride18'])} 7ff={len(x['sentinel7ff'])} imm14={len(x['imm14'])}")
    lines.append("")
    lines.append("Top helper parents:")
    for x in profiles[:40]:
        lines.append(f"  {x['start']} rank={x['rank']} disps={x['row_layout_disps']} calls_layout={x['calls_layout_candidates']}")
    a.out.with_name("summary.txt").write_text("\n".join(lines)+"\n",encoding="utf-8")
    print("\n".join(lines))
if __name__=="__main__":main()
