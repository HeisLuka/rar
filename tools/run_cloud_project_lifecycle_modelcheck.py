#!/usr/bin/env python3
"""Bounded state-space discriminator for the Cloud Project lifecycle contract.

This is an architecture/model experiment, not product implementation. It compares:
1) one generation that advances only on lifecycle transitions,
2) one generation that advances on every metadata/lifecycle mutation,
3) split lifecycle_generation + metadata_version.

The experiment detects whether one counter can simultaneously fence stale metadata
writes and avoid invalidating active edit sessions on harmless rename/move operations.
"""

from __future__ import annotations

import dataclasses
import hashlib
import json
from collections import deque
from pathlib import Path
from typing import Callable, Iterable

OUT = Path("target/cloud-project-lifecycle-modelcheck/receipt.json")


@dataclasses.dataclass(frozen=True)
class Project:
    project_id: str = "project:root"
    document_id: str = "document:root"
    revision_id: str = "revision:genesis"
    source_hash: str = "sha256:" + "a" * 64
    workspace: str = "workspace:a"
    tenant: str = "tenant:1"
    name: str = "Untitled"
    lifecycle: str = "active"
    generation: int = 0
    metadata_version: int = 0
    grants: tuple[str, ...] = ("owner:alice",)
    deleted: bool = False


@dataclasses.dataclass(frozen=True)
class Result:
    accepted: bool
    state: Project
    reason: str


def stable_id(prefix: str, seed: str) -> str:
    digest = hashlib.sha256(seed.encode("utf-8")).hexdigest()[:16]
    return f"{prefix}:{digest}"


class Model:
    def __init__(self, policy: str):
        self.policy = policy

    def _check_lifecycle(self, s: Project, expected_generation: int) -> str | None:
        if s.deleted:
            return "deleted"
        if expected_generation != s.generation:
            return "stale_lifecycle_generation"
        return None

    def rename(
        self,
        s: Project,
        *,
        expected_generation: int,
        expected_metadata_version: int,
        name: str,
    ) -> Result:
        problem = self._check_lifecycle(s, expected_generation)
        if problem:
            return Result(False, s, problem)
        if s.lifecycle != "active":
            return Result(False, s, "not_active")
        if self.policy == "split" and expected_metadata_version != s.metadata_version:
            return Result(False, s, "stale_metadata_version")
        generation = s.generation + (1 if self.policy == "single_all_mutations" else 0)
        metadata_version = s.metadata_version + (1 if self.policy == "split" else 0)
        return Result(
            True,
            dataclasses.replace(
                s,
                name=name,
                generation=generation,
                metadata_version=metadata_version,
            ),
            "accepted",
        )

    def move_same_tenant(
        self,
        s: Project,
        *,
        expected_generation: int,
        expected_metadata_version: int,
        workspace: str,
    ) -> Result:
        problem = self._check_lifecycle(s, expected_generation)
        if problem:
            return Result(False, s, problem)
        if s.lifecycle != "active":
            return Result(False, s, "not_active")
        if self.policy == "split" and expected_metadata_version != s.metadata_version:
            return Result(False, s, "stale_metadata_version")
        generation = s.generation + (1 if self.policy == "single_all_mutations" else 0)
        metadata_version = s.metadata_version + (1 if self.policy == "split" else 0)
        return Result(
            True,
            dataclasses.replace(
                s,
                workspace=workspace,
                generation=generation,
                metadata_version=metadata_version,
            ),
            "accepted",
        )

    def cross_tenant_move(
        self,
        s: Project,
        *,
        expected_generation: int,
        tenant: str,
    ) -> Result:
        problem = self._check_lifecycle(s, expected_generation)
        if problem:
            return Result(False, s, problem)
        return Result(False, s, "cross_tenant_identity_preserving_move_not_v0")

    def trash(self, s: Project, *, expected_generation: int) -> Result:
        problem = self._check_lifecycle(s, expected_generation)
        if problem:
            return Result(False, s, problem)
        if s.lifecycle != "active":
            return Result(False, s, "not_active")
        return Result(
            True,
            dataclasses.replace(s, lifecycle="trashed", generation=s.generation + 1),
            "accepted",
        )

    def restore(self, s: Project, *, expected_generation: int) -> Result:
        problem = self._check_lifecycle(s, expected_generation)
        if problem:
            return Result(False, s, problem)
        if s.lifecycle != "trashed":
            return Result(False, s, "not_trashed")
        return Result(
            True,
            dataclasses.replace(s, lifecycle="active", generation=s.generation + 1),
            "accepted",
        )

    def hard_delete(self, s: Project, *, expected_generation: int) -> Result:
        problem = self._check_lifecycle(s, expected_generation)
        if problem:
            return Result(False, s, problem)
        if s.lifecycle != "trashed":
            return Result(False, s, "must_trash_before_delete")
        return Result(
            True,
            dataclasses.replace(
                s,
                lifecycle="deleted",
                deleted=True,
                generation=s.generation + 1,
            ),
            "accepted",
        )

    def fork(self, s: Project, *, selected_revision: str, request_id: str) -> Project:
        return Project(
            project_id=stable_id("project", request_id),
            document_id=stable_id("document", request_id),
            revision_id=stable_id("revision:genesis", request_id + selected_revision),
            source_hash=s.source_hash,
            workspace=s.workspace,
            tenant=s.tenant,
            name=s.name + " copy",
            lifecycle="active",
            generation=0,
            metadata_version=0,
            grants=(),
            deleted=False,
        )


def metadata_race(policy: str) -> dict:
    model = Model(policy)
    initial = Project()
    expected_generation = initial.generation
    expected_metadata = initial.metadata_version

    first = model.rename(
        initial,
        expected_generation=expected_generation,
        expected_metadata_version=expected_metadata,
        name="A",
    )
    second = model.rename(
        first.state,
        expected_generation=expected_generation,
        expected_metadata_version=expected_metadata,
        name="B",
    )
    return {
        "policy": policy,
        "first_accepted": first.accepted,
        "second_stale_metadata_command_accepted": second.accepted,
        "second_reason": second.reason,
        "generation_before": initial.generation,
        "generation_after_first_rename": first.state.generation,
        "metadata_version_before": initial.metadata_version,
        "metadata_version_after_first_rename": first.state.metadata_version,
        "active_edit_session_generation_token_still_valid_after_rename":
            first.state.generation == initial.generation,
    }


def lifecycle_state_space(depth: int = 6) -> dict:
    model = Model("split")
    root = Project()
    queue = deque([(root, ())])
    seen = {root}
    transitions = 0
    rejected = {}
    violations: list[str] = []

    def operations(s: Project) -> Iterable[tuple[str, Callable[[], Result]]]:
        yield "rename", lambda: model.rename(
            s,
            expected_generation=s.generation,
            expected_metadata_version=s.metadata_version,
            name="N" + str(s.metadata_version + 1),
        )
        yield "move_same_tenant", lambda: model.move_same_tenant(
            s,
            expected_generation=s.generation,
            expected_metadata_version=s.metadata_version,
            workspace="workspace:b" if s.workspace == "workspace:a" else "workspace:a",
        )
        yield "trash", lambda: model.trash(s, expected_generation=s.generation)
        yield "restore", lambda: model.restore(s, expected_generation=s.generation)
        yield "hard_delete", lambda: model.hard_delete(s, expected_generation=s.generation)
        yield "cross_tenant_move", lambda: model.cross_tenant_move(
            s, expected_generation=s.generation, tenant="tenant:2"
        )

    while queue:
        state, history = queue.popleft()
        if len(history) >= depth:
            continue
        for name, op in operations(state):
            result = op()
            transitions += 1
            if not result.accepted:
                rejected[result.reason] = rejected.get(result.reason, 0) + 1
                continue

            nxt = result.state
            if name in {"rename", "move_same_tenant"}:
                if nxt.document_id != state.document_id:
                    violations.append(name + ": changed DocumentId")
                if nxt.revision_id != state.revision_id:
                    violations.append(name + ": changed semantic revision")
                if nxt.generation != state.generation:
                    violations.append(name + ": changed lifecycle generation")
                if nxt.metadata_version != state.metadata_version + 1:
                    violations.append(name + ": did not advance metadata_version")

            if name in {"trash", "restore", "hard_delete"}:
                if nxt.document_id != state.document_id:
                    violations.append(name + ": changed DocumentId")
                if nxt.revision_id != state.revision_id:
                    violations.append(name + ": changed semantic revision")
                if nxt.generation != state.generation + 1:
                    violations.append(name + ": lifecycle generation not monotonic")

            if state.deleted and result.accepted:
                violations.append(name + ": mutation accepted after hard delete")

            if nxt not in seen:
                seen.add(nxt)
                queue.append((nxt, history + (name,)))

    # Explicit stale-generation fence.
    trashed = model.trash(root, expected_generation=0)
    stale_restore = model.restore(trashed.state, expected_generation=0)
    if stale_restore.accepted or stale_restore.reason != "stale_lifecycle_generation":
        violations.append("stale lifecycle generation did not fail closed")

    # Hard delete must be terminal and DocumentId never reusable by a fork.
    restored = model.restore(trashed.state, expected_generation=1)
    trashed2 = model.trash(restored.state, expected_generation=2)
    deleted = model.hard_delete(trashed2.state, expected_generation=3)
    post_delete_rename = model.rename(
        deleted.state,
        expected_generation=deleted.state.generation,
        expected_metadata_version=deleted.state.metadata_version,
        name="resurrected",
    )
    if post_delete_rename.accepted:
        violations.append("hard-deleted project resurrected")

    forks = [
        model.fork(root, selected_revision=root.revision_id, request_id=f"fork-{i}")
        for i in range(100)
    ]
    fork_doc_ids = {p.document_id for p in forks}
    fork_project_ids = {p.project_id for p in forks}
    if len(fork_doc_ids) != 100 or root.document_id in fork_doc_ids:
        violations.append("fork DocumentId uniqueness/non-reuse failed")
    if len(fork_project_ids) != 100 or root.project_id in fork_project_ids:
        violations.append("fork ProjectId uniqueness/non-reuse failed")
    if any(p.grants for p in forks):
        violations.append("fork inherited grants")
    if any(p.revision_id == root.revision_id for p in forks):
        violations.append("fork reused historical RevisionId")

    return {
        "depth": depth,
        "reachable_states": len(seen),
        "transitions_explored": transitions,
        "rejection_counts": rejected,
        "stale_lifecycle_restore": {
            "accepted": stale_restore.accepted,
            "reason": stale_restore.reason,
        },
        "hard_delete_terminal": not post_delete_rename.accepted,
        "forks_checked": len(forks),
        "fork_document_ids_unique": len(fork_doc_ids) == len(forks),
        "fork_project_ids_unique": len(fork_project_ids) == len(forks),
        "violations": violations,
    }


def main() -> None:
    OUT.parent.mkdir(parents=True, exist_ok=True)

    races = {
        policy: metadata_race(policy)
        for policy in ("single_lifecycle_only", "single_all_mutations", "split")
    }
    space = lifecycle_state_space(depth=6)

    conclusions = {
        "single_lifecycle_only_fails_to_fence_concurrent_metadata_write":
            races["single_lifecycle_only"]["second_stale_metadata_command_accepted"],
        "single_all_mutations_fences_metadata_but_invalidates_lifecycle_token_on_rename":
            not races["single_all_mutations"][
                "active_edit_session_generation_token_still_valid_after_rename"
            ],
        "split_generation_fences_metadata_without_invalidating_lifecycle_token":
            (
                not races["split"]["second_stale_metadata_command_accepted"]
                and races["split"]["active_edit_session_generation_token_still_valid_after_rename"]
            ),
    }

    if space["violations"]:
        raise SystemExit("split model invariant violations: " + "; ".join(space["violations"]))
    if not all(conclusions.values()):
        raise SystemExit("expected generation-policy discriminator did not reproduce")

    receipt = {
        "receipt_kind": "chaptera.cloud-project-lifecycle-modelcheck.v1",
        "real_service": False,
        "product_acceptance": False,
        "architecture_source": "Cloud project/workspace lifecycle model — 2026-09-24",
        "metadata_race_discriminator": races,
        "split_model_state_space": space,
        "conclusions": conclusions,
        "bounded_decision": {
            "lifecycle_generation": (
                "Fence lifecycle/admission transitions such as trash/restore/delete; "
                "do not advance for rename/folder metadata alone."
            ),
            "metadata_version": (
                "Use a separate metadata ETag/version for rename/move concurrency; "
                "stale metadata commands fail without invalidating active edit sessions."
            ),
            "semantic_revision": (
                "Rename/move/trash/restore do not create semantic document revisions."
            ),
            "fork": (
                "New ProjectId + DocumentId + genesis identity; grants not inherited by default."
            ),
        },
        "guardrail": (
            "This is a bounded executable architecture model, not deployed Cloud service evidence. "
            "Authorization, assets, jobs, retention and regional data-home effects still require integration tests."
        ),
    }
    OUT.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, indent=2))


if __name__ == "__main__":
    main()
