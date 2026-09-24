#!/usr/bin/env python3
import copy
import unittest

from fit_group_contents_v1 import (
    PROJECT_SCHEMA_V1,
    FitGroupContentsError,
    FitGroupContentsNoOp,
    apply_fit_group_operation_state_v1,
    execute_fit_group_to_contents_v1,
    snapshot_group_v1,
    validate_fit_group_operation_v1,
    validate_fit_group_request_v1,
)
from revision_store import RevisionKernel


SOURCE_HASH="f"*64
DOC="fit-group-doc"


def rect(x,y,w,h):
    return {"x":x,"y":y,"width":w,"height":h}


def project():
    return {
        "schema_version": PROJECT_SCHEMA_V1,
        "source_hash": SOURCE_HASH,
        "operations": [],
        "groups": {
            "g0": {
                "page_id":"page:1",
                "parent_group_id":None,
                "provenance":"chaptera-authored-group-v1",
                "bounds":rect(100,100,200,100),
                "local_coordinate_space":rect(0,0,100,100),
                "children":["a","g1","b"],
            },
            "g1": {
                "page_id":"page:1",
                "parent_group_id":"g0",
                "provenance":"chaptera-authored-group-v1",
                "bounds":rect(50,20,20,40),
                "local_coordinate_space":rect(0,0,200,300),
                "children":["inner"],
            },
        },
        "nodes": {
            "a":{"page_id":"page:1","parent_group_id":"g0","kind":"rectangle","bounds":rect(10,10,20,20),"payload":{"paint":"red"}},
            "g1":{"page_id":"page:1","parent_group_id":"g0","kind":"group","bounds":rect(50,20,20,40),"payload":{"tag":"nested"}},
            "b":{"page_id":"page:1","parent_group_id":"g0","kind":"rectangle","bounds":rect(80,60,10,20),"payload":{"paint":"blue"}},
            "inner":{"page_id":"page:1","parent_group_id":"g1","kind":"rectangle","bounds":rect(20,30,50,60),"payload":{"paint":"green"}},
        },
    }


def request(base_revision, p):
    return {
        "protocol_version":"chaptera.fit-group-contents-intent.v1",
        "document_id":DOC,
        "source_hash":SOURCE_HASH,
        "base_revision_id":base_revision,
        "client_operation_id":"fit-op-0001",
        "command":{
            "kind":"fit_group_to_contents",
            "group_id":"g0",
            "expected_group":snapshot_group_v1(p,"g0"),
        },
    }


class FitGroupContentsV1Tests(unittest.TestCase):
    def test_tight_fit_changes_parameterization_not_pixels(self):
        p=project()
        before=copy.deepcopy(p)
        op,after,_=execute_fit_group_to_contents_v1(p,request("unused",p)["command"])
        self.assertEqual(rect(120,110,160,70),after["groups"]["g0"]["bounds"])
        self.assertEqual(rect(0,0,160,70),after["groups"]["g0"]["local_coordinate_space"])
        self.assertEqual(rect(0,0,40,20),after["nodes"]["a"]["bounds"])
        self.assertEqual(rect(80,10,40,40),after["nodes"]["g1"]["bounds"])
        self.assertEqual(rect(140,50,20,20),after["nodes"]["b"]["bounds"])
        self.assertEqual(before["nodes"]["a"]["payload"],after["nodes"]["a"]["payload"])
        self.assertEqual(before["nodes"]["b"]["payload"],after["nodes"]["b"]["payload"])
        self.assertEqual(before["groups"]["g1"]["local_coordinate_space"],after["groups"]["g1"]["local_coordinate_space"])
        self.assertEqual(before["groups"]["g1"]["children"],after["groups"]["g1"]["children"])
        self.assertEqual("fit_group_to_contents",op["kind"])

    def test_stale_expected_state_fails_before_mutation(self):
        p=project()
        cmd=request("unused",p)["command"]
        cmd["expected_group"]["bounds"]["x"]+=1
        before=copy.deepcopy(p)
        with self.assertRaisesRegex(FitGroupContentsError,"stale"):
            execute_fit_group_to_contents_v1(p,cmd)
        self.assertEqual(before,p)

    def test_already_tight_is_no_change(self):
        p=project()
        op,tight,_=execute_fit_group_to_contents_v1(p,request("unused",p)["command"])
        cmd={"kind":"fit_group_to_contents","group_id":"g0","expected_group":snapshot_group_v1(tight,"g0")}
        with self.assertRaises(FitGroupContentsNoOp):
            execute_fit_group_to_contents_v1(tight,cmd)

    def test_nested_group_must_stay_inside_parent_space(self):
        p=project()
        # Fit nested g1 itself: its current effective child is safely contained.
        cmd={"kind":"fit_group_to_contents","group_id":"g1","expected_group":snapshot_group_v1(p,"g1")}
        op,after,_=execute_fit_group_to_contents_v1(p,cmd)
        self.assertEqual("g1",op["group_id"])
        self.assertEqual(rect(52,24,5,8),after["groups"]["g1"]["bounds"])

    def test_exact_before_after_support_undo_redo_replay(self):
        p=project()
        op,after,_=execute_fit_group_to_contents_v1(p,request("unused",p)["command"])
        undone=apply_fit_group_operation_state_v1(after,op,state="before")
        self.assertEqual(snapshot_group_v1(p,"g0"),snapshot_group_v1(undone,"g0"))
        redone=apply_fit_group_operation_state_v1(undone,op,state="after")
        self.assertEqual(snapshot_group_v1(after,"g0"),snapshot_group_v1(redone,"g0"))

    def test_revision_kernel_commits_one_revision_and_noop_keeps_head(self):
        p=project()
        kernel=RevisionKernel()
        base=kernel.register_baseline(document_id=DOC,source_hash=SOURCE_HASH,project=p)
        req=request(base.revision_id,p)
        result=kernel.commit_fit_group_contents(req)
        self.assertEqual("chaptera.commit-accepted.v1",result["protocol_version"])
        self.assertNotEqual(base.revision_id,result["revision_id"])
        self.assertEqual(1,len(kernel.current_revision(DOC).project["operations"]))

        current=kernel.current_revision(DOC)
        noop_req=request(current.revision_id,current.project)
        noop_req["client_operation_id"]="fit-op-0002"
        noop=kernel.commit_fit_group_contents(noop_req)
        self.assertEqual("chaptera.fit-group-contents-noop.v1",noop["protocol_version"])
        self.assertEqual(current.revision_id,noop["revision_id"])
        self.assertEqual(current.state_id,noop["state_id"])
        self.assertEqual(current.revision_id,kernel.current_revision(DOC).revision_id)

    def test_revision_kernel_rejects_stale_and_schema_fence(self):
        p=project()
        kernel=RevisionKernel()
        base=kernel.register_baseline(document_id=DOC,source_hash=SOURCE_HASH,project=p)
        req=request(base.revision_id,p)
        req["command"]["expected_group"]["children"][0]["local_rect"]["x"]+=1
        rejected=kernel.commit_fit_group_contents(req)
        self.assertEqual("chaptera.commit-rejected.v1",rejected["protocol_version"])
        self.assertEqual(base.revision_id,kernel.current_revision(DOC).revision_id)

        bad=project(); bad["schema_version"]="pub-editor-v0.7"
        with self.assertRaisesRegex(FitGroupContentsError,"schema"):
            snapshot_group_v1(bad,"g0")

    def test_validators_reject_authoritative_smuggling(self):
        p=project()
        req=request("sha256:"+"1"*64,p)
        validate_fit_group_request_v1(req)
        bad=copy.deepcopy(req)
        bad["command"]["planned_bounds"]=rect(0,0,1,1)
        with self.assertRaises(FitGroupContentsError):
            validate_fit_group_request_v1(bad)
        op,_,_=execute_fit_group_to_contents_v1(p,req["command"])
        validate_fit_group_operation_v1(req["command"],op)


if __name__=="__main__":
    unittest.main()
