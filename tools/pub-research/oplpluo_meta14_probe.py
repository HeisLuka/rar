#!/usr/bin/env python3
"""Bounded MSPUB14 meta-0x14 / OplPluo nested-schema consumer probe.

Goal:
- correct the stale "0x2E694104 handler" model: it is an aux nested-schema descriptor;
- calibrate all metadata-type 0x14 rows in the exact Microsoft registry;
- localize generic code that consumes descriptor row metadata/+0x08 aux data;
- intersect that machinery with OplPluo/Rguo/OplUo and callers of 0x2E5AA766.

Static implementation evidence only. No semantic promotion by itself.
"""
from __future__ import annotations
import argparse, hashlib, json, re, struct
from collections import defaultdict
from pathlib import Path

import capstone, pefile

EXPECTED_SHA256="27c00f7f06957f24d392f9c61fbd3b40282f15dc595caf66785b561f559d7b97"
IMAGE_BASE=0x2E000000
ROW_STRIDE=18
CLASS_SENTINEL=0x07FF
META_NESTED=0x14
KNOWN={
    "OplPluo.Rguo.aux":0x2E694104,
    "OplUo.table":0x2E694278,
    "shared_helper":0x2E5AA766,
    "page_or_pluo_predicate":0x2E168721,
    "object_type_getter":0x2E0DD76F,
    "OplPluo.ctor":0x2E1A3666,
    "OplPluo.vtable":0x2E00E36C,
}

def sha256_file(p:Path):
    h=hashlib.sha256()
    with p.open("rb") as f:
        for b in iter(lambda:f.read(1<<20),b""):h.update(b)
    return h.hexdigest()

class PE:
    def __init__(self,p:Path):
        self.p=p; self.blob=p.read_bytes(); self.pe=pefile.PE(str(p)); self.base=self.pe.OPTIONAL_HEADER.ImageBase
        self.sections=[]
        for s in self.pe.sections:
            name=s.Name.rstrip(b"\0").decode("ascii","replace")
            self.sections.append(dict(name=name,rva=s.VirtualAddress,va=self.base+s.VirtualAddress,raw=s.PointerToRawData,
                                      raw_size=s.SizeOfRawData,vsize=s.Misc_VirtualSize,data=self.blob[s.PointerToRawData:s.PointerToRawData+s.SizeOfRawData]))
    def u16(self,o):return struct.unpack_from("<H",self.blob,o)[0]
    def u32(self,o):return struct.unpack_from("<I",self.blob,o)[0]
    def va2off(self,va):
        rva=va-self.base
        for s in self.sections:
            if s["rva"]<=rva<s["rva"]+max(s["raw_size"],s["vsize"]):
                d=rva-s["rva"]
                return None if d>=s["raw_size"] else s["raw"]+d
    def off2va(self,o):
        for s in self.sections:
            if s["raw"]<=o<s["raw"]+s["raw_size"]:return s["va"]+(o-s["raw"])
    def section(self,va):
        if va is None:return None
        rva=va-self.base
        for s in self.sections:
            if s["rva"]<=rva<s["rva"]+max(s["raw_size"],s["vsize"]):return s["name"]
    def ascii(self,va,limit=180):
        o=self.va2off(va)
        if o is None:return None
        raw=self.blob[o:o+limit].split(b"\0",1)[0]
        try:s=raw.decode("ascii")
        except:return None
        return s if len(s)>=2 and all(0x20<=ord(c)<=0x7e for c in s) else None
    def utf16(self,va,limit=180):
        o=self.va2off(va)
        if o is None:return None
        out=bytearray()
        for p in range(o,min(len(self.blob)-1,o+limit*2),2):
            q=self.blob[p:p+2]
            if q==b"\0\0":break
            out.extend(q)
        try:s=out.decode("utf-16le")
        except:return None
        return s if len(s)>=2 and all(0x20<=ord(c)<=0x7e for c in s) else None

def name_at(v:PE,va:int):
    for s in (v.utf16(va),v.ascii(va)):
        if s and re.fullmatch(r"[A-Za-z_?][A-Za-z0-9_.$?@:+-]{1,126}",s):return s
    return None

def scan_tables(v:PE):
    tables=[]
    B=v.blob
    for off in range(0,len(B)-ROW_STRIDE):
        if v.u16(off+4)!=CLASS_SENTINEL:continue
        cn=name_at(v,v.u32(off))
        if not cn or not cn.startswith("Opl"):continue
        rows=[]; pos=off+ROW_STRIDE; prev=-1
        while pos+ROW_STRIDE<=len(B) and len(rows)<2048:
            fid=v.u16(pos+4); nm=name_at(v,v.u32(pos))
            if fid==CLASS_SENTINEL and nm and nm.startswith("Opl"):break
            if fid>0x7fe or fid<=prev or not nm:break
            aux=v.u32(pos+8); meta=v.u16(pos+12)
            rows.append(dict(class_name=cn,field_id=fid,field_name=nm,row_va=v.off2va(pos),row_off=pos,
                             aux_ptr=aux,aux_section=v.section(aux),metadata_type=meta,flags=v.u16(pos+16)))
            prev=fid; pos+=ROW_STRIDE
        if rows: tables.append(dict(class_name=cn,header_va=v.off2va(off),header_off=off,rows=rows))
    best={}
    for t in tables:
        if t["class_name"] not in best or len(t["rows"])>len(best[t["class_name"]]["rows"]):best[t["class_name"]]=t
    return best

def parse_aux(v:PE,ptr:int):
    o=v.va2off(ptr)
    if o is None or o+16>len(v.blob):return None
    name_ptr=v.u32(o); sentinel=v.u32(o+4); table_ptr=v.u32(o+8); tail=v.u32(o+12)
    return dict(aux_va=ptr,section=v.section(ptr),name_ptr=name_ptr,name=name_at(v,name_ptr),
                sentinel=sentinel,table_ptr=table_ptr,table_section=v.section(table_ptr),tail=tail,
                raw_hex=v.blob[o:o+16].hex())

def disasm(v:PE):
    md=capstone.Cs(capstone.CS_ARCH_X86,capstone.CS_MODE_32);md.detail=True;md.skipdata=True
    out=[]
    for s in v.sections:
        if s["name"]!=".text":continue
        for ins in md.disasm(s["data"],s["va"]):
            if ins.mnemonic==".byte":continue
            ops=[]
            for op in ins.operands:
                if op.type==capstone.x86.X86_OP_IMM:
                    ops.append(dict(type="imm",imm=op.imm&0xffffffff))
                elif op.type==capstone.x86.X86_OP_MEM:
                    ops.append(dict(type="mem",base=op.mem.base,index=op.mem.index,scale=op.mem.scale,disp=op.mem.disp))
                elif op.type==capstone.x86.X86_OP_REG:
                    ops.append(dict(type="reg",reg=op.reg))
            out.append(dict(address=ins.address,size=ins.size,mnemonic=ins.mnemonic,op_str=ins.op_str,
                            bytes=bytes(ins.bytes).hex(),ops=ops))
    return out,{x["address"]:i for i,x in enumerate(out)}

def imm_values(ins):
    return [o["imm"] for o in ins["ops"] if o["type"]=="imm"]
def mem_disps(ins):
    return [o["disp"] for o in ins["ops"] if o["type"]=="mem"]
def direct_call_target(ins):
    if ins["mnemonic"]!="call":return None
    vals=imm_values(ins); return vals[0] if vals else None
def abs_refs(ins):
    refs=set(imm_values(ins))
    for o in ins["ops"]:
        if o["type"]=="mem" and o["base"]==0 and o["index"]==0:refs.add(o["disp"]&0xffffffff)
    return refs

def slim(ins):return {"address":f"0x{ins['address']:08X}","mnemonic":ins["mnemonic"],"op_str":ins["op_str"]}

def infer_start(I,idx,max_back=0x500):
    here=I[idx]["address"]
    for j in range(idx,max(-1,idx-400),-1):
        if here-I[j]["address"]>max_back:break
        if I[j]["mnemonic"]=="push" and I[j]["op_str"]=="ebp" and j+1<len(I):
            n=I[j+1]
            if n["mnemonic"]=="mov" and n["op_str"].replace(" ","")=="ebp,esp":return I[j]["address"]
    return None

def body_for(I,A,start,limit=420):
    idx=A.get(start)
    if idx is None:return []
    body=[]
    for x in I[idx:idx+limit]:
        if x["address"]-start>0x900:break
        body.append(x)
        if x["mnemonic"].startswith("ret") and len(body)>5:break
    return body

def callers_of(I,target):
    return [x["address"] for x in I if direct_call_target(x)==target]

def function_features(v:PE,I,A,start,meta14_aux,descriptor_addrs,call_map):
    body=body_for(I,A,start)
    if not body:return None
    mem8=[];memc=[];imm14=[];stride12=[];refs=[];calls=[]
    for x in body:
        ds=mem_disps(x)
        if 8 in ds:mem8.append(x)
        if 12 in ds:memc.append(x)
        if META_NESTED in imm_values(x):imm14.append(x)
        if ROW_STRIDE in imm_values(x):stride12.append(x)
        rr=abs_refs(x)&descriptor_addrs
        if rr:refs.append((x,sorted(rr)))
        ct=direct_call_target(x)
        if ct is not None:calls.append(ct)
    # stronger local co-occurrence: any window around a meta14 immediate containing +8/+12 and/or stride18
    windows=[]
    for x in imm14:
        idx=A[x["address"]]; local=I[max(0,idx-35):min(len(I),idx+36)]
        l8=[z for z in local if 8 in mem_disps(z)]
        lc=[z for z in local if 12 in mem_disps(z)]
        ls=[z for z in local if ROW_STRIDE in imm_values(z)]
        if l8 or lc or ls:
            windows.append(dict(anchor=slim(x),mem8=[slim(z) for z in l8[:8]],mem12=[slim(z) for z in lc[:8]],stride18=[slim(z) for z in ls[:8]]))
    score=len(imm14)*3+len(windows)*8+min(4,len(mem8))+min(4,len(memc))+min(4,len(stride12))*2+len(refs)*10
    return dict(start=f"0x{start:08X}",score=score,instruction_count=len(body),
                imm14=[slim(x) for x in imm14[:20]],mem_disp8=[slim(x) for x in mem8[:20]],mem_disp12=[slim(x) for x in memc[:20]],
                stride18=[slim(x) for x in stride12[:20]],descriptor_refs=[dict(ins=slim(x),targets=[f"0x{r:08X}" for r in rr]) for x,rr in refs[:30]],
                local_windows=windows[:20],calls=[f"0x{x:08X}" for x in sorted(set(calls))[:120]],
                direct_callers=[f"0x{x:08X}" for x in call_map.get(start,[])[:200]],
                body=[slim(x) for x in body[:240]])

def main():
    ap=argparse.ArgumentParser();ap.add_argument("exe",type=Path);ap.add_argument("--out",type=Path,required=True)
    a=ap.parse_args()
    sha=sha256_file(a.exe)
    if sha.lower()!=EXPECTED_SHA256:raise SystemExit(f"hash mismatch {sha}")
    v=PE(a.exe)
    if v.base!=IMAGE_BASE:raise SystemExit(f"image base mismatch 0x{v.base:x}")
    tables=scan_tables(v)
    pluo=tables.get("OplPluo"); uo=tables.get("OplUo")
    if not pluo or not uo:raise SystemExit("OplPluo/OplUo tables missing")
    rguo=next((r for r in pluo["rows"] if r["field_id"]==2 and r["field_name"]=="Rguo"),None)
    if not rguo:raise SystemExit("Rguo row missing")
    rguo_aux=parse_aux(v,rguo["aux_ptr"])
    if rguo["aux_ptr"]!=KNOWN["OplPluo.Rguo.aux"]:raise SystemExit(f"unexpected Rguo aux {rguo['aux_ptr']:x}")
    if not rguo_aux or rguo_aux["name"]!="OplUo" or rguo_aux["table_ptr"]!=KNOWN["OplUo.table"]:
        raise SystemExit("Rguo aux calibration failed")

    meta14=[]
    aux_ptrs=set()
    for cn,t in tables.items():
        for r in t["rows"]:
            if r["metadata_type"]==META_NESTED:
                aux=parse_aux(v,r["aux_ptr"])
                meta14.append({**r,"aux":aux})
                if r["aux_ptr"]:aux_ptrs.add(r["aux_ptr"])
    if len(meta14)!=43:raise SystemExit(f"expected 43 meta14 rows, got {len(meta14)}")

    I,A=disasm(v)
    call_map=defaultdict(list)
    for _ins in I:
        _ct=direct_call_target(_ins)
        if _ct is not None:
            call_map[_ct].append(_ins["address"])
    descriptor_addrs=set(aux_ptrs)
    for t in tables.values():
        descriptor_addrs.add(t["header_va"])
        for r in t["rows"]:descriptor_addrs.add(r["row_va"])

    # Candidate generic descriptor walkers: functions containing immediate 0x14,
    # or functions reading both +8/+12 while using stride 18.
    starts=set()
    for idx,x in enumerate(I):
        if META_NESTED in imm_values(x):
            s=infer_start(I,idx)
            if s:starts.add(s)
    # add starts around stride/read co-occurrence
    for idx,x in enumerate(I):
        if ROW_STRIDE not in imm_values(x):continue
        local=I[max(0,idx-50):min(len(I),idx+51)]
        if any(8 in mem_disps(z) for z in local) and any(12 in mem_disps(z) for z in local):
            s=infer_start(I,idx)
            if s:starts.add(s)

    candidates=[]
    for s in sorted(starts):
        f=function_features(v,I,A,s,aux_ptrs,descriptor_addrs,call_map)
        if f and f["score"]>0:candidates.append(f)
    candidates.sort(key=lambda x:(-x["score"],x["start"]))

    # All direct callers of shared helper, classified by overlap with walker candidates,
    # known OplPluo anchors, descriptor refs and local constants.
    helper=KNOWN["shared_helper"]
    helper_calls=call_map.get(helper,[])
    parent_map=defaultdict(list)
    for cs in helper_calls:
        idx=A.get(cs)
        if idx is None:continue
        s=infer_start(I,idx,max_back=0x700)
        if s:parent_map[s].append(cs)
    walker_starts={int(x["start"],16) for x in candidates[:200]}
    caller_profiles=[]
    for s,callsites in parent_map.items():
        f=function_features(v,I,A,s,aux_ptrs,descriptor_addrs,call_map)
        if not f:continue
        body=body_for(I,A,s)
        refs=set()
        imm59=False; known_calls=[]
        for x in body:
            refs|=abs_refs(x)&descriptor_addrs
            if 0x59 in imm_values(x):imm59=True
            ct=direct_call_target(x)
            if ct in (KNOWN["page_or_pluo_predicate"],KNOWN["object_type_getter"],KNOWN["OplPluo.ctor"]):known_calls.append(ct)
        overlap=[w for w in walker_starts if any(direct_call_target(x)==w for x in body)]
        rank=f["score"]+len(refs)*8+(10 if imm59 else 0)+len(known_calls)*8+len(overlap)*12
        caller_profiles.append(dict(function_start=f"0x{s:08X}",rank=rank,helper_callsites=[f"0x{x:08X}" for x in callsites],
                                    walker_feature_score=f["score"],imm59=imm59,
                                    descriptor_refs=[f"0x{x:08X}" for x in sorted(refs)],
                                    known_calls=[f"0x{x:08X}" for x in sorted(set(known_calls))],
                                    calls_candidate_walkers=[f"0x{x:08X}" for x in sorted(overlap)],
                                    body=f["body"]))
    caller_profiles.sort(key=lambda x:(-x["rank"],x["function_start"]))

    # Direct references to the Rguo aux/table anywhere in code, calibrated as expected likely zero.
    direct_rguo_refs=[]
    for x in I:
        rr=abs_refs(x)&{KNOWN["OplPluo.Rguo.aux"],KNOWN["OplUo.table"]}
        if rr:direct_rguo_refs.append(dict(ins=slim(x),targets=[f"0x{z:08X}" for z in sorted(rr)]))

    # Summarize all meta14 nested classes.
    nested_hist=defaultdict(int); invalid_aux=[]
    for r in meta14:
        aux=r["aux"]
        if aux and aux.get("name"):nested_hist[aux["name"]]+=1
        else:invalid_aux.append({"class":r["class_name"],"field":r["field_name"],"aux_ptr":f"0x{r['aux_ptr']:08X}"})

    result={
      "schema":"oplpluo-meta14-probe.v1",
      "scope":{
        "task":"PUB-T-451 / U-OBJ-01",
        "binary":"MSPUB.EXE 14.0.7162.5000 x86",
        "sha256":sha,
        "boundary":"Static implementation evidence only; metadata type/nested schema is not user-facing semantic meaning."
      },
      "correction":{
        "stale_model":"0x2E694104 is a handler target",
        "exact_model":"0x2E694104 is OplPluo.Rguo aux nested-schema descriptor in .data",
        "rguo_row":rguo,
        "rguo_aux":rguo_aux,
      },
      "meta14":{
        "row_count":len(meta14),
        "nested_class_histogram":dict(sorted(nested_hist.items())),
        "invalid_aux":invalid_aux,
        "rows":meta14,
      },
      "generic_walker_candidates":candidates[:80],
      "shared_helper":{
        "function":f"0x{helper:08X}",
        "direct_callsite_count":len(helper_calls),
        "unique_parent_function_count":len(parent_map),
        "top_parent_profiles":caller_profiles[:80],
      },
      "direct_rguo_aux_or_table_code_refs":direct_rguo_refs,
    }
    summary={
      "meta14_rows":len(meta14),
      "meta14_unique_aux":len(aux_ptrs),
      "meta14_invalid_aux":len(invalid_aux),
      "rguo_aux_section":rguo_aux["section"],
      "rguo_nested_name":rguo_aux["name"],
      "rguo_nested_table":f"0x{rguo_aux['table_ptr']:08X}",
      "direct_rguo_aux_or_table_code_refs":len(direct_rguo_refs),
      "generic_walker_candidates":len(candidates),
      "top_walker_scores":[{"start":x["start"],"score":x["score"],"imm14":len(x["imm14"]),"windows":len(x["local_windows"]),"descriptor_refs":len(x["descriptor_refs"])} for x in candidates[:20]],
      "shared_helper_direct_callsites":len(helper_calls),
      "shared_helper_unique_parent_functions":len(parent_map),
      "top_helper_parent_profiles":[{k:x[k] for k in ("function_start","rank","walker_feature_score","imm59","descriptor_refs","known_calls","calls_candidate_walkers")} for x in caller_profiles[:30]],
    }
    a.out.parent.mkdir(parents=True,exist_ok=True)
    a.out.write_text(json.dumps(result,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
    a.out.with_name("oplpluo-meta14-summary.json").write_text(json.dumps(summary,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
    lines=[
      "PUB-T-451 / U-OBJ-01 meta-0x14 nested-schema consumer probe",
      f"MSPUB SHA256: {sha}",
      f"meta0x14 rows: {len(meta14)}; unique aux: {len(aux_ptrs)}; invalid aux: {len(invalid_aux)}",
      f"Rguo aux: 0x{rguo['aux_ptr']:08X} section={rguo_aux['section']} name={rguo_aux['name']} table=0x{rguo_aux['table_ptr']:08X}",
      f"direct code refs to Rguo aux/table: {len(direct_rguo_refs)}",
      f"generic walker candidates: {len(candidates)}",
      f"shared helper direct callsites: {len(helper_calls)}; unique parent functions: {len(parent_map)}",
      "",
      "Top walker candidates:",
    ]
    for x in candidates[:20]:
        lines.append(f"  {x['start']} score={x['score']} imm14={len(x['imm14'])} windows={len(x['local_windows'])} desc_refs={len(x['descriptor_refs'])}")
    lines.append("")
    lines.append("Top 0x2E5AA766 parent profiles:")
    for x in caller_profiles[:30]:
        lines.append(f"  {x['function_start']} rank={x['rank']} walker={x['walker_feature_score']} imm59={x['imm59']} refs={len(x['descriptor_refs'])} known={x['known_calls']} walker_calls={x['calls_candidate_walkers']}")
    a.out.with_name("summary.txt").write_text("\n".join(lines)+"\n",encoding="utf-8")
    print("\n".join(lines))

if __name__=="__main__":main()
