#!/usr/bin/env python3
"""Story-focus Select All over the canonical ordinary editable domain V1."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from story_edit_domain_v1 import (
    StoryEditDomainError,
    StoryEditDomainV1,
    select_all_range_v1,
)
from text_selection_state_v1 import (
    TextSelectionStateError,
    TextSelectionStateV1,
    build_text_selection_state_v1,
    validate_selection_state_v1,
)
from text_typing_format_state_v1 import TextTypingFormatStateV1


SelectAllInputOwnerV1 = Literal[
    "story_text",
    "composition",
    "modal",
    "inspector",
    "canvas",
    "page_navigator",
]


class TextSelectAllError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class TextSelectAllResultV1:
    protocol_version: Literal["chaptera.text-select-all-result.v1"]
    status: Literal["selected", "suppressed"]
    selection: TextSelectionStateV1
    typing_state: TextTypingFormatStateV1 | None
    input_owner: SelectAllInputOwnerV1
    selected_start_scalar: int
    selected_end_scalar: int
    preferred_inline_x_cleared: bool
    typing_state_cleared: bool
    document_mutation_count: Literal[0]
    undo_history_entry_count: Literal[0]
    reason: str | None


_ALLOWED_OWNERS = {
    "story_text",
    "composition",
    "modal",
    "inspector",
    "canvas",
    "page_navigator",
}


def _fail(code: str, message: str) -> None:
    raise TextSelectAllError(code, message)


def _validate_typing_state_context(
    typing_state: TextTypingFormatStateV1 | None,
    selection: TextSelectionStateV1,
) -> None:
    if typing_state is None:
        return
    if not isinstance(typing_state, TextTypingFormatStateV1):
        _fail("invalid_typing_state", "typing_state must be TextTypingFormatStateV1 or null")
    if (
        typing_state.story_id != selection.story_id
        or typing_state.revision_id != selection.revision_id
        or typing_state.caret_scalar != selection.focus_scalar
    ):
        _fail(
            "typing_context_changed",
            "typing state belongs to a different Story/revision/caret context",
        )


def select_all_story_text_v1(
    *,
    current_selection: TextSelectionStateV1,
    domain: StoryEditDomainV1,
    input_owner: SelectAllInputOwnerV1,
    typing_state: TextTypingFormatStateV1 | None = None,
) -> TextSelectAllResultV1:
    if input_owner not in _ALLOWED_OWNERS:
        _fail("invalid_input_owner", "unsupported Select All focus owner")
    try:
        validate_selection_state_v1(
            state=current_selection,
            domain=domain,
            expected_revision_id=current_selection.revision_id,
        )
    except TextSelectionStateError as exc:
        _fail(exc.code, str(exc))
    _validate_typing_state_context(typing_state, current_selection)

    if input_owner != "story_text":
        start, end = current_selection.normalized_range
        return TextSelectAllResultV1(
            protocol_version="chaptera.text-select-all-result.v1",
            status="suppressed",
            selection=current_selection,
            typing_state=typing_state,
            input_owner=input_owner,
            selected_start_scalar=start,
            selected_end_scalar=end,
            preferred_inline_x_cleared=False,
            typing_state_cleared=False,
            document_mutation_count=0,
            undo_history_entry_count=0,
            reason=f"{input_owner} owns Select All input",
        )

    try:
        start, end = select_all_range_v1(domain)
    except StoryEditDomainError as exc:
        _fail(exc.code, str(exc))

    try:
        selected = build_text_selection_state_v1(
            domain=domain,
            revision_id=current_selection.revision_id,
            anchor_scalar=start,
            focus_scalar=end,
            preferred_inline_x_emu=None,
        )
    except TextSelectionStateError as exc:
        _fail(exc.code, str(exc))

    return TextSelectAllResultV1(
        protocol_version="chaptera.text-select-all-result.v1",
        status="selected",
        selection=selected,
        typing_state=None,
        input_owner=input_owner,
        selected_start_scalar=start,
        selected_end_scalar=end,
        preferred_inline_x_cleared=True,
        typing_state_cleared=typing_state is not None,
        document_mutation_count=0,
        undo_history_entry_count=0,
        reason=None,
    )
