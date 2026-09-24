#!/usr/bin/env python3
import copy
import unittest

from resize_nodes_v1 import (
    ResizeNodesV1Error,
    apply_resize_nodes_operation_state_v1,
    execute_resize_nodes_v1,
    normalize_resize_nodes_request_v1,
    validate_resize_nodes_request_v1,
)
from revision_store import RevisionKernel


DOC="resize-nodes-doc"
SOURCE="9"*64
PAGE="page:1"

def rect(x,y,w,h):
    return {"x":x,"y":y,"width":w,"height":h}

def shape(node_id,bounds,*,page=PAGE,provenance=None,transform=None,parent=None):
    return {
        "node_id":node_id,
        "kind":"shape",
        "shape_kind":"rectangle",
        "page_id":page,
        "parent_id":parent or page,
        "bounds":copy.deepcopy(bounds),
        "transform":copy.deepcopy(transform or {"kind":"identity"}),
        "paint":{
            "fill":{"visible":True,"color":{"r":1,"g":2,"b":3}},
            "stroke":{"visible":True,"color":{"r":4,"g":5,"b":6},"width_emu":12700},
            "provenance":{"kind":"author_created"},
        },
        "provenance":copy.deepcopy(provenance or {"kind":"author_created"}),
    }

def project():
    return {
        "schema_version":"pub-editor-v0.7",
        "source_hash":SOURCE,
        "operations":[],
        "pages":{PAGE:{"authoring_enabled":True}},
        "shapes":{
            "a":shape("a",rect(0,0,100,80)),
            "b":shape("b",rect(200,20,50,60)),
            "source":shape("source",rect(400,0,50,50),provenance={"kind":"source_backed"}),
        },
    }

def entries():
    return [
        {"node_id":"b","expected_before":rect(200,20,50,60),"after":rect(180,30,80,90)},
        {"node_id":"a","expected_before":rect(0,0,100,80),"after":rect(-20,-10,150,120)},
    ]

def request(base,op_id,rows=None):
    normalized=normalize_resize_nodes_request_v1({
        "protocol_version":"chaptera.resize-nodes-intent.v1",
        "document_id":DOC,
        "source_hash":SOURCE,
        "base_revision_id":base,
        "client_operation_id":op_id,
        "command":{"kind":"resize_nodes","page_id":PAGE,"entries":copy.deepcopy(rows or entries())},
    })
    return normalized


class ResizeNodesV1Tests(unittest.TestCase):
    def test_executor_preflights_then_mutates_all_simultaneously(self):
        p=project()
        req=request("unused","op:1")
        op,after,_=execute_resize_nodes_v1(p,req["command"])
        self.assertEqual(["a","b"],[e["node_id"] for e in op["entries"]])
        self.assertEqual(rect(-20,-10,150,120),after["shapes"]["a"]["bounds"])
        self.assertEqual(rect(180,30,80,90),after["shapes"]["b"]["bounds"])
        self.assertEqual([],p["operations"])
        self.assertEqual(1,len(after["operations"]))

    def test_enumeration_normalization_is_deterministic(self):
        first=request("r","same",entries())
        second=request("r","same",list(reversed(entries())))
        self.assertEqual(first,second)

    def test_one_stale_member_rejects_full_batch_without_partial_mutation(self):
        p=project()
        rows=entries()
        rows[0]["expected_before"]["x"]+=1
        req=request("unused","op:stale",rows)
        before=copy.deepcopy(p)
        with self.assertRaisesRegex(ResizeNodesV1Error,"stale"):
            execute_resize_nodes_v1(p,req["command"])
        self.assertEqual(before,p)

    def test_translation_only_batch_is_move_nodes_authority(self):
        rows=entries()
        for row in rows:
            before=row["expected_before"]
            row["after"]=rect(before["x"]+10,before["y"]+20,before["width"],before["height"])
        with self.assertRaisesRegex(ResizeNodesV1Error,"MoveNodes"):
            validate_resize_nodes_request_v1(request("r","op:move",rows))

    def test_invalid_target_rejects_whole_batch(self):
        p=project()
        rows=entries()
        rows[1]={"node_id":"source","expected_before":rect(400,0,50,50),"after":rect(400,0,60,60)}
        req=request("unused","op:source",rows)
        with self.assertRaisesRegex(ResizeNodesV1Error,"author-created"):
            execute_resize_nodes_v1(p,req["command"])

        p=project()
        p["shapes"]["b"]["transform"]={"kind":"rotate90"}
        req=request("unused","op:transform")
        with self.assertRaisesRegex(ResizeNodesV1Error,"identity-transform"):
            execute_resize_nodes_v1(p,req["command"])

    def test_duplicate_and_unsafe_bounds_fail_closed(self):
        rows=entries()
        rows[1]["node_id"]="b"
        with self.assertRaisesRegex(ResizeNodesV1Error,"unique"):
            validate_resize_nodes_request_v1(request("r","op:dup",rows))

        rows=entries()
        rows[1]["after"]["x"]=9_007_199_254_740_991
        with self.assertRaises(ResizeNodesV1Error):
            validate_resize_nodes_request_v1(request("r","op:overflow",rows))

    def test_undo_redo_replay_restore_exact_sets(self):
        p=project()
        req=request("unused","op:history")
        op,after,_=execute_resize_nodes_v1(p,req["command"])
        undone=apply_resize_nodes_operation_state_v1(after,op,state="before")
        self.assertEqual(p["shapes"]["a"]["bounds"],undone["shapes"]["a"]["bounds"])
        self.assertEqual(p["shapes"]["b"]["bounds"],undone["shapes"]["b"]["bounds"])
        redone=apply_resize_nodes_operation_state_v1(undone,op,state="after")
        self.assertEqual(after["shapes"]["a"]["bounds"],redone["shapes"]["a"]["bounds"])
        self.assertEqual(after["shapes"]["b"]["bounds"],redone["shapes"]["b"]["bounds"])

    def test_revision_kernel_commits_one_revision_and_reordered_retry_is_idempotent(self):
        p=project()
        kernel=RevisionKernel()
        base=kernel.register_baseline(document_id=DOC,source_hash=SOURCE,project=p)
        first=kernel.commit_resize_nodes(request(base.revision_id,"op:kernel",entries()))
        second=kernel.commit_resize_nodes(request(base.revision_id,"op:kernel",list(reversed(entries()))))
        self.assertEqual(first,second)
        self.assertEqual("chaptera.commit-accepted.v1",first["protocol_version"])
        self.assertEqual(1,len(kernel.current_revision(DOC).project["operations"]))
        self.assertEqual(["a","b"],[e["node_id"] for e in first["canonical_operation"]["entries"]])

    def test_stale_base_rejects_before_executor_and_keeps_revision(self):
        p=project()
        kernel=RevisionKernel()
        base=kernel.register_baseline(document_id=DOC,source_hash=SOURCE,project=p)
        accepted=kernel.commit_resize_nodes(request(base.revision_id,"op:first"))
        stale=kernel.commit_resize_nodes(request(base.revision_id,"op:stale"))
        self.assertEqual("stale_revision",stale["code"])
        self.assertEqual(accepted["revision_id"],kernel.current_revision(DOC).revision_id)


if __name__=="__main__":
    unittest.main()
