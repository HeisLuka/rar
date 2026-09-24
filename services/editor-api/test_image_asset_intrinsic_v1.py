#!/usr/bin/env python3
import hashlib
import struct
import unittest
import zlib

from image_asset_intrinsic_v1 import (
    ImageAssetIntrinsicError,
    derive_image_asset_intrinsic_v1,
)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def png(width, height):
    signature = b"\x89PNG\r\n\x1a\n"
    payload = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    chunk = (
        struct.pack(">I", len(payload))
        + b"IHDR"
        + payload
        + struct.pack(">I", zlib.crc32(b"IHDR" + payload) & 0xFFFFFFFF)
    )
    return signature + chunk


def exif_app1(orientation):
    # Little-endian TIFF, IFD0 with one SHORT Orientation entry.
    tiff = (
        b"II"
        + (42).to_bytes(2, "little")
        + (8).to_bytes(4, "little")
        + (1).to_bytes(2, "little")
        + (0x0112).to_bytes(2, "little")
        + (3).to_bytes(2, "little")
        + (1).to_bytes(4, "little")
        + orientation.to_bytes(2, "little")
        + b"\x00\x00"
        + (0).to_bytes(4, "little")
    )
    payload = b"Exif\x00\x00" + tiff
    return b"\xff\xe1" + (len(payload) + 2).to_bytes(2, "big") + payload


def jpeg(width, height, orientation=None):
    sof_payload = (
        b"\x08"
        + height.to_bytes(2, "big")
        + width.to_bytes(2, "big")
        + b"\x01"
        + b"\x01\x11\x00"
    )
    sof = b"\xff\xc0" + (len(sof_payload) + 2).to_bytes(2, "big") + sof_payload
    parts = [b"\xff\xd8"]
    if orientation is not None:
        parts.append(exif_app1(orientation))
    parts.append(sof)
    parts.append(b"\xff\xd9")
    return b"".join(parts)


class ImageAssetIntrinsicV1Tests(unittest.TestCase):
    def test_png_dimensions_are_derived_from_exact_ihdr(self):
        data = png(640, 480)
        meta = derive_image_asset_intrinsic_v1(
            asset_bytes=data,
            mime_type="image/png",
            expected_sha256=sha(data),
        )
        self.assertEqual((640, 480), (meta.width_px, meta.height_px))
        self.assertEqual("normal", meta.orientation_class)
        self.assertTrue(meta.create_picture_frame_supported)
        self.assertEqual(sha(data), meta.asset_sha256)

    def test_jpeg_dimensions_and_no_exif_are_normal(self):
        data = jpeg(1200, 800)
        meta = derive_image_asset_intrinsic_v1(
            asset_bytes=data,
            mime_type="image/jpeg",
            expected_sha256=sha(data),
        )
        self.assertEqual((1200, 800), (meta.width_px, meta.height_px))
        self.assertEqual("normal", meta.orientation_class)
        self.assertIsNone(meta.exif_orientation)

    def test_jpeg_orientation_one_is_normal(self):
        data = jpeg(300, 200, orientation=1)
        meta = derive_image_asset_intrinsic_v1(
            asset_bytes=data,
            mime_type="image/jpeg",
            expected_sha256=sha(data),
        )
        self.assertEqual("normal", meta.orientation_class)
        self.assertEqual(1, meta.exif_orientation)
        self.assertTrue(meta.create_picture_frame_supported)

    def test_non_normal_exif_orientation_is_preserved_but_not_admitted(self):
        data = jpeg(300, 200, orientation=6)
        meta = derive_image_asset_intrinsic_v1(
            asset_bytes=data,
            mime_type="image/jpeg",
            expected_sha256=sha(data),
        )
        self.assertEqual((300, 200), (meta.width_px, meta.height_px))
        self.assertEqual("unsupported_orientation", meta.orientation_class)
        self.assertEqual(6, meta.exif_orientation)
        self.assertFalse(meta.create_picture_frame_supported)

    def test_multiple_conflicting_exif_blocks_are_ambiguous(self):
        base = jpeg(300, 200, orientation=None)
        data = base[:2] + exif_app1(1) + exif_app1(6) + base[2:]
        meta = derive_image_asset_intrinsic_v1(
            asset_bytes=data,
            mime_type="image/jpeg",
            expected_sha256=sha(data),
        )
        self.assertEqual("ambiguous_orientation", meta.orientation_class)
        self.assertFalse(meta.create_picture_frame_supported)

    def test_hash_identity_is_revalidated_not_replaced(self):
        data = png(10, 20)
        wrong = "0" * 64
        with self.assertRaisesRegex(ImageAssetIntrinsicError, "SHA-256 mismatch"):
            derive_image_asset_intrinsic_v1(
                asset_bytes=data,
                mime_type="image/png",
                expected_sha256=wrong,
            )

    def test_mime_signature_and_truncation_fail_closed(self):
        data = png(10, 20)
        with self.assertRaises(ImageAssetIntrinsicError):
            derive_image_asset_intrinsic_v1(
                asset_bytes=data,
                mime_type="image/jpeg",
                expected_sha256=sha(data),
            )

        truncated = b"\xff\xd8\xff\xc0\x00\x11\x08\x00"
        with self.assertRaises(ImageAssetIntrinsicError):
            derive_image_asset_intrinsic_v1(
                asset_bytes=truncated,
                mime_type="image/jpeg",
                expected_sha256=sha(truncated),
            )

    def test_dimensions_are_positive_and_chaptera_bounded(self):
        zero = png(0, 100)
        with self.assertRaisesRegex(ImageAssetIntrinsicError, "positive"):
            derive_image_asset_intrinsic_v1(
                asset_bytes=zero,
                mime_type="image/png",
                expected_sha256=sha(zero),
            )

        huge = png(1_000_001, 1)
        with self.assertRaisesRegex(ImageAssetIntrinsicError, "safety bound"):
            derive_image_asset_intrinsic_v1(
                asset_bytes=huge,
                mime_type="image/png",
                expected_sha256=sha(huge),
            )


if __name__ == "__main__":
    unittest.main()
