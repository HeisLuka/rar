import unittest

from group_member_selection_v1 import TopLevelSelectionScopeV1, empty_top_level_scope_v1
from nested_group_selection_v1 import (
    NestedGroupPathEdgeV1,
    NestedGroupPathSnapshotV1,
    NestedGroupSelectionError,
    NestedGroupSelectionScopeV1,
    NestedGroupSelectionTargetV1,
    compose_nested_group_selection_v1,
    enter_root_group_scope_v1,
    enter_selected_child_group_v1,
    escape_parent_nested_v1,
    nested_scope_on_page_change_v1,
    reconcile_nested_group_scope_v1,
    validate_nested_path_snapshot_v1,
)
from object_selection_target_v1 import DirectNodeSelectionV1


def edge(group_id, parent, children):
    return NestedGroupPathEdgeV1(
        group_id=group_id,
        parent_group_id=parent,
        children=tuple(children),
    )


def snapshot(*edges, page_id="page:1"):
    return NestedGroupPathSnapshotV1(page_id=page_id, edges=tuple(edges))


def target(path, node_id, page_id="page:1"):
    return NestedGroupSelectionTargetV1(page_id, tuple(path), node_id)


class NestedGroupSelectionV1Tests(unittest.TestCase):
    def setUp(self):
        self.root = DirectNodeSelectionV1("page:1", "g0")
        self.s1 = snapshot(edge("g0", None, ("g1", "a")))
        self.s2 = snapshot(
            edge("g0", None, ("g1", "a")),
            edge("g1", "g0", ("b", "g2")),
        )
        self.s3 = snapshot(
            edge("g0", None, ("g1", "a")),
            edge("g1", "g0", ("b", "g2")),
            edge("g2", "g1", ("c", "d")),
        )

    def test_enter_root_creates_empty_path_scope(self):
        scope = enter_root_group_scope_v1(
            root_group=self.root,
            snapshot=self.s1,
        )
        self.assertEqual(("g0",), scope.container_path)
        self.assertEqual((), scope.selected)
        self.assertIsNone(scope.primary)

    def test_select_child_group_then_enter_appends_exact_path(self):
        root_scope = enter_root_group_scope_v1(
            root_group=self.root,
            snapshot=self.s1,
        )
        selected = compose_nested_group_selection_v1(
            scope=root_scope,
            snapshot=self.s1,
            candidates=(target(("g0",), "g1"),),
            mode="replace",
        )
        nested = enter_selected_child_group_v1(
            scope=selected,
            current_snapshot=self.s1,
            child_group_id="g1",
            child_snapshot=self.s2,
        )
        self.assertEqual(("g0", "g1"), nested.container_path)
        self.assertEqual((), nested.selected)

    def test_set_algebra_is_limited_to_direct_siblings_same_path(self):
        scope = NestedGroupSelectionScopeV1(
            page_id="page:1",
            container_path=("g0", "g1"),
            selected=(target(("g0", "g1"), "b"),),
            primary=target(("g0", "g1"), "b"),
        )
        added = compose_nested_group_selection_v1(
            scope=scope,
            snapshot=self.s2,
            candidates=(target(("g0", "g1"), "g2"),),
            mode="add",
        )
        self.assertEqual(
            {target(("g0", "g1"), "b"), target(("g0", "g1"), "g2")},
            set(added.selected),
        )
        self.assertEqual(target(("g0", "g1"), "b"), added.primary)

        toggled = compose_nested_group_selection_v1(
            scope=added,
            snapshot=self.s2,
            candidates=(target(("g0", "g1"), "b"),),
            mode="toggle",
        )
        self.assertEqual((target(("g0", "g1"), "g2"),), toggled.selected)
        self.assertEqual(target(("g0", "g1"), "g2"), toggled.primary)

    def test_cross_path_or_non_child_candidate_rejects(self):
        scope = NestedGroupSelectionScopeV1(
            page_id="page:1",
            container_path=("g0", "g1"),
            selected=(),
            primary=None,
        )
        with self.assertRaisesRegex(NestedGroupSelectionError, "exact container path"):
            compose_nested_group_selection_v1(
                scope=scope,
                snapshot=self.s2,
                candidates=(target(("g0",), "a"),),
                mode="add",
            )
        with self.assertRaisesRegex(NestedGroupSelectionError, "not a current direct child"):
            compose_nested_group_selection_v1(
                scope=scope,
                snapshot=self.s2,
                candidates=(target(("g0", "g1"), "missing"),),
                mode="add",
            )

    def test_escape_pops_exactly_one_level_and_selects_exited_group(self):
        scope = NestedGroupSelectionScopeV1(
            page_id="page:1",
            container_path=("g0", "g1", "g2"),
            selected=(target(("g0", "g1", "g2"), "c"),),
            primary=target(("g0", "g1", "g2"), "c"),
        )
        parent = escape_parent_nested_v1(scope=scope, snapshot=self.s3)
        self.assertIsInstance(parent, NestedGroupSelectionScopeV1)
        self.assertEqual(("g0", "g1"), parent.container_path)
        self.assertEqual((target(("g0", "g1"), "g2"),), parent.selected)
        self.assertEqual(target(("g0", "g1"), "g2"), parent.primary)

    def test_escape_outermost_returns_top_level_root(self):
        scope = NestedGroupSelectionScopeV1(
            page_id="page:1",
            container_path=("g0",),
            selected=(target(("g0",), "a"),),
            primary=target(("g0",), "a"),
        )
        top = escape_parent_nested_v1(scope=scope, snapshot=self.s1)
        self.assertIsInstance(top, TopLevelSelectionScopeV1)
        self.assertEqual((self.root,), top.selected)
        self.assertEqual(self.root, top.primary)

    def test_broken_path_escape_and_reconcile_clear_instead_of_shortening(self):
        scope = NestedGroupSelectionScopeV1(
            page_id="page:1",
            container_path=("g0", "g1"),
            selected=(target(("g0", "g1"), "b"),),
            primary=target(("g0", "g1"), "b"),
        )
        broken = snapshot(
            edge("g0", None, ("other", "a")),
            edge("g1", "g0", ("b", "g2")),
        )
        self.assertEqual(
            empty_top_level_scope_v1(),
            escape_parent_nested_v1(scope=scope, snapshot=broken),
        )
        self.assertEqual(
            empty_top_level_scope_v1(),
            reconcile_nested_group_scope_v1(scope=scope, snapshot=broken),
        )

    def test_deleted_selected_child_invalidates_to_empty(self):
        scope = NestedGroupSelectionScopeV1(
            page_id="page:1",
            container_path=("g0", "g1"),
            selected=(target(("g0", "g1"), "b"),),
            primary=target(("g0", "g1"), "b"),
        )
        missing_child = snapshot(
            edge("g0", None, ("g1", "a")),
            edge("g1", "g0", ("g2",)),
        )
        self.assertEqual(
            empty_top_level_scope_v1(),
            reconcile_nested_group_scope_v1(
                scope=scope,
                snapshot=missing_child,
            ),
        )

    def test_path_validation_rejects_cycle_wrong_parent_and_depth_overflow(self):
        with self.assertRaisesRegex(NestedGroupSelectionError, "cycle"):
            NestedGroupSelectionTargetV1(
                "page:1",
                ("g0", "g1", "g0"),
                "a",
            )

        wrong_parent = snapshot(
            edge("g0", None, ("g1",)),
            edge("g1", "wrong", ("a",)),
        )
        with self.assertRaisesRegex(NestedGroupSelectionError, "parent mismatch"):
            validate_nested_path_snapshot_v1(
                page_id="page:1",
                group_path=("g0", "g1"),
                snapshot=wrong_parent,
            )

        with self.assertRaisesRegex(NestedGroupSelectionError, "MAX_AUTHORED_GROUP_DEPTH"):
            NestedGroupSelectionTargetV1(
                "page:1",
                tuple(f"g{i}" for i in range(9)),
                "a",
            )

    def test_enter_requires_selected_child_group_and_valid_extended_snapshot(self):
        scope = enter_root_group_scope_v1(root_group=self.root, snapshot=self.s1)
        with self.assertRaisesRegex(NestedGroupSelectionError, "selected"):
            enter_selected_child_group_v1(
                scope=scope,
                current_snapshot=self.s1,
                child_group_id="g1",
                child_snapshot=self.s2,
            )

        selected = compose_nested_group_selection_v1(
            scope=scope,
            snapshot=self.s1,
            candidates=(target(("g0",), "g1"),),
            mode="replace",
        )
        bad_child_snapshot = snapshot(
            edge("g0", None, ("g1", "a")),
            edge("wrong", "g0", ("b",)),
        )
        with self.assertRaisesRegex(NestedGroupSelectionError, "differ from group_path"):
            enter_selected_child_group_v1(
                scope=selected,
                current_snapshot=self.s1,
                child_group_id="g1",
                child_snapshot=bad_child_snapshot,
            )

    def test_page_change_always_clears_nested_scope(self):
        scope = NestedGroupSelectionScopeV1(
            page_id="page:1",
            container_path=("g0", "g1"),
            selected=(target(("g0", "g1"), "b"),),
            primary=target(("g0", "g1"), "b"),
        )
        self.assertEqual(
            empty_top_level_scope_v1(),
            nested_scope_on_page_change_v1(scope),
        )

    def test_input_enumeration_is_not_semantic_inside_one_path(self):
        scope = NestedGroupSelectionScopeV1(
            page_id="page:1",
            container_path=("g0", "g1"),
            selected=(),
            primary=None,
        )
        b = target(("g0", "g1"), "b")
        g2 = target(("g0", "g1"), "g2")
        first = compose_nested_group_selection_v1(
            scope=scope,
            snapshot=self.s2,
            candidates=(g2, b),
            mode="replace",
        )
        second = compose_nested_group_selection_v1(
            scope=scope,
            snapshot=self.s2,
            candidates=(b, g2),
            mode="replace",
        )
        self.assertEqual(first, second)


if __name__ == "__main__":
    unittest.main()
