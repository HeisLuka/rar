import copy
import pathlib
import sys
import threading
import time
import unittest

HERE = pathlib.Path(__file__).resolve().parent
EDITOR_API = HERE.parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(EDITOR_API))

from authz_v1 import (
    AuthzDenied,
    AuthzKernel,
    CAP_EDIT_GEOMETRY,
    CAP_MEMBER_MANAGE,
    CAP_VIEW,
)
from authorized_revision_gateway import AuthorizedRevisionGateway
from revision_store import RevisionKernel


TENANT = "tenant-a"
OTHER_TENANT = "tenant-b"
DOCUMENT = "10000000-0000-4000-8000-000000000001"
NODE = "30000000-0000-4000-8000-000000000001"
SOURCE_HASH = "a" * 64
SECRET = b"cloud-authz-test-secret"


class FakeMoveExecutor:
    def __init__(self):
        self.calls = 0
        self.started = threading.Event()
        self.release = threading.Event()

    def __call__(self, base_project, command):
        self.calls += 1
        self.started.set()
        self.release.wait(timeout=5)
        before = {"x": 0, "y": 0, "width": 100, "height": 100}
        operation = {
            "kind": "move_node",
            "node_id": command["node_id"],
            "before": before,
            "after": {
                "x": command["x_emu"],
                "y": command["y_emu"],
                "width": 100,
                "height": 100,
            },
        }
        project = copy.deepcopy(base_project)
        project["operations"] = list(project["operations"]) + [copy.deepcopy(operation)]
        return operation, project, [{"key": "node.geometry.position", "state": "supported", "note": None}]


def request(base_revision_id, op_id="op-1"):
    return {
        "protocol_version": "chaptera.commit-request.v1",
        "document_id": DOCUMENT,
        "source_hash": SOURCE_HASH,
        "base_revision_id": base_revision_id,
        "client_operation_id": op_id,
        "command": {
            "kind": "move_node_to",
            "node_id": NODE,
            "x_emu": 12700,
            "y_emu": 25400,
        },
    }


class AuthzKernelTests(unittest.TestCase):
    def setUp(self):
        self.authz = AuthzKernel()
        self.authz.set_role(
            tenant_id=TENANT,
            document_id=DOCUMENT,
            principal_id="owner",
            role="owner",
        )

    def test_viewer_cannot_mutate_and_owner_can_manage_members(self):
        self.authz.set_role(
            tenant_id=TENANT,
            document_id=DOCUMENT,
            principal_id="viewer",
            role="viewer",
        )
        with self.assertRaisesRegex(AuthzDenied, "capability_denied"):
            self.authz.authorize(
                tenant_id=TENANT,
                document_id=DOCUMENT,
                principal_id="viewer",
                capability=CAP_EDIT_GEOMETRY,
            )
        decision = self.authz.authorize(
            tenant_id=TENANT,
            document_id=DOCUMENT,
            principal_id="owner",
            capability=CAP_MEMBER_MANAGE,
        )
        self.assertEqual("owner", decision.role)

    def test_cross_tenant_grant_does_not_authorize_same_principal(self):
        self.authz.set_role(
            tenant_id=TENANT,
            document_id=DOCUMENT,
            principal_id="editor",
            role="editor",
        )
        with self.assertRaisesRegex(AuthzDenied, "grant_missing"):
            self.authz.authorize(
                tenant_id=OTHER_TENANT,
                document_id=DOCUMENT,
                principal_id="editor",
                capability=CAP_VIEW,
            )

    def test_live_revoke_closes_subscription_and_fanout(self):
        self.authz.set_role(
            tenant_id=TENANT,
            document_id=DOCUMENT,
            principal_id="editor",
            role="editor",
        )
        opened = self.authz.subscribe(
            tenant_id=TENANT,
            document_id=DOCUMENT,
            principal_id="editor",
            subscription_id="sub-1",
        )
        self.assertTrue(opened["active"])
        self.assertEqual(("sub-1",), self.authz.fanout_recipients(
            tenant_id=TENANT,
            document_id=DOCUMENT,
        ))

        receipt = self.authz.revoke(
            tenant_id=TENANT,
            document_id=DOCUMENT,
            principal_id="editor",
        )
        self.assertTrue(receipt["active_session_barrier_complete"])
        self.assertEqual((), self.authz.fanout_recipients(
            tenant_id=TENANT,
            document_id=DOCUMENT,
        ))
        state = self.authz.subscription_state("sub-1")
        self.assertFalse(state["active"])
        self.assertEqual("grant_missing", state["closed_reason"])

    def test_share_token_is_scope_expiry_and_revoke_bound(self):
        token = self.authz.issue_share_grant(
            secret=SECRET,
            tenant_id=TENANT,
            document_id=DOCUMENT,
            grant_id="share-1",
            role="commenter",
            now_epoch=100,
            ttl_seconds=30,
        )
        allowed = self.authz.authorize_share_token(
            token=token,
            secret=SECRET,
            tenant_id=TENANT,
            document_id=DOCUMENT,
            capability=CAP_VIEW,
            now_epoch=129,
        )
        self.assertEqual("commenter", allowed.role)

        with self.assertRaisesRegex(AuthzDenied, "share_scope_mismatch"):
            self.authz.authorize_share_token(
                token=token,
                secret=SECRET,
                tenant_id=OTHER_TENANT,
                document_id=DOCUMENT,
                capability=CAP_VIEW,
                now_epoch=129,
            )
        with self.assertRaisesRegex(AuthzDenied, "share_expired"):
            self.authz.authorize_share_token(
                token=token,
                secret=SECRET,
                tenant_id=TENANT,
                document_id=DOCUMENT,
                capability=CAP_VIEW,
                now_epoch=130,
            )

        self.authz.revoke_share_grant(
            tenant_id=TENANT,
            document_id=DOCUMENT,
            grant_id="share-1",
        )
        with self.assertRaisesRegex(AuthzDenied, "share_revoked"):
            self.authz.authorize_share_token(
                token=token,
                secret=SECRET,
                tenant_id=TENANT,
                document_id=DOCUMENT,
                capability=CAP_VIEW,
                now_epoch=120,
            )

    def test_audit_is_payload_free(self):
        self.authz.authorize(
            tenant_id=TENANT,
            document_id=DOCUMENT,
            principal_id="owner",
            capability=CAP_VIEW,
        )
        for event in self.authz.audit_events():
            serialized = repr(event).lower()
            self.assertNotIn("story_text", serialized)
            self.assertNotIn("raw_pub_bytes", serialized)
            self.assertNotIn("asset_bytes", serialized)
            self.assertNotIn("command", event)
            self.assertNotIn("payload", event)


class AuthorizedRevisionGatewayTests(unittest.TestCase):
    def setUp(self):
        self.revisions = RevisionKernel()
        self.baseline = self.revisions.register_baseline(
            document_id=DOCUMENT,
            source_hash=SOURCE_HASH,
            project={
                "schema_version": "pub-editor-v0.4",
                "source_hash": SOURCE_HASH,
                "operations": [],
            },
        )
        self.authz = AuthzKernel()
        self.gateway = AuthorizedRevisionGateway(
            kernel=self.revisions,
            authz=self.authz,
            tenant_id=TENANT,
        )

    def test_forged_viewer_mutation_never_reaches_executor(self):
        self.authz.set_role(
            tenant_id=TENANT,
            document_id=DOCUMENT,
            principal_id="viewer",
            role="viewer",
        )
        executor = FakeMoveExecutor()
        executor.release.set()
        with self.assertRaisesRegex(AuthzDenied, "capability_denied"):
            self.gateway.commit(
                request(self.baseline.revision_id),
                principal_id="viewer",
                executor=executor,
            )
        self.assertEqual(0, executor.calls)
        self.assertEqual(
            self.baseline.revision_id,
            self.revisions.current_revision(DOCUMENT).revision_id,
        )

    def test_editor_commit_is_authorized_but_authz_version_does_not_enter_revision_identity(self):
        self.authz.set_role(
            tenant_id=TENANT,
            document_id=DOCUMENT,
            principal_id="editor",
            role="editor",
        )
        executor = FakeMoveExecutor()
        executor.release.set()
        first = self.gateway.commit(
            request(self.baseline.revision_id, "op-a"),
            principal_id="editor",
            executor=executor,
        )
        self.assertEqual("chaptera.commit-accepted.v1", first["protocol_version"])
        self.assertNotIn("authz_version", first)

    def test_revoke_barrier_linearizes_against_inflight_commit(self):
        self.authz.set_role(
            tenant_id=TENANT,
            document_id=DOCUMENT,
            principal_id="editor",
            role="editor",
        )
        executor = FakeMoveExecutor()
        result_box = {}
        revoke_box = {}

        def do_commit():
            result_box["result"] = self.gateway.commit(
                request(self.baseline.revision_id, "op-race"),
                principal_id="editor",
                executor=executor,
            )

        def do_revoke():
            revoke_box["receipt"] = self.authz.revoke(
                tenant_id=TENANT,
                document_id=DOCUMENT,
                principal_id="editor",
            )

        commit_thread = threading.Thread(target=do_commit)
        commit_thread.start()
        self.assertTrue(executor.started.wait(timeout=2))

        revoke_thread = threading.Thread(target=do_revoke)
        revoke_thread.start()
        time.sleep(0.05)
        self.assertTrue(revoke_thread.is_alive(), "revoke returned before admitted commit linearized")

        executor.release.set()
        commit_thread.join(timeout=3)
        revoke_thread.join(timeout=3)

        self.assertEqual("chaptera.commit-accepted.v1", result_box["result"]["protocol_version"])
        self.assertTrue(revoke_box["receipt"]["active_session_barrier_complete"])

        later_executor = FakeMoveExecutor()
        later_executor.release.set()
        current = self.revisions.current_revision(DOCUMENT).revision_id
        with self.assertRaisesRegex(AuthzDenied, "grant_missing"):
            self.gateway.commit(
                request(current, "op-after-revoke"),
                principal_id="editor",
                executor=later_executor,
            )
        self.assertEqual(0, later_executor.calls)


if __name__ == "__main__":
    unittest.main()
