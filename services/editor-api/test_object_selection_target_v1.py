import json
import unittest

import object_selection_target_v1 as selection_target
from object_selection_target_v1 import (
    DirectNodeSelectionV1,
    GroupMemberSelectionV1,
    ObjectSelectionTargetError,
    ProjectedInstanceSelectionV1,
    selection_target_fingerprint_v1,
    selection_target_from_dict_v1,
    selection_target_to_dict_v1,
    selection_target_to_json_v1,
    target_page_id_v1,
    target_provenance_v1,
    target_variant_v1,
)


class ObjectSelectionTargetV1Tests(unittest.TestCase):
    def test_three_variants_are_distinct_even_for_same_node_identity(self):
        direct = DirectNodeSelectionV1("page:1", "node:42")
        projected = ProjectedInstanceSelectionV1(
            "page:1",
            "instance:7",
            "node:42",
            "master_page",
            "read_only",
        )
        member = GroupMemberSelectionV1("page:1", "group:9", "node:42")

        self.assertNotEqual(direct, projected)
        self.assertNotEqual(direct, member)
        self.assertNotEqual(projected, member)
        self.assertEqual("direct_node", target_variant_v1(direct))
        self.assertEqual("projected_instance", target_variant_v1(projected))
        self.assertEqual("group_member", target_variant_v1(member))

    def test_projected_instances_with_shared_origin_remain_distinct(self):
        first = ProjectedInstanceSelectionV1(
            "page:1",
            "instance:1",
            "node:origin",
            "master_page",
            "read_only",
        )
        second = ProjectedInstanceSelectionV1(
            "page:1",
            "instance:2",
            "node:origin",
            "master_page",
            "read_only",
        )
        self.assertNotEqual(first, second)
        self.assertNotEqual(
            selection_target_fingerprint_v1(first),
            selection_target_fingerprint_v1(second),
        )

    def test_page_identity_participates_in_equality_and_fingerprint(self):
        a = DirectNodeSelectionV1("page:1", "node:1")
        b = DirectNodeSelectionV1("page:2", "node:1")
        self.assertNotEqual(a, b)
        self.assertNotEqual(
            selection_target_fingerprint_v1(a),
            selection_target_fingerprint_v1(b),
        )

    def test_serialization_is_canonical_and_roundtrips_variant_exactly(self):
        targets = (
            DirectNodeSelectionV1("page:1", "node:1"),
            ProjectedInstanceSelectionV1(
                "page:1",
                "instance:1",
                "node:1",
                "linked_story_projection",
                "inspect_only",
            ),
            GroupMemberSelectionV1("page:1", "group:1", "node:1"),
        )
        for target in targets:
            with self.subTest(target=target):
                encoded = selection_target_to_json_v1(target)
                self.assertEqual(
                    encoded,
                    json.dumps(
                        selection_target_to_dict_v1(target),
                        ensure_ascii=False,
                        sort_keys=True,
                        separators=(",", ":"),
                    ),
                )
                decoded = selection_target_from_dict_v1(json.loads(encoded))
                self.assertEqual(target, decoded)
                self.assertEqual(
                    selection_target_fingerprint_v1(target),
                    selection_target_fingerprint_v1(decoded),
                )

    def test_frozen_dataclass_hash_is_variant_aware(self):
        direct = DirectNodeSelectionV1("page:1", "node:1")
        member = GroupMemberSelectionV1("page:1", "group:1", "node:1")
        targets = {direct, member}
        self.assertEqual(2, len(targets))
        self.assertIn(direct, targets)
        self.assertIn(member, targets)

    def test_provenance_accessor_never_erases_variant(self):
        projected = ProjectedInstanceSelectionV1(
            "page:1",
            "instance:7",
            "node:42",
            "master_page",
            "read_only",
        )
        provenance = target_provenance_v1(projected)
        self.assertEqual("projected_instance", provenance["variant"])
        self.assertEqual("instance:7", provenance["instance_id"])
        self.assertEqual("node:42", provenance["origin_node_id"])
        self.assertEqual("page:1", target_page_id_v1(projected))
        self.assertNotIn("node_id", provenance)

    def test_no_generic_node_id_collapse_helper_exists(self):
        forbidden = {
            "to_node_id",
            "target_node_id",
            "selection_target_node_id",
            "as_node_id",
        }
        self.assertTrue(forbidden.isdisjoint(set(dir(selection_target))))

    def test_invalid_identity_components_fail_at_construction(self):
        constructors = (
            lambda: DirectNodeSelectionV1("", "node:1"),
            lambda: DirectNodeSelectionV1("page:1", ""),
            lambda: ProjectedInstanceSelectionV1(
                "page:1", "", "node:1", "master", "read_only"
            ),
            lambda: ProjectedInstanceSelectionV1(
                "page:1", "instance:1", "", "master", "read_only"
            ),
            lambda: ProjectedInstanceSelectionV1(
                "page:1", "instance:1", "node:1", "", "read_only"
            ),
            lambda: ProjectedInstanceSelectionV1(
                "page:1", "instance:1", "node:1", "master", ""
            ),
            lambda: GroupMemberSelectionV1("page:1", "", "node:1"),
            lambda: GroupMemberSelectionV1("page:1", "group:1", ""),
            lambda: GroupMemberSelectionV1("page:1", "same", "same"),
        )
        for constructor in constructors:
            with self.subTest(constructor=constructor):
                with self.assertRaises(ObjectSelectionTargetError):
                    constructor()

    def test_parser_rejects_extra_fields_unknown_variant_and_bad_version(self):
        valid = selection_target_to_dict_v1(
            DirectNodeSelectionV1("page:1", "node:1")
        )
        with self.assertRaisesRegex(ObjectSelectionTargetError, "unexpected"):
            selection_target_from_dict_v1({**valid, "origin_node_id": "node:1"})
        with self.assertRaisesRegex(ObjectSelectionTargetError, "unknown"):
            selection_target_from_dict_v1(
                {
                    "protocol_version": "chaptera.object-selection-target.v1",
                    "variant": "nested_group_path",
                }
            )
        with self.assertRaisesRegex(ObjectSelectionTargetError, "protocol_version"):
            selection_target_from_dict_v1(
                {
                    "protocol_version": "chaptera.object-selection-target.v2",
                    "variant": "direct_node",
                    "page_id": "page:1",
                    "node_id": "node:1",
                }
            )

    def test_v1_group_member_is_exactly_one_level_identity(self):
        payload = selection_target_to_dict_v1(
            GroupMemberSelectionV1("page:1", "group:root", "node:member")
        )
        self.assertEqual(
            {"page_id", "protocol_version", "variant", "root_group_id", "member_node_id"},
            set(payload),
        )
        self.assertNotIn("group_path", payload)


if __name__ == "__main__":
    unittest.main()
