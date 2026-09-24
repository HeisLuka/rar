"""AuthZ integration seam for the Chaptera RevisionKernel.

The browser/service caller supplies principal and tenant identity out of band.
Document semantic revision identity remains owned by RevisionKernel; access
state is checked by AuthzKernel and never enters RevisionId/StateId hashing.
"""

from __future__ import annotations

from typing import Any, Callable, Optional

try:
    from .authz_v1 import (
        AuthzKernel,
        CAP_EDIT,
        CAP_EDIT_GEOMETRY,
        CAP_EDIT_TEXT,
    )
except ImportError:
    from authz_v1 import (
        AuthzKernel,
        CAP_EDIT,
        CAP_EDIT_GEOMETRY,
        CAP_EDIT_TEXT,
    )


class AuthorizedRevisionGateway:
    def __init__(self, *, kernel: Any, authz: AuthzKernel, tenant_id: str) -> None:
        self.kernel = kernel
        self.authz = authz
        self.tenant_id = tenant_id

    def commit(
        self,
        request: dict,
        *,
        principal_id: str,
        executor: Optional[Callable] = None,
        history_executor: Optional[Callable] = None,
    ) -> dict:
        protocol = request.get("protocol_version")
        document_id = request.get("document_id")
        if not isinstance(document_id, str):
            raise ValueError("document_id required")

        if protocol == "chaptera.commit-request.v1":
            command = request.get("command")
            if not isinstance(command, dict):
                raise ValueError("command required")
            kind = command.get("kind")
            if kind != "move_node_to":
                raise ValueError("unsupported bounded commit kind")
            if executor is None:
                raise ValueError("authoritative executor required")
            capability = CAP_EDIT_GEOMETRY
            callback = lambda _decision: self.kernel.commit_move(request, executor)
        elif protocol == "chaptera.story-range-intent.v1":
            if executor is None:
                raise ValueError("authoritative executor required")
            capability = CAP_EDIT_TEXT
            callback = lambda _decision: self.kernel.commit_story_range(request, executor)
        elif protocol == "chaptera.story-edit-transaction-intent.v1":
            capability = CAP_EDIT_TEXT
            callback = (
                (lambda _decision: self.kernel.commit_story_edit_transaction(request))
                if executor is None
                else (lambda _decision: self.kernel.commit_story_edit_transaction(request, executor))
            )
        elif protocol == "chaptera.history-transition-intent.v1":
            if history_executor is None:
                raise ValueError("authoritative history executor required")
            capability = CAP_EDIT
            callback = lambda _decision: self.kernel.commit_history_transition(
                request, history_executor
            )
        else:
            raise ValueError("unsupported commit protocol")

        return self.authz.run_authorized(
            tenant_id=self.tenant_id,
            document_id=document_id,
            principal_id=principal_id,
            capability=capability,
            action="revision.commit",
            callback=callback,
        )
