#!/usr/bin/env python3
import hashlib,json,random,time
from texture_residency_v1 import MaterialKey,TextureResidencyV1
r=TextureResidencyV1(); payloads=[bytes([i])*65536 for i in range(12)]
keys=[MaterialKey(f"resource:{i}",hashlib.sha256(p).hexdigest()) for i,p in enumerate(payloads)]
t0=time.perf_counter_ns();
for i in range(300): r.demand(keys[i%12],payloads[i%12])
first_ms=(time.perf_counter_ns()-t0)/1e6
for k in keys[:6]: r.evict(k)
for i in range(6): r.demand(keys[i],payloads[i])
old=r.binding(keys[6]); r.reset_device(); stale_ok=r.validate_binding(old)
receipt=r.receipt(); receipt.update({"benchmark":{"placements":300,"unique_resources":12,"initial_demand_ms":first_ms,"expected_duplicate_uploads_without_sharing":300,"stale_binding_accepted_after_device_reset":stale_ok},"measurement_class":"synthetic_image_heavy_fake_adapter","real_pub":False,"representative":False})
print(json.dumps(receipt,indent=2,sort_keys=True))
