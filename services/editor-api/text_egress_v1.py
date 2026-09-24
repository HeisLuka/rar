#!/usr/bin/env python3
"""Source-neutral canonical Story -> external text/plain egress V1.

Logical plain-text law:
- canonical U+000D paragraph boundary -> external U+000A LF
- every other valid Unicode scalar is preserved exactly
- no normalization, trimming, punctuation rewrite, case folding or zero-width
  stripping

Optional OS/browser wire adaptation is a final transport-only spelling change.
It never rewrites document state and must round-trip through TextIngressV1 to the
same canonical scalar sequence.
"""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
from typing import Literal

from story_edit_domain_v1 import (
    StoryEditDomainError,
    StoryProvenanceV1,
    derive_story_edit_domain_v1,
    validate_ordinary_story_range_v1,
)
from story_range_v1 import StoryRangeError, validate_scalar_sequence_v1
from text_ingress_v1 import TextIngressError, normalize_external_text_v1


WireNewlineV1 = Literal["lf", "crlf"]


class TextPlainEgressError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class TextPlainEgressV1:
    protocol_version: Literal["chaptera.text-plain-egress.v1"]
    canonical_text: str
    logical_text: str
    canonical_scalar_len: int
    logical_scalar_len: int
    paragraph_boundary_count: int
    logical_text_sha256: str


@dataclass(frozen=True)
class TextPlainWirePayloadV1:
    protocol_version: Literal["chaptera.text-plain-wire.v1"]
    newline_wire: WireNewlineV1
    text: str


def _fail(code: str, message: str) -> None:
    raise TextPlainEgressError(code, message)


def egress_canonical_text_v1(canonical_text: str) -> TextPlainEgressV1:
    try:
        scalar_len = validate_scalar_sequence_v1(canonical_text, "canonical_text")
    except StoryRangeError as exc:
        _fail("invalid_canonical_text", str(exc))
    if "\n" in canonical_text:
        _fail(
            "invalid_canonical_text",
            "canonical Story text must not contain external LF spelling",
        )

    logical = canonical_text.replace("\r", "\n")
    return TextPlainEgressV1(
        protocol_version="chaptera.text-plain-egress.v1",
        canonical_text=canonical_text,
        logical_text=logical,
        canonical_scalar_len=scalar_len,
        logical_scalar_len=len(logical),
        paragraph_boundary_count=canonical_text.count("\r"),
        logical_text_sha256=hashlib.sha256(logical.encode("utf-8")).hexdigest(),
    )


def egress_story_range_v1(
    *,
    story_id: str,
    story_text: str,
    provenance: StoryProvenanceV1,
    start_scalar: int,
    end_scalar: int,
) -> TextPlainEgressV1:
    try:
        domain = derive_story_edit_domain_v1(
            story_id=story_id,
            story_text=story_text,
            provenance=provenance,
        )
        validate_ordinary_story_range_v1(
            domain=domain,
            start_scalar=start_scalar,
            end_scalar=end_scalar,
        )
    except StoryEditDomainError as exc:
        _fail(exc.code, str(exc))

    return egress_canonical_text_v1(story_text[start_scalar:end_scalar])


def logical_text_to_wire_v1(
    payload: TextPlainEgressV1,
    *,
    newline_wire: WireNewlineV1,
) -> TextPlainWirePayloadV1:
    if not isinstance(payload, TextPlainEgressV1):
        _fail("invalid_payload", "TextPlainEgressV1 is required")
    if newline_wire == "lf":
        text = payload.logical_text
    elif newline_wire == "crlf":
        # Logical payload contains LF only as paragraph spelling. Other Unicode
        # scalars, including any literal U+000D, cannot occur because canonical
        # U+000D was already mapped to LF.
        text = payload.logical_text.replace("\n", "\r\n")
    else:
        _fail("invalid_wire", "newline_wire must be lf or crlf")
    return TextPlainWirePayloadV1(
        protocol_version="chaptera.text-plain-wire.v1",
        newline_wire=newline_wire,
        text=text,
    )


def wire_text_to_logical_v1(
    wire: TextPlainWirePayloadV1,
) -> str:
    if not isinstance(wire, TextPlainWirePayloadV1):
        _fail("invalid_wire", "TextPlainWirePayloadV1 is required")
    if wire.newline_wire == "lf":
        if "\r" in wire.text:
            _fail("invalid_wire", "LF wire payload must not contain CR")
        return wire.text
    if wire.newline_wire == "crlf":
        # Reject lone CR/LF so adapter behavior is explicit and deterministic.
        out = []
        index = 0
        while index < len(wire.text):
            ch = wire.text[index]
            if ch == "\r":
                if index + 1 >= len(wire.text) or wire.text[index + 1] != "\n":
                    _fail("invalid_wire", "CRLF wire contains lone CR")
                out.append("\n")
                index += 2
                continue
            if ch == "\n":
                _fail("invalid_wire", "CRLF wire contains lone LF")
            out.append(ch)
            index += 1
        return "".join(out)
    _fail("invalid_wire", "unsupported wire newline convention")


def verify_ingress_round_trip_v1(
    payload: TextPlainEgressV1,
    *,
    newline_wire: WireNewlineV1 = "lf",
) -> bool:
    wire = logical_text_to_wire_v1(payload, newline_wire=newline_wire)
    logical = wire_text_to_logical_v1(wire)
    try:
        replay = normalize_external_text_v1(logical)
    except TextIngressError as exc:
        _fail("round_trip_failed", str(exc))
    if replay.text != payload.canonical_text:
        _fail(
            "round_trip_failed",
            "TextPlainEgressV1 + TextIngressV1 changed canonical scalars",
        )
    return True


def verify_semantic_plain_text_equivalence_v1(
    *,
    canonical_fragment_text: str,
    plain_payload: TextPlainEgressV1,
) -> bool:
    if canonical_fragment_text != plain_payload.canonical_text:
        _fail(
            "dual_flavor_mismatch",
            "semantic fragment text and text/plain source slice differ",
        )
    try:
        replay = normalize_external_text_v1(plain_payload.logical_text)
    except TextIngressError as exc:
        _fail("dual_flavor_mismatch", str(exc))
    if replay.text != canonical_fragment_text:
        _fail(
            "dual_flavor_mismatch",
            "text/plain does not ingress to semantic fragment canonical text",
        )
    return True
