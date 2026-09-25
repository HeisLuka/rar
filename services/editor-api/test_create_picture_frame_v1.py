#!/usr/bin/env python3
import copy
import json
import struct
import unittest
import zlib

from create_picture_frame_v1 import (
    CreatePictureFrameError,
    PLACEMENT_KIND_V1,
    ZERO_CROP_V1,
    generic_free_resize_admitted_v1,
    make_create_picture_frame_executor_v1,
    move_node_admitted_v1,
)
from create_shape_v1 import new_uuid7_node_id_v1
from editor_project_asset_registry_v1 import (
    PROJECT_SCHEMA_V1,
    derive_asset_metadata_v1,
    required_editor_asset_ids_v1,
)
from revision_store import RevisionKernel, canonical_json, project_hash


DOCUMENT_ID = "doc:create-picture"
SOURCE_HASH = "ab" * 32
NODE_ID = "01890f47-0c00-7abc-8def-0123456789ab"
SECOND_NODE_ID = "01890f47-0c01-7abc-8def-0123456789ab"


def _chunk(kind, payload):
    crc = zlib.crc32(kind)
    crc = zlib.crc32(payload, crc) & 0xFFFFFFFF
    return (
        struct.pack(">I", len(payload))
        + kind
        + payload
        + struct.pack(">I", crc)
    )


def png(width, height):
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + _chunk(b"IHDR", ihdr) + _chunk(b"IEND", b"")


def exif_app1(orientation):
    tiff = (
        b"II"
        + (42).to_bytes(2, "little")
        + (8).to_bytes(4, "little")
        + (1).to_bytes(2, "little")
        + (0x0112).to_bytes(2, "little")
        + (3).to_bytes(2, "little")
        + (1).to_bytes(4, "little")
        + orientation.to_bytes(2, "little")
        + b"\x00\x00"
        + (0).to_bytes(4, "little")
    )
    payload = b"Exif\x00\x00" + tiff
    return b"\xff\xe1" + (len(payload) + 2).to_bytes(2, "big") + payload


def jpeg(width, height, orientation):
    sof_payload = (
        b"\x08"
        + height.to_bytes(2, "big")
        + width.to_bytes(2, "big")
        + b"\x01\x01\x11\x00"
    )
    sof = b"\xff\xc0" + (len(sof_payload) + 2).to_bytes(2, "big") + sof_payload
    return b"\xff\xd8" + exif_app1(orientation) + sof + b"\xff\xd9"


ASSET = png(3, 2)
META = derive_asset_metadata_v1(asset_bytes=ASSET, mime_type="image/png")
FRAME = {"x": -100, "y": 200, "width": 300, "height": 200}


def baseline_project(metadata=META):
    return {
        "schema_version": PROJECT_SCHEMA_V1,
        "source_hash": SOURCE_HASH,
        "immutable_source_blob_sha256": "cd" * 32,
        "operations": [],
        "pages": {
            "page:1": {
                "authoring_enabled": True,
                "children": ["source:existing"],
            }
        },
        "shapes": {},
        "text_frames": {},
        "picture_frames": {},
        "groups": {},
        "editor_asset_registry_schema": "chaptera.editor-asset-registry.v1",
        "editor_assets": [metadata.public_dict()],
        "durable_editor_asset_refs": [],
    }


class CreatePictureFrameV1Tests(unittest.TestCase):
    def setUp(self):
        self.project = baseline_project()
        self.kernel = RevisionKernel()
        self.baseline = self.kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=self.project,
        )
        self.executor = make_create_picture_frame_executor_v1(
            {META.asset_sha256: ASSET}
        )

    def request(
        self,
        op_id,
        *,
        node_id=NODE_ID,
        page_id="page:1",
        frame=None,
        asset_sha256=None,
        width_px=3,
        height_px=2,
        base=None,
    ):
        return {
            "protocol_version": "chaptera.create-picture-frame-intent.v1",
            "document_id": DOCUMENT_ID,
            "source_hash": SOURCE_HASH,
            "base_revision_id": base or self.baseline.revision_id,
            "client_operation_id": op_id,
            "command": {
                "kind": "create_picture_frame",
                "node_id": node_id,
                "page_id": page_id,
                "frame": copy.deepcopy(frame or FRAME),
                "asset_sha256": asset_sha256 or META.asset_sha256,
                "intrinsic_width_px": width_px,
                "intrinsic_height_px": height_px,
            },
        }

    def test_atomic_create_uses_full_asset_equal_aspect_state(self):
        result = self.kernel.commit_create_picture_frame(
            self.request("picture-create-0001"),
            self.executor,
        )
        project = self.kernel.current_revision(DOCUMENT_ID).project
        frame = project["picture_frames"][NODE_ID]
        self.assertEqual("image_frame", frame["kind"])
        self.assertEqual("page:1", frame["parent_id"])
        self.assertEqual(FRAME, frame["frame"])
        self.assertEqual(META.asset_sha256, frame["asset_sha256"])
        self.assertEqual(META.asset_sha256, frame["asset"])
        self.assertEqual({"width_px": 3, "height_px": 2, "orientation_class": "normal"}, frame["intrinsic"])
        self.assertEqual(ZERO_CROP_V1, frame["crop"])
        self.assertEqual({"kind": "identity"}, frame["transform"])
        self.assertTrue(frame["visible"])
        self.assertEqual(1000, frame["opacity_milli"])
        self.assertEqual({"kind": PLACEMENT_KIND_V1}, frame["placement"])
        self.assertEqual({"kind": "author_created"}, frame["provenance"])
        self.assertNotIn("fit", json.dumps(frame).lower())
        self.assertNotIn("pan", json.dumps(frame).lower())
        self.assertEqual(["source:existing"], project["pages"]["page:1"]["children"])
        self.assertEqual("create_picture_frame", result["canonical_operation"]["kind"])
        self.assertEqual((META.asset_sha256,), required_editor_asset_ids_v1(project))

    def test_wrong_ratio_rejected_before_revision_mutation(self):
        with self.assertRaisesRegex(CreatePictureFrameError, "ratio"):
            self.kernel.commit_create_picture_frame(
                self.request(
                    "picture-create-bad-ratio",
                    frame={"x": 0, "y": 0, "width": 301, "height": 200},
                ),
                self.executor,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_missing_or_tampered_exact_bytes_rejected_before_mutation(self):
        missing = make_create_picture_frame_executor_v1({})
        with self.assertRaisesRegex(CreatePictureFrameError, "bytes are missing"):
            self.kernel.commit_create_picture_frame(
                self.request("picture-create-missing-bytes"),
                missing,
            )

        tampered = make_create_picture_frame_executor_v1(
            {META.asset_sha256: ASSET + b"x"}
        )
        with self.assertRaises(CreatePictureFrameError):
            self.kernel.commit_create_picture_frame(
                self.request("picture-create-tampered-bytes"),
                tampered,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_supplied_intrinsic_dimensions_must_match_exact_asset(self):
        with self.assertRaisesRegex(CreatePictureFrameError, "intrinsic dimensions"):
            self.kernel.commit_create_picture_frame(
                self.request(
                    "picture-create-wrong-intrinsic",
                    frame={"x": 0, "y": 0, "width": 400, "height": 200},
                    width_px=4,
                    height_px=2,
                ),
                self.executor,
            )

    def test_non_normal_orientation_is_not_admitted(self):
        data = jpeg(3, 2, 6)
        metadata = derive_asset_metadata_v1(
            asset_bytes=data,
            mime_type="image/jpeg",
        )
        project = baseline_project(metadata)
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id="doc:orientation",
            source_hash=SOURCE_HASH,
            project=project,
        )
        request = self.request(
            "picture-create-orientation",
            asset_sha256=metadata.asset_sha256,
            base=baseline.revision_id,
        )
        request["document_id"] = "doc:orientation"
        executor = make_create_picture_frame_executor_v1(
            {metadata.asset_sha256: data}
        )
        with self.assertRaisesRegex(CreatePictureFrameError, "normal image orientation"):
            kernel.commit_create_picture_frame(request, executor)
        self.assertEqual(
            baseline.revision_id,
            kernel.current_revision("doc:orientation").revision_id,
        )

    def test_page_and_node_identity_gates_fail_closed(self):
        with self.assertRaisesRegex(CreatePictureFrameError, "invalid_create_picture_frame_page"):
            self.kernel.commit_create_picture_frame(
                self.request("picture-create-missing-page", page_id="page:missing"),
                self.executor,
            )

        collision = baseline_project()
        collision["shapes"][NODE_ID] = {"kind": "shape"}
        kernel = RevisionKernel()
        baseline = kernel.register_baseline(
            document_id="doc:collision",
            source_hash=SOURCE_HASH,
            project=collision,
        )
        request = self.request("picture-create-collision", base=baseline.revision_id)
        request["document_id"] = "doc:collision"
        with self.assertRaisesRegex(CreatePictureFrameError, "node_id_collision"):
            kernel.commit_create_picture_frame(request, self.executor)

    def test_intent_cannot_supply_crop_fit_parent_transform_or_provenance(self):
        for key, value in (
            ("crop", copy.deepcopy(ZERO_CROP_V1)),
            ("fit_mode", "fill"),
            ("parent_id", "page:forged"),
            ("transform", {"kind": "rotate"}),
            ("provenance", {"kind": "source_backed"}),
        ):
            req = self.request(f"picture-create-extra-{key}")
            req["command"][key] = value
            with self.subTest(key=key):
                with self.assertRaisesRegex(CreatePictureFrameError, "non-intent"):
                    self.kernel.commit_create_picture_frame(req, self.executor)

    def test_executor_cannot_forge_canonical_placement(self):
        def forged(base_project, command):
            op, project, consequences = self.executor(base_project, command)
            op["placement"] = {"kind": "publisher-fill"}
            return op, project, consequences

        with self.assertRaisesRegex(ValueError, "non-canonical"):
            self.kernel.commit_create_picture_frame(
                self.request("picture-create-forged-op"),
                forged,
            )
        self.assertEqual(
            self.baseline.revision_id,
            self.kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_exact_retry_and_stale_revision_follow_revision_kernel_law(self):
        req = self.request("picture-create-idempotent")
        first = self.kernel.commit_create_picture_frame(
            copy.deepcopy(req),
            self.executor,
        )
        second = self.kernel.commit_create_picture_frame(
            copy.deepcopy(req),
            self.executor,
        )
        self.assertEqual(first, second)

        stale = self.kernel.commit_create_picture_frame(
            self.request(
                "picture-create-stale",
                node_id=SECOND_NODE_ID,
                base=self.baseline.revision_id,
            ),
            self.executor,
        )
        self.assertEqual("stale_revision", stale["code"])
        self.assertNotIn(
            SECOND_NODE_ID,
            self.kernel.current_revision(DOCUMENT_ID).project["picture_frames"],
        )

    def test_undo_redo_replay_save_reopen_are_exact_and_asset_metadata_survives_undo(self):
        accepted = self.kernel.commit_create_picture_frame(
            self.request("picture-create-history"),
            self.executor,
        )
        accepted_project = copy.deepcopy(
            self.kernel.current_revision(DOCUMENT_ID).project
        )

        def history(_base_project, kind):
            if kind == "undo":
                return copy.deepcopy(self.project), [
                    {"key": "history.undo", "state": "supported", "note": None}
                ]
            if kind == "redo":
                return copy.deepcopy(accepted_project), [
                    {"key": "history.redo", "state": "supported", "note": None}
                ]
            raise ValueError("unsupported")

        undo = self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": accepted["revision_id"],
                "client_operation_id": "picture-create-undo",
                "command": {"kind": "undo"},
            },
            history,
        )
        current = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertNotIn(NODE_ID, current["picture_frames"])
        self.assertEqual([META.public_dict()], current["editor_assets"])

        self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": undo["revision_id"],
                "client_operation_id": "picture-create-redo",
                "command": {"kind": "redo"},
            },
            history,
        )
        self.assertEqual(
            accepted_project["picture_frames"][NODE_ID],
            self.kernel.current_revision(DOCUMENT_ID).project["picture_frames"][NODE_ID],
        )

        saved = json.loads(canonical_json(accepted_project).decode("utf-8"))
        reopened = RevisionKernel()
        record = reopened.register_baseline(
            document_id="doc:reopened-picture",
            source_hash=SOURCE_HASH,
            project=saved,
        )
        self.assertEqual(project_hash(accepted_project), record.project_hash)

        replay = RevisionKernel()
        replay_base = replay.register_baseline(
            document_id="doc:replay-picture",
            source_hash=SOURCE_HASH,
            project=self.project,
        )
        replay_req = self.request(
            "picture-create-replay",
            base=replay_base.revision_id,
        )
        replay_req["document_id"] = "doc:replay-picture"
        replay.commit_create_picture_frame(replay_req, self.executor)
        self.assertEqual(
            accepted_project["picture_frames"][NODE_ID],
            replay.current_revision("doc:replay-picture").project["picture_frames"][NODE_ID],
        )

    def test_generic_free_resize_is_fenced_but_move_is_admitted(self):
        self.kernel.commit_create_picture_frame(
            self.request("picture-create-capability"),
            self.executor,
        )
        entity = self.kernel.current_revision(DOCUMENT_ID).project["picture_frames"][NODE_ID]
        self.assertFalse(generic_free_resize_admitted_v1(entity))
        self.assertTrue(move_node_admitted_v1(entity))

    def test_uuid7_generator_remains_compatible(self):
        value = new_uuid7_node_id_v1(now_ms=1234, random_bits=123)
        self.assertEqual(7, __import__("uuid").UUID(value).version)


if __name__ == "__main__":
    unittest.main()
