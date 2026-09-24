import unittest

from group_member_selection_v1 import (
    GroupMemberSelectionError,
    GroupMembersSelectionScopeV1,
    GroupScopeReconcileReceiptV1,
    TopLevelSelectionScopeV1,
    clear_selection_scope_v1,
    compose_group_members_v1,
    empty_top_level_scope_v1,
    enter_group_member_v1,
    escape_parent_v1,
    reconcile_group_scope_v1,
    selection_scope_on_page_change_v1,
)
from object_selection_target_v1 import (
    DirectNodeSelectionV1,
    GroupMemberSelectionV1,
    ProjectedInstanceSelectionV1,
)


def root(node_id="group:1", page_id="page:1"):
    return DirectNodeSelectionV1(page_id, node_id)


def member(node_id, root_id="group:1", page_id="page:1"):
    return GroupMemberSelectionV1(page_id, root_id, node_id)


class GroupMemberSelectionV1Tests(unittest.TestCase):
    def test_enter_group_member_selects_exactly_one_member(self):
        r = root()
        m = member("node:a")
        scope = enter_group_member_v1(root_group=r, member=m)
        self.assertEqual(r, scope.root_group)
        self.assertEqual((m,), scope.selected_members)
        self.assertEqual(m, scope.primary)

    def test_group_scope_reuses_generic_replace_add_toggle_subtract(self):
        r = root()
        a, b, c = member("node:a"), member("node:b"), member("node:c")
        scope = enter_group_member_v1(root_group=r, member=a)

        added = compose_group_members_v1(
            scope=scope, candidates=(b, c), mode="add"
        )
        self.assertEqual({a, b, c}, set(added.selected_members))
        self.assertEqual(a, added.primary)

        toggled = compose_group_members_v1(
            scope=added, candidates=(a, b), mode="toggle"
        )
        self.assertEqual((c,), toggled.selected_members)
        self.assertEqual(c, toggled.primary)

        replaced = compose_group_members_v1(
            scope=added, candidates=(b,), mode="replace"
        )
        self.assertEqual((b,), replaced.selected_members)
        self.assertEqual(b, replaced.primary)

        subtracted = compose_group_members_v1(
            scope=added, candidates=(a, c), mode="subtract"
        )
        self.assertEqual((b,), subtracted.selected_members)
        self.assertEqual(b, subtracted.primary)

    def test_candidates_must_share_exact_root_and_page(self):
        scope = enter_group_member_v1(
            root_group=root(),
            member=member("node:a"),
        )
        with self.assertRaisesRegex(GroupMemberSelectionError, "same root Group"):
            compose_group_members_v1(
                scope=scope,
                candidates=(member("node:b", root_id="group:other"),),
                mode="add",
            )
        with self.assertRaisesRegex(GroupMemberSelectionError, "share root page"):
            compose_group_members_v1(
                scope=scope,
                candidates=(member("node:b", page_id="page:2"),),
                mode="add",
            )

    def test_escape_parent_selects_root_only_and_never_promotes_member(self):
        r = root()
        a, b = member("node:a"), member("node:b")
        scope = GroupMembersSelectionScopeV1(
            root_group=r,
            selected_members=(a, b),
            primary=a,
        )
        top = escape_parent_v1(scope)
        self.assertEqual((r,), top.selected)
        self.assertEqual(r, top.primary)
        self.assertNotIn(DirectNodeSelectionV1("page:1", "node:a"), top.selected)
        self.assertNotIn(DirectNodeSelectionV1("page:1", "node:b"), top.selected)

    def test_clear_from_either_scope_returns_empty_top_level(self):
        top = TopLevelSelectionScopeV1(
            selected=(root(),),
            primary=root(),
        )
        group = enter_group_member_v1(
            root_group=root(),
            member=member("node:a"),
        )
        self.assertEqual(empty_top_level_scope_v1(), clear_selection_scope_v1(top))
        self.assertEqual(empty_top_level_scope_v1(), clear_selection_scope_v1(group))

    def test_page_change_always_clears_scope(self):
        top = TopLevelSelectionScopeV1(
            selected=(
                ProjectedInstanceSelectionV1(
                    "page:1", "instance:1", "node:1", "master", "read_only"
                ),
            ),
            primary=None,
        )
        group = enter_group_member_v1(
            root_group=root(),
            member=member("node:a"),
        )
        self.assertEqual(empty_top_level_scope_v1(), selection_scope_on_page_change_v1(top))
        self.assertEqual(empty_top_level_scope_v1(), selection_scope_on_page_change_v1(group))

    def test_reconcile_valid_scope_preserves_members(self):
        scope = GroupMembersSelectionScopeV1(
            root_group=root(),
            selected_members=(member("node:a"), member("node:b")),
            primary=member("node:a"),
        )
        same = reconcile_group_scope_v1(
            scope=scope,
            receipt=GroupScopeReconcileReceiptV1(
                root_survives=True,
                membership_valid=True,
            ),
        )
        self.assertEqual(scope, same)

    def test_reconcile_invalid_membership_returns_root_or_empty_from_explicit_receipt(self):
        scope = enter_group_member_v1(
            root_group=root(),
            member=member("node:a"),
        )
        root_only = reconcile_group_scope_v1(
            scope=scope,
            receipt=GroupScopeReconcileReceiptV1(
                root_survives=True,
                membership_valid=False,
            ),
        )
        self.assertEqual((root(),), root_only.selected)
        self.assertEqual(root(), root_only.primary)

        empty = reconcile_group_scope_v1(
            scope=scope,
            receipt=GroupScopeReconcileReceiptV1(
                root_survives=False,
                membership_valid=False,
            ),
        )
        self.assertEqual(empty_top_level_scope_v1(), empty)

    def test_invalid_reconcile_truth_rejects(self):
        scope = enter_group_member_v1(
            root_group=root(),
            member=member("node:a"),
        )
        with self.assertRaisesRegex(GroupMemberSelectionError, "cannot remain valid"):
            reconcile_group_scope_v1(
                scope=scope,
                receipt=GroupScopeReconcileReceiptV1(
                    root_survives=False,
                    membership_valid=True,
                ),
            )

    def test_group_scope_invariants_fail_closed(self):
        r = root()
        a = member("node:a")
        with self.assertRaisesRegex(GroupMemberSelectionError, "same root Group"):
            GroupMembersSelectionScopeV1(
                root_group=r,
                selected_members=(member("node:b", root_id="group:other"),),
                primary=None,
            )
        with self.assertRaisesRegex(GroupMemberSelectionError, "duplicate-free"):
            GroupMembersSelectionScopeV1(
                root_group=r,
                selected_members=(a, a),
                primary=a,
            )
        with self.assertRaisesRegex(GroupMemberSelectionError, "primary must be selected"):
            GroupMembersSelectionScopeV1(
                root_group=r,
                selected_members=(a,),
                primary=member("node:missing"),
            )
        with self.assertRaisesRegex(GroupMemberSelectionError, "DirectNode"):
            GroupMembersSelectionScopeV1(
                root_group=ProjectedInstanceSelectionV1(
                    "page:1", "i", "group:1", "master", "read_only"
                ),
                selected_members=(a,),
                primary=a,
            )

    def test_top_level_scope_can_hold_typed_variants_but_requires_valid_primary(self):
        direct = DirectNodeSelectionV1("page:1", "node:a")
        projected = ProjectedInstanceSelectionV1(
            "page:1", "i", "node:a", "master", "read_only"
        )
        scope = TopLevelSelectionScopeV1(
            selected=(direct, projected),
            primary=direct,
        )
        self.assertEqual(2, len(scope.selected))
        with self.assertRaisesRegex(GroupMemberSelectionError, "primary must be selected"):
            TopLevelSelectionScopeV1(
                selected=(direct,),
                primary=projected,
            )


if __name__ == "__main__":
    unittest.main()
