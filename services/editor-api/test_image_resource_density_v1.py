#!/usr/bin/env python3
import hashlib
import struct
import unittest
import zlib

from image_resource_density_v1 import (
    ImageResourceDensityError,
    derive_image_resource_density_v1,
)


PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def png_chunk(kind, payload):
    return (
        struct.pack(">I", len(payload))
        + kind
        + payload
        + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
    )


def png(width=100, height=50, phys=None):
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    parts = [PNG_SIGNATURE, png_chunk(b"IHDR", ihdr)]
    if phys is not None:
        x, y, unit = phys
        parts.append(png_chunk(b"pHYs", struct.pack(">IIB", x, y, unit)))
    parts.append(png_chunk(b"IEND", b""))
    return b"".join(parts)


def jpeg_segment(marker, payload):
    return b"\xff" + bytes([marker]) + (len(payload) + 2).to_bytes(2, "big") + payload


def jfif_app0(unit, x, y):
    payload = (
        b"JFIF\x00"
        + b"\x01\x02"
        + bytes([unit])
        + x.to_bytes(2, "big")
        + y.to_bytes(2, "big")
        + b"\x00\x00"
    )
    return jpeg_segment(0xE0, payload)


def exif_density_app1(x_num, x_den, y_num, y_den, unit, *, endian="little"):
    byte_order = b"II" if endian == "little" else b"MM"
    entries = 3
    ifd0_offset = 8
    entries_start = ifd0_offset + 2
    rationals_offset = entries_start + entries * 12 + 4
    x_offset = rationals_offset
    y_offset = rationals_offset + 8

    def u16(value):
        return value.to_bytes(2, endian)

    def u32(value):
        return value.to_bytes(4, endian)

    def entry(tag, field_type, count, value4):
        return u16(tag) + u16(field_type) + u32(count) + value4

    unit_inline = u16(unit) + b"\x00\x00"
    tiff = (
        byte_order
        + u16(42)
        + u32(ifd0_offset)
        + u16(entries)
        + entry(0x011A, 5, 1, u32(x_offset))
        + entry(0x011B, 5, 1, u32(y_offset))
        + entry(0x0128, 3, 1, unit_inline)
        + u32(0)
        + u32(x_num)
        + u32(x_den)
        + u32(y_num)
        + u32(y_den)
    )
    return jpeg_segment(0xE1, b"Exif\x00\x00" + tiff)


def jpeg(width=100, height=50, segments=()):
    sof_payload = (
        b"\x08"
        + height.to_bytes(2, "big")
        + width.to_bytes(2, "big")
        + b"\x01"
        + b"\x01\x11\x00"
    )
    sof = jpeg_segment(0xC0, sof_payload)
    return b"\xff\xd8" + b"".join(segments) + sof + b"\xff\xd9"


def derive(data, mime):
    return derive_image_resource_density_v1(
        asset_bytes=data,
        mime_type=mime,
        expected_sha256=sha(data),
    )


class ImageResourceDensityV1Tests(unittest.TestCase):
    def test_absent_png_density_stays_absent_not_96dpi(self):
        result = derive(png(), "image/png")
        self.assertEqual("absent", result.disposition)
        self.assertIsNone(result.resolved_x_dpi)
        self.assertEqual((), result.evidence)

    def test_png_unitless_phys_preserves_ratio_without_physical_density(self):
        result = derive(png(phys=(3000, 2000, 0)), "image/png")
        self.assertEqual("unitless_only", result.disposition)
        evidence = result.evidence[0]
        self.assertEqual("unitless", evidence.unit)
        self.assertEqual((3000, 1), (evidence.x_raw.numerator, evidence.x_raw.denominator))
        self.assertIsNone(evidence.x_dpi)

    def test_png_metric_phys_derives_exact_rational_dpi_and_keeps_asymmetry(self):
        result = derive(png(phys=(3780, 4724, 1)), "image/png")
        self.assertEqual("single_physical_source", result.disposition)
        self.assertEqual(
            (24003, 250),
            (result.resolved_x_dpi.numerator, result.resolved_x_dpi.denominator),
        )
        self.assertNotEqual(result.resolved_x_dpi, result.resolved_y_dpi)

    def test_jfif_dpi_is_preserved_exactly(self):
        data = jpeg(segments=(jfif_app0(1, 300, 150),))
        result = derive(data, "image/jpeg")
        self.assertEqual("single_physical_source", result.disposition)
        self.assertEqual(
            (300, 1),
            (result.resolved_x_dpi.numerator, result.resolved_x_dpi.denominator),
        )
        self.assertEqual(
            (150, 1),
            (result.resolved_y_dpi.numerator, result.resolved_y_dpi.denominator),
        )

    def test_jfif_dpcm_converts_to_exact_dpi(self):
        data = jpeg(segments=(jfif_app0(2, 100, 50),))
        result = derive(data, "image/jpeg")
        self.assertEqual(
            (254, 1),
            (result.resolved_x_dpi.numerator, result.resolved_x_dpi.denominator),
        )
        self.assertEqual(
            (127, 1),
            (result.resolved_y_dpi.numerator, result.resolved_y_dpi.denominator),
        )

    def test_exif_inches_rational_is_preserved(self):
        data = jpeg(segments=(exif_density_app1(600, 2, 300, 2, 2),))
        result = derive(data, "image/jpeg")
        self.assertEqual("jpeg_exif", result.evidence[0].source)
        self.assertEqual(
            (300, 1),
            (result.resolved_x_dpi.numerator, result.resolved_x_dpi.denominator),
        )
        self.assertEqual(
            (150, 1),
            (result.resolved_y_dpi.numerator, result.resolved_y_dpi.denominator),
        )

    def test_exif_centimeters_convert_exactly(self):
        data = jpeg(segments=(exif_density_app1(100, 1, 50, 1, 3),))
        result = derive(data, "image/jpeg")
        self.assertEqual(
            (254, 1),
            (result.resolved_x_dpi.numerator, result.resolved_x_dpi.denominator),
        )
        self.assertEqual(
            (127, 1),
            (result.resolved_y_dpi.numerator, result.resolved_y_dpi.denominator),
        )

    def test_consistent_jfif_and_exif_are_resolved_without_discarding_sources(self):
        data = jpeg(
            segments=(
                jfif_app0(1, 300, 150),
                exif_density_app1(300, 1, 150, 1, 2),
            )
        )
        result = derive(data, "image/jpeg")
        self.assertEqual("consistent_physical_sources", result.disposition)
        self.assertEqual(2, len(result.evidence))
        self.assertEqual({"jpeg_jfif", "jpeg_exif"}, {e.source for e in result.evidence})

    def test_conflicting_jfif_and_exif_remain_conflict(self):
        data = jpeg(
            segments=(
                jfif_app0(1, 300, 300),
                exif_density_app1(72, 1, 72, 1, 2),
            )
        )
        result = derive(data, "image/jpeg")
        self.assertEqual("conflicting_physical_sources", result.disposition)
        self.assertIsNone(result.resolved_x_dpi)
        self.assertEqual(2, len(result.evidence))

    def test_incomplete_exif_density_fails_closed(self):
        valid = exif_density_app1(300, 1, 300, 1, 2)
        payload = bytearray(valid)
        needle = b"\x1b\x01\x05\x00"
        at = payload.find(needle)
        self.assertGreaterEqual(at, 0)
        payload[at : at + 2] = b"\x00\x02"
        data = jpeg(segments=(bytes(payload),))
        with self.assertRaisesRegex(ImageResourceDensityError, "incomplete"):
            derive(data, "image/jpeg")

    def test_malformed_phys_and_upstream_hash_admission_fail_closed(self):
        broken = png().replace(
            png_chunk(b"IEND", b""),
            png_chunk(b"pHYs", b"\x00" * 8) + png_chunk(b"IEND", b""),
        )
        with self.assertRaisesRegex(ImageResourceDensityError, "length 9"):
            derive(broken, "image/png")

        data = png(phys=(3780, 3780, 1))
        with self.assertRaisesRegex(ImageResourceDensityError, "SHA-256 mismatch"):
            derive_image_resource_density_v1(
                asset_bytes=data,
                mime_type="image/png",
                expected_sha256="0" * 64,
            )

    def test_receipt_is_machine_readable_and_contains_no_payload_bytes(self):
        data = jpeg(segments=(jfif_app0(1, 96, 96),))
        receipt = derive(data, "image/jpeg").to_receipt()
        self.assertEqual("chaptera.image-resource-density.v1", receipt["protocol_version"])
        self.assertEqual(sha(data), receipt["asset_sha256"])
        self.assertNotIn("asset_bytes", receipt)


if __name__ == "__main__":
    unittest.main()
