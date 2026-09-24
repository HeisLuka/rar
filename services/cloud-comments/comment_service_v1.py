"""Public reference contract for Chaptera Cloud comments/review.

The contract keeps comment collaboration state separate from document semantic
history. It models semantic anchors, idempotent mutations, current AuthZ and
lifecycle/orphaning semantics; it is not a production persistence service.
"""

from __future__ import annotations

import copy
import hashlib
import json
from dataclasses import dataclass, field
from typing import Callable, Dict, List, Literal, Optional, Tuple, Union


class CommentConflict(ValueError):
    pass


class CommentRejected(ValueError):
    pass


def _stable_id(prefix: str, *parts: str) -> str:
    return f"{prefix}:" + hashlib.sha256("|".join(parts).encode()).hexdigest()[:24]


def _hash_request(value: dict) -> str:
    body = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(body.encode()).hexdigest()


def _body_hash(body: str) -> str:
    return hashlib.sha256(body.encode("utf-8")).hexdigest()


@dataclass(frozen=True)
class DocumentAnchor:
    revision_id: str
    kind: Literal["document"] = "document"


@dataclass(frozen=True)
class PageAnchor:
    page_id: str
    revision_id: str
    kind: Literal["page"] = "page"


@dataclass(frozen=True)
class NodeAnchor:
    node_id: str
    revision_id: str
    page_id: Optional[str] = None
    last_known_geometry: Optional[Tuple[int, int, int, int]] = None
    kind: Literal["node"] = "node"


@dataclass(frozen=True)
class StoryRangeAnchor:
    story_id: str
    start: int
    end: int
    revision_id: str
    start_affinity: Literal["left", "right"] = "right"
    end_affinity: Literal["left", "right"] = "left"
    quoted_text_hash: Optional[str] = None
    kind: Literal["story_range"] = "story_range"

    def validate(self) -> None:
        if self.start < 0 or self.end < self.start:
            raise ValueError("invalid_story_range")


Anchor = Union[DocumentAnchor, PageAnchor, NodeAnchor, StoryRangeAnchor]


@dataclass(frozen=True)
class TextEdit:
    start: int
    end: int
    replacement_scalar_length: int

    def validate(self, old_story_length: int) -> None:
        if not (0 <= self.start <= self.end <= old_story_length):
            raise ValueError("invalid_text_edit")
        if self.replacement_scalar_length < 0:
            raise ValueError("invalid_replacement_length")


@dataclass
class CommentMessage:
    message_id: str
    author_principal_id: str
    body_hash: str
    deleted: bool = False


@dataclass
class CommentThread:
    thread_id: str
    tenant_id: str
    document_id: str
    created_revision_id: str
    anchor: Anchor
    author_principal_id: str
    status: str = "open"
    orphaned: bool = False
    orphan_reason: Optional[str] = None
    comment_store_version: int = 0
    messages: List[CommentMessage] = field(default_factory=list)


Authz = Callable[[str, str, str, str], bool]


class CloudCommentServiceV1:
    """In-memory executable service contract.

    AuthZ callback arguments are:
      action, tenant_id, document_id, principal_id

    Current document access is checked on every read/write operation. Comment
    mutations advance comment-store version only; document revision is an input
    provenance/context fence, never generated here.
    """

    def __init__(self) -> None:
        self.threads: Dict[str, CommentThread] = {}
        self._commands: Dict[Tuple[str, str], Tuple[str, dict]] = {}
        self._store_version = 0
        self._purged_documents: set[Tuple[str, str]] = set()

    def _check_authz(
        self,
        *,
        action: str,
        tenant_id: str,
        document_id: str,
        principal_id: str,
        authz: Authz,
    ) -> None:
        if not authz(action, tenant_id, document_id, principal_id):
            raise CommentRejected("authz_denied")

    def _assert_document_live(self, tenant_id: str, document_id: str) -> None:
        if (tenant_id, document_id) in self._purged_documents:
            raise CommentRejected("document_purged")

    def _next_version(self) -> int:
        self._store_version += 1
        return self._store_version

    def _command_retry(
        self,
        *,
        tenant_id: str,
        client_request_id: str,
        request: dict,
    ) -> Optional[dict]:
        key = (tenant_id, client_request_id)
        prior = self._commands.get(key)
        if prior is None:
            return None
        request_hash = _hash_request(request)
        prior_hash, result = prior
        if prior_hash != request_hash:
            raise CommentConflict("idempotency_conflict")
        return copy.deepcopy(result)

    def _record_command(
        self,
        *,
        tenant_id: str,
        client_request_id: str,
        request: dict,
        result: dict,
    ) -> dict:
        self._commands[(tenant_id, client_request_id)] = (
            _hash_request(request),
            copy.deepcopy(result),
        )
        return copy.deepcopy(result)

    @staticmethod
    def _validate_anchor(anchor: Anchor) -> None:
        if isinstance(anchor, StoryRangeAnchor):
            anchor.validate()

    def create_thread(
        self,
        *,
        tenant_id: str,
        document_id: str,
        created_revision_id: str,
        anchor: Anchor,
        principal_id: str,
        body: str,
        client_request_id: str,
        authz: Authz,
    ) -> dict:
        self._assert_document_live(tenant_id, document_id)
        self._validate_anchor(anchor)
        self._check_authz(
            action="comment.create",
            tenant_id=tenant_id,
            document_id=document_id,
            principal_id=principal_id,
            authz=authz,
        )
        request = {
            "op": "create_thread",
            "document_id": document_id,
            "created_revision_id": created_revision_id,
            "anchor": self._anchor_dict(anchor),
            "principal_id": principal_id,
            "body_hash": _body_hash(body),
        }
        retry = self._command_retry(
            tenant_id=tenant_id,
            client_request_id=client_request_id,
            request=request,
        )
        if retry is not None:
            return retry

        thread_id = _stable_id("comment-thread", tenant_id, client_request_id)
        message_id = _stable_id("comment-message", thread_id, client_request_id)
        thread = CommentThread(
            thread_id=thread_id,
            tenant_id=tenant_id,
            document_id=document_id,
            created_revision_id=created_revision_id,
            anchor=anchor,
            author_principal_id=principal_id,
            comment_store_version=self._next_version(),
            messages=[
                CommentMessage(
                    message_id=message_id,
                    author_principal_id=principal_id,
                    body_hash=_body_hash(body),
                )
            ],
        )
        self.threads[thread_id] = thread
        result = self.snapshot(thread_id)
        return self._record_command(
            tenant_id=tenant_id,
            client_request_id=client_request_id,
            request=request,
            result=result,
        )

    def reply(
        self,
        *,
        thread_id: str,
        principal_id: str,
        body: str,
        client_request_id: str,
        authz: Authz,
    ) -> dict:
        thread = self.threads[thread_id]
        self._assert_document_live(thread.tenant_id, thread.document_id)
        self._check_authz(
            action="comment.reply",
            tenant_id=thread.tenant_id,
            document_id=thread.document_id,
            principal_id=principal_id,
            authz=authz,
        )
        if thread.status == "deleted":
            raise CommentRejected("thread_deleted")
        request = {
            "op": "reply",
            "thread_id": thread_id,
            "principal_id": principal_id,
            "body_hash": _body_hash(body),
        }
        retry = self._command_retry(
            tenant_id=thread.tenant_id,
            client_request_id=client_request_id,
            request=request,
        )
        if retry is not None:
            return retry
        thread.messages.append(
            CommentMessage(
                message_id=_stable_id("comment-message", thread_id, client_request_id),
                author_principal_id=principal_id,
                body_hash=_body_hash(body),
            )
        )
        thread.comment_store_version = self._next_version()
        result = self.snapshot(thread_id)
        return self._record_command(
            tenant_id=thread.tenant_id,
            client_request_id=client_request_id,
            request=request,
            result=result,
        )

    def set_resolved(
        self,
        *,
        thread_id: str,
        resolved: bool,
        principal_id: str,
        client_request_id: str,
        authz: Authz,
    ) -> dict:
        thread = self.threads[thread_id]
        self._assert_document_live(thread.tenant_id, thread.document_id)
        self._check_authz(
            action="comment.resolve" if resolved else "comment.reopen",
            tenant_id=thread.tenant_id,
            document_id=thread.document_id,
            principal_id=principal_id,
            authz=authz,
        )
        if thread.status == "deleted":
            raise CommentRejected("thread_deleted")
        request = {
            "op": "set_resolved",
            "thread_id": thread_id,
            "resolved": resolved,
            "principal_id": principal_id,
        }
        retry = self._command_retry(
            tenant_id=thread.tenant_id,
            client_request_id=client_request_id,
            request=request,
        )
        if retry is not None:
            return retry
        thread.status = "resolved" if resolved else "open"
        thread.comment_store_version = self._next_version()
        result = self.snapshot(thread_id)
        return self._record_command(
            tenant_id=thread.tenant_id,
            client_request_id=client_request_id,
            request=request,
            result=result,
        )

    def delete_thread(
        self,
        *,
        thread_id: str,
        principal_id: str,
        client_request_id: str,
        authz: Authz,
    ) -> dict:
        thread = self.threads[thread_id]
        self._assert_document_live(thread.tenant_id, thread.document_id)
        self._check_authz(
            action="comment.delete",
            tenant_id=thread.tenant_id,
            document_id=thread.document_id,
            principal_id=principal_id,
            authz=authz,
        )
        request = {
            "op": "delete_thread",
            "thread_id": thread_id,
            "principal_id": principal_id,
        }
        retry = self._command_retry(
            tenant_id=thread.tenant_id,
            client_request_id=client_request_id,
            request=request,
        )
        if retry is not None:
            return retry
        thread.status = "deleted"
        thread.comment_store_version = self._next_version()
        result = self.snapshot(thread_id)
        return self._record_command(
            tenant_id=thread.tenant_id,
            client_request_id=client_request_id,
            request=request,
            result=result,
        )

    def read_thread(
        self,
        *,
        thread_id: str,
        principal_id: str,
        authz: Authz,
    ) -> dict:
        thread = self.threads[thread_id]
        self._assert_document_live(thread.tenant_id, thread.document_id)
        self._check_authz(
            action="comment.read",
            tenant_id=thread.tenant_id,
            document_id=thread.document_id,
            principal_id=principal_id,
            authz=authz,
        )
        if thread.status == "deleted":
            raise CommentRejected("thread_deleted")
        return self.snapshot(thread_id)

    @staticmethod
    def _transform_position(pos: int, edit: TextEdit, affinity: str) -> int:
        a, b, repl = edit.start, edit.end, edit.replacement_scalar_length
        removed = b - a
        delta = repl - removed
        if removed == 0:
            if pos < a:
                return pos
            if pos > a:
                return pos + repl
            return a + repl if affinity == "right" else a
        if pos < a:
            return pos
        if pos > b:
            return pos + delta
        if pos == b:
            return a + repl
        if pos == a:
            return a + repl if affinity == "right" else a
        return a + repl if affinity == "right" else a

    def apply_story_edit(
        self,
        *,
        tenant_id: str,
        document_id: str,
        story_id: str,
        old_story_length: int,
        edit: TextEdit,
        new_revision_id: str,
    ) -> dict:
        self._assert_document_live(tenant_id, document_id)
        edit.validate(old_story_length)
        changed = 0
        orphaned = 0
        for thread in self.threads.values():
            if (
                thread.tenant_id != tenant_id
                or thread.document_id != document_id
                or thread.status == "deleted"
                or not isinstance(thread.anchor, StoryRangeAnchor)
                or thread.anchor.story_id != story_id
            ):
                continue
            anchor = thread.anchor
            anchor.validate()
            if anchor.end > old_story_length:
                raise CommentConflict("anchor_outside_story")
            full_cover = (
                edit.end > edit.start
                and edit.start <= anchor.start
                and edit.end >= anchor.end
            )
            if full_cover:
                thread.orphaned = True
                thread.orphan_reason = "anchored_text_replaced_or_deleted"
                thread.comment_store_version = self._next_version()
                orphaned += 1
                continue
            start = self._transform_position(anchor.start, edit, anchor.start_affinity)
            end = self._transform_position(anchor.end, edit, anchor.end_affinity)
            if start > end:
                start, end = end, start
            new_length = (
                old_story_length
                - (edit.end - edit.start)
                + edit.replacement_scalar_length
            )
            if not (0 <= start <= end <= new_length):
                raise CommentConflict("transformed_anchor_invalid")
            if start == end and anchor.start != anchor.end:
                thread.orphaned = True
                thread.orphan_reason = "anchored_text_collapsed"
                thread.comment_store_version = self._next_version()
                orphaned += 1
                continue
            thread.anchor = StoryRangeAnchor(
                story_id=anchor.story_id,
                start=start,
                end=end,
                revision_id=new_revision_id,
                start_affinity=anchor.start_affinity,
                end_affinity=anchor.end_affinity,
                quoted_text_hash=anchor.quoted_text_hash,
            )
            thread.comment_store_version = self._next_version()
            changed += 1
        return {"changed": changed, "orphaned": orphaned}

    def apply_node_change(
        self,
        *,
        tenant_id: str,
        document_id: str,
        node_id: str,
        new_revision_id: str,
        deleted: bool = False,
        page_id: Optional[str] = None,
        geometry: Optional[Tuple[int, int, int, int]] = None,
    ) -> dict:
        self._assert_document_live(tenant_id, document_id)
        changed = 0
        orphaned = 0
        for thread in self.threads.values():
            if (
                thread.tenant_id != tenant_id
                or thread.document_id != document_id
                or thread.status == "deleted"
                or not isinstance(thread.anchor, NodeAnchor)
                or thread.anchor.node_id != node_id
            ):
                continue
            if deleted:
                thread.orphaned = True
                thread.orphan_reason = "node_deleted"
                thread.comment_store_version = self._next_version()
                orphaned += 1
                continue
            thread.anchor = NodeAnchor(
                node_id=thread.anchor.node_id,
                revision_id=new_revision_id,
                page_id=page_id if page_id is not None else thread.anchor.page_id,
                last_known_geometry=(
                    geometry
                    if geometry is not None
                    else thread.anchor.last_known_geometry
                ),
            )
            thread.comment_store_version = self._next_version()
            changed += 1
        return {"changed": changed, "orphaned": orphaned}

    def fork_document(
        self,
        *,
        source_tenant_id: str,
        source_document_id: str,
        fork_tenant_id: str,
        fork_document_id: str,
    ) -> list[dict]:
        self._assert_document_live(source_tenant_id, source_document_id)
        self._assert_document_live(fork_tenant_id, fork_document_id)
        return []

    def hard_purge_document(self, *, tenant_id: str, document_id: str) -> dict:
        key = (tenant_id, document_id)
        if key in self._purged_documents:
            return {"document_id": document_id, "purged": True, "removed_threads": 0}
        thread_ids = [
            thread_id
            for thread_id, thread in self.threads.items()
            if thread.tenant_id == tenant_id and thread.document_id == document_id
        ]
        for thread_id in thread_ids:
            del self.threads[thread_id]
        stale_command_keys = [
            command_key
            for command_key, (_, result) in self._commands.items()
            if result.get("document_id") == document_id
            and result.get("tenant_id") == tenant_id
        ]
        for command_key in stale_command_keys:
            del self._commands[command_key]
        self._purged_documents.add(key)
        self._next_version()
        return {
            "document_id": document_id,
            "purged": True,
            "removed_threads": len(thread_ids),
        }

    @staticmethod
    def _anchor_dict(anchor: Anchor) -> dict:
        return copy.deepcopy(anchor.__dict__)

    def snapshot(self, thread_id: str) -> dict:
        thread = self.threads[thread_id]
        return {
            "thread_id": thread.thread_id,
            "tenant_id": thread.tenant_id,
            "document_id": thread.document_id,
            "created_revision_id": thread.created_revision_id,
            "anchor": self._anchor_dict(thread.anchor),
            "author_principal_id": thread.author_principal_id,
            "status": thread.status,
            "orphaned": thread.orphaned,
            "orphan_reason": thread.orphan_reason,
            "comment_store_version": thread.comment_store_version,
            "messages": [copy.deepcopy(message.__dict__) for message in thread.messages],
        }

    @property
    def store_version(self) -> int:
        return self._store_version
