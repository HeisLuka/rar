#!/usr/bin/env python3
"""Immutable revision-bound exact literal Story search snapshot V1."""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
from typing import Literal

from story_edit_domain_v1 import StoryEditDomainError, StoryEditDomainV1
from text_ingress_v1 import TextIngressError, normalize_external_text_v1


SearchExtentKindV1 = Literal["full_editable_story", "range"]


class TextFindError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class TextFindExtentV1:
    kind: SearchExtentKindV1
    start_scalar: int | None = None
    end_scalar: int | None = None


@dataclass(frozen=True)
class TextFindMatchV1:
    ordinal: int
    start_scalar: int
    end_scalar: int
    matched_text: str
    matched_text_sha256: str


@dataclass(frozen=True)
class TextFindSnapshotV1:
    protocol_version: Literal["chaptera.text-find-snapshot.v1"]
    policy_version: Literal["chaptera.text-find-policy.v1"]
    revision_id: str
    story_id: str
    query: str
    extent_start_scalar: int
    extent_end_scalar: int
    matches: tuple[TextFindMatchV1, ...]

    def to_dict(self) -> dict:
        return {
            "protocol_version": self.protocol_version,
            "policy_version": self.policy_version,
            "revision_id": self.revision_id,
            "story_id": self.story_id,
            "query": self.query,
            "extent_start_scalar": self.extent_start_scalar,
            "extent_end_scalar": self.extent_end_scalar,
            "matches": [
                {
                    "ordinal": m.ordinal,
                    "start_scalar": m.start_scalar,
                    "end_scalar": m.end_scalar,
                    "matched_text": m.matched_text,
                    "matched_text_sha256": m.matched_text_sha256,
                }
                for m in self.matches
            ],
        }


def _fail(code: str, message: str) -> None:
    raise TextFindError(code, message)


def _require_known_domain(
    *,
    story_id: str,
    story_text: str,
    domain: StoryEditDomainV1,
) -> tuple[int, int]:
    if not isinstance(domain, StoryEditDomainV1):
        _fail("invalid_domain", "StoryEditDomainV1 is required")
    if domain.story_id != story_id:
        _fail("invalid_domain", "StoryEditDomain StoryId mismatch")
    if domain.raw_scalar_len != len(story_text):
        _fail("find_snapshot_stale", "StoryEditDomain no longer matches Story length")
    if domain.status != "known":
        _fail("edit_domain_unknown", "Story edit domain is unknown")
    if domain.editable_start_scalar is None or domain.editable_end_scalar is None:
        _fail("invalid_domain", "known StoryEditDomain lacks editable bounds")
    return domain.editable_start_scalar, domain.editable_end_scalar


def _resolve_extent(
    *,
    editable_start: int,
    editable_end: int,
    extent: TextFindExtentV1,
) -> tuple[int, int]:
    if not isinstance(extent, TextFindExtentV1):
        _fail("invalid_extent", "TextFindExtentV1 is required")

    if extent.kind == "full_editable_story":
        if extent.start_scalar is not None or extent.end_scalar is not None:
            _fail("invalid_extent", "full_editable_story must not carry explicit bounds")
        return editable_start, editable_end

    if extent.kind != "range":
        _fail("invalid_extent", "unsupported search extent kind")

    start = extent.start_scalar
    end = extent.end_scalar
    if (
        not isinstance(start, int)
        or isinstance(start, bool)
        or not isinstance(end, int)
        or isinstance(end, bool)
        or start < editable_start
        or end > editable_end
        or end <= start
    ):
        _fail("invalid_extent", "explicit search extent must be non-empty and editable")
    return start, end


def _normalize_query(external_query: str) -> str:
    try:
        query = normalize_external_text_v1(external_query).text
    except TextIngressError as exc:
        _fail("invalid_query", str(exc))
    if query == "":
        _fail("empty_query", "TextFindSnapshotV1 query must be non-empty")
    return query


def build_text_find_snapshot_v1(
    *,
    revision_id: str,
    story_id: str,
    story_text: str,
    domain: StoryEditDomainV1,
    external_query: str,
    extent: TextFindExtentV1,
) -> TextFindSnapshotV1:
    if not isinstance(revision_id, str) or not revision_id:
        _fail("invalid_revision", "revision_id is required")
    if not isinstance(story_id, str) or not story_id:
        _fail("invalid_story", "story_id is required")
    if not isinstance(story_text, str):
        _fail("invalid_story", "story_text must be string")

    editable_start, editable_end = _require_known_domain(
        story_id=story_id,
        story_text=story_text,
        domain=domain,
    )
    start, end = _resolve_extent(
        editable_start=editable_start,
        editable_end=editable_end,
        extent=extent,
    )
    query = _normalize_query(external_query)

    matches = []
    cursor = start
    ordinal = 0
    qlen = len(query)
    while cursor <= end - qlen:
        found = story_text.find(query, cursor, end)
        if found < 0:
            break
        found_end = found + qlen
        if found < start or found_end > end:
            break
        matched = story_text[found:found_end]
        matches.append(
            TextFindMatchV1(
                ordinal=ordinal,
                start_scalar=found,
                end_scalar=found_end,
                matched_text=matched,
                matched_text_sha256=hashlib.sha256(
                    matched.encode("utf-8")
                ).hexdigest(),
            )
        )
        ordinal += 1
        cursor = found_end

    return TextFindSnapshotV1(
        protocol_version="chaptera.text-find-snapshot.v1",
        policy_version="chaptera.text-find-policy.v1",
        revision_id=revision_id,
        story_id=story_id,
        query=query,
        extent_start_scalar=start,
        extent_end_scalar=end,
        matches=tuple(matches),
    )


def validate_text_find_snapshot_current_v1(
    *,
    snapshot: TextFindSnapshotV1,
    revision_id: str,
    story_id: str,
    story_text: str,
    domain: StoryEditDomainV1,
) -> None:
    if not isinstance(snapshot, TextFindSnapshotV1):
        _fail("invalid_snapshot", "TextFindSnapshotV1 is required")
    if snapshot.revision_id != revision_id or snapshot.story_id != story_id:
        _fail("find_snapshot_stale", "snapshot revision/story identity changed")
    editable_start, editable_end = _require_known_domain(
        story_id=story_id,
        story_text=story_text,
        domain=domain,
    )
    if (
        snapshot.extent_start_scalar < editable_start
        or snapshot.extent_end_scalar > editable_end
    ):
        _fail("find_snapshot_stale", "snapshot extent is no longer editable")
    for match in snapshot.matches:
        if story_text[match.start_scalar:match.end_scalar] != match.matched_text:
            _fail("find_snapshot_stale", "snapshot match no longer equals canonical Story")


def find_next_v1(
    *,
    snapshot: TextFindSnapshotV1,
    navigation_origin: int,
    wrap: bool,
) -> TextFindMatchV1 | None:
    if not isinstance(navigation_origin, int) or isinstance(navigation_origin, bool):
        _fail("invalid_navigation_origin", "navigation origin must be scalar boundary")
    if not isinstance(wrap, bool):
        _fail("invalid_wrap", "wrap must be boolean")
    for match in snapshot.matches:
        if match.start_scalar >= navigation_origin:
            return match
    return snapshot.matches[0] if wrap and snapshot.matches else None


def find_previous_v1(
    *,
    snapshot: TextFindSnapshotV1,
    navigation_origin: int,
    wrap: bool,
) -> TextFindMatchV1 | None:
    if not isinstance(navigation_origin, int) or isinstance(navigation_origin, bool):
        _fail("invalid_navigation_origin", "navigation origin must be scalar boundary")
    if not isinstance(wrap, bool):
        _fail("invalid_wrap", "wrap must be boolean")
    for match in reversed(snapshot.matches):
        if match.end_scalar <= navigation_origin:
            return match
    return snapshot.matches[-1] if wrap and snapshot.matches else None


def serialize_text_find_snapshot_v1(snapshot: TextFindSnapshotV1) -> str:
    if not isinstance(snapshot, TextFindSnapshotV1):
        _fail("invalid_snapshot", "TextFindSnapshotV1 is required")
    return json.dumps(
        snapshot.to_dict(),
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    )
