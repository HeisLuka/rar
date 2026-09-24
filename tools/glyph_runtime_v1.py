#!/usr/bin/env python3
from __future__ import annotations

from dataclasses import dataclass
import copy
import hashlib
from typing import Any

SCHEMA = "chaptera.glyph-runtime.v1"

@dataclass(frozen=True, order=True)
class GlyphKey:
    font_fingerprint: str
    face_index: int
    glyph_id: int
    rendering_mode: str
    scale_bucket: int
    quality: str

@dataclass
class GlyphEntry:
    key: GlyphKey
    semantic_generation: int
    residency_generation: int
    state: str
    material_bytes: bytes
    atlas_page: int | None
    atlas_slot: int | None

class GlyphRuntimeError(ValueError):
    pass

class FakeGlyphMaterializer:
    def __init__(self):
        self.calls=0
    def materialize(self,key:GlyphKey)->bytes:
        self.calls+=1
        raw=f"{key.font_fingerprint}|{key.face_index}|{key.glyph_id}|{key.rendering_mode}|{key.scale_bucket}|{key.quality}".encode()
        digest=hashlib.sha256(raw).digest()
        size=96 if key.rendering_mode=="vector" else max(32,32*key.scale_bucket)
        return (digest*((size+len(digest)-1)//len(digest)))[:size]

class FakeAtlas:
    def __init__(self,page_capacity:int=64):
        if page_capacity<=0: raise ValueError("page_capacity must be positive")
        self.page_capacity=page_capacity
        self.generation=1
        self.placements:dict[GlyphKey,tuple[int,int]]={}
        self.free:list[tuple[int,int]]=[]
    def allocate(self,key:GlyphKey)->tuple[int,int]:
        if key in self.placements: return self.placements[key]
        if self.free:
            place=self.free.pop(0)
        else:
            ordinal=len(self.placements)+len(self.free)
            used=set(self.placements.values())
            ordinal=0
            while (ordinal//self.page_capacity,ordinal%self.page_capacity) in used:
                ordinal+=1
            place=(ordinal//self.page_capacity,ordinal%self.page_capacity)
        self.placements[key]=place
        return place
    def release(self,key:GlyphKey):
        place=self.placements.pop(key,None)
        if place is not None:
            self.free.append(place); self.free.sort()
    def repack(self):
        keys=sorted(self.placements)
        self.placements={key:(i//self.page_capacity,i%self.page_capacity) for i,key in enumerate(keys)}
        self.free=[]
        self.generation+=1
    def occupancy(self)->dict[str,Any]:
        if not self.placements:
            return {"pages":0,"live_slots":0,"capacity_slots":0,"free_slots":0,"fragmentation_ratio":0.0}
        pages=max(p for p,_ in self.placements.values())+1
        capacity=pages*self.page_capacity
        free=capacity-len(self.placements)
        return {"pages":pages,"live_slots":len(self.placements),"capacity_slots":capacity,"free_slots":free,
                "fragmentation_ratio":free/capacity if capacity else 0.0}

def select_material_policy(zoom:float,dpr:float)->tuple[str,int,str]:
    scale=zoom*dpr
    if not (scale>0): raise GlyphRuntimeError("zoom*dpr must be positive")
    if scale<=1: return ("raster",1,"normal")
    if scale<=2: return ("raster",2,"normal")
    if scale<=4: return ("raster",4,"high")
    if scale<=8: return ("raster",8,"high")
    return ("vector",0,"fidelity")

class GlyphRuntimeV1:
    def __init__(self,materializer=None,page_capacity:int=64):
        self.materializer=materializer or FakeGlyphMaterializer()
        self.atlas=FakeAtlas(page_capacity)
        self.device_generation=1
        self.font_state:dict[str,str]={}
        self.entries:dict[GlyphKey,GlyphEntry]={}
        self.metrics={"requests":0,"hits":0,"misses":0,"pending":0,"unsupported":0,"evictions":0,"repacks":0,
                      "device_resets":0,"stale_binding_rejections":0,"upload_bytes":0,"materialize_bytes":0}
    def set_font_state(self,fingerprint:str,state:str):
        if state not in {"ready","pending","blocked","failed"}: raise GlyphRuntimeError("invalid font state")
        self.font_state[fingerprint]=state
    def _binding(self,entry:GlyphEntry)->dict[str,Any]:
        return {"key":entry.key,"semantic_generation":entry.semantic_generation,"residency_generation":entry.residency_generation,
                "device_generation":self.device_generation,"atlas_generation":self.atlas.generation,
                "atlas_page":entry.atlas_page,"atlas_slot":entry.atlas_slot,"state":entry.state}
    def validate_binding(self,binding:dict[str,Any])->bool:
        key=binding.get("key"); entry=self.entries.get(key)
        ok=bool(entry and entry.state=="resident" and binding.get("semantic_generation")==entry.semantic_generation
                and binding.get("residency_generation")==entry.residency_generation
                and binding.get("device_generation")==self.device_generation
                and binding.get("atlas_generation")==self.atlas.generation
                and binding.get("atlas_page")==entry.atlas_page and binding.get("atlas_slot")==entry.atlas_slot)
        if not ok: self.metrics["stale_binding_rejections"]+=1
        return ok
    def request(self,key:GlyphKey)->dict[str,Any]:
        self.metrics["requests"]+=1
        if not isinstance(key.font_fingerprint,str) or len(key.font_fingerprint)<8: raise GlyphRuntimeError("exact font fingerprint required")
        if key.face_index<0 or key.glyph_id<0: raise GlyphRuntimeError("face/glyph id must be non-negative")
        state=self.font_state.get(key.font_fingerprint,"pending")
        if state!="ready":
            self.metrics["pending"]+=1
            return {"state":state,"key":key,"reshaped":False}
        if key.rendering_mode not in {"raster","vector"}:
            self.metrics["unsupported"]+=1
            return {"state":"unsupported","key":key,"reshaped":False}
        existing=self.entries.get(key)
        if existing and existing.state=="resident":
            self.metrics["hits"]+=1
            return {"state":"ready","binding":self._binding(existing),"reshaped":False}
        self.metrics["misses"]+=1
        material=self.materializer.materialize(key)
        page,slot=self.atlas.allocate(key)
        semantic_generation=existing.semantic_generation if existing else 1
        residency_generation=(existing.residency_generation+1) if existing else 1
        entry=GlyphEntry(key,semantic_generation,residency_generation,"resident",material,page,slot)
        self.entries[key]=entry
        self.metrics["materialize_bytes"]+=len(material); self.metrics["upload_bytes"]+=len(material)
        return {"state":"ready","binding":self._binding(entry),"reshaped":False}
    def evict(self,key:GlyphKey):
        entry=self.entries.get(key)
        if entry and entry.state=="resident":
            self.atlas.release(key); entry.state="evicted"; entry.atlas_page=None; entry.atlas_slot=None
            entry.residency_generation+=1; self.metrics["evictions"]+=1
    def repack(self):
        live=[e for e in self.entries.values() if e.state=="resident"]
        old={e.key:(e.atlas_page,e.atlas_slot) for e in live}
        self.atlas.repack()
        moved=0
        for e in live:
            page,slot=self.atlas.placements[e.key]
            if old[e.key]!=(page,slot): moved+=1
            e.atlas_page=page; e.atlas_slot=slot; e.residency_generation+=1
        self.metrics["repacks"]+=1
        return {"moved":moved,"semantic_keys_preserved":sorted(old)==sorted(e.key for e in live)}
    def reset_device(self):
        self.device_generation+=1; self.atlas=FakeAtlas(self.atlas.page_capacity); self.metrics["device_resets"]+=1
        for e in self.entries.values():
            if e.state=="resident":
                e.state="evicted"; e.atlas_page=None; e.atlas_slot=None; e.residency_generation+=1
    def prepare_run(self,run:dict,font_resource:dict,*,face_index:int=0,zoom:float=1,dpr:float=1)->dict[str,Any]:
        fingerprint=font_resource.get("content_hash")
        if not fingerprint: raise GlyphRuntimeError("font resource content_hash required")
        mode,bucket,quality=select_material_policy(zoom,dpr)
        geometry=copy.deepcopy(run["glyphs"])
        materials=[]
        for glyph in run["glyphs"]:
            key=GlyphKey(fingerprint,face_index,int(glyph["glyph_id"]),mode,bucket,quality)
            materials.append(self.request(key))
        return {"geometry":geometry,"materials":materials,"mode":mode,"scale_bucket":bucket,"quality":quality,
                "canonical_geometry_mutated":geometry!=run["glyphs"]}
    def receipt(self)->dict[str,Any]:
        requests=self.metrics["requests"]; hits=self.metrics["hits"]
        return {"schema":SCHEMA,"device_generation":self.device_generation,"atlas_generation":self.atlas.generation,
                "unique_cache_entries":len(self.entries),"atlas":self.atlas.occupancy(),"metrics":dict(self.metrics),
                "hit_ratio":hits/requests if requests else 0.0,
                "authority":{"reshapes_text":False,"discovers_host_fonts":False,"canonical_glyph_positions_mutated":False}}
