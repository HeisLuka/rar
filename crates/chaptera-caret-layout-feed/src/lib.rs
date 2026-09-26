//! Projection from authoritative bounded shaped-flow into the canonical
//! ResolvedTextCaretMapV1 input contract.
//!
//! This crate is glue only. It never shapes text and never consults UI metrics.

use chaptera_text_caret_map_adapter::{
    build_resolved_text_caret_map_v1, CaretMapBuildInputV1, InternalCaretStopV1,
    ResolvedClusterV1, ResolvedLineFragmentV1, ResolvedTextCaretMapV1,
};
use pub_layout::{BoundedShapedFlowScene, BoundedShapedLine};
use pub_model::{Affine2D, CanonicalId, NodeId, StoryId};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaretLayoutFeedError {
    pub code: &'static str,
    pub message: String,
}

impl CaretLayoutFeedError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}

impl fmt::Display for CaretLayoutFeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for CaretLayoutFeedError {}

fn checked_add(a: i64, b: i64) -> Result<i64, CaretLayoutFeedError> {
    a.checked_add(b).ok_or_else(|| CaretLayoutFeedError::new("metric_overflow", "EMU addition overflow"))
}

fn checked_mul(a: i64, b: i64) -> Result<i64, CaretLayoutFeedError> {
    a.checked_mul(b).ok_or_else(|| CaretLayoutFeedError::new("metric_overflow", "EMU multiplication overflow"))
}

fn identity(transform: &Affine2D) -> bool {
    *transform == Affine2D::identity()
}

fn line_clusters(
    line: &BoundedShapedLine,
    frame_page_x: i64,
) -> Result<Vec<ResolvedClusterV1>, CaretLayoutFeedError> {
    if line.scalar_end < line.scalar_start || line.consumed_scalar_end < line.scalar_end {
        return Err(CaretLayoutFeedError::new("invalid_shaped_line", "line scalar ranges are not monotonic"));
    }

    let mut groups: Vec<(u32, i64)> = Vec::new();
    for glyph in &line.glyphs {
        if glyph.cluster < line.scalar_start || glyph.cluster >= line.scalar_end {
            return Err(CaretLayoutFeedError::new("invalid_glyph_cluster", "glyph cluster lies outside visible Story range"));
        }
        if glyph.x_advance.get() < 0 || glyph.y_advance.get() != 0 {
            return Err(CaretLayoutFeedError::new("unsupported_shaping", "V0 requires non-negative horizontal-only advances"));
        }
        match groups.last_mut() {
            Some((cluster, advance)) if *cluster == glyph.cluster => {
                *advance = checked_add(*advance, glyph.x_advance.get())?;
            }
            Some((cluster, _)) if glyph.cluster < *cluster => {
                return Err(CaretLayoutFeedError::new("invalid_glyph_cluster", "glyph cluster order is not monotonic"));
            }
            _ => groups.push((glyph.cluster, glyph.x_advance.get())),
        }
    }

    if line.scalar_start < line.scalar_end && groups.first().map(|v| v.0) != Some(line.scalar_start) {
        return Err(CaretLayoutFeedError::new("invalid_glyph_cluster", "first glyph cluster does not start at line scalar_start"));
    }

    let measured = groups.iter().try_fold(0_i64, |sum, (_, advance)| checked_add(sum, *advance))?;
    if measured != line.measured_width.get() {
        return Err(CaretLayoutFeedError::new("measured_width_mismatch", "grouped glyph advances disagree with shaped line width"));
    }

    let mut out = Vec::new();
    let mut x = 0_i64;
    for (index, (start, advance)) in groups.iter().copied().enumerate() {
        let end = groups.get(index + 1).map_or(line.scalar_end, |next| next.0);
        if end <= start {
            return Err(CaretLayoutFeedError::new("invalid_glyph_cluster", "cluster has non-positive scalar extent"));
        }
        let next_x = checked_add(x, advance)?;
        out.push(ResolvedClusterV1 {
            start_scalar: start,
            end_scalar: end,
            page_x_start_emu: checked_add(frame_page_x, x)?,
            page_x_end_emu: checked_add(frame_page_x, next_x)?,
            frame_x_start_emu: x,
            frame_x_end_emu: next_x,
            painted: true,
            // Internal positions are deliberately not guessed here. When the shaped-flow
            // producer carries explicit internal authority, a successor can project it.
            internal_caret_stops: Vec::<InternalCaretStopV1>::new(),
        });
        x = next_x;
    }

    if line.scalar_end < line.consumed_scalar_end {
        if line.consumed_scalar_end != line.scalar_end + 1 {
            return Err(CaretLayoutFeedError::new(
                "unsupported_delimiter",
                "V0 admits at most one consumed unpainted mandatory delimiter per line",
            ));
        }
        out.push(ResolvedClusterV1 {
            start_scalar: line.scalar_end,
            end_scalar: line.consumed_scalar_end,
            page_x_start_emu: checked_add(frame_page_x, x)?,
            page_x_end_emu: checked_add(frame_page_x, x)?,
            frame_x_start_emu: x,
            frame_x_end_emu: x,
            painted: false,
            internal_caret_stops: Vec::new(),
        });
    }

    if out.is_empty() {
        return Err(CaretLayoutFeedError::new("empty_resolved_line", "caret-map line requires at least one logical cluster"));
    }
    Ok(out)
}

pub fn build_caret_map_from_shaped_flow_v1(
    scene: &BoundedShapedFlowScene,
    layout_revision_id: &str,
    story_id: StoryId,
    story_scalar_len: u32,
) -> Result<ResolvedTextCaretMapV1, CaretLayoutFeedError> {
    if layout_revision_id.is_empty() {
        return Err(CaretLayoutFeedError::new("invalid_layout_revision", "layout_revision_id is required"));
    }

    let surface_ids: BTreeSet<CanonicalId> =
        scene.surfaces.iter().map(|surface| surface.origin.into_canonical()).collect();
    let nodes: BTreeMap<NodeId, _> = scene.nodes.iter().map(|node| (node.origin, node)).collect();

    let mut source_lines: Vec<&BoundedShapedLine> =
        scene.lines.iter().filter(|line| line.story_origin == story_id).collect();
    source_lines.sort_by_key(|line| (line.scalar_start, line.frame_origin, line.frame_line_index));

    let story_text_id = story_id.as_canonical().to_string();
    let line_height = scene.environment.line_height.get();
    if line_height <= 0 {
        return Err(CaretLayoutFeedError::new("invalid_line_height", "shaped-flow line height must be positive"));
    }

    let mut lines = Vec::with_capacity(source_lines.len());
    for (ordinal, source) in source_lines.iter().enumerate() {
        let frame = nodes.get(&source.frame_origin).ok_or_else(|| {
            CaretLayoutFeedError::new("missing_frame_geometry", "shaped line frame is absent from resolved nodes")
        })?;
        if !identity(&frame.transform) {
            return Err(CaretLayoutFeedError::new("unsupported_transform", "V0 caret feed requires identity frame transform"));
        }
        if !surface_ids.contains(&frame.parent_origin) {
            return Err(CaretLayoutFeedError::new("non_page_parent", "frame parent does not identify one resolved page surface"));
        }
        if frame.bounds.width.get() <= 0 || frame.bounds.height.get() <= 0 {
            return Err(CaretLayoutFeedError::new("invalid_frame_geometry", "frame bounds must have positive extent"));
        }

        let row_y = checked_mul(i64::from(source.frame_line_index), line_height)?;
        let row_bottom = checked_add(row_y, line_height)?;
        if row_bottom > frame.bounds.height.get() {
            return Err(CaretLayoutFeedError::new("line_outside_frame", "resolved shaped line exceeds frame height"));
        }
        let page_y_top = checked_add(frame.bounds.y.get(), row_y)?;
        let page_y_bottom = checked_add(frame.bounds.y.get(), row_bottom)?;
        let clusters = line_clusters(source, frame.bounds.x.get())?;

        let line_id = format!("{}:line:{ordinal}", story_text_id);
        lines.push(ResolvedLineFragmentV1 {
            story_id: story_text_id.clone(),
            page_id: frame.parent_origin.to_string(),
            frame_id: source.frame_origin.as_canonical().to_string(),
            line_id,
            flow_ordinal: u32::try_from(ordinal)
                .map_err(|_| CaretLayoutFeedError::new("line_count_overflow", "line ordinal exceeds u32"))?,
            previous_line_id: None,
            next_line_id: None,
            page_y_top_emu: page_y_top,
            page_y_bottom_emu: page_y_bottom,
            frame_y_top_emu: row_y,
            frame_y_bottom_emu: row_bottom,
            clusters,
        });
    }

    for index in 0..lines.len() {
        lines[index].previous_line_id = index.checked_sub(1).map(|i| lines[i].line_id.clone());
        lines[index].next_line_id = lines.get(index + 1).map(|line| line.line_id.clone());
    }

    build_resolved_text_caret_map_v1(CaretMapBuildInputV1 {
        layout_revision_id: layout_revision_id.to_owned(),
        story_id: story_text_id,
        story_scalar_len,
        lines,
    })
    .map_err(|error| CaretLayoutFeedError::new(error.code, error.message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pub_layout::{
        BoundedBreakKind, BoundedLayoutEnvironment, BoundedShapedFlowDescriptor,
        BoundedShapedGlyph, BoundedShapingDescriptor, ResolvedPhysicalNode, ResolvedSurface,
    };
    use pub_model::{CanonicalId, LengthEmu, NodeId, PageId, RectEmu, Size2D};

    fn id(byte: u8) -> CanonicalId { CanonicalId::from_bytes([byte; 16]) }
    fn story(byte: u8) -> StoryId { StoryId::from_canonical(id(byte)) }
    fn node(byte: u8) -> NodeId { NodeId::from_canonical(id(byte)) }
    fn page(byte: u8) -> PageId { PageId::from_canonical(id(byte)) }

    fn glyph(cluster: u32, advance: i64) -> BoundedShapedGlyph {
        BoundedShapedGlyph {
            glyph_id: cluster + 1,
            cluster,
            x_advance: LengthEmu::new(advance),
            y_advance: LengthEmu::ZERO,
            x_offset: LengthEmu::ZERO,
            y_offset: LengthEmu::ZERO,
            unsafe_to_break: false,
        }
    }

    fn scene(lines: Vec<BoundedShapedLine>) -> BoundedShapedFlowScene {
        BoundedShapedFlowScene {
            environment: BoundedShapedFlowDescriptor {
                shaping: BoundedShapingDescriptor {
                    layout: BoundedLayoutEnvironment {
                        engine_revision: "layout:test".into(),
                        font_set_fingerprint: "font:test".into(),
                        resource_fingerprint: "resources:none".into(),
                    },
                    face_index: 0,
                    font_size_emu: LengthEmu::new(1200),
                    shaper_revision: "shape:test".into(),
                },
                line_height: LengthEmu::new(20),
            },
            surfaces: vec![ResolvedSurface {
                origin: page(1),
                size: Size2D::new(LengthEmu::new(1000), LengthEmu::new(1000)),
                bleed: None,
                margins: None,
            }],
            nodes: vec![
                ResolvedPhysicalNode {
                    origin: node(10),
                    parent_origin: page(1).into_canonical(),
                    bounds: RectEmu::new(LengthEmu::new(100), LengthEmu::new(200), LengthEmu::new(300), LengthEmu::new(100)),
                    transform: Affine2D::identity(),
                },
                ResolvedPhysicalNode {
                    origin: node(11),
                    parent_origin: page(1).into_canonical(),
                    bounds: RectEmu::new(LengthEmu::new(500), LengthEmu::new(300), LengthEmu::new(300), LengthEmu::new(100)),
                    transform: Affine2D::identity(),
                },
            ],
            lines,
            origin_mapping: Vec::new(),
            line_origin_mapping: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn projects_exact_cluster_edges_and_explicit_overset_extent() {
        let source = BoundedShapedLine {
            story_origin: story(7), frame_origin: node(10), frame_line_index: 1,
            scalar_start: 0, scalar_end: 3, consumed_scalar_end: 3,
            text: "abc".into(), measured_width: LengthEmu::new(30),
            glyphs: vec![glyph(0,10), glyph(1,10), glyph(2,10)],
            break_kind: BoundedBreakKind::Allowed, reshaped_for_break: false,
        };
        let map = build_caret_map_from_shaped_flow_v1(&scene(vec![source]), "layout:r1", story(7), 5).unwrap();
        assert_eq!(map.story_scalar_len, 5);
        assert_eq!(map.materialized_ranges[0].start_scalar, 0);
        assert_eq!(map.materialized_ranges[0].end_scalar, 3);
        assert_eq!(map.lines[0].page_y_top_emu, 220);
        assert_eq!(map.lines[0].clusters[1].page_x_start_emu, 110);
    }

    #[test]
    fn multi_scalar_cluster_is_not_split_by_guessing() {
        let source = BoundedShapedLine {
            story_origin: story(7), frame_origin: node(10), frame_line_index: 0,
            scalar_start: 0, scalar_end: 3, consumed_scalar_end: 3,
            text: "fix".into(), measured_width: LengthEmu::new(30),
            glyphs: vec![glyph(0,20), glyph(2,10)],
            break_kind: BoundedBreakKind::Allowed, reshaped_for_break: false,
        };
        let map = build_caret_map_from_shaped_flow_v1(&scene(vec![source]), "layout:r1", story(7), 3).unwrap();
        assert_eq!((map.lines[0].clusters[0].start_scalar, map.lines[0].clusters[0].end_scalar), (0,2));
        assert!(map.lines[0].clusters[0].internal_caret_stops.is_empty());
    }

    #[test]
    fn linked_continuation_preserves_two_physical_stops_for_same_scalar() {
        let first = BoundedShapedLine {
            story_origin: story(7), frame_origin: node(10), frame_line_index: 0,
            scalar_start: 0, scalar_end: 1, consumed_scalar_end: 1,
            text: "a".into(), measured_width: LengthEmu::new(10), glyphs: vec![glyph(0,10)],
            break_kind: BoundedBreakKind::Allowed, reshaped_for_break: false,
        };
        let second = BoundedShapedLine {
            story_origin: story(7), frame_origin: node(11), frame_line_index: 0,
            scalar_start: 1, scalar_end: 2, consumed_scalar_end: 2,
            text: "b".into(), measured_width: LengthEmu::new(10), glyphs: vec![glyph(1,10)],
            break_kind: BoundedBreakKind::Allowed, reshaped_for_break: false,
        };
        let map = build_caret_map_from_shaped_flow_v1(&scene(vec![second, first]), "layout:r1", story(7), 2).unwrap();
        assert_eq!(map.caret_stops.iter().filter(|stop| stop.scalar_boundary == 1).count(), 2);
    }

    #[test]
    fn mandatory_delimiter_becomes_unpainted_zero_width_cluster() {
        let source = BoundedShapedLine {
            story_origin: story(7), frame_origin: node(10), frame_line_index: 0,
            scalar_start: 0, scalar_end: 1, consumed_scalar_end: 2,
            text: "a".into(), measured_width: LengthEmu::new(10), glyphs: vec![glyph(0,10)],
            break_kind: BoundedBreakKind::Mandatory, reshaped_for_break: false,
        };
        let map = build_caret_map_from_shaped_flow_v1(&scene(vec![source]), "layout:r1", story(7), 2).unwrap();
        let delimiter = &map.lines[0].clusters[1];
        assert!(!delimiter.painted);
        assert_eq!(delimiter.page_x_start_emu, delimiter.page_x_end_emu);
        assert_eq!((delimiter.start_scalar, delimiter.end_scalar), (1,2));
    }

    #[test]
    fn non_identity_transform_fails_closed() {
        let source = BoundedShapedLine {
            story_origin: story(7), frame_origin: node(10), frame_line_index: 0,
            scalar_start: 0, scalar_end: 1, consumed_scalar_end: 1,
            text: "a".into(), measured_width: LengthEmu::new(10), glyphs: vec![glyph(0,10)],
            break_kind: BoundedBreakKind::Allowed, reshaped_for_break: false,
        };
        let mut s = scene(vec![source]);
        s.nodes[0].transform.tx = LengthEmu::new(1);
        assert_eq!(
            build_caret_map_from_shaped_flow_v1(&s, "layout:r1", story(7), 1).unwrap_err().code,
            "unsupported_transform"
        );
    }
}
