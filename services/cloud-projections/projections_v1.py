"""Public reference contract for Chaptera Cloud product projections.

Search, Recent and thumbnails are deliberately disposable/read-optimized state.
Current lifecycle, authorization, document identity and revision identity remain
authoritative outside this module.
"""

from __future__ import annotations

import copy
import dataclasses
import hashlib
import json
from typing import Dict, Iterable, Mapping, Optional, Tuple


class ProjectionConflict(ValueError):
    pass


class ProjectionRejected(ValueError):
    pass


def _hash(value: object) -> str:
    raw = json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return "sha256:" + hashlib.sha256(raw).hexdigest()


@dataclasses.dataclass(frozen=True)
class CurrentDocument:
    document_id: str
    current_revision_id: str
    metadata_version: int
    name: str
    workspace_id: str
    lifecycle: str = "active"
    allowed_principals: tuple[str, ...] = ()

    def visible_to(self, principal_id: str) -> bool:
        return self.lifecycle == "active" and principal_id in self.allowed_principals


@dataclasses.dataclass(frozen=True)
class SearchEntry:
    document_id: str
    indexed_revision_id: str
    indexed_metadata_version: int
    indexed_name: str
    indexed_workspace_id: str
    terms: tuple[str, ...]
    projection_version: str


class SearchProjectionV1:
    """Eventually-consistent candidate index.

    The index intentionally stores no authoritative ACL decision. Every returned
    hit is joined to current directory/lifecycle/AuthZ state.
    """

    def __init__(self, *, projection_version: str = "search:v1") -> None:
        self.projection_version = projection_version
        self.entries: Dict[str, SearchEntry] = {}

    def index_document(
        self,
        *,
        document: CurrentDocument,
        terms: Iterable[str],
    ) -> dict:
        normalized_terms = tuple(str(term) for term in terms)
        entry = SearchEntry(
            document_id=document.document_id,
            indexed_revision_id=document.current_revision_id,
            indexed_metadata_version=document.metadata_version,
            indexed_name=document.name,
            indexed_workspace_id=document.workspace_id,
            terms=normalized_terms,
            projection_version=self.projection_version,
        )
        self.entries[document.document_id] = entry
        return dataclasses.asdict(entry)

    def delete_index_entry(self, document_id: str) -> None:
        self.entries.pop(document_id, None)

    def candidates(self, query: str) -> list[SearchEntry]:
        q = query.casefold()
        if not q:
            return []
        out: list[SearchEntry] = []
        for entry in self.entries.values():
            if q in entry.indexed_name.casefold() or any(
                q in term.casefold() for term in entry.terms
            ):
                out.append(entry)
        return out

    def search(
        self,
        *,
        query: str,
        principal_id: str,
        current_directory: Mapping[str, CurrentDocument],
    ) -> list[dict]:
        hits: list[dict] = []
        for entry in self.candidates(query):
            current = current_directory.get(entry.document_id)
            if current is None or not current.visible_to(principal_id):
                continue
            revision_fresh = entry.indexed_revision_id == current.current_revision_id
            metadata_fresh = (
                entry.indexed_metadata_version == current.metadata_version
                and entry.indexed_name == current.name
                and entry.indexed_workspace_id == current.workspace_id
            )
            hits.append(
                {
                    "document_id": current.document_id,
                    "name": current.name,
                    "workspace_id": current.workspace_id,
                    "current_revision_id": current.current_revision_id,
                    "indexed_revision_id": entry.indexed_revision_id,
                    "indexed_metadata_version": entry.indexed_metadata_version,
                    "current_metadata_version": current.metadata_version,
                    "freshness": (
                        "fresh" if revision_fresh and metadata_fresh else "stale"
                    ),
                    "revision_fresh": revision_fresh,
                    "metadata_fresh": metadata_fresh,
                    "open_target": {
                        "mode": "current_document",
                        "revision_id": current.current_revision_id,
                    },
                    "projection_version": entry.projection_version,
                }
            )
        return hits


@dataclasses.dataclass(frozen=True)
class RecentActivity:
    principal_id: str
    document_id: str
    activity_id: str
    activity_order: int
    kind: str


class RecentProjectionV1:
    """Per-principal activity projection, independent from semantic revision time."""

    def __init__(self) -> None:
        self._events: Dict[Tuple[str, str], RecentActivity] = {}
        self._latest: Dict[Tuple[str, str], RecentActivity] = {}

    def record_activity(
        self,
        *,
        principal_id: str,
        document_id: str,
        activity_id: str,
        activity_order: int,
        kind: str,
    ) -> dict:
        if not principal_id or not document_id or not activity_id or not kind:
            raise ValueError("bounded activity identity is required")
        if not isinstance(activity_order, int) or isinstance(activity_order, bool):
            raise ValueError("activity_order must be integer")

        event_key = (principal_id, activity_id)
        event = RecentActivity(
            principal_id=principal_id,
            document_id=document_id,
            activity_id=activity_id,
            activity_order=activity_order,
            kind=kind,
        )
        prior = self._events.get(event_key)
        if prior is not None:
            if prior != event:
                raise ProjectionConflict("activity_idempotency_conflict")
            return dataclasses.asdict(prior)

        self._events[event_key] = event
        latest_key = (principal_id, document_id)
        latest = self._latest.get(latest_key)
        if latest is None or (event.activity_order, event.activity_id) > (
            latest.activity_order,
            latest.activity_id,
        ):
            self._latest[latest_key] = event
        return dataclasses.asdict(event)

    def list_recent(
        self,
        *,
        principal_id: str,
        current_directory: Mapping[str, CurrentDocument],
        limit: int = 20,
    ) -> list[dict]:
        if limit < 0:
            raise ValueError("limit must be non-negative")
        rows = [
            activity
            for (principal, _), activity in self._latest.items()
            if principal == principal_id
        ]
        rows.sort(
            key=lambda row: (row.activity_order, row.activity_id),
            reverse=True,
        )

        visible: list[dict] = []
        for row in rows:
            current = current_directory.get(row.document_id)
            if current is None or not current.visible_to(principal_id):
                continue
            visible.append(
                {
                    "document_id": current.document_id,
                    "name": current.name,
                    "workspace_id": current.workspace_id,
                    "current_revision_id": current.current_revision_id,
                    "last_activity_id": row.activity_id,
                    "last_activity_order": row.activity_order,
                    "last_activity_kind": row.kind,
                }
            )
            if len(visible) >= limit:
                break
        return visible


@dataclasses.dataclass(frozen=True)
class ThumbnailKey:
    document_id: str
    revision_id: str
    layout_environment_id: str
    scene_protocol_version: str
    renderer_version: str

    def artifact_id(self) -> str:
        return _hash(
            {
                "document_id": self.document_id,
                "revision_id": self.revision_id,
                "layout_environment_id": self.layout_environment_id,
                "scene_protocol_version": self.scene_protocol_version,
                "renderer_version": self.renderer_version,
            }
        )


@dataclasses.dataclass(frozen=True)
class ThumbnailArtifact:
    key: ThumbnailKey
    artifact_id: str
    content_hash: str
    completion_order: int


class ThumbnailProjectionV1:
    """Immutable thumbnail artifacts with no mutable latest-completion pointer."""

    def __init__(self) -> None:
        self._artifacts: Dict[ThumbnailKey, ThumbnailArtifact] = {}

    def publish(
        self,
        *,
        key: ThumbnailKey,
        content_hash: str,
        completion_order: int,
    ) -> dict:
        if not content_hash.startswith("sha256:"):
            raise ValueError("content_hash must be content-addressed")
        if not isinstance(completion_order, int) or isinstance(completion_order, bool):
            raise ValueError("completion_order must be integer")

        artifact = ThumbnailArtifact(
            key=key,
            artifact_id=key.artifact_id(),
            content_hash=content_hash,
            completion_order=completion_order,
        )
        prior = self._artifacts.get(key)
        if prior is not None:
            if prior.content_hash != artifact.content_hash:
                raise ProjectionConflict("immutable_thumbnail_conflict")
            return self._snapshot(prior)

        self._artifacts[key] = artifact
        return self._snapshot(artifact)

    def select(
        self,
        *,
        principal_id: str,
        current_document: CurrentDocument,
        layout_environment_id: str,
        scene_protocol_version: str,
        renderer_version: str,
        stale_revision_candidates: Iterable[str] = (),
    ) -> dict:
        if not current_document.visible_to(principal_id):
            raise ProjectionRejected("current_document_not_visible")

        exact_key = ThumbnailKey(
            document_id=current_document.document_id,
            revision_id=current_document.current_revision_id,
            layout_environment_id=layout_environment_id,
            scene_protocol_version=scene_protocol_version,
            renderer_version=renderer_version,
        )
        exact = self._artifacts.get(exact_key)
        if exact is not None:
            return {
                **self._snapshot(exact),
                "freshness": "fresh",
                "current_revision_id": current_document.current_revision_id,
            }

        # The caller supplies stale revision candidates in canonical recency
        # order. Completion time/order is intentionally ignored.
        for revision_id in stale_revision_candidates:
            if revision_id == current_document.current_revision_id:
                continue
            key = ThumbnailKey(
                document_id=current_document.document_id,
                revision_id=revision_id,
                layout_environment_id=layout_environment_id,
                scene_protocol_version=scene_protocol_version,
                renderer_version=renderer_version,
            )
            artifact = self._artifacts.get(key)
            if artifact is not None:
                return {
                    **self._snapshot(artifact),
                    "freshness": "stale",
                    "current_revision_id": current_document.current_revision_id,
                }

        return {
            "artifact_id": None,
            "content_hash": None,
            "key": None,
            "completion_order": None,
            "freshness": "missing",
            "current_revision_id": current_document.current_revision_id,
        }

    @staticmethod
    def _snapshot(artifact: ThumbnailArtifact) -> dict:
        return {
            "artifact_id": artifact.artifact_id,
            "content_hash": artifact.content_hash,
            "completion_order": artifact.completion_order,
            "key": dataclasses.asdict(artifact.key),
        }


class CloudProductProjectionsV1:
    """Small façade making projection/authority boundaries explicit."""

    def __init__(self) -> None:
        self.search = SearchProjectionV1()
        self.recent = RecentProjectionV1()
        self.thumbnails = ThumbnailProjectionV1()

    @staticmethod
    def rebuild_contract() -> dict:
        return {
            "search": {
                "rebuildable_from": [
                    "canonical current document/revision state",
                    "search extraction/tokenization policy",
                ],
                "canonical_authority": False,
            },
            "thumbnails": {
                "rebuildable_from": [
                    "exact document revision",
                    "layout environment",
                    "scene/projection protocol",
                    "renderer/thumbnailer version",
                ],
                "canonical_authority": False,
            },
            "recent": {
                "rebuildable_from_revision_stream_alone": False,
                "requires_retained": ["per-principal activity events/state"],
                "canonical_authority": False,
            },
        }
