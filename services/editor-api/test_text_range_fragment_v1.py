#!/usr/bin/env python3
import copy
import json
import unittest

from author_created_story_test_fixture import bind_author_created_story_graph_v1
from paragraph_lifecycle_v1 import (
    ParagraphPropertiesV1,
    ParagraphV1,
    build_story_paragraph_state_v1,
)
from revision_store import RevisionKernel
from story_edit_transaction_v1 import (
    GenericAnchoredSemanticV1,
    build_story_edit_core_state_v1,
    story_edit_core_state_from_dict,
    story_edit_core_state_to_dict,
)
from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    BaseFormatRunV1,
    TextFormatOverrideRunV1,
    build_text_format_overlay_state_v1,
    clear_text_format_property_override_v1,
    effective_property_segments_v1,
    set_text_format_property_v1,
    state_hash_v1,
)
from text_range_fragment_v1 import (
    TextRangeFragmentError,
    UnsupportedSemanticSpanV1,
    build_paste_text_range_request_v1,
    canonical_text_range_fragment_json_v1,
    capture_text_range_fragment_v1,
)


DOC = "doc:fragment"
SOURCE_HASH = "b" * 64
SOURCE_STORY = "01900000-0000-7000-8000-000000000401"
DEST_STORY = "01900000-0000-7000-8000-000000000402"
DEST_FRAME_ID = "01900000-0000-7000-8000-000000000403"
DEST_PAGE_ID = "page:fragment-destination"
NEW1 = "01900000-0000-7000-8000-000000000011"


def fmt(*, size=12000, bold=False, italic=False, color="#000000", font="font:resolved"):
    return BaseCharacterFormatV1(font, size, bold, italic, color)


def props(**values):
    return ParagraphPropertiesV1(tuple(sorted(values.items())))


def core(
    story_id,
    text,
    *,
    basefmt=None,
    overrides=(),
    paragraph_props=None,
    provenance="chaptera_created",
    unsupported=(),
    active_features=(),
    generic=(),
):
    protected = provenance == "imported_mature_quill_terminal_cr"
    boundary_count = text.count("\r") - (1 if protected else 0)
    if paragraph_props is None:
        paragraph_props = [ParagraphPropertiesV1(()) for _ in range(boundary_count + 1)]
    paragraphs = tuple(
        ParagraphV1(f"{story_id}:p:{i}", paragraph_props[i], "source_or_existing")
        for i in range(boundary_count + 1)
    )
    pstate = build_story_paragraph_state_v1(
        story_id=story_id,
        story_text=text,
        paragraphs=paragraphs,
        protected_terminal_cr=protected,
    )
    if basefmt is None:
        basefmt = fmt()
    base_runs = () if not text else (BaseFormatRunV1(0, len(text), basefmt),)
    fstate = build_text_format_overlay_state_v1(
        story_id=story_id,
        base_revision_id="format:base",
        story_scalar_len=len(text),
        base_runs=base_runs,
        overrides=tuple(overrides),
    )
    return build_story_edit_core_state_v1(
        story_id=story_id,
        provenance=provenance,
        paragraph_state=pstate,
        format_state=fstate,
        generic_anchors=tuple(generic),
        unsupported_anchored_semantics=tuple(unsupported),
        active_paragraph_features=tuple(active_features),
    )


def project_for_destination(state, *, story_model=None):
    project = {
        "schema_version": "pub-editor-v0.6",
        "source_hash": SOURCE_HASH,
        "operations": [],
        "stories": {DEST_STORY: state.paragraph_state.story_text},
        "story_models": {
            DEST_STORY: (
                story_edit_core_state_to_dict(state)
                if story_model is None
                else story_model
            )
        },
    }
    return bind_author_created_story_graph_v1(
        project,
        story_id=DEST_STORY,
        frame_id=DEST_FRAME_ID,
        page_id=DEST_PAGE_ID,
    )


def effective(state, prop, scalar):
    return effective_property_segments_v1(
        state=state.format_state,
        prop=prop,
        start_scalar=scalar,
        end_scalar=scalar + 1,
    )[0].value


class TextRangeFragmentV1Tests(unittest.TestCase):
    def test_capture_inside_one_format_span_uses_local_scalar_coordinates(self):
        state = core(
            SOURCE_STORY,
            "ABCDE",
            overrides=(TextFormatOverrideRunV1(1, 4, "bold", True),),
        )
        receipt = capture_text_range_fragment_v1(
            state=state,
            start_scalar=1,
            end_scalar=4,
        )
        self.assertEqual("BCD", receipt.fragment.text)
        bold = [r for r in receipt.fragment.format_runs if r.property == "bold"]
        self.assertEqual(1, len(bold))
        self.assertEqual((0, 3, True), (
            bold[0].start_offset,
            bold[0].end_offset,
            bold[0].value,
        ))

    def test_mixed_effective_formatting_is_clipped_and_complete(self):
        state = core(
            SOURCE_STORY,
            "ABCDE",
            overrides=(
                TextFormatOverrideRunV1(0, 2, "bold", True),
                TextFormatOverrideRunV1(2, 5, "italic", True),
            ),
        )
        fragment = capture_text_range_fragment_v1(
            state=state,
            start_scalar=1,
            end_scalar=4,
        ).fragment
        bold = [r for r in fragment.format_runs if r.property == "bold"]
        italic = [r for r in fragment.format_runs if r.property == "italic"]
        self.assertEqual([(0, 1, True), (1, 3, False)], [
            (r.start_offset, r.end_offset, r.value) for r in bold
        ])
        self.assertEqual([(0, 1, False), (1, 3, True)], [
            (r.start_offset, r.end_offset, r.value) for r in italic
        ])

    def test_equivalent_effective_histories_produce_byte_identical_fragment(self):
        base = core(SOURCE_STORY, "ABCD")
        a1 = set_text_format_property_v1(
            state=base.format_state,
            start_scalar=0,
            end_scalar=4,
            prop="bold",
            value=True,
            expected_state_hash=state_hash_v1(base.format_state),
        ).after_state
        a2 = clear_text_format_property_override_v1(
            state=a1,
            start_scalar=2,
            end_scalar=4,
            prop="bold",
            expected_state_hash=state_hash_v1(a1),
        ).after_state

        direct = build_text_format_overlay_state_v1(
            story_id=SOURCE_STORY,
            base_revision_id="format:base",
            story_scalar_len=4,
            base_runs=(BaseFormatRunV1(0, 4, fmt()),),
            overrides=(TextFormatOverrideRunV1(0, 2, "bold", True),),
        )
        state_a = build_story_edit_core_state_v1(
            story_id=SOURCE_STORY,
            provenance="chaptera_created",
            paragraph_state=base.paragraph_state,
            format_state=a2,
        )
        state_b = build_story_edit_core_state_v1(
            story_id=SOURCE_STORY,
            provenance="chaptera_created",
            paragraph_state=base.paragraph_state,
            format_state=direct,
        )
        fa = capture_text_range_fragment_v1(
            state=state_a, start_scalar=0, end_scalar=4
        ).fragment
        fb = capture_text_range_fragment_v1(
            state=state_b, start_scalar=0, end_scalar=4
        ).fragment
        self.assertEqual(
            canonical_text_range_fragment_json_v1(fa),
            canonical_text_range_fragment_json_v1(fb),
        )

    def test_hyperlink_intersection_is_explicit_capture_unsupported(self):
        state = core(
            SOURCE_STORY,
            "ABCDE",
            unsupported=("hyperlink",),
        )
        with self.assertRaises(TextRangeFragmentError) as caught:
            capture_text_range_fragment_v1(
                state=state,
                start_scalar=1,
                end_scalar=4,
                unsupported_semantic_spans=(
                    UnsupportedSemanticSpanV1("hyperlink", 2, 5),
                ),
            )
        self.assertEqual("capture_unsupported", caught.exception.code)

    def test_unsupported_semantic_outside_selection_can_be_proven_nonoverlapping(self):
        state = core(
            SOURCE_STORY,
            "ABCDEFG",
            unsupported=("hyperlink",),
        )
        receipt = capture_text_range_fragment_v1(
            state=state,
            start_scalar=0,
            end_scalar=2,
            unsupported_semantic_spans=(
                UnsupportedSemanticSpanV1("hyperlink", 5, 7),
            ),
        )
        self.assertEqual("AB", receipt.fragment.text)

    def test_missing_span_inventory_for_known_unsupported_class_fails_closed(self):
        state = core(
            SOURCE_STORY,
            "ABCDE",
            unsupported=("hyperlink",),
        )
        with self.assertRaises(TextRangeFragmentError) as caught:
            capture_text_range_fragment_v1(
                state=state,
                start_scalar=0,
                end_scalar=2,
            )
        self.assertEqual("semantic_inventory_incomplete", caught.exception.code)

    def test_generic_persistent_semantic_intersection_is_not_silently_dropped(self):
        anchor = GenericAnchoredSemanticV1(
            "bookmark:1", "bookmark", 1, 3, False,
            "left", "right", "replacement",
        )
        state = core(SOURCE_STORY, "ABCDE", generic=(anchor,))
        with self.assertRaises(TextRangeFragmentError) as caught:
            capture_text_range_fragment_v1(
                state=state,
                start_scalar=0,
                end_scalar=2,
            )
        self.assertEqual("capture_unsupported", caught.exception.code)

    def test_multi_paragraph_text_with_no_extra_properties_is_lossless_under_base(self):
        state = core(SOURCE_STORY, "A\rB")
        fragment = capture_text_range_fragment_v1(
            state=state,
            start_scalar=0,
            end_scalar=3,
        ).fragment
        self.assertEqual("A\rB", fragment.text)
        self.assertEqual("lossless_under_base", fragment.paragraph_semantics)
        self.assertEqual((), fragment.diagnostics)

    def test_paragraph_alignment_is_explicit_loss_not_silent_transfer(self):
        state = core(
            SOURCE_STORY,
            "A\rB",
            paragraph_props=(props(align="center"), props()),
        )
        fragment = capture_text_range_fragment_v1(
            state=state,
            start_scalar=0,
            end_scalar=1,
        ).fragment
        self.assertEqual("paragraph_semantics_loss", fragment.paragraph_semantics)
        self.assertEqual(("paragraph_semantics_loss",), fragment.diagnostics)

    def test_active_paragraph_feature_is_also_explicit_loss(self):
        state = core(
            SOURCE_STORY,
            "ABC",
            active_features=("alignment",),
        )
        fragment = capture_text_range_fragment_v1(
            state=state,
            start_scalar=0,
            end_scalar=2,
        ).fragment
        self.assertEqual("paragraph_semantics_loss", fragment.paragraph_semantics)

    def test_paste_loss_requires_explicit_acknowledgement(self):
        source = core(
            SOURCE_STORY,
            "A",
            paragraph_props=(props(align="center"),),
        )
        fragment = capture_text_range_fragment_v1(
            state=source,
            start_scalar=0,
            end_scalar=1,
        ).fragment
        with self.assertRaises(TextRangeFragmentError) as caught:
            build_paste_text_range_request_v1(
                fragment=fragment,
                document_id=DOC,
                source_hash=SOURCE_HASH,
                base_revision_id="rev:1",
                client_operation_id="paste-op-00000001",
                destination_story_id=DEST_STORY,
                destination_start_scalar=0,
                destination_end_scalar=0,
                expected_before="",
            )
        self.assertEqual("paragraph_semantics_loss", caught.exception.code)

    def test_atomic_paste_replaces_nonempty_destination_and_preserves_supported_format(self):
        source = core(
            SOURCE_STORY,
            "XY",
            basefmt=fmt(size=18000, bold=True, color="#112233", font="font:source"),
        )
        fragment = capture_text_range_fragment_v1(
            state=source,
            start_scalar=0,
            end_scalar=2,
        ).fragment

        dest = core(
            DEST_STORY,
            "abcd",
            basefmt=fmt(size=9000, bold=False, color="#000000", font="font:dest"),
        )
        kernel = RevisionKernel()
        project = project_for_destination(dest)
        baseline = kernel.register_baseline(
            document_id=DOC,
            source_hash=SOURCE_HASH,
            project=project,
        )
        plan = build_paste_text_range_request_v1(
            fragment=fragment,
            document_id=DOC,
            source_hash=SOURCE_HASH,
            base_revision_id=baseline.revision_id,
            client_operation_id="paste-op-00000002",
            destination_story_id=DEST_STORY,
            destination_start_scalar=1,
            destination_end_scalar=3,
            expected_before="bc",
        )
        result = kernel.commit_story_edit_transaction(plan.request)
        self.assertEqual("chaptera.commit-accepted.v1", result["protocol_version"])
        after = story_edit_core_state_from_dict(
            kernel.current_revision(DOC).project["story_models"][DEST_STORY]
        )
        self.assertEqual("aXYd", after.paragraph_state.story_text)
        self.assertEqual(18000, effective(after, "font_size_emu", 1))
        self.assertTrue(effective(after, "bold", 1))
        self.assertEqual("#112233", effective(after, "text_color_rgb", 1))
        # Font-resource cloning is a non-goal: inserted base inherits destination.
        inserted_base = [
            run for run in after.format_state.base_runs
            if run.start_scalar <= 1 < run.end_scalar
        ][0]
        self.assertEqual("font:dest", inserted_base.format.font_resource_id)

    def test_paste_at_story_start_and_end(self):
        source = core(SOURCE_STORY, "X")
        fragment = capture_text_range_fragment_v1(
            state=source, start_scalar=0, end_scalar=1
        ).fragment
        for start, expected in ((0, "XAB"), (2, "ABX")):
            dest = core(DEST_STORY, "AB")
            kernel = RevisionKernel()
            project = project_for_destination(dest)
            baseline = kernel.register_baseline(
                document_id=DOC,
                source_hash=SOURCE_HASH,
                project=project,
            )
            plan = build_paste_text_range_request_v1(
                fragment=fragment,
                document_id=DOC,
                source_hash=SOURCE_HASH,
                base_revision_id=baseline.revision_id,
                client_operation_id=f"paste-op-{start:08d}",
                destination_story_id=DEST_STORY,
                destination_start_scalar=start,
                destination_end_scalar=start,
                expected_before="",
            )
            kernel.commit_story_edit_transaction(plan.request)
            after = story_edit_core_state_from_dict(
                kernel.current_revision(DOC).project["story_models"][DEST_STORY]
            )
            self.assertEqual(expected, after.paragraph_state.story_text)

    def test_multi_paragraph_paste_uses_destination_paragraph_lifecycle_ids(self):
        source = core(SOURCE_STORY, "A\rB")
        fragment = capture_text_range_fragment_v1(
            state=source, start_scalar=0, end_scalar=3
        ).fragment
        dest = core(DEST_STORY, "Z")
        kernel = RevisionKernel()
        project = project_for_destination(dest)
        baseline = kernel.register_baseline(
            document_id=DOC, source_hash=SOURCE_HASH, project=project
        )
        plan = build_paste_text_range_request_v1(
            fragment=fragment,
            document_id=DOC,
            source_hash=SOURCE_HASH,
            base_revision_id=baseline.revision_id,
            client_operation_id="paste-op-00000005",
            destination_story_id=DEST_STORY,
            destination_start_scalar=1,
            destination_end_scalar=1,
            expected_before="",
            paragraph_inserted_ids=(NEW1,),
        )
        kernel.commit_story_edit_transaction(plan.request)
        after = story_edit_core_state_from_dict(
            kernel.current_revision(DOC).project["story_models"][DEST_STORY]
        )
        self.assertEqual("ZA\rB", after.paragraph_state.story_text)
        self.assertEqual(
            NEW1,
            after.paragraph_state.paragraphs[1].paragraph_id,
        )

    def test_copy_paste_retry_inverse_and_reopen_are_stable(self):
        source = core(
            SOURCE_STORY,
            "XY",
            overrides=(TextFormatOverrideRunV1(0, 2, "italic", True),),
        )
        fragment = capture_text_range_fragment_v1(
            state=source, start_scalar=0, end_scalar=2
        ).fragment
        dest = core(DEST_STORY, "AB")
        before_dict = story_edit_core_state_to_dict(dest)
        kernel = RevisionKernel()
        project = project_for_destination(dest, story_model=before_dict)
        baseline = kernel.register_baseline(
            document_id=DOC, source_hash=SOURCE_HASH, project=project
        )
        plan = build_paste_text_range_request_v1(
            fragment=fragment,
            document_id=DOC,
            source_hash=SOURCE_HASH,
            base_revision_id=baseline.revision_id,
            client_operation_id="paste-op-00000006",
            destination_story_id=DEST_STORY,
            destination_start_scalar=1,
            destination_end_scalar=1,
            expected_before="",
        )
        first = kernel.commit_story_edit_transaction(copy.deepcopy(plan.request))
        second = kernel.commit_story_edit_transaction(copy.deepcopy(plan.request))
        self.assertEqual(first, second)
        operation = first["canonical_operation"]
        self.assertEqual(before_dict, operation["inverse_state"])
        reopened = json.loads(json.dumps(operation["after_state"], ensure_ascii=False))
        self.assertEqual(
            operation["after_state"],
            story_edit_core_state_to_dict(
                story_edit_core_state_from_dict(reopened)
            ),
        )


if __name__ == "__main__":
    unittest.main()
