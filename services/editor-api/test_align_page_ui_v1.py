#!/usr/bin/env python3
import copy
import unittest

from align_page_ui_v1 import (
    build_align_page_move_request_v1,
    classify_align_page_target_v1,
)
from revision_store import RevisionKernel

DOCUMENT_ID="doc:align-page"
SOURCE_HASH="ab"*32
NODE_ID="shape:1"


def shape(x=10,y=20,w=100,h=50):
    return {
        "node_id":NODE_ID,
        "kind":"shape",
        "shape_kind":"rectangle",
        "page_id":"page:1",
        "parent_id":"page:1",
        "bounds":{"x":x,"y":y,"width":w,"height":h},
        "transform":{"kind":"identity"},
        "provenance":{"kind":"author_created"},
    }


def project():
    return {
        "schema_version":"pub-editor-v0.4",
        "source_hash":SOURCE_HASH,
        "operations":[],
        "nodes":{NODE_ID:shape()},
    }


def executor(base,command):
    p=copy.deepcopy(base)
    node=p["nodes"][command["node_id"]]
    before=copy.deepcopy(node["bounds"])
    after={
        "x":command["x_emu"],
        "y":command["y_emu"],
        "width":before["width"],
        "height":before["height"],
    }
    operation={"kind":"move_node","node_id":command["node_id"],"before":before,"after":after}
    node["bounds"]=copy.deepcopy(after)
    p["operations"].append(copy.deepcopy(operation))
    return operation,p,[{"key":"node.geometry.position","state":"supported","note":None}]


class AlignPageUiV1Tests(unittest.TestCase):
    def test_all_six_modes_use_page_planner_result(self):
        page={"x":0,"y":0,"width":1000,"height":800}
        expected={
            "align_left":(0,20),
            "align_right":(900,20),
            "align_top":(10,0),
            "align_bottom":(10,750),
            "align_horizontal_center":(450,20),
            "align_vertical_center":(10,375),
        }
        for mode,(x,y) in expected.items():
            with self.subTest(mode=mode):
                result=build_align_page_move_request_v1(
                    shape=shape(),page_bounds=page,mode=mode,
                    document_id=DOCUMENT_ID,source_hash=SOURCE_HASH,
                    base_revision_id="rev",client_operation_id=f"align-{mode}",
                )
                self.assertEqual("submit_move",result.status)
                self.assertEqual(x,result.move_request["command"]["x_emu"])
                self.assertEqual(y,result.move_request["command"]["y_emu"])
                self.assertEqual(NODE_ID,result.node_id)

    def test_no_change_emits_no_move(self):
        result=build_align_page_move_request_v1(
            shape=shape(x=0),page_bounds={"x":0,"y":0,"width":1000,"height":800},
            mode="align_left",document_id=DOCUMENT_ID,source_hash=SOURCE_HASH,
            base_revision_id="rev",client_operation_id="align-noop",
        )
        self.assertEqual("no_change",result.status)
        self.assertIsNone(result.move_request)

    def test_source_nested_and_transformed_targets_are_disabled(self):
        source=shape()
        source["provenance"]={"kind":"source_backed"}
        nested=shape()
        nested["parent_id"]="group:1"
        transformed=shape()
        transformed["transform"]={"kind":"rotation","deg":10}
        for candidate in (source,nested,transformed):
            with self.subTest(candidate=candidate):
                eligibility=classify_align_page_target_v1(candidate)
                self.assertFalse(eligibility.enabled)
                result=build_align_page_move_request_v1(
                    shape=candidate,page_bounds={"x":0,"y":0,"width":1000,"height":800},
                    mode="align_left",document_id=DOCUMENT_ID,source_hash=SOURCE_HASH,
                    base_revision_id="rev",client_operation_id="disabled",
                )
                self.assertEqual("disabled",result.status)
                self.assertIsNone(result.move_request)

    def test_commit_is_exactly_one_movenode_and_selection_identity_survives(self):
        p=project()
        kernel=RevisionKernel()
        baseline=kernel.register_baseline(document_id=DOCUMENT_ID,source_hash=SOURCE_HASH,project=p)
        result=build_align_page_move_request_v1(
            shape=p["nodes"][NODE_ID],
            page_bounds={"x":0,"y":0,"width":1000,"height":800},
            mode="align_right",
            document_id=DOCUMENT_ID,source_hash=SOURCE_HASH,
            base_revision_id=baseline.revision_id,client_operation_id="align-page-commit",
        )
        accepted=kernel.commit_move(result.move_request,executor)
        current=kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual(1,len(current["operations"]))
        self.assertEqual("move_node",accepted["canonical_operation"]["kind"])
        self.assertEqual(NODE_ID,accepted["canonical_operation"]["node_id"])
        self.assertEqual(NODE_ID,result.node_id)
        self.assertEqual({"x":900,"y":20,"width":100,"height":50},current["nodes"][NODE_ID]["bounds"])

    def test_undo_restores_prior_position(self):
        base_project=project()
        kernel=RevisionKernel()
        baseline=kernel.register_baseline(document_id=DOCUMENT_ID,source_hash=SOURCE_HASH,project=base_project)
        ui=build_align_page_move_request_v1(
            shape=base_project["nodes"][NODE_ID],
            page_bounds={"x":0,"y":0,"width":1000,"height":800},
            mode="align_bottom",document_id=DOCUMENT_ID,source_hash=SOURCE_HASH,
            base_revision_id=baseline.revision_id,client_operation_id="align-before-undo",
        )
        accepted=kernel.commit_move(ui.move_request,executor)
        edited=copy.deepcopy(kernel.current_revision(DOCUMENT_ID).project)

        def history(_base,kind):
            if kind=="undo": return copy.deepcopy(base_project),[]
            if kind=="redo": return copy.deepcopy(edited),[]
            raise ValueError(kind)

        kernel.commit_history_transition({
            "protocol_version":"chaptera.history-transition-intent.v1",
            "document_id":DOCUMENT_ID,
            "source_hash":SOURCE_HASH,
            "base_revision_id":accepted["revision_id"],
            "client_operation_id":"align-undo",
            "command":{"kind":"undo"},
        },history)
        self.assertEqual({"x":10,"y":20,"width":100,"height":50},kernel.current_revision(DOCUMENT_ID).project["nodes"][NODE_ID]["bounds"])


if __name__=="__main__":
    unittest.main()
