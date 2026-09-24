#!/usr/bin/env python3
import unittest

from group_child_batch_lifecycle_v1 import (
    GroupChildBatchLifecycleError,
    GroupChildCandidateV1,
    plan_group_child_batch_insert_v1,
    plan_group_child_batch_remove_v1,
    restore_group_child_batch_removal_v1,
)


def candidates(parent, *ids):
    return tuple(GroupChildCandidateV1(node_id=node_id, parent_group_id=parent) for node_id in ids)


class GroupChildBatchLifecycleV1Tests(unittest.TestCase):
    def test_append_block_at_front_preserves_explicit_candidate_order(self):
        lane = ("a", "b", "c")
        plan = plan_group_child_batch_insert_v1(
            parent_group_id="g",
            expected_children=lane,
            current_children=lane,
            candidates=candidates("g", "n2", "n1"),
            policy="append_block_at_front",
        )
        self.assertEqual(("a", "b", "c", "n2", "n1"), plan.after_children)
        self.assertEqual(("n2", "n1"), plan.inserted_node_ids)

    def test_insert_block_after_explicit_anchor(self):
        lane = ("a", "b", "c")
        plan = plan_group_child_batch_insert_v1(
            parent_group_id="g",
            expected_children=lane,
            current_children=lane,
            candidates=candidates("g", "x", "y"),
            policy="insert_block_after_anchor",
            anchor_node_id="b",
        )
        self.assertEqual(("a", "b", "x", "y", "c"), plan.after_children)

    def test_remove_is_simultaneous_and_inverse_restores_exact_lane(self):
        lane = ("a", "b", "c", "d", "e")
        plan = plan_group_child_batch_remove_v1(
            parent_group_id="g",
            expected_children=lane,
            current_children=lane,
            candidates=candidates("g", "d", "b"),
        )
        self.assertEqual(("a", "c", "e"), plan.after_children)
        self.assertEqual(("b", "d"), plan.removed_node_ids)
        self.assertEqual((1, 3), plan.removed_indices)
        self.assertEqual(lane, restore_group_child_batch_removal_v1(plan))

    def test_stale_duplicate_membership_wrong_parent_source_backed_and_bad_anchor_fail_closed(self):
        lane = ("a", "b")
        with self.assertRaisesRegex(GroupChildBatchLifecycleError, "stale"):
            plan_group_child_batch_insert_v1(
                parent_group_id="g",
                expected_children=lane,
                current_children=("b", "a"),
                candidates=candidates("g", "x"),
                policy="append_block_at_front",
            )
        with self.assertRaisesRegex(GroupChildBatchLifecycleError, "already belongs"):
            plan_group_child_batch_insert_v1(
                parent_group_id="g",
                expected_children=lane,
                current_children=lane,
                candidates=candidates("g", "b"),
                policy="append_block_at_front",
            )
        with self.assertRaisesRegex(GroupChildBatchLifecycleError, "wrong parent"):
            plan_group_child_batch_insert_v1(
                parent_group_id="g",
                expected_children=lane,
                current_children=lane,
                candidates=candidates("other", "x"),
                policy="append_block_at_front",
            )
        with self.assertRaisesRegex(GroupChildBatchLifecycleError, "source-backed"):
            plan_group_child_batch_insert_v1(
                parent_group_id="g",
                expected_children=lane,
                current_children=lane,
                candidates=(GroupChildCandidateV1("x", "g", provenance="source_backed"),),
                policy="append_block_at_front",
            )
        with self.assertRaisesRegex(GroupChildBatchLifecycleError, "anchor"):
            plan_group_child_batch_insert_v1(
                parent_group_id="g",
                expected_children=lane,
                current_children=lane,
                candidates=candidates("g", "x"),
                policy="insert_block_after_anchor",
                anchor_node_id="missing",
            )

    def test_remove_preserves_unselected_order_and_rejects_missing_candidate(self):
        lane = ("a", "b", "c", "d")
        plan = plan_group_child_batch_remove_v1(
            parent_group_id="g",
            expected_children=lane,
            current_children=lane,
            candidates=candidates("g", "c", "a"),
        )
        self.assertEqual(("b", "d"), plan.after_children)
        with self.assertRaisesRegex(GroupChildBatchLifecycleError, "not a current direct child"):
            plan_group_child_batch_remove_v1(
                parent_group_id="g",
                expected_children=lane,
                current_children=lane,
                candidates=candidates("g", "x"),
            )


if __name__ == "__main__":
    unittest.main()
