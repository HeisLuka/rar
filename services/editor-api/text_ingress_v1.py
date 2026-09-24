#!/usr/bin/env python3
"""Canonical external text ingress V1.

External valid-Unicode text is normalized exactly once before Story commit:
- CRLF -> one U+000D
- lone LF -> U+000D
- lone CR -> U+000D
Everything else is preserved scalar-for-scalar.

Canonical semantic Story fragments bypass normalization and are validation-only.
No terminal CR is appended and no Unicode normalization or typography rewrite
is performed.
"""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json

from story_range_v1 import validate_scalar_sequence_v1


class TextIngressError(ValueError):
    pass


@dataclass(frozen=True)
class CanonicalStoryTextV1:
    protocol_version: str
    text: str
    scalar_len: int
    paragraph_boundary_count: int
    text_sha256: str

    def to_dict(self) -> dict:
        return {
            "protocol_version": self.protocol_version,
            "text": self.text,
            "scalar_len": self.scalar_len,
            "paragraph_boundary_count": self.paragraph_boundary_count,
            "text_sha256": self.text_sha256,
        }


def _wrap_error(exc: ValueError) -> TextIngressError:
    return TextIngressError(str(exc))


def _canonical_result(text: str) -> CanonicalStoryTextV1:
    try:
        scalar_len = validate_scalar_sequence_v1(text, "canonical_text")
    except ValueError as exc:
        raise _wrap_error(exc) from exc
    return CanonicalStoryTextV1(
        protocol_version="chaptera.text-ingress.v1",
        text=text,
        scalar_len=scalar_len,
        paragraph_boundary_count=text.count("\r"),
        text_sha256=hashlib.sha256(text.encode("utf-8")).hexdigest(),
    )


def normalize_external_text_v1(input_text: str) -> CanonicalStoryTextV1:
    """Normalize only platform newline spelling in one deterministic pass."""
    try:
        validate_scalar_sequence_v1(input_text, "input_text")
    except ValueError as exc:
        raise _wrap_error(exc) from exc

    out: list[str] = []
    index = 0
    while index < len(input_text):
        ch = input_text[index]
        if ch == "\r":
            out.append("\r")
            if index + 1 < len(input_text) and input_text[index + 1] == "\n":
                index += 2
                continue
        elif ch == "\n":
            out.append("\r")
        else:
            out.append(ch)
        index += 1

    return _canonical_result("".join(out))


def validate_canonical_fragment_text_v1(text: str) -> CanonicalStoryTextV1:
    """Validate already-canonical semantic Story text without renormalizing it."""
    try:
        validate_scalar_sequence_v1(text, "fragment_text")
    except ValueError as exc:
        raise _wrap_error(exc) from exc
    if "\n" in text:
        raise TextIngressError(
            "canonical Story fragment must not contain LF/CRLF delimiters"
        )
    return _canonical_result(text)


def canonical_story_text_json_v1(value: CanonicalStoryTextV1) -> str:
    if not isinstance(value, CanonicalStoryTextV1):
        raise TextIngressError("CanonicalStoryTextV1 is required")
    return json.dumps(
        value.to_dict(),
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    )


def canonical_story_text_from_json_v1(payload: str) -> CanonicalStoryTextV1:
    try:
        value = json.loads(payload)
    except json.JSONDecodeError as exc:
        raise TextIngressError("invalid serialized CanonicalStoryTextV1") from exc
    if not isinstance(value, dict):
        raise TextIngressError("serialized CanonicalStoryTextV1 must be an object")
    expected = {
        "protocol_version",
        "text",
        "scalar_len",
        "paragraph_boundary_count",
        "text_sha256",
    }
    if set(value) != expected:
        raise TextIngressError("serialized CanonicalStoryTextV1 fields are not exact V1")
    if value.get("protocol_version") != "chaptera.text-ingress.v1":
        raise TextIngressError("serialized CanonicalStoryTextV1 protocol mismatch")

    result = _canonical_result(value.get("text"))
    if result.to_dict() != value:
        raise TextIngressError(
            "serialized CanonicalStoryTextV1 does not match deterministic replay"
        )
    return result
