#!/usr/bin/env python3
import json
import unittest

from multi_story_text_transaction_v1 import (
    derive_multi_story_text_receipts_v1,
    replay_multi_story_after_v1,
    restore_multi_story_before_v1,
)
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
from text_find_snapshot_v1 import TextFindExtentV1, build_text_find_snapshot_v1
from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    BaseFormatRunV1,
    build_text_format_overlay_state_v1,
)


DOC="doc:multi-story"
SOURCE="c"*64


def fmt():
    return BaseCharacterFormatV1(
        "font:resolved",12000,False,False,"#000000"
    )


def core(story_id,text,*,unsupported=()):
    ps=build_story_paragraph_state_v1(
        story_id=story_id,
        story_text=text,
        paragraphs=(
            ParagraphV1(
                f"paragraph:{story_id}",
                ParagraphPropertiesV1((("align","left"),)),
                "source_or_existing",
            ),
        ),
        protected_terminal_cr=False,
    )
    fs=build_text_format_overlay_state_v1(
        story_id=story_id,
        base_revision_id="format:base",
        story_scalar_len=len(text),
        base_runs=() if not text else (BaseFormatRunV1(0,len(text),fmt()),),
    )
    return build_story_edit_core_state_v1(
        story_id=story_id,
        provenance="chaptera_created",
        paragraph_state=ps,
        format_state=fs,
        unsupported_anchored_semantics=tuple(unsupported),
    )


def project(states):
    return {
        "schema_version":"pub-editor-v0.6",
        "source_hash":SOURCE,
        "operations":[],
        "stories":{
            story_id:state.paragraph_state.story_text
            for story_id,state in states.items()
        },
        "story_models":{
            story_id:story_edit_core_state_to_dict(state)
            for story_id,state in states.items()
        },
    }


def story_edit_command(story_id,start,end,expected,replacement):
    return {
        "kind":"story_edit_transaction",
        "story_id":story_id,
        "start_scalar":start,
        "end_scalar":end,
        "expected_before":expected,
        "replacement_text":replacement,
        "paragraph_inserted_ids":[],
        "paragraph_inserted_property_presets":[],
        "typing_format":None,
        "fragment_format_runs":[],
        "incoming_semantic_kinds":[],
    }


class MultiStoryTextTransactionV1Tests(unittest.TestCase):
    def setUp(self):
        self.states={
            "story:a":core("story:a","ABC"),
            "story:b":core("story:b","XYZ"),
        }
        self.kernel=RevisionKernel()
        self.baseline=self.kernel.register_baseline(
            document_id=DOC,
            source_hash=SOURCE,
            project=project(self.states),
        )

    def entry(self,story_id,command,kind="story_edit_transaction"):
        return {
            "story_id":story_id,
            "producer_kind":kind,
            "producer_command":command,
        }

    def request(self,entries,op="multi-story-0001",base=None):
        base=base or self.baseline.revision_id
        return {
            "protocol_version":"chaptera.multi-story-text-transaction-intent.v1",
            "document_id":DOC,
            "source_hash":SOURCE,
            "base_revision_id":base,
            "client_operation_id":op,
            "command":{
                "kind":"multi_story_text_transaction",
                "base_revision_id":base,
                "entries":list(entries),
            },
        }

    def current_text(self,story_id):
        return self.kernel.current_revision(DOC).project["stories"][story_id]

    def test_two_story_edits_commit_as_exactly_one_revision_and_one_operation(self):
        entries=(
            self.entry("story:b",story_edit_command("story:b",1,2,"Y","2")),
            self.entry("story:a",story_edit_command("story:a",1,2,"B","1")),
        )
        result=self.kernel.commit_multi_story_text_transaction(
            self.request(entries)
        )
        self.assertEqual("chaptera.commit-accepted.v1",result["protocol_version"])
        self.assertEqual("A1C",self.current_text("story:a"))
        self.assertEqual("X2Z",self.current_text("story:b"))
        current=self.kernel.current_revision(DOC)
        self.assertEqual(self.baseline.revision_id,current.parent_revision_id)
        self.assertEqual(1,len(current.project["operations"]))
        self.assertEqual(
            ["story:a","story:b"],
            result["canonical_operation"]["story_ids"],
        )

    def test_input_story_order_permutations_produce_identical_revision(self):
        entries_a=(
            self.entry("story:b",story_edit_command("story:b",0,1,"X","Q")),
            self.entry("story:a",story_edit_command("story:a",0,1,"A","P")),
        )
        result_a=self.kernel.commit_multi_story_text_transaction(
            self.request(entries_a,op="multi-story-order-a")
        )

        kernel2=RevisionKernel()
        baseline2=kernel2.register_baseline(
            document_id=DOC,
            source_hash=SOURCE,
            project=project(self.states),
        )
        entries_b=tuple(reversed(entries_a))
        req=self.request(entries_b,op="multi-story-order-b",base=baseline2.revision_id)
        result_b=kernel2.commit_multi_story_text_transaction(req)

        self.assertEqual(result_a["revision_id"],result_b["revision_id"])
        self.assertEqual(
            result_a["canonical_operation"],
            result_b["canonical_operation"],
        )

    def test_one_unsupported_story_aborts_all_without_partial_commit(self):
        states={
            "story:a":core("story:a","ABC"),
            "story:b":core("story:b","XYZ",unsupported=("hyperlink",)),
        }
        kernel=RevisionKernel()
        baseline=kernel.register_baseline(
            document_id=DOC,source_hash=SOURCE,project=project(states)
        )
        entries=(
            self.entry("story:a",story_edit_command("story:a",1,2,"B","1")),
            self.entry("story:b",story_edit_command("story:b",1,2,"Y","2")),
        )
        result=kernel.commit_multi_story_text_transaction(
            self.request(entries,op="multi-story-fail",base=baseline.revision_id)
        )
        self.assertEqual("anchored_semantics_unsupported",result["code"])
        current=kernel.current_revision(DOC)
        self.assertEqual(baseline.revision_id,current.revision_id)
        self.assertEqual("ABC",current.project["stories"]["story:a"])
        self.assertEqual("XYZ",current.project["stories"]["story:b"])
        self.assertEqual([],current.project["operations"])

    def test_mixed_story_edit_and_find_replace_producers_share_one_commit(self):
        domain=derive_story_edit_domain_v1(
            story_id="story:b",
            story_text="XYZ",
            provenance="chaptera_created",
        )
        snap=build_text_find_snapshot_v1(
            revision_id=self.baseline.revision_id,
            story_id="story:b",
            story_text="XYZ",
            domain=domain,
            external_query="Y",
            extent=TextFindExtentV1("full_editable_story"),
        )
        find_command={
            "kind":"story_find_replace",
            "story_id":"story:b",
            "base_story_revision_id":self.baseline.revision_id,
            "find_snapshot":snap.to_dict(),
            "selected_match_ordinals":[0],
            "external_replacement_text":"22",
            "paragraph_ids_by_match":[],
            "format_generation_id":"multi-find-format",
        }
        entries=(
            self.entry("story:a",story_edit_command("story:a",1,2,"B","1")),
            self.entry("story:b",find_command,"story_find_replace"),
        )
        result=self.kernel.commit_multi_story_text_transaction(
            self.request(entries,op="multi-story-mixed")
        )
        self.assertEqual("chaptera.commit-accepted.v1",result["protocol_version"])
        self.assertEqual("A1C",self.current_text("story:a"))
        self.assertEqual("X22Z",self.current_text("story:b"))
        self.assertEqual(1,len(self.kernel.current_revision(DOC).project["operations"]))

    def test_receipts_are_transient_and_inverse_is_derived_from_semantic_operation(self):
        entries=(
            self.entry("story:a",story_edit_command("story:a",1,2,"B","LONG")),
            self.entry("story:b",story_edit_command("story:b",1,2,"Y","")),
        )
        result=self.kernel.commit_multi_story_text_transaction(
            self.request(entries,op="multi-story-receipts")
        )
        op=result["canonical_operation"]
        consequence=next(
            item for item in result["consequences"]
            if item["key"]=="text.multi_story_reconciliation"
        )
        self.assertEqual(
            derive_multi_story_text_receipts_v1(op,direction="forward"),
            consequence["receipt_map"],
        )
        inverse=derive_multi_story_text_receipts_v1(op,direction="inverse")
        self.assertEqual({"story:a","story:b"},set(inverse))
        self.assertEqual(
            (1,5),
            (
                inverse["story:a"]["normalized_edits"][0]["base_start_scalar"],
                inverse["story:a"]["normalized_edits"][0]["base_end_scalar"],
            ),
        )
        persisted=json.dumps(
            self.kernel.current_revision(DOC).project["operations"],
            sort_keys=True,
        )
        self.assertNotIn("receipt_map",persisted)
        self.assertNotIn("text-edit-receipt",persisted)

    def test_exact_before_and_after_state_maps_support_undo_redo_replay(self):
        result=self.kernel.commit_multi_story_text_transaction(
            self.request((
                self.entry("story:a",story_edit_command("story:a",0,1,"A","Q")),
                self.entry("story:b",story_edit_command("story:b",2,3,"Z","R")),
            ),op="multi-story-state-maps")
        )
        op=result["canonical_operation"]
        before=restore_multi_story_before_v1(op)
        after=replay_multi_story_after_v1(op)
        self.assertEqual("ABC",before["story:a"]["paragraph_state"]["story_text"])
        self.assertEqual("XYZ",before["story:b"]["paragraph_state"]["story_text"])
        self.assertEqual("QBC",after["story:a"]["paragraph_state"]["story_text"])
        self.assertEqual("XYR",after["story:b"]["paragraph_state"]["story_text"])

    def test_duplicate_story_entry_rejects_before_execution(self):
        entry=self.entry("story:a",story_edit_command("story:a",0,1,"A","Q"))
        with self.assertRaises(ValueError):
            self.kernel.commit_multi_story_text_transaction(
                self.request((entry,entry),op="multi-story-duplicate")
            )
        self.assertEqual(self.baseline.revision_id,self.kernel.current_revision(DOC).revision_id)

    def test_find_replace_local_base_must_equal_common_base(self):
        domain=derive_story_edit_domain_v1(
            story_id="story:b",story_text="XYZ",provenance="chaptera_created"
        )
        snap=build_text_find_snapshot_v1(
            revision_id=self.baseline.revision_id,
            story_id="story:b",
            story_text="XYZ",
            domain=domain,
            external_query="Y",
            extent=TextFindExtentV1("full_editable_story"),
        )
        command={
            "kind":"story_find_replace",
            "story_id":"story:b",
            "base_story_revision_id":"wrong-revision",
            "find_snapshot":snap.to_dict(),
            "selected_match_ordinals":[0],
            "external_replacement_text":"Q",
            "paragraph_ids_by_match":[],
            "format_generation_id":"multi-find-format",
        }
        result=self.kernel.commit_multi_story_text_transaction(
            self.request((
                self.entry("story:b",command,"story_find_replace"),
            ),op="multi-story-wrong-local-base")
        )
        self.assertEqual("stale_revision",result["code"])
        self.assertEqual(self.baseline.revision_id,self.kernel.current_revision(DOC).revision_id)


if __name__=="__main__":
    unittest.main()
