#!/usr/bin/env python3
import copy
import unittest

from authored_group_hierarchy_v1 import (
    MAX_AUTHORED_GROUP_DEPTH_V1,
    AuthoredGroupHierarchyError,
    validate_authored_group_hierarchy_v1,
)


def rect(x=0, y=0, w=100, h=100):
    return {"x": x, "y": y, "width": w, "height": h}


def valid_project():
    return {
        "pages": {"page:1": {"children": ["group:outer"]}},
        "authored_stack": {"page:1": ["group:outer"]},
        "nodes": {
            "group:outer": {
                "kind": "group",
                "author_created": True,
                "parent_id": "page:1",
                "bounds": rect(10, 20, 400, 300),
                "local_coordinate_space": rect(0, 0, 400, 300),
                "children": ["rect:a", "group:inner"],
            },
            "rect:a": {
                "kind": "rectangle",
                "author_created": True,
                "parent_id": "group:outer",
                "bounds": rect(10, 10, 80, 60),
            },
            "group:inner": {
                "kind": "group",
                "author_created": True,
                "parent_id": "group:outer",
                "bounds": rect(100, 80, 200, 150),
                "local_coordinate_space": rect(0, 0, 200, 150),
                "children": ["rect:b"],
            },
            "rect:b": {
                "kind": "rectangle",
                "author_created": True,
                "parent_id": "group:inner",
                "bounds": rect(20, 30, 50, 40),
            },
        },
    }


class AuthoredGroupHierarchyV1Tests(unittest.TestCase):
    def test_valid_nested_tree_passes(self):
        self.assertTrue(validate_authored_group_hierarchy_v1(valid_project()))

    def test_nested_group_is_not_top_level_stack_member(self):
        project = valid_project()
        project["authored_stack"]["page:1"].append("group:inner")
        with self.assertRaisesRegex(AuthoredGroupHierarchyError, "nested authored node"):
            validate_authored_group_hierarchy_v1(project)

    def test_top_level_authored_member_must_be_in_stack(self):
        project = valid_project()
        project["authored_stack"]["page:1"] = []
        with self.assertRaisesRegex(AuthoredGroupHierarchyError, "missing from authored stack"):
            validate_authored_group_hierarchy_v1(project)

    def test_duplicate_group_children_fail_closed(self):
        project = valid_project()
        project["nodes"]["group:outer"]["children"].append("rect:a")
        with self.assertRaisesRegex(AuthoredGroupHierarchyError, "duplicates"):
            validate_authored_group_hierarchy_v1(project)

    def test_parent_back_reference_mismatch_fails_closed(self):
        project = valid_project()
        project["nodes"]["rect:a"]["parent_id"] = "page:1"
        with self.assertRaisesRegex(AuthoredGroupHierarchyError, "does not point back"):
            validate_authored_group_hierarchy_v1(project)

    def test_child_must_be_contained_in_immediate_local_space(self):
        project = valid_project()
        project["nodes"]["rect:b"]["bounds"] = rect(180, 140, 50, 40)
        with self.assertRaisesRegex(AuthoredGroupHierarchyError, "exceeds parent"):
            validate_authored_group_hierarchy_v1(project)

    def test_local_space_must_be_positive_and_zero_origin(self):
        for bad in (rect(1, 0, 200, 150), rect(0, 0, 0, 150)):
            project = valid_project()
            project["nodes"]["group:inner"]["local_coordinate_space"] = bad
            with self.subTest(bad=bad):
                with self.assertRaises(AuthoredGroupHierarchyError):
                    validate_authored_group_hierarchy_v1(project)

    def test_depth_limit_is_product_owned_and_exact(self):
        project = {
            "pages": {"page:1": {"children": ["g1"]}},
            "authored_stack": {"page:1": ["g1"]},
            "nodes": {},
        }
        for depth in range(1, MAX_AUTHORED_GROUP_DEPTH_V1 + 2):
            node_id = f"g{depth}"
            parent_id = "page:1" if depth == 1 else f"g{depth - 1}"
            child_id = f"g{depth + 1}" if depth < MAX_AUTHORED_GROUP_DEPTH_V1 + 1 else "leaf"
            project["nodes"][node_id] = {
                "kind": "group",
                "author_created": True,
                "parent_id": parent_id,
                "bounds": rect(),
                "local_coordinate_space": rect(),
                "children": [child_id],
            }
        project["nodes"]["leaf"] = {
            "kind": "rectangle",
            "author_created": True,
            "parent_id": f"g{MAX_AUTHORED_GROUP_DEPTH_V1 + 1}",
            "bounds": rect(1, 1, 10, 10),
        }
        with self.assertRaisesRegex(AuthoredGroupHierarchyError, "depth overflow"):
            validate_authored_group_hierarchy_v1(project)

    def test_valid_save_reopen_shape_is_deterministic(self):
        project = valid_project()
        reopened = copy.deepcopy(project)
        self.assertTrue(validate_authored_group_hierarchy_v1(project))
        self.assertTrue(validate_authored_group_hierarchy_v1(reopened))


if __name__ == "__main__":
    unittest.main()
