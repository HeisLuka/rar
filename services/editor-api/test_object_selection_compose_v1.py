import unittest

from object_selection_compose_v1 import (
    ObjectSelectionComposeError,
    compose_object_selection_v1,
)
from object_selection_target_v1 import (
    DirectNodeSelectionV1,
    GroupMemberSelectionV1,
    ProjectedInstanceSelectionV1,
    selection_target_to_json_v1,
)


def d(node_id):
    return DirectNodeSelectionV1("page:1", node_id)


class ObjectSelectionComposeV1Tests(unittest.TestCase):
    def test_replace_returns_exact_candidates_and_single_primary(self):
        result = compose_object_selection_v1(
            base_set=(d("a"), d("b")),
            base_primary=d("a"),
            candidate_set=(d("c"),),
            mode="replace",
        )
        self.assertEqual((d("c"),), result.selected_set)
        self.assertEqual(d("c"), result.primary)

    def test_replace_many_has_no_primary_even_if_old_primary_is_present(self):
        result = compose_object_selection_v1(
            base_set=(d("a"), d("b")),
            base_primary=d("a"),
            candidate_set=(d("a"), d("c")),
            mode="replace",
        )
        self.assertEqual({d("a"), d("c")}, set(result.selected_set))
        self.assertIsNone(result.primary)

    def test_add_preserves_base_primary_if_it_remains_selected(self):
        result = compose_object_selection_v1(
            base_set=(d("a"),),
            base_primary=d("a"),
            candidate_set=(d("b"),),
            mode="add",
        )
        self.assertEqual({d("a"), d("b")}, set(result.selected_set))
        self.assertEqual(d("a"), result.primary)

    def test_toggle_uses_symmetric_difference_and_recomputes_primary(self):
        result = compose_object_selection_v1(
            base_set=(d("a"), d("b")),
            base_primary=d("a"),
            candidate_set=(d("a"), d("b"), d("c")),
            mode="toggle",
        )
        self.assertEqual((d("c"),), result.selected_set)
        self.assertEqual(d("c"), result.primary)

    def test_subtract_can_clear_primary_or_selection(self):
        one = compose_object_selection_v1(
            base_set=(d("a"), d("b")),
            base_primary=d("a"),
            candidate_set=(d("a"),),
            mode="subtract",
        )
        self.assertEqual((d("b"),), one.selected_set)
        self.assertEqual(d("b"), one.primary)

        empty = compose_object_selection_v1(
            base_set=(d("b"),),
            base_primary=d("b"),
            candidate_set=(d("b"),),
            mode="subtract",
        )
        self.assertEqual((), empty.selected_set)
        self.assertIsNone(empty.primary)

    def test_empty_candidate_is_legal_for_every_mode(self):
        base = (d("a"), d("b"))
        primary = d("a")
        for mode in ("add", "toggle", "subtract"):
            with self.subTest(mode=mode):
                result = compose_object_selection_v1(
                    base_set=base,
                    base_primary=primary,
                    candidate_set=(),
                    mode=mode,
                )
                self.assertEqual(set(base), set(result.selected_set))
                self.assertEqual(primary, result.primary)

        replace = compose_object_selection_v1(
            base_set=base,
            base_primary=primary,
            candidate_set=(),
            mode="replace",
        )
        self.assertEqual((), replace.selected_set)
        self.assertIsNone(replace.primary)

    def test_enumeration_order_is_not_semantic_and_output_is_canonical(self):
        a, b, c = d("a"), d("b"), d("c")
        first = compose_object_selection_v1(
            base_set=(c, a),
            base_primary=None,
            candidate_set=(b,),
            mode="add",
        )
        second = compose_object_selection_v1(
            base_set=(a, c),
            base_primary=None,
            candidate_set=(b,),
            mode="add",
        )
        self.assertEqual(first, second)
        self.assertEqual(
            tuple(sorted(first.selected_set, key=selection_target_to_json_v1)),
            first.selected_set,
        )

    def test_variant_agnostic_algebra_does_not_collapse_identities(self):
        direct = DirectNodeSelectionV1("page:1", "node:1")
        projected = ProjectedInstanceSelectionV1(
            "page:1",
            "instance:1",
            "node:1",
            "master",
            "read_only",
        )
        member = GroupMemberSelectionV1("page:1", "group:1", "node:1")
        result = compose_object_selection_v1(
            base_set=(direct,),
            base_primary=direct,
            candidate_set=(projected, member),
            mode="add",
        )
        self.assertEqual(3, len(result.selected_set))
        self.assertIn(direct, result.selected_set)
        self.assertIn(projected, result.selected_set)
        self.assertIn(member, result.selected_set)
        self.assertEqual(direct, result.primary)

    def test_invalid_primary_rejects_fail_closed(self):
        with self.assertRaisesRegex(ObjectSelectionComposeError, "contained"):
            compose_object_selection_v1(
                base_set=(d("a"),),
                base_primary=d("missing"),
                candidate_set=(),
                mode="add",
            )

    def test_duplicate_base_or_candidate_rejects(self):
        with self.assertRaisesRegex(ObjectSelectionComposeError, "duplicate-free"):
            compose_object_selection_v1(
                base_set=(d("a"), d("a")),
                base_primary=None,
                candidate_set=(),
                mode="add",
            )
        with self.assertRaisesRegex(ObjectSelectionComposeError, "duplicate-free"):
            compose_object_selection_v1(
                base_set=(),
                base_primary=None,
                candidate_set=(d("a"), d("a")),
                mode="replace",
            )

    def test_unknown_mode_rejects(self):
        with self.assertRaisesRegex(ObjectSelectionComposeError, "unsupported"):
            compose_object_selection_v1(
                base_set=(),
                base_primary=None,
                candidate_set=(),
                mode="xor-ish",
            )


if __name__ == "__main__":
    unittest.main()
