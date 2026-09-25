#!/usr/bin/env python3
import json
from render_color_contract_v1 import *

src = (1.0, 0.0, 0.0, 0.5)
dst = (0.0, 1.0, 0.0, 1.0)
receipt = {
    "schema": "chaptera.render-color-contract-receipt.v1",
    "measurement_class": "synthetic_reference_color_oracle",
    "real_pub": False,
    "representative": False,
    "contract": DEFAULT_CONTRACT.receipt(),
    "target": rebuild_target(),
    "backend_requirements": backend_requirement_set(),
    "reference_fixture": {
        "src_rgba": src,
        "dst_rgba": dst,
        "linear_srgb_result": composite_over(src, dst),
        "encoded_srgb_negative_oracle": composite_over_encoded_srgb(src, dst),
        "wrong_blend_space_detected": composite_over(src, dst) != composite_over_encoded_srgb(src, dst),
    },
    "resource_dispositions": {
        "explicit_srgb": color_disposition("explicit_srgb"),
        "unknown_profile": color_disposition("unknown_profile"),
        "unknown_profile_assumed": color_disposition("unknown_profile", assume_srgb_allowed=True),
    },
}
print(json.dumps(receipt, indent=2, sort_keys=True))
