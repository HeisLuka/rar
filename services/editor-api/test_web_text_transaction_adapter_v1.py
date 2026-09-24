#!/usr/bin/env python3
import unittest

from paragraph_lifecycle_v1 import (
    ParagraphPropertiesV1,
    ParagraphV1,
    build_story_paragraph_state_v1,
)
from revision_store import RevisionKernel
from security.authz_v1 import AuthzDenied, AuthzKernel
from security.authorized_revision_gateway import AuthorizedRevisionGateway
from story_edit_transaction_v1 import (
    build_story_edit_core_state_v1,
    story_edit_core_state_from_dict,
    story_edit_core_state_to_dict,
)
from text_format_overlay_v1 import (
    BaseCharacterFormatV1,
    BaseFormatRunV1,
    build_text_format_overlay_state_v1,
)
from web_text_transaction_adapter_v1 import (
    WebTextTransactionAdapterError,
    lower_browser_story_range_intent_v1,
)


DOCUMENT_ID = "doc:web-text"
SOURCE_HASH = "a" * 64
STORY_ID = "story:1"
REV_OP = "browser-text-op-00000001"
NEW_P1 = "01900000-0000-7000-8000-000000000001"


def fmt():
    return BaseCharacterFormatV1(
        "font:resolved",
        12000,
        False,
        False,
        "#000000",
    )


def core_state(text="ABC", *, unsupported=()):
    paragraphs = tuple(
        ParagraphV1(
            f"paragraph:{index + 1}",
            ParagraphPropertiesV1(()),
            "source_or_existing",
        )
        for index in range(text.count("\r") + 1)
    )
    paragraph_state = build_story_paragraph_state_v1(
        story_id=STORY_ID,
        story_text=text,
        paragraphs=paragraphs,
        protected_terminal_cr=False,
    )
    base_runs = () if not text else (BaseFormatRunV1(0, len(text), fmt()),)
    format_state = build_text_format_overlay_state_v1(
        story_id=STORY_ID,
        base_revision_id="format:base",
        story_scalar_len=len(text),
        base_runs=base_runs,
    )
    return build_story_edit_core_state_v1(
        story_id=STORY_ID,
        provenance="chaptera_created",
        paragraph_state=paragraph_state,
        format_state=format_state,
        unsupported_anchored_semantics=tuple(unsupported),
    )


def project_for(core):
    return {
        "schema_version": "pub-editor-v0.6",
        "source_hash": SOURCE_HASH,
        "operations": [],
        "stories": {STORY_ID: core.paragraph_state.story_text},
        "story_models": {STORY_ID: story_edit_core_state_to_dict(core)},
    }


def browser_request(base_revision_id, *, start, end, replacement, command_extra=None):
    command = {
        "kind": "replace_story_range",
        "story_id": STORY_ID,
        "start_scalar": start,
        "end_scalar": end,
        "replacement_text": replacement,
    }
    if command_extra:
        command.update(command_extra)
    return {
        "protocol_version": "chaptera.story-range-intent.v1",
        "document_id": DOCUMENT_ID,
        "source_hash": SOURCE_HASH,
        "base_revision_id": base_revision_id,
        "client_operation_id": REV_OP,
        "depends_on_client_operation_id": None,
        "command": command,
    }


class WebTextTransactionAdapterV1Tests(unittest.TestCase):
    def test_browser_cannot_supply_canonical_before_state(self):
        base = project_for(core_state("ABC"))
        request = browser_request(
            "sha256:" + "b" * 64,
            start=1,
            end=2,
            replacement="X",
            command_extra={"expected_before": "forged"},
        )
        with self.assertRaises(WebTextTransactionAdapterError) as caught:
            lower_browser_story_range_intent_v1(
                browser_request=request,
                base_project=base,
            )
        self.assertEqual("invalid_browser_text_intent", caught.exception.code)

    def test_server_derives_before_and_normalizes_external_newlines(self):
        base = project_for(core_state("ABC"))
        request = browser_request(
            "sha256:" + "b" * 64,
            start=1,
            end=2,
            replacement="X\r\nY",
        )
        lowered, receipt = lower_browser_story_range_intent_v1(
            browser_request=request,
            base_project=base,
            paragraph_id_factory=lambda ordinal: NEW_P1,
        )
        command = lowered["command"]
        self.assertEqual("chaptera.story-edit-transaction-intent.v1", lowered["protocol_version"])
        self.assertEqual("B", command["expected_before"])
        self.assertEqual("X\rY", command["replacement_text"])
        self.assertEqual([NEW_P1], command["paragraph_inserted_ids"])
        self.assertIsNone(command["typing_format"])
        self.assertEqual([], command["fragment_format_runs"])
        self.assertEqual([], command["incoming_semantic_kinds"])
        self.assertFalse(receipt.browser_before_state_accepted)
        self.assertEqual(1, receipt.inserted_paragraph_count)

    def test_paragraph_insertion_requires_server_allocator(self):
        base = project_for(core_state("ABC"))
        request = browser_request(
            "sha256:" + "b" * 64,
            start=1,
            end=1,
            replacement="\n",
        )
        with self.assertRaises(WebTextTransactionAdapterError) as caught:
            lower_browser_story_range_intent_v1(
                browser_request=request,
                base_project=base,
            )
        self.assertEqual("paragraph_id_allocator_required", caught.exception.code)

    def test_out_of_range_browser_range_fails_before_transaction(self):
        base = project_for(core_state("ABC"))
        request = browser_request(
            "sha256:" + "b" * 64,
            start=2,
            end=9,
            replacement="X",
        )
        with self.assertRaises(WebTextTransactionAdapterError) as caught:
            lower_browser_story_range_intent_v1(
                browser_request=request,
                base_project=base,
            )
        self.assertEqual("story_range_outside_canonical_story", caught.exception.code)

    def test_lowered_request_commits_one_atomic_story_transaction_through_authz(self):
        kernel = RevisionKernel()
        core = core_state("ABC")
        baseline = kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=project_for(core),
        )
        request = browser_request(
            baseline.revision_id,
            start=1,
            end=2,
            replacement="X",
        )
        lowered, _ = lower_browser_story_range_intent_v1(
            browser_request=request,
            base_project=kernel.read_revision(
                document_id=DOCUMENT_ID,
                revision_id=baseline.revision_id,
            ).project,
        )

        authz = AuthzKernel()
        authz.set_role(
            tenant_id="tenant:1",
            document_id=DOCUMENT_ID,
            principal_id="editor:1",
            role="editor",
        )
        gateway = AuthorizedRevisionGateway(
            kernel=kernel,
            authz=authz,
            tenant_id="tenant:1",
        )
        result = gateway.commit(
            lowered,
            principal_id="editor:1",
        )
        self.assertEqual("chaptera.commit-accepted.v1", result["protocol_version"])
        self.assertEqual(
            "story_edit_transaction",
            result["canonical_operation"]["kind"],
        )
        current = kernel.current_revision(DOCUMENT_ID)
        self.assertEqual(baseline.revision_id, current.parent_revision_id)
        after = story_edit_core_state_from_dict(
            current.project["story_models"][STORY_ID]
        )
        self.assertEqual("AXC", after.paragraph_state.story_text)
        self.assertEqual("AXC", current.project["stories"][STORY_ID])
        self.assertEqual(1, len(current.project["operations"]))

    def test_viewer_cannot_commit_lowered_text_transaction(self):
        kernel = RevisionKernel()
        core = core_state("ABC")
        baseline = kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=project_for(core),
        )
        lowered, _ = lower_browser_story_range_intent_v1(
            browser_request=browser_request(
                baseline.revision_id,
                start=1,
                end=2,
                replacement="X",
            ),
            base_project=kernel.read_revision(
                document_id=DOCUMENT_ID,
                revision_id=baseline.revision_id,
            ).project,
        )
        authz = AuthzKernel()
        authz.set_role(
            tenant_id="tenant:1",
            document_id=DOCUMENT_ID,
            principal_id="viewer:1",
            role="viewer",
        )
        gateway = AuthorizedRevisionGateway(
            kernel=kernel,
            authz=authz,
            tenant_id="tenant:1",
        )
        with self.assertRaises(AuthzDenied):
            gateway.commit(lowered, principal_id="viewer:1")
        self.assertEqual(
            baseline.revision_id,
            kernel.current_revision(DOCUMENT_ID).revision_id,
        )

    def test_feature_specific_anchored_semantics_still_fail_closed(self):
        kernel = RevisionKernel()
        core = core_state("ABC", unsupported=("hyperlink",))
        baseline = kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=project_for(core),
        )
        lowered, _ = lower_browser_story_range_intent_v1(
            browser_request=browser_request(
                baseline.revision_id,
                start=1,
                end=2,
                replacement="X",
            ),
            base_project=kernel.read_revision(
                document_id=DOCUMENT_ID,
                revision_id=baseline.revision_id,
            ).project,
        )
        result = kernel.commit_story_edit_transaction(lowered)
        self.assertEqual("anchored_semantics_unsupported", result["code"])
        self.assertEqual(
            baseline.revision_id,
            kernel.current_revision(DOCUMENT_ID).revision_id,
        )


if __name__ == "__main__":
    unittest.main()
