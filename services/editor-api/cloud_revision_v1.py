"""CLOUD-REVISION-01 service bridge for canonical authoring revisions.

The Rar Web revision kernel and the Engine REVISION-MODEL-01 primitive use
*different* revision identities.  This module keeps that boundary explicit:
a Rar service revision is bound to one canonical authoring RevisionId instead
of pretending that the two hashes are interchangeable.

Semantic mutation remains owned by the authoritative canonical executor.  This
module only validates/normalizes the serialized SemanticDiffV1 contract, binds
the canonical child receipt to the immutable Rar service revision, and provides
exact derived-artifact fences.
"""

from __future__ import annotations

import copy
import re
import uuid
from dataclasses import dataclass
from typing import Callable, Dict, Optional, Tuple

from revision_store import RevisionKernel, hash_id


AUTHORING_REVISION_SCHEMA_V1 = "chaptera.cdm.authoring-revision.v1"
SEMANTIC_DIFF_COMMIT_PROTOCOL_V1 = "chaptera.semantic-diff-commit.v1"
DERIVED_ARTIFACT_FENCE_SCHEMA_V1 = "chaptera.derived-artifact-fence.v1"

_CANONICAL_REVISION_RE = re.compile(r"^[0-9a-f]{64}$")
_SHA256_FINGERPRINT_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
_OPERATION_ORDER = {
    "add_entity": 0,
    "remove_entity": 1,
    "update_node_bounds": 2,
    "update_story_text": 3,
    "rebind_entity": 4,
}
_SUPPORTED_MUTATIONS = {"update_node_bounds", "update_story_text"}
_DERIVED_STAGES = {"layout", "scene", "preview", "export"}


class SemanticDiffContractError(ValueError):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(f"{code}: {message}")
        self.code = code


class DerivedArtifactConflict(ValueError):
    pass


def _require_text(value: object, field: str) -> str:
    if not isinstance(value, str) or not value:
        raise SemanticDiffContractError("invalid_field", f"{field} must be non-empty text")
    return value


def _canonical_id_bytes(value: object, field: str) -> bytes:
    text = _require_text(value, field)
    try:
        parsed = uuid.UUID(text)
    except (ValueError, AttributeError) as exc:
        raise SemanticDiffContractError(
            "invalid_canonical_id",
            f"{field} must be canonical 128-bit UUID text",
        ) from exc
    canonical = str(parsed)
    if text.lower() != canonical:
        raise SemanticDiffContractError(
            "invalid_canonical_id",
            f"{field} must use canonical UUID hyphen placement",
        )
    return parsed.bytes


def _require_canonical_revision_id(value: object, field: str) -> str:
    text = _require_text(value, field)
    if not _CANONICAL_REVISION_RE.fullmatch(text):
        raise SemanticDiffContractError(
            "invalid_revision_id",
            f"{field} must be the 64-lowercase-hex REVISION-MODEL-01 RevisionId",
        )
    return text


def _require_i64(value: object, field: str) -> int:
    if (
        not isinstance(value, int)
        or isinstance(value, bool)
        or value < -(2**63)
        or value > 2**63 - 1
    ):
        raise SemanticDiffContractError("invalid_rect", f"{field} must be signed i64 EMU")
    return value


def _normalize_rect(value: object, field: str) -> dict:
    if not isinstance(value, dict) or set(value) != {"x", "y", "width", "height"}:
        raise SemanticDiffContractError(
            "invalid_rect",
            f"{field} must contain exactly x/y/width/height",
        )
    return {
        "x": _require_i64(value["x"], f"{field}.x"),
        "y": _require_i64(value["y"], f"{field}.y"),
        "width": _require_i64(value["width"], f"{field}.width"),
        "height": _require_i64(value["height"], f"{field}.height"),
    }


def _normalize_operation(operation: object, index: int) -> Tuple[Tuple[int, bytes], dict]:
    if not isinstance(operation, dict):
        raise SemanticDiffContractError(
            "invalid_operation",
            f"operations[{index}] must be an object",
        )
    kind = operation.get("kind")
    if kind not in _OPERATION_ORDER:
        raise SemanticDiffContractError(
            "invalid_operation",
            f"operations[{index}].kind is unsupported",
        )

    if kind == "update_node_bounds":
        if set(operation) != {"kind", "node_id", "before", "after"}:
            raise SemanticDiffContractError(
                "invalid_operation",
                "update_node_bounds has non-canonical fields",
            )
        target = _canonical_id_bytes(operation["node_id"], "node_id")
        normalized = {
            "kind": kind,
            "node_id": str(uuid.UUID(bytes=target)),
            "before": _normalize_rect(operation["before"], "before"),
            "after": _normalize_rect(operation["after"], "after"),
        }
    elif kind == "update_story_text":
        if set(operation) != {"kind", "story_id", "before", "after"}:
            raise SemanticDiffContractError(
                "invalid_operation",
                "update_story_text has non-canonical fields",
            )
        target = _canonical_id_bytes(operation["story_id"], "story_id")
        before = operation["before"]
        after = operation["after"]
        if not isinstance(before, str) or not isinstance(after, str):
            raise SemanticDiffContractError(
                "invalid_operation",
                "update_story_text before/after must be strings",
            )
        normalized = {
            "kind": kind,
            "story_id": str(uuid.UUID(bytes=target)),
            "before": before,
            "after": after,
        }
    else:
        if set(operation) != {"kind", "target_id"}:
            raise SemanticDiffContractError(
                "invalid_operation",
                f"{kind} has non-canonical fields",
            )
        target = _canonical_id_bytes(operation["target_id"], "target_id")
        normalized = {
            "kind": kind,
            "target_id": str(uuid.UUID(bytes=target)),
        }

    return (_OPERATION_ORDER[kind], target), normalized


def normalize_semantic_diff_v1(diff: object) -> dict:
    """Normalize the public serialized subset of REVISION-MODEL-01 SemanticDiffV1.

    Ordering and duplicate-target rules mirror pub-model/revision.rs.  Add,
    remove and rebind remain valid *serialized* operation classes but are
    rejected by the current mutation consumer, exactly as the canonical
    apply_source_graph_diff_v1 implementation does.
    """

    if not isinstance(diff, dict) or set(diff) != {
        "schema_version",
        "base_revision_id",
        "operations",
    }:
        raise SemanticDiffContractError(
            "invalid_semantic_diff",
            "SemanticDiffV1 must contain schema_version/base_revision_id/operations",
        )
    if diff["schema_version"] != AUTHORING_REVISION_SCHEMA_V1:
        raise SemanticDiffContractError(
            "unsupported_revision_schema",
            f"expected {AUTHORING_REVISION_SCHEMA_V1}",
        )
    base_revision_id = _require_canonical_revision_id(
        diff["base_revision_id"],
        "base_revision_id",
    )
    operations = diff["operations"]
    if not isinstance(operations, list):
        raise SemanticDiffContractError(
            "invalid_semantic_diff",
            "operations must be a list",
        )

    normalized = [_normalize_operation(item, i) for i, item in enumerate(operations)]
    normalized.sort(key=lambda pair: pair[0])
    for left, right in zip(normalized, normalized[1:]):
        if left[0] == right[0]:
            raise SemanticDiffContractError(
                "duplicate_target",
                "SemanticDiffV1 contains the same canonical operation target twice",
            )

    return {
        "schema_version": AUTHORING_REVISION_SCHEMA_V1,
        "base_revision_id": base_revision_id,
        "operations": [item for _, item in normalized],
    }


def require_supported_semantic_mutations_v1(diff: dict) -> None:
    for operation in diff["operations"]:
        if operation["kind"] not in _SUPPORTED_MUTATIONS:
            raise SemanticDiffContractError(
                "unsupported_operation",
                f"{operation['kind']} is intentionally fail-closed in REVISION-MODEL-01 V1",
            )


def validate_canonical_child_receipt_v1(receipt: object, expected_parent: str) -> dict:
    if not isinstance(receipt, dict) or set(receipt) != {
        "schema_version",
        "revision_id",
        "parent_revision_id",
    }:
        raise SemanticDiffContractError(
            "invalid_canonical_revision_receipt",
            "canonical child receipt must contain schema_version/revision_id/parent_revision_id",
        )
    if receipt["schema_version"] != AUTHORING_REVISION_SCHEMA_V1:
        raise SemanticDiffContractError(
            "unsupported_revision_schema",
            "canonical child uses a different revision schema",
        )
    revision_id = _require_canonical_revision_id(receipt["revision_id"], "revision_id")
    parent_revision_id = _require_canonical_revision_id(
        receipt["parent_revision_id"],
        "parent_revision_id",
    )
    if parent_revision_id != expected_parent:
        raise SemanticDiffContractError(
            "wrong_parent_revision",
            "canonical child is not parented by SemanticDiffV1.base_revision_id",
        )
    if revision_id == parent_revision_id:
        raise SemanticDiffContractError(
            "invalid_child_revision",
            "canonical child revision must differ from its parent",
        )
    return {
        "schema_version": AUTHORING_REVISION_SCHEMA_V1,
        "revision_id": revision_id,
        "parent_revision_id": parent_revision_id,
    }


CanonicalSemanticExecutor = Callable[[dict, dict], Tuple[dict, dict, list]]


class CloudRevisionKernelV1(RevisionKernel):
    """RevisionKernel plus an explicit service↔canonical revision binding."""

    def __init__(self) -> None:
        super().__init__()
        self._canonical_by_service_revision: Dict[Tuple[str, str], str] = {}

    def register_canonical_baseline(
        self,
        *,
        document_id: str,
        source_hash: str,
        project: dict,
        canonical_revision_id: str,
    ):
        canonical_revision_id = _require_canonical_revision_id(
            canonical_revision_id,
            "canonical_revision_id",
        )
        record = super().register_baseline(
            document_id=document_id,
            source_hash=source_hash,
            project=project,
        )
        key = (document_id, record.revision_id)
        prior = self._canonical_by_service_revision.get(key)
        if prior is not None and prior != canonical_revision_id:
            raise SemanticDiffContractError(
                "canonical_binding_conflict",
                "service baseline is already bound to a different canonical revision",
            )
        self._canonical_by_service_revision[key] = canonical_revision_id
        return record

    def canonical_revision_for(self, *, document_id: str, service_revision_id: str) -> str:
        try:
            return self._canonical_by_service_revision[(document_id, service_revision_id)]
        except KeyError as exc:
            raise SemanticDiffContractError(
                "canonical_revision_unbound",
                "service revision has no canonical authoring RevisionId binding",
            ) from exc

    def commit_semantic_diff(
        self,
        request: dict,
        executor: CanonicalSemanticExecutor,
    ) -> dict:
        normalized_request = copy.deepcopy(request)
        command = normalized_request.get("command")
        if not isinstance(command, dict) or command.get("kind") != "apply_semantic_diff_v1":
            raise SemanticDiffContractError(
                "invalid_request",
                "command.kind must be apply_semantic_diff_v1",
            )
        normalized_diff = normalize_semantic_diff_v1(command.get("semantic_diff"))
        require_supported_semantic_mutations_v1(normalized_diff)
        normalized_request["command"] = {
            "kind": "apply_semantic_diff_v1",
            "semantic_diff": normalized_diff,
        }

        def request_validator(candidate: dict) -> None:
            if candidate.get("protocol_version") != SEMANTIC_DIFF_COMMIT_PROTOCOL_V1:
                raise SemanticDiffContractError(
                    "invalid_request",
                    f"protocol_version must be {SEMANTIC_DIFF_COMMIT_PROTOCOL_V1}",
                )
            for field in (
                "document_id",
                "source_hash",
                "base_revision_id",
                "client_operation_id",
            ):
                _require_text(candidate.get(field), field)
            cmd = candidate.get("command")
            if not isinstance(cmd, dict) or set(cmd) != {"kind", "semantic_diff"}:
                raise SemanticDiffContractError(
                    "invalid_request",
                    "semantic diff command contains non-contract fields",
                )
            if cmd.get("kind") != "apply_semantic_diff_v1":
                raise SemanticDiffContractError(
                    "invalid_request",
                    "semantic diff command kind changed during normalization",
                )
            semantic_diff = normalize_semantic_diff_v1(cmd["semantic_diff"])
            require_supported_semantic_mutations_v1(semantic_diff)
            bound = self.canonical_revision_for(
                document_id=candidate["document_id"],
                service_revision_id=candidate["base_revision_id"],
            )
            if semantic_diff["base_revision_id"] != bound:
                raise SemanticDiffContractError(
                    "wrong_base_revision",
                    "SemanticDiffV1 base does not match the canonical revision bound to the Rar base",
                )

        def bound_executor(base_project: dict, cmd: dict):
            semantic_diff = cmd["semantic_diff"]
            child_receipt, resulting_project, consequences = executor(
                base_project,
                copy.deepcopy(semantic_diff),
            )
            child = validate_canonical_child_receipt_v1(
                child_receipt,
                semantic_diff["base_revision_id"],
            )
            canonical_operation = {
                "kind": "apply_semantic_diff_v1",
                "semantic_diff": copy.deepcopy(semantic_diff),
                "canonical_child_revision": child,
            }
            return canonical_operation, resulting_project, consequences

        def canonical_validator(cmd: dict, operation: dict) -> None:
            if not isinstance(operation, dict) or set(operation) != {
                "kind",
                "semantic_diff",
                "canonical_child_revision",
            }:
                raise SemanticDiffContractError(
                    "invalid_canonical_operation",
                    "canonical semantic-diff operation receipt is malformed",
                )
            if operation.get("kind") != "apply_semantic_diff_v1":
                raise SemanticDiffContractError(
                    "invalid_canonical_operation",
                    "canonical operation kind differs from accepted intent",
                )
            if operation.get("semantic_diff") != cmd.get("semantic_diff"):
                raise SemanticDiffContractError(
                    "invalid_canonical_operation",
                    "canonical executor changed the normalized SemanticDiffV1",
                )
            validate_canonical_child_receipt_v1(
                operation["canonical_child_revision"],
                cmd["semantic_diff"]["base_revision_id"],
            )

        result = self._commit_command(
            normalized_request,
            bound_executor,
            request_validator=request_validator,
            canonical_validator=canonical_validator,
        )
        if result.get("protocol_version") == "chaptera.commit-accepted.v1":
            child = result["canonical_operation"]["canonical_child_revision"]["revision_id"]
            key = (result["document_id"], result["revision_id"])
            prior = self._canonical_by_service_revision.get(key)
            if prior is not None and prior != child:
                raise SemanticDiffContractError(
                    "canonical_binding_conflict",
                    "accepted service revision maps to two canonical revisions",
                )
            self._canonical_by_service_revision[key] = child
            result = copy.deepcopy(result)
            result["canonical_revision_id"] = child
            result["canonical_parent_revision_id"] = result["canonical_operation"][
                "canonical_child_revision"
            ]["parent_revision_id"]
        return result


@dataclass(frozen=True)
class DerivedArtifactFenceV1:
    document_id: str
    service_revision_id: str
    canonical_revision_id: str
    stage: str
    stage_version: str
    environment_fingerprint: str
    input_fingerprint: str

    def validate(self) -> None:
        _require_text(self.document_id, "document_id")
        _require_text(self.service_revision_id, "service_revision_id")
        _require_canonical_revision_id(self.canonical_revision_id, "canonical_revision_id")
        if self.stage not in _DERIVED_STAGES:
            raise ValueError(f"unsupported derived-artifact stage: {self.stage}")
        _require_text(self.stage_version, "stage_version")
        for field, value in (
            ("environment_fingerprint", self.environment_fingerprint),
            ("input_fingerprint", self.input_fingerprint),
        ):
            if not _SHA256_FINGERPRINT_RE.fullmatch(value):
                raise ValueError(f"{field} must be sha256:<64 lowercase hex>")

    def as_dict(self) -> dict:
        self.validate()
        return {
            "schema_version": DERIVED_ARTIFACT_FENCE_SCHEMA_V1,
            "document_id": self.document_id,
            "service_revision_id": self.service_revision_id,
            "canonical_revision_id": self.canonical_revision_id,
            "stage": self.stage,
            "stage_version": self.stage_version,
            "environment_fingerprint": self.environment_fingerprint,
            "input_fingerprint": self.input_fingerprint,
        }

    def fence_id(self) -> str:
        return hash_id(self.as_dict())


class DerivedArtifactStoreV1:
    """Non-destructive exact-fence artifact registry.

    A revision/environment/stage change is an exact-key miss, not destructive
    global invalidation. Historical artifacts therefore remain addressable by
    their original fence.
    """

    def __init__(self) -> None:
        self._content_by_fence: Dict[str, str] = {}

    def publish(self, fence: DerivedArtifactFenceV1, content_hash: str) -> str:
        fence_id = fence.fence_id()
        if not _SHA256_FINGERPRINT_RE.fullmatch(content_hash):
            raise ValueError("content_hash must be sha256:<64 lowercase hex>")
        prior = self._content_by_fence.get(fence_id)
        if prior is not None and prior != content_hash:
            raise DerivedArtifactConflict(
                "same exact derived-artifact fence produced different content"
            )
        self._content_by_fence[fence_id] = content_hash
        return fence_id

    def resolve(self, fence: DerivedArtifactFenceV1) -> Optional[str]:
        return self._content_by_fence.get(fence.fence_id())

    def rebuild_required(self, fence: DerivedArtifactFenceV1) -> bool:
        return self.resolve(fence) is None
