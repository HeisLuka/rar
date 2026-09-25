#!/usr/bin/env python3
import hashlib
import json

from buffer_residency_v1 import BufferResidencyV1
from render_memory_governor_v1 import (
    BufferResidencyMemoryAdapterV1,
    RenderMemoryGovernorV1,
    TextureResidencyMemoryAdapterV1,
)
from texture_residency_v1 import MaterialKey, TextureResidencyV1

texture = TextureResidencyV1()
payloads = [b"a" * 1024, b"b" * 2048]
keys = [
    MaterialKey("img:a", hashlib.sha256(payloads[0]).hexdigest(), derivative="2x"),
    MaterialKey("img:b", hashlib.sha256(payloads[1]).hexdigest(), derivative="source"),
]
for key, payload in zip(keys, payloads):
    texture.demand(key, payload)
    texture.release_placement(key)

buffers = BufferResidencyV1(initial_capacity=8192)
buffers.allocate("page:visible", b"v" * 1024, page_id="page:visible")
buffers.allocate("page:far", b"f" * 2048, page_id="page:far")

governor = RenderMemoryGovernorV1(
    soft_target_bytes=4096,
    high_target_bytes=6144,
    rearm_margin_bytes=512,
    cooldown_frames=4,
)
governor.register_pool("texture", "texture", TextureResidencyMemoryAdapterV1(texture))
governor.register_pool("buffer", "buffer", BufferResidencyMemoryAdapterV1(buffers))
for row in texture.memory_entries():
    governor.note_entry("texture", row["identity"], last_used_frame=0, quality_rank=1)
governor.note_entry("buffer", "page:visible", last_used_frame=10, active_window=True)
governor.note_entry("buffer", "page:far", last_used_frame=0)
governor.pin("buffer", "page:visible", reason="visible-frame", expires_after_frame=12)
reclaim = governor.reclaim_to(target_resident_bytes=4096, frame=10, reason="synthetic-soft-pressure")
print(json.dumps({
    "governor": governor.receipt(10),
    "reclaim": reclaim,
}, sort_keys=True, indent=2))
