#!/usr/bin/env python3
import unittest

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
from text_find_snapshot_v1 import TextFindExtentV1, build_text_find_snapshot_v1
from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    BaseFormatRunV1,
    TextFormatOverrideRunV1,
    build_text_format_overlay_state_v1,
    effective_property_segments_v1,
)
from story_edit_domain_v1 import derive_story_edit_domain_v1


DOC="doc:find-replace"
HASH="b"*64
STORY="story:1"
P1="paragraph:1"
P2="paragraph:2"
NEW1="01900000-0000-7000-8000-000000000101"
NEW2="01900000-0000-7000-8000-000000000102"


def fmt(bold=False):
    return BaseCharacterFormatV1("font:resolved",12000,bold,False,"#000000")


def core(text, *, overrides=(), provenance="chaptera_created", unsupported=()):
    protected=provenance=="imported_mature_quill_terminal_cr"
    count=text.count("\r")-(1 if protected else 0)+1
    paragraphs=tuple(
        ParagraphV1(
            [P1,P2,"paragraph:3"][i],
            ParagraphPropertiesV1((("align","left"),)),
            "source_or_existing",
        )
        for i in range(count)
    )
    ps=build_story_paragraph_state_v1(
        story_id=STORY,
        story_text=text,
        paragraphs=paragraphs,
        protected_terminal_cr=protected,
    )
    fs=build_text_format_overlay_state_v1(
        story_id=STORY,
        base_revision_id="format:base",
        story_scalar_len=len(text),
        base_runs=() if not text else (BaseFormatRunV1(0,len(text),fmt()),),
        overrides=tuple(overrides),
    )
    return build_story_edit_core_state_v1(
        story_id=STORY,
        provenance=provenance,
        paragraph_state=ps,
        format_state=fs,
        unsupported_anchored_semantics=tuple(unsupported),
    )


def project(c):
    return {
        "schema_version":"pub-editor-v0.6",
        "source_hash":HASH,
        "operations":[],
        "stories":{STORY:c.paragraph_state.story_text},
        "story_models":{STORY:story_edit_core_state_to_dict(c)},
    }


def bold_at(c, scalar):
    return effective_property_segments_v1(
        state=c.format_state,
        prop="bold",
        start_scalar=scalar,
        end_scalar=scalar+1,
    )[0].value


class StoryFindReplaceV1Tests(unittest.TestCase):
    def setUp(self):
        self.kernel=RevisionKernel()

    def register(self,c):
        return self.kernel.register_baseline(
            document_id=DOC,
            source_hash=HASH,
            project=project(c),
        )

    def snapshot(self, baseline, c, query):
        text=c.paragraph_state.story_text
        d=derive_story_edit_domain_v1(
            story_id=STORY,
            story_text=text,
            provenance=c.provenance,
        )
        return build_text_find_snapshot_v1(
            revision_id=baseline.revision_id,
            story_id=STORY,
            story_text=text,
            domain=d,
            external_query=query,
            extent=TextFindExtentV1("full_editable_story"),
        )

    def request(self, baseline, snap, ordinals, replacement, *, ids=(), op="find-replace-0001"):
        return {
            "protocol_version":"chaptera.story-find-replace-intent.v1",
            "document_id":DOC,
            "source_hash":HASH,
            "base_revision_id":baseline.revision_id,
            "client_operation_id":op,
            "command":{
                "kind":"story_find_replace",
                "story_id":STORY,
                "base_story_revision_id":baseline.revision_id,
                "find_snapshot":snap.to_dict(),
                "selected_match_ordinals":list(ordinals),
                "external_replacement_text":replacement,
                "paragraph_ids_by_match":[
                    {"match_ordinal":ordinal,"paragraph_ids":list(match_ids)}
                    for ordinal,match_ids in ids
                ],
                "format_generation_id":"find-replace-format-generation",
            },
        }

    def current(self):
        raw=self.kernel.current_revision(DOC).project["story_models"][STORY]
        return story_edit_core_state_from_dict(raw)

    def test_replace_all_is_one_revision_and_does_not_recurse_into_inserted_query(self):
        c=core("aaa")
        baseline=self.register(c)
        snap=self.snapshot(baseline,c,"a")
        result=self.kernel.commit_story_find_replace(
            self.request(baseline,snap,(0,1,2),"aa")
        )
        self.assertEqual("chaptera.commit-accepted.v1",result["protocol_version"])
        self.assertEqual("aaaaaa",self.current().paragraph_state.story_text)
        current=self.kernel.current_revision(DOC)
        self.assertEqual(baseline.revision_id,current.parent_revision_id)
        self.assertEqual(1,len(current.project["operations"]))
        op=result["canonical_operation"]
        self.assertEqual([0,1,2],op["selected_match_ordinals"])
        self.assertTrue(op["snapshot_staled"])

    def test_selected_match_enumeration_is_normalized_by_base_coordinate(self):
        c=core("a x a")
        baseline=self.register(c)
        snap=self.snapshot(baseline,c,"a")
        result=self.kernel.commit_story_find_replace(
            self.request(baseline,snap,(1,0),"Z")
        )
        self.assertEqual("Z x Z",self.current().paragraph_state.story_text)
        self.assertEqual(
            [0,1],
            result["canonical_operation"]["selected_match_ordinals"],
        )

    def test_empty_replacement_deletes_all_selected_matches(self):
        c=core("ab ab")
        baseline=self.register(c)
        snap=self.snapshot(baseline,c,"ab")
        result=self.kernel.commit_story_find_replace(
            self.request(baseline,snap,(0,1),"")
        )
        self.assertEqual("chaptera.commit-accepted.v1",result["protocol_version"])
        self.assertEqual(" ",self.current().paragraph_state.story_text)

    def test_uniform_effective_format_is_frozen_per_match_from_base(self):
        c=core(
            "aa xx aa",
            overrides=(TextFormatOverrideRunV1(0,2,"bold",True),),
        )
        baseline=self.register(c)
        snap=self.snapshot(baseline,c,"aa")
        result=self.kernel.commit_story_find_replace(
            self.request(baseline,snap,(0,1),"Z")
        )
        self.assertEqual("chaptera.commit-accepted.v1",result["protocol_version"])
        after=self.current()
        self.assertTrue(bold_at(after,0))
        self.assertFalse(bold_at(after,5))

    def test_one_mixed_format_match_aborts_whole_command(self):
        c=core(
            "aa aa",
            overrides=(TextFormatOverrideRunV1(3,4,"bold",True),),
        )
        baseline=self.register(c)
        snap=self.snapshot(baseline,c,"aa")
        result=self.kernel.commit_story_find_replace(
            self.request(baseline,snap,(0,1),"Z")
        )
        self.assertEqual("mixed_format_match_unsupported",result["code"])
        self.assertEqual(baseline.revision_id,self.kernel.current_revision(DOC).revision_id)
        self.assertEqual("aa aa",self.current().paragraph_state.story_text)

    def test_paragraph_boundary_replace_uses_composite_lifecycle_and_preallocated_id(self):
        c=core("A\rB")
        baseline=self.register(c)
        snap=self.snapshot(baseline,c,"\n")
        result=self.kernel.commit_story_find_replace(
            self.request(baseline,snap,(0,),"\nX",ids=((0,(NEW1,)),))
        )
        self.assertEqual("chaptera.commit-accepted.v1",result["protocol_version"])
        after=self.current()
        self.assertEqual("A\rXB",after.paragraph_state.story_text)
        self.assertEqual(
            (P1,NEW1),
            tuple(p.paragraph_id for p in after.paragraph_state.paragraphs),
        )

    def test_protected_terminal_cr_is_never_in_snapshot_or_replaced(self):
        c=core("A\r",provenance="imported_mature_quill_terminal_cr")
        baseline=self.register(c)
        snap=self.snapshot(baseline,c,"\n")
        self.assertEqual((),snap.matches)

    def test_stale_revision_aborts_without_commit(self):
        c=core("abc abc")
        baseline=self.register(c)
        snap=self.snapshot(baseline,c,"abc")
        req=self.request(baseline,snap,(0,),"X")
        req["command"]["base_story_revision_id"]="different-revision"
        result=self.kernel.commit_story_find_replace(req)
        self.assertIn(result["code"],{"find_snapshot_stale","invalid_find_snapshot"})
        self.assertEqual(baseline.revision_id,self.kernel.current_revision(DOC).revision_id)

    def test_unsupported_feature_semantics_fail_closed(self):
        c=core("abc",unsupported=("hyperlink",))
        baseline=self.register(c)
        snap=self.snapshot(baseline,c,"abc")
        result=self.kernel.commit_story_find_replace(
            self.request(baseline,snap,(0,),"X")
        )
        self.assertEqual("anchored_semantics_unsupported",result["code"])
        self.assertEqual(baseline.revision_id,self.kernel.current_revision(DOC).revision_id)


if __name__=="__main__":
    unittest.main()
