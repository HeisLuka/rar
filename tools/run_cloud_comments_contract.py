#!/usr/bin/env python3
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "services" / "cloud-comments"))

from comment_service_v1 import (
    CloudCommentServiceV1,
    CommentRejected,
    NodeAnchor,
    StoryRangeAnchor,
    TextEdit,
)

OUT = ROOT / "target" / "cloud-comments-v1" / "receipt.json"


def allow_all(action, tenant_id, document_id, principal_id):
    return True


svc = CloudCommentServiceV1()
thread = svc.create_thread(
    tenant_id="tenant:1",
    document_id="doc:1",
    created_revision_id="rev:1",
    anchor=StoryRangeAnchor(
        story_id="story:1",
        start=10,
        end=15,
        revision_id="rev:1",
    ),
    principal_id="user:1",
    body="hello",
    client_request_id="create:1",
    authz=allow_all,
)
retry = svc.create_thread(
    tenant_id="tenant:1",
    document_id="doc:1",
    created_revision_id="rev:1",
    anchor=StoryRangeAnchor(
        story_id="story:1",
        start=10,
        end=15,
        revision_id="rev:1",
    ),
    principal_id="user:1",
    body="hello",
    client_request_id="create:1",
    authz=allow_all,
)
reply = svc.reply(
    thread_id=thread["thread_id"],
    principal_id="user:2",
    body="reply",
    client_request_id="reply:1",
    authz=allow_all,
)
resolved = svc.set_resolved(
    thread_id=thread["thread_id"],
    resolved=True,
    principal_id="user:1",
    client_request_id="resolve:1",
    authz=allow_all,
)
reopened = svc.set_resolved(
    thread_id=thread["thread_id"],
    resolved=False,
    principal_id="user:1",
    client_request_id="reopen:1",
    authz=allow_all,
)
transform = svc.apply_story_edit(
    tenant_id="tenant:1",
    document_id="doc:1",
    story_id="story:1",
    old_story_length=26,
    edit=TextEdit(start=10, end=10, replacement_scalar_length=2),
    new_revision_id="rev:2",
)
story_after = svc.snapshot(thread["thread_id"])

node = svc.create_thread(
    tenant_id="tenant:1",
    document_id="doc:1",
    created_revision_id="rev:2",
    anchor=NodeAnchor(
        node_id="node:42",
        revision_id="rev:2",
        page_id="page:1",
        last_known_geometry=(1, 2, 3, 4),
    ),
    principal_id="user:1",
    body="node",
    client_request_id="node:1",
    authz=allow_all,
)
svc.apply_node_change(
    tenant_id="tenant:1",
    document_id="doc:1",
    node_id="node:42",
    new_revision_id="rev:3",
    page_id="page:2",
    geometry=(10, 20, 30, 40),
)
node_moved = svc.snapshot(node["thread_id"])
svc.apply_node_change(
    tenant_id="tenant:1",
    document_id="doc:1",
    node_id="node:42",
    new_revision_id="rev:4",
    deleted=True,
)
node_deleted = svc.snapshot(node["thread_id"])

orphan = svc.create_thread(
    tenant_id="tenant:1",
    document_id="doc:1",
    created_revision_id="rev:4",
    anchor=StoryRangeAnchor(
        story_id="story:orphan",
        start=2,
        end=5,
        revision_id="rev:4",
    ),
    principal_id="user:1",
    body="orphan",
    client_request_id="orphan:1",
    authz=allow_all,
)
svc.apply_story_edit(
    tenant_id="tenant:1",
    document_id="doc:1",
    story_id="story:orphan",
    old_story_length=10,
    edit=TextEdit(start=2, end=5, replacement_scalar_length=3),
    new_revision_id="rev:5",
)
orphan_after = svc.snapshot(orphan["thread_id"])

fork_threads = svc.fork_document(
    source_tenant_id="tenant:1",
    source_document_id="doc:1",
    fork_tenant_id="tenant:1",
    fork_document_id="doc:fork",
)

auth_state = {"allow": True}


def authz(action, tenant_id, document_id, principal_id):
    return auth_state["allow"]


revoked_thread = svc.create_thread(
    tenant_id="tenant:1",
    document_id="doc:1",
    created_revision_id="rev:5",
    anchor=StoryRangeAnchor(
        story_id="story:1",
        start=0,
        end=1,
        revision_id="rev:5",
    ),
    principal_id="user:3",
    body="revoke",
    client_request_id="revoke:1",
    authz=authz,
)
auth_state["allow"] = False
revoked_read_denied = False
try:
    svc.read_thread(
        thread_id=revoked_thread["thread_id"],
        principal_id="user:3",
        authz=authz,
    )
except CommentRejected as exc:
    revoked_read_denied = str(exc) == "authz_denied"

purge = svc.hard_purge_document(tenant_id="tenant:1", document_id="doc:1")
terminal_purge = False
try:
    svc.create_thread(
        tenant_id="tenant:1",
        document_id="doc:1",
        created_revision_id="rev:6",
        anchor=StoryRangeAnchor(
            story_id="story:1",
            start=0,
            end=1,
            revision_id="rev:6",
        ),
        principal_id="user:1",
        body="resurrect",
        client_request_id="resurrect:1",
        authz=allow_all,
    )
except CommentRejected as exc:
    terminal_purge = str(exc) == "document_purged"

receipt = {
    "receipt_kind": "chaptera.cloud-comments-v1.contract",
    "deployed_service": False,
    "canonical_private_core": False,
    "invariants": {
        "create_retry_same_thread": retry["thread_id"] == thread["thread_id"],
        "comment_store_version_advances_without_document_revision_creation": (
            reply["comment_store_version"] > thread["comment_store_version"]
            and resolved["comment_store_version"] > reply["comment_store_version"]
            and reopened["comment_store_version"] > resolved["comment_store_version"]
            and reopened["created_revision_id"] == "rev:1"
        ),
        "story_boundary_affinity_transforms_canonical_scalar_range": (
            transform == {"changed": 1, "orphaned": 0}
            and story_after["anchor"]["start"] == 12
            and story_after["anchor"]["end"] == 17
            and story_after["anchor"]["revision_id"] == "rev:2"
        ),
        "whole_story_target_replace_orphans": (
            orphan_after["orphaned"]
            and orphan_after["orphan_reason"] == "anchored_text_replaced_or_deleted"
        ),
        "node_move_preserves_node_id": (
            node_moved["anchor"]["node_id"] == "node:42"
            and node_moved["anchor"]["page_id"] == "page:2"
        ),
        "node_delete_orphans": (
            node_deleted["orphaned"]
            and node_deleted["orphan_reason"] == "node_deleted"
        ),
        "current_authz_rechecked_on_read": revoked_read_denied,
        "fork_inherits_no_comments_by_default": fork_threads == [],
        "hard_purge_is_terminal": terminal_purge and purge["removed_threads"] >= 1,
    },
    "guardrail": (
        "Public service reference only; real EditorSession Story split/merge adapter, "
        "mentions/notifications, moderation, production storage/retention and screen-reader UX remain open."
    ),
}
assert all(receipt["invariants"].values()), receipt
OUT.parent.mkdir(parents=True, exist_ok=True)
OUT.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
print(json.dumps(receipt, indent=2))
