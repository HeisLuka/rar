#!/usr/bin/env python3
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "services" / "cloud-projections"))

from projections_v1 import (
    CloudProductProjectionsV1,
    CurrentDocument,
    ProjectionRejected,
    ThumbnailKey,
)

OUT = ROOT / "target" / "cloud-projections-v1" / "receipt.json"
ALICE = "alice"


def doc(
    document_id,
    revision,
    *,
    name="Newsletter",
    workspace="workspace:a",
    metadata_version=1,
    lifecycle="active",
    allowed=(ALICE,),
):
    return CurrentDocument(
        document_id=document_id,
        current_revision_id=revision,
        metadata_version=metadata_version,
        name=name,
        workspace_id=workspace,
        lifecycle=lifecycle,
        allowed_principals=allowed,
    )


svc = CloudProductProjectionsV1()

# Search: stale candidate survives in the index, but current authority decides visibility.
indexed = doc("doc:search", "rev:1", name="Old Name", workspace="workspace:a")
svc.search.index_document(document=indexed, terms=("legacy phrase", "brochure"))

revoked = doc(
    "doc:search",
    "rev:2",
    name="New Name",
    workspace="workspace:b",
    metadata_version=2,
    allowed=(),
)
revoked_hits = svc.search.search(
    query="legacy",
    principal_id=ALICE,
    current_directory={"doc:search": revoked},
)

current = doc(
    "doc:search",
    "rev:2",
    name="New Name",
    workspace="workspace:b",
    metadata_version=2,
)
stale_hits = svc.search.search(
    query="legacy",
    principal_id=ALICE,
    current_directory={"doc:search": current},
)
new_name_hits_before_reindex = svc.search.search(
    query="new",
    principal_id=ALICE,
    current_directory={"doc:search": current},
)

purged = doc(
    "doc:search",
    "rev:2",
    name="New Name",
    workspace="workspace:b",
    metadata_version=2,
    lifecycle="purged",
)
purged_hits = svc.search.search(
    query="legacy",
    principal_id=ALICE,
    current_directory={"doc:search": purged},
)

# Recent: user activity order, not revision chronology, with current-access filtering.
svc.recent.record_activity(
    principal_id=ALICE,
    document_id="doc:newer-revision",
    activity_id="activity:0001",
    activity_order=450,
    kind="open",
)
svc.recent.record_activity(
    principal_id=ALICE,
    document_id="doc:older-revision",
    activity_id="activity:0002",
    activity_order=500,
    kind="open",
)
recent_directory = {
    "doc:newer-revision": doc(
        "doc:newer-revision", "rev:500", name="Newer revision"
    ),
    "doc:older-revision": doc(
        "doc:older-revision", "rev:100", name="Older revision"
    ),
}
recent_order = svc.recent.list_recent(
    principal_id=ALICE,
    current_directory=recent_directory,
)
recent_after_revoke = svc.recent.list_recent(
    principal_id=ALICE,
    current_directory={
        **recent_directory,
        "doc:older-revision": doc(
            "doc:older-revision",
            "rev:100",
            name="Older revision",
            allowed=(),
        ),
    },
)

# Thumbnails: newer revision completes first, old background work completes later.
new_key = ThumbnailKey(
    "doc:thumb", "rev:102", "env:a", "scene:v1", "thumb:v2"
)
old_key = ThumbnailKey(
    "doc:thumb", "rev:101", "env:a", "scene:v1", "thumb:v2"
)
other_env_key = ThumbnailKey(
    "doc:thumb", "rev:102", "env:b", "scene:v1", "thumb:v2"
)
new_artifact = svc.thumbnails.publish(
    key=new_key,
    content_hash="sha256:" + "2" * 64,
    completion_order=1,
)
svc.thumbnails.publish(
    key=old_key,
    content_hash="sha256:" + "1" * 64,
    completion_order=2,
)
env_artifact = svc.thumbnails.publish(
    key=other_env_key,
    content_hash="sha256:" + "3" * 64,
    completion_order=3,
)

thumb_current_doc = doc("doc:thumb", "rev:102")
fresh_thumb = svc.thumbnails.select(
    principal_id=ALICE,
    current_document=thumb_current_doc,
    layout_environment_id="env:a",
    scene_protocol_version="scene:v1",
    renderer_version="thumb:v2",
    stale_revision_candidates=["rev:101"],
)

# A separate projection with only the old revision demonstrates explicit stale fallback.
stale_svc = CloudProductProjectionsV1()
stale_svc.thumbnails.publish(
    key=old_key,
    content_hash="sha256:" + "1" * 64,
    completion_order=99,
)
stale_thumb = stale_svc.thumbnails.select(
    principal_id=ALICE,
    current_document=thumb_current_doc,
    layout_environment_id="env:a",
    scene_protocol_version="scene:v1",
    renderer_version="thumb:v2",
    stale_revision_candidates=["rev:101"],
)

purged_thumb_denied = False
try:
    svc.thumbnails.select(
        principal_id=ALICE,
        current_document=doc(
            "doc:thumb",
            "rev:102",
            lifecycle="purged",
        ),
        layout_environment_id="env:a",
        scene_protocol_version="scene:v1",
        renderer_version="thumb:v2",
    )
except ProjectionRejected as exc:
    purged_thumb_denied = str(exc) == "current_document_not_visible"

rebuild = CloudProductProjectionsV1.rebuild_contract()

receipt = {
    "receipt_kind": "chaptera.cloud-product-projections-v1.contract",
    "deployed_service": False,
    "search_backend_selected": False,
    "production_freshness_slo_selected": False,
    "invariants": {
        "stale_search_candidate_cannot_leak_after_revoke": revoked_hits == [],
        "purged_document_cannot_resurrect_from_stale_search_index": purged_hits == [],
        "search_hit_uses_current_metadata_and_explicit_stale_freshness": (
            len(stale_hits) == 1
            and stale_hits[0]["name"] == "New Name"
            and stale_hits[0]["workspace_id"] == "workspace:b"
            and stale_hits[0]["freshness"] == "stale"
            and stale_hits[0]["open_target"]["revision_id"] == "rev:2"
        ),
        "eventual_search_recall_is_not_faked_by_metadata_overlay": (
            new_name_hits_before_reindex == []
        ),
        "recent_is_per_principal_activity_not_revision_order": (
            [row["document_id"] for row in recent_order]
            == ["doc:older-revision", "doc:newer-revision"]
        ),
        "recent_current_access_filter_suppresses_revoked_document": (
            [row["document_id"] for row in recent_after_revoke]
            == ["doc:newer-revision"]
        ),
        "thumbnail_full_fence_distinguishes_layout_environment": (
            new_artifact["artifact_id"] != env_artifact["artifact_id"]
        ),
        "thumbnail_completion_order_cannot_regress_current_selector": (
            fresh_thumb["freshness"] == "fresh"
            and fresh_thumb["key"]["revision_id"] == "rev:102"
            and fresh_thumb["completion_order"] == 1
        ),
        "thumbnail_fallback_is_explicitly_stale": (
            stale_thumb["freshness"] == "stale"
            and stale_thumb["key"]["revision_id"] == "rev:101"
        ),
        "thumbnail_read_obeys_current_lifecycle_authz": purged_thumb_denied,
        "recent_requires_activity_state_beyond_revisionstream": (
            rebuild["recent"]["rebuildable_from_revision_stream_alone"] is False
        ),
        "projection_state_is_not_canonical_authority": (
            rebuild["search"]["canonical_authority"] is False
            and rebuild["thumbnails"]["canonical_authority"] is False
            and rebuild["recent"]["canonical_authority"] is False
        ),
    },
    "guardrail": (
        "Public service-reference contract only. Search vendor/tokenization, privacy/redaction, "
        "freshness SLO, thumbnail formats/tiers, activity retention and deployed reconciliation/DR remain open."
    ),
}
assert all(receipt["invariants"].values()), receipt
OUT.parent.mkdir(parents=True, exist_ok=True)
OUT.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
print(json.dumps(receipt, indent=2))
