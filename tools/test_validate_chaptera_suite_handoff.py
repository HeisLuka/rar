import unittest

from tools.validate_chaptera_suite_handoff import validate_acceptance, validate_packet


SHA = "a" * 64


def packet(target="chaptera.editor"):
    if target == "chaptera.editor":
        requested_job = "edit_supported_pub"
        capability = "reader_supported"
        loss_state = "none_observed"
    else:
        requested_job = "diagnose_or_recover"
        capability = "reader_failure_recovery_eligible"
        loss_state = "unknown"
    return {
        "protocol_version": "chaptera.suite-handoff.v1",
        "sender_product_id": "chaptera.reader",
        "target_product_id": target,
        "requested_job": requested_job,
        "source": {"path": r"C:\\private\\source.pub", "sha256": SHA, "kind": "pub"},
        "context": {"capability": capability, "loss_state": loss_state},
        "provenance": {
            "source_identity_verified": True,
            "mutable_document_state_included": False,
        },
        "consent": {"local_file_handoff": True, "user_initiated": True},
    }


def acceptance(receiver="chaptera.editor"):
    return {
        "protocol_version": "chaptera.suite-handoff-acceptance.v1",
        "sender_product_id": "chaptera.reader",
        "receiver_product_id": receiver,
        "requested_job": (
            "edit_supported_pub" if receiver == "chaptera.editor" else "diagnose_or_recover"
        ),
        "handoff_packet_sha256": "b" * 64,
        "source_sha256": SHA,
        "source_unchanged": True,
        "receiver_capability_admitted": True,
        "mutable_document_state_received": False,
        "source_path_serialized": False,
    }


class SuiteHandoffValidatorTests(unittest.TestCase):
    def test_editor_packet_passes(self):
        summary = validate_packet(packet())
        self.assertEqual(summary["target"], "chaptera.editor")

    def test_rescue_packet_passes(self):
        summary = validate_packet(packet("chaptera.rescue"))
        self.assertEqual(summary["requested_job"], "diagnose_or_recover")

    def test_context_mismatch_fails(self):
        value = packet()
        value["context"]["capability"] = "reader_failure_recovery_eligible"
        value["context"]["loss_state"] = "unknown"
        with self.assertRaisesRegex(AssertionError, "mismatch"):
            validate_packet(value)

    def test_acceptance_is_source_free(self):
        summary = validate_acceptance(acceptance())
        self.assertTrue(summary["source_free"])

    def test_acceptance_cannot_serialize_source_path(self):
        value = acceptance()
        value["source_path_serialized"] = True
        with self.assertRaises(AssertionError):
            validate_acceptance(value)


if __name__ == "__main__":
    unittest.main()
