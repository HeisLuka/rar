#!/usr/bin/env python3
"""Revision-bound exact literal search across canonical editable Stories V1."""

from __future__ import annotations

from dataclasses import dataclass
import json
from typing import Literal

from story_edit_domain_v1 import (
    StoryEditDomainError,
    StoryProvenanceV1,
    derive_story_edit_domain_v1,
)
from text_find_snapshot_v1 import (
    TextFindError,
    TextFindExtentV1,
    TextFindMatchV1,
    TextFindSnapshotV1,
    build_text_find_snapshot_v1,
)
from text_ingress_v1 import TextIngressError, normalize_external_text_v1


class DocumentTextFindError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class DocumentStorySearchInputV1:
    story_id: str
    story_text: str
    provenance: StoryProvenanceV1


@dataclass(frozen=True)
class DocumentStoryFindResultV1:
    story_id: str
    status: Literal["searched", "unsupported"]
    snapshot: TextFindSnapshotV1 | None
    reason: str | None


@dataclass(frozen=True)
class DocumentTextFindSnapshotV1:
    protocol_version: Literal["chaptera.document-text-find-snapshot.v1"]
    policy_version: Literal["chaptera.document-text-find-policy.v1"]
    revision_id: str
    query: str
    story_results: tuple[DocumentStoryFindResultV1, ...]

    @property
    def exhaustive_searchable(self) -> bool:
        return all(item.status == "searched" for item in self.story_results)

    @property
    def total_match_count(self) -> int:
        return sum(
            len(item.snapshot.matches)
            for item in self.story_results
            if item.snapshot is not None
        )

    def to_dict(self) -> dict:
        return {
            "protocol_version": self.protocol_version,
            "policy_version": self.policy_version,
            "revision_id": self.revision_id,
            "query": self.query,
            "story_results": [
                {
                    "story_id": item.story_id,
                    "status": item.status,
                    "snapshot": (
                        None if item.snapshot is None else item.snapshot.to_dict()
                    ),
                    "reason": item.reason,
                }
                for item in self.story_results
            ],
        }


def _fail(code: str, message: str) -> None:
    raise DocumentTextFindError(code, message)


def _canonical_inputs(
    stories: tuple[DocumentStorySearchInputV1, ...],
) -> tuple[DocumentStorySearchInputV1, ...]:
    if not isinstance(stories, tuple):
        _fail("invalid_story_registry", "stories must be an ordered tuple")
    seen = set()
    out = []
    for item in stories:
        if not isinstance(item, DocumentStorySearchInputV1):
            _fail("invalid_story_registry", "stories contain invalid entry")
        if not isinstance(item.story_id, str) or not item.story_id:
            _fail("invalid_story_registry", "StoryId is required")
        if item.story_id in seen:
            _fail("duplicate_story", "document search StoryIds must be unique")
        if not isinstance(item.story_text, str):
            _fail("invalid_story_registry", "Story text must be string")
        seen.add(item.story_id)
        out.append(item)
    out.sort(key=lambda item: item.story_id)
    return tuple(out)


def _canonical_query(external_query: str) -> str:
    try:
        query = normalize_external_text_v1(external_query).text
    except TextIngressError as exc:
        _fail("invalid_query", str(exc))
    if query == "":
        _fail("empty_query", "document text find query must be non-empty")
    return query


def build_document_text_find_snapshot_v1(
    *,
    revision_id: str,
    stories: tuple[DocumentStorySearchInputV1, ...],
    external_query: str,
) -> DocumentTextFindSnapshotV1:
    if not isinstance(revision_id, str) or not revision_id:
        _fail("invalid_revision", "revision_id is required")
    query = _canonical_query(external_query)
    canonical = _canonical_inputs(stories)
    results = []

    for item in canonical:
        try:
            domain = derive_story_edit_domain_v1(
                story_id=item.story_id,
                story_text=item.story_text,
                provenance=item.provenance,
            )
        except StoryEditDomainError as exc:
            results.append(
                DocumentStoryFindResultV1(
                    story_id=item.story_id,
                    status="unsupported",
                    snapshot=None,
                    reason=f"{exc.code}:{exc}",
                )
            )
            continue

        if domain.status != "known":
            results.append(
                DocumentStoryFindResultV1(
                    story_id=item.story_id,
                    status="unsupported",
                    snapshot=None,
                    reason=domain.status,
                )
            )
            continue

        try:
            snapshot = build_text_find_snapshot_v1(
                revision_id=revision_id,
                story_id=item.story_id,
                story_text=item.story_text,
                domain=domain,
                external_query=query,
                extent=TextFindExtentV1("full_editable_story"),
            )
        except TextFindError as exc:
            results.append(
                DocumentStoryFindResultV1(
                    story_id=item.story_id,
                    status="unsupported",
                    snapshot=None,
                    reason=f"{exc.code}:{exc}",
                )
            )
            continue
        results.append(
            DocumentStoryFindResultV1(
                story_id=item.story_id,
                status="searched",
                snapshot=snapshot,
                reason=None,
            )
        )

    return DocumentTextFindSnapshotV1(
        protocol_version="chaptera.document-text-find-snapshot.v1",
        policy_version="chaptera.document-text-find-policy.v1",
        revision_id=revision_id,
        query=query,
        story_results=tuple(results),
    )


def ordered_document_matches_v1(
    snapshot: DocumentTextFindSnapshotV1,
) -> tuple[tuple[str, TextFindMatchV1], ...]:
    if not isinstance(snapshot, DocumentTextFindSnapshotV1):
        _fail("invalid_snapshot", "DocumentTextFindSnapshotV1 is required")
    out = []
    for item in snapshot.story_results:
        if item.snapshot is None:
            continue
        for match in item.snapshot.matches:
            out.append((item.story_id, match))
    return tuple(out)


def validate_document_text_find_snapshot_current_v1(
    *,
    snapshot: DocumentTextFindSnapshotV1,
    revision_id: str,
    stories: tuple[DocumentStorySearchInputV1, ...],
) -> None:
    if not isinstance(snapshot, DocumentTextFindSnapshotV1):
        _fail("invalid_snapshot", "DocumentTextFindSnapshotV1 is required")
    if snapshot.revision_id != revision_id:
        _fail("document_find_snapshot_stale", "document authoring revision changed")
    rebuilt = build_document_text_find_snapshot_v1(
        revision_id=revision_id,
        stories=stories,
        external_query=snapshot.query,
    )
    if rebuilt != snapshot:
        _fail(
            "document_find_snapshot_stale",
            "current canonical Story registry/search domains differ from snapshot",
        )


def serialize_document_text_find_snapshot_v1(
    snapshot: DocumentTextFindSnapshotV1,
) -> str:
    if not isinstance(snapshot, DocumentTextFindSnapshotV1):
        _fail("invalid_snapshot", "DocumentTextFindSnapshotV1 is required")
    return json.dumps(
        snapshot.to_dict(),
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    )
