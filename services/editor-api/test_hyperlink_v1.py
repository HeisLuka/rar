#!/usr/bin/env python3
import copy
import unittest

from hyperlink_v1 import (
    HyperlinkAuthoringError,
    build_hyperlink_span_v1,
    execute_hyperlink_operation_v1,
    hyperlink_span_from_dict_v1,
    hyperlink_span_to_dict_v1,
    normalize_hyperlink_target_v1,
    rebase_hyperlinks_for_unambiguous_story_edit_v1,
    replay_hyperlink_operation_v1,
    undo_hyperlink_operation_v1,
)
from revision_store import RevisionKernel


DOC="doc:hyperlink"
SOURCE="h"*64
STORY="story:1"


def project(text="Hello link world"):
    return {
        "schema_version":"pub-editor-v0.6",
        "source_hash":SOURCE,
        "operations":[],
        "stories":{STORY:text},
        "hyperlinks":{},
    }


def request(base_revision,op_id,command):
    return {
        "protocol_version":"chaptera.hyperlink-intent.v1",
        "document_id":DOC,
        "source_hash":SOURCE,
        "base_revision_id":base_revision,
        "client_operation_id":op_id,
        "command":command,
    }


class HyperlinkAuthoringV1Tests(unittest.TestCase):
    def test_http_https_absolute_targets_preserve_raw_and_normalize_scheme_only(self):
        t=normalize_hyperlink_target_v1("HTTPS://Example.COM/A%2Fb?q=X#Frag")
        self.assertEqual("HTTPS://Example.COM/A%2Fb?q=X#Frag",t.raw_url)
        self.assertEqual("https://Example.COM/A%2Fb?q=X#Frag",t.normalized_url)
        for bad in ("mailto:a@example.com","/relative"," ftp://example.com ","https://"):
            with self.assertRaises(HyperlinkAuthoringError):
                normalize_hyperlink_target_v1(bad)

    def test_create_update_remove_roundtrip_exact_project_state(self):
        base=project()
        span=build_hyperlink_span_v1(
            span_id="link:1",story_id=STORY,start_scalar=6,end_scalar=10,
            raw_url="https://example.com/A",
        )
        create={"kind":"hyperlink_create","span":hyperlink_span_to_dict_v1(span)}
        op1,p1,_=execute_hyperlink_operation_v1(base,create)
        self.assertEqual(span,hyperlink_span_from_dict_v1(p1["hyperlinks"]["link:1"]))
        self.assertEqual(base,undo_hyperlink_operation_v1(p1,op1))
        self.assertEqual(p1,replay_hyperlink_operation_v1(base,op1))

        update={
            "kind":"hyperlink_update",
            "expected_before":hyperlink_span_to_dict_v1(span),
            "raw_url":"HTTP://example.org/new",
        }
        op2,p2,_=execute_hyperlink_operation_v1(p1,update)
        updated=hyperlink_span_from_dict_v1(p2["hyperlinks"]["link:1"])
        self.assertEqual("HTTP://example.org/new",updated.target.raw_url)
        self.assertEqual("http://example.org/new",updated.target.normalized_url)
        self.assertEqual(p1,undo_hyperlink_operation_v1(p2,op2))
        self.assertEqual(p2,replay_hyperlink_operation_v1(p1,op2))

        remove={
            "kind":"hyperlink_remove",
            "expected_before":hyperlink_span_to_dict_v1(updated),
        }
        op3,p3,_=execute_hyperlink_operation_v1(p2,remove)
        self.assertEqual({},p3["hyperlinks"])
        self.assertEqual(p2,undo_hyperlink_operation_v1(p3,op3))
        self.assertEqual(p3,replay_hyperlink_operation_v1(p2,op3))

    def test_ranges_are_nonempty_inside_existing_story(self):
        base=project("abc")
        for a,b in ((0,0),(2,1)):
            with self.assertRaises(HyperlinkAuthoringError):
                build_hyperlink_span_v1(
                    span_id="bad",story_id=STORY,start_scalar=a,end_scalar=b,
                    raw_url="https://example.com",
                )
        too_long=build_hyperlink_span_v1(
            span_id="long",story_id=STORY,start_scalar=0,end_scalar=4,
            raw_url="https://example.com",
        )
        with self.assertRaises(HyperlinkAuthoringError) as caught:
            execute_hyperlink_operation_v1(
                base,{"kind":"hyperlink_create","span":hyperlink_span_to_dict_v1(too_long)}
            )
        self.assertEqual("invalid_hyperlink_range",caught.exception.code)

    def test_overlap_rejected_but_adjacency_allowed(self):
        base=project("abcdef")
        one=build_hyperlink_span_v1(
            span_id="link:1",story_id=STORY,start_scalar=1,end_scalar=3,
            raw_url="https://one.example",
        )
        _,p1,_=execute_hyperlink_operation_v1(
            base,{"kind":"hyperlink_create","span":hyperlink_span_to_dict_v1(one)}
        )
        overlap=build_hyperlink_span_v1(
            span_id="link:2",story_id=STORY,start_scalar=2,end_scalar=4,
            raw_url="https://two.example",
        )
        with self.assertRaises(HyperlinkAuthoringError) as caught:
            execute_hyperlink_operation_v1(
                p1,{"kind":"hyperlink_create","span":hyperlink_span_to_dict_v1(overlap)}
            )
        self.assertEqual("hyperlink_overlap",caught.exception.code)
        adjacent=build_hyperlink_span_v1(
            span_id="link:3",story_id=STORY,start_scalar=3,end_scalar=5,
            raw_url="https://three.example",
        )
        _,p2,_=execute_hyperlink_operation_v1(
            p1,{"kind":"hyperlink_create","span":hyperlink_span_to_dict_v1(adjacent)}
        )
        self.assertEqual({"link:1","link:3"},set(p2["hyperlinks"]))

    def test_exact_before_precondition_protects_update_and_remove(self):
        base=project()
        span=build_hyperlink_span_v1(
            span_id="link:1",story_id=STORY,start_scalar=6,end_scalar=10,
            raw_url="https://example.com",
        )
        _,p1,_=execute_hyperlink_operation_v1(
            base,{"kind":"hyperlink_create","span":hyperlink_span_to_dict_v1(span)}
        )
        stale=build_hyperlink_span_v1(
            span_id="link:1",story_id=STORY,start_scalar=6,end_scalar=10,
            raw_url="https://stale.example",
        )
        for command in (
            {"kind":"hyperlink_update","expected_before":hyperlink_span_to_dict_v1(stale),"raw_url":"https://new.example"},
            {"kind":"hyperlink_remove","expected_before":hyperlink_span_to_dict_v1(stale)},
        ):
            with self.assertRaises(HyperlinkAuthoringError) as caught:
                execute_hyperlink_operation_v1(p1,command)
            self.assertEqual("hyperlink_precondition_failed",caught.exception.code)

    def test_source_projected_link_is_distinct_and_read_only_for_explicit_v1_edit(self):
        source_span=build_hyperlink_span_v1(
            span_id="source:1",story_id=STORY,start_scalar=0,end_scalar=5,
            raw_url="https://source.example",provenance="source_publisher",
        )
        base=project()
        base["hyperlinks"]={"source:1":hyperlink_span_to_dict_v1(source_span)}
        kernel=RevisionKernel()
        baseline=kernel.register_baseline(document_id=DOC,source_hash=SOURCE,project=base)
        result=kernel.commit_hyperlink(request(
            baseline.revision_id,"source-edit-0001",
            {
                "kind":"hyperlink_update",
                "expected_before":hyperlink_span_to_dict_v1(source_span),
                "raw_url":"https://changed.example",
            },
        ))
        self.assertEqual("chaptera.commit-rejected.v1",result["protocol_version"])
        self.assertEqual("source_hyperlink_read_only",result["code"])
        self.assertEqual(base,kernel.current_revision(DOC).project)

    def test_revision_kernel_persists_idempotent_create_update_remove(self):
        base=project()
        kernel=RevisionKernel()
        baseline=kernel.register_baseline(document_id=DOC,source_hash=SOURCE,project=base)
        span=build_hyperlink_span_v1(
            span_id="link:1",story_id=STORY,start_scalar=6,end_scalar=10,
            raw_url="https://example.com",
        )
        create_req=request(
            baseline.revision_id,"hyper-create-0001",
            {"kind":"hyperlink_create","span":hyperlink_span_to_dict_v1(span)},
        )
        first=kernel.commit_hyperlink(copy.deepcopy(create_req))
        again=kernel.commit_hyperlink(copy.deepcopy(create_req))
        self.assertEqual("chaptera.commit-accepted.v1",first["protocol_version"])
        self.assertEqual(first,again)
        self.assertIn("link:1",kernel.current_revision(DOC).project["hyperlinks"])
        self.assertEqual(1,len(kernel.current_revision(DOC).project["operations"]))

        current=kernel.current_revision(DOC)
        updated_before=hyperlink_span_from_dict_v1(current.project["hyperlinks"]["link:1"])
        update_req=request(
            current.revision_id,"hyper-update-0001",
            {
                "kind":"hyperlink_update",
                "expected_before":hyperlink_span_to_dict_v1(updated_before),
                "raw_url":"https://updated.example/path",
            },
        )
        upd=kernel.commit_hyperlink(update_req)
        self.assertEqual("chaptera.commit-accepted.v1",upd["protocol_version"])
        current=kernel.current_revision(DOC)
        updated=hyperlink_span_from_dict_v1(current.project["hyperlinks"]["link:1"])
        self.assertEqual("https://updated.example/path",updated.target.raw_url)

        remove_req=request(
            current.revision_id,"hyper-remove-0001",
            {"kind":"hyperlink_remove","expected_before":hyperlink_span_to_dict_v1(updated)},
        )
        rem=kernel.commit_hyperlink(remove_req)
        self.assertEqual("chaptera.commit-accepted.v1",rem["protocol_version"])
        self.assertEqual({},kernel.current_revision(DOC).project["hyperlinks"])
        self.assertEqual(3,len(kernel.current_revision(DOC).project["operations"]))

    def test_revision_kernel_stale_create_rejects_without_mutation(self):
        base=project()
        kernel=RevisionKernel()
        baseline=kernel.register_baseline(document_id=DOC,source_hash=SOURCE,project=base)
        a=build_hyperlink_span_v1(
            span_id="link:a",story_id=STORY,start_scalar=0,end_scalar=5,
            raw_url="https://a.example",
        )
        b=build_hyperlink_span_v1(
            span_id="link:b",story_id=STORY,start_scalar=11,end_scalar=16,
            raw_url="https://b.example",
        )
        accepted=kernel.commit_hyperlink(request(
            baseline.revision_id,"hyper-a-0001",
            {"kind":"hyperlink_create","span":hyperlink_span_to_dict_v1(a)},
        ))
        self.assertEqual("chaptera.commit-accepted.v1",accepted["protocol_version"])
        stale=kernel.commit_hyperlink(request(
            baseline.revision_id,"hyper-b-0001",
            {"kind":"hyperlink_create","span":hyperlink_span_to_dict_v1(b)},
        ))
        self.assertEqual("chaptera.commit-rejected.v1",stale["protocol_version"])
        self.assertEqual("stale_revision",stale["code"])
        self.assertEqual({"link:a"},set(kernel.current_revision(DOC).project["hyperlinks"]))

    def test_unambiguous_edit_strictly_before_rebases_through_shared_range_transform(self):
        span=build_hyperlink_span_v1(
            span_id="link:1",story_id=STORY,start_scalar=6,end_scalar=10,
            raw_url="https://example.com",
        )
        out=rebase_hyperlinks_for_unambiguous_story_edit_v1(
            spans=(span,),story_id=STORY,
            edit_start_scalar=0,edit_end_scalar=2,replacement_length=5,
        )
        self.assertEqual((9,13),(out[0].start_scalar,out[0].end_scalar))

    def test_unambiguous_edit_strictly_after_leaves_link_unchanged(self):
        span=build_hyperlink_span_v1(
            span_id="link:1",story_id=STORY,start_scalar=1,end_scalar=4,
            raw_url="https://example.com",
        )
        out=rebase_hyperlinks_for_unambiguous_story_edit_v1(
            spans=(span,),story_id=STORY,
            edit_start_scalar=5,edit_end_scalar=6,replacement_length=2,
        )
        self.assertEqual(span,out[0])

    def test_boundary_touch_interior_crossing_and_full_cover_all_fail_closed(self):
        span=build_hyperlink_span_v1(
            span_id="link:1",story_id=STORY,start_scalar=3,end_scalar=7,
            raw_url="https://example.com",
        )
        cases=(
            (3,3,1),(7,7,1),(4,5,1),(1,3,2),
            (7,9,1),(2,8,2),(2,5,1),(5,9,1),
        )
        for a,b,n in cases:
            with self.subTest(case=(a,b,n)):
                with self.assertRaises(HyperlinkAuthoringError) as caught:
                    rebase_hyperlinks_for_unambiguous_story_edit_v1(
                        spans=(span,),story_id=STORY,
                        edit_start_scalar=a,edit_end_scalar=b,replacement_length=n,
                    )
                self.assertEqual("hyperlink_edit_policy_required",caught.exception.code)


if __name__=="__main__":
    unittest.main()
