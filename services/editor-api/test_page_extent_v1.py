#!/usr/bin/env python3
import copy
import unittest

from page_extent_v1 import PageExtentV1Error, apply_set_page_extent_v1
from revision_store import RevisionKernel


DOCUMENT_ID = "document:page-size"
SOURCE_HASH = "a" * 64
PAGE_ID = "page:customer-1"


def extent(width, height):
    return {"width_emu": width, "height_emu": height}


def project():
    return {
        "schema_version": "pub-editor-v0.4",
        "source_hash": SOURCE_HASH,
        "operations": [],
        "pages": {
            PAGE_ID: {
                "size": extent(7_315_200, 9_753_600),
            }
        },
        "shapes": {
            "shape:1": {
                "page_id": PAGE_ID,
                "bounds": {"x": -100, "y": 200, "width": 3000, "height": 4000},
                "transform": {"kind": "identity"},
            }
        },
        "stories": {"story:1": "Keep me exactly"},
    }


def command(before=None, after=None):
    return {
        "kind": "set_page_extent",
        "page_id": PAGE_ID,
        "expected_before": before or extent(7_315_200, 9_753_600),
        "after": after or extent(10_000_000, 8_000_000),
        "semantics": "keep_objects_fixed",
    }


def request(base_revision_id, op_id, cmd=None):
    return {
        "protocol_version": "chaptera.page-extent-intent.v1",
        "document_id": DOCUMENT_ID,
        "source_hash": SOURCE_HASH,
        "base_revision_id": base_revision_id,
        "client_operation_id": op_id,
        "command": copy.deepcopy(cmd or command()),
    }


class CountingExecutor:
    def __init__(self):
        self.calls = 0

    def __call__(self, base_project, accepted_command):
        self.calls += 1
        return apply_set_page_extent_v1(base_project, accepted_command)


class PageExtentV1Tests(unittest.TestCase):
    def setUp(self):
        self.base = project()
        self.kernel = RevisionKernel()
        self.baseline = self.kernel.register_baseline(
            document_id=DOCUMENT_ID,
            source_hash=SOURCE_HASH,
            project=self.base,
        )
        self.executor = CountingExecutor()

    @staticmethod
    def admitted_customer_page(command):
        if command["page_id"] != PAGE_ID:
            raise ValueError("page_role_unresolved")

    def test_commit_changes_only_page_extent_and_preserves_objects_and_story(self):
        accepted = self.kernel.commit_page_extent(
            request(self.baseline.revision_id, "page-size-op-0001"),
            self.executor,
            pre_execute_validator=self.admitted_customer_page,
        )
        self.assertEqual("chaptera.commit-accepted.v1", accepted["protocol_version"])
        current = self.kernel.current_revision(DOCUMENT_ID).project
        self.assertEqual(extent(10_000_000, 8_000_000), current["pages"][PAGE_ID]["size"])
        self.assertEqual(self.base["shapes"], current["shapes"])
        self.assertEqual(self.base["stories"], current["stories"])
        self.assertEqual(SOURCE_HASH, current["source_hash"])
        self.assertEqual(1, len(current["operations"]))
        self.assertEqual("keep_objects_fixed", accepted["canonical_operation"]["semantics"])

    def test_external_authority_gate_rejects_master_service_and_unsafe_morph_before_execution(self):
        for reason in ("master_page", "service_page", "tracking_morph_unknown"):
            def reject(_command, reason=reason):
                raise ValueError(reason)

            with self.assertRaisesRegex(ValueError, reason):
                self.kernel.commit_page_extent(
                    request(self.baseline.revision_id, f"page-size-{reason}-0001"),
                    self.executor,
                    pre_execute_validator=reject,
                )
        self.assertEqual(0, self.executor.calls)
        self.assertEqual(self.baseline.revision_id, self.kernel.current_revision(DOCUMENT_ID).revision_id)

    def test_stale_before_state_rejects_without_mutating_base(self):
        bad = command(before=extent(1, 2))
        before = copy.deepcopy(self.base)
        with self.assertRaisesRegex(PageExtentV1Error, "stale_page_extent"):
            self.kernel.commit_page_extent(
                request(self.baseline.revision_id, "page-size-stale-0001", bad),
                self.executor,
                pre_execute_validator=self.admitted_customer_page,
            )
        self.assertEqual(before, self.base)
        self.assertEqual(self.baseline.revision_id, self.kernel.current_revision(DOCUMENT_ID).revision_id)

    def test_noop_nonpositive_overflow_and_extra_fields_fail_before_executor(self):
        noop = command(after=extent(7_315_200, 9_753_600))
        with self.assertRaisesRegex(ValueError, "no-op"):
            self.kernel.commit_page_extent(
                request(self.baseline.revision_id, "page-size-noop-0001", noop),
                self.executor,
            )

        nonpositive = command(after=extent(0, 10))
        with self.assertRaisesRegex(ValueError, "positive"):
            self.kernel.commit_page_extent(
                request(self.baseline.revision_id, "page-size-zero-0001", nonpositive),
                self.executor,
            )

        overflow = command(after=extent(9_007_199_254_740_992, 10))
        with self.assertRaisesRegex(ValueError, "safe EMU"):
            self.kernel.commit_page_extent(
                request(self.baseline.revision_id, "page-size-overflow-0001", overflow),
                self.executor,
            )

        extra = command()
        extra["paper_size"] = "A4"
        with self.assertRaisesRegex(ValueError, "non-intent"):
            self.kernel.commit_page_extent(
                request(self.baseline.revision_id, "page-size-extra-0001", extra),
                self.executor,
            )
        self.assertEqual(0, self.executor.calls)

    def test_exact_retry_and_stale_revision_are_idempotent(self):
        req = request(self.baseline.revision_id, "page-size-retry-0001")
        first = self.kernel.commit_page_extent(
            copy.deepcopy(req), self.executor,
            pre_execute_validator=self.admitted_customer_page,
        )
        retry = self.kernel.commit_page_extent(
            copy.deepcopy(req), self.executor,
            pre_execute_validator=self.admitted_customer_page,
        )
        self.assertEqual(first, retry)
        self.assertEqual(1, self.executor.calls)

        stale = self.kernel.commit_page_extent(
            request(self.baseline.revision_id, "page-size-stale-rev-0001"),
            self.executor,
            pre_execute_validator=self.admitted_customer_page,
        )
        self.assertEqual("stale_revision", stale["code"])
        self.assertEqual(1, self.executor.calls)

    def test_replay_and_undo_redo_restore_exact_extent(self):
        accepted = self.kernel.commit_page_extent(
            request(self.baseline.revision_id, "page-size-history-0001"),
            self.executor,
            pre_execute_validator=self.admitted_customer_page,
        )
        edited = copy.deepcopy(self.kernel.current_revision(DOCUMENT_ID).project)
        canonical = accepted["canonical_operation"]
        replay_command = {
            "kind": "set_page_extent",
            "page_id": canonical["page_id"],
            "expected_before": copy.deepcopy(canonical["before"]),
            "after": copy.deepcopy(canonical["after"]),
            "semantics": canonical["semantics"],
        }
        replay_op, replayed, _ = apply_set_page_extent_v1(copy.deepcopy(self.base), replay_command)
        self.assertEqual(canonical, replay_op)
        self.assertEqual(edited["pages"], replayed["pages"])
        self.assertEqual(edited["shapes"], replayed["shapes"])
        self.assertEqual(edited["stories"], replayed["stories"])

        def history_executor(_base, kind):
            if kind == "undo":
                return copy.deepcopy(self.base), [{"key": "history.undo", "state": "supported", "note": None}]
            if kind == "redo":
                return copy.deepcopy(edited), [{"key": "history.redo", "state": "supported", "note": None}]
            raise ValueError("unexpected history kind")

        undo = self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": accepted["revision_id"],
                "client_operation_id": "page-size-undo-0001",
                "command": {"kind": "undo"},
            },
            history_executor,
        )
        self.assertEqual(self.base["pages"], self.kernel.current_revision(DOCUMENT_ID).project["pages"])

        self.kernel.commit_history_transition(
            {
                "protocol_version": "chaptera.history-transition-intent.v1",
                "document_id": DOCUMENT_ID,
                "source_hash": SOURCE_HASH,
                "base_revision_id": undo["revision_id"],
                "client_operation_id": "page-size-redo-0001",
                "command": {"kind": "redo"},
            },
            history_executor,
        )
        self.assertEqual(edited, self.kernel.current_revision(DOCUMENT_ID).project)


if __name__ == "__main__":
    unittest.main()
