#!/usr/bin/env python3
"""Bounded physical-density metadata over exact admitted PNG/JPEG bytes.

This module deliberately reuses EDITOR-ASSET-INTRINSIC-01 for byte identity,
MIME/signature, dimensions and bounded EXIF structure admission. It adds only
physical pixel-density evidence:

- PNG pHYs;
- JPEG JFIF APP0 density;
- JPEG Exif/TIFF XResolution, YResolution and ResolutionUnit.

No default DPI is invented. Conflicting physical sources remain explicit.
"""

from __future__ import annotations

from dataclasses import asdict, dataclass
from fractions import Fraction
from typing import Literal

from image_asset_intrinsic_v1 import (
    ImageAssetIntrinsicError,
    derive_image_asset_intrinsic_v1,
)


PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
MAX_METADATA_RECORDS_V1 = 4096


class ImageResourceDensityError(ValueError):
    pass


DensityDispositionV1 = Literal[
    "absent",
    "unitless_only",
    "single_physical_source",
    "consistent_physical_sources",
    "conflicting_physical_sources",
]


@dataclass(frozen=True)
class RationalV1:
    numerator: int
    denominator: int

    @classmethod
    def from_fraction(cls, value: Fraction) -> "RationalV1":
        return cls(value.numerator, value.denominator)


@dataclass(frozen=True)
class DensityEvidenceV1:
    source: Literal["png_phys", "jpeg_jfif", "jpeg_exif"]
    source_index: int
    unit: Literal["unitless", "meter", "inch", "centimeter"]
    x_raw: RationalV1
    y_raw: RationalV1
    x_dpi: RationalV1 | None
    y_dpi: RationalV1 | None


@dataclass(frozen=True)
class ImageResourceDensityV1:
    protocol_version: Literal["chaptera.image-resource-density.v1"]
    asset_sha256: str
    mime_type: Literal["image/png", "image/jpeg"]
    evidence: tuple[DensityEvidenceV1, ...]
    disposition: DensityDispositionV1
    resolved_x_dpi: RationalV1 | None
    resolved_y_dpi: RationalV1 | None

    def to_receipt(self) -> dict:
        return asdict(self)


def _fail(message: str) -> None:
    raise ImageResourceDensityError(message)


def _read_u16(data: bytes, offset: int, endian: str, label: str) -> int:
    if offset < 0 or offset + 2 > len(data):
        _fail(f"{label} is truncated")
    return int.from_bytes(data[offset : offset + 2], endian)


def _read_u32(data: bytes, offset: int, endian: str, label: str) -> int:
    if offset < 0 or offset + 4 > len(data):
        _fail(f"{label} is truncated")
    return int.from_bytes(data[offset : offset + 4], endian)


def _rational(num: int, den: int, label: str) -> Fraction:
    if den == 0:
        _fail(f"{label} denominator is zero")
    if num <= 0 or den < 0:
        _fail(f"{label} must be positive")
    return Fraction(num, den)


def _density_evidence(
    *,
    source: Literal["png_phys", "jpeg_jfif", "jpeg_exif"],
    source_index: int,
    unit: Literal["unitless", "meter", "inch", "centimeter"],
    x_raw: Fraction,
    y_raw: Fraction,
) -> DensityEvidenceV1:
    if x_raw <= 0 or y_raw <= 0:
        _fail(f"{source} density must be positive")

    if unit == "unitless":
        x_dpi = y_dpi = None
    elif unit == "meter":
        x_dpi = x_raw * Fraction(127, 5000)
        y_dpi = y_raw * Fraction(127, 5000)
    elif unit == "inch":
        x_dpi = x_raw
        y_dpi = y_raw
    elif unit == "centimeter":
        x_dpi = x_raw * Fraction(127, 50)
        y_dpi = y_raw * Fraction(127, 50)
    else:
        raise AssertionError(unit)

    return DensityEvidenceV1(
        source=source,
        source_index=source_index,
        unit=unit,
        x_raw=RationalV1.from_fraction(x_raw),
        y_raw=RationalV1.from_fraction(y_raw),
        x_dpi=RationalV1.from_fraction(x_dpi) if x_dpi is not None else None,
        y_dpi=RationalV1.from_fraction(y_dpi) if y_dpi is not None else None,
    )


def _iter_png_chunks(data: bytes):
    if not data.startswith(PNG_SIGNATURE):
        _fail("PNG signature mismatch after intrinsic admission")

    offset = len(PNG_SIGNATURE)
    count = 0
    while offset < len(data):
        count += 1
        if count > MAX_METADATA_RECORDS_V1:
            _fail("PNG chunk count exceeds Chaptera V1 metadata bound")
        if offset + 12 > len(data):
            _fail("PNG chunk framing is truncated")

        length = int.from_bytes(data[offset : offset + 4], "big")
        chunk_type = data[offset + 4 : offset + 8]
        payload_start = offset + 8
        payload_end = payload_start + length
        framed_end = payload_end + 4
        if payload_end < payload_start or framed_end > len(data):
            _fail("PNG chunk payload is truncated")

        yield chunk_type, data[payload_start:payload_end]
        offset = framed_end
        if chunk_type == b"IEND":
            break


def _parse_png_density(data: bytes) -> list[DensityEvidenceV1]:
    result = []
    for index, (chunk_type, payload) in enumerate(_iter_png_chunks(data)):
        if chunk_type != b"pHYs":
            continue
        if len(payload) != 9:
            _fail("PNG pHYs chunk must have length 9")
        x_ppu = int.from_bytes(payload[0:4], "big")
        y_ppu = int.from_bytes(payload[4:8], "big")
        unit = payload[8]
        if unit == 0:
            unit_name = "unitless"
        elif unit == 1:
            unit_name = "meter"
        else:
            _fail("PNG pHYs unit specifier must be 0 or 1")
        result.append(
            _density_evidence(
                source="png_phys",
                source_index=index,
                unit=unit_name,
                x_raw=Fraction(x_ppu, 1),
                y_raw=Fraction(y_ppu, 1),
            )
        )
    return result


def _iter_jpeg_segments(data: bytes):
    if len(data) < 4 or data[:2] != b"\xff\xd8":
        _fail("JPEG SOI signature mismatch after intrinsic admission")

    offset = 2
    count = 0
    while offset < len(data):
        if data[offset] != 0xFF:
            _fail("JPEG marker prefix is malformed")
        while offset < len(data) and data[offset] == 0xFF:
            offset += 1
        if offset >= len(data):
            _fail("JPEG marker is truncated")

        marker = data[offset]
        offset += 1
        if marker in {0xD9, 0xDA}:
            break
        if marker == 0x01 or 0xD0 <= marker <= 0xD7:
            continue

        count += 1
        if count > MAX_METADATA_RECORDS_V1:
            _fail("JPEG segment count exceeds Chaptera V1 metadata bound")
        if offset + 2 > len(data):
            _fail("JPEG segment length is truncated")
        segment_length = int.from_bytes(data[offset : offset + 2], "big")
        if segment_length < 2:
            _fail("JPEG segment length is invalid")
        segment_end = offset + segment_length
        if segment_end > len(data):
            _fail("JPEG segment payload is truncated")

        yield count - 1, marker, data[offset + 2 : segment_end]
        offset = segment_end


def _parse_jfif_density(index: int, payload: bytes) -> DensityEvidenceV1 | None:
    if not payload.startswith(b"JFIF\x00"):
        return None
    if len(payload) < 14:
        _fail("JPEG JFIF APP0 payload is truncated")

    unit = payload[7]
    x_density = int.from_bytes(payload[8:10], "big")
    y_density = int.from_bytes(payload[10:12], "big")
    if unit == 0:
        unit_name = "unitless"
    elif unit == 1:
        unit_name = "inch"
    elif unit == 2:
        unit_name = "centimeter"
    else:
        _fail("JPEG JFIF density unit must be 0, 1 or 2")

    return _density_evidence(
        source="jpeg_jfif",
        source_index=index,
        unit=unit_name,
        x_raw=Fraction(x_density, 1),
        y_raw=Fraction(y_density, 1),
    )


def _parse_tiff_rational(tiff: bytes, entry: int, endian: str, label: str) -> Fraction:
    field_type = _read_u16(tiff, entry + 2, endian, f"{label} type")
    count = _read_u32(tiff, entry + 4, endian, f"{label} count")
    if field_type != 5 or count != 1:
        _fail(f"{label} must be RATIONAL count 1")
    value_offset = _read_u32(tiff, entry + 8, endian, f"{label} offset")
    num = _read_u32(tiff, value_offset, endian, f"{label} numerator")
    den = _read_u32(tiff, value_offset + 4, endian, f"{label} denominator")
    return _rational(num, den, label)


def _parse_exif_density(index: int, payload: bytes) -> DensityEvidenceV1 | None:
    if not payload.startswith(b"Exif\x00\x00"):
        return None

    tiff = payload[6:]
    if len(tiff) < 8:
        _fail("EXIF TIFF header is truncated")
    byte_order = tiff[:2]
    if byte_order == b"II":
        endian = "little"
    elif byte_order == b"MM":
        endian = "big"
    else:
        _fail("EXIF byte order is invalid")
    if _read_u16(tiff, 2, endian, "EXIF magic") != 42:
        _fail("EXIF TIFF magic is invalid")

    ifd0 = _read_u32(tiff, 4, endian, "EXIF IFD0 offset")
    if ifd0 < 8 or ifd0 + 2 > len(tiff):
        _fail("EXIF IFD0 offset is invalid")
    count = _read_u16(tiff, ifd0, endian, "EXIF IFD0 entry count")
    start = ifd0 + 2
    end = start + count * 12
    if end > len(tiff):
        _fail("EXIF IFD0 entries are truncated")

    entries: dict[int, int] = {}
    for entry_index in range(count):
        entry = start + entry_index * 12
        tag = _read_u16(tiff, entry, endian, "EXIF tag")
        if tag not in {0x011A, 0x011B, 0x0128}:
            continue
        if tag in entries:
            _fail(f"EXIF density tag 0x{tag:04X} is duplicated")
        entries[tag] = entry

    if not entries:
        return None
    if set(entries) != {0x011A, 0x011B, 0x0128}:
        _fail("EXIF density tags are incomplete")

    x_resolution = _parse_tiff_rational(tiff, entries[0x011A], endian, "EXIF XResolution")
    y_resolution = _parse_tiff_rational(tiff, entries[0x011B], endian, "EXIF YResolution")

    unit_entry = entries[0x0128]
    unit_type = _read_u16(tiff, unit_entry + 2, endian, "EXIF ResolutionUnit type")
    unit_count = _read_u32(tiff, unit_entry + 4, endian, "EXIF ResolutionUnit count")
    if unit_type != 3 or unit_count != 1:
        _fail("EXIF ResolutionUnit must be SHORT count 1")
    unit = _read_u16(tiff, unit_entry + 8, endian, "EXIF ResolutionUnit value")
    if unit == 1:
        unit_name = "unitless"
    elif unit == 2:
        unit_name = "inch"
    elif unit == 3:
        unit_name = "centimeter"
    else:
        _fail("EXIF ResolutionUnit must be 1, 2 or 3")

    return _density_evidence(
        source="jpeg_exif",
        source_index=index,
        unit=unit_name,
        x_raw=x_resolution,
        y_raw=y_resolution,
    )


def _parse_jpeg_density(data: bytes) -> list[DensityEvidenceV1]:
    result = []
    for index, marker, payload in _iter_jpeg_segments(data):
        if marker == 0xE0:
            evidence = _parse_jfif_density(index, payload)
        elif marker == 0xE1:
            evidence = _parse_exif_density(index, payload)
        else:
            evidence = None
        if evidence is not None:
            result.append(evidence)
    return result


def _fraction(value: RationalV1) -> Fraction:
    return Fraction(value.numerator, value.denominator)


def _resolve(
    evidence: list[DensityEvidenceV1],
) -> tuple[DensityDispositionV1, RationalV1 | None, RationalV1 | None]:
    physical = [
        item for item in evidence if item.x_dpi is not None and item.y_dpi is not None
    ]
    if not evidence:
        return "absent", None, None
    if not physical:
        return "unitless_only", None, None

    pairs = {
        (_fraction(item.x_dpi), _fraction(item.y_dpi))
        for item in physical
        if item.x_dpi is not None and item.y_dpi is not None
    }
    if len(pairs) > 1:
        return "conflicting_physical_sources", None, None

    x_dpi, y_dpi = next(iter(pairs))
    disposition: DensityDispositionV1 = (
        "single_physical_source"
        if len(physical) == 1
        else "consistent_physical_sources"
    )
    return (
        disposition,
        RationalV1.from_fraction(x_dpi),
        RationalV1.from_fraction(y_dpi),
    )


def derive_image_resource_density_v1(
    *,
    asset_bytes: bytes,
    mime_type: str,
    expected_sha256: str,
) -> ImageResourceDensityV1:
    try:
        intrinsic = derive_image_asset_intrinsic_v1(
            asset_bytes=asset_bytes,
            mime_type=mime_type,
            expected_sha256=expected_sha256,
        )
    except ImageAssetIntrinsicError as error:
        raise ImageResourceDensityError(str(error)) from error

    if intrinsic.mime_type == "image/png":
        evidence = _parse_png_density(asset_bytes)
    else:
        evidence = _parse_jpeg_density(asset_bytes)

    disposition, x_dpi, y_dpi = _resolve(evidence)
    return ImageResourceDensityV1(
        protocol_version="chaptera.image-resource-density.v1",
        asset_sha256=intrinsic.asset_sha256,
        mime_type=intrinsic.mime_type,
        evidence=tuple(evidence),
        disposition=disposition,
        resolved_x_dpi=x_dpi,
        resolved_y_dpi=y_dpi,
    )
