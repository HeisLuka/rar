"""Chaptera CloudProject lifecycle V1 public reference implementation.

Control-plane project lifecycle above RevisionStream. Metadata/lifecycle mutations
must not masquerade as semantic document revisions.
"""

from __future__ import annotations

import copy
import hashlib
import json
from dataclasses import dataclass, field
from typing import Dict, Optional, Tuple


class LifecycleConflict(ValueError):
    pass


class LifecycleRejected(ValueError):
    pass


def _stable_id(prefix: str, *parts: str) -> str:
    data = json.dumps(parts, separators=(",", ":"), ensure_ascii=False).encode()
    return f"{prefix}:" + hashlib.sha256(data).hexdigest()[:24]


@dataclass
class ProjectRecord:
    project_id: str
    document_id: str
    tenant_id: str
    workspace_id: str
    name: str
    source_hash: str
    current_revision_id: str
    lifecycle_state: str = "active"
    lifecycle_generation: int = 0
    metadata_version: int = 0
    grants: Tuple[str, ...] = field(default_factory=tuple)
    asset_bindings: Tuple[str, ...] = field(default_factory=tuple)
    comments_inherited: bool = False
    deleted: bool = False
    provenance: Optional[dict] = None


class CloudProjectLifecycleV1:
    def __init__(self) -> None:
        self.projects: Dict[str, ProjectRecord] = {}
        self._idempotency: Dict[Tuple[str, str], Tuple[str, dict]] = {}

    @staticmethod
    def _request_hash(value: dict) -> str:
        body = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
        return hashlib.sha256(body.encode()).hexdigest()

    def _idempotent(self, scope: str, request_id: str, request: dict):
        key = (scope, request_id)
        digest = self._request_hash(request)
        prior = self._idempotency.get(key)
        if prior is None:
            return key, digest, None
        old_digest, result = prior
        if old_digest != digest:
            raise LifecycleConflict("idempotency_conflict")
        return key, digest, copy.deepcopy(result)

    def _store_result(self, key, digest, result):
        self._idempotency[key] = (digest, copy.deepcopy(result))
        return copy.deepcopy(result)

    def create_from_upload(
        self,
        *,
        tenant_id: str,
        workspace_id: str,
        source_hash: str,
        initial_revision_id: str,
        name: str,
        request_id: str,
        owner_grant: str,
        asset_bindings: Tuple[str, ...] = (),
    ) -> dict:
        request = locals().copy()
        request.pop("self")
        key, digest, prior = self._idempotent(tenant_id, request_id, request)
        if prior is not None:
            return prior
        project_id = _stable_id("project", tenant_id, request_id)
        document_id = _stable_id("document", tenant_id, request_id)
        if project_id in self.projects:
            raise LifecycleConflict("project_id_collision")
        record = ProjectRecord(
            project_id=project_id,
            document_id=document_id,
            tenant_id=tenant_id,
            workspace_id=workspace_id,
            name=name,
            source_hash=source_hash,
            current_revision_id=initial_revision_id,
            grants=(owner_grant,),
            asset_bindings=tuple(asset_bindings),
            provenance={"kind": "import", "source_hash": source_hash},
        )
        self.projects[project_id] = record
        return self._store_result(key, digest, self.snapshot(project_id))

    def snapshot(self, project_id: str) -> dict:
        record = self.projects[project_id]
        return copy.deepcopy(record.__dict__)

    def _active(self, project_id: str) -> ProjectRecord:
        record = self.projects[project_id]
        if record.deleted or record.lifecycle_state == "deleted":
            raise LifecycleRejected("deleted")
        return record

    def rename(
        self,
        *,
        project_id: str,
        expected_lifecycle_generation: int,
        expected_metadata_version: int,
        name: str,
        request_id: str,
    ) -> dict:
        request = locals().copy()
        request.pop("self")
        key, digest, prior = self._idempotent(project_id, request_id, request)
        if prior is not None:
            return prior
        r = self._active(project_id)
        if r.lifecycle_state != "active":
            raise LifecycleRejected("not_active")
        if r.lifecycle_generation != expected_lifecycle_generation:
            raise LifecycleConflict("stale_lifecycle_generation")
        if r.metadata_version != expected_metadata_version:
            raise LifecycleConflict("stale_metadata_version")
        r.name = name
        r.metadata_version += 1
        return self._store_result(key, digest, self.snapshot(project_id))

    def move_within_tenant(
        self,
        *,
        project_id: str,
        target_workspace_id: str,
        target_tenant_id: str,
        expected_lifecycle_generation: int,
        expected_metadata_version: int,
        request_id: str,
    ) -> dict:
        request = locals().copy()
        request.pop("self")
        key, digest, prior = self._idempotent(project_id, request_id, request)
        if prior is not None:
            return prior
        r = self._active(project_id)
        if target_tenant_id != r.tenant_id:
            raise LifecycleRejected("cross_tenant_identity_preserving_move_not_v0")
        if r.lifecycle_state != "active":
            raise LifecycleRejected("not_active")
        if r.lifecycle_generation != expected_lifecycle_generation:
            raise LifecycleConflict("stale_lifecycle_generation")
        if r.metadata_version != expected_metadata_version:
            raise LifecycleConflict("stale_metadata_version")
        r.workspace_id = target_workspace_id
        r.metadata_version += 1
        return self._store_result(key, digest, self.snapshot(project_id))

    def trash(self, *, project_id: str, expected_lifecycle_generation: int, request_id: str) -> dict:
        return self._lifecycle_transition(
            project_id=project_id,
            expected_lifecycle_generation=expected_lifecycle_generation,
            request_id=request_id,
            from_state="active",
            to_state="trashed",
        )

    def restore(self, *, project_id: str, expected_lifecycle_generation: int, request_id: str) -> dict:
        return self._lifecycle_transition(
            project_id=project_id,
            expected_lifecycle_generation=expected_lifecycle_generation,
            request_id=request_id,
            from_state="trashed",
            to_state="active",
        )

    def _lifecycle_transition(
        self,
        *,
        project_id: str,
        expected_lifecycle_generation: int,
        request_id: str,
        from_state: str,
        to_state: str,
    ) -> dict:
        request = locals().copy()
        request.pop("self")
        key, digest, prior = self._idempotent(project_id, request_id, request)
        if prior is not None:
            return prior
        r = self._active(project_id)
        if r.lifecycle_generation != expected_lifecycle_generation:
            raise LifecycleConflict("stale_lifecycle_generation")
        if r.lifecycle_state != from_state:
            raise LifecycleRejected(f"not_{from_state}")
        r.lifecycle_state = to_state
        r.lifecycle_generation += 1
        return self._store_result(key, digest, self.snapshot(project_id))

    def hard_delete(
        self,
        *,
        project_id: str,
        expected_lifecycle_generation: int,
        request_id: str,
    ) -> dict:
        request = locals().copy()
        request.pop("self")
        key, digest, prior = self._idempotent(project_id, request_id, request)
        if prior is not None:
            return prior
        r = self._active(project_id)
        if r.lifecycle_generation != expected_lifecycle_generation:
            raise LifecycleConflict("stale_lifecycle_generation")
        if r.lifecycle_state != "trashed":
            raise LifecycleRejected("must_trash_before_delete")
        r.lifecycle_state = "deleted"
        r.deleted = True
        r.lifecycle_generation += 1
        result = self.snapshot(project_id)
        return self._store_result(key, digest, result)

    def fork_from_revision(
        self,
        *,
        source_project_id: str,
        selected_revision_id: str,
        target_workspace_id: str,
        request_id: str,
        name: Optional[str] = None,
    ) -> dict:
        source = self._active(source_project_id)
        request = {
            "source_project_id": source_project_id,
            "selected_revision_id": selected_revision_id,
            "target_workspace_id": target_workspace_id,
            "request_id": request_id,
            "name": name,
        }
        key, digest, prior = self._idempotent(source_project_id + ":fork", request_id, request)
        if prior is not None:
            return prior

        project_id = _stable_id("project", source.tenant_id, "fork", request_id)
        document_id = _stable_id("document", source.tenant_id, "fork", request_id)
        genesis_revision_id = _stable_id(
            "revision:genesis", document_id, selected_revision_id, source.source_hash
        )
        bindings = tuple(
            _stable_id("asset-binding", document_id, binding)
            for binding in source.asset_bindings
        )
        fork = ProjectRecord(
            project_id=project_id,
            document_id=document_id,
            tenant_id=source.tenant_id,
            workspace_id=target_workspace_id,
            name=name or (source.name + " copy"),
            source_hash=source.source_hash,
            current_revision_id=genesis_revision_id,
            grants=(),
            asset_bindings=bindings,
            comments_inherited=False,
            provenance={
                "kind": "fork",
                "source_project_id": source.project_id,
                "source_document_id": source.document_id,
                "selected_revision_id": selected_revision_id,
            },
        )
        if document_id == source.document_id or genesis_revision_id == selected_revision_id:
            raise AssertionError("fork reused source semantic identity")
        self.projects[project_id] = fork
        return self._store_result(key, digest, self.snapshot(project_id))
