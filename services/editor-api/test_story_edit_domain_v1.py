#!/usr/bin/env python3
import unittest

from story_edit_domain_v1 import (
    StoryEditDomainError,
    delete_forward_decision_v1,
    derive_story_edit_domain_v1,
    reopen_story_edit_domain_input_v1,
    replace_story_range_in_domain_v1,
    select_all_range_v1,
    serialize_story_edit_domain_input_v1,
    story_end_boundary_v1,
    validate_ordinary_story_range_v1,
)


class StoryEditDomainV1Tests(unittest.TestCase):
    def test_chaptera_empty_and_nonempty_story_use_full_extent(self):
        empty = derive_story_edit_domain_v1(
            story_id="a",
            story_text="",
            provenance="chaptera_created",
        )
        self.assertEqual((0, 0), select_all_range_v1(empty))
        self.assertEqual(0, story_end_boundary_v1(empty))
        self.assertEqual((), empty.protected_ranges)

        nonempty = derive_story_edit_domain_v1(
            story_id="a",
            story_text="A\rB",
            provenance="chaptera_created",
        )
        self.assertEqual((0, 3), select_all_range_v1(nonempty))
        self.assertEqual(3, story_end_boundary_v1(nonempty))

    def test_chaptera_authored_final_cr_is_not_auto_protected(self):
        domain = derive_story_edit_domain_v1(
            story_id="a",
            story_text="A\r",
            provenance="chaptera_created",
        )
        self.assertEqual((0, 2), select_all_range_v1(domain))
        self.assertEqual((), domain.protected_ranges)
        validate_ordinary_story_range_v1(
            domain=domain,
            start_scalar=1,
            end_scalar=2,
        )

    def test_imported_one_empty_terminal_cr_story_has_zero_editable_scalars(self):
        domain = derive_story_edit_domain_v1(
            story_id="q",
            story_text="\r",
            provenance="imported_mature_quill_terminal_cr",
        )
        self.assertEqual(1, domain.raw_scalar_len)
        self.assertEqual(0, domain.editable_scalar_len)
        self.assertEqual((0, 0), select_all_range_v1(domain))
        self.assertEqual(0, story_end_boundary_v1(domain))
        self.assertEqual(1, len(domain.protected_ranges))
        self.assertEqual((0, 1), (
            domain.protected_ranges[0].start_scalar,
            domain.protected_ranges[0].end_scalar,
        ))

    def test_imported_nonempty_terminal_cr_protects_only_final_scalar(self):
        domain = derive_story_edit_domain_v1(
            story_id="q",
            story_text="A\rB\r",
            provenance="imported_mature_quill_terminal_cr",
        )
        self.assertEqual((0, 3), select_all_range_v1(domain))
        self.assertEqual((3, 4), (
            domain.protected_ranges[0].start_scalar,
            domain.protected_ranges[0].end_scalar,
        ))

        # Internal CR remains ordinary editable structure.
        validate_ordinary_story_range_v1(
            domain=domain,
            start_scalar=1,
            end_scalar=2,
        )

    def test_insertion_at_editable_end_occurs_before_protected_suffix(self):
        result = replace_story_range_in_domain_v1(
            story_id="q",
            story_text="ABC\r",
            provenance="imported_mature_quill_terminal_cr",
            start_scalar=3,
            end_scalar=3,
            expected_before="",
            replacement_text="X",
        )
        self.assertEqual("ABCX\r", result.after_text)

    def test_direct_replacement_crossing_protected_suffix_rejects_without_clamp(self):
        domain = derive_story_edit_domain_v1(
            story_id="q",
            story_text="ABC\r",
            provenance="imported_mature_quill_terminal_cr",
        )
        with self.assertRaises(StoryEditDomainError) as caught:
            validate_ordinary_story_range_v1(
                domain=domain,
                start_scalar=2,
                end_scalar=4,
            )
        self.assertEqual("protected_story_structure", caught.exception.code)

    def test_select_all_replacement_preserves_protected_suffix(self):
        result = replace_story_range_in_domain_v1(
            story_id="q",
            story_text="ABC\r",
            provenance="imported_mature_quill_terminal_cr",
            start_scalar=0,
            end_scalar=3,
            expected_before="ABC",
            replacement_text="Z",
        )
        self.assertEqual("Z\r", result.after_text)

    def test_delete_forward_at_editable_end_is_boundary_noop(self):
        domain = derive_story_edit_domain_v1(
            story_id="q",
            story_text="ABC\r",
            provenance="imported_mature_quill_terminal_cr",
        )
        decision = delete_forward_decision_v1(
            domain=domain,
            caret_boundary=3,
        )
        self.assertEqual("boundary_noop", decision.action)
        self.assertEqual((3, 3), (decision.start_scalar, decision.end_scalar))

    def test_delete_forward_inside_domain_targets_one_admitted_scalar(self):
        domain = derive_story_edit_domain_v1(
            story_id="a",
            story_text="ABC",
            provenance="chaptera_created",
        )
        decision = delete_forward_decision_v1(
            domain=domain,
            caret_boundary=1,
        )
        self.assertEqual("delete_one_scalar", decision.action)
        self.assertEqual((1, 2), (decision.start_scalar, decision.end_scalar))

    def test_unknown_imported_provenance_fails_closed_even_with_trailing_cr(self):
        domain = derive_story_edit_domain_v1(
            story_id="u",
            story_text="ABC\r",
            provenance="imported_unknown",
        )
        self.assertEqual("edit_domain_unknown", domain.status)
        with self.assertRaises(StoryEditDomainError) as caught:
            select_all_range_v1(domain)
        self.assertEqual("edit_domain_unknown", caught.exception.code)

    def test_proven_terminal_cr_claim_requires_actual_final_cr(self):
        with self.assertRaises(StoryEditDomainError) as caught:
            derive_story_edit_domain_v1(
                story_id="q",
                story_text="ABC",
                provenance="imported_mature_quill_terminal_cr",
            )
        self.assertEqual("invalid_provenance", caught.exception.code)

    def test_reopen_replays_explicit_provenance_not_trailing_character_heuristic(self):
        chaptera_payload = serialize_story_edit_domain_input_v1(
            story_id="a",
            story_text="ABC\r",
            provenance="chaptera_created",
        )
        chaptera = reopen_story_edit_domain_input_v1(chaptera_payload)
        self.assertEqual((0, 4), select_all_range_v1(chaptera))
        self.assertEqual((), chaptera.protected_ranges)

        imported_payload = serialize_story_edit_domain_input_v1(
            story_id="q",
            story_text="ABC\r",
            provenance="imported_mature_quill_terminal_cr",
        )
        imported = reopen_story_edit_domain_input_v1(imported_payload)
        self.assertEqual((0, 3), select_all_range_v1(imported))
        self.assertEqual(1, len(imported.protected_ranges))


if __name__ == "__main__":
    unittest.main()
