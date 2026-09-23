import copy
import json
import unittest
from pathlib import Path

from tools.validate_auth_wrap_native_receipt import validate_receipt

FIXTURE = Path("packages/research/auth-wrap/v1/fixtures/native-receipt.synthetic.json")


def native_closure(receipt):
    r = copy.deepcopy(receipt)
    r["receipt_kind"] = "native_observation"
    r["scope"]["closure_candidate"] = True
    r["environment"]["reset_provider_receipt_verified"] = True
    r["environment"]["cold_restore_pair_verified"] = True
    r["environment"]["environment_fingerprint_sha256"] = "a" * 64

    for family in ("family_a", "family_b"):
        for key in r[family]["capture"]:
            r[family]["capture"][key] = True

    r["family_a"]["post_save_0x47_outcome"] = "baseline_ref_restored"
    r["family_a"]["post_save_fopt_outcome"] = "unchanged"
    r["family_a"]["layout_outcome"] = "followed_geometry_fopt"

    r["family_b"]["post_save_0x47_outcome"] = "regenerated_for_new_wrap_state"
    r["family_b"]["post_save_fopt_outcome"] = "requested_change_persisted"
    r["family_b"]["layout_outcome"] = "followed_new_wrap_state"

    r["conclusion"] = {
        "authority_class": "inverse_cache",
        "both_conflict_families_executed": True,
        "save_reopen_evidence_complete": True,
        "needs_additional_native_discriminator": False,
    }
    return r


class AuthWrapNativeReceiptTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.synthetic = json.loads(FIXTURE.read_text(encoding="utf-8"))

    def test_synthetic_contract_passes_without_claiming_truth(self):
        result = validate_receipt(copy.deepcopy(self.synthetic))
        self.assertEqual(result["authority_class"], "inconclusive")
        self.assertFalse(result["closure_candidate"])
        self.assertTrue(result["source_free_receipt"])

    def test_mock_native_closure_shape_passes(self):
        result = validate_receipt(native_closure(self.synthetic))
        self.assertEqual(result["authority_class"], "inverse_cache")
        self.assertTrue(result["closure_candidate"])
        self.assertTrue(result["family_a_complete"])
        self.assertTrue(result["family_b_complete"])

    def test_wrong_public_fixture_sha_is_rejected(self):
        r = copy.deepcopy(self.synthetic)
        r["fixture"]["sha256"] = "b" * 64
        with self.assertRaises(AssertionError):
            validate_receipt(r)

    def test_native_observation_requires_real_reset_evidence(self):
        for key in ("reset_provider_receipt_verified", "cold_restore_pair_verified"):
            r = native_closure(self.synthetic)
            r["environment"][key] = False
            with self.assertRaises(AssertionError, msg=key):
                validate_receipt(r)

        r = native_closure(self.synthetic)
        r["environment"]["environment_fingerprint_sha256"] = None
        with self.assertRaises(AssertionError):
            validate_receipt(r)

    def test_closure_requires_both_complete_conflict_families(self):
        for family in ("family_a", "family_b"):
            for key in self.synthetic[family]["capture"]:
                r = native_closure(self.synthetic)
                r[family]["capture"][key] = False
                with self.assertRaises(AssertionError, msg=f"{family}.{key}"):
                    validate_receipt(r)

    def test_closure_cannot_hide_unobserved_outcome(self):
        fields = [
            ("family_a", "post_save_0x47_outcome"),
            ("family_a", "post_save_fopt_outcome"),
            ("family_a", "layout_outcome"),
            ("family_b", "post_save_0x47_outcome"),
            ("family_b", "post_save_fopt_outcome"),
            ("family_b", "layout_outcome"),
        ]
        for family, key in fields:
            r = native_closure(self.synthetic)
            r[family][key] = "not_observed"
            with self.assertRaises(AssertionError, msg=f"{family}.{key}"):
                validate_receipt(r)

    def test_synthetic_receipt_cannot_assert_authority(self):
        r = copy.deepcopy(self.synthetic)
        r["conclusion"]["authority_class"] = "source"
        with self.assertRaises(AssertionError):
            validate_receipt(r)

    def test_private_values_are_rejected(self):
        for key in self.synthetic["privacy"]:
            r = copy.deepcopy(self.synthetic)
            r["privacy"][key] = True
            with self.assertRaises(AssertionError, msg=key):
                validate_receipt(r)

    def test_extra_private_fields_are_rejected_by_schema(self):
        for key, value in [
            ("local_path", "C:/lab/private.pub"),
            ("host_name", "PUB-LAB-01"),
            ("document_text", "secret"),
            ("pub_bytes", "base64..."),
        ]:
            r = copy.deepcopy(self.synthetic)
            r[key] = value
            with self.assertRaises(AssertionError, msg=key):
                validate_receipt(r)


if __name__ == "__main__":
    unittest.main()
