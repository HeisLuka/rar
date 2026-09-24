#!/usr/bin/env python3
import copy
import unittest

from validate_authoring_picture_frame_receipt import validate_schema, validate_semantics

F0 = "sha256:" + "a" * 64
F1 = "sha256:" + "b" * 64
F2 = "sha256:" + "c" * 64
C0 = "sha256:" + "d" * 64
C1 = "sha256:" + "e" * 64
A0 = "ab_" + "1" * 32
A1 = "ab_" + "2" * 32


def state(frame, asset, crop):
    return {
        "frame_state_hash": frame,
        "asset_binding_id": asset,
        "crop_state_hash": crop,
        "crop_authority": "native_crop_v1",
    }


def valid_receipt():
    baseline = state(F0, A0, C0)
    cropped = state(F0, A0, C1)
    replaced = state(F0, A1, C1)
    moved = state(F1, A1, C1)
    resized = state(F2, A1, C1)
    return {
        "receipt_version": "chaptera.authoring-picture-frame-receipt.v1",
        "producer": {
            "implementation": "chaptera-private-picture-authoring",
            "commit_or_build": "deadbeef",
            "core_integration": True,
        },
        "fixture_kind": "synthetic_picture",
        "target_gate": {
            "mature_0x2c": True,
            "embedded_png_or_jpeg": True,
            "direct_page_owned": True,
            "identity_transform": True,
            "explicit_native_crop": True,
            "crop_semantics_authoritative": True,
            "ambiguous_state": False,
        },
        "states": {
            "baseline": copy.deepcopy(baseline),
            "after_crop": copy.deepcopy(cropped),
            "crop_undo": copy.deepcopy(baseline),
            "crop_redo": copy.deepcopy(cropped),
            "after_replace": copy.deepcopy(replaced),
            "after_move": copy.deepcopy(moved),
            "after_resize": copy.deepcopy(resized),
            "replay": copy.deepcopy(resized),
        },
        "negative_probes": {
            "same_crop_noop_rejected": True,
            "stale_crop_rejected": True,
            "grouped_picture_rejected": True,
            "transformed_picture_rejected": True,
            "ambiguous_crop_rejected": True,
            "same_asset_replace_rejected": True,
        },
        "output_probe": {
            "idml": "preserved",
            "odg": "explicit_loss",
            "fixed_pdf": "blocked",
            "silent_uncropped_source_fallback": False,
            "silent_old_asset_fallback": False,
        },
        "privacy": {
            "source_pub_unchanged": True,
            "source_write_count": 0,
            "fit_pan_synthesized": False,
            "raw_source_bytes_in_receipt": False,
            "raw_asset_bytes_in_receipt": False,
            "node_id_in_receipt": False,
            "asset_sha_in_receipt": False,
            "crop_values_in_receipt": False,
        },
    }


class AuthoringPictureFrameReceiptTests(unittest.TestCase):
    def test_valid_receipt_is_admitted(self):
        receipt = valid_receipt()
        validate_schema(receipt)
        validate_semantics(receipt)

    def test_set_crop_must_not_change_frame(self):
        receipt = valid_receipt()
        receipt["states"]["after_crop"]["frame_state_hash"] = F1
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_set_crop_must_not_change_asset(self):
        receipt = valid_receipt()
        receipt["states"]["after_crop"]["asset_binding_id"] = A1
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_crop_operation_must_change_crop(self):
        receipt = valid_receipt()
        receipt["states"]["after_crop"]["crop_state_hash"] = C0
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_replace_image_must_preserve_crop(self):
        receipt = valid_receipt()
        receipt["states"]["after_replace"]["crop_state_hash"] = C0
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_move_must_preserve_asset_crop(self):
        receipt = valid_receipt()
        receipt["states"]["after_move"]["asset_binding_id"] = A0
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_resize_must_preserve_asset_crop(self):
        receipt = valid_receipt()
        receipt["states"]["after_resize"]["crop_state_hash"] = C0
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_replay_must_restore_full_tuple(self):
        receipt = valid_receipt()
        receipt["states"]["replay"]["frame_state_hash"] = F1
        with self.assertRaises(AssertionError):
            validate_semantics(receipt)

    def test_silent_uncropped_fallback_is_forbidden(self):
        receipt = valid_receipt()
        receipt["output_probe"]["silent_uncropped_source_fallback"] = True
        with self.assertRaises(AssertionError):
            validate_schema(receipt)

    def test_fit_pan_synthesis_is_forbidden(self):
        receipt = valid_receipt()
        receipt["privacy"]["fit_pan_synthesized"] = True
        with self.assertRaises(AssertionError):
            validate_schema(receipt)


if __name__ == "__main__":
    unittest.main()
