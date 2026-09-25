#!/usr/bin/env python3
import unittest

from editor_multi_select_instance_v1 import (
    EditorMultiSelectInstanceError,
    change_instance_selection_page_v1,
    click_instance_target_v1,
    empty_instance_selection_v1,
    inspector_instance_selection_v1,
    selected_mutation_node_ids_v1,
    shift_click_instance_target_v1,
)
from object_selection_target_v1 import (
    DirectNodeSelectionV1,
    GroupMemberSelectionV1,
    ProjectedInstanceSelectionV1,
)


class EditorMultiSelectInstanceV1Tests(unittest.TestCase):
    def test_repeated_projected_instances_with_shared_origin_remain_distinct(self):
        s=empty_instance_selection_v1(page_id="page:1")
        a=ProjectedInstanceSelectionV1("page:1","inst:1","origin:1","master","read_only")
        b=ProjectedInstanceSelectionV1("page:1","inst:2","origin:1","master","read_only")
        s=shift_click_instance_target_v1(s,candidate=a)
        s=shift_click_instance_target_v1(s,candidate=b)
        self.assertEqual(2,len(s.selected))
        self.assertNotEqual(s.selected[0],s.selected[1])

    def test_group_member_rejected_from_top_level_instance_set(self):
        s=empty_instance_selection_v1(page_id="page:1")
        with self.assertRaises(EditorMultiSelectInstanceError):
            click_instance_target_v1(
                s,
                candidate=GroupMemberSelectionV1("page:1","group:1","child:1"),
            )

    def test_page_change_clears_selection(self):
        s=click_instance_target_v1(
            empty_instance_selection_v1(page_id="page:1"),
            candidate=DirectNodeSelectionV1("page:1","n1"),
        )
        s=change_instance_selection_page_v1(s,page_id="page:2")
        self.assertEqual("page:2",s.page_id)
        self.assertEqual((),s.selected)

    def test_projected_target_blocks_mutation_lowering(self):
        s=click_instance_target_v1(
            empty_instance_selection_v1(page_id="page:1"),
            candidate=ProjectedInstanceSelectionV1(
                "page:1","inst:1","origin:1","carrier","read_only"
            ),
        )
        with self.assertRaisesRegex(EditorMultiSelectInstanceError,"inspectable"):
            selected_mutation_node_ids_v1(
                s,operation_safe_direct_node_ids=frozenset({"origin:1"})
            )

    def test_direct_targets_require_explicit_operation_safe_admission(self):
        s=empty_instance_selection_v1(page_id="page:1")
        s=shift_click_instance_target_v1(
            s,candidate=DirectNodeSelectionV1("page:1","a")
        )
        s=shift_click_instance_target_v1(
            s,candidate=DirectNodeSelectionV1("page:1","b")
        )
        self.assertEqual(
            ("a","b"),
            selected_mutation_node_ids_v1(
                s,operation_safe_direct_node_ids=frozenset({"a","b"})
            ),
        )
        with self.assertRaisesRegex(EditorMultiSelectInstanceError,"explicit operation-safe"):
            selected_mutation_node_ids_v1(
                s,operation_safe_direct_node_ids=frozenset({"a"})
            )

    def test_inspector_separates_instance_and_origin_identity(self):
        p=ProjectedInstanceSelectionV1(
            "page:1","inst:9","origin:2","master","read_only"
        )
        s=click_instance_target_v1(
            empty_instance_selection_v1(page_id="page:1"),candidate=p
        )
        info=inspector_instance_selection_v1(s)[0]
        self.assertEqual("inst:9",info["instance_id"])
        self.assertEqual("origin:2",info["origin_node_id"])
        self.assertTrue(info["read_only"])


if __name__=="__main__":
    unittest.main()
