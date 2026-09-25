#!/usr/bin/env python3
from __future__ import annotations

from dataclasses import asdict, dataclass
import hashlib
import json
import math
from typing import Iterable

SCHEMA = "chaptera.render-color-contract.v1"
TARGET_SCHEMA = "chaptera.render-color-target.v1"
MATERIAL_KEY_SCHEMA = "chaptera.render-color-material-key.v1"


@dataclass(frozen=True)
class RenderColorContractV1:
    schema: str = SCHEMA
    working_color_space: str = "linear-srgb"
    source_transfer: str = "srgb"
    output_transfer: str = "srgb"
    ir_alpha_mode: str = "straight"
    runtime_storage_alpha_mode: str = "premultiplied"
    compositing_law: str = "source-over-linear-srgb"
    target_color_space: str = "srgb"
    dynamic_range: str = "sdr"
    target_alpha_mode: str = "premultiplied"
    pixel_format_class: str = "rgba8-unorm-srgb"
    tone_mapping_policy: str = "none"

    def receipt(self) -> dict:
        return asdict(self)


DEFAULT_CONTRACT = RenderColorContractV1()


def _clamp01(value: float) -> float:
    if not math.isfinite(value):
        raise ValueError("non-finite color component")
    return min(1.0, max(0.0, value))


def srgb_to_linear(value: float) -> float:
    value = _clamp01(value)
    if value <= 0.04045:
        return value / 12.92
    return ((value + 0.055) / 1.055) ** 2.4


def linear_to_srgb(value: float) -> float:
    value = _clamp01(value)
    if value <= 0.0031308:
        return value * 12.92
    return 1.055 * (value ** (1 / 2.4)) - 0.055


def premultiply_rgba(rgba: Iterable[float]) -> tuple[float, float, float, float]:
    r, g, b, a = map(_clamp01, rgba)
    return (r * a, g * a, b * a, a)


def unpremultiply_rgba(rgba: Iterable[float]) -> tuple[float, float, float, float]:
    r, g, b, a = map(_clamp01, rgba)
    if a == 0:
        return (0.0, 0.0, 0.0, 0.0)
    return (_clamp01(r / a), _clamp01(g / a), _clamp01(b / a), a)


def _source_over_premultiplied(src, dst):
    sr, sg, sb, sa = src
    dr, dg, db, da = dst
    one_minus_sa = 1.0 - sa
    return (
        sr + dr * one_minus_sa,
        sg + dg * one_minus_sa,
        sb + db * one_minus_sa,
        sa + da * one_minus_sa,
    )


def composite_over(src_rgba, dst_rgba, contract: RenderColorContractV1 = DEFAULT_CONTRACT):
    if contract.compositing_law != "source-over-linear-srgb":
        raise ValueError("unsupported compositing law")
    sr, sg, sb, sa = map(_clamp01, src_rgba)
    dr, dg, db, da = map(_clamp01, dst_rgba)
    src_linear = (srgb_to_linear(sr) * sa, srgb_to_linear(sg) * sa, srgb_to_linear(sb) * sa, sa)
    dst_linear = (srgb_to_linear(dr) * da, srgb_to_linear(dg) * da, srgb_to_linear(db) * da, da)
    rr, rg, rb, ra = _source_over_premultiplied(src_linear, dst_linear)
    if ra == 0:
        return (0.0, 0.0, 0.0, 0.0)
    return (
        _clamp01(linear_to_srgb(rr / ra)),
        _clamp01(linear_to_srgb(rg / ra)),
        _clamp01(linear_to_srgb(rb / ra)),
        _clamp01(ra),
    )


def composite_over_encoded_srgb(src_rgba, dst_rgba):
    """Negative oracle: encoded-sRGB source-over, intentionally not the V1 law."""
    src = premultiply_rgba(src_rgba)
    dst = premultiply_rgba(dst_rgba)
    return unpremultiply_rgba(_source_over_premultiplied(src, dst))


def color_disposition(profile_state: str, *, assume_srgb_allowed: bool = False) -> dict:
    if profile_state in {"literal_srgb", "explicit_srgb"}:
        return {
            "state": "exact",
            "source_color_space": "srgb",
            "conversion_target": "linear-srgb",
            "reason": None,
        }
    if profile_state in {"embedded_icc_unsupported", "unknown_profile", "untagged_unknown"}:
        if assume_srgb_allowed:
            return {
                "state": "assumed",
                "source_color_space": "srgb-assumed",
                "conversion_target": "linear-srgb",
                "reason": profile_state,
            }
        return {
            "state": "unknown",
            "source_color_space": None,
            "conversion_target": None,
            "reason": profile_state,
        }
    raise ValueError(f"unsupported profile state: {profile_state}")


def material_color_key(
    *,
    logical_material_id: str,
    source_disposition: dict,
    alpha_semantics: str = "straight",
    pixel_format: str = "rgba8",
) -> str:
    payload = {
        "schema": MATERIAL_KEY_SCHEMA,
        "logical_material_id": logical_material_id,
        "source_disposition": source_disposition,
        "alpha_semantics": alpha_semantics,
        "pixel_format": pixel_format,
        "contract": DEFAULT_CONTRACT.receipt(),
    }
    raw = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return "sha256:" + hashlib.sha256(raw).hexdigest()


def backend_requirement_set(contract: RenderColorContractV1 = DEFAULT_CONTRACT) -> dict:
    """Source-neutral requirements for RENDER-BACKEND-CAPABILITY consumers."""
    return {
        "schema": "chaptera.render-color-backend-requirements.v1",
        "mandatory_correctness": [
            f"color_space:{contract.target_color_space}",
            f"dynamic_range:{contract.dynamic_range}",
            f"alpha_mode:{contract.target_alpha_mode}",
            f"pixel_format:{contract.pixel_format_class}",
            f"compositing:{contract.compositing_law}",
        ],
        "optional_quality": ["wide_gamut:p3", "dynamic_range:hdr"],
        "performance_only": ["color_conversion_acceleration"],
    }


def rebuild_target(contract: RenderColorContractV1 = DEFAULT_CONTRACT) -> dict:
    """Device/backend rebuild reconstructs the same disposable target contract."""
    return {
        "schema": TARGET_SCHEMA,
        "color_space": contract.target_color_space,
        "dynamic_range": contract.dynamic_range,
        "alpha_mode": contract.target_alpha_mode,
        "pixel_format_class": contract.pixel_format_class,
        "tone_mapping_policy": contract.tone_mapping_policy,
        "contract_version": contract.schema,
    }
