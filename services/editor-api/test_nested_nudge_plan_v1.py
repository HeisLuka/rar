import unittest

import nested_nudge_plan_v1 as nested_nudge
from authored_group_geometry_v1 import RectEmu
from group_transform_chain_v1 import AuthoredGroupEdgeV1
from nested_group_selection_v1 import (
    NestedGroupPathEdgeV1,
    NestedGroupPathSnapshotV1,
    NestedGroupSelectionScopeV1,
    NestedGroupSelectionTargetV1,
)
from nested_multi_geometry_v1 import NestedMultiMemberV1
from nudge_plan_v1 import BASE_NUDGE_EMU, COARSE_NUDGE_EMU


def path_edge(group_id, parent, children):
    return NestedGroupPathEdgeV1(
        group_id=group_id,
        parent_group_id=parent,
        children=tuple(children),
    )


def transform_edge(group_id, page_id, parent, children, bounds, local):
    return AuthoredGroupEdgeV1(
        group_id=group_id,
        page_id=page_id,
        parent_group_id=parent,
        children=tuple(children),
        bounds_in_parent=bounds,
        local_coordinate_space=local,
    )


def scope_for(path, node_ids, page_id="page:1"):
    selected = tuple(
        NestedGroupSelectionTargetV1(page_id, tuple(path), node_id)
        for node_id in node_ids
    )
    return NestedGroupSelectionScopeV1(
        page_id=page_id,
        container_path=tuple(path),
        selected=selected,
        primary=None,
    )


class NestedNudgePlanV1Tests(unittest.TestCase):
    def setUp(self):
        self.scope = scope_for(("g0",), ("a", "b"))
        self.snapshot = NestedGroupPathSnapshotV1(
            page_id="page:1",
            edges=(path_edge("g0", None, ("a", "b")),),
        )
        self.ancestry = (
            transform_edge(
                "g0",
                "page:1",
                None,
                ("a", "b"),
                RectEmu(0, 0, 2_000_000, 2_000_000),
                RectEmu(0, 0, 1_000_000, 1_000_000),
            ),
        )
        self.members = (
            NestedMultiMemberV1("b", RectEmu(300_000, 200_000, 50_000, 60_000)),
            NestedMultiMemberV1("a", RectEmu(100_000, 100_000, 40_000, 50_000)),
        )

    def test_base_right_nudge_uses_one_local_vector_for_every_member(self):
        plan = nested_nudge.plan_nested_nudge_v1(
            scope=self.scope,
            selection_snapshot=self.snapshot,
            ancestry=self.ancestry,
            members=self.members,
            direction="right",
            modifier_state="none",
        )
        self.assertEqual("planned", plan.status)
        self.assertEqual((BASE_NUDGE_EMU, 0), plan.requested_page_delta)
        self.assertEqual((BASE_NUDGE_EMU // 2, 0), plan.local_translation)
        self.assertEqual(["a", "b"], [m.node_id for m in plan.members])

        for result in plan.members:
            self.assertEqual(
                BASE_NUDGE_EMU // 2,
                result.after_local.x - result.before_local.x,
            )
            self.assertEqual(
                0,
                result.after_local.y - result.before_local.y,
            )
            self.assertEqual(
                result.before_page.x + BASE_NUDGE_EMU,
                result.after_page.x,
            )
            self.assertEqual(result.before_page.y, result.after_page.y)
            self.assertEqual(result.before_page.width, result.after_page.width)
            self.assertEqual(result.before_page.height, result.after_page.height)

    def test_coarse_up_nudge_is_document_space_not_local_scale(self):
        plan = nested_nudge.plan_nested_nudge_v1(
            scope=self.scope,
            selection_snapshot=self.snapshot,
            ancestry=self.ancestry,
            members=self.members,
            direction="up",
            modifier_state="coarse",
        )
        self.assertEqual("planned", plan.status)
        self.assertEqual((0, -COARSE_NUDGE_EMU), plan.requested_page_delta)
        self.assertEqual((0, -(COARSE_NUDGE_EMU // 2)), plan.local_translation)
        for result in plan.members:
            self.assertEqual(
                result.before_page.y - COARSE_NUDGE_EMU,
                result.after_page.y,
            )

    def test_nudge_policy_is_called_exactly_once(self):
        original = nested_nudge.nudge_plan_v1.plan_nudge_v1
        calls = []

        def counted(**kwargs):
            calls.append(kwargs)
            return original(**kwargs)

        nested_nudge.nudge_plan_v1.plan_nudge_v1 = counted
        try:
            plan = nested_nudge.plan_nested_nudge_v1(
                scope=self.scope,
                selection_snapshot=self.snapshot,
                ancestry=self.ancestry,
                members=self.members,
                direction="left",
                modifier_state="none",
            )
        finally:
            nested_nudge.nudge_plan_v1.plan_nudge_v1 = original

        self.assertEqual("planned", plan.status)
        self.assertEqual(
            [{"direction": "left", "modifier_state": "none"}],
            calls,
        )

    def test_anchor_can_be_exact_while_second_member_proves_no_common_vector(self):
        # Scale 5/3. For D=118872, anchor x=0 admits local delta 71323:
        # round(71323*5/3)=118872. But at x=1 the same local delta moves
        # the page edge by 118871 because the rounding phase differs.
        scope = scope_for(("g0",), ("a", "b"))
        snap = NestedGroupPathSnapshotV1(
            page_id="page:1",
            edges=(path_edge("g0", None, ("a", "b")),),
        )
        ancestry = (
            transform_edge(
                "g0",
                "page:1",
                None,
                ("a", "b"),
                RectEmu(0, 0, 500_000, 300_000),
                RectEmu(0, 0, 300_000, 300_000),
            ),
        )
        members = (
            NestedMultiMemberV1("a", RectEmu(0, 10, 10, 10)),
            NestedMultiMemberV1("b", RectEmu(1, 40, 10, 10)),
        )

        plan = nested_nudge.plan_nested_nudge_v1(
            scope=scope,
            selection_snapshot=snap,
            ancestry=ancestry,
            members=members,
            direction="right",
            modifier_state="none",
        )
        self.assertEqual("not_exactly_representable", plan.status)
        self.assertEqual(
            "single_local_vector_does_not_translate_every_member_exactly",
            plan.reason,
        )
        self.assertIsNone(plan.local_translation)
        self.assertEqual((), plan.members)

    def test_non_exact_anchor_translation_fails_before_member_specific_fallback(self):
        scope = scope_for(("g0",), ("a", "b"))
        snap = NestedGroupPathSnapshotV1(
            page_id="page:1",
            edges=(path_edge("g0", None, ("a", "b")),),
        )
        ancestry = (
            transform_edge(
                "g0",
                "page:1",
                None,
                ("a", "b"),
                RectEmu(0, 0, 7, 100_000),
                RectEmu(0, 0, 3, 100_000),
            ),
        )
        members = (
            NestedMultiMemberV1("a", RectEmu(0, 10, 1, 10)),
            NestedMultiMemberV1("b", RectEmu(2, 40, 1, 10)),
        )
        plan = nested_nudge.plan_nested_nudge_v1(
            scope=scope,
            selection_snapshot=snap,
            ancestry=ancestry,
            members=members,
            direction="right",
            modifier_state="none",
        )
        self.assertEqual("not_exactly_representable", plan.status)
        self.assertEqual(
            "anchor_translation_not_exactly_representable",
            plan.reason,
        )

    def test_translation_that_leaves_current_container_fails_without_refit(self):
        scope = scope_for(("g0",), ("a", "b"))
        snap = NestedGroupPathSnapshotV1(
            page_id="page:1",
            edges=(path_edge("g0", None, ("a", "b")),),
        )
        ancestry = (
            transform_edge(
                "g0",
                "page:1",
                None,
                ("a", "b"),
                RectEmu(0, 0, 2_000_000, 2_000_000),
                RectEmu(0, 0, 2_000_000, 2_000_000),
            ),
        )
        members = (
            NestedMultiMemberV1("a", RectEmu(1_900_000, 10, 50_000, 10)),
            NestedMultiMemberV1("b", RectEmu(1_800_000, 40, 50_000, 10)),
        )
        plan = nested_nudge.plan_nested_nudge_v1(
            scope=scope,
            selection_snapshot=snap,
            ancestry=ancestry,
            members=members,
            direction="right",
            modifier_state="coarse",
        )
        self.assertEqual("not_exactly_representable", plan.status)
        self.assertEqual("translated_member_exits_current_container", plan.reason)

    def test_invalid_policy_modifier_is_explicit_error_not_not_exact(self):
        with self.assertRaisesRegex(nested_nudge.NestedNudgePlanError, "modifier"):
            nested_nudge.plan_nested_nudge_v1(
                scope=self.scope,
                selection_snapshot=self.snapshot,
                ancestry=self.ancestry,
                members=self.members,
                direction="right",
                modifier_state="shift",
            )

    def test_selected_geometry_mismatch_fails_before_nudge_policy(self):
        with self.assertRaises(nested_nudge.NestedNudgePlanError):
            nested_nudge.plan_nested_nudge_v1(
                scope=self.scope,
                selection_snapshot=self.snapshot,
                ancestry=self.ancestry,
                members=(self.members[0],),
                direction="right",
                modifier_state="none",
            )


if __name__ == "__main__":
    unittest.main()
