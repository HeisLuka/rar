#!/usr/bin/env python3
"""AUTHORING-PARAGRAPH-ALIGN-01 export/loss consumer.

This module consumes the canonical paragraph state already owned by
RevisionKernel. It does not invent IDML/ODG paragraph serialization. Until the
physical editable exporters expose paragraph-alignment materialization, every
known effective paragraph alignment is carried as explicit deterministic loss
instead of being silently dropped.

The source/base layer is immutable. Chaptera alignment_override, when present,
masks it for effective authoring/layout state; clearing the override reveals
the preserved base again.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal, Mapping, Sequence


SupportedAlignmentV1 = Literal["left", "center", "right"]
EditableTargetV1 = Literal["idml", "odg"]

_SUPPORTED = frozenset(("left", "center", "right"))
_TARGETS = frozenset(("idml", "odg"))


class ParagraphAlignmentExportError(ValueError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True, order=True)
class ParagraphAlignmentLossV1:
    paragraph_id: str
    target: EditableTargetV1
    effective_alignment: SupportedAlignmentV1
    source_layer: Literal["base", "chaptera_override"]
    code: Literal["paragraph_alignment_not_materialized"] = (
        "paragraph_alignment_not_materialized"
    )
    severity: Literal["loss"] = "loss"
    property_path: Literal["paragraph.alignment"] = "paragraph.alignment"


@dataclass(frozen=True)
class ParagraphAlignmentExportAssessmentV1:
    target: EditableTargetV1
    losses: tuple[ParagraphAlignmentLossV1, ...]

    @property
    def can_serialize_without_alignment_loss(self) -> bool:
        return not self.losses


def _fail(code: str, message: str) -> None:
    raise ParagraphAlignmentExportError(code, message)


def _canonical_paragraphs(project: Mapping[str, object]) -> Mapping[str, object]:
    paragraphs = project.get("paragraphs")
    if not isinstance(paragraphs, Mapping):
        _fail("paragraphs_missing", "canonical project must contain a paragraphs mapping")
    return paragraphs


def effective_paragraph_alignment_v1(paragraph: Mapping[str, object]) -> tuple[str, str]:
    """Return (effective alignment, source layer) for one canonical paragraph."""
    base = paragraph.get("base_alignment")
    override = paragraph.get("alignment_override")

    if not isinstance(base, str):
        _fail("base_alignment_invalid", "paragraph base_alignment must be a string")
    if override is not None and not isinstance(override, str):
        _fail(
            "alignment_override_invalid",
            "paragraph alignment_override must be null or a string",
        )

    if override is not None:
        if override not in _SUPPORTED:
            _fail(
                "alignment_override_unsupported",
                "Chaptera paragraph override must be left, center or right",
            )
        return override, "chaptera_override"

    if base in _SUPPORTED:
        return base, "base"

    # Unsupported imported values are deliberately preserved/read-only in
    # authoring. They are not normalized into the bounded Left/Center/Right
    # export claim.
    return base, "base"


def assess_paragraph_alignment_export_v1(
    project: Mapping[str, object],
    *,
    target: EditableTargetV1,
    paragraph_ids: Sequence[str] | None = None,
) -> ParagraphAlignmentExportAssessmentV1:
    if target not in _TARGETS:
        _fail("target_unsupported", "target must be idml or odg")

    paragraphs = _canonical_paragraphs(project)
    if paragraph_ids is None:
        selected_ids = sorted(paragraphs)
    else:
        if not paragraph_ids:
            _fail("paragraph_selection_empty", "paragraph_ids must be non-empty when supplied")
        if len(set(paragraph_ids)) != len(paragraph_ids):
            _fail("paragraph_selection_duplicate", "paragraph_ids must be unique")
        selected_ids = sorted(paragraph_ids)

    losses: list[ParagraphAlignmentLossV1] = []
    for paragraph_id in selected_ids:
        paragraph = paragraphs.get(paragraph_id)
        if not isinstance(paragraph, Mapping):
            _fail("paragraph_missing", f"paragraph {paragraph_id!r} is missing")

        stored_id = paragraph.get("paragraph_id")
        if stored_id != paragraph_id:
            _fail(
                "paragraph_identity_mismatch",
                f"paragraph mapping key {paragraph_id!r} does not match paragraph_id",
            )

        effective, layer = effective_paragraph_alignment_v1(paragraph)
        if effective not in _SUPPORTED:
            # Imported InterWord/Distribute/ambiguous state remains outside
            # the bounded V1 authored-alignment claim. A supported Chaptera
            # override would have masked it above and therefore *would* emit
            # a loss item.
            continue

        losses.append(
            ParagraphAlignmentLossV1(
                paragraph_id=paragraph_id,
                target=target,
                effective_alignment=effective,
                source_layer=layer,
            )
        )

    return ParagraphAlignmentExportAssessmentV1(
        target=target,
        losses=tuple(losses),
    )
