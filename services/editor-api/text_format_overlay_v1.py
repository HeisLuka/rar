#!/usr/bin/env python3
"""Chaptera-native character-format overlay V1.

Source/base formatting is immutable input. Chaptera authoring owns only explicit
property overrides over canonical Unicode-scalar ranges.

V1 properties:
- font_size_emu
- bold
- italic
- text_color_rgb

Absence of an override means inherit/base. For booleans, explicit(False) is a
real override whenever the base is True. Redundant overrides equal to base are
removed by canonical normalization. Zero-length durable spans are forbidden.
"""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
from typing import Any, Literal


FormatPropertyV1 = Literal["font_size_emu", "bold", "italic", "text_color_rgb"]
_ALLOWED_PROPERTIES = {
    "font_size_emu",
    "bold",
    "italic",
    "text_color_rgb",
}


class TextFormatOverlayError(ValueError):
    pass


@dataclass(frozen=True)
class BaseCharacterFormatV1:
    font_resource_id: str
    font_size_emu: int
    bold: bool
    italic: bool
    text_color_rgb: str


@dataclass(frozen=True)
class BaseFormatRunV1:
    start_scalar: int
    end_scalar: int
    format: BaseCharacterFormatV1


@dataclass(frozen=True)
class TextFormatOverrideRunV1:
    start_scalar: int
    end_scalar: int
    property: FormatPropertyV1
    value: Any


@dataclass(frozen=True)
class TextFormatOverlayStateV1:
    protocol_version: Literal["chaptera.text-format-overlay.v1"]
    story_id: str
    base_revision_id: str
    story_scalar_len: int
    base_runs: tuple[BaseFormatRunV1, ...]
    overrides: tuple[TextFormatOverrideRunV1, ...]


@dataclass(frozen=True)
class EffectivePropertySegmentV1:
    start_scalar: int
    end_scalar: int
    property: FormatPropertyV1
    value: Any
    source: Literal["base", "chaptera_override"]


@dataclass(frozen=True)
class TextFormatOperationReceiptV1:
    protocol_version: Literal["chaptera.text-format-operation-receipt.v1"]
    command: dict
    before_state: TextFormatOverlayStateV1
    after_state: TextFormatOverlayStateV1
    before_effective: tuple[EffectivePropertySegmentV1, ...]
    after_effective: tuple[EffectivePropertySegmentV1, ...]
    requires_authoritative_relayout: bool
    export_policy: Literal["chaptera_override_or_explicit_loss"]


def _fail(message: str) -> None:
    raise TextFormatOverlayError(message)


def _validate_rgb(value: Any) -> str:
    if (
        not isinstance(value, str)
        or len(value) != 7
        or not value.startswith("#")
        or any(ch not in "0123456789abcdefABCDEF" for ch in value[1:])
    ):
        _fail("text_color_rgb must be #RRGGBB")
    return value.upper()


def _validate_property_value(prop: str, value: Any) -> Any:
    if prop not in _ALLOWED_PROPERTIES:
        _fail("unsupported character-format property")
    if prop == "font_size_emu":
        if (
            not isinstance(value, int)
            or isinstance(value, bool)
            or value <= 0
            or value > 9_007_199_254_740_991
        ):
            _fail("font_size_emu must be a positive JavaScript-safe integer")
        return value
    if prop in {"bold", "italic"}:
        if not isinstance(value, bool):
            _fail(f"{prop} must be boolean")
        return value
    return _validate_rgb(value)


def _validate_base_format(fmt: BaseCharacterFormatV1) -> BaseCharacterFormatV1:
    if not isinstance(fmt, BaseCharacterFormatV1):
        _fail("base format must be BaseCharacterFormatV1")
    if not isinstance(fmt.font_resource_id, str) or not fmt.font_resource_id:
        _fail("base formatting requires resolved font_resource_id")
    size = _validate_property_value("font_size_emu", fmt.font_size_emu)
    if not isinstance(fmt.bold, bool) or not isinstance(fmt.italic, bool):
        _fail("base bold/italic must be boolean")
    color = _validate_rgb(fmt.text_color_rgb)
    return BaseCharacterFormatV1(
        font_resource_id=fmt.font_resource_id,
        font_size_emu=size,
        bold=fmt.bold,
        italic=fmt.italic,
        text_color_rgb=color,
    )


def _validate_range(start: int, end: int, story_len: int) -> None:
    if (
        not isinstance(start, int)
        or isinstance(start, bool)
        or not isinstance(end, int)
        or isinstance(end, bool)
        or start < 0
        or end <= start
        or end > story_len
    ):
        _fail("format range must be non-empty and within Story scalar extent")


def _validate_base_runs(
    story_len: int,
    base_runs: tuple[BaseFormatRunV1, ...],
) -> tuple[BaseFormatRunV1, ...]:
    if not isinstance(base_runs, tuple):
        _fail("base_runs must be an ordered tuple")
    if story_len == 0:
        if base_runs:
            _fail("empty Story must not carry base character-format runs")
        return ()
    if not base_runs:
        _fail("non-empty Story requires explicit base formatting coverage")

    out = []
    cursor = 0
    for index, run in enumerate(base_runs):
        if not isinstance(run, BaseFormatRunV1):
            _fail(f"base_runs[{index}] must be BaseFormatRunV1")
        if run.start_scalar != cursor:
            _fail("base formatting must cover Story contiguously without gaps")
        if (
            not isinstance(run.end_scalar, int)
            or isinstance(run.end_scalar, bool)
            or run.end_scalar <= run.start_scalar
            or run.end_scalar > story_len
        ):
            _fail("base formatting run range is invalid")
        out.append(
            BaseFormatRunV1(
                run.start_scalar,
                run.end_scalar,
                _validate_base_format(run.format),
            )
        )
        cursor = run.end_scalar
    if cursor != story_len:
        _fail("base formatting must cover full Story scalar extent")
    return tuple(out)


def _base_value_at(
    base_runs: tuple[BaseFormatRunV1, ...],
    prop: FormatPropertyV1,
    scalar: int,
) -> Any:
    for run in base_runs:
        if run.start_scalar <= scalar < run.end_scalar:
            return getattr(run.format, prop)
    _fail("base formatting does not cover requested scalar")


def _override_value_at(
    overrides: tuple[TextFormatOverrideRunV1, ...],
    prop: FormatPropertyV1,
    scalar: int,
) -> Any | None:
    found = None
    for run in overrides:
        if run.property != prop:
            continue
        if run.start_scalar <= scalar < run.end_scalar:
            if found is not None:
                _fail("canonical override state contains overlap")
            found = run.value
    return found


def _normalize_overrides(
    *,
    story_len: int,
    base_runs: tuple[BaseFormatRunV1, ...],
    overrides: tuple[TextFormatOverrideRunV1, ...],
) -> tuple[TextFormatOverrideRunV1, ...]:
    if story_len == 0:
        if overrides:
            _fail("empty Story cannot carry durable formatting overrides")
        return ()

    validated = []
    for index, run in enumerate(overrides):
        if not isinstance(run, TextFormatOverrideRunV1):
            _fail(f"overrides[{index}] must be TextFormatOverrideRunV1")
        _validate_range(run.start_scalar, run.end_scalar, story_len)
        validated.append(
            TextFormatOverrideRunV1(
                run.start_scalar,
                run.end_scalar,
                run.property,
                _validate_property_value(run.property, run.value),
            )
        )

    normalized: list[TextFormatOverrideRunV1] = []
    for prop in sorted(_ALLOWED_PROPERTIES):
        prop_runs = tuple(run for run in validated if run.property == prop)
        boundaries = {0, story_len}
        for base in base_runs:
            boundaries.add(base.start_scalar)
            boundaries.add(base.end_scalar)
        for run in prop_runs:
            boundaries.add(run.start_scalar)
            boundaries.add(run.end_scalar)
        points = sorted(boundaries)

        for a, b in zip(points, points[1:]):
            if a == b:
                continue
            covering = [
                run for run in prop_runs
                if run.start_scalar <= a and b <= run.end_scalar
            ]
            if len(covering) > 1:
                _fail("format override runs for one property must not overlap")
            if not covering:
                continue
            explicit = covering[0].value
            base = _base_value_at(base_runs, prop, a)
            if explicit == base:
                continue
            candidate = TextFormatOverrideRunV1(a, b, prop, explicit)
            if (
                normalized
                and normalized[-1].property == prop
                and normalized[-1].value == explicit
                and normalized[-1].end_scalar == a
            ):
                previous = normalized[-1]
                normalized[-1] = TextFormatOverrideRunV1(
                    previous.start_scalar,
                    b,
                    prop,
                    explicit,
                )
            else:
                normalized.append(candidate)

    normalized.sort(
        key=lambda run: (
            run.property,
            run.start_scalar,
            run.end_scalar,
            json.dumps(run.value, sort_keys=True),
        )
    )
    return tuple(normalized)


def build_text_format_overlay_state_v1(
    *,
    story_id: str,
    base_revision_id: str,
    story_scalar_len: int,
    base_runs: tuple[BaseFormatRunV1, ...],
    overrides: tuple[TextFormatOverrideRunV1, ...] = (),
) -> TextFormatOverlayStateV1:
    if not isinstance(story_id, str) or not story_id:
        _fail("story_id is required")
    if not isinstance(base_revision_id, str) or not base_revision_id:
        _fail("base_revision_id is required")
    if (
        not isinstance(story_scalar_len, int)
        or isinstance(story_scalar_len, bool)
        or story_scalar_len < 0
    ):
        _fail("story_scalar_len must be a non-negative integer")
    canonical_base = _validate_base_runs(story_scalar_len, base_runs)
    canonical_overrides = _normalize_overrides(
        story_len=story_scalar_len,
        base_runs=canonical_base,
        overrides=overrides,
    )
    return TextFormatOverlayStateV1(
        protocol_version="chaptera.text-format-overlay.v1",
        story_id=story_id,
        base_revision_id=base_revision_id,
        story_scalar_len=story_scalar_len,
        base_runs=canonical_base,
        overrides=canonical_overrides,
    )


def _format_to_dict(fmt: BaseCharacterFormatV1) -> dict:
    return {
        "font_resource_id": fmt.font_resource_id,
        "font_size_emu": fmt.font_size_emu,
        "bold": fmt.bold,
        "italic": fmt.italic,
        "text_color_rgb": fmt.text_color_rgb,
    }


def state_dict_v1(state: TextFormatOverlayStateV1) -> dict:
    return {
        "protocol_version": state.protocol_version,
        "story_id": state.story_id,
        "base_revision_id": state.base_revision_id,
        "story_scalar_len": state.story_scalar_len,
        "base_runs": [
            {
                "start_scalar": run.start_scalar,
                "end_scalar": run.end_scalar,
                "format": _format_to_dict(run.format),
            }
            for run in state.base_runs
        ],
        "overrides": [
            {
                "start_scalar": run.start_scalar,
                "end_scalar": run.end_scalar,
                "property": run.property,
                "value": run.value,
            }
            for run in state.overrides
        ],
    }


def state_hash_v1(state: TextFormatOverlayStateV1) -> str:
    payload = json.dumps(
        state_dict_v1(state),
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def effective_property_segments_v1(
    *,
    state: TextFormatOverlayStateV1,
    prop: FormatPropertyV1,
    start_scalar: int,
    end_scalar: int,
) -> tuple[EffectivePropertySegmentV1, ...]:
    if prop not in _ALLOWED_PROPERTIES:
        _fail("unsupported character-format property")
    _validate_range(start_scalar, end_scalar, state.story_scalar_len)

    boundaries = {start_scalar, end_scalar}
    for run in state.base_runs:
        if start_scalar < run.end_scalar and run.start_scalar < end_scalar:
            boundaries.add(max(start_scalar, run.start_scalar))
            boundaries.add(min(end_scalar, run.end_scalar))
    for run in state.overrides:
        if run.property == prop and start_scalar < run.end_scalar and run.start_scalar < end_scalar:
            boundaries.add(max(start_scalar, run.start_scalar))
            boundaries.add(min(end_scalar, run.end_scalar))
    points = sorted(boundaries)

    segments: list[EffectivePropertySegmentV1] = []
    for a, b in zip(points, points[1:]):
        explicit = _override_value_at(state.overrides, prop, a)
        if explicit is None:
            value = _base_value_at(state.base_runs, prop, a)
            source = "base"
        else:
            value = explicit
            source = "chaptera_override"
        current = EffectivePropertySegmentV1(a, b, prop, value, source)
        if (
            segments
            and segments[-1].end_scalar == a
            and segments[-1].value == value
            and segments[-1].source == source
        ):
            previous = segments[-1]
            segments[-1] = EffectivePropertySegmentV1(
                previous.start_scalar,
                b,
                prop,
                value,
                source,
            )
        else:
            segments.append(current)
    return tuple(segments)


def _trim_property_runs(
    *,
    overrides: tuple[TextFormatOverrideRunV1, ...],
    prop: FormatPropertyV1,
    start: int,
    end: int,
) -> list[TextFormatOverrideRunV1]:
    out = []
    for run in overrides:
        if run.property != prop or run.end_scalar <= start or end <= run.start_scalar:
            out.append(run)
            continue
        if run.start_scalar < start:
            out.append(
                TextFormatOverrideRunV1(
                    run.start_scalar,
                    start,
                    run.property,
                    run.value,
                )
            )
        if end < run.end_scalar:
            out.append(
                TextFormatOverrideRunV1(
                    end,
                    run.end_scalar,
                    run.property,
                    run.value,
                )
            )
    return out


def _apply_format_operation_v1(
    *,
    state: TextFormatOverlayStateV1,
    kind: str,
    start_scalar: int,
    end_scalar: int,
    prop: FormatPropertyV1,
    value: Any | None,
    expected_state_hash: str,
) -> TextFormatOperationReceiptV1:
    if expected_state_hash != state_hash_v1(state):
        _fail("stale format-overlay state")
    _validate_range(start_scalar, end_scalar, state.story_scalar_len)
    if prop not in _ALLOWED_PROPERTIES:
        _fail("unsupported character-format property")

    before_effective = effective_property_segments_v1(
        state=state,
        prop=prop,
        start_scalar=start_scalar,
        end_scalar=end_scalar,
    )
    provisional = _trim_property_runs(
        overrides=state.overrides,
        prop=prop,
        start=start_scalar,
        end=end_scalar,
    )
    normalized_value = None
    if kind == "set":
        normalized_value = _validate_property_value(prop, value)
        provisional.append(
            TextFormatOverrideRunV1(
                start_scalar,
                end_scalar,
                prop,
                normalized_value,
            )
        )
    elif kind != "clear":
        _fail("unsupported format operation kind")

    after = build_text_format_overlay_state_v1(
        story_id=state.story_id,
        base_revision_id=state.base_revision_id,
        story_scalar_len=state.story_scalar_len,
        base_runs=state.base_runs,
        overrides=tuple(provisional),
    )
    after_effective = effective_property_segments_v1(
        state=after,
        prop=prop,
        start_scalar=start_scalar,
        end_scalar=end_scalar,
    )
    command = {
        "protocol_version": "chaptera.text-format-operation.v1",
        "kind": (
            "set_text_format_property"
            if kind == "set"
            else "clear_text_format_property_override"
        ),
        "story_id": state.story_id,
        "start_scalar": start_scalar,
        "end_scalar": end_scalar,
        "property": prop,
        "value": normalized_value,
        "expected_state_hash": expected_state_hash,
        "before_state_hash": state_hash_v1(state),
        "after_state_hash": state_hash_v1(after),
    }
    return TextFormatOperationReceiptV1(
        protocol_version="chaptera.text-format-operation-receipt.v1",
        command=command,
        before_state=state,
        after_state=after,
        before_effective=before_effective,
        after_effective=after_effective,
        requires_authoritative_relayout=True,
        export_policy="chaptera_override_or_explicit_loss",
    )


def set_text_format_property_v1(
    *,
    state: TextFormatOverlayStateV1,
    start_scalar: int,
    end_scalar: int,
    prop: FormatPropertyV1,
    value: Any,
    expected_state_hash: str,
) -> TextFormatOperationReceiptV1:
    return _apply_format_operation_v1(
        state=state,
        kind="set",
        start_scalar=start_scalar,
        end_scalar=end_scalar,
        prop=prop,
        value=value,
        expected_state_hash=expected_state_hash,
    )


def clear_text_format_property_override_v1(
    *,
    state: TextFormatOverlayStateV1,
    start_scalar: int,
    end_scalar: int,
    prop: FormatPropertyV1,
    expected_state_hash: str,
) -> TextFormatOperationReceiptV1:
    return _apply_format_operation_v1(
        state=state,
        kind="clear",
        start_scalar=start_scalar,
        end_scalar=end_scalar,
        prop=prop,
        value=None,
        expected_state_hash=expected_state_hash,
    )


def undo_text_format_operation_v1(
    receipt: TextFormatOperationReceiptV1,
) -> TextFormatOverlayStateV1:
    if not isinstance(receipt, TextFormatOperationReceiptV1):
        _fail("TextFormatOperationReceiptV1 is required")
    return receipt.before_state


def replay_text_format_operation_v1(
    receipt: TextFormatOperationReceiptV1,
) -> TextFormatOverlayStateV1:
    if not isinstance(receipt, TextFormatOperationReceiptV1):
        _fail("TextFormatOperationReceiptV1 is required")
    command = receipt.command
    kind = command.get("kind")
    if kind == "set_text_format_property":
        replay = set_text_format_property_v1(
            state=receipt.before_state,
            start_scalar=command["start_scalar"],
            end_scalar=command["end_scalar"],
            prop=command["property"],
            value=command["value"],
            expected_state_hash=command["expected_state_hash"],
        )
    elif kind == "clear_text_format_property_override":
        replay = clear_text_format_property_override_v1(
            state=receipt.before_state,
            start_scalar=command["start_scalar"],
            end_scalar=command["end_scalar"],
            prop=command["property"],
            expected_state_hash=command["expected_state_hash"],
        )
    else:
        _fail("unsupported serialized format operation")
    if replay.after_state != receipt.after_state:
        _fail("format operation replay did not reproduce canonical state")
    return replay.after_state
