#!/usr/bin/env python3
import unittest

from resolved_text_caret_map_v1 import (
    InternalCaretStopV1,
    ResolvedClusterV1,
    ResolvedLineFragmentV1,
    ResolvedTextCaretMapError,
    build_resolved_text_caret_map_v1,
    caret_map_hash_v1,
    hit_test_story_position_v1,
    resolve_story_position_v1,
    selection_geometry_v1,
)


def cluster(a, b, x0, x1, *, painted=True, internal=()):
    return ResolvedClusterV1(
        a, b,
        x0, x1,
        x0, x1,
        painted,
        tuple(internal),
    )


def line(
    line_id,
    ordinal,
    clusters,
    *,
    page="page:1",
    frame="frame:1",
    prev=None,
    next=None,
    y=0,
):
    return ResolvedLineFragmentV1(
        story_id="story:1",
        page_id=page,
        frame_id=frame,
        line_id=line_id,
        flow_ordinal=ordinal,
        previous_line_id=prev,
        next_line_id=next,
        page_y_top_emu=y,
        page_y_bottom_emu=y + 20,
        frame_y_top_emu=y,
        frame_y_bottom_emu=y + 20,
        clusters=tuple(clusters),
    )


class ResolvedTextCaretMapV1Tests(unittest.TestCase):
    def test_ascii_one_scalar_clusters_expose_cluster_edge_stops(self):
        m = build_resolved_text_caret_map_v1(
            layout_revision_id="layout:1",
            story_id="story:1",
            story_scalar_len=3,
            lines=(
                line("l1", 0, (
                    cluster(0, 1, 0, 10),
                    cluster(1, 2, 10, 20),
                    cluster(2, 3, 20, 30),
                )),
            ),
        )
        self.assertEqual((0, 1, 2, 3), tuple(
            sorted({stop.scalar_boundary for stop in m.caret_stops})
        ))
        self.assertEqual(20, resolve_story_position_v1(
            caret_map=m, scalar_boundary=2
        ).page_x_emu)

    def test_emoji_is_one_unicode_scalar_boundary_step(self):
        m = build_resolved_text_caret_map_v1(
            layout_revision_id="layout:1",
            story_id="story:1",
            story_scalar_len=2,
            lines=(line("l1", 0, (
                cluster(0, 1, 0, 12),
                cluster(1, 2, 12, 24),
            )),),
        )
        # Python/Chaptera scalar coordinate: supplementary emoji occupies one
        # scalar slot, not two UTF-16 code units.
        self.assertEqual(12, resolve_story_position_v1(
            caret_map=m, scalar_boundary=1
        ).page_x_emu)

    def test_combining_cluster_interior_is_explicit_unsupported(self):
        m = build_resolved_text_caret_map_v1(
            layout_revision_id="layout:1",
            story_id="story:1",
            story_scalar_len=2,
            lines=(line("l1", 0, (cluster(0, 2, 0, 15),)),),
        )
        with self.assertRaises(ResolvedTextCaretMapError) as caught:
            resolve_story_position_v1(caret_map=m, scalar_boundary=1)
        self.assertEqual("internal_cluster_unsupported", caught.exception.code)

    def test_real_ligature_like_cluster_is_not_split_proportionally(self):
        m = build_resolved_text_caret_map_v1(
            layout_revision_id="layout:1",
            story_id="story:1",
            story_scalar_len=2,
            lines=(line("l1", 0, (cluster(0, 2, 0, 18),)),),
        )
        geometry = selection_geometry_v1(
            caret_map=m,
            start_scalar=0,
            end_scalar=1,
        )
        self.assertEqual("unsupported", geometry.coverage_state)
        self.assertEqual(((0, 1),), tuple(
            (r.start_scalar, r.end_scalar) for r in geometry.unsupported_ranges
        ))
        self.assertEqual((), geometry.rectangles)

    def test_explicit_internal_caret_authority_admits_partial_cluster(self):
        m = build_resolved_text_caret_map_v1(
            layout_revision_id="layout:1",
            story_id="story:1",
            story_scalar_len=2,
            lines=(line("l1", 0, (
                cluster(
                    0, 2, 0, 20,
                    internal=(InternalCaretStopV1(1, 9, 9),),
                ),
            )),),
        )
        self.assertEqual(9, resolve_story_position_v1(
            caret_map=m, scalar_boundary=1
        ).page_x_emu)
        geometry = selection_geometry_v1(
            caret_map=m, start_scalar=0, end_scalar=1
        )
        self.assertEqual("complete", geometry.coverage_state)
        self.assertEqual(9, geometry.rectangles[0].page_x_end_emu)

    def test_hit_test_before_inside_after_line_uses_nearest_admitted_stop(self):
        m = build_resolved_text_caret_map_v1(
            layout_revision_id="layout:1",
            story_id="story:1",
            story_scalar_len=3,
            lines=(line("l1", 0, (
                cluster(0,1,0,10),
                cluster(1,2,10,20),
                cluster(2,3,20,30),
            )),),
        )
        self.assertEqual(0, hit_test_story_position_v1(
            caret_map=m,page_id="page:1",page_x_emu=-100,page_y_emu=10
        ).scalar_boundary)
        self.assertEqual(2, hit_test_story_position_v1(
            caret_map=m,page_id="page:1",page_x_emu=19,page_y_emu=10
        ).scalar_boundary)
        self.assertEqual(3, hit_test_story_position_v1(
            caret_map=m,page_id="page:1",page_x_emu=100,page_y_emu=10
        ).scalar_boundary)

    def test_mandatory_cr_can_be_logically_covered_without_painted_rect(self):
        l1 = line(
            "l1", 0,
            (
                cluster(0,1,0,10),
                cluster(1,2,10,10,painted=False),
            ),
            next="l2",
            y=0,
        )
        l2 = line(
            "l2", 1,
            (cluster(2,3,0,10),),
            prev="l1",
            y=30,
        )
        m = build_resolved_text_caret_map_v1(
            layout_revision_id="layout:1",
            story_id="story:1",
            story_scalar_len=3,
            lines=(l1,l2),
        )
        geometry = selection_geometry_v1(
            caret_map=m,start_scalar=0,end_scalar=3
        )
        self.assertEqual("complete", geometry.coverage_state)
        self.assertEqual(((0,3),), tuple(
            (r.start_scalar,r.end_scalar) for r in geometry.covered_ranges
        ))
        self.assertEqual(2, len(geometry.rectangles))

    def test_linked_frame_order_comes_from_flow_not_geometry_sort(self):
        l1 = line(
            "l1", 0, (cluster(0,1,100,110),),
            frame="frame:A",next="l2",y=1000,
        )
        l2 = line(
            "l2", 1, (cluster(1,2,0,10),),
            frame="frame:B",prev="l1",y=0,
        )
        m = build_resolved_text_caret_map_v1(
            layout_revision_id="layout:1",
            story_id="story:1",
            story_scalar_len=2,
            lines=(l2,l1),
        )
        self.assertEqual(("l1","l2"), tuple(x.line_id for x in m.lines))
        geometry=selection_geometry_v1(caret_map=m,start_scalar=0,end_scalar=2)
        self.assertEqual(("l1","l2"), tuple(x.line_id for x in geometry.rectangles))

    def test_same_scalar_multi_stop_requires_grounded_stop_identity(self):
        l1=line("l1",0,(cluster(0,1,0,10),),next="l2",y=0)
        l2=line("l2",1,(cluster(1,2,0,10),),prev="l1",y=30)
        m=build_resolved_text_caret_map_v1(
            layout_revision_id="layout:1",
            story_id="story:1",
            story_scalar_len=2,
            lines=(l1,l2),
        )
        matches=[s for s in m.caret_stops if s.scalar_boundary==1]
        self.assertEqual(2,len(matches))
        with self.assertRaises(ResolvedTextCaretMapError) as caught:
            resolve_story_position_v1(caret_map=m,scalar_boundary=1)
        self.assertEqual("caret_affinity_required",caught.exception.code)
        chosen=resolve_story_position_v1(
            caret_map=m,scalar_boundary=1,stop_id=matches[1].stop_id
        )
        self.assertEqual(matches[1],chosen)

    def test_placed_to_overset_selection_is_partial_with_exact_unplaced_suffix(self):
        m=build_resolved_text_caret_map_v1(
            layout_revision_id="layout:1",
            story_id="story:1",
            story_scalar_len=5,
            lines=(line("l1",0,(
                cluster(0,1,0,10),
                cluster(1,2,10,20),
                cluster(2,3,20,30),
            )),),
        )
        geometry=selection_geometry_v1(caret_map=m,start_scalar=1,end_scalar=5)
        self.assertEqual("partial",geometry.coverage_state)
        self.assertEqual(((1,3),),tuple(
            (r.start_scalar,r.end_scalar) for r in geometry.covered_ranges
        ))
        self.assertEqual(((3,5),),tuple(
            (r.start_scalar,r.end_scalar) for r in geometry.unplaced_ranges
        ))

    def test_wholly_overset_nonempty_selection_is_unplaced_not_empty(self):
        m=build_resolved_text_caret_map_v1(
            layout_revision_id="layout:1",
            story_id="story:1",
            story_scalar_len=5,
            lines=(line("l1",0,(cluster(0,3,0,30),)),),
        )
        geometry=selection_geometry_v1(caret_map=m,start_scalar=3,end_scalar=5)
        self.assertFalse(geometry.is_empty)
        self.assertEqual("unplaced",geometry.coverage_state)
        self.assertEqual((),geometry.rectangles)
        self.assertEqual(((3,5),),tuple(
            (r.start_scalar,r.end_scalar) for r in geometry.unplaced_ranges
        ))

    def test_empty_selection_is_distinct_complete_empty_geometry(self):
        m=build_resolved_text_caret_map_v1(
            layout_revision_id="layout:1",
            story_id="story:1",
            story_scalar_len=1,
            lines=(line("l1",0,(cluster(0,1,0,10),)),),
        )
        geometry=selection_geometry_v1(caret_map=m,start_scalar=1,end_scalar=1)
        self.assertTrue(geometry.is_empty)
        self.assertEqual("complete",geometry.coverage_state)
        self.assertEqual((),geometry.unplaced_ranges)

    def test_reflow_revision_fence_rejects_stale_caret_map_use(self):
        m=build_resolved_text_caret_map_v1(
            layout_revision_id="layout:2",
            story_id="story:1",
            story_scalar_len=1,
            lines=(line("l1",0,(cluster(0,1,0,10),)),),
        )
        with self.assertRaises(ResolvedTextCaretMapError) as caught:
            resolve_story_position_v1(
                caret_map=m,
                scalar_boundary=0,
                expected_layout_revision_id="layout:1",
            )
        self.assertEqual("stale_layout_map",caught.exception.code)

    def test_input_line_enumeration_does_not_change_canonical_map_hash(self):
        l1=line("l1",0,(cluster(0,1,0,10),),next="l2",y=100)
        l2=line("l2",1,(cluster(1,2,0,10),),prev="l1",y=0)
        a=build_resolved_text_caret_map_v1(
            layout_revision_id="layout:1",
            story_id="story:1",
            story_scalar_len=2,
            lines=(l1,l2),
        )
        b=build_resolved_text_caret_map_v1(
            layout_revision_id="layout:1",
            story_id="story:1",
            story_scalar_len=2,
            lines=(l2,l1),
        )
        self.assertEqual(a,b)
        self.assertEqual(caret_map_hash_v1(a),caret_map_hash_v1(b))

    def test_invalid_story_flow_adjacency_fails_closed(self):
        with self.assertRaises(ResolvedTextCaretMapError) as caught:
            build_resolved_text_caret_map_v1(
                layout_revision_id="layout:1",
                story_id="story:1",
                story_scalar_len=2,
                lines=(
                    line("l1",0,(cluster(0,1,0,10),),next=None),
                    line("l2",1,(cluster(1,2,0,10),),prev="l1"),
                ),
            )
        self.assertEqual("invalid_layout",caught.exception.code)


if __name__=="__main__":
    unittest.main()
