import unittest

from projections_v1 import (
    CloudProductProjectionsV1,
    CurrentDocument,
    ProjectionConflict,
    ProjectionRejected,
    ThumbnailKey,
)


ALICE = "alice"
BOB = "bob"


def doc(
    document_id,
    revision,
    *,
    name="Newsletter",
    workspace="workspace:a",
    metadata_version=1,
    lifecycle="active",
    allowed=(ALICE,),
):
    return CurrentDocument(
        document_id=document_id,
        current_revision_id=revision,
        metadata_version=metadata_version,
        name=name,
        workspace_id=workspace,
        lifecycle=lifecycle,
        allowed_principals=allowed,
    )


class ProjectionContractTests(unittest.TestCase):
    def test_stale_search_index_cannot_leak_after_revoke(self):
        svc = CloudProductProjectionsV1()
        indexed = doc("doc:1", "rev:1", allowed=(ALICE,))
        svc.search.index_document(document=indexed, terms=("air force", "chapter"))
        current = doc("doc:1", "rev:1", allowed=())
        hits = svc.search.search(
            query="newsletter",
            principal_id=ALICE,
            current_directory={"doc:1": current},
        )
        self.assertEqual([], hits)

    def test_stale_search_index_cannot_resurrect_purged_document(self):
        svc = CloudProductProjectionsV1()
        indexed = doc("doc:1", "rev:1")
        svc.search.index_document(document=indexed, terms=("chapter",))
        current = doc("doc:1", "rev:1", lifecycle="purged")
        self.assertEqual(
            [],
            svc.search.search(
                query="chapter",
                principal_id=ALICE,
                current_directory={"doc:1": current},
            ),
        )

    def test_search_uses_current_metadata_overlay_and_explicit_freshness(self):
        svc = CloudProductProjectionsV1()
        indexed = doc(
            "doc:1",
            "rev:1",
            name="Old Name",
            workspace="workspace:a",
            metadata_version=1,
        )
        svc.search.index_document(document=indexed, terms=("old phrase",))
        current = doc(
            "doc:1",
            "rev:2",
            name="New Name",
            workspace="workspace:b",
            metadata_version=2,
        )
        hits = svc.search.search(
            query="old",
            principal_id=ALICE,
            current_directory={"doc:1": current},
        )
        self.assertEqual(1, len(hits))
        hit = hits[0]
        self.assertEqual("New Name", hit["name"])
        self.assertEqual("workspace:b", hit["workspace_id"])
        self.assertEqual("stale", hit["freshness"])
        self.assertFalse(hit["revision_fresh"])
        self.assertFalse(hit["metadata_fresh"])
        self.assertEqual("rev:2", hit["open_target"]["revision_id"])

    def test_new_name_is_not_magically_recalled_before_reindex(self):
        svc = CloudProductProjectionsV1()
        indexed = doc("doc:1", "rev:1", name="Old Name")
        svc.search.index_document(document=indexed, terms=("brochure",))
        current = doc("doc:1", "rev:1", name="New Name", metadata_version=2)
        hits = svc.search.search(
            query="new",
            principal_id=ALICE,
            current_directory={"doc:1": current},
        )
        self.assertEqual([], hits)

    def test_recent_is_per_principal_activity_not_revision_order(self):
        svc = CloudProductProjectionsV1()
        directory = {
            "doc:a": doc("doc:a", "rev:100", name="Older revision"),
            "doc:b": doc("doc:b", "rev:500", name="Newer revision"),
        }
        svc.recent.record_activity(
            principal_id=ALICE,
            document_id="doc:b",
            activity_id="activity:0001",
            activity_order=450,
            kind="open",
        )
        svc.recent.record_activity(
            principal_id=ALICE,
            document_id="doc:a",
            activity_id="activity:0002",
            activity_order=500,
            kind="open",
        )
        rows = svc.recent.list_recent(
            principal_id=ALICE,
            current_directory=directory,
        )
        self.assertEqual(["doc:a", "doc:b"], [row["document_id"] for row in rows])

    def test_recent_read_path_filters_current_revoke_and_purge(self):
        svc = CloudProductProjectionsV1()
        svc.recent.record_activity(
            principal_id=ALICE,
            document_id="doc:a",
            activity_id="activity:0001",
            activity_order=500,
            kind="open",
        )
        svc.recent.record_activity(
            principal_id=ALICE,
            document_id="doc:b",
            activity_id="activity:0002",
            activity_order=400,
            kind="edit",
        )
        directory = {
            "doc:a": doc("doc:a", "rev:1", allowed=()),
            "doc:b": doc("doc:b", "rev:2", lifecycle="purged"),
        }
        self.assertEqual(
            [],
            svc.recent.list_recent(
                principal_id=ALICE,
                current_directory=directory,
            ),
        )

    def test_recent_activity_idempotency_conflict_fails_closed(self):
        svc = CloudProductProjectionsV1()
        svc.recent.record_activity(
            principal_id=ALICE,
            document_id="doc:a",
            activity_id="activity:0001",
            activity_order=1,
            kind="open",
        )
        with self.assertRaisesRegex(ProjectionConflict, "activity_idempotency_conflict"):
            svc.recent.record_activity(
                principal_id=ALICE,
                document_id="doc:b",
                activity_id="activity:0001",
                activity_order=1,
                kind="open",
            )

    def test_thumbnail_identity_includes_environment_and_versions(self):
        base = dict(
            document_id="doc:1",
            revision_id="rev:10",
            scene_protocol_version="scene:v1",
            renderer_version="thumb:v2",
        )
        a = ThumbnailKey(layout_environment_id="env:a", **base)
        b = ThumbnailKey(layout_environment_id="env:b", **base)
        c = ThumbnailKey(
            layout_environment_id="env:a",
            document_id="doc:1",
            revision_id="rev:10",
            scene_protocol_version="scene:v2",
            renderer_version="thumb:v2",
        )
        d = ThumbnailKey(
            layout_environment_id="env:a",
            document_id="doc:1",
            revision_id="rev:10",
            scene_protocol_version="scene:v1",
            renderer_version="thumb:v3",
        )
        self.assertEqual(4, len({a.artifact_id(), b.artifact_id(), c.artifact_id(), d.artifact_id()}))

    def test_thumbnail_artifact_is_immutable_per_full_key(self):
        svc = CloudProductProjectionsV1()
        key = ThumbnailKey("doc:1", "rev:1", "env:a", "scene:v1", "thumb:v1")
        first = svc.thumbnails.publish(
            key=key,
            content_hash="sha256:" + "a" * 64,
            completion_order=1,
        )
        retry = svc.thumbnails.publish(
            key=key,
            content_hash="sha256:" + "a" * 64,
            completion_order=999,
        )
        self.assertEqual(first["artifact_id"], retry["artifact_id"])
        with self.assertRaisesRegex(ProjectionConflict, "immutable_thumbnail_conflict"):
            svc.thumbnails.publish(
                key=key,
                content_hash="sha256:" + "b" * 64,
                completion_order=2,
            )

    def test_thumbnail_completion_order_cannot_regress_current_selector(self):
        svc = CloudProductProjectionsV1()
        current = doc("doc:1", "rev:102")
        new_key = ThumbnailKey("doc:1", "rev:102", "env:a", "scene:v1", "thumb:v1")
        old_key = ThumbnailKey("doc:1", "rev:101", "env:a", "scene:v1", "thumb:v1")

        svc.thumbnails.publish(
            key=new_key,
            content_hash="sha256:" + "2" * 64,
            completion_order=1,
        )
        svc.thumbnails.publish(
            key=old_key,
            content_hash="sha256:" + "1" * 64,
            completion_order=2,
        )
        selected = svc.thumbnails.select(
            principal_id=ALICE,
            current_document=current,
            layout_environment_id="env:a",
            scene_protocol_version="scene:v1",
            renderer_version="thumb:v1",
            stale_revision_candidates=["rev:101"],
        )
        self.assertEqual("fresh", selected["freshness"])
        self.assertEqual("rev:102", selected["key"]["revision_id"])
        self.assertEqual(1, selected["completion_order"])

    def test_missing_current_thumbnail_returns_explicit_stale_or_missing(self):
        svc = CloudProductProjectionsV1()
        current = doc("doc:1", "rev:102")
        old_key = ThumbnailKey("doc:1", "rev:101", "env:a", "scene:v1", "thumb:v1")
        svc.thumbnails.publish(
            key=old_key,
            content_hash="sha256:" + "1" * 64,
            completion_order=99,
        )
        stale = svc.thumbnails.select(
            principal_id=ALICE,
            current_document=current,
            layout_environment_id="env:a",
            scene_protocol_version="scene:v1",
            renderer_version="thumb:v1",
            stale_revision_candidates=["rev:101"],
        )
        self.assertEqual("stale", stale["freshness"])
        self.assertEqual("rev:101", stale["key"]["revision_id"])

        missing = svc.thumbnails.select(
            principal_id=ALICE,
            current_document=current,
            layout_environment_id="env:b",
            scene_protocol_version="scene:v1",
            renderer_version="thumb:v1",
            stale_revision_candidates=["rev:101"],
        )
        self.assertEqual("missing", missing["freshness"])
        self.assertIsNone(missing["artifact_id"])

    def test_thumbnail_read_checks_current_visibility(self):
        svc = CloudProductProjectionsV1()
        current = doc("doc:1", "rev:1", allowed=())
        with self.assertRaisesRegex(ProjectionRejected, "current_document_not_visible"):
            svc.thumbnails.select(
                principal_id=ALICE,
                current_document=current,
                layout_environment_id="env:a",
                scene_protocol_version="scene:v1",
                renderer_version="thumb:v1",
            )

    def test_rebuild_contract_keeps_recent_outside_revisionstream_only(self):
        rebuild = CloudProductProjectionsV1.rebuild_contract()
        self.assertFalse(rebuild["search"]["canonical_authority"])
        self.assertFalse(rebuild["thumbnails"]["canonical_authority"])
        self.assertFalse(rebuild["recent"]["rebuildable_from_revision_stream_alone"])
        self.assertIn("per-principal activity events/state", rebuild["recent"]["requires_retained"])


if __name__ == "__main__":
    unittest.main()
