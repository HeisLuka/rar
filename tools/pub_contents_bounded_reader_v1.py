#!/usr/bin/env python3
"""Bounded mature-PUB Contents traversal using packed 11-bit tags.

This is a deliberately small public-safe reader slice. It proves that the
grounded packed-tag codec can drive bounded traversal without treating high
field-id bits as part of the physical wire class.

The reader preserves exact tag bytes and source spans. It does not assign PUB
semantics to unknown field ids and it stops at the first genuinely unsupported
normalized wire class instead of guessing a payload length.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Iterable

from pub_contents_packed_tag_v1 import decode_packed_tag

FIXED_PAYLOAD_LENGTHS = {
    0x08: 0,
    0x10: 2,
    0x18: 2,
    0x20: 4,
    0x28: 8,
    0x38: 16,
    0x68: 4,
    0x70: 4,
    0x78: 0,
}

VARIABLE_WIRE_TYPES = frozenset({0x80, 0x88, 0x90, 0x98, 0xA0, 0xC0})


@dataclass(frozen=True)
class SourceSpan:
    offset: int
    length: int

    @property
    def end(self) -> int:
        return self.offset + self.length


@dataclass(frozen=True)
class RawContentsBlock:
    field_id: int
    wire_type: int
    raw_tag: bytes
    tag_span: SourceSpan
    source: SourceSpan
    payload_span: SourceSpan
    declared_length: int | None = None
    declared_length_span: SourceSpan | None = None


@dataclass(frozen=True)
class BoundedContentsFields:
    fields: tuple[RawContentsBlock, ...]
    source: SourceSpan
    unsupported_tail: SourceSpan | None

    def is_fully_decoded(self) -> bool:
        return self.unsupported_tail is None


@dataclass(frozen=True)
class Contents0x2cChunk:
    source: SourceSpan
    declared_length: int
    declared_length_span: SourceSpan
    fields: tuple[RawContentsBlock, ...]
    unsupported_tail: SourceSpan | None

    def is_fully_decoded(self) -> bool:
        return self.unsupported_tail is None


class BoundedContentsReadError(ValueError):
    def __init__(self, *, code: str, offset: int, detail: str):
        super().__init__(f"{code} at offset {offset}: {detail}")
        self.code = code
        self.offset = offset
        self.detail = detail


def _checked_end(start: int, length: int, limit: int, *, code: str) -> int:
    if length < 0:
        raise BoundedContentsReadError(code=code, offset=start, detail="negative length")
    end = start + length
    if end > limit:
        raise BoundedContentsReadError(
            code=code,
            offset=start,
            detail=f"range end {end} exceeds bounded end {limit}",
        )
    return end


def _parse_block(data: bytes, offset: int, end: int) -> RawContentsBlock | None:
    if end - offset < 2:
        raise BoundedContentsReadError(
            code="truncated_tag",
            offset=offset,
            detail=f"need 2 bytes, have {end - offset}",
        )

    raw_tag = bytes(data[offset : offset + 2])
    tag = decode_packed_tag(raw_tag)

    fixed_payload = FIXED_PAYLOAD_LENGTHS.get(tag.wire_type)
    if fixed_payload is not None:
        block_end = _checked_end(
            offset,
            2 + fixed_payload,
            end,
            code="truncated_fixed_payload",
        )
        return RawContentsBlock(
            field_id=tag.field_id,
            wire_type=tag.wire_type,
            raw_tag=raw_tag,
            tag_span=SourceSpan(offset, 2),
            source=SourceSpan(offset, block_end - offset),
            payload_span=SourceSpan(offset + 2, fixed_payload),
        )

    if tag.wire_type in VARIABLE_WIRE_TYPES:
        if end - offset < 6:
            raise BoundedContentsReadError(
                code="truncated_variable_header",
                offset=offset,
                detail=f"need tag + u32 length, have {end - offset} bytes",
            )
        declared_length = int.from_bytes(data[offset + 2 : offset + 6], "little")
        if declared_length < 4:
            raise BoundedContentsReadError(
                code="invalid_declared_length",
                offset=offset,
                detail=f"declared length {declared_length} is smaller than 4",
            )
        block_end = _checked_end(
            offset,
            2 + declared_length,
            end,
            code="variable_block_overrun",
        )
        return RawContentsBlock(
            field_id=tag.field_id,
            wire_type=tag.wire_type,
            raw_tag=raw_tag,
            tag_span=SourceSpan(offset, 2),
            source=SourceSpan(offset, block_end - offset),
            payload_span=SourceSpan(offset + 6, declared_length - 4),
            declared_length=declared_length,
            declared_length_span=SourceSpan(offset + 2, 4),
        )

    return None


def parse_bounded_fields(
    data: bytes,
    *,
    start: int = 0,
    length: int | None = None,
) -> BoundedContentsFields:
    if start < 0 or start > len(data):
        raise BoundedContentsReadError(
            code="invalid_start",
            offset=start,
            detail=f"data length is {len(data)}",
        )
    if length is None:
        length = len(data) - start
    end = _checked_end(start, length, len(data), code="bounded_span_overrun")

    fields: list[RawContentsBlock] = []
    offset = start
    unsupported_tail = None
    while offset < end:
        block = _parse_block(data, offset, end)
        if block is None:
            unsupported_tail = SourceSpan(offset, end - offset)
            break
        fields.append(block)
        offset = block.source.end

    return BoundedContentsFields(
        fields=tuple(fields),
        source=SourceSpan(start, length),
        unsupported_tail=unsupported_tail,
    )


def parse_0x2c_chunk(data: bytes, *, offset: int = 0) -> Contents0x2cChunk:
    if offset < 0 or offset > len(data):
        raise BoundedContentsReadError(
            code="invalid_chunk_offset",
            offset=offset,
            detail=f"data length is {len(data)}",
        )
    if len(data) - offset < 4:
        raise BoundedContentsReadError(
            code="truncated_chunk_length",
            offset=offset,
            detail=f"need 4 bytes, have {len(data) - offset}",
        )

    declared_length = int.from_bytes(data[offset : offset + 4], "little")
    if declared_length < 4:
        raise BoundedContentsReadError(
            code="chunk_length_too_small",
            offset=offset,
            detail=f"declared length {declared_length} is smaller than 4",
        )
    chunk_end = _checked_end(
        offset,
        declared_length,
        len(data),
        code="chunk_range_overrun",
    )
    field_set = parse_bounded_fields(
        data,
        start=offset + 4,
        length=chunk_end - (offset + 4),
    )
    return Contents0x2cChunk(
        source=SourceSpan(offset, declared_length),
        declared_length=declared_length,
        declared_length_span=SourceSpan(offset, 4),
        fields=field_set.fields,
        unsupported_tail=field_set.unsupported_tail,
    )


def fields_with_id(
    fields: Iterable[RawContentsBlock], field_id: int
) -> tuple[RawContentsBlock, ...]:
    if not 0 <= field_id <= 0x7FF:
        raise ValueError("field_id must fit 11 bits")
    return tuple(field for field in fields if field.field_id == field_id)


def unique_field(
    fields: Iterable[RawContentsBlock], field_id: int
) -> RawContentsBlock:
    matches = fields_with_id(fields, field_id)
    if not matches:
        raise KeyError(f"field 0x{field_id:03X} not found")
    if len(matches) != 1:
        raise ValueError(f"field 0x{field_id:03X} is not unique")
    return matches[0]
