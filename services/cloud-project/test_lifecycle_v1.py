import unittest

from lifecycle_v1 import CloudProjectLifecycleV1, LifecycleConflict, LifecycleRejected


TENANT="tenant:1"
WORKSPACE="workspace:a"
SOURCE_HASH="a"*64
REVISION="sha256:"+"b"*64


class CloudProjectLifecycleV1Tests(unittest.TestCase):
    def create(self):
        service=CloudProjectLifecycleV1()
        result=service.create_from_upload(
            tenant_id=TENANT,
            workspace_id=WORKSPACE,
            source_hash=SOURCE_HASH,
            initial_revision_id=REVISION,
            name="Newsletter",
            request_id="create-request-0001",
            owner_grant="owner:alice",
            asset_bindings=("asset:source-image-1",),
        )
        return service,result

    def test_create_retry_is_idempotent(self):
        service,first=self.create()
        second=service.create_from_upload(
            tenant_id=TENANT,
            workspace_id=WORKSPACE,
            source_hash=SOURCE_HASH,
            initial_revision_id=REVISION,
            name="Newsletter",
            request_id="create-request-0001",
            owner_grant="owner:alice",
            asset_bindings=("asset:source-image-1",),
        )
        self.assertEqual(first,second)

    def test_create_same_id_different_request_conflicts(self):
        service,_=self.create()
        with self.assertRaisesRegex(LifecycleConflict,"idempotency_conflict"):
            service.create_from_upload(
                tenant_id=TENANT,
                workspace_id=WORKSPACE,
                source_hash=SOURCE_HASH,
                initial_revision_id=REVISION,
                name="Other",
                request_id="create-request-0001",
                owner_grant="owner:alice",
                asset_bindings=("asset:source-image-1",),
            )

    def test_rename_preserves_document_revision_and_lifecycle_generation(self):
        service,created=self.create()
        renamed=service.rename(
            project_id=created["project_id"],
            expected_lifecycle_generation=0,
            expected_metadata_version=0,
            name="Renamed",
            request_id="rename-request-0001",
        )
        self.assertEqual(created["document_id"],renamed["document_id"])
        self.assertEqual(created["current_revision_id"],renamed["current_revision_id"])
        self.assertEqual(0,renamed["lifecycle_generation"])
        self.assertEqual(1,renamed["metadata_version"])
        self.assertEqual("Renamed",renamed["name"])

    def test_stale_metadata_command_fails_closed(self):
        service,created=self.create()
        service.rename(
            project_id=created["project_id"],
            expected_lifecycle_generation=0,
            expected_metadata_version=0,
            name="A",
            request_id="rename-request-0002",
        )
        with self.assertRaisesRegex(LifecycleConflict,"stale_metadata_version"):
            service.rename(
                project_id=created["project_id"],
                expected_lifecycle_generation=0,
                expected_metadata_version=0,
                name="B",
                request_id="rename-request-0003",
            )

    def test_metadata_change_does_not_invalidate_lifecycle_token(self):
        service,created=self.create()
        renamed=service.rename(
            project_id=created["project_id"],
            expected_lifecycle_generation=0,
            expected_metadata_version=0,
            name="A",
            request_id="rename-request-0004",
        )
        trashed=service.trash(
            project_id=created["project_id"],
            expected_lifecycle_generation=renamed["lifecycle_generation"],
            request_id="trash-request-0001",
        )
        self.assertEqual(1,trashed["lifecycle_generation"])
        self.assertEqual(1,trashed["metadata_version"])

    def test_trash_restore_preserve_semantic_identity(self):
        service,created=self.create()
        trashed=service.trash(
            project_id=created["project_id"],
            expected_lifecycle_generation=0,
            request_id="trash-request-0002",
        )
        restored=service.restore(
            project_id=created["project_id"],
            expected_lifecycle_generation=1,
            request_id="restore-request-0001",
        )
        self.assertEqual(created["project_id"],restored["project_id"])
        self.assertEqual(created["document_id"],restored["document_id"])
        self.assertEqual(created["current_revision_id"],restored["current_revision_id"])
        self.assertEqual(2,restored["lifecycle_generation"])
        self.assertEqual("active",restored["lifecycle_state"])

    def test_stale_lifecycle_generation_fails_closed(self):
        service,created=self.create()
        service.trash(
            project_id=created["project_id"],
            expected_lifecycle_generation=0,
            request_id="trash-request-0003",
        )
        with self.assertRaisesRegex(LifecycleConflict,"stale_lifecycle_generation"):
            service.restore(
                project_id=created["project_id"],
                expected_lifecycle_generation=0,
                request_id="restore-request-0002",
            )

    def test_same_tenant_move_preserves_identity_cross_tenant_rejected(self):
        service,created=self.create()
        moved=service.move_within_tenant(
            project_id=created["project_id"],
            target_workspace_id="workspace:b",
            target_tenant_id=TENANT,
            expected_lifecycle_generation=0,
            expected_metadata_version=0,
            request_id="move-request-0001",
        )
        self.assertEqual(created["project_id"],moved["project_id"])
        self.assertEqual(created["document_id"],moved["document_id"])
        self.assertEqual(created["current_revision_id"],moved["current_revision_id"])
        self.assertEqual(0,moved["lifecycle_generation"])
        self.assertEqual(1,moved["metadata_version"])
        with self.assertRaisesRegex(LifecycleRejected,"cross_tenant_identity_preserving_move_not_v0"):
            service.move_within_tenant(
                project_id=created["project_id"],
                target_workspace_id="workspace:x",
                target_tenant_id="tenant:2",
                expected_lifecycle_generation=0,
                expected_metadata_version=1,
                request_id="move-request-0002",
            )

    def test_fork_creates_new_semantic_identity_without_grants_or_comments(self):
        service,created=self.create()
        fork=service.fork_from_revision(
            source_project_id=created["project_id"],
            selected_revision_id=created["current_revision_id"],
            target_workspace_id="workspace:b",
            request_id="fork-request-0001",
        )
        self.assertNotEqual(created["project_id"],fork["project_id"])
        self.assertNotEqual(created["document_id"],fork["document_id"])
        self.assertNotEqual(created["current_revision_id"],fork["current_revision_id"])
        self.assertEqual(created["source_hash"],fork["source_hash"])
        self.assertEqual((),fork["grants"])
        self.assertFalse(fork["comments_inherited"])
        self.assertNotEqual(created["asset_bindings"],fork["asset_bindings"])
        self.assertEqual("fork",fork["provenance"]["kind"])
        self.assertEqual(created["document_id"],fork["provenance"]["source_document_id"])

    def test_fork_retry_same_result_changed_revision_conflicts(self):
        service,created=self.create()
        first=service.fork_from_revision(
            source_project_id=created["project_id"],
            selected_revision_id=created["current_revision_id"],
            target_workspace_id="workspace:b",
            request_id="fork-request-0002",
        )
        second=service.fork_from_revision(
            source_project_id=created["project_id"],
            selected_revision_id=created["current_revision_id"],
            target_workspace_id="workspace:b",
            request_id="fork-request-0002",
        )
        self.assertEqual(first,second)
        with self.assertRaisesRegex(LifecycleConflict,"idempotency_conflict"):
            service.fork_from_revision(
                source_project_id=created["project_id"],
                selected_revision_id="sha256:"+"c"*64,
                target_workspace_id="workspace:b",
                request_id="fork-request-0002",
            )

    def test_hard_delete_is_terminal_and_cannot_resurrect(self):
        service,created=self.create()
        trashed=service.trash(
            project_id=created["project_id"],
            expected_lifecycle_generation=0,
            request_id="trash-request-0004",
        )
        deleted=service.hard_delete(
            project_id=created["project_id"],
            expected_lifecycle_generation=trashed["lifecycle_generation"],
            request_id="delete-request-0001",
        )
        self.assertTrue(deleted["deleted"])
        self.assertEqual("deleted",deleted["lifecycle_state"])
        with self.assertRaisesRegex(LifecycleRejected,"deleted"):
            service.rename(
                project_id=created["project_id"],
                expected_lifecycle_generation=deleted["lifecycle_generation"],
                expected_metadata_version=deleted["metadata_version"],
                name="resurrect",
                request_id="rename-after-delete",
            )


if __name__ == "__main__":
    unittest.main()
