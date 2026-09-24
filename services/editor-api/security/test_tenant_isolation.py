import pathlib
import sys
import unittest

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from tenant_isolation import (
    POLICY,
    TempScopeRegistry,
    TenantIsolationError,
    authorize_worker_input,
    build_tenant_audit_event,
    build_worker_job,
    issue_artifact_grant,
    issue_artifact_handle,
    tenant_job_cache_key,
    verify_artifact_grant,
    verify_artifact_handle,
)


SECRET = b"test-only-tenant-secret"
A = "tenant-a"
B = "tenant-b"
HASH = "a" * 64


def handle(tenant, artifact_id="artifact-1", kind="source"):
    return issue_artifact_handle(
        secret=SECRET,
        tenant_id=tenant,
        artifact_id=artifact_id,
        artifact_kind=kind,
        content_hash=HASH,
    )


class TenantIsolationTests(unittest.TestCase):
    def test_same_bytes_have_different_tenant_handles_and_cache_keys(self):
        self.assertNotEqual(handle(A), handle(B))
        self.assertNotEqual(
            tenant_job_cache_key(tenant_id=A, content_hash=HASH, variant="scene"),
            tenant_job_cache_key(tenant_id=B, content_hash=HASH, variant="scene"),
        )

    def test_cross_tenant_artifact_handle_fails_closed(self):
        a = handle(A)
        self.assertEqual(
            verify_artifact_handle(a, secret=SECRET, tenant_id=A)["tenant_id"],
            A,
        )
        with self.assertRaisesRegex(TenantIsolationError, "cross-tenant"):
            verify_artifact_handle(a, secret=SECRET, tenant_id=B)

    def test_worker_gets_only_explicit_read_only_handles(self):
        a1 = handle(A, "source-1", "source")
        a2 = handle(A, "scene-1", "scene")
        job = build_worker_job(
            secret=SECRET,
            tenant_id=A,
            job_id="job-1",
            input_handles=[a1],
        )
        self.assertEqual(job["input_permissions"], "read-only")
        self.assertTrue(job["temp_cleanup_required"])
        self.assertEqual(
            authorize_worker_input(job, a1, secret=SECRET, tenant_id=A)["artifact_id"],
            "source-1",
        )
        with self.assertRaises(TenantIsolationError):
            authorize_worker_input(job, a2, secret=SECRET, tenant_id=A)
        with self.assertRaises(TenantIsolationError):
            authorize_worker_input(job, a1, secret=SECRET, tenant_id=B)

    def test_cross_tenant_handle_cannot_enter_job_inputs(self):
        with self.assertRaises(TenantIsolationError):
            build_worker_job(
                secret=SECRET,
                tenant_id=A,
                job_id="job-2",
                input_handles=[handle(B)],
            )

    def test_signed_grant_is_short_lived_tenant_and_artifact_bound(self):
        h = handle(A, "export-1", "export")
        grant = issue_artifact_grant(
            secret=SECRET,
            tenant_id=A,
            artifact_handle=h,
            now_epoch=100,
            ttl_seconds=30,
        )
        payload = verify_artifact_grant(
            grant,
            secret=SECRET,
            tenant_id=A,
            artifact_id="export-1",
            now_epoch=129,
        )
        self.assertEqual(payload["permission"], "read")

        with self.assertRaises(TenantIsolationError):
            verify_artifact_grant(
                grant,
                secret=SECRET,
                tenant_id=B,
                artifact_id="export-1",
                now_epoch=129,
            )
        with self.assertRaises(TenantIsolationError):
            verify_artifact_grant(
                grant,
                secret=SECRET,
                tenant_id=A,
                artifact_id="other",
                now_epoch=129,
            )
        with self.assertRaises(TenantIsolationError):
            verify_artifact_grant(
                grant,
                secret=SECRET,
                tenant_id=A,
                artifact_id="export-1",
                now_epoch=130,
            )
        tampered = grant[:-1] + ("A" if grant[-1] != "A" else "B")
        with self.assertRaises(TenantIsolationError):
            verify_artifact_grant(
                tampered,
                secret=SECRET,
                tenant_id=A,
                artifact_id="export-1",
                now_epoch=129,
            )

    def test_temp_scope_cannot_cross_tenant_or_survive_cleanup(self):
        registry = TempScopeRegistry()
        ns = registry.open_job(tenant_id=A, job_id="job-temp")
        registry.assert_access(tenant_id=A, job_id="job-temp", namespace=ns)
        with self.assertRaises(TenantIsolationError):
            registry.assert_access(tenant_id=B, job_id="job-temp", namespace=ns)

        closed = registry.close_job(tenant_id=A, job_id="job-temp")
        self.assertEqual(closed, ns)
        with self.assertRaises(TenantIsolationError):
            registry.assert_access(tenant_id=A, job_id="job-temp", namespace=ns)
        with self.assertRaisesRegex(TenantIsolationError, "cannot be reused"):
            registry.open_job(tenant_id=A, job_id="job-temp")

    def test_job_namespaces_are_tenant_specific(self):
        a = build_worker_job(
            secret=SECRET,
            tenant_id=A,
            job_id="same-job",
            input_handles=[handle(A)],
        )
        b = build_worker_job(
            secret=SECRET,
            tenant_id=B,
            job_id="same-job",
            input_handles=[handle(B)],
        )
        self.assertNotEqual(a["output_namespace"], b["output_namespace"])
        self.assertNotEqual(a["temp_namespace"], b["temp_namespace"])

    def test_audit_is_structural_and_rejects_payload_content(self):
        event = build_tenant_audit_event(
            tenant_id=A,
            action="artifact.read",
            result="denied",
            metadata={
                "resource_kind": "source",
                "job_id": "job-7",
                "error_code": "tenant_mismatch",
            },
        )
        self.assertEqual(event["tenant_id"], A)

        for key, value in (
            ("document_text", "private words"),
            ("payload", "secret"),
            ("source_path", "/tmp/source.pub"),
            ("token", "secret-token"),
        ):
            with self.subTest(key=key):
                with self.assertRaises(TenantIsolationError):
                    build_tenant_audit_event(
                        tenant_id=A,
                        action="bad",
                        result="denied",
                        metadata={key: value},
                    )

    def test_job_input_limit_is_fail_closed(self):
        too_many = [
            handle(A, f"a-{i}", "source")
            for i in range(POLICY["max_job_inputs"] + 1)
        ]
        with self.assertRaises(TenantIsolationError):
            build_worker_job(
                secret=SECRET,
                tenant_id=A,
                job_id="job-overflow",
                input_handles=too_many,
            )


if __name__ == "__main__":
    unittest.main()
