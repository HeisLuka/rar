#!/usr/bin/env python3
import hashlib
import unittest

from pub_contents_bounded_reader_v1 import (
    BoundedContentsReadError,
    SourceSpan,
    parse_0x2c_chunk,
    parse_bounded_fields,
    unique_field,
)

# Exact bounded OplChp bytes from the retained strict Microsoft Publisher 11
# oracle artifact (run 35524363847, artifact 10608394106).
# Public fixture: help.pub SHA-256
# 1e7f38b3ce1d0d956815992b15d361c405fc4bbdced5cdabb3c4581327cc183e
# Root OH/seqNum 298, root chunk offset 14794, bounded owner span 34..512.
PUBLISHER11_OPLCHP_BOUNDED = bytes.fromhex(
    "130a1b229c310000248aa201000001882c0000000180260000000f00540069006d006500730020004e00650077002000"
    "52006f006d0061006e00000002882c0000000180260000000f00540069006d006500730020004e006500770020005200"
    "6f006d0061006e00000003882c0000000180260000000f00540069006d006500730020004e0065007700200052006f00"
    "6d0061006e00000004882c0000000180260000000f00540069006d006500730020004e0065007700200052006f006d00"
    "61006e00000006882c0000000180260000000f00540069006d006500730020004e0065007700200052006f006d006100"
    "6e00000007882c0000000180260000000f00540069006d006500730020004e0065007700200052006f006d0061006e00"
    "000008881a00000001801400000006004d0061006e00670061006c0000000d881800000001801200000005004c006100"
    "740068006100000011882400000001801e0000000b0041006e006700730061006e00610020004e006500770000002c88"
    "2c0000000180260000000f00540069006d006500730020004e0065007700200052006f006d0061006e00000039225053"
    "0200448a2a0000000122010000080222ffffffff0322000000200422ffffffff0522ffffffff0682060000000000"
)


class BoundedContentsReaderTests(unittest.TestCase):
    def test_real_publisher11_extended_owner_span_traverses_to_end(self):
        self.assertEqual(478, len(PUBLISHER11_OPLCHP_BOUNDED))
        self.assertEqual(
            "853abc32e3f4b5f4414bf9a2469a032c418d509d86fa10dc42b7bbc996aea1f9",
            hashlib.sha256(PUBLISHER11_OPLCHP_BOUNDED).hexdigest(),
        )
        parsed = parse_bounded_fields(PUBLISHER11_OPLCHP_BOUNDED)
        self.assertTrue(parsed.is_fully_decoded())
        self.assertEqual(
            [0x213, 0x21B, 0x224, 0x239, 0x244],
            [field.field_id for field in parsed.fields],
        )
        self.assertEqual(
            [0x08, 0x20, 0x88, 0x20, 0x88],
            [field.wire_type for field in parsed.fields],
        )
        self.assertEqual(parsed.source.end, parsed.fields[-1].source.end)

    def test_semantic_lookup_uses_decoded_11_bit_field_id(self):
        parsed = parse_bounded_fields(PUBLISHER11_OPLCHP_BOUNDED)
        script_fonts = unique_field(parsed.fields, 0x224)
        self.assertEqual(bytes.fromhex("24 8A"), script_fonts.raw_tag)
        self.assertEqual(0x88, script_fonts.wire_type)
        self.assertEqual(SourceSpan(8, 2), script_fonts.tag_span)
        self.assertEqual(420, script_fonts.source.length)
        self.assertEqual(418, script_fonts.declared_length)
        with self.assertRaises(KeyError):
            unique_field(parsed.fields, 0x024)

    def test_low_id_chunk_behavior_remains_compatible(self):
        data = bytes([
            0x10, 0x00, 0x00, 0x00,
            0x27, 0x20, 0x16, 0x00, 0x00, 0x00,
            0x37, 0x68, 0x49, 0x01, 0x00, 0x00,
        ])
        chunk = parse_0x2c_chunk(data)
        self.assertTrue(chunk.is_fully_decoded())
        self.assertEqual([0x27, 0x37], [field.field_id for field in chunk.fields])
        self.assertEqual([0x20, 0x68], [field.wire_type for field in chunk.fields])

    def test_high_field_bits_no_longer_create_false_unsupported_tail(self):
        extended = bytes.fromhex("24 8A 08 00 00 00 DE AD BE EF")
        trailing = bytes.fromhex("27 20 16 00 00 00")
        body = extended + trailing
        data = (len(body) + 4).to_bytes(4, "little") + body
        chunk = parse_0x2c_chunk(data)
        self.assertTrue(chunk.is_fully_decoded())
        self.assertEqual([0x224, 0x27], [field.field_id for field in chunk.fields])

    def test_genuinely_unsupported_wire_preserves_exact_tail(self):
        known = bytes.fromhex("27 20 16 00 00 00")
        unsupported = bytes.fromhex("01 C8 AA BB")
        body = known + unsupported
        data = (len(body) + 4).to_bytes(4, "little") + body
        chunk = parse_0x2c_chunk(data)
        self.assertEqual(1, len(chunk.fields))
        self.assertEqual(SourceSpan(10, 4), chunk.unsupported_tail)

    def test_malformed_variable_length_fails_at_exact_field_start(self):
        data = bytes.fromhex("24 8A 03 00 00 00")
        with self.assertRaises(BoundedContentsReadError) as caught:
            parse_bounded_fields(data)
        self.assertEqual("invalid_declared_length", caught.exception.code)
        self.assertEqual(0, caught.exception.offset)

    def test_variable_block_cannot_cross_bounded_parent(self):
        data = bytes.fromhex("24 8A 0C 00 00 00 AA BB CC DD")
        with self.assertRaises(BoundedContentsReadError) as caught:
            parse_bounded_fields(data)
        self.assertEqual("variable_block_overrun", caught.exception.code)
        self.assertEqual(0, caught.exception.offset)


if __name__ == "__main__":
    unittest.main()
