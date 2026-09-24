import unittest
from quota_v1 import CloudQuotaV1, QuotaConflict, QuotaRejected


class CloudQuotaV1Tests(unittest.TestCase):
    def quota(self):
        return CloudQuotaV1(
            shared_capacity=10,
            semantic_headroom=5,
            export_cap=4,
            background_cap=4,
        )

    def test_exact_retry_is_idempotent(self):
        q=self.quota()
        first=q.reserve(tenant_id="t1",reservation_id="r1",work_class="export",amount=2)
        second=q.reserve(tenant_id="t1",reservation_id="r1",work_class="export",amount=2)
        self.assertEqual(first,second)

    def test_same_id_changed_amount_or_class_conflicts(self):
        q=self.quota()
        q.reserve(tenant_id="t1",reservation_id="r1",work_class="export",amount=2)
        with self.assertRaisesRegex(QuotaConflict,"reservation_conflict"):
            q.reserve(tenant_id="t1",reservation_id="r1",work_class="export",amount=3)
        with self.assertRaisesRegex(QuotaConflict,"reservation_conflict"):
            q.reserve(tenant_id="t1",reservation_id="r1",work_class="background",amount=2)

    def test_export_cannot_consume_protected_semantic_headroom(self):
        q=self.quota()
        q.reserve(tenant_id="t1",reservation_id="i1",work_class="interactive",amount=5)
        q.reserve(tenant_id="t1",reservation_id="e1",work_class="export",amount=4)
        with self.assertRaisesRegex(QuotaRejected,"shared_capacity_exhausted"):
            q.reserve(tenant_id="t1",reservation_id="e2",work_class="export",amount=2)
        # Interactive may still use the protected reserve.
        i2=q.reserve(tenant_id="t1",reservation_id="i2",work_class="interactive",amount=5)
        self.assertFalse(i2["released"])
        self.assertEqual(5,q.usage("t1")["protected_interactive"])

    def test_background_degrades_before_semantic_headroom(self):
        q=self.quota()
        q.reserve(tenant_id="t1",reservation_id="e1",work_class="export",amount=4)
        q.reserve(tenant_id="t1",reservation_id="b1",work_class="background",amount=4)
        with self.assertRaisesRegex(QuotaRejected,"background_budget_paused"):
            q.reserve(tenant_id="t1",reservation_id="b2",work_class="background",amount=1)
        # 2 shared slots + 5 protected remain for interactive.
        q.reserve(tenant_id="t1",reservation_id="i1",work_class="interactive",amount=7)
        with self.assertRaisesRegex(QuotaRejected,"semantic_headroom_exhausted"):
            q.reserve(tenant_id="t1",reservation_id="i2",work_class="interactive",amount=1)

    def test_semantic_exhaustion_rejects_before_reservation(self):
        q=self.quota()
        q.reserve(tenant_id="t1",reservation_id="i1",work_class="interactive",amount=15)
        before=q.usage("t1").copy()
        with self.assertRaisesRegex(QuotaRejected,"semantic_headroom_exhausted"):
            q.reserve(tenant_id="t1",reservation_id="i2",work_class="interactive",amount=1)
        self.assertEqual(before,q.usage("t1"))

    def test_release_is_exact_once_and_frees_capacity(self):
        q=self.quota()
        r=q.reserve(tenant_id="t1",reservation_id="e1",work_class="export",amount=4)
        q.release(tenant_id="t1",reservation_id="e1",expected_lease_generation=r["lease_generation"])
        self.assertEqual(0,q.usage("t1")["export"])
        with self.assertRaisesRegex(QuotaConflict,"already_released"):
            q.release(tenant_id="t1",reservation_id="e1",expected_lease_generation=r["lease_generation"])

    def test_renewed_lease_fences_stale_expiry(self):
        q=self.quota()
        r=q.reserve(tenant_id="t1",reservation_id="e1",work_class="export",amount=2)
        renewed=q.renew_lease(
            tenant_id="t1",reservation_id="e1",expected_lease_generation=r["lease_generation"]
        )
        with self.assertRaisesRegex(QuotaConflict,"lease_advanced"):
            q.expire_stale(
                tenant_id="t1",reservation_id="e1",observed_lease_generation=r["lease_generation"]
            )
        expired=q.expire_stale(
            tenant_id="t1",reservation_id="e1",observed_lease_generation=renewed["lease_generation"]
        )
        self.assertTrue(expired["released"])

    def test_tenants_are_isolated(self):
        q=self.quota()
        q.reserve(tenant_id="t1",reservation_id="e1",work_class="export",amount=4)
        q.reserve(tenant_id="t2",reservation_id="e1",work_class="export",amount=4)
        self.assertEqual(4,q.usage("t1")["export"])
        self.assertEqual(4,q.usage("t2")["export"])


if __name__=="__main__":
    unittest.main()
