import unittest

from authored_group_geometry_v1 import RectEmu
from group_transform_chain_v1 import AuthoredGroupEdgeV1, GroupPointEmu
from nested_group_selection_v1 import (
    NestedGroupPathEdgeV1,
    NestedGroupPathSnapshotV1,
    NestedGroupSelectionScopeV1,
    NestedGroupSelectionTargetV1,
)
from nested_multi_geometry_v1 import (
    NestedMultiGeometryError,
    NestedMultiMemberV1,
    plan_nested_multi_move_v1,
    plan_nested_multi_resize_v1,
)


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


def selected_scope(path, node_ids, page_id="page:1"):
    targets = tuple(
        NestedGroupSelectionTargetV1(page_id, tuple(path), node_id)
        for node_id in node_ids
    )
    return NestedGroupSelectionScopeV1(
        page_id=page_id,
        container_path=tuple(path),
        selected=targets,
        primary=targets[0] if len(targets) == 1 else None,
    )


class NestedMultiGeometryV1Tests(unittest.TestCase):
    def setUp(self):
        self.scope = selected_scope(("g0",), ("a", "b"))
        self.snapshot = NestedGroupPathSnapshotV1(
            page_id="page:1",
            edges=(path_edge("g0", None, ("a", "b", "s")),),
        )
        self.ancestry = (
            transform_edge(
                "g0",
                "page:1",
                None,
                ("a", "b", "s"),
                RectEmu(100, 100, 200, 200),
                RectEmu(0, 0, 100, 100),
            ),
        )
        self.members = (
            NestedMultiMemberV1("b", RectEmu(50, 40, 20, 20)),
            NestedMultiMemberV1("a", RectEmu(10, 10, 20, 20)),
        )

    def test_move_maps_page_gesture_once_and_applies_identical_local_delta(self):
        plan = plan_nested_multi_move_v1(
            scope=self.scope,
            selection_snapshot=self.snapshot,
            ancestry=self.ancestry,
            members=self.members,
            gesture_base_page_point=GroupPointEmu(100, 100),
            gesture_current_page_point=GroupPointEmu(120, 110),
        )
        self.assertEqual("move", plan.kind)
        self.assertEqual((10, 5), plan.local_translation)
        self.assertEqual(RectEmu(10, 10, 60, 50), plan.base_local_aggregate)
        self.assertEqual(RectEmu(20, 15, 60, 50), plan.target_local_aggregate)

        by_id = {member.node_id: member for member in plan.members}
        self.assertEqual(RectEmu(10, 10, 20, 20), by_id["a"].before_local)
        self.assertEqual(RectEmu(20, 15, 20, 20), by_id["a"].after_local)
        self.assertEqual(RectEmu(50, 40, 20, 20), by_id["b"].before_local)
        self.assertEqual(RectEmu(60, 45, 20, 20), by_id["b"].after_local)

        for result in plan.members:
            self.assertEqual(
                result.before_local.width,
                result.after_local.width,
            )
            self.assertEqual(
                result.before_local.height,
                result.after_local.height,
            )
            self.assertEqual(
                result.after_local.x - result.before_local.x,
                10,
            )
            self.assertEqual(
                result.after_local.y - result.before_local.y,
                5,
            )

        self.assertEqual(RectEmu(120, 120, 40, 40), by_id["a"].before_page)
        self.assertEqual(RectEmu(140, 130, 40, 40), by_id["a"].after_page)

    def test_move_through_two_group_boundaries_uses_current_group_local_space(self):
        scope = selected_scope(("g0", "g1"), ("a", "b"))
        snap = NestedGroupPathSnapshotV1(
            page_id="page:1",
            edges=(
                path_edge("g0", None, ("g1",)),
                path_edge("g1", "g0", ("a", "b")),
            ),
        )
        ancestry = (
            transform_edge(
                "g0",
                "page:1",
                None,
                ("g1",),
                RectEmu(0, 0, 200, 200),
                RectEmu(0, 0, 100, 100),
            ),
            transform_edge(
                "g1",
                "page:1",
                "g0",
                ("a", "b"),
                RectEmu(20, 20, 60, 60),
                RectEmu(0, 0, 120, 120),
            ),
        )
        members = (
            NestedMultiMemberV1("a", RectEmu(10, 10, 20, 20)),
            NestedMultiMemberV1("b", RectEmu(60, 60, 20, 20)),
        )
        plan = plan_nested_multi_move_v1(
            scope=scope,
            selection_snapshot=snap,
            ancestry=ancestry,
            members=members,
            gesture_base_page_point=GroupPointEmu(40, 40),
            gesture_current_page_point=GroupPointEmu(60, 50),
        )
        self.assertEqual((20, 10), plan.local_translation)
        by_id = {member.node_id: member for member in plan.members}
        self.assertEqual(RectEmu(30, 20, 20, 20), by_id["a"].after_local)
        self.assertEqual(RectEmu(80, 70, 20, 20), by_id["b"].after_local)

    def test_resize_delegates_one_local_aggregate_to_multi_resize(self):
        plan = plan_nested_multi_resize_v1(
            scope=self.scope,
            selection_snapshot=self.snapshot,
            ancestry=self.ancestry,
            members=self.members,
            handle="e",
            target_aggregate_page_rect=RectEmu(120, 120, 160, 100),
        )
        self.assertEqual("resize", plan.kind)
        self.assertEqual("e", plan.resize_handle)
        self.assertEqual(RectEmu(10, 10, 60, 50), plan.base_local_aggregate)
        self.assertEqual(RectEmu(10, 10, 80, 50), plan.target_local_aggregate)
        self.assertEqual(RectEmu(120, 120, 160, 100), plan.target_page_aggregate)

        by_id = {member.node_id: member for member in plan.members}
        self.assertEqual(RectEmu(10, 10, 27, 20), by_id["a"].after_local)
        self.assertEqual(RectEmu(63, 40, 27, 20), by_id["b"].after_local)
        self.assertEqual(RectEmu(120, 120, 54, 40), by_id["a"].after_page)
        self.assertEqual(RectEmu(226, 180, 54, 40), by_id["b"].after_page)

    def test_non_exact_page_point_rejects_move_instead_of_guessing(self):
        scope = selected_scope(("g0",), ("a", "b"))
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
                RectEmu(0, 0, 3, 3),
                RectEmu(0, 0, 2, 2),
            ),
        )
        members = (
            NestedMultiMemberV1("a", RectEmu(0, 0, 1, 1)),
            NestedMultiMemberV1("b", RectEmu(1, 1, 1, 1)),
        )
        with self.assertRaisesRegex(NestedMultiGeometryError, "not exactly representable"):
            plan_nested_multi_move_v1(
                scope=scope,
                selection_snapshot=snap,
                ancestry=ancestry,
                members=members,
                gesture_base_page_point=GroupPointEmu(0, 0),
                gesture_current_page_point=GroupPointEmu(1, 0),
            )

    def test_non_exact_page_aggregate_rejects_resize(self):
        scope = selected_scope(("g0",), ("a", "b"))
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
                RectEmu(0, 0, 3, 3),
                RectEmu(0, 0, 2, 2),
            ),
        )
        members = (
            NestedMultiMemberV1("a", RectEmu(0, 0, 1, 1)),
            NestedMultiMemberV1("b", RectEmu(1, 1, 1, 1)),
        )
        with self.assertRaisesRegex(NestedMultiGeometryError, "not exactly representable"):
            plan_nested_multi_resize_v1(
                scope=scope,
                selection_snapshot=snap,
                ancestry=ancestry,
                members=members,
                handle="w",
                target_aggregate_page_rect=RectEmu(1, 0, 2, 3),
            )

    def test_move_cannot_escape_current_group_without_refit(self):
        with self.assertRaisesRegex(NestedMultiGeometryError, "exceeds local coordinate space"):
            plan_nested_multi_move_v1(
                scope=self.scope,
                selection_snapshot=self.snapshot,
                ancestry=self.ancestry,
                members=self.members,
                gesture_base_page_point=GroupPointEmu(100, 100),
                gesture_current_page_point=GroupPointEmu(180, 180),
            )

    def test_broken_path_and_mixed_member_snapshot_fail_closed(self):
        broken_snapshot = NestedGroupPathSnapshotV1(
            page_id="page:1",
            edges=(path_edge("wrong", None, ("a", "b")),),
        )
        with self.assertRaises(NestedMultiGeometryError):
            plan_nested_multi_move_v1(
                scope=self.scope,
                selection_snapshot=broken_snapshot,
                ancestry=self.ancestry,
                members=self.members,
                gesture_base_page_point=GroupPointEmu(100, 100),
                gesture_current_page_point=GroupPointEmu(120, 100),
            )

        missing = (NestedMultiMemberV1("a", RectEmu(10, 10, 20, 20)),)
        with self.assertRaisesRegex(NestedMultiGeometryError, "at least two"):
            plan_nested_multi_move_v1(
                scope=self.scope,
                selection_snapshot=self.snapshot,
                ancestry=self.ancestry,
                members=missing,
                gesture_base_page_point=GroupPointEmu(100, 100),
                gesture_current_page_point=GroupPointEmu(120, 100),
            )

    def test_unsupported_member_and_noop_move_fail_closed(self):
        unsupported = (
            NestedMultiMemberV1("a", RectEmu(10, 10, 20, 20)),
            NestedMultiMemberV1(
                "b",
                RectEmu(50, 40, 20, 20),
                provenance="source-backed",
            ),
        )
        with self.assertRaisesRegex(NestedMultiGeometryError, "unsupported provenance"):
            plan_nested_multi_move_v1(
                scope=self.scope,
                selection_snapshot=self.snapshot,
                ancestry=self.ancestry,
                members=unsupported,
                gesture_base_page_point=GroupPointEmu(100, 100),
                gesture_current_page_point=GroupPointEmu(120, 100),
            )

        with self.assertRaisesRegex(NestedMultiGeometryError, "no-op"):
            plan_nested_multi_move_v1(
                scope=self.scope,
                selection_snapshot=self.snapshot,
                ancestry=self.ancestry,
                members=self.members,
                gesture_base_page_point=GroupPointEmu(100, 100),
                gesture_current_page_point=GroupPointEmu(100, 100),
            )


if __name__ == "__main__":
    unittest.main()
