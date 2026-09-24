#!/usr/bin/env python3
import copy
import unittest

from editor_nudge_v1 import (
    AUTHORED_DIRECT_MOVABLE_V1,
    EditorNudgeError,
    SelectionGeometryV1,
    commit_editor_nudge_v1,
    plan_editor_nudge_v1,
)
from nudge_plan_v1 import BASE_NUDGE_EMU, COARSE_NUDGE_EMU
from object_selection_target_v1 import (
    DirectNodeSelectionV1,
    GroupMemberSelectionV1,
    ProjectedInstanceSelectionV1,
)
from revision_store import RevisionKernel

DOC = "10000000-0000-4000-8000-000000000001"
NODE = "30000000-0000-4000-8000-000000000001"
SOURCE = "a" * 64
OP = "90000000-0000-4000-8000-000000000001"
GEOM = SelectionGeometryV1(1000, 2000, 5000, 6000)


def base_args(revision_id):
    return {
        "document_id": DOC,
        "source_hash": SOURCE,
        "base_revision_id": revision_id,
        "client_operation_id": OP,
        "focus_owner": "canvas",
        "selections": (DirectNodeSelectionV1(page_id="page:1", node_id=NODE),),
        "selected_geometry": GEOM,
        "selected_mutation_class": AUTHORED_DIRECT_MOVABLE_V1,
        "direction": "right",
        "modifier_state": "none",
    }


class FakeMoveExecutor:
    def __init__(self):
        self.calls = 0

    def __call__(self, project, command):
        self.calls += 1
        before = copy.deepcopy(project["bounds"][command["node_id"]])
        after = {
            "x": command["x_emu"],
            "y": command["y_emu"],
            "width": before["width"],
            "height": before["height"],
        }
        operation = {
            "kind": "move_node",
            "node_id": command["node_id"],
            "before": before,
            "after": after,
        }
        updated = copy.deepcopy(project)
        updated["bounds"][command["node_id"]] = copy.deepcopy(after)
        updated["operations"] = list(updated["operations"]) + [copy.deepcopy(operation)]
        return operation, updated, [
            {"key": "node.geometry.position", "state": "supported", "note": None}
        ]


class EditorNudgeV1Tests(unittest.TestCase):
    def setUp(self):
        self.kernel = RevisionKernel()
        self.project = {
            "schema_version": "pub-editor-v0.4",
            "source_hash": SOURCE,
            "operations": [],
            "bounds": {
                NODE: {"x": 1000, "y": 2000, "width": 5000, "height": 6000}
            },
        }
        self.baseline = self.kernel.register_baseline(
            document_id=DOC,
            source_hash=SOURCE,
            project=self.project,
        )
        self.executor = FakeMoveExecutor()

    def test_base_arrow_builds_exact_absolute_move_target(self):
        dispatch = plan_editor_nudge_v1(**base_args(self.baseline.revision_id))
        self.assertEqual("commit", dispatch.outcome)
        self.assertEqual(
            1000 + BASE_NUDGE_EMU,
            dispatch.request["command"]["x_emu"],
        )
        self.assertEqual(2000, dispatch.request["command"]["y_emu"])
        self.assertEqual(NODE, dispatch.request["command"]["node_id"])

    def test_coarse_step_reuses_shared_nudge_plan(self):
        args = base_args(self.baseline.revision_id)
        args["direction"] = "up"
        args["modifier_state"] = "coarse"
        dispatch = plan_editor_nudge_v1(**args)
        self.assertEqual("commit", dispatch.outcome)
        self.assertEqual(1000, dispatch.request["command"]["x_emu"])
        self.assertEqual(2000 - COARSE_NUDGE_EMU, dispatch.request["command"]["y_emu"])

    def test_each_accepted_key_is_exactly_one_revision_commit(self):
        result = commit_editor_nudge_v1(
            kernel=self.kernel,
            executor=self.executor,
            **base_args(self.baseline.revision_id),
        )
        self.assertEqual("chaptera.commit-accepted.v1", result["protocol_version"])
        self.assertEqual(1, self.executor.calls)
        self.assertEqual(
            1000 + BASE_NUDGE_EMU,
            result["canonical_operation"]["after"]["x"],
        )

    def test_story_focus_routes_arrows_without_move(self):
        args = base_args(self.baseline.revision_id)
        args["focus_owner"] = "story"
        result = commit_editor_nudge_v1(
            kernel=self.kernel,
            executor=self.executor,
            **args,
        )
        self.assertEqual("route_story", result.outcome)
        self.assertEqual(0, self.executor.calls)

    def test_non_canvas_focus_and_multi_selection_commit_nothing(self):
        args = base_args(self.baseline.revision_id)
        args["focus_owner"] = "other"
        self.assertEqual("ignored", plan_editor_nudge_v1(**args).outcome)

        args = base_args(self.baseline.revision_id)
        args["selections"] = (
            DirectNodeSelectionV1(page_id="page:1", node_id=NODE),
            DirectNodeSelectionV1(page_id="page:1", node_id="node:2"),
        )
        self.assertEqual("single_selection_required", plan_editor_nudge_v1(**args).reason)

    def test_projected_and_group_member_targets_are_not_admitted(self):
        cases = [
            ProjectedInstanceSelectionV1(
                page_id="page:1",
                instance_id="inst:1",
                origin_node_id=NODE,
                projection_kind="master",
                mutation_class="read_only",
            ),
            GroupMemberSelectionV1(
                page_id="page:1",
                root_group_id="group:1",
                member_node_id=NODE,
            ),
        ]
        for target in cases:
            args = base_args(self.baseline.revision_id)
            args["selections"] = (target,)
            with self.subTest(target=target):
                self.assertEqual(
                    "direct_node_selection_required",
                    plan_editor_nudge_v1(**args).reason,
                )

    def test_direct_but_non_authored_or_non_movable_target_is_rejected(self):
        args = base_args(self.baseline.revision_id)
        args["selected_mutation_class"] = "source-backed-direct"
        result = plan_editor_nudge_v1(**args)
        self.assertEqual("ignored", result.outcome)
        self.assertEqual("authored_direct_movable_target_required", result.reason)

    def test_unsupported_modifier_or_direction_commits_nothing(self):
        for key, value in (("modifier_state", "alt"), ("direction", "diagonal")):
            args = base_args(self.baseline.revision_id)
            args[key] = value
            result = commit_editor_nudge_v1(
                kernel=self.kernel,
                executor=self.executor,
                **args,
            )
            self.assertEqual("ignored", result.outcome)
        self.assertEqual(0, self.executor.calls)

    def test_overflow_fails_before_kernel_commit(self):
        args = base_args(self.baseline.revision_id)
        args["selected_geometry"] = SelectionGeometryV1(
            9_007_199_254_740_900,
            0,
            50,
            50,
        )
        with self.assertRaisesRegex(EditorNudgeError, "JavaScript-safe"):
            plan_editor_nudge_v1(**args)
        self.assertEqual(0, self.executor.calls)


if __name__ == "__main__":
    unittest.main()
