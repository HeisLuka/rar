#!/usr/bin/env python3
import unittest

from pub_contents_packed_tag_v1 import decode_packed_tag, encode_packed_tag


class PackedContentsTagTests(unittest.TestCase):
    def test_exact_vectors(self):
        vectors = [
            (0x213, 0x08, bytes.fromhex("13 0A")),
            (0x224, 0x88, bytes.fromhex("24 8A")),
            (0x206, 0x80, bytes.fromhex("06 82")),
            (0x257, 0x88, bytes.fromhex("57 8A")),
        ]
        for field_id, wire_type, raw in vectors:
            with self.subTest(field_id=field_id, wire_type=wire_type):
                decoded = decode_packed_tag(raw)
                self.assertEqual(decoded.field_id, field_id)
                self.assertEqual(decoded.wire_type, wire_type)
                self.assertEqual(decoded.raw, raw)
                self.assertEqual(encode_packed_tag(field_id, wire_type), raw)

    def test_low_id_compatibility(self):
        raw = bytes.fromhex("05 20")
        decoded = decode_packed_tag(raw)
        self.assertEqual(decoded.field_id, 0x005)
        self.assertEqual(decoded.wire_type, 0x20)
        self.assertEqual(encode_packed_tag(decoded.field_id, decoded.wire_type), raw)

    def test_high_field_bits_do_not_change_wire_type(self):
        decoded = decode_packed_tag(bytes.fromhex("24 8A"))
        self.assertEqual(decoded.field_id, 0x224)
        self.assertEqual(decoded.wire_type, 0x88)
        self.assertNotEqual(decoded.wire_type, 0x8A)

    def test_round_trip_all_field_bits_for_representative_wires(self):
        for field_id in (0, 1, 0xFF, 0x100, 0x224, 0x7FF):
            for wire_type in (0x08, 0x20, 0x68, 0x80, 0x88, 0xA0):
                raw = encode_packed_tag(field_id, wire_type)
                decoded = decode_packed_tag(raw)
                self.assertEqual((decoded.field_id, decoded.wire_type), (field_id, wire_type))
                self.assertEqual(decoded.raw, raw)

    def test_reject_non_normalized_wire(self):
        with self.assertRaises(ValueError):
            encode_packed_tag(0x224, 0x8A)

    def test_reject_field_out_of_range(self):
        with self.assertRaises(ValueError):
            encode_packed_tag(0x800, 0x88)

    def test_reject_short_or_long_raw_tag(self):
        for raw in (b"", b"\x24", b"\x24\x8a\x00"):
            with self.assertRaises(ValueError):
                decode_packed_tag(raw)


if __name__ == "__main__":
    unittest.main()
