#!/usr/bin/env python3
import unittest

from paragraph_lifecycle_v1 import (
    ParagraphEditV1,
    ParagraphLifecycleError,
    ParagraphPropertiesV1,
    ParagraphV1,
    apply_paragraph_lifecycle_v1,
    build_story_paragraph_state_v1,
    replay_paragraph_lifecycle_v1,
    state_hash_v1,
    undo_paragraph_lifecycle_v1,
)


ID1 = "01900000-0000-7000-8000-000000000001"
ID2 = "01900000-0000-7000-8000-000000000002"
ID3 = "01900000-0000-7000-8000-000000000003"
ID4 = "01900000-0000-7000-8000-000000000004"


def props(name):
    return ParagraphPropertiesV1((("align", name),))


def p(pid, name, provenance="source_or_existing"):
    return ParagraphV1(pid, props(name), provenance)


def base(text="A\rB", paragraphs=None, protected=False):
    if paragraphs is None:
        paragraphs=(p("p1","left"),p("p2","right"))
    return build_story_paragraph_state_v1(
        story_id="story:1",
        story_text=text,
        paragraphs=tuple(paragraphs),
        protected_terminal_cr=protected,
    )


class ParagraphLifecycleV1Tests(unittest.TestCase):
    def test_text_edit_without_boundary_change_preserves_paragraph_identity(self):
        s = base()
        receipt = apply_paragraph_lifecycle_v1(
            base_state=s,
            edits=(ParagraphEditV1(0,1,"A","X"),),
            expected_base_state_hash=state_hash_v1(s),
        )
        self.assertEqual("X\rB", receipt.after_state.story_text)
        self.assertEqual(s.paragraphs, receipt.after_state.paragraphs)

    def test_insert_boundary_splits_and_inherits_upstream_properties(self):
        s = build_story_paragraph_state_v1(
            story_id="story:1",
            story_text="AB",
            paragraphs=(p("p1","left"),),
        )
        receipt = apply_paragraph_lifecycle_v1(
            base_state=s,
            edits=(
                ParagraphEditV1(
                    1,1,"","\r",
                    inserted_paragraph_ids=(ID1,),
                ),
            ),
            expected_base_state_hash=state_hash_v1(s),
        )
        self.assertEqual("A\rB", receipt.after_state.story_text)
        self.assertEqual(("p1",ID1), tuple(x.paragraph_id for x in receipt.after_state.paragraphs))
        self.assertEqual(props("left"), receipt.after_state.paragraphs[1].properties)
        self.assertEqual("chaptera_created", receipt.after_state.paragraphs[1].provenance)

    def test_explicit_paragraph_preset_overrides_inheritance(self):
        s = build_story_paragraph_state_v1(
            story_id="story:1",
            story_text="AB",
            paragraphs=(p("p1","left"),),
        )
        receipt = apply_paragraph_lifecycle_v1(
            base_state=s,
            edits=(
                ParagraphEditV1(
                    1,1,"","\r",
                    inserted_paragraph_ids=(ID1,),
                    inserted_property_presets=(props("center"),),
                ),
            ),
            expected_base_state_hash=state_hash_v1(s),
        )
        self.assertEqual(props("center"), receipt.after_state.paragraphs[1].properties)

    def test_delete_boundary_merges_with_upstream_identity_and_properties(self):
        s = base()
        receipt = apply_paragraph_lifecycle_v1(
            base_state=s,
            edits=(ParagraphEditV1(1,2,"\r",""),),
            expected_base_state_hash=state_hash_v1(s),
        )
        self.assertEqual("AB", receipt.after_state.story_text)
        self.assertEqual(("p1",), tuple(x.paragraph_id for x in receipt.after_state.paragraphs))
        self.assertEqual(props("left"), receipt.after_state.paragraphs[0].properties)
        self.assertEqual("p2", receipt.removed_paragraphs[0].paragraph.paragraph_id)
        self.assertEqual(props("right"), receipt.removed_paragraphs[0].paragraph.properties)

    def test_cross_paragraph_replace_removes_then_inserts_deterministically(self):
        s = build_story_paragraph_state_v1(
            story_id="story:1",
            story_text="A\rB\rC",
            paragraphs=(p("p1","A"),p("p2","B"),p("p3","C")),
        )
        receipt = apply_paragraph_lifecycle_v1(
            base_state=s,
            edits=(
                ParagraphEditV1(
                    1,4,"\rB\r","\rX\r",
                    inserted_paragraph_ids=(ID1,ID2),
                ),
            ),
            expected_base_state_hash=state_hash_v1(s),
        )
        self.assertEqual("A\rX\rC", receipt.after_state.story_text)
        self.assertEqual(
            ("p1",ID1,ID2),
            tuple(x.paragraph_id for x in receipt.after_state.paragraphs),
        )
        self.assertEqual(
            (props("A"),props("A"),props("A")),
            tuple(x.properties for x in receipt.after_state.paragraphs),
        )
        self.assertEqual(("p2","p3"), tuple(x.paragraph.paragraph_id for x in receipt.removed_paragraphs))

    def test_multi_edit_input_order_does_not_change_result(self):
        s = build_story_paragraph_state_v1(
            story_id="story:1",
            story_text="ABCD",
            paragraphs=(p("p1","A"),),
        )
        left = ParagraphEditV1(
            1,1,"","\r",
            inserted_paragraph_ids=(ID1,),
        )
        right = ParagraphEditV1(
            3,3,"","\r",
            inserted_paragraph_ids=(ID2,),
        )
        a = apply_paragraph_lifecycle_v1(
            base_state=s,
            edits=(right,left),
            expected_base_state_hash=state_hash_v1(s),
        )
        b = apply_paragraph_lifecycle_v1(
            base_state=s,
            edits=(left,right),
            expected_base_state_hash=state_hash_v1(s),
        )
        self.assertEqual(a.after_state,b.after_state)
        self.assertEqual(
            (1,3),
            tuple(e.base_start_scalar for e in a.normalized_edits),
        )

    def test_later_split_observes_properties_after_earlier_merge(self):
        s = build_story_paragraph_state_v1(
            story_id="story:1",
            story_text="A\rB\rC",
            paragraphs=(p("p1","A"),p("p2","B"),p("p3","C")),
        )
        delete_first_boundary = ParagraphEditV1(1,2,"\r","")
        insert_before_second_boundary = ParagraphEditV1(
            3,3,"","\r",
            inserted_paragraph_ids=(ID1,),
        )
        receipt = apply_paragraph_lifecycle_v1(
            base_state=s,
            edits=(insert_before_second_boundary,delete_first_boundary),
            expected_base_state_hash=state_hash_v1(s),
        )
        self.assertEqual("AB\r\rC", receipt.after_state.story_text)
        self.assertEqual(
            ("p1",ID1,"p3"),
            tuple(x.paragraph_id for x in receipt.after_state.paragraphs),
        )
        self.assertEqual(props("A"), receipt.after_state.paragraphs[1].properties)

    def test_two_inserted_boundaries_bind_ids_by_boundary_ordinal(self):
        s = build_story_paragraph_state_v1(
            story_id="story:1",
            story_text="AB",
            paragraphs=(p("p1","A"),),
        )
        receipt = apply_paragraph_lifecycle_v1(
            base_state=s,
            edits=(
                ParagraphEditV1(
                    1,1,"","\r\r",
                    inserted_paragraph_ids=(ID1,ID2),
                    inserted_property_presets=(props("X"),None),
                ),
            ),
            expected_base_state_hash=state_hash_v1(s),
        )
        self.assertEqual(("p1",ID1,ID2), tuple(x.paragraph_id for x in receipt.after_state.paragraphs))
        self.assertEqual(props("X"), receipt.after_state.paragraphs[1].properties)
        self.assertEqual(props("X"), receipt.after_state.paragraphs[2].properties)

    def test_invalid_or_missing_preallocated_ids_fail_closed(self):
        s = build_story_paragraph_state_v1(
            story_id="story:1",
            story_text="AB",
            paragraphs=(p("p1","A"),),
        )
        with self.assertRaisesRegex(ParagraphLifecycleError, "preallocated"):
            apply_paragraph_lifecycle_v1(
                base_state=s,
                edits=(ParagraphEditV1(1,1,"","\r"),),
                expected_base_state_hash=state_hash_v1(s),
            )
        with self.assertRaisesRegex(ParagraphLifecycleError, "UUIDv7"):
            apply_paragraph_lifecycle_v1(
                base_state=s,
                edits=(
                    ParagraphEditV1(
                        1,1,"","\r",
                        inserted_paragraph_ids=("not-a-uuid",),
                    ),
                ),
                expected_base_state_hash=state_hash_v1(s),
            )

    def test_overlapping_or_duplicate_script_ranges_fail_closed(self):
        s = build_story_paragraph_state_v1(
            story_id="story:1",
            story_text="ABCD",
            paragraphs=(p("p1","A"),),
        )
        with self.assertRaisesRegex(ParagraphLifecycleError, "overlap"):
            apply_paragraph_lifecycle_v1(
                base_state=s,
                edits=(
                    ParagraphEditV1(0,2,"AB","A"),
                    ParagraphEditV1(1,3,"BC","B"),
                ),
                expected_base_state_hash=state_hash_v1(s),
            )
        with self.assertRaisesRegex(ParagraphLifecycleError, "duplicate"):
            apply_paragraph_lifecycle_v1(
                base_state=s,
                edits=(
                    ParagraphEditV1(1,1,"","X"),
                    ParagraphEditV1(1,1,"","Y"),
                ),
                expected_base_state_hash=state_hash_v1(s),
            )

    def test_protected_terminal_cr_is_not_a_chaptera_split_or_edit_target(self):
        s = build_story_paragraph_state_v1(
            story_id="story:1",
            story_text="ABC\r",
            paragraphs=(p("p1","A"),),
            protected_terminal_cr=True,
        )
        # Insert exactly before protected suffix: new CR is an authoring boundary.
        ok = apply_paragraph_lifecycle_v1(
            base_state=s,
            edits=(
                ParagraphEditV1(
                    3,3,"","X\r",
                    inserted_paragraph_ids=(ID1,),
                ),
            ),
            expected_base_state_hash=state_hash_v1(s),
        )
        self.assertEqual("ABCX\r\r", ok.after_state.story_text)
        self.assertTrue(ok.after_state.protected_terminal_cr)
        self.assertEqual(("p1",ID1), tuple(x.paragraph_id for x in ok.after_state.paragraphs))

        with self.assertRaisesRegex(ParagraphLifecycleError, "protected"):
            apply_paragraph_lifecycle_v1(
                base_state=s,
                edits=(ParagraphEditV1(3,4,"\r",""),),
                expected_base_state_hash=state_hash_v1(s),
            )

    def test_lf_is_not_interpreted_as_paragraph_boundary(self):
        s = build_story_paragraph_state_v1(
            story_id="story:1",
            story_text="AB",
            paragraphs=(p("p1","A"),),
        )
        with self.assertRaisesRegex(ParagraphLifecycleError, "canonical"):
            apply_paragraph_lifecycle_v1(
                base_state=s,
                edits=(
                    ParagraphEditV1(
                        1,1,"","\n",
                        inserted_paragraph_ids=(),
                    ),
                ),
                expected_base_state_hash=state_hash_v1(s),
            )

    def test_undo_redo_replay_preserve_exact_ids_and_properties(self):
        s = base()
        receipt = apply_paragraph_lifecycle_v1(
            base_state=s,
            edits=(
                ParagraphEditV1(
                    1,2,"\r","\r",
                    inserted_paragraph_ids=(ID1,),
                ),
            ),
            expected_base_state_hash=state_hash_v1(s),
        )
        self.assertEqual(s, undo_paragraph_lifecycle_v1(receipt))
        replayed = replay_paragraph_lifecycle_v1(receipt)
        self.assertEqual(receipt.after_state,replayed)
        self.assertEqual(ID1,replayed.paragraphs[1].paragraph_id)


if __name__ == "__main__":
    unittest.main()
