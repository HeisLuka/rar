#!/usr/bin/env python3
import copy
import unittest

from authored_group_geometry_v1 import RectEmu, materialize_group_child_rect_v1
from group_insert_refit_v1 import (
    GroupInsertRefitPlanError,
    ProposedGroupInsertChildV1,
    plan_group_insert_refit_v1,
)


def rect(x, y, w, h):
    return {"x": x, "y": y, "width": w, "height": h}


def project():
    return {
        "pages": {"page:1": {"children": ["group:g"]}},
        "authored_stack": {"page:1": ["group:g"]},
        "nodes": {
            "group:g": {
                "kind": "group",
                "author_created": True,
                "parent_id": "page:1",
                "bounds": rect(100, 100, 200, 100),
                "local_coordinate_space": rect(0, 0, 100, 100),
                "children": ["rect:a", "rect:b"],
            },
            "rect:a": {
                "kind": "rectangle",
                "author_created": True,
                "parent_id": "group:g",
                "bounds": rect(10, 10, 20, 20),
            },
            "rect:b": {
                "kind": "rectangle",
                "author_created": True,
                "parent_id": "group:g",
                "bounds": rect(60, 60, 20, 20),
            },
        },
    }


class GroupInsertRefitPlanV1Tests(unittest.TestCase):
    def plan(self, proposed, **kwargs):
        return plan_group_insert_refit_v1(
            project=project(),
            group_id="group:g",
            expected_group_bounds=RectEmu(100, 100, 200, 100),
            expected_local_coordinate_space=RectEmu(0, 0, 100, 100),
            expected_child_order=("rect:a", "rect:b"),
            proposed_children=proposed,
            **kwargs,
        )

    def test_contained_insert_keeps_envelope_and_derives_exact_scaled_local(self):
        desired = RectEmu(150, 120, 40, 20)
        plan = self.plan((ProposedGroupInsertChildV1("rect:new", desired),))
        self.assertEqual("no_envelope_change", plan.status)
        self.assertEqual(RectEmu(100, 100, 200, 100), plan.new_group_bounds)
        self.assertEqual(RectEmu(0, 0, 100, 100), plan.new_local_coordinate_space)
        new = plan.proposed_children[0]
        self.assertEqual(desired, new.effective_parent_rect)
        self.assertEqual(RectEmu(25, 20, 20, 20), new.local_rect)
        self.assertEqual(
            desired,
            materialize_group_child_rect_v1(
                local_coordinate_space=plan.new_local_coordinate_space,
                child_local_bounds=new.local_rect,
                current_group_bounds=plan.new_group_bounds,
            ),
        )

    def test_multi_insert_expands_once_and_preserves_every_existing_effective_rect(self):
        before = [
            materialize_group_child_rect_v1(
                local_coordinate_space=RectEmu(0, 0, 100, 100),
                child_local_bounds=RectEmu(10, 10, 20, 20),
                current_group_bounds=RectEmu(100, 100, 200, 100),
            ),
            materialize_group_child_rect_v1(
                local_coordinate_space=RectEmu(0, 0, 100, 100),
                child_local_bounds=RectEmu(60, 60, 20, 20),
                current_group_bounds=RectEmu(100, 100, 200, 100),
            ),
        ]
        proposed = (
            ProposedGroupInsertChildV1("rect:new1", RectEmu(50, 80, 30, 30)),
            ProposedGroupInsertChildV1("rect:new2", RectEmu(320, 160, 40, 20)),
        )
        plan = self.plan(proposed)
        self.assertEqual("planned", plan.status)
        self.assertEqual(RectEmu(50, 80, 310, 120), plan.new_group_bounds)
        self.assertEqual(RectEmu(0, 0, 310, 120), plan.new_local_coordinate_space)
        self.assertEqual(["rect:a", "rect:b"], [x.node_id for x in plan.existing_children])
        self.assertEqual(before, [x.effective_parent_rect for x in plan.existing_children])
        self.assertEqual(["rect:new1", "rect:new2"], [x.node_id for x in plan.proposed_children])
        for result in plan.existing_children + plan.proposed_children:
            self.assertEqual(
                result.effective_parent_rect,
                materialize_group_child_rect_v1(
                    local_coordinate_space=plan.new_local_coordinate_space,
                    child_local_bounds=result.local_rect,
                    current_group_bounds=plan.new_group_bounds,
                ),
            )

    def test_nested_escape_returns_ancestor_candidate(self):
        plan = self.plan(
            (ProposedGroupInsertChildV1("rect:new", RectEmu(320, 100, 20, 20)),),
            parent_local_coordinate_space=RectEmu(0, 0, 330, 300),
        )
        self.assertEqual("ancestor_refit_required", plan.status)
        self.assertEqual(plan.new_group_bounds, plan.ancestor_candidate_rect)

    def test_stale_header_or_order_fails_closed(self):
        with self.assertRaisesRegex(GroupInsertRefitPlanError, "stale"):
            plan_group_insert_refit_v1(
                project=project(),
                group_id="group:g",
                expected_group_bounds=RectEmu(101, 100, 200, 100),
                expected_local_coordinate_space=RectEmu(0, 0, 100, 100),
                expected_child_order=("rect:a", "rect:b"),
                proposed_children=(ProposedGroupInsertChildV1("new", RectEmu(120, 120, 10, 10)),),
            )
        with self.assertRaisesRegex(GroupInsertRefitPlanError, "stale"):
            plan_group_insert_refit_v1(
                project=project(),
                group_id="group:g",
                expected_group_bounds=RectEmu(100, 100, 200, 100),
                expected_local_coordinate_space=RectEmu(0, 0, 100, 100),
                expected_child_order=("rect:b", "rect:a"),
                proposed_children=(ProposedGroupInsertChildV1("new", RectEmu(120, 120, 10, 10)),),
            )

    def test_existing_or_duplicate_proposed_ids_fail_closed(self):
        with self.assertRaisesRegex(GroupInsertRefitPlanError, "already exists"):
            self.plan((ProposedGroupInsertChildV1("rect:a", RectEmu(120, 120, 10, 10)),))
        with self.assertRaisesRegex(GroupInsertRefitPlanError, "unique"):
            self.plan((
                ProposedGroupInsertChildV1("new", RectEmu(120, 120, 10, 10)),
                ProposedGroupInsertChildV1("new", RectEmu(140, 120, 10, 10)),
            ))

    def test_invalid_hierarchy_fails_closed(self):
        bad = project()
        bad["nodes"]["rect:a"]["parent_id"] = "group:other"
        with self.assertRaisesRegex(GroupInsertRefitPlanError, "invalid authored hierarchy"):
            plan_group_insert_refit_v1(
                project=bad,
                group_id="group:g",
                expected_group_bounds=RectEmu(100, 100, 200, 100),
                expected_local_coordinate_space=RectEmu(0, 0, 100, 100),
                expected_child_order=("rect:a", "rect:b"),
                proposed_children=(ProposedGroupInsertChildV1("new", RectEmu(120, 120, 10, 10)),),
            )

    def test_non_exact_inverse_is_rejected(self):
        p = project()
        p["nodes"]["group:g"]["bounds"] = rect(0, 0, 3, 3)
        p["nodes"]["group:g"]["local_coordinate_space"] = rect(0, 0, 2, 2)
        p["nodes"]["rect:a"]["bounds"] = rect(0, 0, 1, 1)
        p["nodes"]["rect:b"]["bounds"] = rect(1, 1, 1, 1)
        with self.assertRaises(GroupInsertRefitPlanError):
            plan_group_insert_refit_v1(
                project=p,
                group_id="group:g",
                expected_group_bounds=RectEmu(0, 0, 3, 3),
                expected_local_coordinate_space=RectEmu(0, 0, 2, 2),
                expected_child_order=("rect:a", "rect:b"),
                proposed_children=(ProposedGroupInsertChildV1("new", RectEmu(1, 1, 1, 1)),),
            )


if __name__ == "__main__":
    unittest.main()
