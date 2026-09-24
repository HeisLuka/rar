#!/usr/bin/env python3
"""Provenance-qualified ordinary Story edit domain V1.

Raw canonical Story text and ordinary user-editable text are not always the same
extent. V1 distinguishes three explicit provenance classes:

- chaptera_created:
  all canonical scalars are ordinary editable content; no sentinel is invented.
- imported_mature_quill_terminal_cr:
  provenance explicitly proves the final U+000D is source-backed structural
  terminal paragraph state; that one scalar is protected.
- imported_unknown:
  source provenance cannot prove whether a trailing U+000D is structural;
  operations that require an edit domain fail closed.

Protection is never inferred from the trailing character value alone.
"""

from __future__ import annotations

from dataclasses import dataclass
import json
from typing import Literal

from story_range_v1 import (
    StoryRangeError,
    StoryRangeResultV1,
    replace_story_range_v1,
    validate_scalar_sequence_v1,
)


StoryProvenanceV1 = Literal[
    "chaptera_created",
    "imported_mature_quill_terminal_cr",
    "imported_unknown",
]
DomainStatusV1 = Literal["known", "edit_domain_unknown"]


class StoryEditDomainError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class ProtectedStoryRangeV1:
    start_scalar: int
    end_scalar: int
    reason: Literal["source_terminal_paragraph_mark"]
    provenance: Literal["imported_mature_quill_terminal_cr"]


@dataclass(frozen=True)
class StoryEditDomainV1:
    protocol_version: Literal["chaptera.story-edit-domain.v1"]
    story_id: str
    provenance: StoryProvenanceV1
    status: DomainStatusV1
    raw_scalar_len: int
    editable_start_scalar: int | None
    editable_end_scalar: int | None
    caret_start_boundary: int | None
    caret_end_boundary: int | None
    protected_ranges: tuple[ProtectedStoryRangeV1, ...]

    @property
    def editable_scalar_len(self) -> int | None:
        if self.editable_start_scalar is None or self.editable_end_scalar is None:
            return None
        return self.editable_end_scalar - self.editable_start_scalar

    def to_dict(self) -> dict:
        return {
            "protocol_version": self.protocol_version,
            "story_id": self.story_id,
            "provenance": self.provenance,
            "status": self.status,
            "raw_scalar_len": self.raw_scalar_len,
            "editable_start_scalar": self.editable_start_scalar,
            "editable_end_scalar": self.editable_end_scalar,
            "caret_start_boundary": self.caret_start_boundary,
            "caret_end_boundary": self.caret_end_boundary,
            "protected_ranges": [
                {
                    "start_scalar": item.start_scalar,
                    "end_scalar": item.end_scalar,
                    "reason": item.reason,
                    "provenance": item.provenance,
                }
                for item in self.protected_ranges
            ],
        }


@dataclass(frozen=True)
class DeleteForwardDecisionV1:
    action: Literal["delete_one_scalar", "boundary_noop"]
    start_scalar: int
    end_scalar: int


def _fail(code: str, message: str) -> None:
    raise StoryEditDomainError(code, message)


def derive_story_edit_domain_v1(
    *,
    story_id: str,
    story_text: str,
    provenance: StoryProvenanceV1,
) -> StoryEditDomainV1:
    if not isinstance(story_id, str) or not story_id:
        _fail("invalid_story", "story_id is required")
    try:
        scalar_len = validate_scalar_sequence_v1(story_text, "story_text")
    except StoryRangeError as exc:
        _fail("invalid_story", str(exc))

    if provenance == "chaptera_created":
        return StoryEditDomainV1(
            protocol_version="chaptera.story-edit-domain.v1",
            story_id=story_id,
            provenance=provenance,
            status="known",
            raw_scalar_len=scalar_len,
            editable_start_scalar=0,
            editable_end_scalar=scalar_len,
            caret_start_boundary=0,
            caret_end_boundary=scalar_len,
            protected_ranges=(),
        )

    if provenance == "imported_mature_quill_terminal_cr":
        if scalar_len < 1 or not story_text.endswith("\r"):
            _fail(
                "invalid_provenance",
                "proven terminal-CR provenance requires final U+000D scalar",
            )
        protected = ProtectedStoryRangeV1(
            start_scalar=scalar_len - 1,
            end_scalar=scalar_len,
            reason="source_terminal_paragraph_mark",
            provenance="imported_mature_quill_terminal_cr",
        )
        return StoryEditDomainV1(
            protocol_version="chaptera.story-edit-domain.v1",
            story_id=story_id,
            provenance=provenance,
            status="known",
            raw_scalar_len=scalar_len,
            editable_start_scalar=0,
            editable_end_scalar=scalar_len - 1,
            caret_start_boundary=0,
            caret_end_boundary=scalar_len - 1,
            protected_ranges=(protected,),
        )

    if provenance == "imported_unknown":
        return StoryEditDomainV1(
            protocol_version="chaptera.story-edit-domain.v1",
            story_id=story_id,
            provenance=provenance,
            status="edit_domain_unknown",
            raw_scalar_len=scalar_len,
            editable_start_scalar=None,
            editable_end_scalar=None,
            caret_start_boundary=None,
            caret_end_boundary=None,
            protected_ranges=(),
        )

    _fail("invalid_provenance", "unsupported Story provenance")


def _require_known(domain: StoryEditDomainV1) -> None:
    if not isinstance(domain, StoryEditDomainV1):
        _fail("invalid_domain", "StoryEditDomainV1 is required")
    if domain.status != "known":
        _fail(
            "edit_domain_unknown",
            "ordinary edit domain is unknown for imported Story provenance",
        )


def validate_ordinary_story_range_v1(
    *,
    domain: StoryEditDomainV1,
    start_scalar: int,
    end_scalar: int,
) -> None:
    _require_known(domain)
    if (
        not isinstance(start_scalar, int)
        or isinstance(start_scalar, bool)
        or not isinstance(end_scalar, int)
        or isinstance(end_scalar, bool)
        or start_scalar < 0
        or end_scalar < start_scalar
        or end_scalar > domain.raw_scalar_len
    ):
        _fail("invalid_range", "ordinary Story scalar range is invalid")

    assert domain.editable_start_scalar is not None
    assert domain.editable_end_scalar is not None
    if start_scalar < domain.editable_start_scalar:
        _fail("invalid_range", "ordinary Story range starts before editable content")

    # Insertion exactly at editable_end is admitted and occurs immediately
    # before any protected suffix.
    if start_scalar == end_scalar == domain.editable_end_scalar:
        return

    if end_scalar > domain.editable_end_scalar or start_scalar > domain.editable_end_scalar:
        if domain.protected_ranges:
            _fail(
                "protected_story_structure",
                "ordinary Story range overlaps protected source structure",
            )
        _fail("invalid_range", "ordinary Story range exceeds editable content")


def select_all_range_v1(domain: StoryEditDomainV1) -> tuple[int, int]:
    _require_known(domain)
    assert domain.editable_start_scalar is not None
    assert domain.editable_end_scalar is not None
    return domain.editable_start_scalar, domain.editable_end_scalar


def story_end_boundary_v1(domain: StoryEditDomainV1) -> int:
    _require_known(domain)
    assert domain.caret_end_boundary is not None
    return domain.caret_end_boundary


def delete_forward_decision_v1(
    *,
    domain: StoryEditDomainV1,
    caret_boundary: int,
) -> DeleteForwardDecisionV1:
    _require_known(domain)
    if (
        not isinstance(caret_boundary, int)
        or isinstance(caret_boundary, bool)
        or caret_boundary < 0
    ):
        _fail("invalid_caret", "caret boundary is invalid")
    assert domain.caret_start_boundary is not None
    assert domain.caret_end_boundary is not None
    if caret_boundary < domain.caret_start_boundary or caret_boundary > domain.caret_end_boundary:
        _fail("invalid_caret", "caret boundary is outside ordinary Story domain")
    if caret_boundary == domain.caret_end_boundary:
        return DeleteForwardDecisionV1(
            action="boundary_noop",
            start_scalar=caret_boundary,
            end_scalar=caret_boundary,
        )
    return DeleteForwardDecisionV1(
        action="delete_one_scalar",
        start_scalar=caret_boundary,
        end_scalar=caret_boundary + 1,
    )


def replace_story_range_in_domain_v1(
    *,
    story_id: str,
    story_text: str,
    provenance: StoryProvenanceV1,
    start_scalar: int,
    end_scalar: int,
    expected_before: str,
    replacement_text: str,
) -> StoryRangeResultV1:
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
    try:
        result = replace_story_range_v1(
            story_id=story_id,
            story_text=story_text,
            start_scalar=start_scalar,
            end_scalar=end_scalar,
            expected_before=expected_before,
            replacement_text=replacement_text,
            requires_terminal_cr=(
                provenance == "imported_mature_quill_terminal_cr"
            ),
        )
    except StoryRangeError as exc:
        _fail("story_range_rejected", str(exc))
    return result


def serialize_story_edit_domain_input_v1(
    *,
    story_id: str,
    story_text: str,
    provenance: StoryProvenanceV1,
) -> str:
    # Persist provenance explicitly. Reopen must not re-infer it from text.
    domain = derive_story_edit_domain_v1(
        story_id=story_id,
        story_text=story_text,
        provenance=provenance,
    )
    return json.dumps(
        {
            "protocol_version": "chaptera.story-edit-domain-input.v1",
            "story_id": story_id,
            "story_text": story_text,
            "provenance": provenance,
            "derived_domain": domain.to_dict(),
        },
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    )


def reopen_story_edit_domain_input_v1(payload: str) -> StoryEditDomainV1:
    try:
        value = json.loads(payload)
    except json.JSONDecodeError as exc:
        _fail("invalid_serialization", "invalid Story edit-domain payload")
    if not isinstance(value, dict):
        _fail("invalid_serialization", "Story edit-domain payload must be object")
    expected = {
        "protocol_version",
        "story_id",
        "story_text",
        "provenance",
        "derived_domain",
    }
    if set(value) != expected:
        _fail("invalid_serialization", "Story edit-domain payload fields are not exact V1")
    if value.get("protocol_version") != "chaptera.story-edit-domain-input.v1":
        _fail("invalid_serialization", "Story edit-domain payload protocol mismatch")

    domain = derive_story_edit_domain_v1(
        story_id=value.get("story_id"),
        story_text=value.get("story_text"),
        provenance=value.get("provenance"),
    )
    if domain.to_dict() != value.get("derived_domain"):
        _fail(
            "invalid_serialization",
            "persisted Story edit domain does not replay from explicit provenance",
        )
    return domain
