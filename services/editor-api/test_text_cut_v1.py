#!/usr/bin/env python3
import copy
import unittest

from paragraph_lifecycle_v1 import (
    ParagraphPropertiesV1,
    ParagraphV1,
    build_story_paragraph_state_v1,
)
from revision_store import RevisionKernel
from story_edit_domain_v1 import derive_story_edit_domain_v1
from story_edit_transaction_v1 import (
    build_story_edit_core_state_v1,
    story_edit_core_state_from_dict,
    story_edit_core_state_to_dict,
)
from text_cut_v1 import (
    ClipboardWriteResultV1,
    plan_text_cut_after_clipboard_v1,
    prepare_text_cut_v1,
    reconcile_text_cut_accepted_v1,
)
from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    BaseFormatRunV1,
    TextFormatOverrideRunV1,
    build_text_format_overlay_state_v1,
)
from text_range_fragment_v1 import UnsupportedSemanticSpanV1
from text_selection_state_v1 import build_text_selection_state_v1


DOC="doc:cut"
STORY="story:cut"
SOURCE_HASH="c"*64


def fmt():
    return BaseCharacterFormatV1("font:resolved",12000,False,False,"#000000")


def props(**values):
    return ParagraphPropertiesV1(tuple(sorted(values.items())))


def core(
    text,
    *,
    overrides=(),
    paragraph_props=None,
    unsupported=(),
    active_features=(),
    provenance="chaptera_created",
):
    protected=provenance=="imported_mature_quill_terminal_cr"
    boundary_count=text.count("\r")-(1 if protected else 0)
    if paragraph_props is None:
        paragraph_props=[ParagraphPropertiesV1(()) for _ in range(boundary_count+1)]
    paragraphs=tuple(
        ParagraphV1(f"p:{i}",paragraph_props[i],"source_or_existing")
        for i in range(boundary_count+1)
    )
    pstate=build_story_paragraph_state_v1(
        story_id=STORY,
        story_text=text,
        paragraphs=paragraphs,
        protected_terminal_cr=protected,
    )
    fstate=build_text_format_overlay_state_v1(
        story_id=STORY,
        base_revision_id="format:base",
        story_scalar_len=len(text),
        base_runs=() if not text else (BaseFormatRunV1(0,len(text),fmt()),),
        overrides=tuple(overrides),
    )
    return build_story_edit_core_state_v1(
        story_id=STORY,
        provenance=provenance,
        paragraph_state=pstate,
        format_state=fstate,
        unsupported_anchored_semantics=tuple(unsupported),
        active_paragraph_features=tuple(active_features),
    )


def selection(state,start,end,revision="rev:1",preferred=91):
    d=derive_story_edit_domain_v1(
        story_id=STORY,
        story_text=state.paragraph_state.story_text,
        provenance=state.provenance,
    )
    return build_text_selection_state_v1(
        domain=d,
        revision_id=revision,
        anchor_scalar=start,
        focus_scalar=end,
        preferred_inline_x_emu=preferred,
    )


def prepare(state,start,end,*,revision="rev:1",op="cut-op-00000001",**kwargs):
    return prepare_text_cut_v1(
        document_id=DOC,
        source_hash=SOURCE_HASH,
        base_revision_id=revision,
        client_operation_id=op,
        current_selection=selection(state,start,end,revision=revision),
        before_state=state,
        **kwargs,
    )


def confirmed():
    return ClipboardWriteResultV1(
        protocol_version="chaptera.clipboard-write-result.v1",
        status="confirmed_success",
        transport_receipt_id="clipboard:1",
    )


def project(state):
    return {
        "schema_version":"pub-editor-v0.6",
        "source_hash":SOURCE_HASH,
        "operations":[],
        "stories":{STORY:state.paragraph_state.story_text},
        "story_models":{STORY:story_edit_core_state_to_dict(state)},
    }


class TextCutV1Tests(unittest.TestCase):
    def test_prepare_captures_rich_fragment_and_plain_from_same_frozen_range(self):
        state=core(
            "A\rBCD",
            overrides=(TextFormatOverrideRunV1(2,5,"bold",True),),
        )
        p=prepare(state,1,5)
        self.assertEqual("ready",p.status)
        self.assertTrue(p.clipboard_write_required)
        self.assertEqual("\rBCD",p.capture.payload.fragment.text)
        self.assertEqual("\rBCD",p.capture.payload.plain_text.canonical_text)
        self.assertEqual("\nBCD",p.capture.payload.plain_text.logical_text)
        self.assertEqual(
            p.capture.payload.fragment.scalar_len,
            p.capture.payload.plain_text.canonical_scalar_len,
        )
        self.assertEqual(0,p.document_mutation_count)

    def test_empty_selection_is_noop_before_clipboard(self):
        state=core("ABC")
        p=prepare(state,1,1)
        self.assertEqual("no_op",p.status)
        self.assertFalse(p.clipboard_write_required)
        self.assertIsNone(p.capture)

    def test_hyperlink_overlap_fails_before_clipboard_and_delete(self):
        state=core("ABCDE",unsupported=("hyperlink",))
        p=prepare(
            state,1,4,
            unsupported_semantic_spans=(
                UnsupportedSemanticSpanV1("hyperlink",2,5),
            ),
        )
        self.assertEqual("clipboard_semantics_unsupported",p.status)
        self.assertFalse(p.clipboard_write_required)
        decision=plan_text_cut_after_clipboard_v1(
            prepared=p,clipboard_result=confirmed()
        )
        self.assertEqual("no_delete",decision.status)
        self.assertIsNone(decision.request)

    def test_missing_semantic_inventory_fails_before_clipboard(self):
        state=core("ABCDE",unsupported=("hyperlink",))
        p=prepare(state,0,2)
        self.assertEqual("clipboard_semantics_unsupported",p.status)
        self.assertFalse(p.clipboard_write_required)

    def test_paragraph_semantics_loss_is_not_destructively_downgraded(self):
        state=core(
            "ABC",
            paragraph_props=(props(align="center"),),
        )
        p=prepare(state,0,2)
        self.assertEqual("clipboard_semantics_unsupported",p.status)
        self.assertEqual("paragraph_semantics_loss",p.reason)
        self.assertFalse(p.clipboard_write_required)

    def test_non_story_or_composition_ownership_suppresses_without_clipboard(self):
        state=core("ABC")
        for owner in ("composition","modal","inspector","canvas"):
            p=prepare(state,0,2,input_owner=owner)
            self.assertEqual("suppressed",p.status,owner)
            self.assertFalse(p.clipboard_write_required,owner)
        p=prepare(state,0,2,composition_active=True)
        self.assertEqual("suppressed",p.status)
        self.assertFalse(p.clipboard_write_required)

    def test_every_nonconfirmed_clipboard_result_creates_zero_delete_request(self):
        state=core("ABC")
        p=prepare(state,0,2)
        for status in (
            "failed","denied","cancelled","timeout","unknown","unsupported"
        ):
            d=plan_text_cut_after_clipboard_v1(
                prepared=p,
                clipboard_result=ClipboardWriteResultV1(
                    protocol_version="chaptera.clipboard-write-result.v1",
                    status=status,
                    reason=status,
                ),
            )
            self.assertEqual("clipboard_not_confirmed",d.status,status)
            self.assertIsNone(d.request,status)
            self.assertEqual(0,d.document_mutation_count,status)
            self.assertEqual(
                status in {"timeout","unknown"},
                d.clipboard_external_state_unknown,
                status,
            )

    def test_only_confirmed_success_lowers_exact_frozen_delete_with_fixed_id(self):
        state=core("ABCDE")
        p=prepare(state,1,4,op="cut-fixed-000001")
        d=plan_text_cut_after_clipboard_v1(
            prepared=p,clipboard_result=confirmed()
        )
        self.assertEqual("delete_ready",d.status)
        self.assertEqual("cut-fixed-000001",d.client_operation_id)
        self.assertEqual("cut-fixed-000001",d.request["client_operation_id"])
        self.assertEqual("rev:1",d.request["base_revision_id"])
        self.assertEqual(
            {
                "kind":"story_edit_transaction",
                "story_id":STORY,
                "start_scalar":1,
                "end_scalar":4,
                "expected_before":"BCD",
                "replacement_text":"",
                "paragraph_inserted_ids":[],
                "paragraph_inserted_property_presets":[],
                "typing_format":None,
                "fragment_format_runs":[],
                "incoming_semantic_kinds":[],
            },
            d.request["command"],
        )
        again=plan_text_cut_after_clipboard_v1(
            prepared=p,clipboard_result=confirmed()
        )
        self.assertEqual(d.request,again.request)
        self.assertEqual("reconcile_same_operation_id",d.retry_policy)

    def test_confirmed_cut_commits_exactly_one_revision_and_retry_is_idempotent(self):
        state=core("ABCDE")
        kernel=RevisionKernel()
        baseline=kernel.register_baseline(
            document_id=DOC,source_hash=SOURCE_HASH,project=project(state)
        )
        p=prepare(
            state,1,4,revision=baseline.revision_id,op="cut-commit-000001"
        )
        d=plan_text_cut_after_clipboard_v1(
            prepared=p,clipboard_result=confirmed()
        )
        first=kernel.commit_story_edit_transaction(copy.deepcopy(d.request))
        second=kernel.commit_story_edit_transaction(copy.deepcopy(d.request))
        self.assertEqual("chaptera.commit-accepted.v1",first["protocol_version"])
        self.assertEqual(first,second)
        current=kernel.current_revision(DOC)
        after=story_edit_core_state_from_dict(current.project["story_models"][STORY])
        self.assertEqual("AE",after.paragraph_state.story_text)
        self.assertEqual(1,len(current.project["operations"]))
        self.assertEqual(baseline.revision_id,current.parent_revision_id)

    def test_stale_after_confirmed_clipboard_rejects_delete_and_keeps_intervening_doc(self):
        state=core("ABCDE")
        kernel=RevisionKernel()
        baseline=kernel.register_baseline(
            document_id=DOC,source_hash=SOURCE_HASH,project=project(state)
        )
        p=prepare(
            state,1,4,revision=baseline.revision_id,op="cut-stale-000001"
        )
        cut=plan_text_cut_after_clipboard_v1(
            prepared=p,clipboard_result=confirmed()
        )
        intervening={
            "protocol_version":"chaptera.story-edit-transaction-intent.v1",
            "document_id":DOC,
            "source_hash":SOURCE_HASH,
            "base_revision_id":baseline.revision_id,
            "client_operation_id":"intervene-000001",
            "command":{
                "kind":"story_edit_transaction",
                "story_id":STORY,
                "start_scalar":5,
                "end_scalar":5,
                "expected_before":"",
                "replacement_text":"Z",
                "paragraph_inserted_ids":[],
                "paragraph_inserted_property_presets":[],
                "typing_format":None,
                "fragment_format_runs":[],
                "incoming_semantic_kinds":[],
            },
        }
        accepted=kernel.commit_story_edit_transaction(intervening)
        self.assertEqual("chaptera.commit-accepted.v1",accepted["protocol_version"])
        rejected=kernel.commit_story_edit_transaction(copy.deepcopy(cut.request))
        self.assertEqual("chaptera.commit-rejected.v1",rejected["protocol_version"])
        self.assertEqual("stale_revision",rejected["code"])
        after=story_edit_core_state_from_dict(
            kernel.current_revision(DOC).project["story_models"][STORY]
        )
        self.assertEqual("ABCDEZ",after.paragraph_state.story_text)
        self.assertEqual(1,len(kernel.current_revision(DOC).project["operations"]))

    def test_accepted_cut_reconciliation_collapses_at_removed_start_and_clears_preferred_x(self):
        state=core("ABCDE")
        kernel=RevisionKernel()
        baseline=kernel.register_baseline(
            document_id=DOC,source_hash=SOURCE_HASH,project=project(state)
        )
        p=prepare(
            state,1,4,revision=baseline.revision_id,op="cut-reconcile-0001"
        )
        d=plan_text_cut_after_clipboard_v1(
            prepared=p,clipboard_result=confirmed()
        )
        accepted=kernel.commit_story_edit_transaction(copy.deepcopy(d.request))
        after=story_edit_core_state_from_dict(
            kernel.current_revision(DOC).project["story_models"][STORY]
        )
        resulting_domain=derive_story_edit_domain_v1(
            story_id=STORY,
            story_text=after.paragraph_state.story_text,
            provenance=after.provenance,
        )
        r=reconcile_text_cut_accepted_v1(
            capture=p.capture,
            request=d.request,
            canonical_operation=accepted["canonical_operation"],
            resulting_revision_id=accepted["revision_id"],
            resulting_domain=resulting_domain,
        )
        self.assertEqual((1,1),r.selection.normalized_range)
        self.assertEqual(accepted["revision_id"],r.selection.revision_id)
        self.assertIsNone(r.selection.preferred_inline_x_emu)
        self.assertTrue(r.typing_state_cleared)
        self.assertTrue(r.preferred_inline_x_cleared)
        self.assertEqual(1,r.authoring_revision_count)
        self.assertEqual(1,r.undo_history_entry_count)
        self.assertEqual("external_not_restored",r.clipboard_undo_policy)

    def test_cut_across_paragraph_boundary_uses_existing_lifecycle_atomically(self):
        state=core("A\rB")
        kernel=RevisionKernel()
        baseline=kernel.register_baseline(
            document_id=DOC,source_hash=SOURCE_HASH,project=project(state)
        )
        p=prepare(
            state,1,2,revision=baseline.revision_id,op="cut-paragraph-0001"
        )
        self.assertEqual("ready",p.status)
        d=plan_text_cut_after_clipboard_v1(
            prepared=p,clipboard_result=confirmed()
        )
        accepted=kernel.commit_story_edit_transaction(d.request)
        self.assertEqual("chaptera.commit-accepted.v1",accepted["protocol_version"])
        after=story_edit_core_state_from_dict(
            kernel.current_revision(DOC).project["story_models"][STORY]
        )
        self.assertEqual("AB",after.paragraph_state.story_text)
        self.assertEqual(1,len(after.paragraph_state.paragraphs))


if __name__=="__main__":
    unittest.main()
