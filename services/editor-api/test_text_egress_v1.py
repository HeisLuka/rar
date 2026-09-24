#!/usr/bin/env python3
import unittest

from text_egress_v1 import (
    TextPlainEgressError,
    TextPlainWirePayloadV1,
    egress_canonical_text_v1,
    egress_story_range_v1,
    logical_text_to_wire_v1,
    verify_ingress_round_trip_v1,
    verify_semantic_plain_text_equivalence_v1,
    wire_text_to_logical_v1,
)


class TextPlainEgressV1Tests(unittest.TestCase):
    def test_one_paragraph_no_newline_is_preserved(self):
        payload = egress_canonical_text_v1("plain 😀 e\u0301")
        self.assertEqual("plain 😀 e\u0301", payload.logical_text)
        self.assertEqual(0, payload.paragraph_boundary_count)

    def test_two_canonical_paragraphs_map_cr_to_logical_lf(self):
        payload = egress_canonical_text_v1("one\rtwo")
        self.assertEqual("one\ntwo", payload.logical_text)
        self.assertEqual(1, payload.paragraph_boundary_count)
        self.assertEqual(len("one\rtwo"), payload.canonical_scalar_len)
        self.assertEqual(len("one\ntwo"), payload.logical_scalar_len)

    def test_protected_terminal_cr_is_not_appended_to_editable_selection(self):
        payload = egress_story_range_v1(
            story_id="story:q",
            story_text="ABC\r",
            provenance="imported_mature_quill_terminal_cr",
            start_scalar=0,
            end_scalar=3,
        )
        self.assertEqual("ABC", payload.canonical_text)
        self.assertEqual("ABC", payload.logical_text)

    def test_internal_editable_cr_exports_as_lf_even_with_protected_terminal(self):
        payload = egress_story_range_v1(
            story_id="story:q",
            story_text="A\rB\r",
            provenance="imported_mature_quill_terminal_cr",
            start_scalar=0,
            end_scalar=3,
        )
        self.assertEqual("A\rB", payload.canonical_text)
        self.assertEqual("A\nB", payload.logical_text)

    def test_direct_selection_of_protected_terminal_rejects(self):
        with self.assertRaises(TextPlainEgressError) as caught:
            egress_story_range_v1(
                story_id="story:q",
                story_text="ABC\r",
                provenance="imported_mature_quill_terminal_cr",
                start_scalar=3,
                end_scalar=4,
            )
        self.assertEqual("protected_story_structure", caught.exception.code)

    def test_empty_range_yields_empty_external_text(self):
        payload = egress_story_range_v1(
            story_id="story:a",
            story_text="ABC",
            provenance="chaptera_created",
            start_scalar=1,
            end_scalar=1,
        )
        self.assertEqual("", payload.canonical_text)
        self.assertEqual("", payload.logical_text)

    def test_emoji_combining_zwj_and_zero_width_are_preserved_exactly(self):
        text = "👩\u200d💻 e\u0301 \u200b"
        payload = egress_canonical_text_v1(text)
        self.assertEqual(text, payload.logical_text)

    def test_precomposed_and_decomposed_forms_remain_distinct(self):
        pre = egress_canonical_text_v1("é")
        decomp = egress_canonical_text_v1("e\u0301")
        self.assertEqual("é", pre.logical_text)
        self.assertEqual("e\u0301", decomp.logical_text)
        self.assertNotEqual(pre.logical_text, decomp.logical_text)

    def test_external_lf_is_not_accepted_as_canonical_story_input(self):
        with self.assertRaises(TextPlainEgressError) as caught:
            egress_canonical_text_v1("one\ntwo")
        self.assertEqual("invalid_canonical_text", caught.exception.code)

    def test_logical_lf_round_trips_through_ingress(self):
        payload = egress_canonical_text_v1("A\r😀\rB")
        self.assertTrue(verify_ingress_round_trip_v1(payload, newline_wire="lf"))

    def test_crlf_wire_adapter_round_trips_to_identical_canonical_story(self):
        payload = egress_canonical_text_v1("A\r😀\rB")
        wire = logical_text_to_wire_v1(payload, newline_wire="crlf")
        self.assertEqual("A\r\n😀\r\nB", wire.text)
        self.assertEqual("A\n😀\nB", wire_text_to_logical_v1(wire))
        self.assertTrue(verify_ingress_round_trip_v1(payload, newline_wire="crlf"))

    def test_crlf_wire_rejects_lone_newline_spellings(self):
        with self.assertRaises(TextPlainEgressError):
            wire_text_to_logical_v1(
                TextPlainWirePayloadV1(
                    "chaptera.text-plain-wire.v1",
                    "crlf",
                    "A\rB",
                )
            )
        with self.assertRaises(TextPlainEgressError):
            wire_text_to_logical_v1(
                TextPlainWirePayloadV1(
                    "chaptera.text-plain-wire.v1",
                    "crlf",
                    "A\nB",
                )
            )

    def test_semantic_fragment_and_plain_text_dual_flavor_are_equivalent(self):
        canonical = "A\rB😀"
        payload = egress_canonical_text_v1(canonical)
        self.assertTrue(
            verify_semantic_plain_text_equivalence_v1(
                canonical_fragment_text=canonical,
                plain_payload=payload,
            )
        )

    def test_dual_flavor_mismatch_fails_closed(self):
        payload = egress_canonical_text_v1("A\rB")
        with self.assertRaises(TextPlainEgressError) as caught:
            verify_semantic_plain_text_equivalence_v1(
                canonical_fragment_text="A\rC",
                plain_payload=payload,
            )
        self.assertEqual("dual_flavor_mismatch", caught.exception.code)

    def test_unknown_imported_provenance_fails_closed(self):
        with self.assertRaises(TextPlainEgressError) as caught:
            egress_story_range_v1(
                story_id="story:u",
                story_text="ABC\r",
                provenance="imported_unknown",
                start_scalar=0,
                end_scalar=3,
            )
        self.assertEqual("edit_domain_unknown", caught.exception.code)


if __name__ == "__main__":
    unittest.main()
