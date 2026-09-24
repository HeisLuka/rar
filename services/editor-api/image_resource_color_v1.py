#!/usr/bin/env python3
"""Bounded deterministic PNG/JPEG color metadata disposition V1.

This module classifies what exact admitted image bytes claim about source color.
It does not decode pixels or execute ICC transforms. Asset/resource identity
remains the existing exact byte identity; color metadata is derived/rebuildable.
"""

from __future__ import annotations

from dataclasses import asdict, dataclass
import hashlib
import json
import zlib
from typing import Literal

PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
SCHEMA = "chaptera.image-color-disposition.v1"
PARSER_VERSION = "image-color-metadata-parser.v1"

MAX_PNG_CHUNKS = 4096
MAX_JPEG_SEGMENTS = 4096
MAX_JPEG_ICC_SEGMENTS = 64
MAX_ICC_PROFILE_BYTES = 1_048_576
MAX_EXIF_IFD_ENTRIES = 256

DispositionClass = Literal[
    "explicit_srgb",
    "embedded_icc",
    "png_transfer_chromaticity",
    "exif_uncalibrated",
    "untagged",
    "invalid",
    "unsupported",
    "unknown",
]


class ImageColorDispositionError(ValueError):
    pass


@dataclass(frozen=True)
class IccProfileFactsV1:
    sha256: str
    byte_len: int
    declared_byte_len: int
    major_version: int
    profile_class: str
    data_color_space: str
    pcs: str


@dataclass(frozen=True)
class ImageColorDispositionV1:
    schema: Literal["chaptera.image-color-disposition.v1"]
    parser_version: str
    resource_id: str
    content_sha256: str
    mime_type: str
    disposition_class: DispositionClass
    exact_color_ready: bool
    metadata_sources: tuple[str, ...]
    reason_codes: tuple[str, ...]
    icc: IccProfileFactsV1 | None
    png_gamma_scaled_100000: int | None
    png_chromaticities_scaled_100000: tuple[int, ...] | None
    exif_color_space: str | None

    def public_descriptor(self) -> dict:
        return asdict(self)


def _fail(message: str) -> None:
    raise ImageColorDispositionError(message)


def _validate_identity(
    *,
    asset_bytes: bytes,
    expected_sha256: str,
    resource_id: str,
    mime_type: str,
) -> None:
    if not isinstance(asset_bytes, bytes):
        _fail("asset_bytes must be exact bytes")
    if mime_type not in {"image/png", "image/jpeg"}:
        _fail("V1 supports only exact image/png or image/jpeg")
    if (
        not isinstance(expected_sha256, str)
        or len(expected_sha256) != 64
        or any(ch not in "0123456789abcdef" for ch in expected_sha256)
    ):
        _fail("expected_sha256 must be lowercase hex SHA-256")
    if hashlib.sha256(asset_bytes).hexdigest() != expected_sha256:
        _fail("asset SHA-256 mismatch")
    if not isinstance(resource_id, str) or not resource_id:
        _fail("resource_id is required")


def _ascii_signature(value: bytes) -> str:
    return value.decode("ascii", errors="replace")


def _parse_icc_profile(payload: bytes) -> IccProfileFactsV1:
    if not isinstance(payload, bytes) or len(payload) < 128:
        _fail("ICC profile is shorter than the 128-byte header")
    if len(payload) > MAX_ICC_PROFILE_BYTES:
        _fail("ICC profile exceeds Chaptera V1 byte bound")
    declared = int.from_bytes(payload[0:4], "big")
    if declared != len(payload):
        _fail("ICC declared size does not equal embedded profile length")
    if payload[36:40] != b"acsp":
        _fail("ICC profile signature is invalid")
    major = payload[8]
    return IccProfileFactsV1(
        sha256=hashlib.sha256(payload).hexdigest(),
        byte_len=len(payload),
        declared_byte_len=declared,
        major_version=major,
        profile_class=_ascii_signature(payload[12:16]),
        data_color_space=_ascii_signature(payload[16:20]),
        pcs=_ascii_signature(payload[20:24]),
    )


def _bounded_zlib_decompress(payload: bytes) -> bytes:
    try:
        decoder = zlib.decompressobj()
        out = decoder.decompress(payload, MAX_ICC_PROFILE_BYTES + 1)
        if len(out) > MAX_ICC_PROFILE_BYTES:
            _fail("PNG iCCP decompressed profile exceeds Chaptera V1 bound")
        if not decoder.eof:
            _fail("PNG iCCP compressed profile is truncated or exceeds bound")
        if decoder.unused_data:
            _fail("PNG iCCP contains trailing compressed data")
        return out
    except zlib.error as exc:
        _fail(f"PNG iCCP decompression failed: {exc}")


def _parse_png_iccp(payload: bytes) -> IccProfileFactsV1:
    nul = payload.find(b"\x00")
    if nul <= 0 or nul > 79:
        _fail("PNG iCCP profile name is invalid")
    if nul + 2 > len(payload):
        _fail("PNG iCCP compression fields are truncated")
    if payload[nul + 1] != 0:
        _fail("PNG iCCP compression method must be 0")
    compressed = payload[nul + 2 :]
    if not compressed:
        _fail("PNG iCCP compressed profile is empty")
    return _parse_icc_profile(_bounded_zlib_decompress(compressed))


def _png_disposition(
    *,
    asset_bytes: bytes,
    resource_id: str,
    content_sha256: str,
) -> ImageColorDispositionV1:
    if len(asset_bytes) < 8 or asset_bytes[:8] != PNG_SIGNATURE:
        _fail("PNG signature mismatch")

    offset = 8
    chunk_count = 0
    seen_idat = False
    seen_iend = False
    srgb_intent = None
    icc = None
    gamma = None
    chrm = None
    sources: list[str] = []
    reasons: list[str] = []
    duplicates: set[str] = set()

    while offset < len(asset_bytes):
        chunk_count += 1
        if chunk_count > MAX_PNG_CHUNKS:
            _fail("PNG chunk count exceeds Chaptera V1 bound")
        if offset + 12 > len(asset_bytes):
            _fail("PNG chunk header is truncated")
        length = int.from_bytes(asset_bytes[offset : offset + 4], "big")
        chunk_type = asset_bytes[offset + 4 : offset + 8]
        end = offset + 12 + length
        if end > len(asset_bytes):
            _fail("PNG chunk payload is truncated")
        payload = asset_bytes[offset + 8 : offset + 8 + length]
        expected_crc = int.from_bytes(asset_bytes[offset + 8 + length : end], "big")
        actual_crc = zlib.crc32(chunk_type)
        actual_crc = zlib.crc32(payload, actual_crc) & 0xFFFFFFFF
        if actual_crc != expected_crc:
            _fail("PNG chunk CRC mismatch")

        if chunk_type == b"IDAT":
            seen_idat = True
        elif chunk_type == b"IEND":
            if length != 0:
                _fail("PNG IEND must be empty")
            seen_iend = True
            offset = end
            break
        elif chunk_type in {b"sRGB", b"iCCP", b"gAMA", b"cHRM"}:
            if seen_idat:
                reasons.append("color_chunk_after_idat")
            name = chunk_type.decode("ascii")
            if name in sources:
                duplicates.add(name)
            sources.append(name)
            if chunk_type == b"sRGB":
                if len(payload) != 1 or payload[0] > 3:
                    reasons.append("invalid_srgb_chunk")
                else:
                    srgb_intent = payload[0]
            elif chunk_type == b"iCCP":
                try:
                    icc = _parse_png_iccp(payload)
                except ImageColorDispositionError:
                    reasons.append("invalid_iccp")
            elif chunk_type == b"gAMA":
                if len(payload) != 4:
                    reasons.append("invalid_gama_chunk")
                else:
                    gamma = int.from_bytes(payload, "big")
                    if gamma == 0:
                        reasons.append("invalid_gama_zero")
            elif chunk_type == b"cHRM":
                if len(payload) != 32:
                    reasons.append("invalid_chrm_chunk")
                else:
                    chrm = tuple(
                        int.from_bytes(payload[i : i + 4], "big")
                        for i in range(0, 32, 4)
                    )

        offset = end

    if not seen_iend:
        _fail("PNG contains no IEND")
    if offset != len(asset_bytes):
        _fail("PNG contains trailing bytes after IEND")

    if duplicates:
        reasons.append("duplicate_color_declaration")
    if srgb_intent is not None and icc is not None:
        reasons.append("conflicting_srgb_and_iccp")
    reasons = sorted(set(reasons))

    if reasons:
        cls: DispositionClass = "invalid"
        exact = False
    elif icc is not None:
        cls = "embedded_icc"
        exact = False
    elif srgb_intent is not None:
        cls = "explicit_srgb"
        exact = True
    elif gamma is not None or chrm is not None:
        cls = "png_transfer_chromaticity"
        exact = False
    else:
        cls = "untagged"
        exact = False

    return ImageColorDispositionV1(
        schema=SCHEMA,
        parser_version=PARSER_VERSION,
        resource_id=resource_id,
        content_sha256=content_sha256,
        mime_type="image/png",
        disposition_class=cls,
        exact_color_ready=exact,
        metadata_sources=tuple(sorted(set(sources))),
        reason_codes=tuple(reasons),
        icc=icc if cls == "embedded_icc" else None,
        png_gamma_scaled_100000=gamma,
        png_chromaticities_scaled_100000=chrm,
        exif_color_space=None,
    )


def _read_u16(data: bytes, offset: int, endian: str, label: str) -> int:
    if offset < 0 or offset + 2 > len(data):
        _fail(f"{label} is truncated")
    return int.from_bytes(data[offset : offset + 2], endian)


def _read_u32(data: bytes, offset: int, endian: str, label: str) -> int:
    if offset < 0 or offset + 4 > len(data):
        _fail(f"{label} is truncated")
    return int.from_bytes(data[offset : offset + 4], endian)


def _parse_ifd_entries(
    tiff: bytes,
    offset: int,
    endian: str,
    *,
    label: str,
) -> list[tuple[int, int, int, int]]:
    count = _read_u16(tiff, offset, endian, f"{label} entry count")
    if count > MAX_EXIF_IFD_ENTRIES:
        _fail(f"{label} entry count exceeds Chaptera V1 bound")
    start = offset + 2
    end = start + count * 12
    if end > len(tiff):
        _fail(f"{label} entries are truncated")
    entries = []
    for index in range(count):
        p = start + index * 12
        entries.append(
            (
                _read_u16(tiff, p, endian, f"{label} tag"),
                _read_u16(tiff, p + 2, endian, f"{label} type"),
                _read_u32(tiff, p + 4, endian, f"{label} count"),
                _read_u32(tiff, p + 8, endian, f"{label} value"),
            )
        )
    return entries


def _parse_exif_color_space(payload: bytes) -> str | None:
    if not payload.startswith(b"Exif\x00\x00"):
        return None
    tiff = payload[6:]
    if len(tiff) < 8:
        _fail("EXIF TIFF header is truncated")
    if tiff[:2] == b"II":
        endian = "little"
    elif tiff[:2] == b"MM":
        endian = "big"
    else:
        _fail("EXIF byte order is invalid")
    if _read_u16(tiff, 2, endian, "EXIF magic") != 42:
        _fail("EXIF TIFF magic is invalid")
    ifd0 = _read_u32(tiff, 4, endian, "EXIF IFD0 offset")
    entries = _parse_ifd_entries(tiff, ifd0, endian, label="EXIF IFD0")
    exif_ptrs = [value for tag, typ, count, value in entries if tag == 0x8769 and typ == 4 and count == 1]
    if not exif_ptrs:
        return None
    if len(set(exif_ptrs)) != 1:
        _fail("EXIF contains conflicting ExifIFD pointers")
    exif_entries = _parse_ifd_entries(tiff, exif_ptrs[0], endian, label="EXIF sub-IFD")
    values = []
    for tag, typ, count, value in exif_entries:
        if tag != 0xA001:
            continue
        if typ != 3 or count != 1:
            _fail("EXIF ColorSpace must be SHORT count 1")
        if endian == "little":
            values.append(value & 0xFFFF)
        else:
            values.append((value >> 16) & 0xFFFF)
    if not values:
        return None
    if len(set(values)) != 1:
        _fail("EXIF contains conflicting ColorSpace values")
    value = values[0]
    if value == 1:
        return "srgb"
    if value == 0xFFFF:
        return "uncalibrated"
    return f"unknown:{value}"


def _jpeg_disposition(
    *,
    asset_bytes: bytes,
    resource_id: str,
    content_sha256: str,
) -> ImageColorDispositionV1:
    if len(asset_bytes) < 4 or asset_bytes[:2] != b"\xff\xd8":
        _fail("JPEG SOI signature mismatch")

    offset = 2
    segment_count = 0
    icc_parts: dict[int, bytes] = {}
    icc_total = None
    icc_errors: list[str] = []
    exif_values: list[str] = []
    exif_errors: list[str] = []
    sources: list[str] = []

    while offset < len(asset_bytes):
        segment_count += 1
        if segment_count > MAX_JPEG_SEGMENTS:
            _fail("JPEG marker count exceeds Chaptera V1 bound")
        if asset_bytes[offset] != 0xFF:
            _fail("JPEG marker prefix is malformed")
        while offset < len(asset_bytes) and asset_bytes[offset] == 0xFF:
            offset += 1
        if offset >= len(asset_bytes):
            _fail("JPEG marker is truncated")
        marker = asset_bytes[offset]
        offset += 1
        if marker == 0xD9:
            break
        if marker == 0xDA:
            break
        if marker == 0x01 or 0xD0 <= marker <= 0xD7:
            continue
        if offset + 2 > len(asset_bytes):
            _fail("JPEG segment length is truncated")
        segment_length = int.from_bytes(asset_bytes[offset : offset + 2], "big")
        if segment_length < 2:
            _fail("JPEG segment length is invalid")
        end = offset + segment_length
        if end > len(asset_bytes):
            _fail("JPEG segment payload is truncated")
        payload = asset_bytes[offset + 2 : end]

        if marker == 0xE2 and payload.startswith(b"ICC_PROFILE\x00"):
            sources.append("JPEG_ICC_APP2")
            if len(payload) < 14:
                icc_errors.append("icc_segment_header_truncated")
            else:
                seq = payload[12]
                total = payload[13]
                if total < 1 or total > MAX_JPEG_ICC_SEGMENTS:
                    icc_errors.append("icc_segment_count_out_of_range")
                elif seq < 1 or seq > total:
                    icc_errors.append("icc_segment_sequence_out_of_range")
                else:
                    if icc_total is None:
                        icc_total = total
                    elif icc_total != total:
                        icc_errors.append("icc_segment_count_conflict")
                    if seq in icc_parts:
                        icc_errors.append("icc_segment_duplicate")
                    else:
                        icc_parts[seq] = payload[14:]

        if marker == 0xE1 and payload.startswith(b"Exif\x00\x00"):
            sources.append("EXIF_APP1")
            try:
                value = _parse_exif_color_space(payload)
                if value is not None:
                    exif_values.append(value)
            except ImageColorDispositionError:
                exif_errors.append("invalid_exif_colorspace")

        offset = end

    icc = None
    if icc_parts or icc_total is not None:
        if icc_total is None:
            icc_errors.append("icc_segment_count_missing")
        elif set(icc_parts) != set(range(1, icc_total + 1)):
            icc_errors.append("icc_segment_missing")
        elif not icc_errors:
            assembled = b"".join(icc_parts[index] for index in range(1, icc_total + 1))
            try:
                icc = _parse_icc_profile(assembled)
            except ImageColorDispositionError:
                icc_errors.append("invalid_icc_profile")

    exif_unique = sorted(set(exif_values))
    if len(exif_unique) > 1:
        exif_errors.append("conflicting_exif_colorspace")
    exif = exif_unique[0] if len(exif_unique) == 1 else None

    reasons = sorted(set(icc_errors + exif_errors))
    if reasons:
        cls: DispositionClass = "invalid"
        exact = False
    elif icc is not None:
        cls = "embedded_icc"
        exact = False
    elif exif == "srgb":
        cls = "explicit_srgb"
        exact = True
    elif exif == "uncalibrated":
        cls = "exif_uncalibrated"
        exact = False
    elif exif is not None and exif.startswith("unknown:"):
        cls = "unknown"
        exact = False
    else:
        cls = "untagged"
        exact = False

    return ImageColorDispositionV1(
        schema=SCHEMA,
        parser_version=PARSER_VERSION,
        resource_id=resource_id,
        content_sha256=content_sha256,
        mime_type="image/jpeg",
        disposition_class=cls,
        exact_color_ready=exact,
        metadata_sources=tuple(sorted(set(sources))),
        reason_codes=tuple(reasons),
        icc=icc if cls == "embedded_icc" else None,
        png_gamma_scaled_100000=None,
        png_chromaticities_scaled_100000=None,
        exif_color_space=exif,
    )


def derive_image_color_disposition_v1(
    *,
    asset_bytes: bytes,
    mime_type: str,
    expected_sha256: str,
    resource_id: str,
) -> ImageColorDispositionV1:
    _validate_identity(
        asset_bytes=asset_bytes,
        expected_sha256=expected_sha256,
        resource_id=resource_id,
        mime_type=mime_type,
    )
    if mime_type == "image/png":
        return _png_disposition(
            asset_bytes=asset_bytes,
            resource_id=resource_id,
            content_sha256=expected_sha256,
        )
    return _jpeg_disposition(
        asset_bytes=asset_bytes,
        resource_id=resource_id,
        content_sha256=expected_sha256,
    )


def project_image_resource_descriptor_v1(
    *,
    resource_id: str,
    content_sha256: str,
    mime_type: str,
    disposition: ImageColorDispositionV1,
) -> dict:
    if disposition.resource_id != resource_id or disposition.content_sha256 != content_sha256:
        _fail("derived color metadata identity does not match resource descriptor")
    if disposition.mime_type != mime_type:
        _fail("derived color metadata MIME does not match resource descriptor")
    return {
        "resource_id": resource_id,
        "content_hash": content_sha256,
        "mime": mime_type,
        "image_color_disposition": disposition.public_descriptor(),
    }


def material_color_cache_key_v1(
    *,
    resource_id: str,
    derivative_id: str,
    disposition: ImageColorDispositionV1,
    transform_policy_id: str,
) -> str:
    payload = {
        "schema": "chaptera.image-color-material-key.v1",
        "resource_id": resource_id,
        "derivative_id": derivative_id,
        "disposition": disposition.public_descriptor(),
        "transform_policy_id": transform_policy_id,
    }
    raw = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return "sha256:" + hashlib.sha256(raw).hexdigest()
