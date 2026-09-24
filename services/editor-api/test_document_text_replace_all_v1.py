#!/usr/bin/env python3
import copy
import unittest

from document_text_find_v1 import (
    DocumentStorySearchInputV1,
    build_document_text_find_snapshot_v1,
)
from multi_story_text_transaction_v1 import restore_multi_story_before_v1
from paragraph_lifecycle_v1 import (
    ParagraphPropertiesV1,
    ParagraphV1,
    build_story_paragraph_state_v1,
)
from revision_store import RevisionKernel
from story_edit_transaction_v1 import (
    build_story_edit_core_state_v1,
    story_edit_core_state_from_dict,
    story_edit_core_state_to_dict,
)
from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    BaseFormatRunV1,
    build_text_format_overlay_state_v1,
)


DOC="doc:document-replace"
SOURCE="d"*64
NEW1="01900000-0000-7000-8000-000000000201"
NEW2="01900000-0000-7000-8000-000000000202"


def fmt():
    return BaseCharacterFormatV1(
        "font:resolved",12000,False,False,"#000000"
    )


def paragraph_count(text,provenance):
    count=text.count("\r")+1
    if provenance=="imported_mature_quill_terminal_cr":
        count-=1
    return count


def core(story_id,text,*,provenance="chaptera_created",unsupported=()):
    count=paragraph_count(text,provenance)
    ps=build_story_paragraph_state_v1(
        story_id=story_id,
        story_text=text,
        paragraphs=tuple(
            ParagraphV1(
                f"paragraph:{story_id}:{i}",
                ParagraphPropertiesV1((("align","left"),)),
                "source_or_existing",
            )
            for i in range(count)
        ),
        protected_terminal_cr=(
            provenance=="imported_mature_quill_terminal_cr"
        ),
    )
    fs=build_text_format_overlay_state_v1(
        story_id=story_id,
        base_revision_id="format:base",
        story_scalar_len=len(text),
        base_runs=() if not text else (BaseFormatRunV1(0,len(text),fmt()),),
    )
    return build_story_edit_core_state_v1(
        story_id=story_id,
        provenance=provenance,
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


def snapshot(revision_id,states,query):
    stories=tuple(
        DocumentStorySearchInputV1(
            story_id=story_id,
            story_text=state.paragraph_state.story_text,
            provenance=state.provenance,
        )
        for story_id,state in states.items()
    )
    return build_document_text_find_snapshot_v1(
        revision_id=revision_id,
        stories=stories,
        external_query=query,
    )


class DocumentTextReplaceAllV1Tests(unittest.TestCase):
    def register(self,states):
        kernel=RevisionKernel()
        baseline=kernel.register_baseline(
            document_id=DOC,
            source_hash=SOURCE,
            project=project(states),
        )
        return kernel,baseline

    def request(self,baseline,snap,replacement,*,ids=(),op="doc-replace-0001"):
        return {
            "protocol_version":"chaptera.document-text-replace-all-intent.v1",
            "document_id":DOC,
            "source_hash":SOURCE,
            "base_revision_id":baseline.revision_id,
            "client_operation_id":op,
            "command":{
                "kind":"document_text_replace_all",
                "base_revision_id":baseline.revision_id,
                "document_find_snapshot":snap.to_dict(),
                "external_replacement_text":replacement,
                "paragraph_ids_by_match":[
                    {
                        "story_id":story_id,
                        "match_ordinal":ordinal,
                        "paragraph_ids":list(paragraph_ids),
                    }
                    for story_id,ordinal,paragraph_ids in ids
                ],
            },
        }

    def test_zero_match_is_deterministic_noop_without_revision_or_undo(self):
        states={"story:a":core("story:a","ABC"),"story:b":core("story:b","XYZ")}
        kernel,baseline=self.register(states)
        snap=snapshot(baseline.revision_id,states,"needle")
        req=self.request(baseline,snap,"replacement",op="doc-replace-zero")
        first=kernel.commit_document_text_replace_all(req)
        second=kernel.commit_document_text_replace_all(req)
        self.assertEqual("chaptera.document-text-replace-all-noop.v1",first["protocol_version"])
        self.assertEqual(first,second)
        self.assertFalse(first["state_changed"])
        self.assertEqual(baseline.revision_id,first["revision_id"])
        self.assertEqual(baseline.revision_id,kernel.current_revision(DOC).revision_id)
        self.assertEqual([],kernel.current_revision(DOC).project["operations"])

    def test_matches_in_one_story_commit_one_document_operation(self):
        states={"story:a":core("story:a","foo foo"),"story:b":core("story:b","none")}
        kernel,baseline=self.register(states)
        snap=snapshot(baseline.revision_id,states,"foo")
        result=kernel.commit_document_text_replace_all(
            self.request(baseline,snap,"bar",op="doc-replace-one-story")
        )
        self.assertEqual("chaptera.commit-accepted.v1",result["protocol_version"])
        current=kernel.current_revision(DOC)
        self.assertEqual("bar bar",current.project["stories"]["story:a"])
        self.assertEqual("none",current.project["stories"]["story:b"])
        self.assertEqual(baseline.revision_id,current.parent_revision_id)
        self.assertEqual(1,len(current.project["operations"]))
        op=result["canonical_operation"]
        self.assertEqual(["story:a"],op["affected_story_ids"])
        self.assertEqual(2,op["total_match_count"])
        self.assertEqual(
            ["story:a"],
            op["multi_story_operation"]["story_ids"],
        )

    def test_matches_across_stories_are_one_revision_and_one_undo_unit(self):
        states={
            "story:z":core("story:z","x z x"),
            "story:a":core("story:a","x"),
            "story:m":core("story:m","none"),
        }
        kernel,baseline=self.register(states)
        snap=snapshot(baseline.revision_id,states,"x")
        result=kernel.commit_document_text_replace_all(
            self.request(baseline,snap,"Q",op="doc-replace-many")
        )
        current=kernel.current_revision(DOC)
        self.assertEqual("Q z Q",current.project["stories"]["story:z"])
        self.assertEqual("Q",current.project["stories"]["story:a"])
        self.assertEqual("none",current.project["stories"]["story:m"])
        self.assertEqual(1,len(current.project["operations"]))
        self.assertEqual(
            ["story:a","story:z"],
            result["canonical_operation"]["affected_story_ids"],
        )
        self.assertEqual(
            ["story:a","story:z"],
            result["canonical_operation"]["multi_story_operation"]["layout_invalidation_story_ids"],
        )

    def test_unsupported_search_domain_blocks_exhaustive_command_even_with_other_matches(self):
        states={
            "story:a":core("story:a","needle"),
            "story:b":core(
                "story:b","maybe\r",provenance="imported_unknown"
            ),
        }
        kernel,baseline=self.register(states)
        snap=snapshot(baseline.revision_id,states,"needle")
        self.assertFalse(snap.exhaustive_searchable)
        result=kernel.commit_document_text_replace_all(
            self.request(baseline,snap,"X",op="doc-replace-incomplete")
        )
        self.assertEqual("document_search_incomplete",result["code"])
        self.assertEqual(baseline.revision_id,kernel.current_revision(DOC).revision_id)
        self.assertEqual("needle",kernel.current_revision(DOC).project["stories"]["story:a"])

    def test_one_unsupported_local_match_aborts_all_stories(self):
        states={
            "story:a":core("story:a","foo"),
            "story:b":core("story:b","foo",unsupported=("hyperlink",)),
        }
        kernel,baseline=self.register(states)
        snap=snapshot(baseline.revision_id,states,"foo")
        result=kernel.commit_document_text_replace_all(
            self.request(baseline,snap,"bar",op="doc-replace-local-fail")
        )
        self.assertEqual("anchored_semantics_unsupported",result["code"])
        current=kernel.current_revision(DOC)
        self.assertEqual(baseline.revision_id,current.revision_id)
        self.assertEqual("foo",current.project["stories"]["story:a"])
        self.assertEqual("foo",current.project["stories"]["story:b"])
        self.assertEqual([],current.project["operations"])

    def test_replacement_containing_query_is_not_recursively_researched(self):
        states={
            "story:a":core("story:a","aa"),
            "story:b":core("story:b","a"),
        }
        kernel,baseline=self.register(states)
        snap=snapshot(baseline.revision_id,states,"a")
        result=kernel.commit_document_text_replace_all(
            self.request(baseline,snap,"aa",op="doc-replace-recursion")
        )
        self.assertEqual("chaptera.commit-accepted.v1",result["protocol_version"])
        self.assertEqual("aaaa",kernel.current_revision(DOC).project["stories"]["story:a"])
        self.assertEqual("aa",kernel.current_revision(DOC).project["stories"]["story:b"])
        self.assertEqual(3,result["canonical_operation"]["total_match_count"])

    def test_snapshot_tamper_is_stale_and_commits_nothing(self):
        states={"story:a":core("story:a","foo")}
        kernel,baseline=self.register(states)
        snap=snapshot(baseline.revision_id,states,"foo")
        req=self.request(baseline,snap,"bar",op="doc-replace-stale-snapshot")
        req=copy.deepcopy(req)
        req["command"]["document_find_snapshot"]["story_results"][0]["snapshot"]["matches"][0]["matched_text"]="tampered"
        result=kernel.commit_document_text_replace_all(req)
        self.assertEqual("document_find_snapshot_stale",result["code"])
        self.assertEqual(baseline.revision_id,kernel.current_revision(DOC).revision_id)

    def test_paragraph_boundary_replacement_requires_preallocated_ids_and_preserves_atomicity(self):
        states={"story:a":core("story:a","A\rB")}
        kernel,baseline=self.register(states)
        snap=snapshot(baseline.revision_id,states,"\n")

        rejected=kernel.commit_document_text_replace_all(
            self.request(baseline,snap,"\nX",op="doc-replace-no-ids")
        )
        self.assertEqual("invalid_paragraph_ids",rejected["code"])
        self.assertEqual(baseline.revision_id,kernel.current_revision(DOC).revision_id)

        accepted=kernel.commit_document_text_replace_all(
            self.request(
                baseline,
                snap,
                "\nX",
                ids=(("story:a",0,(NEW1,)),),
                op="doc-replace-with-ids",
            )
        )
        self.assertEqual("chaptera.commit-accepted.v1",accepted["protocol_version"])
        state=story_edit_core_state_from_dict(
            kernel.current_revision(DOC).project["story_models"]["story:a"]
        )
        self.assertEqual("A\rXB",state.paragraph_state.story_text)
        self.assertEqual(
            NEW1,
            state.paragraph_state.paragraphs[1].paragraph_id,
        )

    def test_nested_multi_story_operation_preserves_exact_inverse_states(self):
        states={
            "story:a":core("story:a","foo"),
            "story:b":core("story:b","foo"),
        }
        kernel,baseline=self.register(states)
        snap=snapshot(baseline.revision_id,states,"foo")
        result=kernel.commit_document_text_replace_all(
            self.request(baseline,snap,"bar",op="doc-replace-inverse")
        )
        before=restore_multi_story_before_v1(
            result["canonical_operation"]["multi_story_operation"]
        )
        self.assertEqual("foo",before["story:a"]["paragraph_state"]["story_text"])
        self.assertEqual("foo",before["story:b"]["paragraph_state"]["story_text"])
        self.assertTrue(result["canonical_operation"]["document_snapshot_staled"])


if __name__=="__main__":
    unittest.main()
