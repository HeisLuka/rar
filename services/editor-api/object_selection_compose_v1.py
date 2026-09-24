#!/usr/bin/env python3
"""Pure deterministic ObjectSelectionTargetV1 set composition.

This planner owns only transient set algebra and primary-selection rules.
Target discovery, page/scope homogeneity, geometry, focus routing and document
mutation are deliberately caller-owned.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

from object_selection_target_v1 import (
    ObjectSelectionTargetError,
    ObjectSelectionTargetV1,
    selection_target_to_json_v1,
    target_variant_v1,
)


SelectionComposeModeV1 = Literal["replace", "add", "toggle", "subtract"]


class ObjectSelectionComposeError(ValueError):
    pass


@dataclass(frozen=True)
class ObjectSelectionComposeResultV1:
    selected_set: tuple[ObjectSelectionTargetV1, ...]
    primary: ObjectSelectionTargetV1 | None


def _fail(message: str) -> None:
    raise ObjectSelectionComposeError(message)


def _validate_target(value: ObjectSelectionTargetV1, label: str) -> None:
    try:
        target_variant_v1(value)
    except ObjectSelectionTargetError as exc:
        raise ObjectSelectionComposeError(f"{label}: {exc}") from exc


def _validate_unique_set(
    values: tuple[ObjectSelectionTargetV1, ...],
    label: str,
) -> None:
    if not isinstance(values, tuple):
        _fail(f"{label} must be a tuple")
    for index, value in enumerate(values):
        _validate_target(value, f"{label}[{index}]")
    if len(set(values)) != len(values):
        _fail(f"{label} must be duplicate-free")


def _normalized(
    values: set[ObjectSelectionTargetV1],
) -> tuple[ObjectSelectionTargetV1, ...]:
    return tuple(
        sorted(
            values,
            key=selection_target_to_json_v1,
        )
    )


def compose_object_selection_v1(
    *,
    base_set: tuple[ObjectSelectionTargetV1, ...],
    base_primary: ObjectSelectionTargetV1 | None,
    candidate_set: tuple[ObjectSelectionTargetV1, ...],
    mode: SelectionComposeModeV1,
) -> ObjectSelectionComposeResultV1:
    _validate_unique_set(base_set, "base_set")
    _validate_unique_set(candidate_set, "candidate_set")
    if mode not in {"replace", "add", "toggle", "subtract"}:
        _fail("unsupported selection composition mode")

    base = set(base_set)
    candidates = set(candidate_set)

    if base_primary is not None:
        _validate_target(base_primary, "base_primary")
        if base_primary not in base:
            _fail("base_primary must be contained in base_set")

    if mode == "replace":
        selected = candidates
    elif mode == "add":
        selected = base | candidates
    elif mode == "toggle":
        selected = base ^ candidates
    else:
        selected = base - candidates

    normalized = _normalized(selected)

    if mode != "replace" and base_primary is not None and base_primary in selected:
        primary = base_primary
    elif len(normalized) == 1:
        primary = normalized[0]
    else:
        primary = None

    return ObjectSelectionComposeResultV1(
        selected_set=normalized,
        primary=primary,
    )
