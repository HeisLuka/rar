#!/usr/bin/env python3
"""Deterministic intrinsic metadata for editor-owned PNG/JPEG assets V1.

Asset identity remains the existing SHA-256. This module derives rebuildable
metadata from exact admitted bytes only:
  width_px, height_px, orientation_class.

No pixel decode, DPI, physical size, resampling, crop/fit policy, color
normalization or transcoding is performed here.
"""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import struct
from typing import Literal


PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
MAX_IMAGE_AXIS_PX_V1 = 1_000_000  # Chaptera safety bound, not a format limit.
MAX_IMAGE_PIXELS_V1 = 1_000_000_000  # Chaptera safety bound, not a format limit.

OrientationClassV1 = Literal[
    "normal",
    "unsupported_orientation",
    "ambiguous_orientation",
]


class ImageAssetIntrinsicError(ValueError):
    pass


@dataclass(frozen=True)
class ImageAssetIntrinsicV1:
    protocol_version: Literal["chaptera.image-asset-intrinsic.v1"]
    asset_sha256: str
    mime_type: Literal["image/png", "image/jpeg"]
    width_px: int
    height_px: int
    orientation_class: OrientationClassV1
    exif_orientation: int | None

    @property
    def create_picture_frame_supported(self) -> bool:
        return self.orientation_class == "normal"


def _fail(message: str) -> None:
    raise ImageAssetIntrinsicError(message)


def _validate_dimensions(width: int, height: int) -> None:
    for value, label in ((width, "width_px"), (height, "height_px")):
        if not isinstance(value, int) or isinstance(value, bool) or value <= 0:
            _fail(f"{label} must be positive")
        if value > MAX_IMAGE_AXIS_PX_V1:
            _fail(f"{label} exceeds Chaptera V1 safety bound")
    if width * height > MAX_IMAGE_PIXELS_V1:
        _fail("image pixel count exceeds Chaptera V1 safety bound")


def _parse_png_dimensions(data: bytes) -> tuple[int, int]:
    if len(data) < 24:
        _fail("PNG header is truncated")
    if data[:8] != PNG_SIGNATURE:
        _fail("PNG signature mismatch")
    length = struct.unpack(">I", data[8:12])[0]
    chunk_type = data[12:16]
    if chunk_type != b"IHDR" or length != 13:
        _fail("PNG first chunk must be canonical IHDR length 13")
    if len(data) < 8 + 12 + length:
        _fail("PNG IHDR payload is truncated")
    width, height = struct.unpack(">II", data[16:24])
    _validate_dimensions(width, height)
    return width, height


def _read_u16(data: bytes, offset: int, endian: str, label: str) -> int:
    if offset < 0 or offset + 2 > len(data):
        _fail(f"{label} is truncated")
    return int.from_bytes(data[offset : offset + 2], endian)


def _read_u32(data: bytes, offset: int, endian: str, label: str) -> int:
    if offset < 0 or offset + 4 > len(data):
        _fail(f"{label} is truncated")
    return int.from_bytes(data[offset : offset + 4], endian)


def _parse_exif_orientation(payload: bytes) -> int | None:
    """Parse IFD0 Orientation from one Exif APP1 payload.

    Returns None when a valid Exif block contains no Orientation tag.
    Malformed/truncated Exif structure fails explicitly.
    """
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
    ifd0_offset = _read_u32(tiff, 4, endian, "EXIF IFD0 offset")
    if ifd0_offset < 8 or ifd0_offset + 2 > len(tiff):
        _fail("EXIF IFD0 offset is invalid")

    count = _read_u16(tiff, ifd0_offset, endian, "EXIF IFD0 entry count")
    entries_start = ifd0_offset + 2
    entries_end = entries_start + count * 12
    if entries_end > len(tiff):
        _fail("EXIF IFD0 entries are truncated")

    orientations = []
    for index in range(count):
        entry = entries_start + index * 12
        tag = _read_u16(tiff, entry, endian, "EXIF tag")
        if tag != 0x0112:
            continue
        field_type = _read_u16(tiff, entry + 2, endian, "EXIF Orientation type")
        value_count = _read_u32(tiff, entry + 4, endian, "EXIF Orientation count")
        if field_type != 3 or value_count != 1:
            _fail("EXIF Orientation tag must be SHORT count 1")
        value = _read_u16(tiff, entry + 8, endian, "EXIF Orientation value")
        if value < 1 or value > 8:
            _fail("EXIF Orientation value must be 1..8")
        orientations.append(value)

    if not orientations:
        return None
    if len(set(orientations)) != 1:
        return -1
    return orientations[0]


SOF_MARKERS = {
    0xC0, 0xC1, 0xC2, 0xC3,
    0xC5, 0xC6, 0xC7,
    0xC9, 0xCA, 0xCB,
    0xCD, 0xCE, 0xCF,
}


def _parse_jpeg(data: bytes) -> tuple[int, int, OrientationClassV1, int | None]:
    if len(data) < 4 or data[:2] != b"\xff\xd8":
        _fail("JPEG SOI signature mismatch")

    offset = 2
    width = height = None
    orientations: list[int | None] = []

    while offset < len(data):
        if data[offset] != 0xFF:
            _fail("JPEG marker prefix is malformed")
        while offset < len(data) and data[offset] == 0xFF:
            offset += 1
        if offset >= len(data):
            _fail("JPEG marker is truncated")
        marker = data[offset]
        offset += 1

        if marker == 0xD9:
            break
        if marker == 0xDA:
            if width is None or height is None:
                _fail("JPEG reached SOS before a supported SOF")
            break
        if marker in {0x01} or 0xD0 <= marker <= 0xD7:
            continue

        if offset + 2 > len(data):
            _fail("JPEG segment length is truncated")
        segment_length = int.from_bytes(data[offset : offset + 2], "big")
        if segment_length < 2:
            _fail("JPEG segment length is invalid")
        segment_end = offset + segment_length
        if segment_end > len(data):
            _fail("JPEG segment payload is truncated")
        payload = data[offset + 2 : segment_end]

        if marker == 0xE1 and payload.startswith(b"Exif\x00\x00"):
            orientations.append(_parse_exif_orientation(payload))

        if marker in SOF_MARKERS:
            if len(payload) < 6:
                _fail("JPEG SOF payload is truncated")
            frame_height = int.from_bytes(payload[1:3], "big")
            frame_width = int.from_bytes(payload[3:5], "big")
            _validate_dimensions(frame_width, frame_height)
            if width is not None and (width, height) != (frame_width, frame_height):
                _fail("JPEG contains conflicting SOF dimensions")
            width, height = frame_width, frame_height

        offset = segment_end

    if width is None or height is None:
        _fail("JPEG contains no supported SOF dimensions")

    concrete = [value for value in orientations if value is not None]
    if any(value == -1 for value in concrete):
        orientation_class: OrientationClassV1 = "ambiguous_orientation"
        exif_orientation = None
    else:
        unique = set(concrete)
        if len(unique) > 1:
            orientation_class = "ambiguous_orientation"
            exif_orientation = None
        elif not unique:
            orientation_class = "normal"
            exif_orientation = None
        else:
            value = next(iter(unique))
            exif_orientation = value
            orientation_class = (
                "normal" if value == 1 else "unsupported_orientation"
            )

    return width, height, orientation_class, exif_orientation


def derive_image_asset_intrinsic_v1(
    *,
    asset_bytes: bytes,
    mime_type: str,
    expected_sha256: str,
) -> ImageAssetIntrinsicV1:
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

    actual_sha256 = hashlib.sha256(asset_bytes).hexdigest()
    if actual_sha256 != expected_sha256:
        _fail("asset SHA-256 mismatch")

    if mime_type == "image/png":
        width, height = _parse_png_dimensions(asset_bytes)
        orientation_class: OrientationClassV1 = "normal"
        exif_orientation = None
    else:
        width, height, orientation_class, exif_orientation = _parse_jpeg(asset_bytes)

    return ImageAssetIntrinsicV1(
        protocol_version="chaptera.image-asset-intrinsic.v1",
        asset_sha256=expected_sha256,
        mime_type=mime_type,
        width_px=width,
        height_px=height,
        orientation_class=orientation_class,
        exif_orientation=exif_orientation,
    )
