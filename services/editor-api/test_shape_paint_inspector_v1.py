#!/usr/bin/env python3
import copy
import unittest

from revision_store import RevisionKernel
from shape_paint_inspector_v1 import (
    ShapePaintInspectorError,
    build_set_fill_request_v1,
    build_set_stroke_request_v1,
    inspect_shape_paint_v1,
    points_text_to_emu_v1,
)

DOCUMENT_ID="doc:paint-ui"
SOURCE_HASH="ab"*32
NODE_ID="shape:authored:1"
FILL={"visible":True,"color":{"r":1,"g":2,"b":3}}
STROKE={"visible":True,"color":{"r":4,"g":5,"b":6},"width_emu":12700}


def project():
    return {
        "schema_version":"pub-editor-v0.4",
        "source_hash":SOURCE_HASH,
        "operations":[],
        "shapes":{
            NODE_ID:{
                "node_id":NODE_ID,
                "kind":"shape",
                "shape_kind":"rectangle",
                "paint":{
                    "fill":copy.deepcopy(FILL),
                    "stroke":copy.deepcopy(STROKE),
                    "provenance":{"kind":"author_created"},
                },
            }
        },
    }


def executor(base, command):
    p=copy.deepcopy(base)
    axis="fill" if command["kind"]=="set_fill" else "stroke"
    current=p["shapes"][command["node_id"]]["paint"][axis]
    if current != command["expected_before"]:
        raise ValueError("stale paint")
    operation={
        "kind":command["kind"],
        "node_id":command["node_id"],
        "before":copy.deepcopy(current),
        "after":copy.deepcopy(command["after"]),
    }
    p["shapes"][command["node_id"]]["paint"][axis]=copy.deepcopy(command["after"])
    p["operations"].append(copy.deepcopy(operation))
    return operation,p,[{"key":f"shape.{axis}","state":"supported","note":None}]


class ShapePaintInspectorV1Tests(unittest.TestCase):
    def test_exact_point_conversion_never_uses_float(self):
        self.assertEqual(12700, points_text_to_emu_v1("1"))
        self.assertEqual(6350, points_text_to_emu_v1("0.5"))
        self.assertEqual(25400, points_text_to_emu_v1("2.000"))
        with self.assertRaisesRegex(ShapePaintInspectorError,"exactly representable"):
            points_text_to_emu_v1("0.00001")
        with self.assertRaises(ShapePaintInspectorError):
            points_text_to_emu_v1("nan")

    def test_author_created_literal_paint_is_editable(self):
        snap=inspect_shape_paint_v1(project()["shapes"][NODE_ID])
        self.assertTrue(snap.editable)
        self.assertEqual(FILL,snap.fill)
        self.assertEqual(STROKE,snap.stroke)

    def test_source_or_unresolved_paint_is_read_only(self):
        source=project()["shapes"][NODE_ID]
        source["paint"]["provenance"]={"kind":"source_backed","source_ref":{"x":1}}
        snap=inspect_shape_paint_v1(source)
        self.assertFalse(snap.editable)
        self.assertEqual("source_or_inherited_paint",snap.reason)
        unresolved=project()["shapes"][NODE_ID]
        unresolved["paint"].pop("fill")
        self.assertFalse(inspect_shape_paint_v1(unresolved).editable)

    def test_unchanged_input_emits_no_operation(self):
        snap=inspect_shape_paint_v1(project()["shapes"][NODE_ID])
        self.assertIsNone(build_set_fill_request_v1(
            snapshot=snap,visible=True,r=1,g=2,b=3,
            document_id=DOCUMENT_ID,source_hash=SOURCE_HASH,
            base_revision_id="r",client_operation_id="noop-fill",
        ))
        self.assertIsNone(build_set_stroke_request_v1(
            snapshot=snap,visible=True,r=4,g=5,b=6,width_points="1",
            document_id=DOCUMENT_ID,source_hash=SOURCE_HASH,
            base_revision_id="r",client_operation_id="noop-stroke",
        ))

    def test_fill_and_stroke_each_submit_exactly_one_canonical_operation(self):
        p=project()
        kernel=RevisionKernel()
        base=kernel.register_baseline(document_id=DOCUMENT_ID,source_hash=SOURCE_HASH,project=p)
        snap=inspect_shape_paint_v1(p["shapes"][NODE_ID])

        fill_req=build_set_fill_request_v1(
            snapshot=snap,visible=False,r=10,g=20,b=30,
            document_id=DOCUMENT_ID,source_hash=SOURCE_HASH,
            base_revision_id=base.revision_id,client_operation_id="paint-ui-fill-1",
        )
        fill_result=kernel.commit_shape_fill(fill_req,executor)
        self.assertEqual("set_fill",fill_result["canonical_operation"]["kind"])
        self.assertEqual(
            {"visible":False,"color":{"r":10,"g":20,"b":30}},
            kernel.current_revision(DOCUMENT_ID).project["shapes"][NODE_ID]["paint"]["fill"],
        )

        current=kernel.current_revision(DOCUMENT_ID)
        snap2=inspect_shape_paint_v1(current.project["shapes"][NODE_ID])
        stroke_req=build_set_stroke_request_v1(
            snapshot=snap2,visible=True,r=40,g=50,b=60,width_points="2",
            document_id=DOCUMENT_ID,source_hash=SOURCE_HASH,
            base_revision_id=current.revision_id,client_operation_id="paint-ui-stroke-1",
        )
        stroke_result=kernel.commit_shape_stroke(stroke_req,executor)
        self.assertEqual("set_stroke",stroke_result["canonical_operation"]["kind"])
        self.assertEqual(25400,kernel.current_revision(DOCUMENT_ID).project["shapes"][NODE_ID]["paint"]["stroke"]["width_emu"])

    def test_invalid_color_or_read_only_commit_fails_before_request(self):
        snap=inspect_shape_paint_v1(project()["shapes"][NODE_ID])
        with self.assertRaisesRegex(ShapePaintInspectorError,"sRGB"):
            build_set_fill_request_v1(
                snapshot=snap,visible=True,r=256,g=0,b=0,
                document_id=DOCUMENT_ID,source_hash=SOURCE_HASH,
                base_revision_id="r",client_operation_id="bad",
            )
        source=project()["shapes"][NODE_ID]
        source["paint"]["provenance"]={"kind":"source_backed"}
        readonly=inspect_shape_paint_v1(source)
        with self.assertRaises(ShapePaintInspectorError):
            build_set_fill_request_v1(
                snapshot=readonly,visible=True,r=0,g=0,b=0,
                document_id=DOCUMENT_ID,source_hash=SOURCE_HASH,
                base_revision_id="r",client_operation_id="ro",
            )


if __name__=="__main__":
    unittest.main()
