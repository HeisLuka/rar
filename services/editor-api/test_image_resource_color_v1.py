#!/usr/bin/env python3
import hashlib
import struct
import unittest
import zlib

from image_resource_color_v1 import *


def chunk(kind: bytes, payload: bytes) -> bytes:
    crc = zlib.crc32(kind)
    crc = zlib.crc32(payload, crc) & 0xFFFFFFFF
    return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", crc)


def icc_profile(*, color=b"RGB ", pcs=b"XYZ ", profile_class=b"mntr", major=4) -> bytes:
    data = bytearray(128)
    data[0:4] = (128).to_bytes(4, "big")
    data[8] = major
    data[12:16] = profile_class
    data[16:20] = color
    data[20:24] = pcs
    data[36:40] = b"acsp"
    return bytes(data)


def png_with(*, color_chunks=()) -> bytes:
    ihdr = struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)
    raw = b"\x00\x00\x00\x00"
    return (
        PNG_SIGNATURE
        + chunk(b"IHDR", ihdr)
        + b"".join(color_chunks)
        + chunk(b"IDAT", zlib.compress(raw))
        + chunk(b"IEND", b"")
    )


def jpeg_segment(marker: int, payload: bytes) -> bytes:
    return b"\xff" + bytes([marker]) + (len(payload) + 2).to_bytes(2, "big") + payload


def exif_colorspace(value: int) -> bytes:
    tiff = bytearray()
    tiff += b"II"
    tiff += (42).to_bytes(2, "little")
    tiff += (8).to_bytes(4, "little")
    tiff += (1).to_bytes(2, "little")
    tiff += (0x8769).to_bytes(2, "little")
    tiff += (4).to_bytes(2, "little")
    tiff += (1).to_bytes(4, "little")
    tiff += (26).to_bytes(4, "little")
    tiff += (0).to_bytes(4, "little")
    tiff += (1).to_bytes(2, "little")
    tiff += (0xA001).to_bytes(2, "little")
    tiff += (3).to_bytes(2, "little")
    tiff += (1).to_bytes(4, "little")
    tiff += value.to_bytes(2, "little") + b"\x00\x00"
    tiff += (0).to_bytes(4, "little")
    return b"Exif\x00\x00" + bytes(tiff)


def jpeg_with(*, app_segments=()) -> bytes:
    sof = b"\x08\x00\x01\x00\x01\x03\x01\x11\x00\x02\x11\x00\x03\x11\x00"
    sos = b"\x03\x01\x00\x02\x11\x03\x11\x00\x3f\x00"
    return b"\xff\xd8" + b"".join(app_segments) + jpeg_segment(0xC0, sof) + jpeg_segment(0xDA, sos)


def derive(data: bytes, mime: str):
    sha = hashlib.sha256(data).hexdigest()
    return derive_image_color_disposition_v1(
        asset_bytes=data,
        mime_type=mime,
        expected_sha256=sha,
        resource_id="resource:1",
    )


class ImageResourceColorTests(unittest.TestCase):
    def test_png_srgb_is_explicit_exact_srgb(self):
        d = derive(png_with(color_chunks=[chunk(b"sRGB", b"\x00")]), "image/png")
        self.assertEqual(d.disposition_class, "explicit_srgb")
        self.assertTrue(d.exact_color_ready)

    def test_png_valid_iccp_has_deterministic_fingerprint(self):
        profile = icc_profile()
        payload = b"profile\x00\x00" + zlib.compress(profile)
        data = png_with(color_chunks=[chunk(b"iCCP", payload)])
        a, b = derive(data, "image/png"), derive(data, "image/png")
        self.assertEqual(a.disposition_class, "embedded_icc")
        self.assertEqual(a.icc.sha256, hashlib.sha256(profile).hexdigest())
        self.assertEqual(a, b)
        self.assertFalse(a.exact_color_ready)

    def test_png_iccp_bomb_and_corruption_fail_closed(self):
        bomb = b"profile\x00\x00" + zlib.compress(b"x" * (MAX_ICC_PROFILE_BYTES + 1))
        d = derive(png_with(color_chunks=[chunk(b"iCCP", bomb)]), "image/png")
        self.assertEqual(d.disposition_class, "invalid")
        self.assertIn("invalid_iccp", d.reason_codes)
        corrupt = derive(
            png_with(color_chunks=[chunk(b"iCCP", b"profile\x00\x00not-zlib")]),
            "image/png",
        )
        self.assertEqual(corrupt.disposition_class, "invalid")

    def test_png_conflicting_srgb_and_iccp_is_invalid(self):
        payload = b"profile\x00\x00" + zlib.compress(icc_profile())
        d = derive(png_with(color_chunks=[chunk(b"sRGB", b"\x00"), chunk(b"iCCP", payload)]), "image/png")
        self.assertEqual(d.disposition_class, "invalid")
        self.assertIn("conflicting_srgb_and_iccp", d.reason_codes)

    def test_png_gama_chrm_only_is_not_srgb(self):
        gamma = chunk(b"gAMA", (45455).to_bytes(4, "big"))
        chrm = chunk(b"cHRM", b"".join(x.to_bytes(4, "big") for x in range(1, 9)))
        d = derive(png_with(color_chunks=[gamma, chrm]), "image/png")
        self.assertEqual(d.disposition_class, "png_transfer_chromaticity")
        self.assertFalse(d.exact_color_ready)

    def test_jpeg_single_segment_icc(self):
        profile = icc_profile()
        app = jpeg_segment(0xE2, b"ICC_PROFILE\x00" + bytes([1, 1]) + profile)
        d = derive(jpeg_with(app_segments=[app]), "image/jpeg")
        self.assertEqual(d.disposition_class, "embedded_icc")
        self.assertEqual(d.icc.sha256, hashlib.sha256(profile).hexdigest())

    def test_jpeg_multi_segment_icc_assembles_in_sequence_order(self):
        profile = icc_profile()
        cut = 60
        app2 = jpeg_segment(0xE2, b"ICC_PROFILE\x00" + bytes([2, 2]) + profile[cut:])
        app1 = jpeg_segment(0xE2, b"ICC_PROFILE\x00" + bytes([1, 2]) + profile[:cut])
        d = derive(jpeg_with(app_segments=[app2, app1]), "image/jpeg")
        self.assertEqual(d.disposition_class, "embedded_icc")
        self.assertEqual(d.icc.byte_len, len(profile))

    def test_jpeg_missing_duplicate_and_out_of_range_icc_fail(self):
        profile = icc_profile()
        missing = jpeg_segment(0xE2, b"ICC_PROFILE\x00" + bytes([1, 2]) + profile[:60])
        d = derive(jpeg_with(app_segments=[missing]), "image/jpeg")
        self.assertEqual(d.disposition_class, "invalid")
        duplicate = [
            jpeg_segment(0xE2, b"ICC_PROFILE\x00" + bytes([1, 1]) + profile),
            jpeg_segment(0xE2, b"ICC_PROFILE\x00" + bytes([1, 1]) + profile),
        ]
        self.assertEqual(derive(jpeg_with(app_segments=duplicate), "image/jpeg").disposition_class, "invalid")
        bad = jpeg_segment(0xE2, b"ICC_PROFILE\x00" + bytes([2, 1]) + profile)
        self.assertEqual(derive(jpeg_with(app_segments=[bad]), "image/jpeg").disposition_class, "invalid")

    def test_jpeg_exif_srgb_and_uncalibrated_are_distinct(self):
        srgb = derive(jpeg_with(app_segments=[jpeg_segment(0xE1, exif_colorspace(1))]), "image/jpeg")
        unc = derive(jpeg_with(app_segments=[jpeg_segment(0xE1, exif_colorspace(0xFFFF))]), "image/jpeg")
        self.assertEqual(srgb.disposition_class, "explicit_srgb")
        self.assertTrue(srgb.exact_color_ready)
        self.assertEqual(unc.disposition_class, "exif_uncalibrated")
        self.assertFalse(unc.exact_color_ready)

    def test_untagged_stays_untagged(self):
        self.assertEqual(derive(png_with(), "image/png").disposition_class, "untagged")
        self.assertEqual(derive(jpeg_with(), "image/jpeg").disposition_class, "untagged")

    def test_resource_identity_is_unchanged_by_derived_metadata(self):
        data = png_with(color_chunks=[chunk(b"sRGB", b"\x00")])
        sha = hashlib.sha256(data).hexdigest()
        d = derive(data, "image/png")
        descriptor = project_image_resource_descriptor_v1(
            resource_id="resource:1",
            content_sha256=sha,
            mime_type="image/png",
            disposition=d,
        )
        self.assertEqual(descriptor["resource_id"], "resource:1")
        self.assertEqual(descriptor["content_hash"], sha)
        self.assertNotIn("raw_profile", str(descriptor))

    def test_material_cache_cannot_alias_incompatible_color_interpretations(self):
        srgb = derive(png_with(color_chunks=[chunk(b"sRGB", b"\x00")]), "image/png")
        untagged = derive(png_with(), "image/png")
        a = material_color_cache_key_v1(
            resource_id="resource:1", derivative_id="full", disposition=srgb, transform_policy_id="srgb-v1"
        )
        b = material_color_cache_key_v1(
            resource_id="resource:1", derivative_id="full", disposition=untagged, transform_policy_id="assume-srgb-v1"
        )
        self.assertNotEqual(a, b)


if __name__ == "__main__":
    unittest.main()
