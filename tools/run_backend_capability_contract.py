#!/usr/bin/env python3
import json
from backend_capability_v1 import *

req = RenderRequirementSet(
    mandatory_features=frozenset({"rect", "image_rgba8", "clip:rect"}),
    optional_quality_features=frozenset({"timestamp_query", "msaa4"}),
    performance_features=frozenset({"timestamp_query"}),
    max_texture_dimension=4096,
    max_target_dimension=4096,
    max_buffer_bytes=64 * 1024 * 1024,
)
rows = {name: compatibility(fake_backend(name), req) for name in ["full", "no_stencil", "low_limit", "no_timing", "degraded", "permanent_failure"]}
life = BackendLifecycle(fake_backend("full"))
old = life.make_handle("texture:logical-1")
life.lose()
life.recreate(fake_backend("no_timing", generation=2))
rebuild = life.rebuild(scene_fingerprint="scene:source-neutral", resource_fingerprint="resources:exact", view_fingerprint="view:1")
receipt = {
    "schema": "chaptera.backend-capability-contract-receipt.v1",
    "measurement_class": "synthetic_fake_backend_contract",
    "real_pub": False,
    "representative": False,
    "requirements": req.receipt(),
    "compatibility": rows,
    "recovery": {
        "old_handle_valid_after_recreate": life.validate_handle(old),
        "rebuild": rebuild,
        "lifecycle": life.receipt(),
    },
}
print(json.dumps(receipt, indent=2, sort_keys=True))
