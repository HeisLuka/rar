#!/usr/bin/env python3
import copy
import hashlib
import struct
import unittest
import zlib

from editor_project_asset_registry_v1 import (
    LEGACY_PROJECT_SCHEMA_V1,
    PROJECT_SCHEMA_V1,
    EditorProjectAssetRegistryError,
    apply_project_with_assets_v1,
    derive_asset_metadata_v1,
    normalize_project_asset_registry_v1,
    required_editor_asset_ids_v1,
    serialize_project_asset_registry_v1,
)


def chunk(kind, payload):
    crc=zlib.crc32(kind)
    crc=zlib.crc32(payload,crc)&0xffffffff
    return struct.pack(">I",len(payload))+kind+payload+struct.pack(">I",crc)


def png(width,height,marker):
    ihdr=struct.pack(">IIBBBBB",width,height,8,2,0,0,0)
    # Intrinsic V1 only needs canonical PNG signature/IHDR; include IEND for a sane fixture.
    return b"\x89PNG\r\n\x1a\n"+chunk(b"IHDR",ihdr)+chunk(b"tEXt",marker)+chunk(b"IEND",b"")


A=png(10,20,b"A")
B=png(30,40,b"B")
C=png(50,60,b"C")
MA=derive_asset_metadata_v1(asset_bytes=A,mime_type="image/png")
MB=derive_asset_metadata_v1(asset_bytes=B,mime_type="image/png")
MC=derive_asset_metadata_v1(asset_bytes=C,mime_type="image/png")


def project():
    return {
        "schema_version":PROJECT_SCHEMA_V1,
        "source_hash":"a"*64,
        "operations":[
            {"kind":"create_picture_frame","node_id":"picture:1","asset_sha256":MA.asset_sha256},
            {"kind":"delete_node","node_id":"picture:1"},
            {"kind":"replace_image","node_id":"picture:2","asset_sha256":MB.asset_sha256},
        ],
        "durable_editor_asset_refs":[],
        "editor_assets":[],
        "imported_editor_assets":[
            MA.asset_sha256,MB.asset_sha256,MC.asset_sha256
        ],
        "final_graph":{"picture:2":{"asset_sha256":MB.asset_sha256}},
    }


class EditorProjectAssetRegistryV1Tests(unittest.TestCase):
    def test_reachability_comes_from_operation_log_not_final_graph(self):
        p=project()
        self.assertEqual(
            tuple(sorted((MA.asset_sha256,MB.asset_sha256))),
            required_editor_asset_ids_v1(p),
        )
        self.assertNotIn(MA.asset_sha256,str(p["final_graph"]))

    def test_serialization_keeps_required_and_drops_unreferenced_import(self):
        p=serialize_project_asset_registry_v1(
            project(),
            {MA.asset_sha256:MA,MB.asset_sha256:MB,MC.asset_sha256:MC},
        )
        self.assertEqual(
            [MA.asset_sha256,MB.asset_sha256],
            [row["asset_sha256"] for row in p["editor_assets"]],
        )
        self.assertNotIn("imported_editor_assets",p)
        self.assertNotIn(MC.asset_sha256,[row["asset_sha256"] for row in p["editor_assets"]])

    def test_create_then_delete_still_requires_creation_asset_for_replay(self):
        p=project()
        p["operations"]=p["operations"][:2]
        p["final_graph"]={}
        self.assertEqual((MA.asset_sha256,),required_editor_asset_ids_v1(p))

    def test_complete_byte_preflight_occurs_before_any_replay(self):
        p=serialize_project_asset_registry_v1(
            project(),{MA.asset_sha256:MA,MB.asset_sha256:MB}
        )
        calls=[]
        def apply(session,op,assets):
            calls.append(op["kind"])
            session["ops"].append(op["kind"])
            return session
        with self.assertRaisesRegex(EditorProjectAssetRegistryError,"bytes are missing"):
            apply_project_with_assets_v1(
                p,asset_bytes_by_sha={MA.asset_sha256:A},
                fresh_session={"ops":[]},apply_operation=apply,
            )
        self.assertEqual([],calls)

    def test_tampered_asset_aborts_before_replay(self):
        p=serialize_project_asset_registry_v1(
            project(),{MA.asset_sha256:MA,MB.asset_sha256:MB}
        )
        calls=[]
        with self.assertRaises(EditorProjectAssetRegistryError):
            apply_project_with_assets_v1(
                p,
                asset_bytes_by_sha={MA.asset_sha256:A,MB.asset_sha256:B+b"x"},
                fresh_session={"ops":[]},
                apply_operation=lambda s,o,a:(calls.append(o["kind"]) or s),
            )
        self.assertEqual([],calls)

    def test_intrinsic_metadata_is_revalidated(self):
        bad=copy.deepcopy(MA.public_dict())
        bad["intrinsic"]["width_px"]+=1
        p=project()
        p["operations"]=p["operations"][:1]
        p["editor_assets"]=[bad]
        with self.assertRaisesRegex(EditorProjectAssetRegistryError,"intrinsic"):
            apply_project_with_assets_v1(
                p,asset_bytes_by_sha={MA.asset_sha256:A},
                fresh_session={"ops":[]},
                apply_operation=lambda s,o,a:s,
            )

    def test_successful_replay_receives_only_validated_required_assets(self):
        p=serialize_project_asset_registry_v1(
            project(),{MA.asset_sha256:MA,MB.asset_sha256:MB,MC.asset_sha256:MC}
        )
        original={"ops":[]}
        def apply(session,op,assets):
            self.assertEqual({MA.asset_sha256,MB.asset_sha256},set(assets))
            session["ops"].append(op["kind"])
            return session
        result=apply_project_with_assets_v1(
            p,
            asset_bytes_by_sha={MA.asset_sha256:A,MB.asset_sha256:B,MC.asset_sha256:C},
            fresh_session=original,
            apply_operation=apply,
        )
        self.assertEqual([],original["ops"])
        self.assertEqual(["create_picture_frame","delete_node","replace_image"],result["ops"])

    def test_replay_failure_does_not_mutate_caller_session(self):
        p=serialize_project_asset_registry_v1(
            project(),{MA.asset_sha256:MA,MB.asset_sha256:MB}
        )
        original={"ops":[]}
        def apply(session,op,assets):
            session["ops"].append(op["kind"])
            if op["kind"]=="delete_node":
                raise RuntimeError("synthetic replay failure")
            return session
        with self.assertRaisesRegex(EditorProjectAssetRegistryError,"transactional"):
            apply_project_with_assets_v1(
                p,asset_bytes_by_sha={MA.asset_sha256:A,MB.asset_sha256:B},
                fresh_session=original,apply_operation=apply,
            )
        self.assertEqual([],original["ops"])

    def test_legacy_replacement_assets_migrate_one_to_one(self):
        current=serialize_project_asset_registry_v1(
            project(),{MA.asset_sha256:MA,MB.asset_sha256:MB}
        )
        legacy=copy.deepcopy(current)
        legacy["schema_version"]=LEGACY_PROJECT_SCHEMA_V1
        legacy["replacement_assets"]=legacy.pop("editor_assets")
        migrated=normalize_project_asset_registry_v1(legacy)
        self.assertEqual(PROJECT_SCHEMA_V1,migrated["schema_version"])
        self.assertEqual(current["editor_assets"],migrated["editor_assets"])
        self.assertEqual(current["operations"],migrated["operations"])

    def test_new_schema_never_silently_accepts_legacy_field(self):
        p=project()
        p["replacement_assets"]=[]
        with self.assertRaisesRegex(EditorProjectAssetRegistryError,"legacy"):
            normalize_project_asset_registry_v1(p)

    def test_unknown_asset_looking_field_fails_closed(self):
        p=project()
        p["operations"].append({
            "kind":"future_picture_op",
            "nested":{"asset_sha256":MC.asset_sha256},
        })
        with self.assertRaisesRegex(EditorProjectAssetRegistryError,"unaccounted"):
            required_editor_asset_ids_v1(p)

    def test_explicit_future_editor_asset_refs_are_reachable(self):
        p=project()
        p["operations"].append({
            "kind":"future_picture_op",
            "editor_asset_refs":[MC.asset_sha256],
        })
        self.assertIn(MC.asset_sha256,required_editor_asset_ids_v1(p))

    def test_wrong_mime_and_missing_metadata_fail_closed(self):
        with self.assertRaises(EditorProjectAssetRegistryError):
            derive_asset_metadata_v1(asset_bytes=A,mime_type="image/jpeg")
        with self.assertRaisesRegex(EditorProjectAssetRegistryError,"metadata is missing"):
            serialize_project_asset_registry_v1(
                project(),{MA.asset_sha256:MA}
            )


if __name__=="__main__":
    unittest.main()
