#!/usr/bin/env python3
"""Bounded mature-PUB Contents packed-tag codec.

Physical tag law:
  field_id  = byte0 | ((byte1 & 0x07) << 8)
  wire_type = byte1 & 0xF8

This module is intentionally semantic-free. It preserves the exact raw two-byte tag
and provides only the minimum bounded primitive needed by future Reader slices.
"""

from dataclasses import dataclass

SUPPORTED_WIRE_TYPES = frozenset({0x08, 0x20, 0x68, 0x80, 0x88, 0xA0})


@dataclass(frozen=True)
class PackedContentsTag:
    field_id: int
    wire_type: int
    raw: bytes

    def __post_init__(self):
        if not 0 <= self.field_id <= 0x7FF:
            raise ValueError("field_id must fit 11 bits")
        if not 0 <= self.wire_type <= 0xF8 or self.wire_type & 0x07:
            raise ValueError("wire_type must be normalized (low 3 bits clear)")
        if len(self.raw) != 2:
            raise ValueError("raw tag must contain exactly 2 bytes")


def decode_packed_tag(raw: bytes) -> PackedContentsTag:
    if len(raw) != 2:
        raise ValueError("packed tag requires exactly 2 bytes")
    b0, b1 = raw
    field_id = b0 | ((b1 & 0x07) << 8)
    wire_type = b1 & 0xF8
    return PackedContentsTag(field_id=field_id, wire_type=wire_type, raw=bytes(raw))


def encode_packed_tag(field_id: int, wire_type: int) -> bytes:
    if not 0 <= field_id <= 0x7FF:
        raise ValueError("field_id must fit 11 bits")
    if not 0 <= wire_type <= 0xF8 or wire_type & 0x07:
        raise ValueError("wire_type must be normalized (low 3 bits clear)")
    return bytes((field_id & 0xFF, wire_type | ((field_id >> 8) & 0x07)))


def is_supported_wire_type(wire_type: int) -> bool:
    return wire_type in SUPPORTED_WIRE_TYPES
