#!/usr/bin/env python3
"""Minimal packed-field helpers for FALSE-OMISSION-01.

Provenance: extracted from historical HeisLuka/pub-rs PR #141
(publisher11_packed_oracle.py at 3620c3cb8bffaca95f490b58cd862ff2ab6aae7f).
Only the 11-bit header and block-bound logic required by the bounded audit is
retained here.
"""

FIXED = {
    0x08: 2,
    0x10: 4,
    0x18: 4,
    0x20: 6,
    0x28: 10,
    0x38: 18,
    0x68: 6,
    0x70: 6,
    0x78: 2,
}


def wire_base(meta: int) -> int:
    return 0x08 if meta == 0 else (meta << 3) & 0xF8


def packed_header(field_id: int, meta: int) -> bytes:
    if not 0 <= field_id <= 0x7FF:
        raise ValueError(f"field id 0x{field_id:X} outside observed 11-bit range")
    return bytes((field_id & 0xFF, wire_base(meta) | ((field_id >> 8) & 7)))


def decode_header(b0: int, b1: int) -> tuple[int, int]:
    return b0 | ((b1 & 7) << 8), b1 & 0xF8


def bounds(chunk: bytes, offset: int, kind: int):
    if kind in FIXED:
        end = offset + FIXED[kind]
        if end > len(chunk):
            raise ValueError("fixed block overrun")
        return end, None, None
    if kind >= 0x80:
        if offset + 6 > len(chunk):
            raise ValueError("truncated variable block")
        declared = int.from_bytes(chunk[offset + 2:offset + 6], "little")
        if declared < 4:
            raise ValueError("invalid variable block length")
        end = offset + 2 + declared
        if end > len(chunk):
            raise ValueError("variable block overrun")
        return end, offset + 6, end
    raise ValueError(f"unsupported wire base 0x{kind:02X}")
