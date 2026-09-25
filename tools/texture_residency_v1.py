#!/usr/bin/env python3
from __future__ import annotations
from dataclasses import dataclass
from hashlib import sha256
from typing import Dict, Tuple

@dataclass(frozen=True)
class MaterialKey:
    resource_id: str
    content_hash: str
    derivative: str = "source"
    color_space: str = "srgb"
    pixel_format: str = "rgba8"
    mip_policy: str = "none"
    schema: str = "chaptera.texture-material.v1"

@dataclass
class Residency:
    key: MaterialKey
    generation: int
    device_generation: int
    state: str
    source_bytes: int
    uploaded_bytes: int
    resident_bytes: int
    placements: int = 0

class FakeTextureAdapter:
    def __init__(self): self.upload_calls=[]
    def upload(self,key:MaterialKey,payload:bytes)->Tuple[str,int]:
        handle="tex:"+sha256((repr(key)+str(len(payload))).encode()).hexdigest()[:16]
        self.upload_calls.append((key,len(payload),handle)); return handle,len(payload)

class TextureResidencyV1:
    def __init__(self, adapter=None):
        self.adapter=adapter or FakeTextureAdapter(); self.device_generation=1; self._entries:Dict[MaterialKey,Residency]={}; self._handles={}; self.metrics={"uploads":0,"uploads_avoided":0,"uploaded_bytes":0,"evictions":0,"reloads":0,"stale_binding_rejections":0,"device_resets":0}
    def demand(self,key:MaterialKey,payload:bytes):
        if sha256(payload).hexdigest()!=key.content_hash: raise ValueError("content hash mismatch")
        entry=self._entries.get(key)
        if entry and entry.state=="Resident": entry.placements+=1; self.metrics["uploads_avoided"]+=1; return self.binding(key)
        reload=entry is not None
        handle,uploaded=self.adapter.upload(key,payload); generation=(entry.generation+1 if entry else 1)
        entry=Residency(key,generation,self.device_generation,"Resident",len(payload),uploaded,uploaded,1); self._entries[key]=entry; self._handles[key]=handle
        self.metrics["uploads"]+=1; self.metrics["uploaded_bytes"]+=uploaded; self.metrics["reloads"]+=int(reload)
        return self.binding(key)
    def binding(self,key):
        e=self._entries[key]; return {"key":key,"generation":e.generation,"device_generation":e.device_generation,"handle":self._handles.get(key),"state":e.state}
    def validate_binding(self,b):
        e=self._entries.get(b["key"]); ok=bool(e and e.state=="Resident" and b["generation"]==e.generation and b["device_generation"]==self.device_generation)
        if not ok:self.metrics["stale_binding_rejections"]+=1
        return ok
    def release_placement(self,key):
        e=self._entries[key]; e.placements=max(0,e.placements-1)
    def evict(self,key):
        e=self._entries[key]; e.state="Evicted"; e.resident_bytes=0; e.placements=0; self._handles.pop(key,None); self.metrics["evictions"]+=1
    def identity_for_key(self,key):
        return "texture:"+sha256(repr(key).encode()).hexdigest()

    def entry_for_identity(self,identity):
        for key,entry in self._entries.items():
            if self.identity_for_key(key)==identity:
                return entry
        raise KeyError(identity)

    def memory_entries(self):
        return [
            {
                "identity":self.identity_for_key(key),
                "resident_bytes":entry.resident_bytes if entry.state=="Resident" else 0,
                "reclaimable":entry.state=="Resident" and entry.placements==0,
                "state":entry.state,
                "demand_count":entry.placements,
                "generation":entry.generation,
            }
            for key,entry in sorted(self._entries.items(),key=lambda row:self.identity_for_key(row[0]))
        ]

    def evict_identity(self,identity):
        for key,entry in self._entries.items():
            if self.identity_for_key(key)!=identity:
                continue
            reclaimed=entry.resident_bytes if entry.state=="Resident" else 0
            if entry.state=="Resident":
                self.evict(key)
            return {"bytes_reclaimed":reclaimed,"identity":identity}
        return {"bytes_reclaimed":0,"identity":identity}

    def invalidate_identity(self,identity):
        result=self.evict_identity(identity); result["invalidated"]=identity; return result

    def reset_device(self):
        self.device_generation+=1; self.metrics["device_resets"]+=1
        for e in self._entries.values(): e.state="Evicted"; e.resident_bytes=0; e.placements=0
        self._handles.clear()
    def receipt(self):
        return {"schema":"chaptera.texture-residency.v1","device_generation":self.device_generation,"unique_materials":len(self._entries),"resident_materials":sum(e.state=="Resident" for e in self._entries.values()),"resident_bytes":sum(e.resident_bytes for e in self._entries.values()),"metrics":dict(self.metrics),"authority":{"resource_identity":"input_exact_resource_id_plus_content_hash_and_variant","physical_handles_semantic":False,"network_decode_owned_here":False}}
