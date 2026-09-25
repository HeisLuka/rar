import copy
import pathlib
import sys
import unittest

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from web_security_boundary import (
    PROFILE,
    SecurityBoundaryError,
    assert_browser_payload_source_neutral,
    authorize_external_fetch,
    build_parse_job,
    build_telemetry_event,
    finalize_pub_upload,
    issue_resource_token,
    tenant_cache_key,
    validate_upload_start,
    verify_resource_token,
)


SECRET = b"test-only-secret-not-production"
TENANT_A = "tenant-a"
TENANT_B = "tenant-b"
DOC = "10000000-0000-4000-8000-000000000001"
RESOURCE = "40000000-0000-4000-8000-000000000001"
CFB = bytes.fromhex("d0cf11e0a1b11ae1") + b"bounded-fixture"


class WebSecurityBoundaryTests(unittest.TestCase):
    def test_declared_oversize_is_rejected_before_body_buffering(self):
        with self.assertRaisesRegex(SecurityBoundaryError, "content-length"):
            validate_upload_start(
                tenant_id=TENANT_A,
                content_length=PROFILE["upload"]["max_content_length_bytes"] + 1,
            )

    def test_upload_receipt_binds_tenant_hash_and_opaque_handle(self):
        receipt = finalize_pub_upload(
            secret=SECRET,
            tenant_id=TENANT_A,
            content=CFB,
            declared_content_length=len(CFB),
            display_name="../../not-a-path.pub",
        )
        self.assertTrue(receipt["immutable_source"])
        self.assertTrue(receipt["source_blob_handle"].startswith("src_"))
        self.assertNotIn("not-a-path", receipt["source_blob_handle"])
        self.assertNotIn("source_path", receipt)

        other = finalize_pub_upload(
            secret=SECRET,
            tenant_id=TENANT_B,
            content=CFB,
            declared_content_length=len(CFB),
        )
        self.assertEqual(receipt["source_hash"], other["source_hash"])
        self.assertNotEqual(receipt["source_blob_handle"], other["source_blob_handle"])

    def test_non_cfb_and_length_mismatch_fail_closed(self):
        with self.assertRaises(SecurityBoundaryError):
            finalize_pub_upload(
                secret=SECRET,
                tenant_id=TENANT_A,
                content=b"not-cfb",
                declared_content_length=7,
            )
        with self.assertRaises(SecurityBoundaryError):
            finalize_pub_upload(
                secret=SECRET,
                tenant_id=TENANT_A,
                content=CFB,
                declared_content_length=len(CFB) + 1,
            )

    def test_parse_job_has_no_network_or_physical_source_path(self):
        receipt = finalize_pub_upload(
            secret=SECRET,
            tenant_id=TENANT_A,
            content=CFB,
            declared_content_length=len(CFB),
        )
        job = build_parse_job(receipt)
        self.assertFalse(job["network_access"])
        self.assertEqual(job["temp_storage"], "disposable")
        self.assertNotIn("source_path", job)
        self.assertNotIn("filesystem_path", job)
        self.assertEqual(job["tenant_id"], TENANT_A)

    def test_browser_payload_guard_rejects_private_carriers_recursively(self):
        assert_browser_payload_source_neutral({
            "document_id": DOC,
            "resources": [{"fetch_handle": "opaque"}],
        })
        for payload in (
            {"source_path": "C:/secret.pub"},
            {"node": {"cfb_path": "Contents"}},
            {"items": [{"raw_pub_bytes": "AAAA"}]},
        ):
            with self.subTest(payload=payload):
                with self.assertRaises(SecurityBoundaryError):
                    assert_browser_payload_source_neutral(payload)

    def test_resource_token_is_tenant_document_resource_bound_and_expiring(self):
        token = issue_resource_token(
            secret=SECRET,
            tenant_id=TENANT_A,
            document_id=DOC,
            resource_id=RESOURCE,
            now_epoch=1000,
            ttl_seconds=60,
        )
        payload = verify_resource_token(
            token,
            secret=SECRET,
            tenant_id=TENANT_A,
            document_id=DOC,
            resource_id=RESOURCE,
            now_epoch=1059,
        )
        self.assertEqual(payload["permission"], "read")

        with self.assertRaises(SecurityBoundaryError):
            verify_resource_token(
                token,
                secret=SECRET,
                tenant_id=TENANT_B,
                document_id=DOC,
                resource_id=RESOURCE,
                now_epoch=1059,
            )
        with self.assertRaises(SecurityBoundaryError):
            verify_resource_token(
                token,
                secret=SECRET,
                tenant_id=TENANT_A,
                document_id=DOC,
                resource_id=RESOURCE,
                now_epoch=1060,
            )
        tampered = token[:-1] + ("A" if token[-1] != "A" else "B")
        with self.assertRaises(SecurityBoundaryError):
            verify_resource_token(
                tampered,
                secret=SECRET,
                tenant_id=TENANT_A,
                document_id=DOC,
                resource_id=RESOURCE,
                now_epoch=1059,
            )

    def test_same_content_hash_never_shares_cache_identity_across_tenants(self):
        h = "a" * 64
        a = tenant_cache_key(tenant_id=TENANT_A, content_hash=h, variant="preview")
        b = tenant_cache_key(tenant_id=TENANT_B, content_hash=h, variant="preview")
        self.assertNotEqual(a, b)

    def test_external_fetch_is_default_deny_including_ssrf_targets(self):
        for url in (
            "https://example.com/a.png",
            "http://127.0.0.1/admin",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.1/private",
            "file:///etc/passwd",
        ):
            with self.subTest(url=url):
                with self.assertRaises(SecurityBoundaryError):
                    authorize_external_fetch(url)

    def test_telemetry_allowlist_excludes_document_content_and_secrets(self):
        event = build_telemetry_event(
            code="parse.failed",
            metadata={"stage": "parse", "status": "error", "error_code": "cfb.invalid"},
        )
        self.assertEqual(event["code"], "parse.failed")

        for key, value in (
            ("document_text", "secret words"),
            ("raw_pub_bytes", "AAAA"),
            ("signed_url", "https://example.test/token"),
            ("resource_token", "r1.secret"),
            ("error_code", "this looks like copied document text"),
        ):
            with self.subTest(key=key):
                with self.assertRaises(SecurityBoundaryError):
                    build_telemetry_event(code="bad", metadata={key: value})


    def test_resource_token_issue_time_must_be_structural_integer(self):
        with self.assertRaises(SecurityBoundaryError):
            issue_resource_token(
                secret=SECRET,
                tenant_id=TENANT_A,
                document_id=DOC,
                resource_id=RESOURCE,
                now_epoch=-1,
            )


if __name__ == "__main__":
    unittest.main()
