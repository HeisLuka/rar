#!/usr/bin/env python3
import unittest

from group_child_reorder_v1 import (
    GroupChildReorderError,
    GroupChildSnapshotV1,
    plan_reorder_group_children_set_v1,
)


def snaps(parent, children):
    return tuple(GroupChildSnapshotV1(node_id=node, parent_group_id=parent) for node in children)


class GroupChildReorderV1Tests(unittest.TestCase):
    def test_front_and_back_ignore_caller_selection_order(self):
        lane = ("a", "b", "c", "d", "e")
        front = plan_reorder_group_children_set_v1(
            parent_group_id="g",
            expected_children=lane,
            current_children=lane,
            child_snapshots=snaps("g", lane),
            selected_node_ids=("d", "b"),
            mode="bring_to_front",
        )
        back = plan_reorder_group_children_set_v1(
            parent_group_id="g",
            expected_children=lane,
            current_children=lane,
            child_snapshots=snaps("g", lane),
            selected_node_ids=("d", "b"),
            mode="send_to_back",
        )
        self.assertEqual(("a", "c", "e", "b", "d"), front.after_children)
        self.assertEqual(("b", "d", "a", "c", "e"), back.after_children)
        self.assertEqual(("b", "d"), front.selected_in_lane_order)

    def test_one_step_preserves_selected_and_unselected_relative_order(self):
        lane = ("a", "b", "c", "d", "e")
        forward = plan_reorder_group_children_set_v1(
            parent_group_id="g",
            expected_children=lane,
            current_children=lane,
            child_snapshots=snaps("g", lane),
            selected_node_ids=("b", "d"),
            mode="bring_forward_one",
        )
        backward = plan_reorder_group_children_set_v1(
            parent_group_id="g",
            expected_children=lane,
            current_children=lane,
            child_snapshots=snaps("g", lane),
            selected_node_ids=("b", "d"),
            mode="send_backward_one",
        )
        self.assertEqual(("a", "c", "b", "e", "d"), forward.after_children)
        self.assertEqual(("b", "a", "d", "c", "e"), backward.after_children)

    def test_contiguous_selected_block_moves_as_stable_block(self):
        lane = ("a", "b", "c", "d", "e")
        plan = plan_reorder_group_children_set_v1(
            parent_group_id="g",
            expected_children=lane,
            current_children=lane,
            child_snapshots=snaps("g", lane),
            selected_node_ids=("b", "c"),
            mode="bring_forward_one",
        )
        self.assertEqual(("a", "d", "b", "c", "e"), plan.after_children)

    def test_boundary_noop_is_explicit(self):
        lane = ("a", "b", "c")
        plan = plan_reorder_group_children_set_v1(
            parent_group_id="g",
            expected_children=lane,
            current_children=lane,
            child_snapshots=snaps("g", lane),
            selected_node_ids=("c",),
            mode="bring_forward_one",
        )
        self.assertEqual("no_change", plan.status)
        self.assertEqual(lane, plan.after_children)

    def test_stale_wrong_parent_imported_and_membership_mismatch_fail_closed(self):
        lane = ("a", "b", "c")
        with self.assertRaisesRegex(GroupChildReorderError, "stale"):
            plan_reorder_group_children_set_v1(
                parent_group_id="g",
                expected_children=lane,
                current_children=("a", "c", "b"),
                child_snapshots=snaps("g", ("a", "c", "b")),
                selected_node_ids=("b",),
                mode="bring_to_front",
            )
        with self.assertRaisesRegex(GroupChildReorderError, "mixed-parent"):
            plan_reorder_group_children_set_v1(
                parent_group_id="g",
                expected_children=lane,
                current_children=lane,
                child_snapshots=(
                    GroupChildSnapshotV1("a", "g"),
                    GroupChildSnapshotV1("b", "other"),
                    GroupChildSnapshotV1("c", "g"),
                ),
                selected_node_ids=("b",),
                mode="bring_to_front",
            )
        with self.assertRaisesRegex(GroupChildReorderError, "source-backed"):
            plan_reorder_group_children_set_v1(
                parent_group_id="g",
                expected_children=lane,
                current_children=lane,
                child_snapshots=(
                    GroupChildSnapshotV1("a", "g"),
                    GroupChildSnapshotV1("b", "g", provenance="source_backed"),
                    GroupChildSnapshotV1("c", "g"),
                ),
                selected_node_ids=("b",),
                mode="bring_to_front",
            )
        with self.assertRaisesRegex(GroupChildReorderError, "membership"):
            plan_reorder_group_children_set_v1(
                parent_group_id="g",
                expected_children=lane,
                current_children=lane,
                child_snapshots=snaps("g", ("a", "b")),
                selected_node_ids=("b",),
                mode="bring_to_front",
            )


if __name__ == "__main__":
    unittest.main()
