import unittest

from comment_service_v1 import (
    CloudCommentServiceV1,
    CommentConflict,
    CommentRejected,
    NodeAnchor,
    StoryRangeAnchor,
    TextEdit,
)


def allow_all(action, tenant_id, document_id, principal_id):
    return True


class CommentServiceTests(unittest.TestCase):
    def setUp(self):
        self.svc = CloudCommentServiceV1()

    def create_story_thread(self, request_id="req:1"):
        return self.svc.create_thread(
            tenant_id="tenant:1",
            document_id="doc:1",
            created_revision_id="rev:1",
            anchor=StoryRangeAnchor(
                story_id="story:1",
                start=10,
                end=15,
                revision_id="rev:1",
            ),
            principal_id="user:1",
            body="hello",
            client_request_id=request_id,
            authz=allow_all,
        )

    def test_create_is_idempotent_and_separate_from_document_revision(self):
        first = self.create_story_thread()
        second = self.create_story_thread()
        self.assertEqual(first, second)
        self.assertEqual("rev:1", first["created_revision_id"])
        self.assertEqual(1, first["comment_store_version"])
        self.assertEqual(1, self.svc.store_version)

    def test_same_id_changed_request_conflicts(self):
        self.create_story_thread()
        with self.assertRaisesRegex(CommentConflict, "idempotency_conflict"):
            self.svc.create_thread(
                tenant_id="tenant:1",
                document_id="doc:1",
                created_revision_id="rev:1",
                anchor=StoryRangeAnchor(
                    story_id="story:1",
                    start=11,
                    end=15,
                    revision_id="rev:1",
                ),
                principal_id="user:1",
                body="hello",
                client_request_id="req:1",
                authz=allow_all,
            )

    def test_authz_checked_on_create_read_reply_and_resolve(self):
        def authz(action, tenant_id, document_id, principal_id):
            return action in {"comment.create", "comment.read"}

        thread = self.svc.create_thread(
            tenant_id="tenant:1",
            document_id="doc:1",
            created_revision_id="rev:1",
            anchor=StoryRangeAnchor(
                story_id="story:1", start=0, end=1, revision_id="rev:1"
            ),
            principal_id="commenter:1",
            body="x",
            client_request_id="create:authz",
            authz=authz,
        )
        self.assertEqual(thread["thread_id"], self.svc.read_thread(
            thread_id=thread["thread_id"],
            principal_id="commenter:1",
            authz=authz,
        )["thread_id"])
        with self.assertRaisesRegex(CommentRejected, "authz_denied"):
            self.svc.reply(
                thread_id=thread["thread_id"],
                principal_id="commenter:1",
                body="no",
                client_request_id="reply:denied",
                authz=authz,
            )
        with self.assertRaisesRegex(CommentRejected, "authz_denied"):
            self.svc.set_resolved(
                thread_id=thread["thread_id"],
                resolved=True,
                principal_id="commenter:1",
                client_request_id="resolve:denied",
                authz=authz,
            )

    def test_revoke_blocks_read_without_deleting_thread(self):
        state = {"allow": True}

        def authz(action, tenant_id, document_id, principal_id):
            return state["allow"]

        thread = self.svc.create_thread(
            tenant_id="tenant:1",
            document_id="doc:1",
            created_revision_id="rev:1",
            anchor=StoryRangeAnchor(
                story_id="story:1", start=0, end=2, revision_id="rev:1"
            ),
            principal_id="user:1",
            body="x",
            client_request_id="create:revoke",
            authz=authz,
        )
        state["allow"] = False
        with self.assertRaisesRegex(CommentRejected, "authz_denied"):
            self.svc.read_thread(
                thread_id=thread["thread_id"],
                principal_id="user:1",
                authz=authz,
            )
        self.assertIn(thread["thread_id"], self.svc.threads)

    def test_story_range_transform_boundary_affinities(self):
        thread = self.create_story_thread()
        result = self.svc.apply_story_edit(
            tenant_id="tenant:1",
            document_id="doc:1",
            story_id="story:1",
            old_story_length=26,
            edit=TextEdit(start=10, end=10, replacement_scalar_length=2),
            new_revision_id="rev:2",
        )
        self.assertEqual({"changed": 1, "orphaned": 0}, result)
        snap = self.svc.snapshot(thread["thread_id"])
        self.assertEqual(12, snap["anchor"]["start"])
        self.assertEqual(17, snap["anchor"]["end"])
        self.assertEqual("rev:2", snap["anchor"]["revision_id"])

    def test_full_story_target_replace_orphans_not_retargets(self):
        thread = self.create_story_thread()
        result = self.svc.apply_story_edit(
            tenant_id="tenant:1",
            document_id="doc:1",
            story_id="story:1",
            old_story_length=26,
            edit=TextEdit(start=10, end=15, replacement_scalar_length=5),
            new_revision_id="rev:2",
        )
        self.assertEqual({"changed": 0, "orphaned": 1}, result)
        snap = self.svc.snapshot(thread["thread_id"])
        self.assertTrue(snap["orphaned"])
        self.assertEqual("anchored_text_replaced_or_deleted", snap["orphan_reason"])
        self.assertEqual(10, snap["anchor"]["start"])
        self.assertEqual(15, snap["anchor"]["end"])

    def test_node_move_preserves_identity_and_delete_orphans(self):
        thread = self.svc.create_thread(
            tenant_id="tenant:1",
            document_id="doc:1",
            created_revision_id="rev:1",
            anchor=NodeAnchor(
                node_id="node:42",
                page_id="page:1",
                revision_id="rev:1",
                last_known_geometry=(1, 2, 3, 4),
            ),
            principal_id="user:1",
            body="node",
            client_request_id="node:create",
            authz=allow_all,
        )
        moved = self.svc.apply_node_change(
            tenant_id="tenant:1",
            document_id="doc:1",
            node_id="node:42",
            new_revision_id="rev:2",
            page_id="page:2",
            geometry=(10, 20, 30, 40),
        )
        self.assertEqual({"changed": 1, "orphaned": 0}, moved)
        snap = self.svc.snapshot(thread["thread_id"])
        self.assertEqual("node:42", snap["anchor"]["node_id"])
        self.assertEqual("page:2", snap["anchor"]["page_id"])
        deleted = self.svc.apply_node_change(
            tenant_id="tenant:1",
            document_id="doc:1",
            node_id="node:42",
            new_revision_id="rev:3",
            deleted=True,
        )
        self.assertEqual({"changed": 0, "orphaned": 1}, deleted)
        self.assertEqual("node_deleted", self.svc.snapshot(thread["thread_id"])["orphan_reason"])

    def test_reply_resolve_reopen_advance_comment_store_only(self):
        thread = self.create_story_thread()
        v1 = thread["comment_store_version"]
        replied = self.svc.reply(
            thread_id=thread["thread_id"],
            principal_id="user:2",
            body="reply",
            client_request_id="reply:1",
            authz=allow_all,
        )
        resolved = self.svc.set_resolved(
            thread_id=thread["thread_id"],
            resolved=True,
            principal_id="user:1",
            client_request_id="resolve:1",
            authz=allow_all,
        )
        reopened = self.svc.set_resolved(
            thread_id=thread["thread_id"],
            resolved=False,
            principal_id="user:1",
            client_request_id="reopen:1",
            authz=allow_all,
        )
        self.assertGreater(replied["comment_store_version"], v1)
        self.assertGreater(resolved["comment_store_version"], replied["comment_store_version"])
        self.assertGreater(reopened["comment_store_version"], resolved["comment_store_version"])
        self.assertEqual("rev:1", reopened["created_revision_id"])

    def test_fork_does_not_inherit_comments(self):
        self.create_story_thread()
        fork_threads = self.svc.fork_document(
            source_tenant_id="tenant:1",
            source_document_id="doc:1",
            fork_tenant_id="tenant:1",
            fork_document_id="doc:2",
        )
        self.assertEqual([], fork_threads)

    def test_hard_purge_is_terminal_and_removes_thread(self):
        thread = self.create_story_thread()
        result = self.svc.hard_purge_document(tenant_id="tenant:1", document_id="doc:1")
        self.assertEqual(1, result["removed_threads"])
        self.assertNotIn(thread["thread_id"], self.svc.threads)
        with self.assertRaisesRegex(CommentRejected, "document_purged"):
            self.svc.create_thread(
                tenant_id="tenant:1",
                document_id="doc:1",
                created_revision_id="rev:2",
                anchor=StoryRangeAnchor(
                    story_id="story:1", start=0, end=1, revision_id="rev:2"
                ),
                principal_id="user:1",
                body="resurrect",
                client_request_id="resurrect:1",
                authz=allow_all,
            )


if __name__ == "__main__":
    unittest.main()
