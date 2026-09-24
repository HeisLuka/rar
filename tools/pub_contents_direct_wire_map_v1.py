#!/usr/bin/env python3
"""Publisher 11 direct-property metadata type -> normalized Contents wire map.

Only byte-validated direct-property mappings are admitted here.
The XML-only 0x15 -> 0xA8 False/default observation is intentionally excluded
until a physical tag is observed.
"""

DIRECT_METADATA_TO_WIRE = {
    0x00: 0x08,
    0x02: 0x10,
    0x03: 0x18,
    0x04: 0x20,
    0x05: 0x28,
    0x07: 0x38,
    0x09: 0x48,
    0x0B: 0x58,
    0x0D: 0x68,
    0x0E: 0x70,
    0x0F: 0x78,
    0x10: 0x80,
    0x11: 0x88,
    0x12: 0x90,
    0x13: 0x98,
    0x14: 0xA0,
    0x18: 0xC0,
}


def normalized_wire_for_metadata_type(metadata_type: int) -> int:
    try:
        return DIRECT_METADATA_TO_WIRE[metadata_type]
    except KeyError as exc:
        raise ValueError(f"metadata type 0x{metadata_type:02X} is not physically admitted") from exc


def metadata_type_for_normalized_wire(wire_type: int) -> int:
    matches = [meta for meta, wire in DIRECT_METADATA_TO_WIRE.items() if wire == wire_type]
    if len(matches) != 1:
        raise ValueError(f"wire type 0x{wire_type:02X} is not uniquely admitted")
    return matches[0]
