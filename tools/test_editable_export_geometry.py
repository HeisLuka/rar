#!/usr/bin/env python3
import json
import pathlib
import tempfile
import unittest
import zipfile

from verify_editable_export_geometry import (
    RectEmu,
    canonical_node_hex,
    expected_idml_anchors,
    format_emu_points,
    verify_export,
)

NODE_ID = "21111111-1111-4111-8111-111111111111"
RECT = RectEmu(x=127_000, y=254_000, width=1_270_000, height=635_000)


def idml_xml(rect=RECT, node_id=NODE_ID):
    anchors = expected_idml_anchors(rect)
    points = "\n".join(
        f'<PathPointType Anchor="{anchor}" LeftDirection="{anchor}" RightDirection="{anchor}"/>'
        for anchor in anchors
    )
    return f'''<?xml version="1.0" encoding="UTF-8"?>
<idPkg:Spread xmlns:idPkg="http://ns.adobe.com/AdobeInDesign/idml/1.0/packaging">
  <Spread Self="usp_test">
    <TextFrame Self="uf{canonical_node_hex(node_id)}" ParentStory="us_test">
      <Properties>
        <PathGeometry>
          <GeometryPathType PathOpen="false">
            <PathPointArray>
              {points}
            </PathPointArray>
          </GeometryPathType>
        </PathGeometry>
      </Properties>
    </TextFrame>
  </Spread>
</idPkg:Spread>
'''


def odg_xml(rect=RECT, node_id=NODE_ID):
    return f'''<?xml version="1.0" encoding="UTF-8"?>
<office:document-content
 xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
 xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
 xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0">
  <office:body>
    <draw:frame
      draw:name="Frame_{canonical_node_hex(node_id)}"
      svg:x="{format_emu_points(rect.x)}pt"
      svg:y="{format_emu_points(rect.y)}pt"
      svg:width="{format_emu_points(rect.width)}pt"
      svg:height="{format_emu_points(rect.height)}pt"/>
  </office:body>
</office:document-content>
'''


class EditableExportGeometryVerifierTests(unittest.TestCase):
    def test_point_formatter_matches_adapter_law(self):
        self.assertEqual("10", format_emu_points(127_000))
        self.assertEqual("-20", format_emu_points(-254_000))
        self.assertEqual("0.00007874015748", format_emu_points(1))

    def test_idml_binds_node_identity_and_all_four_corners(self):
        with tempfile.TemporaryDirectory() as tmp:
            artifact = pathlib.Path(tmp) / "proof.idml"
            with zipfile.ZipFile(artifact, "w") as zf:
                zf.writestr("Spreads/Spread_test.xml", idml_xml())
            receipt = verify_export(artifact, "idml", NODE_ID, RECT)

        self.assertTrue(receipt["geometry_matches_edit"])
        self.assertEqual("uf" + canonical_node_hex(NODE_ID), receipt["identity_binding"])
        self.assertEqual(expected_idml_anchors(RECT), receipt["observed"]["anchors_pt"])
        self.assertNotIn("artifact_path", receipt)
        self.assertRegex(receipt["artifact_sha256"], r"^[0-9a-f]{64}$")

    def test_odg_binds_node_identity_and_exact_rect(self):
        with tempfile.TemporaryDirectory() as tmp:
            artifact = pathlib.Path(tmp) / "proof.odg"
            with zipfile.ZipFile(artifact, "w") as zf:
                zf.writestr("content.xml", odg_xml())
            receipt = verify_export(artifact, "odg", NODE_ID, RECT)

        self.assertTrue(receipt["geometry_matches_edit"])
        self.assertEqual("Frame_" + canonical_node_hex(NODE_ID), receipt["identity_binding"])
        self.assertEqual("10pt", receipt["observed"]["x"])
        self.assertEqual("20pt", receipt["observed"]["y"])
        self.assertEqual("100pt", receipt["observed"]["width"])
        self.assertEqual("50pt", receipt["observed"]["height"])

    def test_idml_rejects_geometry_from_wrong_rect(self):
        with tempfile.TemporaryDirectory() as tmp:
            artifact = pathlib.Path(tmp) / "proof.idml"
            with zipfile.ZipFile(artifact, "w") as zf:
                zf.writestr("Spreads/Spread_test.xml", idml_xml())
            wrong = RectEmu(RECT.x + 1, RECT.y, RECT.width, RECT.height)
            with self.assertRaises(AssertionError):
                verify_export(artifact, "idml", NODE_ID, wrong)

    def test_odg_rejects_wrong_node_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            artifact = pathlib.Path(tmp) / "proof.odg"
            with zipfile.ZipFile(artifact, "w") as zf:
                zf.writestr("content.xml", odg_xml())
            with self.assertRaises(AssertionError):
                verify_export(
                    artifact,
                    "odg",
                    "31111111-1111-4111-8111-111111111111",
                    RECT,
                )

    def test_receipt_is_source_free_data_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            artifact = pathlib.Path(tmp) / "private-name.idml"
            with zipfile.ZipFile(artifact, "w") as zf:
                zf.writestr("Spreads/Spread_test.xml", idml_xml())
            receipt = verify_export(artifact, "idml", NODE_ID, RECT)
            encoded = json.dumps(receipt)

        self.assertNotIn(str(artifact), encoded)
        self.assertNotIn("private-name.idml", encoded)


if __name__ == "__main__":
    unittest.main()
