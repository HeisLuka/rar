#!/usr/bin/env python3
import copy
import unittest

from authoring_fragment_v1 import apply_paste_fragment_v1
from create_shape_v1 import new_uuid7_node_id_v1
from duplicate_rectangle_v1 import (
    DUPLICATE_OFFSET_EMU_V1,
    DUPLICATE_PLACEMENT_POLICY_V1,
    DuplicateRectangleV1Error,
    execute_duplicate_rectangle_v1,
    paste_command_from_duplicate_operation_v1,
)
from revision_store import RevisionKernel


DOC="duplicate-rectangle-doc"
SOURCE="7"*64
PAGE="page:1"
SOURCE_ID=new_uuid7_node_id_v1(now_ms=1_700_000_000_000,random_bits=1)
DUP1=new_uuid7_node_id_v1(now_ms=1_700_000_000_001,random_bits=2)
DUP2=new_uuid7_node_id_v1(now_ms=1_700_000_000_002,random_bits=3)

def source_shape(node_id=SOURCE_ID,x=100,y=200):
    return {
        "node_id":node_id,
        "kind":"shape",
        "shape_kind":"rectangle",
        "page_id":PAGE,
        "parent_id":PAGE,
        "bounds":{"x":x,"y":y,"width":300,"height":400},
        "transform":{"kind":"identity"},
        "paint":{
            "fill":{"visible":True,"color":{"r":10,"g":20,"b":30}},
            "stroke":{"visible":True,"color":{"r":40,"g":50,"b":60},"width_emu":12700},
            "provenance":{"kind":"author_created"},
        },
        "provenance":{"kind":"author_created"},
    }

def project():
    return {
        "schema_version":"pub-editor-v0.7",
        "source_hash":SOURCE,
        "operations":[],
        "pages":{PAGE:{"authoring_enabled":True}},
        "shapes":{SOURCE_ID:source_shape()},
    }

def request(base,op_id,source_id=SOURCE_ID,dest=DUP1):
    return {
        "protocol_version":"chaptera.duplicate-rectangle-intent.v1",
        "document_id":DOC,
        "source_hash":SOURCE,
        "base_revision_id":base,
        "client_operation_id":op_id,
        "command":{
            "kind":"duplicate_rectangle",
            "source_node_id":source_id,
            "destination_node_id":dest,
            "placement_policy":DUPLICATE_PLACEMENT_POLICY_V1,
        },
    }


class DuplicateRectangleV1Tests(unittest.TestCase):
    def test_duplicate_is_capture_then_canonical_paste_not_second_clone_engine(self):
        p=project()
        op,after,consequences=execute_duplicate_rectangle_v1(p,request("unused","op:1")["command"])
        self.assertEqual("paste_fragment",op["kind"])
        self.assertEqual(1,len(after["operations"]))
        self.assertEqual(op,after["operations"][0])
        self.assertEqual(DUP1,op["identity_map"]["destination_node_id"])
        self.assertEqual(SOURCE_ID,op["fragment"]["rectangle"]["source_provenance"]["source_node_id"])
        self.assertEqual(DUP1,consequences[-1]["note"])

    def test_paint_state_is_exact_clone_and_geometry_uses_versioned_offset(self):
        p=project()
        op,after,_=execute_duplicate_rectangle_v1(p,request("unused","op:2")["command"])
        src=p["shapes"][SOURCE_ID]
        dup=after["shapes"][DUP1]
        self.assertEqual(src["paint"],dup["paint"])
        self.assertEqual(src["bounds"]["width"],dup["bounds"]["width"])
        self.assertEqual(src["bounds"]["height"],dup["bounds"]["height"])
        self.assertEqual(src["bounds"]["x"]+DUPLICATE_OFFSET_EMU_V1,dup["bounds"]["x"])
        self.assertEqual(src["bounds"]["y"]+DUPLICATE_OFFSET_EMU_V1,dup["bounds"]["y"])
        self.assertEqual({"kind":"identity"},dup["transform"])

    def test_revision_kernel_commits_one_revision_and_selects_duplicate(self):
        p=project()
        kernel=RevisionKernel()
        base=kernel.register_baseline(document_id=DOC,source_hash=SOURCE,project=p)
        result=kernel.commit_duplicate_rectangle(request(base.revision_id,"op:kernel"))
        self.assertEqual("chaptera.commit-accepted.v1",result["protocol_version"])
        self.assertEqual("paste_fragment",result["canonical_operation"]["kind"])
        self.assertEqual(1,len(kernel.current_revision(DOC).project["operations"]))
        self.assertEqual(DUP1,result["consequences"][-1]["note"])

    def test_replay_from_canonical_paste_restores_same_id_state_placement(self):
        p=project()
        op,after,_=execute_duplicate_rectangle_v1(p,request("unused","op:replay")["command"])
        redo_op,redone,_=apply_paste_fragment_v1(
            p,
            paste_command_from_duplicate_operation_v1(op),
        )
        self.assertEqual(op,redo_op)
        self.assertEqual(after["shapes"][DUP1],redone["shapes"][DUP1])

    def test_repeated_duplicate_uses_distinct_persisted_id(self):
        p=project()
        _,after,_=execute_duplicate_rectangle_v1(p,request("unused","op:first",dest=DUP1)["command"])
        second_req=request("unused","op:second",source_id=DUP1,dest=DUP2)
        second_op,after2,consequences=execute_duplicate_rectangle_v1(after,second_req["command"])
        self.assertIn(DUP1,after2["shapes"])
        self.assertIn(DUP2,after2["shapes"])
        self.assertNotEqual(DUP1,DUP2)
        self.assertEqual(DUP2,second_op["identity_map"]["destination_node_id"])
        self.assertEqual(DUP2,consequences[-1]["note"])
        self.assertEqual(
            after["shapes"][DUP1]["bounds"]["x"]+DUPLICATE_OFFSET_EMU_V1,
            after2["shapes"][DUP2]["bounds"]["x"],
        )

    def test_source_backed_grouped_or_transformed_source_fails_closed(self):
        for mutate,pattern in (
            (lambda s:s.update({"provenance":{"kind":"source_backed"}}),"author-created"),
            (lambda s:s.update({"parent_id":"group:1"}),"direct page-owned"),
            (lambda s:s.update({"transform":{"kind":"rotate90"}}),"identity-transform"),
        ):
            p=project()
            mutate(p["shapes"][SOURCE_ID])
            with self.subTest(pattern=pattern):
                with self.assertRaisesRegex(DuplicateRectangleV1Error,pattern):
                    execute_duplicate_rectangle_v1(p,request("unused","op:bad")["command"])

    def test_destination_collision_and_invalid_uuid_fail_closed(self):
        p=project()
        p["shapes"][DUP1]=source_shape(DUP1,500,500)
        with self.assertRaisesRegex(DuplicateRectangleV1Error,"collision"):
            execute_duplicate_rectangle_v1(p,request("unused","op:collision")["command"])

        bad=request("unused","op:uuid")
        bad["command"]["destination_node_id"]="not-a-uuid"
        kernel=RevisionKernel()
        base=kernel.register_baseline(document_id=DOC,source_hash=SOURCE,project=project())
        bad["base_revision_id"]=base.revision_id
        with self.assertRaisesRegex(DuplicateRectangleV1Error,"UUIDv7"):
            kernel.commit_duplicate_rectangle(bad)

    def test_wrong_policy_fails_closed(self):
        p=project()
        bad=request("unused","op:policy")["command"]
        bad["placement_policy"]="publisher.guess"
        with self.assertRaisesRegex(DuplicateRectangleV1Error,"unsupported"):
            execute_duplicate_rectangle_v1(p,bad)


if __name__=="__main__":
    unittest.main()
