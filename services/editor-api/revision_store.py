"""Bounded server-side revision kernel for Chaptera Web Editor V1.

This module owns revision/idempotency bookkeeping only. It deliberately does not
parse PUB files or implement semantic mutations. The caller supplies an
authoritative executor that must be backed by the canonical editor core.
"""

from __future__ import annotations

import copy
import hashlib
import json
import sys
from dataclasses import dataclass
from typing import Any, Callable, Dict, Optional, Tuple

try:
    from authored_stack_v1 import reorder_authored_lane, validate_authored_lane
except ModuleNotFoundError:
    # Some local producer builders load revision_store.py directly via
    # importlib.spec_from_file_location without placing this sibling directory
    # on sys.path. Load the source-neutral sibling explicitly in that case.
    import importlib.util
    import pathlib

    _authored_stack_path = pathlib.Path(__file__).with_name("authored_stack_v1.py")
    _authored_stack_spec = importlib.util.spec_from_file_location(
        "chaptera_authored_stack_v1",
        _authored_stack_path,
    )
    if _authored_stack_spec is None or _authored_stack_spec.loader is None:
        raise ImportError("cannot load authored_stack_v1 sibling module")
    _authored_stack_module = importlib.util.module_from_spec(_authored_stack_spec)
    sys.modules[_authored_stack_spec.name] = _authored_stack_module
    _authored_stack_spec.loader.exec_module(_authored_stack_module)
    reorder_authored_lane = _authored_stack_module.reorder_authored_lane
    validate_authored_lane = _authored_stack_module.validate_authored_lane

try:
    from story_range_v1 import (
        validate_scalar_sequence_v1,
        validate_story_range_operation_v1,
    )
except ModuleNotFoundError:
    import importlib.util
    import pathlib

    _story_range_path = pathlib.Path(__file__).with_name("story_range_v1.py")
    _story_range_spec = importlib.util.spec_from_file_location(
        "chaptera_story_range_v1",
        _story_range_path,
    )
    if _story_range_spec is None or _story_range_spec.loader is None:
        raise ImportError("cannot load story_range_v1 sibling module")
    _story_range_module = importlib.util.module_from_spec(_story_range_spec)
    sys.modules[_story_range_spec.name] = _story_range_module
    _story_range_spec.loader.exec_module(_story_range_module)
    validate_scalar_sequence_v1 = _story_range_module.validate_scalar_sequence_v1
    validate_story_range_operation_v1 = (
        _story_range_module.validate_story_range_operation_v1
    )


try:
    from create_shape_container_v2 import validate_create_shape_v2_intent
except ModuleNotFoundError:
    import importlib.util
    import pathlib

    _create_shape_v2_path = pathlib.Path(__file__).with_name("create_shape_container_v2.py")
    _create_shape_v2_spec = importlib.util.spec_from_file_location(
        "chaptera_create_shape_container_v2",
        _create_shape_v2_path,
    )
    if _create_shape_v2_spec is None or _create_shape_v2_spec.loader is None:
        raise ImportError("cannot load create_shape_container_v2 sibling module")
    _create_shape_v2_module = importlib.util.module_from_spec(_create_shape_v2_spec)
    sys.modules[_create_shape_v2_spec.name] = _create_shape_v2_module
    _create_shape_v2_sibling_dir = str(_create_shape_v2_path.parent)
    _create_shape_v2_added_path = _create_shape_v2_sibling_dir not in sys.path
    if _create_shape_v2_added_path:
        sys.path.insert(0, _create_shape_v2_sibling_dir)
    try:
        _create_shape_v2_spec.loader.exec_module(_create_shape_v2_module)
    finally:
        if _create_shape_v2_added_path:
            sys.path.remove(_create_shape_v2_sibling_dir)
    validate_create_shape_v2_intent = _create_shape_v2_module.validate_create_shape_v2_intent


try:
    from create_shape_v1 import (
        validate_create_shape_intent_v1,
        validate_creation_paint_v1,
        validate_rect_emu_v1,
        validate_uuid7_node_id_v1,
    )
except ModuleNotFoundError:
    import importlib.util
    import pathlib

    _create_shape_path = pathlib.Path(__file__).with_name("create_shape_v1.py")
    _create_shape_spec = importlib.util.spec_from_file_location(
        "chaptera_create_shape_v1",
        _create_shape_path,
    )
    if _create_shape_spec is None or _create_shape_spec.loader is None:
        raise ImportError("cannot load create_shape_v1 sibling module")
    _create_shape_module = importlib.util.module_from_spec(_create_shape_spec)
    sys.modules[_create_shape_spec.name] = _create_shape_module
    _create_shape_spec.loader.exec_module(_create_shape_module)
    validate_create_shape_intent_v1 = _create_shape_module.validate_create_shape_intent_v1
    validate_creation_paint_v1 = _create_shape_module.validate_creation_paint_v1
    validate_rect_emu_v1 = _create_shape_module.validate_rect_emu_v1
    validate_uuid7_node_id_v1 = _create_shape_module.validate_uuid7_node_id_v1

try:
    from authoring_fragment_v1 import (
        canonical_paste_fragment_operation_v1,
        validate_paste_fragment_intent_v1,
    )
except ModuleNotFoundError:
    import importlib.util
    import pathlib

    _authoring_fragment_path = pathlib.Path(__file__).with_name("authoring_fragment_v1.py")
    _authoring_fragment_spec = importlib.util.spec_from_file_location(
        "chaptera_authoring_fragment_v1",
        _authoring_fragment_path,
    )
    if _authoring_fragment_spec is None or _authoring_fragment_spec.loader is None:
        raise ImportError("cannot load authoring_fragment_v1 sibling module")
    _authoring_fragment_module = importlib.util.module_from_spec(_authoring_fragment_spec)
    sys.modules[_authoring_fragment_spec.name] = _authoring_fragment_module
    _authoring_fragment_spec.loader.exec_module(_authoring_fragment_module)
    canonical_paste_fragment_operation_v1 = (
        _authoring_fragment_module.canonical_paste_fragment_operation_v1
    )
    validate_paste_fragment_intent_v1 = (
        _authoring_fragment_module.validate_paste_fragment_intent_v1
    )


try:
    from story_find_replace_v1 import (
        StoryFindReplaceError,
        execute_story_find_replace_v1,
        validate_story_find_replace_operation_v1,
        validate_story_find_replace_request_v1,
    )
except ModuleNotFoundError:
    import pathlib

    _story_find_replace_dir = str(pathlib.Path(__file__).resolve().parent)
    if _story_find_replace_dir not in sys.path:
        sys.path.insert(0, _story_find_replace_dir)
    from story_find_replace_v1 import (
        StoryFindReplaceError,
        execute_story_find_replace_v1,
        validate_story_find_replace_operation_v1,
        validate_story_find_replace_request_v1,
    )

try:
    from story_edit_transaction_v1 import (
        StoryEditTransactionError,
        execute_story_edit_transaction_v1,
        validate_story_edit_transaction_operation_v1,
        validate_story_edit_transaction_request_v1,
    )
except ModuleNotFoundError:
    import pathlib

    _story_edit_dir = str(pathlib.Path(__file__).resolve().parent)
    if _story_edit_dir not in sys.path:
        sys.path.insert(0, _story_edit_dir)
    from story_edit_transaction_v1 import (
        StoryEditTransactionError,
        execute_story_edit_transaction_v1,
        validate_story_edit_transaction_operation_v1,
        validate_story_edit_transaction_request_v1,
    )


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
AuthoritativeHistoryExecutor = Callable[[dict, str], Tuple[dict, list]]


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

    def commit_move_nodes(
        self,
        request: dict,
        executor: AuthoritativeExecutor,
        *,
        pre_execute_validator: Optional[Callable[[dict], None]] = None,
    ) -> dict:
        normalized = copy.deepcopy(request)
        command = normalized.get("command")
        if isinstance(command, dict) and isinstance(command.get("entries"), list):
            command["entries"] = sorted(
                command["entries"],
                key=lambda entry: entry.get("node_id", "")
                if isinstance(entry, dict)
                else "",
            )
        return self._commit_command(
            normalized,
            executor,
            request_validator=self._validate_move_nodes_request_shape,
            canonical_validator=self._validate_canonical_move_nodes,
            pre_execute_validator=pre_execute_validator,
        )

    def commit_resize(
        self,
        request: dict,
        executor: AuthoritativeExecutor,
        *,
        pre_execute_validator: Optional[Callable[[dict], None]] = None,
    ) -> dict:
        return self._commit_command(
            request,
            executor,
            request_validator=self._validate_resize_request_shape,
            canonical_validator=self._validate_canonical_resize,
            pre_execute_validator=pre_execute_validator,
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

    def commit_image_crop(
        self,
        request: dict,
        executor: AuthoritativeExecutor,
        *,
        pre_execute_validator: Optional[Callable[[dict], None]] = None,
    ) -> dict:
        return self._commit_command(
            request,
            executor,
            request_validator=self._validate_image_crop_request_shape,
            canonical_validator=self._validate_canonical_image_crop,
            pre_execute_validator=pre_execute_validator,
        )

    def commit_text_frame_columns(
        self,
        request: dict,
        executor: AuthoritativeExecutor,
        *,
        pre_execute_validator: Optional[Callable[[dict], None]] = None,
    ) -> dict:
        return self._commit_command(
            request,
            executor,
            request_validator=self._validate_text_frame_columns_request_shape,
            canonical_validator=self._validate_canonical_text_frame_columns,
            pre_execute_validator=pre_execute_validator,
        )

    def commit_create_shape(
        self,
        request: dict,
        executor: AuthoritativeExecutor,
    ) -> dict:
        return self._commit_command(
            request,
            executor,
            request_validator=self._validate_create_shape_request_shape,
            canonical_validator=self._validate_canonical_create_shape,
        )

    def commit_create_shape_v2(
        self,
        request: dict,
        executor: AuthoritativeExecutor,
    ) -> dict:
        return self._commit_command(
            request,
            executor,
            request_validator=self._validate_create_shape_v2_request_shape,
            canonical_validator=self._validate_canonical_create_shape_v2,
        )

    def commit_paste_fragment(
        self,
        request: dict,
        executor: AuthoritativeExecutor,
    ) -> dict:
        return self._commit_command(
            request,
            executor,
            request_validator=self._validate_paste_fragment_request_shape,
            canonical_validator=self._validate_canonical_paste_fragment,
        )

    def commit_shape_fill(
        self,
        request: dict,
        executor: AuthoritativeExecutor,
        *,
        pre_execute_validator: Optional[Callable[[dict], None]] = None,
    ) -> dict:
        return self._commit_command(
            request,
            executor,
            request_validator=self._validate_shape_fill_request_shape,
            canonical_validator=self._validate_canonical_shape_fill,
            pre_execute_validator=pre_execute_validator,
        )

    def commit_shape_stroke(
        self,
        request: dict,
        executor: AuthoritativeExecutor,
        *,
        pre_execute_validator: Optional[Callable[[dict], None]] = None,
    ) -> dict:
        return self._commit_command(
            request,
            executor,
            request_validator=self._validate_shape_stroke_request_shape,
            canonical_validator=self._validate_canonical_shape_stroke,
            pre_execute_validator=pre_execute_validator,
        )

    def commit_delete_node(
        self,
        request: dict,
        executor: AuthoritativeExecutor,
        *,
        pre_execute_validator: Optional[Callable[[dict], None]] = None,
    ) -> dict:
        return self._commit_command(
            request,
            executor,
            request_validator=self._validate_delete_node_request_shape,
            canonical_validator=self._validate_canonical_delete_node,
            pre_execute_validator=pre_execute_validator,
        )

    def commit_paragraph_alignment(
        self,
        request: dict,
        executor: AuthoritativeExecutor,
        *,
        pre_execute_validator: Optional[Callable[[dict], None]] = None,
    ) -> dict:
        return self._commit_command(
            request,
            executor,
            request_validator=self._validate_paragraph_alignment_request_shape,
            canonical_validator=self._validate_canonical_paragraph_alignment,
            pre_execute_validator=pre_execute_validator,
        )

    def commit_reorder_authored_stack(
        self,
        request: dict,
        executor: AuthoritativeExecutor,
        *,
        pre_execute_validator: Optional[Callable[[dict], None]] = None,
    ) -> dict:
        return self._commit_command(
            request,
            executor,
            request_validator=self._validate_reorder_authored_stack_request_shape,
            canonical_validator=self._validate_canonical_reorder_authored_stack,
            pre_execute_validator=pre_execute_validator,
        )

    def commit_story_range(
        self,
        request: dict,
        executor: AuthoritativeExecutor,
    ) -> dict:
        return self._commit_command(
            request,
            executor,
            request_validator=self._validate_story_range_request_shape,
            canonical_validator=self._validate_canonical_story_range,
        )

    def commit_story_edit_transaction(
        self,
        request: dict,
        executor: AuthoritativeExecutor = execute_story_edit_transaction_v1,
    ) -> dict:
        """Commit one fully preflighted Story candidate as one revision."""

        def bound_executor(base_project: dict, command: dict):
            operation, resulting_project, consequences = executor(
                base_project,
                command,
            )
            story_id = command["story_id"]
            story_models = resulting_project.get("story_models")
            if (
                not isinstance(story_models, dict)
                or story_models.get(story_id) != operation.get("after_state")
            ):
                raise ValueError(
                    "StoryEditTransactionV1 resulting project is not bound "
                    "to canonical after_state"
                )
            stories = resulting_project.get("stories")
            after_state = operation.get("after_state")
            after_paragraph = (
                after_state.get("paragraph_state")
                if isinstance(after_state, dict)
                else None
            )
            after_text = (
                after_paragraph.get("story_text")
                if isinstance(after_paragraph, dict)
                else None
            )
            if (
                not isinstance(stories, dict)
                or stories.get(story_id) != after_text
            ):
                raise ValueError(
                    "StoryEditTransactionV1 Story text mirror differs "
                    "from canonical after_state"
                )
            return operation, resulting_project, consequences

        try:
            return self._commit_command(
                request,
                bound_executor,
                request_validator=validate_story_edit_transaction_request_v1,
                canonical_validator=validate_story_edit_transaction_operation_v1,
            )
        except StoryEditTransactionError as exc:
            document_id = request["document_id"]
            client_operation_id = request["client_operation_id"]
            request_digest = hash_id(request)
            idem_key = (document_id, client_operation_id)
            current = (
                self._documents[document_id].current_revision_id
                if document_id in self._documents
                else None
            )
            result = self._rejected(
                request,
                code=exc.code,
                current_revision_id=current,
                retryable=False,
            )
            self._idempotency[idem_key] = (
                request_digest,
                copy.deepcopy(result),
            )
            return result

    def commit_story_find_replace(
        self,
        request: dict,
        executor: AuthoritativeExecutor = execute_story_find_replace_v1,
    ) -> dict:
        """Commit one same-Story Find/Replace command as one revision."""

        def bound_executor(base_project: dict, command: dict):
            operation, resulting_project, consequences = executor(base_project, command)
            story_id = command["story_id"]
            story_models = resulting_project.get("story_models")
            stories = resulting_project.get("stories")
            after_state = operation.get("after_state")
            paragraph = after_state.get("paragraph_state") if isinstance(after_state, dict) else None
            after_text = paragraph.get("story_text") if isinstance(paragraph, dict) else None
            if (
                not isinstance(story_models, dict)
                or story_models.get(story_id) != after_state
                or not isinstance(stories, dict)
                or stories.get(story_id) != after_text
            ):
                raise ValueError("StoryFindReplaceV1 resulting project differs from canonical after_state")
            return operation, resulting_project, consequences

        try:
            return self._commit_command(
                request,
                bound_executor,
                request_validator=validate_story_find_replace_request_v1,
                canonical_validator=validate_story_find_replace_operation_v1,
            )
        except StoryFindReplaceError as exc:
            document_id = request["document_id"]
            client_operation_id = request["client_operation_id"]
            request_digest = hash_id(request)
            idem_key = (document_id, client_operation_id)
            current = (
                self._documents[document_id].current_revision_id
                if document_id in self._documents
                else None
            )
            result = self._rejected(
                request,
                code=exc.code,
                current_revision_id=current,
                retryable=False,
            )
            self._idempotency[idem_key] = (request_digest, copy.deepcopy(result))
            return result

    def commit_history_transition(
        self,
        request: dict,
        executor: AuthoritativeHistoryExecutor,
    ) -> dict:
        """Commit authoritative undo/redo as a fresh immutable revision.

        The browser supplies only the history intent. The authoritative executor
        owns the actual EditorSession undo/redo transition and returns the
        resulting canonical EditorProject. A history transition may therefore
        reuse a prior state_id while still receiving a fresh revision_id.
        """
        self._validate_history_request_shape(request)
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

        base = self._revisions[doc.current_revision_id]
        transition_kind = request["command"]["kind"]
        resulting_project, consequences = executor(
            copy.deepcopy(base.project),
            transition_kind,
        )
        self._validate_project_source(resulting_project, doc.source_hash)

        sid = state_id(document_id, doc.source_hash, resulting_project)
        transition_digest = hash_id(
            {
                "protocol_version": "chaptera.history-transition.v1",
                "kind": transition_kind,
                "base_revision_id": base.revision_id,
                "base_state_id": base.state_id,
                "resulting_state_id": sid,
            }
        )
        rid = revision_id(
            document_id,
            doc.source_hash,
            base.revision_id,
            sid,
            transition_kind,
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
            transition_kind=transition_kind,
            transition_hash=transition_digest,
            project=copy.deepcopy(resulting_project),
        )

        self._revisions[rid] = record
        doc.current_revision_id = rid

        result = {
            "protocol_version": "chaptera.history-transition-accepted.v1",
            "document_id": document_id,
            "source_hash": doc.source_hash,
            "base_revision_id": base.revision_id,
            "revision_id": rid,
            "state_id": sid,
            "client_operation_id": client_operation_id,
            "transition_kind": transition_kind,
            "project_schema_version": resulting_project["schema_version"],
            "consequences": copy.deepcopy(consequences),
            "scene_refresh": "full_snapshot",
        }
        self._idempotency[idem_key] = (request_digest, copy.deepcopy(result))
        return result

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
    def _validate_history_request_shape(request: dict) -> None:
        if request.get("protocol_version") != "chaptera.history-transition-intent.v1":
            raise ValueError("V1 history transition protocol_version is required")
        command = request.get("command")
        if not isinstance(command, dict) or set(command) != {"kind"}:
            raise ValueError("history transition contains non-intent/authoritative fields")
        if command.get("kind") not in {"undo", "redo"}:
            raise ValueError("V1 history transition must be undo or redo")

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
    def _validate_move_nodes_rect(rect: dict, label: str) -> None:
        if not isinstance(rect, dict) or set(rect) != {"x", "y", "width", "height"}:
            raise ValueError(f"{label} must contain exact x/y/width/height")
        for field in ("x", "y"):
            value = rect[field]
            if (
                not isinstance(value, int)
                or isinstance(value, bool)
                or value < MIN_SAFE_EMU
                or value > MAX_SAFE_EMU
            ):
                raise ValueError(f"{label}.{field} is outside the V1 JavaScript-safe EMU range")
        for field in ("width", "height"):
            value = rect[field]
            if (
                not isinstance(value, int)
                or isinstance(value, bool)
                or value <= 0
                or value > MAX_SAFE_EMU
            ):
                raise ValueError(f"{label}.{field} must be a positive JavaScript-safe EMU integer")
        if (
            rect["x"] + rect["width"] > MAX_SAFE_EMU
            or rect["x"] + rect["width"] < MIN_SAFE_EMU
            or rect["y"] + rect["height"] > MAX_SAFE_EMU
            or rect["y"] + rect["height"] < MIN_SAFE_EMU
        ):
            raise ValueError(f"{label} overflows the V1 JavaScript-safe EMU range")

    @staticmethod
    def _validate_move_nodes_request_shape(request: dict) -> None:
        if request.get("protocol_version") != "chaptera.move-nodes-intent.v1":
            raise ValueError("V1 MoveNodes protocol_version is required")
        command = request.get("command")
        if (
            not isinstance(command, dict)
            or command.get("kind") != "move_nodes"
            or set(command) != {"kind", "page_id", "entries"}
        ):
            raise ValueError("MoveNodesV1 contains non-intent fields")
        page_id = command.get("page_id")
        if not isinstance(page_id, str) or not page_id:
            raise ValueError("MoveNodesV1 page_id is required")
        entries = command.get("entries")
        if not isinstance(entries, list) or not entries or len(entries) > 1024:
            raise ValueError("MoveNodesV1 entries must be a non-empty bounded list")

        node_ids = []
        for index, entry in enumerate(entries):
            if (
                not isinstance(entry, dict)
                or set(entry) != {"node_id", "expected_before", "after"}
            ):
                raise ValueError(f"MoveNodesV1 entry[{index}] is malformed")
            node_id = entry.get("node_id")
            if not isinstance(node_id, str) or not node_id:
                raise ValueError(f"MoveNodesV1 entry[{index}].node_id is required")
            node_ids.append(node_id)
            RevisionKernel._validate_move_nodes_rect(
                entry.get("expected_before"),
                f"MoveNodesV1 entry[{index}].expected_before",
            )
            RevisionKernel._validate_move_nodes_rect(
                entry.get("after"),
                f"MoveNodesV1 entry[{index}].after",
            )
            before = entry["expected_before"]
            after = entry["after"]
            if before["width"] != after["width"] or before["height"] != after["height"]:
                raise ValueError("MoveNodesV1 entries must encode translation only")
            if before == after:
                raise ValueError("MoveNodesV1 entries must not be no-ops")

        if len(set(node_ids)) != len(node_ids):
            raise ValueError("MoveNodesV1 NodeIds must be unique")
        if node_ids != sorted(node_ids):
            raise ValueError("MoveNodesV1 entries must be normalized by NodeId")

    @staticmethod
    def _validate_resize_request_shape(request: dict) -> None:
        if request.get("protocol_version") != "chaptera.resize-node-intent.v1":
            raise ValueError("V1 ResizeNode protocol_version is required")
        command = request.get("command")
        if not isinstance(command, dict) or command.get("kind") != "resize_node_to":
            raise ValueError("V1 ResizeNode requires resize_node_to command intent")
        allowed = {"kind", "node_id", "x_emu", "y_emu", "width_emu", "height_emu"}
        if set(command) != allowed:
            raise ValueError("resize_node_to contains non-intent/authoritative fields")
        node_id = command.get("node_id")
        if not isinstance(node_id, str) or not node_id:
            raise ValueError("resize_node_to node_id is required")
        for field in ("x_emu", "y_emu"):
            value = command.get(field)
            if (
                not isinstance(value, int)
                or isinstance(value, bool)
                or value < MIN_SAFE_EMU
                or value > MAX_SAFE_EMU
            ):
                raise ValueError("resize origin EMU must be a JavaScript-safe integer")
        for field in ("width_emu", "height_emu"):
            value = command.get(field)
            if (
                not isinstance(value, int)
                or isinstance(value, bool)
                or value <= 0
                or value > MAX_SAFE_EMU
            ):
                raise ValueError("resize size EMU must be a positive JavaScript-safe integer")
        if (
            command["x_emu"] + command["width_emu"] > MAX_SAFE_EMU
            or command["x_emu"] + command["width_emu"] < MIN_SAFE_EMU
            or command["y_emu"] + command["height_emu"] > MAX_SAFE_EMU
            or command["y_emu"] + command["height_emu"] < MIN_SAFE_EMU
        ):
            raise ValueError("resize bounds overflow the V1 JavaScript-safe EMU range")

    @staticmethod
    def _validate_story_range_request_shape(request: dict) -> None:
        if request.get("protocol_version") != "chaptera.story-range-intent.v1":
            raise ValueError("V1 Story range protocol_version is required")
        command = request.get("command")
        if not isinstance(command, dict) or command.get("kind") != "replace_story_range":
            raise ValueError("V1 Story range requires replace_story_range command")
        allowed = {
            "kind",
            "story_id",
            "start_scalar",
            "end_scalar",
            "expected_before",
            "replacement_text",
        }
        if set(command) != allowed:
            raise ValueError("replace_story_range contains non-intent/authoritative fields")
        story_id = command.get("story_id")
        if not isinstance(story_id, str) or not story_id:
            raise ValueError("story_id is required")
        start = command.get("start_scalar")
        end = command.get("end_scalar")
        if (
            not isinstance(start, int)
            or isinstance(start, bool)
            or not isinstance(end, int)
            or isinstance(end, bool)
            or start < 0
            or end < start
            or end > 0xFFFFFFFF
        ):
            raise ValueError("Story scalar range is invalid")
        validate_scalar_sequence_v1(command.get("expected_before"), "expected_before")
        validate_scalar_sequence_v1(command.get("replacement_text"), "replacement_text")
        depends = request.get("depends_on_client_operation_id")
        if depends is not None and (not isinstance(depends, str) or len(depends) < 8):
            raise ValueError("depends_on_client_operation_id is invalid")

    @staticmethod
    def _validate_reorder_authored_stack_request_shape(request: dict) -> None:
        if request.get("protocol_version") != "chaptera.authored-stack-reorder-intent.v1":
            raise ValueError("V1 authored-stack protocol_version is required")
        command = request.get("command")
        allowed = {"kind", "page_id", "node_id", "expected_before", "mode"}
        if (
            not isinstance(command, dict)
            or command.get("kind") != "reorder_authored_stack"
            or set(command) != allowed
        ):
            raise ValueError("ReorderAuthoredStackV1 contains non-intent fields")
        for field in ("page_id", "node_id"):
            value = command.get(field)
            if not isinstance(value, str) or not value:
                raise ValueError(f"ReorderAuthoredStackV1 {field} is required")
        validate_authored_lane(command.get("expected_before"))
        if command["node_id"] not in command["expected_before"]:
            raise ValueError("authored-stack target is not in expected lane")
        # Also validates mode and no-op against the exact expected lane.
        reorder_authored_lane(
            command["expected_before"],
            command["node_id"],
            command.get("mode"),
        )

    @staticmethod
    def _validate_canonical_reorder_authored_stack(command: dict, operation: dict) -> None:
        if operation.get("kind") != "reorder_authored_stack":
            raise ValueError("authoritative executor returned non-authored-stack operation")
        for field in ("page_id", "node_id", "mode"):
            if operation.get(field) != command.get(field):
                raise ValueError(f"canonical authored-stack {field} differs from accepted intent")
        before = operation.get("before")
        after = operation.get("after")
        validate_authored_lane(before)
        validate_authored_lane(after)
        if before != command.get("expected_before"):
            raise ValueError("canonical authored-stack before lane differs from expected precondition")
        expected_after = reorder_authored_lane(
            before,
            command["node_id"],
            command["mode"],
        )
        if after != expected_after:
            raise ValueError("canonical authored-stack after lane violates V1 reorder law")

    @staticmethod
    def _validate_paragraph_alignment_state(state: dict, label: str) -> None:
        if not isinstance(state, dict) or set(state) != {"base_alignment", "override"}:
            raise ValueError(f"{label} must contain exactly base_alignment/override")
        if state.get("base_alignment") not in {
            "left",
            "center",
            "right",
            "interword",
            "distribute",
            "ambiguous",
        }:
            raise ValueError(f"{label}.base_alignment is unsupported")
        override = state.get("override")
        if override is not None and override not in {"left", "center", "right"}:
            raise ValueError(f"{label}.override is unsupported")

    @staticmethod
    def _validate_paragraph_alignment_request_shape(request: dict) -> None:
        if request.get("protocol_version") != "chaptera.paragraph-alignment-intent.v1":
            raise ValueError("V1 paragraph alignment protocol_version is required")
        command = request.get("command")
        if not isinstance(command, dict):
            raise ValueError("paragraph alignment command is required")
        kind = command.get("kind")
        if kind == "set_paragraph_alignment_override":
            allowed = {"kind", "paragraph_ids", "expected_state_ids", "value"}
            if set(command) != allowed:
                raise ValueError("SetParagraphAlignmentOverride contains non-intent fields")
            if command.get("value") not in {"left", "center", "right"}:
                raise ValueError("paragraph alignment value must be left/center/right")
        elif kind == "clear_paragraph_alignment_override":
            allowed = {"kind", "paragraph_ids", "expected_state_ids"}
            if set(command) != allowed:
                raise ValueError("ClearParagraphAlignmentOverride contains non-intent fields")
        else:
            raise ValueError("unsupported V1 paragraph alignment command")

        paragraph_ids = command.get("paragraph_ids")
        if (
            not isinstance(paragraph_ids, list)
            or not paragraph_ids
            or len(paragraph_ids) > 1024
            or any(not isinstance(value, str) or not value for value in paragraph_ids)
            or len(set(paragraph_ids)) != len(paragraph_ids)
        ):
            raise ValueError("paragraph_ids must be a non-empty unique string list")

        expected = command.get("expected_state_ids")
        if not isinstance(expected, dict) or set(expected) != set(paragraph_ids):
            raise ValueError("expected_state_ids must exactly cover paragraph_ids")
        for paragraph_id, state_hash in expected.items():
            if (
                not isinstance(state_hash, str)
                or not state_hash.startswith("sha256:")
                or len(state_hash) != 71
                or any(ch not in "0123456789abcdef" for ch in state_hash[7:])
            ):
                raise ValueError(f"invalid expected state hash for {paragraph_id}")

    @staticmethod
    def _validate_canonical_paragraph_alignment(command: dict, operation: dict) -> None:
        if operation.get("kind") != command.get("kind"):
            raise ValueError("canonical paragraph alignment operation kind mismatch")
        changes = operation.get("changes")
        paragraph_ids = command.get("paragraph_ids")
        if (
            not isinstance(changes, list)
            or [change.get("paragraph_id") for change in changes] != paragraph_ids
        ):
            raise ValueError("canonical paragraph changes must match requested ParagraphIds exactly")

        requested_value = command.get("value")
        for change in changes:
            paragraph_id = change.get("paragraph_id")
            before = change.get("before")
            after = change.get("after")
            RevisionKernel._validate_paragraph_alignment_state(
                before, "canonical before paragraph alignment"
            )
            RevisionKernel._validate_paragraph_alignment_state(
                after, "canonical after paragraph alignment"
            )
            before_state_id = hash_id(before)
            if before_state_id != command["expected_state_ids"][paragraph_id]:
                raise ValueError(
                    "canonical paragraph before-state differs from expected precondition"
                )
            if change.get("before_state_id") != before_state_id:
                raise ValueError("canonical paragraph before_state_id is not bound to before state")
            if after["base_alignment"] != before["base_alignment"]:
                raise ValueError("paragraph alignment operation cannot rewrite base alignment")

            if command["kind"] == "clear_paragraph_alignment_override":
                expected_override = None
            elif before["base_alignment"] == requested_value:
                expected_override = None
            else:
                expected_override = requested_value
            if after["override"] != expected_override:
                raise ValueError("canonical paragraph override normalization mismatch")
            if after == before:
                raise ValueError("canonical paragraph alignment operation contains a no-op target")

    @staticmethod
    def _validate_delete_node_request_shape(request: dict) -> None:
        if request.get("protocol_version") != "chaptera.delete-node-intent.v1":
            raise ValueError("V1 DeleteNode protocol_version is required")
        command = request.get("command")
        allowed = {
            "kind",
            "node_id",
            "expected_state_id",
            "expected_parent_id",
            "expected_child_index",
        }
        if (
            not isinstance(command, dict)
            or command.get("kind") != "delete_node"
            or set(command) != allowed
        ):
            raise ValueError("DeleteNode contains non-intent/authoritative fields")
        for field in ("node_id", "expected_parent_id"):
            value = command.get(field)
            if not isinstance(value, str) or not value:
                raise ValueError(f"DeleteNode {field} is required")
        expected_state_id = command.get("expected_state_id")
        if (
            not isinstance(expected_state_id, str)
            or not expected_state_id.startswith("sha256:")
            or len(expected_state_id) != 71
            or any(ch not in "0123456789abcdef" for ch in expected_state_id[7:])
        ):
            raise ValueError("DeleteNode expected_state_id must be sha256:<lowercase hex>")
        index = command.get("expected_child_index")
        if not isinstance(index, int) or isinstance(index, bool) or index < 0:
            raise ValueError("DeleteNode expected_child_index must be a non-negative integer")

    @staticmethod
    def _validate_canonical_delete_node(command: dict, operation: dict) -> None:
        if operation.get("kind") != "delete_node":
            raise ValueError("authoritative executor returned non-DeleteNode operation")
        if operation.get("node_id") != command.get("node_id"):
            raise ValueError("canonical DeleteNode targets a different node")
        if operation.get("parent_id") != command.get("expected_parent_id"):
            raise ValueError("canonical DeleteNode parent differs from expected precondition")
        if operation.get("child_index") != command.get("expected_child_index"):
            raise ValueError("canonical DeleteNode child index differs from expected precondition")
        before_entity = operation.get("before_entity")
        if not isinstance(before_entity, dict):
            raise ValueError("authoritative executor must derive canonical before_entity")
        before_state_id = hash_id(before_entity)
        if before_state_id != command.get("expected_state_id"):
            raise ValueError("canonical DeleteNode entity state differs from expected precondition")
        if operation.get("before_state_id") != before_state_id:
            raise ValueError("canonical DeleteNode before_state_id is not bound to before_entity")

    @staticmethod
    def _validate_text_frame_columns_state(state: dict, label: str) -> None:
        if not isinstance(state, dict) or set(state) != {"column_count", "gutter_emu"}:
            raise ValueError(f"{label} must contain exactly column_count/gutter_emu")
        column_count = state.get("column_count")
        if (
            not isinstance(column_count, int)
            or isinstance(column_count, bool)
            or column_count < 1
            or column_count > 1024
        ):
            raise ValueError(f"{label}.column_count must be an integer in 1..1024")
        gutter = state.get("gutter_emu")
        if (
            not isinstance(gutter, int)
            or isinstance(gutter, bool)
            or gutter < 0
            or gutter > MAX_SAFE_EMU
        ):
            raise ValueError(f"{label}.gutter_emu must be a non-negative JavaScript-safe integer")

    @staticmethod
    def _validate_text_frame_columns_request_shape(request: dict) -> None:
        if request.get("protocol_version") != "chaptera.text-frame-columns-intent.v1":
            raise ValueError("V1 text-frame columns protocol_version is required")
        command = request.get("command")
        allowed = {"kind", "node_id", "expected_before", "after"}
        if (
            not isinstance(command, dict)
            or command.get("kind") != "set_text_frame_columns"
            or set(command) != allowed
        ):
            raise ValueError("SetTextFrameColumns contains non-intent/authoritative fields")
        node_id = command.get("node_id")
        if not isinstance(node_id, str) or not node_id:
            raise ValueError("SetTextFrameColumns node_id is required")
        RevisionKernel._validate_text_frame_columns_state(
            command.get("expected_before"),
            "expected_before columns",
        )
        RevisionKernel._validate_text_frame_columns_state(
            command.get("after"),
            "after columns",
        )
        if command["expected_before"] == command["after"]:
            raise ValueError("SetTextFrameColumns no-op is not a durable edit")

    @staticmethod
    def _validate_canonical_text_frame_columns(command: dict, operation: dict) -> None:
        if operation.get("kind") != "set_text_frame_columns":
            raise ValueError("authoritative executor returned non-SetTextFrameColumns operation")
        if operation.get("node_id") != command.get("node_id"):
            raise ValueError("canonical SetTextFrameColumns targets a different node")
        before = operation.get("before")
        after = operation.get("after")
        RevisionKernel._validate_text_frame_columns_state(
            before,
            "canonical before columns",
        )
        RevisionKernel._validate_text_frame_columns_state(
            after,
            "canonical after columns",
        )
        if before != command.get("expected_before"):
            raise ValueError(
                "canonical SetTextFrameColumns before-state differs from expected precondition"
            )
        if after != command.get("after"):
            raise ValueError(
                "canonical SetTextFrameColumns after-state differs from accepted intent"
            )
        if before == after:
            raise ValueError("canonical SetTextFrameColumns must change column state")

    @staticmethod
    def _validate_create_shape_request_shape(request: dict) -> None:
        if request.get("protocol_version") != "chaptera.create-shape-intent.v1":
            raise ValueError("V1 CreateShape protocol_version is required")
        validate_create_shape_intent_v1(request.get("command"))

    @staticmethod
    def _validate_create_shape_v2_request_shape(request: dict) -> None:
        if request.get("protocol_version") != "chaptera.create-shape-intent.v2":
            raise ValueError("V2 CreateShape protocol_version is required")
        validate_create_shape_v2_intent(request.get("command"))

    @staticmethod
    def _validate_canonical_create_shape_v2(command: dict, operation: dict) -> None:
        expected_keys = {
            "kind", "node_id", "page_id", "destination", "parent_id",
            "shape_kind", "bounds", "desired_effective_page_rect", "transform",
            "paint", "provenance", "order_before", "order_after",
            "insertion_policy",
        }
        if (
            not isinstance(operation, dict)
            or set(operation) != expected_keys
            or operation.get("kind") != "create_shape_v2"
        ):
            raise ValueError("authoritative executor returned malformed CreateShapeV2 operation")
        if operation.get("node_id") != command.get("node_id"):
            raise ValueError("canonical CreateShapeV2 NodeId differs from accepted intent")
        if operation.get("page_id") != command.get("page_id"):
            raise ValueError("canonical CreateShapeV2 page differs from accepted intent")
        if operation.get("destination") != command.get("destination"):
            raise ValueError("canonical CreateShapeV2 destination differs from accepted intent")
        if operation.get("parent_id") != command.get("destination", {}).get("id"):
            raise ValueError("canonical CreateShapeV2 parent differs from accepted destination")
        if operation.get("shape_kind") != "rectangle":
            raise ValueError("canonical CreateShapeV2 must create ordinary rectangle")
        if operation.get("transform") != {"kind": "identity"}:
            raise ValueError("canonical CreateShapeV2 transform must be identity")
        if operation.get("provenance") != {"kind": "author_created"}:
            raise ValueError("canonical CreateShapeV2 provenance must be author_created")
        validate_uuid7_node_id_v1(operation.get("node_id"))
        validate_rect_emu_v1(operation.get("bounds"), "canonical CreateShapeV2 bounds")
        placement = command.get("placement", {})
        if operation.get("bounds") != placement.get("destination_local_rect"):
            raise ValueError("canonical CreateShapeV2 local bounds differ from placement")
        if operation.get("desired_effective_page_rect") != placement.get("desired_effective_page_rect"):
            raise ValueError("canonical CreateShapeV2 effective bounds differ from placement")
        if operation.get("order_before") != command.get("expected_order_lane"):
            raise ValueError("canonical CreateShapeV2 order_before differs from precondition")
        expected_after = list(command.get("expected_order_lane", [])) + [command.get("node_id")]
        if operation.get("order_after") != expected_after:
            raise ValueError("canonical CreateShapeV2 order_after violates append-front policy")
        if operation.get("insertion_policy") != command.get("insertion_policy"):
            raise ValueError("canonical CreateShapeV2 insertion policy differs from intent")
        paint = operation.get("paint")
        if not isinstance(paint, dict) or set(paint) != {"fill", "stroke", "provenance"}:
            raise ValueError("canonical CreateShapeV2 paint must contain fill/stroke/provenance")
        if paint.get("provenance") != {"kind": "author_created"}:
            raise ValueError("canonical CreateShapeV2 paint provenance must be author_created")
        validate_creation_paint_v1({"fill": paint.get("fill"), "stroke": paint.get("stroke")})
        if paint.get("fill") != command.get("paint", {}).get("fill"):
            raise ValueError("canonical CreateShapeV2 fill differs from accepted intent")
        if paint.get("stroke") != command.get("paint", {}).get("stroke"):
            raise ValueError("canonical CreateShapeV2 stroke differs from accepted intent")

    @staticmethod
    def _validate_canonical_create_shape(command: dict, operation: dict) -> None:
        expected_keys = {
            "kind",
            "node_id",
            "page_id",
            "parent_id",
            "shape_kind",
            "bounds",
            "transform",
            "paint",
            "provenance",
        }
        if (
            not isinstance(operation, dict)
            or set(operation) != expected_keys
            or operation.get("kind") != "create_shape"
        ):
            raise ValueError("authoritative executor returned malformed CreateShape operation")
        if operation.get("node_id") != command.get("node_id"):
            raise ValueError("canonical CreateShape NodeId differs from accepted intent")
        if operation.get("page_id") != command.get("page_id"):
            raise ValueError("canonical CreateShape page differs from accepted intent")
        if operation.get("parent_id") != command.get("page_id"):
            raise ValueError("canonical CreateShape parent must equal accepted page")
        if operation.get("shape_kind") != "rectangle":
            raise ValueError("canonical CreateShape must create ordinary rectangle")
        if operation.get("transform") != {"kind": "identity"}:
            raise ValueError("canonical CreateShape transform must be identity")
        if operation.get("provenance") != {"kind": "author_created"}:
            raise ValueError("canonical CreateShape entity provenance must be author_created")

        validate_uuid7_node_id_v1(operation.get("node_id"))
        validate_rect_emu_v1(operation.get("bounds"), "canonical CreateShape bounds")
        if operation.get("bounds") != command.get("bounds"):
            raise ValueError("canonical CreateShape bounds differ from accepted intent")

        paint = operation.get("paint")
        if not isinstance(paint, dict) or set(paint) != {"fill", "stroke", "provenance"}:
            raise ValueError("canonical CreateShape paint must contain fill/stroke/provenance")
        if paint.get("provenance") != {"kind": "author_created"}:
            raise ValueError("canonical CreateShape paint provenance must be author_created")
        validate_creation_paint_v1(
            {
                "fill": paint.get("fill"),
                "stroke": paint.get("stroke"),
            }
        )
        if paint.get("fill") != command.get("paint", {}).get("fill"):
            raise ValueError("canonical CreateShape fill differs from accepted intent")
        if paint.get("stroke") != command.get("paint", {}).get("stroke"):
            raise ValueError("canonical CreateShape stroke differs from accepted intent")

    @staticmethod
    def _validate_paste_fragment_request_shape(request: dict) -> None:
        if request.get("protocol_version") != "chaptera.paste-fragment-intent.v1":
            raise ValueError("V1 PasteFragment protocol_version is required")
        validate_paste_fragment_intent_v1(request.get("command"))

    @staticmethod
    def _validate_canonical_paste_fragment(command: dict, operation: dict) -> None:
        expected = canonical_paste_fragment_operation_v1(command)
        if operation != expected:
            raise ValueError("authoritative executor returned non-canonical PasteFragment operation")

    @staticmethod
    def _validate_srgb_color(color: dict, label: str) -> None:
        if not isinstance(color, dict) or set(color) != {"r", "g", "b"}:
            raise ValueError(f"{label} must contain exactly r/g/b")
        for channel in ("r", "g", "b"):
            value = color.get(channel)
            if (
                not isinstance(value, int)
                or isinstance(value, bool)
                or value < 0
                or value > 255
            ):
                raise ValueError(f"{label}.{channel} must be an sRGB byte")

    @staticmethod
    def _validate_shape_fill_state(fill: dict, label: str) -> None:
        if not isinstance(fill, dict) or set(fill) != {"visible", "color"}:
            raise ValueError(f"{label} must contain exactly visible/color")
        if not isinstance(fill.get("visible"), bool):
            raise ValueError(f"{label}.visible must be boolean")
        RevisionKernel._validate_srgb_color(fill.get("color"), f"{label}.color")

    @staticmethod
    def _validate_shape_stroke_state(stroke: dict, label: str) -> None:
        if not isinstance(stroke, dict) or set(stroke) != {"visible", "color", "width_emu"}:
            raise ValueError(f"{label} must contain exactly visible/color/width_emu")
        if not isinstance(stroke.get("visible"), bool):
            raise ValueError(f"{label}.visible must be boolean")
        RevisionKernel._validate_srgb_color(stroke.get("color"), f"{label}.color")
        width = stroke.get("width_emu")
        if (
            not isinstance(width, int)
            or isinstance(width, bool)
            or width <= 0
            or width > MAX_SAFE_EMU
        ):
            raise ValueError(f"{label}.width_emu must be a positive JavaScript-safe EMU integer")

    @staticmethod
    def _validate_shape_fill_request_shape(request: dict) -> None:
        if request.get("protocol_version") != "chaptera.shape-fill-intent.v1":
            raise ValueError("V1 shape fill protocol_version is required")
        command = request.get("command")
        allowed = {"kind", "node_id", "expected_before", "after"}
        if (
            not isinstance(command, dict)
            or command.get("kind") != "set_fill"
            or set(command) != allowed
        ):
            raise ValueError("SetFill contains non-intent/authoritative fields")
        node_id = command.get("node_id")
        if not isinstance(node_id, str) or not node_id:
            raise ValueError("SetFill node_id is required")
        RevisionKernel._validate_shape_fill_state(
            command.get("expected_before"),
            "expected_before fill",
        )
        RevisionKernel._validate_shape_fill_state(command.get("after"), "after fill")
        if command["expected_before"] == command["after"]:
            raise ValueError("SetFill no-op is not a durable edit")

    @staticmethod
    def _validate_shape_stroke_request_shape(request: dict) -> None:
        if request.get("protocol_version") != "chaptera.shape-stroke-intent.v1":
            raise ValueError("V1 shape stroke protocol_version is required")
        command = request.get("command")
        allowed = {"kind", "node_id", "expected_before", "after"}
        if (
            not isinstance(command, dict)
            or command.get("kind") != "set_stroke"
            or set(command) != allowed
        ):
            raise ValueError("SetStroke contains non-intent/authoritative fields")
        node_id = command.get("node_id")
        if not isinstance(node_id, str) or not node_id:
            raise ValueError("SetStroke node_id is required")
        RevisionKernel._validate_shape_stroke_state(
            command.get("expected_before"),
            "expected_before stroke",
        )
        RevisionKernel._validate_shape_stroke_state(command.get("after"), "after stroke")
        if command["expected_before"] == command["after"]:
            raise ValueError("SetStroke no-op is not a durable edit")

    @staticmethod
    def _validate_canonical_shape_fill(command: dict, operation: dict) -> None:
        if (
            not isinstance(operation, dict)
            or set(operation) != {"kind", "node_id", "before", "after"}
            or operation.get("kind") != "set_fill"
        ):
            raise ValueError("authoritative executor returned malformed SetFill operation")
        if operation.get("node_id") != command.get("node_id"):
            raise ValueError("canonical SetFill targets a different node")
        RevisionKernel._validate_shape_fill_state(operation.get("before"), "canonical before fill")
        RevisionKernel._validate_shape_fill_state(operation.get("after"), "canonical after fill")
        if operation["before"] != command.get("expected_before"):
            raise ValueError("canonical SetFill before-state differs from expected precondition")
        if operation["after"] != command.get("after"):
            raise ValueError("canonical SetFill after-state differs from accepted intent")
        if operation["before"] == operation["after"]:
            raise ValueError("canonical SetFill must change fill state")

    @staticmethod
    def _validate_canonical_shape_stroke(command: dict, operation: dict) -> None:
        if (
            not isinstance(operation, dict)
            or set(operation) != {"kind", "node_id", "before", "after"}
            or operation.get("kind") != "set_stroke"
        ):
            raise ValueError("authoritative executor returned malformed SetStroke operation")
        if operation.get("node_id") != command.get("node_id"):
            raise ValueError("canonical SetStroke targets a different node")
        RevisionKernel._validate_shape_stroke_state(
            operation.get("before"),
            "canonical before stroke",
        )
        RevisionKernel._validate_shape_stroke_state(
            operation.get("after"),
            "canonical after stroke",
        )
        if operation["before"] != command.get("expected_before"):
            raise ValueError("canonical SetStroke before-state differs from expected precondition")
        if operation["after"] != command.get("after"):
            raise ValueError("canonical SetStroke after-state differs from accepted intent")
        if operation["before"] == operation["after"]:
            raise ValueError("canonical SetStroke must change stroke state")

    @staticmethod
    def _validate_crop_state(crop: dict, label: str) -> None:
        if not isinstance(crop, dict) or set(crop) != {"left", "top", "right", "bottom"}:
            raise ValueError(f"{label} must contain exactly left/top/right/bottom")
        for side in ("left", "top", "right", "bottom"):
            value = crop.get(side)
            if (
                not isinstance(value, int)
                or isinstance(value, bool)
                or value < MIN_SAFE_EMU
                or value > MAX_SAFE_EMU
            ):
                raise ValueError(f"{label}.{side} must be a JavaScript-safe integer")

    @staticmethod
    def _validate_image_crop_request_shape(request: dict) -> None:
        if request.get("protocol_version") != "chaptera.image-crop-intent.v1":
            raise ValueError("V1 image crop protocol_version is required")
        command = request.get("command")
        if not isinstance(command, dict) or command.get("kind") != "set_image_crop":
            raise ValueError("V1 image crop requires set_image_crop command")
        allowed = {"kind", "node_id", "expected_before", "after"}
        if set(command) != allowed:
            raise ValueError("set_image_crop contains non-intent/authoritative fields")
        node_id = command.get("node_id")
        if not isinstance(node_id, str) or not node_id:
            raise ValueError("set_image_crop node_id is required")
        RevisionKernel._validate_crop_state(command.get("expected_before"), "expected_before")
        RevisionKernel._validate_crop_state(command.get("after"), "after")
        if command["expected_before"] == command["after"]:
            raise ValueError("set_image_crop no-op is not admitted")

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
    def _validate_canonical_move_nodes(command: dict, operation: dict) -> None:
        if (
            not isinstance(operation, dict)
            or operation.get("kind") != "move_nodes"
            or set(operation) != {"kind", "page_id", "entries"}
        ):
            raise ValueError("authoritative executor returned malformed MoveNodes operation")
        if operation.get("page_id") != command.get("page_id"):
            raise ValueError("canonical MoveNodes page differs from accepted intent")
        entries = operation.get("entries")
        requested = command.get("entries")
        if not isinstance(entries, list) or len(entries) != len(requested):
            raise ValueError("canonical MoveNodes entry count differs from accepted intent")

        canonical_ids = []
        for index, (expected, actual) in enumerate(zip(requested, entries)):
            if (
                not isinstance(actual, dict)
                or set(actual) != {"node_id", "before", "after"}
            ):
                raise ValueError(f"canonical MoveNodes entry[{index}] is malformed")
            if actual.get("node_id") != expected.get("node_id"):
                raise ValueError("canonical MoveNodes NodeId order differs from normalized intent")
            canonical_ids.append(actual["node_id"])
            RevisionKernel._validate_move_nodes_rect(
                actual.get("before"),
                f"canonical MoveNodes entry[{index}].before",
            )
            RevisionKernel._validate_move_nodes_rect(
                actual.get("after"),
                f"canonical MoveNodes entry[{index}].after",
            )
            if actual["before"] != expected["expected_before"]:
                raise ValueError("canonical MoveNodes before-state differs from expected precondition")
            if actual["after"] != expected["after"]:
                raise ValueError("canonical MoveNodes after-state differs from accepted intent")
            if (
                actual["before"]["width"] != actual["after"]["width"]
                or actual["before"]["height"] != actual["after"]["height"]
            ):
                raise ValueError("canonical MoveNodes entries must encode translation only")
            if actual["before"] == actual["after"]:
                raise ValueError("canonical MoveNodes entries must not be no-ops")

        if canonical_ids != sorted(canonical_ids) or len(set(canonical_ids)) != len(canonical_ids):
            raise ValueError("canonical MoveNodes entries are not uniquely normalized by NodeId")

    @staticmethod
    def _validate_canonical_resize(command: dict, operation: dict) -> None:
        if operation.get("kind") != "resize_node":
            raise ValueError("authoritative executor returned non-ResizeNode operation")
        if operation.get("node_id") != command.get("node_id"):
            raise ValueError("canonical ResizeNode targets a different node")
        before = operation.get("before")
        after = operation.get("after")
        if not isinstance(before, dict) or not isinstance(after, dict):
            raise ValueError("authoritative executor must derive exact ResizeNode before/after")
        expected_after = {
            "x": command.get("x_emu"),
            "y": command.get("y_emu"),
            "width": command.get("width_emu"),
            "height": command.get("height_emu"),
        }
        if after != expected_after:
            raise ValueError("canonical ResizeNode after-bounds do not match accepted intent")
        for label, rect in (("before", before), ("after", after)):
            if set(rect) != {"x", "y", "width", "height"}:
                raise ValueError(f"canonical ResizeNode {label} bounds are malformed")
            for field in ("x", "y", "width", "height"):
                value = rect[field]
                if (
                    not isinstance(value, int)
                    or isinstance(value, bool)
                    or value < MIN_SAFE_EMU
                    or value > MAX_SAFE_EMU
                ):
                    raise ValueError(
                        f"canonical ResizeNode {label}.{field} is outside the V1 JavaScript-safe EMU range"
                    )
            if rect["width"] <= 0 or rect["height"] <= 0:
                raise ValueError("canonical ResizeNode rectangles must have positive width/height")
            if (
                rect["x"] + rect["width"] > MAX_SAFE_EMU
                or rect["x"] + rect["width"] < MIN_SAFE_EMU
                or rect["y"] + rect["height"] > MAX_SAFE_EMU
                or rect["y"] + rect["height"] < MIN_SAFE_EMU
            ):
                raise ValueError("canonical ResizeNode bounds overflow the V1 JavaScript-safe EMU range")
        if before == after:
            raise ValueError("canonical ResizeNode no-op is not admitted")
        if before["width"] == after["width"] and before["height"] == after["height"]:
            raise ValueError("canonical ResizeNode cannot encode a pure move")

    @staticmethod
    def _validate_canonical_story_range(command: dict, operation: dict) -> None:
        validate_story_range_operation_v1(command, operation)

    @staticmethod
    def _validate_canonical_image_crop(command: dict, operation: dict) -> None:
        if operation.get("kind") != "set_image_crop":
            raise ValueError("authoritative executor returned non-SetImageCrop operation")
        if operation.get("node_id") != command.get("node_id"):
            raise ValueError("canonical SetImageCrop targets a different node")
        before = operation.get("before")
        after = operation.get("after")
        RevisionKernel._validate_crop_state(before, "canonical before crop")
        RevisionKernel._validate_crop_state(after, "canonical after crop")
        if before != command.get("expected_before"):
            raise ValueError("canonical SetImageCrop before-state differs from expected precondition")
        if after != command.get("after"):
            raise ValueError("canonical SetImageCrop after-state differs from accepted intent")
        if before == after:
            raise ValueError("canonical SetImageCrop must change crop state")

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
