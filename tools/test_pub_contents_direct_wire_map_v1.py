#!/usr/bin/env python3
import unittest

from pub_contents_direct_wire_map_v1 import (
    DIRECT_METADATA_TO_WIRE,
    metadata_type_for_normalized_wire,
    normalized_wire_for_metadata_type,
)


class DirectWireMapTests(unittest.TestCase):
    def test_exact_byte_validated_map(self):
        expected = {
            0x00: 0x08,
            0x02: 0x10,
            0x03: 0x18,
            0x04: 0x20,
            0x05: 0x28,
            0x07: 0x38,
            0x09: 0x48,
            0x0B: 0x58,
            0x0D: 0x68,
            0x0E: 0x70,
            0x0F: 0x78,
            0x10: 0x80,
            0x11: 0x88,
            0x12: 0x90,
            0x13: 0x98,
            0x14: 0xA0,
            0x18: 0xC0,
        }
        self.assertEqual(DIRECT_METADATA_TO_WIRE, expected)

    def test_round_trip_admitted_pairs(self):
        for metadata_type, wire_type in DIRECT_METADATA_TO_WIRE.items():
            self.assertEqual(normalized_wire_for_metadata_type(metadata_type), wire_type)
            self.assertEqual(metadata_type_for_normalized_wire(wire_type), metadata_type)

    def test_xml_only_0x15_is_not_promoted(self):
        with self.assertRaises(ValueError):
            normalized_wire_for_metadata_type(0x15)

    def test_unknown_wire_fails_closed(self):
        with self.assertRaises(ValueError):
            metadata_type_for_normalized_wire(0xA8)

    def test_extended_field_wire_examples_still_normalize(self):
        self.assertEqual(normalized_wire_for_metadata_type(0x11), 0x88)
        self.assertEqual(normalized_wire_for_metadata_type(0x10), 0x80)
        self.assertEqual(normalized_wire_for_metadata_type(0x04), 0x20)


if __name__ == "__main__":
    unittest.main()
