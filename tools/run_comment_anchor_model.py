#!/usr/bin/env python3
"""Executable anchor model for Chaptera comments/review.

Reference-model evidence only. It compares fragile revision/geometry anchors with
semantic NodeId / Story-range anchors transformed by canonical scalar edits.
"""

from __future__ import annotations

import dataclasses
import json
import random
from pathlib import Path
from typing import Literal, Optional

OUT = Path("target/cloud-comments/comment-anchor.json")


@dataclasses.dataclass(frozen=True)
class StoryRange:
    story_id: str
    start: int
    end: int
    start_affinity: Literal["left", "right"] = "right"
    end_affinity: Literal["left", "right"] = "left"

    def valid_for(self, length: int) -> bool:
        return 0 <= self.start <= self.end <= length


@dataclasses.dataclass
class Thread:
    thread_id: str
    document_id: str
    created_revision_id: str
    target_kind: str
    node_id: Optional[str] = None
    page_id: Optional[str] = None
    story_range: Optional[StoryRange] = None
    geometry: Optional[tuple[int, int, int, int]] = None
    resolved: bool = False
    orphaned: bool = False
    orphan_reason: Optional[str] = None
    revision_counter: int = 0


@dataclasses.dataclass(frozen=True)
class TextEdit:
    start: int
    end: int
    replacement_length: int


def transform_position(pos: int, edit: TextEdit, affinity: str) -> int:
    a, b, r = edit.start, edit.end, edit.replacement_length
    removed = b - a
    delta = r - removed

    if removed == 0:
        # Pure insertion. Position exactly at insertion point needs affinity.
        if pos < a:
            return pos
        if pos > a:
            return pos + r
        return a + r if affinity == "right" else a

    if pos < a:
        return pos
    if pos > b:
        return pos + delta
    if pos == b:
        return a + r
    if pos == a:
        return a + r if affinity == "right" else a
    # Position inside replaced interval.
    return a + r if affinity == "right" else a


def transform_story_range(anchor: StoryRange, edit: TextEdit, old_length: int) -> tuple[Optional[StoryRange], str]:
    assert anchor.valid_for(old_length)
    assert 0 <= edit.start <= edit.end <= old_length

    # If a replacement/deletion fully covers the old anchored content, do not
    # silently pretend the replacement is the same semantic target.
    if edit.end > edit.start and edit.start <= anchor.start and edit.end >= anchor.end:
        return None, "anchored_text_replaced_or_deleted"

    new_length = old_length - (edit.end - edit.start) + edit.replacement_length
    start = transform_position(anchor.start, edit, anchor.start_affinity)
    end = transform_position(anchor.end, edit, anchor.end_affinity)

    # Partial overlap may collapse one edge; keep the surviving/evolving range.
    if start > end:
        start, end = end, start
    transformed = StoryRange(
        story_id=anchor.story_id,
        start=start,
        end=end,
        start_affinity=anchor.start_affinity,
        end_affinity=anchor.end_affinity,
    )
    if not transformed.valid_for(new_length):
        raise AssertionError((anchor, edit, transformed, old_length, new_length))
    if transformed.start == transformed.end and anchor.start != anchor.end:
        return None, "anchored_text_collapsed"
    return transformed, "transformed"


def deterministic_cases() -> dict:
    original = "abcdefghijklmnopqrstuvwxyz"
    base = StoryRange("story:1", 10, 15)  # klmno

    cases = {}

    # Insert before: original target shifts.
    edit = TextEdit(3, 3, 4)
    transformed, reason = transform_story_range(base, edit, len(original))
    assert transformed == StoryRange("story:1", 14, 19)
    cases["insert_before"] = {"range": dataclasses.asdict(transformed), "reason": reason}

    # Insert exactly at start: right-affinity keeps anchor on original content.
    edit = TextEdit(10, 10, 2)
    transformed, reason = transform_story_range(base, edit, len(original))
    assert transformed.start == 12 and transformed.end == 17
    cases["insert_at_start"] = {"range": dataclasses.asdict(transformed), "reason": reason}

    # Insert exactly at end: end-left affinity excludes appended material.
    edit = TextEdit(15, 15, 2)
    transformed, reason = transform_story_range(base, edit, len(original))
    assert transformed.start == 10 and transformed.end == 15
    cases["insert_at_end"] = {"range": dataclasses.asdict(transformed), "reason": reason}

    # Insert inside: thread stays attached to the evolving target and expands.
    edit = TextEdit(12, 12, 3)
    transformed, reason = transform_story_range(base, edit, len(original))
    assert transformed.start == 10 and transformed.end == 18
    cases["insert_inside"] = {"range": dataclasses.asdict(transformed), "reason": reason}

    # Delete prefix of anchored text: surviving tail remains.
    edit = TextEdit(8, 12, 0)
    transformed, reason = transform_story_range(base, edit, len(original))
    assert transformed is not None
    assert transformed.start == 8 and transformed.end == 11
    cases["partial_delete"] = {"range": dataclasses.asdict(transformed), "reason": reason}

    # Delete whole target: orphan instead of silent retarget.
    edit = TextEdit(9, 16, 0)
    transformed, reason = transform_story_range(base, edit, len(original))
    assert transformed is None
    cases["full_delete"] = {"range": None, "reason": reason}

    # Replace whole target: also orphan; new text is not automatically same target.
    edit = TextEdit(10, 15, 5)
    transformed, reason = transform_story_range(base, edit, len(original))
    assert transformed is None
    cases["full_replace"] = {"range": None, "reason": reason}
    return cases


def randomized_range_invariants(seed: int = 1411, steps: int = 5000) -> dict:
    rng = random.Random(seed)
    length = 500
    anchor: Optional[StoryRange] = StoryRange("story:random", 200, 240)
    transformed_steps = 0
    orphaned_at = None

    for i in range(steps):
        a = rng.randrange(0, length + 1)
        b = rng.randrange(a, min(length, a + 25) + 1)
        repl = rng.randrange(0, 20)
        edit = TextEdit(a, b, repl)

        if anchor is not None:
            new_anchor, _ = transform_story_range(anchor, edit, length)
        else:
            new_anchor = None

        length = length - (b - a) + repl
        if new_anchor is not None:
            assert new_anchor.valid_for(length)
            anchor = new_anchor
            transformed_steps += 1
        else:
            anchor = None
            orphaned_at = i if orphaned_at is None else orphaned_at

    return {
        "seed": seed,
        "steps": steps,
        "transformed_steps_before_orphan_or_end": transformed_steps,
        "orphaned_at_step": orphaned_at,
        "final_story_length": length,
        "invalid_ranges": 0,
    }


def node_anchor_cases() -> dict:
    thread = Thread(
        thread_id="thread:node",
        document_id="doc:1",
        created_revision_id="rev:1",
        target_kind="node",
        node_id="node:42",
        page_id="page:1",
        geometry=(10, 20, 100, 80),
    )
    original_geometry = thread.geometry

    # Move/resize changes geometry but NodeId identity survives.
    thread.geometry = (500, 600, 150, 90)
    survives_move_resize = thread.node_id == "node:42" and not thread.orphaned

    # Page move also preserves NodeId.
    thread.page_id = "page:2"
    survives_page_move = thread.node_id == "node:42" and not thread.orphaned

    # Delete the semantic node -> preserve thread/history but orphan target.
    thread.orphaned = True
    thread.orphan_reason = "node_deleted"

    return {
        "original_geometry": original_geometry,
        "survives_move_resize_by_node_id": survives_move_resize,
        "survives_page_move_by_node_id": survives_page_move,
        "delete_result": {
            "thread_preserved": True,
            "orphaned": thread.orphaned,
            "reason": thread.orphan_reason,
        },
        "geometry_only_anchor_would_not_preserve_semantic_identity": True,
    }


def collaboration_store_cases() -> dict:
    semantic_revision = "rev:100"
    comment_store_version = 0

    # Create/resolve/reopen mutate collaborative data, not document content revision.
    comment_store_version += 1  # create
    after_create_revision = semantic_revision
    comment_store_version += 1  # resolve
    after_resolve_revision = semantic_revision
    comment_store_version += 1  # reopen
    after_reopen_revision = semantic_revision

    # Fork does not inherit threads by default.
    source_threads = ["thread:1", "thread:2"]
    fork_threads: list[str] = []

    # Commenter can comment but not edit document semantics.
    commenter_caps = {"document.read", "document.comment"}
    can_comment = "document.comment" in commenter_caps
    can_edit = "document.edit" in commenter_caps

    assert after_create_revision == after_resolve_revision == after_reopen_revision
    assert fork_threads == []
    assert can_comment and not can_edit

    return {
        "semantic_revision_unchanged_by_comment_create_resolve_reopen": True,
        "comment_store_version": comment_store_version,
        "fork_inherits_threads_by_default": False,
        "commenter_can_comment": can_comment,
        "commenter_can_edit_document": can_edit,
        "store_direction": "separate_collaboration_store_with_revision_context",
    }


def lifecycle_cases() -> dict:
    # Trash can hide/make comments read-only with document lifecycle; hard purge is terminal.
    return {
        "archive_or_trash": "retain_threads_but_follow_document_access/lifecycle_visibility",
        "restore": "same_threads_reappear_with_same_thread_ids_if_not_hard_purged",
        "hard_delete": "purge_comment_content_and_anchor_index_with_document_retention_contract",
        "no_resurrection_from_stale_projection": True,
    }


def main() -> None:
    OUT.parent.mkdir(parents=True, exist_ok=True)
    deterministic = deterministic_cases()
    random_invariants = randomized_range_invariants()
    nodes = node_anchor_cases()
    store = collaboration_store_cases()
    lifecycle = lifecycle_cases()

    receipt = {
        "receipt_kind": "chaptera.comment-anchor-reference-model.v1",
        "deployed_service": False,
        "canonical_private_core": False,
        "deterministic_story_range_cases": deterministic,
        "randomized_story_range_invariants": random_invariants,
        "node_anchor_cases": nodes,
        "collaboration_store_cases": store,
        "lifecycle_cases": lifecycle,
        "bounded_findings": {
            "revision_exact_geometry_is_too_fragile_as_primary_anchor": True,
            "node_anchor_should_use_canonical_node_id": True,
            "story_anchor_should_use_story_id_and_canonical_scalar_range": True,
            "story_range_needs_boundary_affinity": True,
            "whole_target_delete_or_replace_should_orphan_not_silently_retarget": True,
            "comment_mutations_should_not_create_semantic_document_revision": True,
            "comments_should_live_in_separate_collaboration_store": True,
            "fork_should_not_inherit_comments_by_default": True,
            "commenter_role_is_coherent_if_document_comment_is_separate_from_document_edit": True,
        },
        "anchor_direction": {
            "document": "DocumentId + creation/current revision context",
            "page": "PageId + revision context",
            "node": "NodeId + optional last-known page/geometry for orphan UX",
            "story_range": (
                "StoryId + canonical scalar [start,end) + start/right and end/left affinity "
                "+ creation revision + optional quoted-text hash for diagnostics"
            ),
            "geometry": "secondary visual fallback only, never primary semantic identity when Node/Story exists",
        },
        "guardrail": (
            "Reference-model evidence only. Real EditorSession text deltas, split/merge Story operations, "
            "mentions/notifications, screen-reader UX, moderation and deployed retention remain open."
        ),
    }
    OUT.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(receipt, indent=2))


if __name__ == "__main__":
    main()
