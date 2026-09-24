#!/usr/bin/env python3
import copy
import json
import unittest

from paragraph_lifecycle_v1 import (
    ParagraphPropertiesV1,
    ParagraphV1,
    build_story_paragraph_state_v1,
)
from revision_store import RevisionKernel
from story_edit_transaction_v1 import (
    GenericAnchoredSemanticV1,
    build_story_edit_core_state_v1,
    execute_story_edit_transaction_v1,
    restore_story_edit_transaction_before_v1,
    story_edit_core_state_from_dict,
    story_edit_core_state_id_v1,
    story_edit_core_state_to_dict,
)
from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    BaseFormatRunV1,
    TextFormatOverrideRunV1,
    build_text_format_overlay_state_v1,
    effective_property_segments_v1,
)


DOCUMENT_ID = "doc:story-tx"
SOURCE_HASH = "a" * 64
STORY_ID = "story:1"
P1 = "paragraph:1"
P2 = "paragraph:2"
P3 = "paragraph:3"
NEW1 = "01900000-0000-7000-8000-000000000001"
NEW2 = "01900000-0000-7000-8000-000000000002"


def fmt(*, size=12000, bold=False, italic=False, color="#000000"):
    return BaseCharacterFormatV1(
        "font:resolved",
        size,
        bold,
        italic,
        color,
    )


def props(name="left"):
    return ParagraphPropertiesV1((("align", name),))


def paragraph(pid, name="left"):
    return ParagraphV1(pid, props(name), "source_or_existing")


def paragraph_count_for(text, protected):
    boundaries = text.count("\r")
    if protected:
        boundaries -= 1
    return boundaries + 1


def make_core_state(
    text,
    *,
    provenance="chaptera_created",
    overrides=(),
    unsupported=(),
    active_features=(),
    generic_anchors=(),
    empty_preset=None,
    paragraph_names=None,
):
    protected = provenance == "imported_mature_quill_terminal_cr"
    count = paragraph_count_for(text, protected)
    if paragraph_names is None:
        paragraph_names = ["left"] * count
    ids = [P1, P2, P3, "paragraph:4", "paragraph:5"][:count]
    paragraphs = tuple(
        paragraph(pid, name)
        for pid, name in zip(ids, paragraph_names)
    )
    pstate = build_story_paragraph_state_v1(
        story_id=STORY_ID,
        story_text=text,
        paragraphs=paragraphs,
        protected_terminal_cr=protected,
    )
    base_runs = (
        ()
        if len(text) == 0
        else (BaseFormatRunV1(0, len(text), fmt()),)
    )
    fstate = build_text_format_overlay_state_v1(
        story_id=STORY_ID,
        base_revision_id="format:base",
        story_scalar_len=len(text),
        base_runs=base_runs,
        overrides=tuple(overrides),
    )
    return build_story_edit_core_state_v1(
        story_id=STORY_ID,
        provenance=provenance,
        paragraph_state=pstate,
        format_state=fstate,
        generic_anchors=tuple(generic_anchors),
        unsupported_anchored_semantics=tuple(unsupported),
        active_paragraph_features=tuple(active_features),
        empty_story_preset_format=empty_preset,
    )


def make_project(core):
    text = core.paragraph_state.story_text
    return {
        "schema_version": "pub-editor-v0.6",
        "source_hash": SOURCE_HASH,
        "operations": [],
        "stories": {STORY_ID: text},
        "story_models": {STORY_ID: story_edit_core_state_to_dict(core)},
    }


def effective(core, prop, scalar):
    segments = effective_property_segments_v1(
        state=core.format_state,
        prop=prop,
        start_scalar=scalar,
        end_scalar=scalar + 1,
    )
    return segments[0].value


class StoryEditTransactionV1Tests(unittest.TestCase):
    def setUp(self):
        self.kernel = RevisionKernel()

    def register(self, core):
        project = make_project(core)
        baseline = self.kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=project,
        )
        return project, baseline

    def request(
        self,
        baseline,
        *,
        op_id,
        start,
        end,
        expected,
        replacement,
        paragraph_ids=(),
        paragraph_presets=(),
        typing=None,
        fragment_runs=(),
        incoming_semantics=(),
        base_revision_id=None,
    ):
        return {
            "protocol_version": "chaptera.story-edit-transaction-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": base_revision_id or baseline.revision_id,
            "client_operation_id": op_id,
            "command": {
                "kind": "story_edit_transaction",
                "story_id": STORY_ID,
                "start_scalar": start,
                "end_scalar": end,
                "expected_before": expected,
                "replacement_text": replacement,
                "paragraph_inserted_ids": list(paragraph_ids),
                "paragraph_inserted_property_presets": list(paragraph_presets),
                "typing_format": typing,
                "fragment_format_runs": list(fragment_runs),
                "incoming_semantic_kinds": list(incoming_semantics),
            },
        }

    def current_core(self):
        raw = self.kernel.current_revision(DOCUMENT_ID).project["story_models"][STORY_ID]
        return story_edit_core_state_from_dict(raw)

    def test_plain_insert_inside_formatted_text_is_one_revision(self):
        core = make_core_state(
            "ABC",
            overrides=(TextFormatOverrideRunV1(0, 3, "bold", True),),
        )
        _, baseline = self.register(core)
        result = self.kernel.commit_story_edit_transaction(
            self.request(
                baseline,
                op_id="story-tx-00000001",
                start=1,
                end=1,
                expected="",
                replacement="X",
            )
        )
        self.assertEqual("chaptera.commit-accepted.v1", result["protocol_version"])
        self.assertEqual("story_edit_transaction", result["canonical_operation"]["kind"])
        self.assertEqual("AXBC", self.current_core().paragraph_state.story_text)
        self.assertTrue(effective(self.current_core(), "bold", 1))
        self.assertEqual(baseline.revision_id, self.kernel.current_revision(DOCUMENT_ID).parent_revision_id)
        self.assertEqual(1, len(self.kernel.current_revision(DOCUMENT_ID).project["operations"]))

    def test_delete_crossing_format_span_rebases_without_stale_extent(self):
        core = make_core_state(
            "ABCDE",
            overrides=(TextFormatOverrideRunV1(1, 4, "bold", True),),
        )
        _, baseline = self.register(core)
        result = self.kernel.commit_story_edit_transaction(
            self.request(
                baseline,
                op_id="story-tx-00000002",
                start=2,
                end=4,
                expected="CD",
                replacement="",
            )
        )
        self.assertEqual("chaptera.commit-accepted.v1", result["protocol_version"])
        after = self.current_core()
        self.assertEqual("ABE", after.paragraph_state.story_text)
        self.assertEqual(3, after.format_state.story_scalar_len)
        self.assertEqual(
            (TextFormatOverrideRunV1(1, 2, "bold", True),),
            after.format_state.overrides,
        )

    def test_hyperlink_bearing_story_rejects_before_any_revision_change(self):
        core = make_core_state("ABC", unsupported=("hyperlink",))
        _, baseline = self.register(core)
        result = self.kernel.commit_story_edit_transaction(
            self.request(
                baseline,
                op_id="story-tx-00000003",
                start=1,
                end=1,
                expected="",
                replacement="X",
            )
        )
        self.assertEqual("anchored_semantics_unsupported", result["code"])
        self.assertEqual(baseline.revision_id, self.kernel.current_revision(DOCUMENT_ID).revision_id)
        self.assertEqual(0, len(self.kernel.current_revision(DOCUMENT_ID).project["operations"]))

    def test_incoming_hyperlink_fragment_is_also_fail_closed(self):
        core = make_core_state("ABC")
        _, baseline = self.register(core)
        result = self.kernel.commit_story_edit_transaction(
            self.request(
                baseline,
                op_id="story-tx-00000004",
                start=1,
                end=1,
                expected="",
                replacement="X",
                incoming_semantics=("hyperlink",),
            )
        )
        self.assertEqual("anchored_semantics_unsupported", result["code"])
        self.assertEqual(baseline.revision_id, self.kernel.current_revision(DOCUMENT_ID).revision_id)

    def test_replace_across_paragraph_boundary_updates_text_and_ids_atomically(self):
        core = make_core_state(
            "A\rB",
            paragraph_names=("upstream", "downstream"),
        )
        _, baseline = self.register(core)
        result = self.kernel.commit_story_edit_transaction(
            self.request(
                baseline,
                op_id="story-tx-00000005",
                start=1,
                end=3,
                expected="\rB",
                replacement="X\rY",
                paragraph_ids=(NEW1,),
            )
        )
        self.assertEqual("chaptera.commit-accepted.v1", result["protocol_version"])
        after = self.current_core()
        self.assertEqual("AX\rY", after.paragraph_state.story_text)
        self.assertEqual(
            (P1, NEW1),
            tuple(p.paragraph_id for p in after.paragraph_state.paragraphs),
        )
        self.assertEqual(
            ("downstream" in str(result["canonical_operation"]["paragraph_lifecycle"])),
            False,
        )
        self.assertEqual(
            [P2],
            result["canonical_operation"]["paragraph_lifecycle"]["removed_paragraph_ids"],
        )

    def test_enter_split_and_typing_format_commit_together(self):
        core = make_core_state("AB")
        _, baseline = self.register(core)
        result = self.kernel.commit_story_edit_transaction(
            self.request(
                baseline,
                op_id="story-tx-00000006",
                start=1,
                end=1,
                expected="",
                replacement="\r",
                paragraph_ids=(NEW1,),
                typing={"bold": True},
            )
        )
        self.assertEqual("chaptera.commit-accepted.v1", result["protocol_version"])
        after = self.current_core()
        self.assertEqual("A\rB", after.paragraph_state.story_text)
        self.assertEqual((P1, NEW1), tuple(p.paragraph_id for p in after.paragraph_state.paragraphs))
        self.assertTrue(effective(after, "bold", 1))
        self.assertEqual("typing_snapshot", result["canonical_operation"]["format_assignment"]["source"])

    def test_boundary_delete_merge_keeps_upstream_paragraph(self):
        core = make_core_state(
            "A\rB",
            paragraph_names=("left", "right"),
        )
        _, baseline = self.register(core)
        result = self.kernel.commit_story_edit_transaction(
            self.request(
                baseline,
                op_id="story-tx-00000007",
                start=1,
                end=2,
                expected="\r",
                replacement="",
            )
        )
        after = self.current_core()
        self.assertEqual("AB", after.paragraph_state.story_text)
        self.assertEqual((P1,), tuple(p.paragraph_id for p in after.paragraph_state.paragraphs))
        self.assertEqual(
            [P2],
            result["canonical_operation"]["paragraph_lifecycle"]["removed_paragraph_ids"],
        )

    def test_paragraph_feature_boundary_edit_rejects_without_text_first_repair_later(self):
        core = make_core_state("A\rB", active_features=("alignment",))
        _, baseline = self.register(core)
        result = self.kernel.commit_story_edit_transaction(
            self.request(
                baseline,
                op_id="story-tx-00000008",
                start=1,
                end=2,
                expected="\r",
                replacement="",
            )
        )
        self.assertEqual("paragraph_feature_lifecycle_unsupported", result["code"])
        self.assertEqual(baseline.revision_id, self.kernel.current_revision(DOCUMENT_ID).revision_id)
        self.assertEqual("A\rB", self.current_core().paragraph_state.story_text)

    def test_plain_text_edit_is_allowed_when_paragraph_feature_topology_does_not_change(self):
        core = make_core_state("ABC", active_features=("alignment",))
        _, baseline = self.register(core)
        result = self.kernel.commit_story_edit_transaction(
            self.request(
                baseline,
                op_id="story-tx-00000009",
                start=1,
                end=2,
                expected="B",
                replacement="X",
            )
        )
        self.assertEqual("chaptera.commit-accepted.v1", result["protocol_version"])
        self.assertEqual("AXC", self.current_core().paragraph_state.story_text)

    def test_rich_fragment_character_format_is_admitted_without_hyperlink_semantics(self):
        core = make_core_state("AB")
        _, baseline = self.register(core)
        result = self.kernel.commit_story_edit_transaction(
            self.request(
                baseline,
                op_id="story-tx-00000010",
                start=1,
                end=1,
                expected="",
                replacement="XY",
                fragment_runs=(
                    {
                        "start_offset": 0,
                        "end_offset": 2,
                        "property": "italic",
                        "value": True,
                    },
                ),
            )
        )
        self.assertEqual("chaptera.commit-accepted.v1", result["protocol_version"])
        after = self.current_core()
        self.assertTrue(effective(after, "italic", 1))
        self.assertTrue(effective(after, "italic", 2))
        self.assertEqual("semantic_fragment", result["canonical_operation"]["format_assignment"]["source"])

    def test_imported_one_empty_story_inserts_before_protected_terminal_cr(self):
        core = make_core_state(
            "\r",
            provenance="imported_mature_quill_terminal_cr",
        )
        _, baseline = self.register(core)
        result = self.kernel.commit_story_edit_transaction(
            self.request(
                baseline,
                op_id="story-tx-00000011",
                start=0,
                end=0,
                expected="",
                replacement="A",
            )
        )
        self.assertEqual("chaptera.commit-accepted.v1", result["protocol_version"])
        after = self.current_core()
        self.assertEqual("A\r", after.paragraph_state.story_text)
        self.assertTrue(after.paragraph_state.protected_terminal_cr)

    def test_select_all_like_replace_preserves_protected_suffix(self):
        core = make_core_state(
            "ABC\r",
            provenance="imported_mature_quill_terminal_cr",
        )
        _, baseline = self.register(core)
        result = self.kernel.commit_story_edit_transaction(
            self.request(
                baseline,
                op_id="story-tx-00000012",
                start=0,
                end=3,
                expected="ABC",
                replacement="Z",
            )
        )
        self.assertEqual("chaptera.commit-accepted.v1", result["protocol_version"])
        self.assertEqual("Z\r", self.current_core().paragraph_state.story_text)

    def test_overlap_with_protected_source_structure_rejects_zero_commit(self):
        core = make_core_state(
            "ABC\r",
            provenance="imported_mature_quill_terminal_cr",
        )
        _, baseline = self.register(core)
        result = self.kernel.commit_story_edit_transaction(
            self.request(
                baseline,
                op_id="story-tx-00000013",
                start=2,
                end=4,
                expected="C\r",
                replacement="X",
            )
        )
        self.assertEqual("protected_story_structure", result["code"])
        self.assertEqual(baseline.revision_id, self.kernel.current_revision(DOCUMENT_ID).revision_id)
        self.assertEqual("ABC\r", self.current_core().paragraph_state.story_text)

    def test_generic_anchor_uses_shared_transform_inside_same_candidate(self):
        anchor = GenericAnchoredSemanticV1(
            semantic_id="bookmark:1",
            semantic_kind="bookmark",
            start_scalar=1,
            end_scalar=3,
            allow_empty=False,
            start_affinity="left",
            end_affinity="right",
            full_cover_policy="replacement",
        )
        core = make_core_state("ABCD", generic_anchors=(anchor,))
        _, baseline = self.register(core)
        result = self.kernel.commit_story_edit_transaction(
            self.request(
                baseline,
                op_id="story-tx-00000014",
                start=2,
                end=2,
                expected="",
                replacement="XY",
            )
        )
        self.assertEqual("chaptera.commit-accepted.v1", result["protocol_version"])
        after = self.current_core()
        self.assertEqual((1, 5), (
            after.generic_anchors[0].start_scalar,
            after.generic_anchors[0].end_scalar,
        ))

    def test_unexpected_participant_failure_leaves_revision_pointer_unchanged(self):
        core = make_core_state("ABC")
        _, baseline = self.register(core)

        def exploding_executor(_project, _command):
            raise ValueError("induced participant failure")

        with self.assertRaisesRegex(ValueError, "induced participant failure"):
            self.kernel.commit_story_edit_transaction(
                self.request(
                    baseline,
                    op_id="story-tx-00000015",
                    start=1,
                    end=1,
                    expected="",
                    replacement="X",
                ),
                exploding_executor,
            )
        self.assertEqual(baseline.revision_id, self.kernel.current_revision(DOCUMENT_ID).revision_id)
        self.assertEqual(0, len(self.kernel.current_revision(DOCUMENT_ID).project["operations"]))

    def test_post_candidate_canonical_validation_failure_also_commits_nothing(self):
        core = make_core_state("ABC")
        _, baseline = self.register(core)

        def corrupt_executor(project, command):
            operation, resulting, consequences = execute_story_edit_transaction_v1(project, command)
            operation = copy.deepcopy(operation)
            operation["after_state_id"] = "sha256:" + "0" * 64
            return operation, resulting, consequences

        with self.assertRaisesRegex(ValueError, "after_state_id"):
            self.kernel.commit_story_edit_transaction(
                self.request(
                    baseline,
                    op_id="story-tx-00000016",
                    start=1,
                    end=1,
                    expected="",
                    replacement="X",
                ),
                corrupt_executor,
            )
        self.assertEqual(baseline.revision_id, self.kernel.current_revision(DOCUMENT_ID).revision_id)

    def test_retry_is_idempotent_and_reuses_preallocated_paragraph_identity(self):
        core = make_core_state("AB")
        _, baseline = self.register(core)
        req = self.request(
            baseline,
            op_id="story-tx-00000017",
            start=1,
            end=1,
            expected="",
            replacement="\r",
            paragraph_ids=(NEW1,),
        )
        first = self.kernel.commit_story_edit_transaction(copy.deepcopy(req))
        second = self.kernel.commit_story_edit_transaction(copy.deepcopy(req))
        self.assertEqual(first, second)
        after = self.current_core()
        self.assertEqual((P1, NEW1), tuple(p.paragraph_id for p in after.paragraph_state.paragraphs))
        self.assertEqual(1, len(self.kernel.current_revision(DOCUMENT_ID).project["operations"]))

    def test_semantic_rejection_retry_is_also_idempotent(self):
        core = make_core_state("ABC", unsupported=("hyperlink",))
        _, baseline = self.register(core)
        req = self.request(
            baseline,
            op_id="story-tx-00000018",
            start=1,
            end=1,
            expected="",
            replacement="X",
        )
        first = self.kernel.commit_story_edit_transaction(copy.deepcopy(req))
        second = self.kernel.commit_story_edit_transaction(copy.deepcopy(req))
        self.assertEqual(first, second)
        self.assertEqual("anchored_semantics_unsupported", first["code"])
        self.assertEqual(baseline.revision_id, self.kernel.current_revision(DOCUMENT_ID).revision_id)

    def test_inverse_redo_and_reopen_are_exact_canonical_states(self):
        core = make_core_state(
            "ABC",
            overrides=(TextFormatOverrideRunV1(0, 3, "bold", True),),
        )
        before_dict = story_edit_core_state_to_dict(core)
        _, baseline = self.register(core)
        result = self.kernel.commit_story_edit_transaction(
            self.request(
                baseline,
                op_id="story-tx-00000019",
                start=1,
                end=2,
                expected="B",
                replacement="XY",
                typing={"italic": True},
            )
        )
        operation = result["canonical_operation"]
        self.assertEqual(before_dict, restore_story_edit_transaction_before_v1(operation))

        after = story_edit_core_state_from_dict(operation["after_state"])
        self.assertEqual(operation["after_state_id"], story_edit_core_state_id_v1(after))
        self.assertEqual(
            operation["after_state"],
            self.kernel.current_revision(DOCUMENT_ID).project["story_models"][STORY_ID],
        )

        # Save/reopen is plain canonical JSON; reparse must preserve the exact
        # state identity, not recompute identities from current text.
        reopened_raw = json.loads(json.dumps(operation["after_state"], ensure_ascii=False))
        reopened = story_edit_core_state_from_dict(reopened_raw)
        self.assertEqual(after, reopened)
        self.assertEqual(operation["after_state_id"], story_edit_core_state_id_v1(reopened))
        self.assertEqual("layout_unknown", operation["layout_status"])


if __name__ == "__main__":
    unittest.main()
