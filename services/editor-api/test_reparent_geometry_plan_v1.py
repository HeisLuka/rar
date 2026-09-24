import unittest

from authored_group_geometry_v1 import RectEmu
from group_transform_chain_v1 import AuthoredGroupEdgeV1
from reparent_geometry_plan_v1 import (
    ReparentContainerV1,
    plan_reparent_geometry_v1,
)


def page(page_id, children):
    return ReparentContainerV1(
        kind="page",
        container_id=page_id,
        page_id=page_id,
        children=tuple(children),
    )


def group(group_id, page_id, bounds, local, children, *, parent=None, prefix=(), provenance="chaptera-authored-group-v1"):
    edge = AuthoredGroupEdgeV1(
        group_id=group_id,
        page_id=page_id,
        parent_group_id=parent,
        children=tuple(children),
        bounds_in_parent=bounds,
        local_coordinate_space=local,
        provenance=provenance,
    )
    return ReparentContainerV1(
        kind="group",
        container_id=group_id,
        page_id=page_id,
        children=tuple(children),
        ancestry=tuple(prefix) + (edge,),
    )


class ReparentGeometryPlanV1Tests(unittest.TestCase):
    def test_page_to_group_preserves_exact_page_geometry(self):
        source = page("page:1", ("n", "other"))
        destination = group(
            "g",
            "page:1",
            RectEmu(0, 0, 100, 100),
            RectEmu(0, 0, 100, 100),
            ("d",),
        )
        plan = plan_reparent_geometry_v1(
            node_id="n",
            node_authority="chaptera-authored",
            source=source,
            destination=destination,
            source_local_rect=RectEmu(20, 30, 25, 15),
            destination_insert_index=1,
            destination_anchor_node_id=None,
        )
        self.assertEqual("planned", plan.status)
        self.assertEqual(RectEmu(20, 30, 25, 15), plan.geometry.source_effective_page_rect)
        self.assertEqual(RectEmu(20, 30, 25, 15), plan.geometry.destination_local_rect)
        self.assertEqual(plan.geometry.source_effective_page_rect, plan.geometry.destination_effective_page_rect)
        self.assertEqual(0, plan.removal.source_index)
        self.assertEqual("other", plan.removal.next_node_id)
        self.assertEqual(1, plan.insertion.insert_index)

    def test_scaled_group_to_page_preserves_visible_rect_not_local_numbers(self):
        source = group(
            "g",
            "page:1",
            RectEmu(100, 100, 200, 100),
            RectEmu(0, 0, 100, 100),
            ("n", "s"),
        )
        destination = page("page:1", ("x",))
        plan = plan_reparent_geometry_v1(
            node_id="n",
            node_authority="chaptera-authored",
            source=source,
            destination=destination,
            source_local_rect=RectEmu(25, 20, 25, 20),
            destination_insert_index=1,
            destination_anchor_node_id=None,
        )
        self.assertEqual("planned", plan.status)
        self.assertEqual(RectEmu(150, 120, 50, 20), plan.geometry.source_effective_page_rect)
        self.assertEqual(RectEmu(150, 120, 50, 20), plan.geometry.destination_local_rect)

    def test_group_to_group_exact_quantized_roundtrip(self):
        source = group(
            "source:g",
            "page:1",
            RectEmu(0, 0, 100, 100),
            RectEmu(0, 0, 100, 100),
            ("n",),
        )
        destination = group(
            "dest:g",
            "page:1",
            RectEmu(100, 100, 200, 200),
            RectEmu(0, 0, 100, 100),
            ("d",),
        )
        plan = plan_reparent_geometry_v1(
            node_id="n",
            node_authority="chaptera-authored",
            source=source,
            destination=destination,
            source_local_rect=RectEmu(120, 140, 40, 20),
            destination_insert_index=0,
            destination_anchor_node_id="d",
        )
        # Source page container is source:g itself, so the chosen local rect is
        # outside source:g and must fail stale rather than be silently projected.
        self.assertEqual("rejected", plan.status)
        self.assertEqual("stale_membership", plan.reason)

        source_page = page("page:1", ("n",))
        exact = plan_reparent_geometry_v1(
            node_id="n",
            node_authority="chaptera-authored",
            source=source_page,
            destination=destination,
            source_local_rect=RectEmu(120, 140, 40, 20),
            destination_insert_index=0,
            destination_anchor_node_id="d",
        )
        self.assertEqual("planned", exact.status)
        self.assertEqual(RectEmu(10, 20, 20, 10), exact.geometry.destination_local_rect)
        self.assertEqual(RectEmu(120, 140, 40, 20), exact.geometry.destination_effective_page_rect)

    def test_destination_integer_quantization_must_roundtrip_exactly(self):
        source = page("page:1", ("n",))
        destination = group(
            "g",
            "page:1",
            RectEmu(0, 0, 3, 3),
            RectEmu(0, 0, 2, 2),
            ("d",),
        )
        plan = plan_reparent_geometry_v1(
            node_id="n",
            node_authority="chaptera-authored",
            source=source,
            destination=destination,
            source_local_rect=RectEmu(1, 0, 2, 3),
            destination_insert_index=1,
            destination_anchor_node_id=None,
        )
        self.assertEqual("rejected", plan.status)
        self.assertEqual("not_exactly_representable", plan.reason)

    def test_destination_group_never_auto_refits(self):
        source = page("page:1", ("n",))
        destination = group(
            "g",
            "page:1",
            RectEmu(0, 0, 100, 100),
            RectEmu(0, 0, 100, 100),
            ("d",),
        )
        plan = plan_reparent_geometry_v1(
            node_id="n",
            node_authority="chaptera-authored",
            source=source,
            destination=destination,
            source_local_rect=RectEmu(90, 90, 20, 20),
            destination_insert_index=1,
            destination_anchor_node_id=None,
        )
        self.assertEqual("rejected", plan.status)
        self.assertEqual("destination_out_of_bounds", plan.reason)

    def test_same_parent_cycle_and_cross_page_reject(self):
        source = page("page:1", ("n",))
        same = page("page:1", ("n",))
        result = plan_reparent_geometry_v1(
            node_id="n",
            node_authority="chaptera-authored",
            source=source,
            destination=same,
            source_local_rect=RectEmu(0, 0, 10, 10),
            destination_insert_index=0,
            destination_anchor_node_id="n",
        )
        self.assertEqual("same_parent", result.reason)

        cycle_destination = group(
            "n",
            "page:1",
            RectEmu(0, 0, 100, 100),
            RectEmu(0, 0, 100, 100),
            ("d",),
        )
        cycle = plan_reparent_geometry_v1(
            node_id="n",
            node_authority="chaptera-authored",
            source=source,
            destination=cycle_destination,
            source_local_rect=RectEmu(10, 10, 10, 10),
            destination_insert_index=0,
            destination_anchor_node_id="d",
        )
        self.assertEqual("cycle", cycle.reason)

        other_page = page("page:2", ())
        cross = plan_reparent_geometry_v1(
            node_id="n",
            node_authority="chaptera-authored",
            source=source,
            destination=other_page,
            source_local_rect=RectEmu(10, 10, 10, 10),
            destination_insert_index=0,
            destination_anchor_node_id=None,
        )
        self.assertEqual("cross_page", cross.reason)

    def test_descendant_cycle_and_stale_anchor_reject(self):
        source = page("page:1", ("group:n",))
        destination = group(
            "descendant:g",
            "page:1",
            RectEmu(0, 0, 100, 100),
            RectEmu(0, 0, 100, 100),
            ("d",),
        )
        cycle = plan_reparent_geometry_v1(
            node_id="group:n",
            node_authority="chaptera-authored",
            source=source,
            destination=destination,
            source_local_rect=RectEmu(10, 10, 20, 20),
            destination_insert_index=0,
            destination_anchor_node_id="d",
            node_descendant_ids=("descendant:g",),
        )
        self.assertEqual("cycle", cycle.reason)

        non_cycle_source = page("page:1", ("n",))
        stale = plan_reparent_geometry_v1(
            node_id="n",
            node_authority="chaptera-authored",
            source=non_cycle_source,
            destination=destination,
            source_local_rect=RectEmu(10, 10, 20, 20),
            destination_insert_index=0,
            destination_anchor_node_id=None,
        )
        self.assertEqual("stale_membership", stale.reason)

    def test_stale_source_and_unsupported_authority_fail_closed(self):
        source = page("page:1", ("other",))
        destination = group(
            "dest:g",
            "page:1",
            RectEmu(0, 0, 100, 100),
            RectEmu(0, 0, 100, 100),
            ("x",),
        )
        stale = plan_reparent_geometry_v1(
            node_id="n",
            node_authority="chaptera-authored",
            source=source,
            destination=destination,
            source_local_rect=RectEmu(10, 10, 20, 20),
            destination_insert_index=1,
            destination_anchor_node_id=None,
        )
        self.assertEqual("stale_membership", stale.reason)

        source_ok = page("page:1", ("n",))
        unsupported = plan_reparent_geometry_v1(
            node_id="n",
            node_authority="source-backed",
            source=source_ok,
            destination=destination,
            source_local_rect=RectEmu(10, 10, 20, 20),
            destination_insert_index=1,
            destination_anchor_node_id=None,
        )
        self.assertEqual("unsupported", unsupported.reason)

    def test_imported_destination_group_is_not_admitted(self):
        source = page("page:1", ("n",))
        destination = group(
            "g",
            "page:1",
            RectEmu(0, 0, 100, 100),
            RectEmu(0, 0, 100, 100),
            ("d",),
            provenance="source-backed",
        )
        plan = plan_reparent_geometry_v1(
            node_id="n",
            node_authority="chaptera-authored",
            source=source,
            destination=destination,
            source_local_rect=RectEmu(10, 10, 20, 20),
            destination_insert_index=0,
            destination_anchor_node_id="d",
        )
        self.assertEqual("unsupported", plan.reason)


if __name__ == "__main__":
    unittest.main()
