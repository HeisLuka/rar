"""Bounded server-side revision kernel for Chaptera Web Editor V1.

This module owns revision/idempotency bookkeeping only. It deliberately does not
parse PUB files or implement semantic mutations. The caller supplies an
authoritative executor that must be backed by the canonical editor core.
"""

from __future__ import annotations

import copy
import hashlib
import json
from dataclasses import dataclass
from typing import Any, Callable, Dict, Optional, Tuple


MAX_SAFE_EMU = 9_007_199_254_740_991
MIN_SAFE_EMU = -MAX_SAFE_EMU


def canonical_json(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def hash_id(value: Any) -> str:
    return "sha256:" + hashlib.sha256(canonical_json(value)).hexdigest()


def project_hash(project: dict) -> str:
    return hash_id(project)


def state_id(document_id: str, source_hash: str, project: dict) -> str:
    return hash_id(
        {
            "protocol_version": "chaptera.authoring-state.v1",
            "document_id": document_id,
            "source_hash": source_hash,
            "project_schema_version": project["schema_version"],
            "project_hash": project_hash(project),
        }
    )


def revision_id(
    document_id: str,
    source_hash: str,
    parent_revision_id: Optional[str],
    authoring_state_id: str,
    transition_kind: str,
    transition_hash: Optional[str],
) -> str:
    return hash_id(
        {
            "protocol_version": "chaptera.revision-node.v1",
            "document_id": document_id,
            "source_hash": source_hash,
            "parent_revision_id": parent_revision_id,
            "state_id": authoring_state_id,
            "transition_kind": transition_kind,
            "transition_hash": transition_hash,
        }
    )


@dataclass(frozen=True)
class RevisionRecord:
    document_id: str
    source_hash: str
    revision_id: str
    state_id: str
    parent_revision_id: Optional[str]
    project_schema_version: str
    project_hash: str
    transition_kind: str
    transition_hash: Optional[str]
    project: dict


@dataclass
class DocumentState:
    document_id: str
    source_hash: str
    current_revision_id: str


AuthoritativeExecutor = Callable[[dict, dict], Tuple[dict, dict, list]]


class RevisionKernel:
    def __init__(self) -> None:
        self._documents: Dict[str, DocumentState] = {}
        self._revisions: Dict[str, RevisionRecord] = {}
        self._idempotency: Dict[Tuple[str, str], Tuple[str, dict]] = {}

    def register_baseline(
        self,
        *,
        document_id: str,
        source_hash: str,
        project: dict,
    ) -> RevisionRecord:
        existing = self._documents.get(document_id)
        if existing is not None:
            return self._revisions[existing.current_revision_id]

        self._validate_project_source(project, source_hash)
        sid = state_id(document_id, source_hash, project)
        rid = revision_id(
            document_id,
            source_hash,
            None,
            sid,
            "baseline",
            None,
        )
        record = RevisionRecord(
            document_id=document_id,
            source_hash=source_hash,
            revision_id=rid,
            state_id=sid,
            parent_revision_id=None,
            project_schema_version=project["schema_version"],
            project_hash=project_hash(project),
            transition_kind="baseline",
            transition_hash=None,
            project=copy.deepcopy(project),
        )
        self._revisions[rid] = record
        self._documents[document_id] = DocumentState(
            document_id=document_id,
            source_hash=source_hash,
            current_revision_id=rid,
        )
        return record

    def current_revision(self, document_id: str) -> RevisionRecord:
        doc = self._documents[document_id]
        return self._revisions[doc.current_revision_id]

    def read_revision(self, *, document_id: str, revision_id: str) -> RevisionRecord:
        """Return an isolated immutable-revision snapshot for cloud consumers."""
        record = self._revisions.get(revision_id)
        if record is None or record.document_id != document_id:
            raise KeyError("revision not found for document")
        return copy.deepcopy(record)

    def has_revision(self, *, document_id: str, revision_id: str) -> bool:
        record = self._revisions.get(revision_id)
        return record is not None and record.document_id == document_id

    def commit_move(
        self,
        request: dict,
        executor: AuthoritativeExecutor,
    ) -> dict:
        return self._commit_command(
            request,
            executor,
            request_validator=self._validate_move_request_shape,
            canonical_validator=self._validate_canonical_move,
        )

    def commit_replace_image(
        self,
        request: dict,
        executor: AuthoritativeExecutor,
        *,
        pre_execute_validator: Optional[Callable[[dict], None]] = None,
    ) -> dict:
        return self._commit_command(
            request,
            executor,
            request_validator=self._validate_replace_image_request_shape,
            canonical_validator=self._validate_canonical_replace_image,
            pre_execute_validator=pre_execute_validator,
        )

    def _commit_command(
        self,
        request: dict,
        executor: AuthoritativeExecutor,
        *,
        request_validator: Callable[[dict], None],
        canonical_validator: Callable[[dict, dict], None],
        pre_execute_validator: Optional[Callable[[dict], None]] = None,
    ) -> dict:
        request_validator(request)
        document_id = request["document_id"]
        client_operation_id = request["client_operation_id"]
        request_digest = hash_id(request)
        idem_key = (document_id, client_operation_id)

        prior = self._idempotency.get(idem_key)
        if prior is not None:
            prior_hash, prior_result = prior
            if prior_hash != request_digest:
                return self._rejected(
                    request,
                    code="idempotency_conflict",
                    current_revision_id=self._documents.get(document_id).current_revision_id
                    if document_id in self._documents
                    else None,
                    retryable=False,
                )
            return copy.deepcopy(prior_result)

        if document_id not in self._documents:
            result = self._rejected(
                request,
                code="invalid_command",
                current_revision_id=None,
                retryable=False,
            )
            self._idempotency[idem_key] = (request_digest, copy.deepcopy(result))
            return result

        doc = self._documents[document_id]
        if request["source_hash"] != doc.source_hash:
            result = self._rejected(
                request,
                code="source_hash_mismatch",
                current_revision_id=doc.current_revision_id,
                retryable=False,
            )
            self._idempotency[idem_key] = (request_digest, copy.deepcopy(result))
            return result

        if request["base_revision_id"] != doc.current_revision_id:
            result = self._rejected(
                request,
                code="stale_revision",
                current_revision_id=doc.current_revision_id,
                retryable=True,
            )
            self._idempotency[idem_key] = (request_digest, copy.deepcopy(result))
            return result

        if pre_execute_validator is not None:
            pre_execute_validator(copy.deepcopy(request["command"]))

        base = self._revisions[doc.current_revision_id]
        canonical_operation, resulting_project, consequences = executor(
            copy.deepcopy(base.project),
            copy.deepcopy(request["command"]),
        )

        canonical_validator(request["command"], canonical_operation)
        self._validate_project_source(resulting_project, doc.source_hash)

        transition_digest = hash_id(canonical_operation)
        sid = state_id(document_id, doc.source_hash, resulting_project)
        rid = revision_id(
            document_id,
            doc.source_hash,
            base.revision_id,
            sid,
            "commit",
            transition_digest,
        )
        record = RevisionRecord(
            document_id=document_id,
            source_hash=doc.source_hash,
            revision_id=rid,
            state_id=sid,
            parent_revision_id=base.revision_id,
            project_schema_version=resulting_project["schema_version"],
            project_hash=project_hash(resulting_project),
            transition_kind="commit",
            transition_hash=transition_digest,
            project=copy.deepcopy(resulting_project),
        )

        # Atomic persistence boundary for this bounded in-memory kernel:
        # all validation above must finish before any durable state pointer moves.
        self._revisions[rid] = record
        doc.current_revision_id = rid

        result = {
            "protocol_version": "chaptera.commit-accepted.v1",
            "document_id": document_id,
            "source_hash": doc.source_hash,
            "base_revision_id": base.revision_id,
            "revision_id": rid,
            "state_id": sid,
            "client_operation_id": client_operation_id,
            "canonical_operation": copy.deepcopy(canonical_operation),
            "project_schema_version": resulting_project["schema_version"],
            "consequences": copy.deepcopy(consequences),
            "scene_refresh": "full_snapshot",
        }
        self._idempotency[idem_key] = (request_digest, copy.deepcopy(result))
        return result

    def _rejected(
        self,
        request: dict,
        *,
        code: str,
        current_revision_id: Optional[str],
        retryable: bool,
    ) -> dict:
        return {
            "protocol_version": "chaptera.commit-rejected.v1",
            "document_id": request["document_id"],
            "base_revision_id": request["base_revision_id"],
            "current_revision_id": current_revision_id,
            "client_operation_id": request["client_operation_id"],
            "code": code,
            "message_key": f"revision.{code}",
            "retryable": retryable,
        }

    @staticmethod
    def _validate_move_request_shape(request: dict) -> None:
        command = request.get("command")
        if not isinstance(command, dict) or command.get("kind") != "move_node_to":
            raise ValueError("V1 only accepts move_node_to command intent")
        if "before" in command:
            raise ValueError("browser command cannot carry authoritative before-state")
        for field in ("x_emu", "y_emu"):
            value = command.get(field)
            if (not isinstance(value, int) or isinstance(value, bool)
                    or value < MIN_SAFE_EMU or value > MAX_SAFE_EMU):
                raise ValueError("browser EMU must be a JavaScript-safe integer")

    @staticmethod
    def _validate_replace_image_request_shape(request: dict) -> None:
        command = request.get("command")
        if not isinstance(command, dict) or command.get("kind") != "replace_image":
            raise ValueError("V1 ReplaceImage requires replace_image command intent")
        if "before_asset" in command:
            raise ValueError("browser command cannot carry authoritative before_asset")
        asset_sha256 = command.get("asset_sha256")
        if (
            not isinstance(asset_sha256, str)
            or len(asset_sha256) != 64
            or any(ch not in "0123456789abcdef" for ch in asset_sha256)
        ):
            raise ValueError("replace_image asset_sha256 must be lowercase SHA-256")
        node_id = command.get("node_id")
        if not isinstance(node_id, str) or not node_id:
            raise ValueError("replace_image node_id is required")

    @staticmethod
    def _validate_project_source(project: dict, source_hash: str) -> None:
        if project.get("source_hash") != source_hash:
            raise ValueError("canonical project source hash changed")
        if not isinstance(project.get("schema_version"), str):
            raise ValueError("canonical project schema_version is required")
        if not isinstance(project.get("operations"), list):
            raise ValueError("canonical project operations must be a list")

    @staticmethod
    def _validate_canonical_move(command: dict, operation: dict) -> None:
        if operation.get("kind") != "move_node":
            raise ValueError("authoritative executor returned non-MoveNode operation")
        if operation.get("node_id") != command.get("node_id"):
            raise ValueError("canonical operation targets a different node")
        after = operation.get("after") or {}
        if after.get("x") != command.get("x_emu") or after.get("y") != command.get("y_emu"):
            raise ValueError("canonical MoveNode after-position does not match accepted intent")
        before = operation.get("before")
        if not isinstance(before, dict):
            raise ValueError("authoritative executor must derive canonical before-state")
        for label, rect in (("before", before), ("after", after)):
            for field in ("x", "y", "width", "height"):
                value = rect.get(field)
                if (not isinstance(value, int) or isinstance(value, bool)
                        or value < MIN_SAFE_EMU or value > MAX_SAFE_EMU):
                    raise ValueError(f"canonical {label}.{field} is outside the V1 JavaScript-safe EMU range")
        if before.get("width", 0) <= 0 or before.get("height", 0) <= 0 \
                or after.get("width", 0) <= 0 or after.get("height", 0) <= 0:
            raise ValueError("canonical MoveNode rectangles must have positive width/height")


    @staticmethod
    def _validate_canonical_replace_image(command: dict, operation: dict) -> None:
        if operation.get("kind") != "replace_image":
            raise ValueError("authoritative executor returned non-ReplaceImage operation")
        if operation.get("node_id") != command.get("node_id"):
            raise ValueError("canonical ReplaceImage targets a different node")
        if "before_asset" not in operation:
            raise ValueError("authoritative executor must derive canonical before_asset")
        before_asset = operation.get("before_asset")
        if before_asset is not None and (
            not isinstance(before_asset, str)
            or len(before_asset) != 64
            or any(ch not in "0123456789abcdef" for ch in before_asset)
        ):
            raise ValueError("canonical ReplaceImage before_asset must be SHA-256 or null")
        if operation.get("after_asset") != command.get("asset_sha256"):
            raise ValueError("canonical ReplaceImage after_asset does not match accepted asset")
