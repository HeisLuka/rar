import unittest

from export_job_v1 import CloudExportJobServiceV1, ExportConflict, ExportRejected


REV1="sha256:"+"a"*64
REV2="sha256:"+"b"*64
LAYOUT="sha256:"+"c"*64


def allow_all(action, job):
    return True


class ExportJobTests(unittest.TestCase):
    def create(self, authz=allow_all):
        svc=CloudExportJobServiceV1()
        job=svc.create_export(
            tenant_id="tenant:1",document_id="doc:1",exact_revision_id=REV1,
            target_profile="pdf:v1",layout_environment_id=LAYOUT,
            client_request_id="export-request-0001",authz=authz
        )
        return svc,job

    def test_create_is_idempotent_and_exact_revision_pinned(self):
        svc,first=self.create()
        second=svc.create_export(
            tenant_id="tenant:1",document_id="doc:1",exact_revision_id=REV1,
            target_profile="pdf:v1",layout_environment_id=LAYOUT,
            client_request_id="export-request-0001",authz=allow_all
        )
        self.assertEqual(first,second)
        self.assertEqual(REV1,first["exact_revision_id"])

    def test_same_id_changed_revision_conflicts(self):
        svc,_=self.create()
        with self.assertRaisesRegex(ExportConflict,"idempotency_conflict"):
            svc.create_export(
                tenant_id="tenant:1",document_id="doc:1",exact_revision_id=REV2,
                target_profile="pdf:v1",layout_environment_id=LAYOUT,
                client_request_id="export-request-0001",authz=allow_all
            )

    def test_authz_is_checked_at_create(self):
        svc=CloudExportJobServiceV1()
        with self.assertRaisesRegex(ExportRejected,"authz_denied"):
            svc.create_export(
                tenant_id="tenant:1",document_id="doc:1",exact_revision_id=REV1,
                target_profile="pdf:v1",layout_environment_id=LAYOUT,
                client_request_id="export-request-0002",authz=lambda action,job: False
            )
        self.assertEqual({},svc.jobs)

    def test_claim_lease_expiry_and_reclaim(self):
        svc,job=self.create()
        claimed=svc.claim(job_id=job["job_id"],worker_id="worker:a",authz=allow_all)
        self.assertEqual("running",claimed["state"])
        self.assertEqual(1,claimed["lease_generation"])
        with self.assertRaisesRegex(ExportConflict,"lease_held"):
            svc.claim(job_id=job["job_id"],worker_id="worker:b",authz=allow_all)
        queued=svc.expire_lease(job_id=job["job_id"],expected_lease_generation=1)
        self.assertEqual("queued",queued["state"])
        reclaimed=svc.claim(job_id=job["job_id"],worker_id="worker:b",authz=allow_all)
        self.assertEqual(2,reclaimed["lease_generation"])

    def test_queued_cancel_is_terminal_without_publish(self):
        svc,job=self.create()
        cancelled=svc.request_cancel(job_id=job["job_id"])
        self.assertEqual("cancelled",cancelled["state"])
        with self.assertRaisesRegex(ExportRejected,"not_claimable"):
            svc.claim(job_id=job["job_id"],worker_id="worker:a",authz=allow_all)

    def test_running_cancel_wins_before_publish(self):
        svc,job=self.create()
        claim=svc.claim(job_id=job["job_id"],worker_id="worker:a",authz=allow_all)
        svc.request_cancel(job_id=job["job_id"])
        with self.assertRaisesRegex(ExportRejected,"cancelled_before_publish"):
            svc.publish(
                job_id=job["job_id"],worker_id="worker:a",
                lease_generation=claim["lease_generation"],
                artifact_content_hash="d"*64,loss_report_hash="e"*64,authz=allow_all
            )
        self.assertEqual("cancelled",svc.snapshot(job["job_id"])["state"])
        self.assertIsNone(svc.snapshot(job["job_id"])["artifact_binding_id"])

    def test_authz_rechecked_before_publish(self):
        svc,job=self.create()
        claim=svc.claim(job_id=job["job_id"],worker_id="worker:a",authz=allow_all)
        with self.assertRaisesRegex(ExportRejected,"authz_denied"):
            svc.publish(
                job_id=job["job_id"],worker_id="worker:a",
                lease_generation=claim["lease_generation"],
                artifact_content_hash="d"*64,loss_report_hash="e"*64,
                authz=lambda action,j: action!="publish"
            )
        snap=svc.snapshot(job["job_id"])
        self.assertEqual("failed",snap["state"])
        self.assertIsNone(snap["artifact_binding_id"])

    def test_success_is_bound_to_exact_revision_and_late_cancel_too_late(self):
        svc,job=self.create()
        claim=svc.claim(job_id=job["job_id"],worker_id="worker:a",authz=allow_all)
        succeeded=svc.publish(
            job_id=job["job_id"],worker_id="worker:a",
            lease_generation=claim["lease_generation"],
            artifact_content_hash="d"*64,loss_report_hash="e"*64,authz=allow_all
        )
        self.assertEqual("succeeded",succeeded["state"])
        self.assertEqual(REV1,succeeded["exact_revision_id"])
        with self.assertRaisesRegex(ExportRejected,"too_late"):
            svc.request_cancel(job_id=job["job_id"])

    def test_download_rechecks_authz_and_expiry(self):
        svc,job=self.create()
        claim=svc.claim(job_id=job["job_id"],worker_id="worker:a",authz=allow_all)
        svc.publish(
            job_id=job["job_id"],worker_id="worker:a",
            lease_generation=claim["lease_generation"],
            artifact_content_hash="d"*64,loss_report_hash="e"*64,authz=allow_all
        )
        with self.assertRaisesRegex(ExportRejected,"authz_denied"):
            svc.authorize_download(job_id=job["job_id"],authz=lambda action,j: False)
        allowed=svc.authorize_download(job_id=job["job_id"],authz=allow_all)
        self.assertEqual(REV1,allowed["exact_revision_id"])
        expired=svc.expire_artifact(job_id=job["job_id"])
        self.assertTrue(expired["artifact_expired"])
        self.assertEqual("succeeded",expired["state"])
        with self.assertRaisesRegex(ExportRejected,"artifact_expired"):
            svc.authorize_download(job_id=job["job_id"],authz=allow_all)

    def test_content_hash_does_not_grant_cross_tenant_download(self):
        svc1,job1=self.create()
        claim1=svc1.claim(job_id=job1["job_id"],worker_id="w1",authz=allow_all)
        out1=svc1.publish(
            job_id=job1["job_id"],worker_id="w1",
            lease_generation=claim1["lease_generation"],
            artifact_content_hash="f"*64,loss_report_hash="e"*64,authz=allow_all
        )

        svc2=CloudExportJobServiceV1()
        job2=svc2.create_export(
            tenant_id="tenant:2",document_id="doc:2",exact_revision_id=REV1,
            target_profile="pdf:v1",layout_environment_id=LAYOUT,
            client_request_id="export-request-0001",authz=allow_all
        )
        claim2=svc2.claim(job_id=job2["job_id"],worker_id="w2",authz=allow_all)
        out2=svc2.publish(
            job_id=job2["job_id"],worker_id="w2",
            lease_generation=claim2["lease_generation"],
            artifact_content_hash="f"*64,loss_report_hash="e"*64,authz=allow_all
        )
        self.assertEqual(out1["artifact_content_hash"],out2["artifact_content_hash"])
        self.assertNotEqual(out1["artifact_binding_id"],out2["artifact_binding_id"])


if __name__=="__main__":
    unittest.main()
