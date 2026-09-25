#!/usr/bin/env python3
"""Presentation-only desktop projection for canonical one-frame Story overset state.

The UI never measures text. It accepts only the source-neutral layout-state
envelope produced by AUTHORING-OVERSET-01 and maps it to visible frame/status
affordances.
"""

from __future__ import annotations

from dataclasses import dataclass
import re


_HASH_ID = re.compile(r"^sha256:[0-9a-f]{64}$")
_STATES = {"fits", "overset", "layout_unknown"}
_KEYS = {
    "story_hash",
    "scalar_count",
    "state",
    "reason_code",
    "environment_authoritative",
    "layout_environment_hash",
}


class EditorOversetUiError(ValueError):
    pass


@dataclass(frozen=True)
class OversetUiProjectionV1:
    state: str
    frame_marker_visible: bool
    frame_marker_kind: str | None
    status_severity: str
    status_title: str
    status_message: str
    full_story_retained: bool
    environment_authoritative: bool
    reason_code: str | None


def _require_hash_id(value: object, label: str) -> str:
    if not isinstance(value, str) or _HASH_ID.fullmatch(value) is None:
        raise EditorOversetUiError(f"{label} must be sha256:<64 lowercase hex>")
    return value


def validate_authoring_overset_state_v1(value: object) -> dict:
    if not isinstance(value, dict):
        raise EditorOversetUiError("overset layout state must be an object")
    if set(value) != _KEYS:
        missing = sorted(_KEYS - set(value))
        extra = sorted(set(value) - _KEYS)
        raise EditorOversetUiError(
            f"overset layout state fields mismatch: missing={missing} extra={extra}"
        )

    _require_hash_id(value["story_hash"], "story_hash")
    scalar_count = value["scalar_count"]
    if isinstance(scalar_count, bool) or not isinstance(scalar_count, int) or scalar_count < 0:
        raise EditorOversetUiError("scalar_count must be a non-negative integer")

    state = value["state"]
    if state not in _STATES:
        raise EditorOversetUiError("unsupported overset state")

    authoritative = value["environment_authoritative"]
    if not isinstance(authoritative, bool):
        raise EditorOversetUiError("environment_authoritative must be boolean")

    reason = value["reason_code"]
    layout_hash = value["layout_environment_hash"]

    if state in {"fits", "overset"}:
        if authoritative is not True:
            raise EditorOversetUiError(
                f"{state} requires an authoritative layout environment"
            )
        _require_hash_id(layout_hash, "layout_environment_hash")
        if reason is not None:
            raise EditorOversetUiError(f"{state} must not carry a reason_code")
    else:
        if authoritative is not False:
            raise EditorOversetUiError(
                "layout_unknown must remain explicitly non-authoritative"
            )
        if layout_hash is not None:
            raise EditorOversetUiError(
                "layout_unknown must not claim an authoritative layout hash"
            )
        if not isinstance(reason, str) or not reason:
            raise EditorOversetUiError("layout_unknown requires a reason_code")

    return value


def project_editor_overset_ui_v1(layout_state: object) -> OversetUiProjectionV1:
    state = validate_authoring_overset_state_v1(layout_state)
    kind = state["state"]

    if kind == "fits":
        return OversetUiProjectionV1(
            state="fits",
            frame_marker_visible=False,
            frame_marker_kind=None,
            status_severity="ok",
            status_title="Text fits",
            status_message="All canonical Story text is placed in this text frame.",
            full_story_retained=True,
            environment_authoritative=True,
            reason_code=None,
        )

    if kind == "overset":
        return OversetUiProjectionV1(
            state="overset",
            frame_marker_visible=True,
            frame_marker_kind="overset",
            status_severity="warning",
            status_title="Text overflow",
            status_message=(
                "Some Story text is not placed in this frame. "
                "The full canonical Story is retained."
            ),
            full_story_retained=True,
            environment_authoritative=True,
            reason_code=None,
        )

    return OversetUiProjectionV1(
        state="layout_unknown",
        frame_marker_visible=True,
        frame_marker_kind="layout_unknown",
        status_severity="unknown",
        status_title="Text fit unknown",
        status_message=(
            "Text fit cannot be determined from an authoritative layout environment. "
            "No fit or overflow result is being guessed."
        ),
        full_story_retained=True,
        environment_authoritative=False,
        reason_code=state["reason_code"],
    )
