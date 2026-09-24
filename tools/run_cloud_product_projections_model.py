#!/usr/bin/env python3
"""Executable product-projection model for search/recent/thumbnails.

Reference-model evidence only. It checks stale-index authorization, current
metadata overlay, recent activity semantics, thumbnail cache identity, and
out-of-order thumbnail completion.
"""

from __future__ import annotations

import dataclasses
import hashlib
import json
from pathlib import Path
from typing import Dict, Optional

OUT = Path("target/cloud-projections/product-projections.json")


def h(value: object) -> str:
    raw = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return "sha256:" + hashlib.sha256(raw).hexdigest()


@dataclasses.dataclass
class Document:
    document_id: str
    current_revision: str
    metadata_version: int
    name: str
    workspace: str
    lifecycle: str = "active"
    acl_generation: int = 1
    allowed_principals: tuple[str, ...] = ("alice",)


@dataclasses.dataclass
class SearchEntry:
    document_id: str
    indexed_revision: str
    indexed_metadata_version: int
    indexed_acl_generation: int
    indexed_name: str
    indexed_workspace: str
    terms: tuple[str, ...]
    indexed_allowed_principals: tuple[str, ...]


class SearchProjection:
    def __init__(self) -> None:
        self.entries: Dict[str, SearchEntry] = {}

    def index(self, doc: Document, terms: tuple[str, ...]) -> None:
        self.entries[doc.document_id] = SearchEntry(
            document_id=doc.document_id,
            indexed_revision=doc.current_revision,
            indexed_metadata_version=doc.metadata_version,
            indexed_acl_generation=doc.acl_generation,
            indexed_name=doc.name,
            indexed_workspace=doc.workspace,
            terms=terms,
            indexed_allowed_principals=doc.allowed_principals,
        )

    def candidates(self, query: str) -> list[SearchEntry]:
        q = query.lower()
        return [
            e for e in self.entries.values()
            if q in e.indexed_name.lower() or any(q in t.lower() for t in e.terms)
        ]


def search_acl_discriminator() -> dict:
    doc = Document("doc:1", "rev:1", 1, "Newsletter", "workspace:a")
    index = SearchProjection()
    index.index(doc, ("newsletter", "air force", "chapter"))

    # Current authorization is revoked after indexing; index is stale.
    doc.acl_generation += 1
    doc.allowed_principals = ()

    candidate = index.candidates("newsletter")[0]
    naive_visible = "alice" in candidate.indexed_allowed_principals
    safe_visible = (
        doc.lifecycle == "active"
        and "alice" in doc.allowed_principals
    )
    assert naive_visible is True
    assert safe_visible is False

    # Hard purge/lifecycle terminality must also suppress stale hits.
    doc.lifecycle = "purged"
    purge_visible = doc.lifecycle == "active" and "alice" in doc.allowed_principals
    assert purge_visible is False

    return {
        "stale_index_acl_would_leak": naive_visible,
        "current_authz_filter_visible": safe_visible,
        "purged_document_visible_from_stale_index": purge_visible,
        "decision": "search index may select candidates; current lifecycle/AuthZ gates every returned hit",
    }


def metadata_overlay_discriminator() -> dict:
    doc = Document("doc:2", "rev:5", 3, "Old Name", "workspace:a")
    index = SearchProjection()
    index.index(doc, ("old", "brochure"))

    # Rename/move do not wait for search reindex.
    doc.name = "New Name"
    doc.workspace = "workspace:b"
    doc.metadata_version += 1

    old_entry = index.entries[doc.document_id]
    stale_display = {
        "name": old_entry.indexed_name,
        "workspace": old_entry.indexed_workspace,
    }
    current_overlay = {"name": doc.name, "workspace": doc.workspace}
    assert stale_display != current_overlay

    old_query_hit = bool(index.candidates("old"))
    new_query_hit_before_reindex = bool(index.candidates("new"))
    assert old_query_hit and not new_query_hit_before_reindex

    return {
        "indexed_metadata": stale_display,
        "current_directory_overlay": current_overlay,
        "old_name_can_remain_recall_hit_until_reindex": old_query_hit,
        "new_name_not_recalled_until_reindex": not new_query_hit_before_reindex,
        "display_should_use_current_metadata_not_index_copy": True,
        "freshness_contract_required": True,
    }


def recent_discriminator() -> dict:
    docs = {
        "doc:a": {"revision_time": 100, "last_opened_by_alice": 500},
        "doc:b": {"revision_time": 400, "last_opened_by_alice": 450},
    }
    by_revision = sorted(docs, key=lambda d: docs[d]["revision_time"], reverse=True)
    by_activity = sorted(docs, key=lambda d: docs[d]["last_opened_by_alice"], reverse=True)
    assert by_revision == ["doc:b", "doc:a"]
    assert by_activity == ["doc:a", "doc:b"]

    # Revocation after activity must filter at read time.
    current_allowed = {"doc:a": False, "doc:b": True}
    safe_recent = [d for d in by_activity if current_allowed[d]]
    assert safe_recent == ["doc:b"]

    return {
        "revision_order": by_revision,
        "user_activity_order": by_activity,
        "safe_recent_after_revocation": safe_recent,
        "recent_is_user_activity_projection_not_revision_order": True,
    }


def thumbnail_key(
    document_id: str,
    revision_id: str,
    layout_environment_id: Optional[str] = None,
    protocol_version: Optional[str] = None,
    thumbnailer_version: Optional[str] = None,
) -> str:
    payload = {
        "document": document_id,
        "revision": revision_id,
    }
    if layout_environment_id is not None:
        payload["layout_environment"] = layout_environment_id
    if protocol_version is not None:
        payload["protocol_version"] = protocol_version
    if thumbnailer_version is not None:
        payload["thumbnailer_version"] = thumbnailer_version
    return h(payload)


def thumbnail_identity_discriminator() -> dict:
    naive_a = thumbnail_key("doc:1", "rev:10")
    naive_b = thumbnail_key("doc:1", "rev:10")
    assert naive_a == naive_b

    strong_a = thumbnail_key(
        "doc:1", "rev:10", "env:fontset-a", "scene:v1", "thumb:v2"
    )
    strong_b = thumbnail_key(
        "doc:1", "rev:10", "env:fontset-b", "scene:v1", "thumb:v2"
    )
    assert strong_a != strong_b

    # The same semantic revision can render differently under different layout envs.
    bytes_a = h({"revision": "rev:10", "env": "env:fontset-a", "pixels": "A"})
    bytes_b = h({"revision": "rev:10", "env": "env:fontset-b", "pixels": "B"})
    assert bytes_a != bytes_b

    return {
        "naive_document_revision_key_collides_across_layout_environment": naive_a == naive_b,
        "different_layout_environments_produce_different_bytes": bytes_a != bytes_b,
        "strong_keys_distinct": strong_a != strong_b,
        "required_key_fields": [
            "document_id",
            "revision_id",
            "layout_environment_id",
            "scene/projection protocol version",
            "thumbnailer/render version",
        ],
    }


def thumbnail_completion_race() -> dict:
    current_revision = "rev:102"
    completed_order = ["rev:102", "rev:101"]  # old job finishes last

    naive_latest_completion_pointer = None
    immutable: Dict[str, str] = {}
    for rev in completed_order:
        immutable[rev] = "thumb:" + rev
        naive_latest_completion_pointer = rev

    assert naive_latest_completion_pointer == "rev:101"

    def select_current() -> tuple[str, str]:
        if current_revision in immutable:
            return immutable[current_revision], "fresh"
        newest_available = completed_order[-1] if completed_order else None
        return (
            immutable[newest_available] if newest_available else "placeholder",
            "stale" if newest_available else "missing",
        )

    selected, freshness = select_current()
    assert selected == "thumb:rev:102"
    assert freshness == "fresh"

    # If exact current is absent, stale is explicit rather than masquerading as current.
    del immutable[current_revision]
    selected_stale, freshness_stale = select_current()
    assert freshness_stale == "stale"

    return {
        "completion_order": completed_order,
        "naive_latest_completion_regresses_to": naive_latest_completion_pointer,
        "revision_keyed_selector_returns_current_when_available": selected,
        "when_current_missing": {
            "selected": selected_stale,
            "freshness": freshness_stale,
        },
        "old_job_cannot_overwrite_current_thumbnail_identity": True,
    }


def search_revision_freshness() -> dict:
    doc = Document("doc:3", "rev:12", 1, "Plan", "workspace:a")
    index = SearchProjection()
    index.index(doc, ("legacy phrase",))

    # Current content advances and may no longer contain the indexed phrase.
    doc.current_revision = "rev:13"
    candidate = index.candidates("legacy phrase")[0]
    freshness = "fresh" if candidate.indexed_revision == doc.current_revision else "stale"
    assert freshness == "stale"

    return {
        "indexed_revision": candidate.indexed_revision,
        "current_revision": doc.current_revision,
        "freshness": freshness,
        "global_full_text_may_have_bounded_stale_false_positives": True,
        "open_target": "current document by default, not silently the indexed historical revision",
        "exact_revision_search_requires_separate_explicit_product_mode": True,
    }


def main() -> None:
    OUT.parent.mkdir(parents=True, exist_ok=True)
    receipt = {
        "receipt_kind": "chaptera.cloud-product-projections-reference-model.v1",
        "deployed_service": False,
        "search_backend_selected": False,
        "experiments": {
            "search_acl": search_acl_discriminator(),
            "metadata_overlay": metadata_overlay_discriminator(),
            "recent": recent_discriminator(),
            "thumbnail_identity": thumbnail_identity_discriminator(),
            "thumbnail_completion_race": thumbnail_completion_race(),
            "search_revision_freshness": search_revision_freshness(),
        },
        "bounded_findings": {
            "search_index_acl_is_not_authority": True,
            "current_lifecycle_authz_must_filter_every_hit": True,
            "search_display_metadata_should_overlay_current_directory_state": True,
            "recent_is_per_principal_activity_not_revision_order": True,
            "thumbnail_key_requires_layout_environment_and_renderer_protocol_fences": True,
            "thumbnail_completion_order_must_not_define_currentness": True,
            "stale_thumbnail_must_be_explicit": True,
            "global_full_text_search_requires_freshness_semantics": True,
        },
        "product_direction": {
            "search": (
                "eventually-consistent candidate projection; current AuthZ/lifecycle is authoritative; "
                "return current document identity by default and expose/measure projection freshness"
            ),
            "recent": (
                "per-principal activity projection filtered by current access; opening a document may "
                "change recent order without creating a semantic document revision"
            ),
            "thumbnail": (
                "immutable revision/environment/version-fenced artifacts; current selector chooses an "
                "exact-current thumbnail or explicit stale/placeholder state"
            ),
        },
        "guardrail": (
            "Reference-model evidence only. It does not choose Elasticsearch/Postgres/YDB/search vendor, "
            "freshness SLO, thumbnail format, full-text tokenization or deployed projection pipeline."
        ),
    }
    OUT.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, indent=2))


if __name__ == "__main__":
    main()
