//! Bounded authoring -> layout projection proof.
//!
//! This crate intentionally depends only on `pub-model`. It does not define a
//! universal OpenLayout schema and does not import CFB, Contents, Quill, Escher,
//! RawPublication, or SourceCapsule state.
//!
//! The types here exist to close LAYOUT-PROJECTION-01 on grounded semantic
//! families before a broader layout contract is stabilized.

use pub_model::{
    Affine2D, BoxEdges, CanonicalId, EffectiveTableGridV1, GroundedRulerGuide, LengthEmu, NodeId,
    Page, PageId, ParagraphId, PublisherGuideRole, RectEmu, RulerGuide, RulerGuideAxis,
    SimpleRectangularTable, SimpleTableCell, Size2D, Story, StoryFrame, StoryId, TableCellAddress,
    TableCellId, TextRunId,
};
use serde::{Deserialize, Serialize};

mod break_policy;
mod internal_caret;
mod resolve;
mod shaped_flow;
mod shaping;
mod table_layout;
mod text_flow;

pub use break_policy::{
    BOUNDED_BREAK_POLICY_REVISION, BoundedBreakCandidate, BoundedBreakKind, BoundedBreakPolicy,
    BoundedBreakPolicyError, break_policy_for_shaped_text,
};
pub use internal_caret::{
    GDEF_FORMAT1_AUTHORITY_SOURCE, INTERNAL_CARET_AUTHORITY_REVISION, InternalCaretAuthorityError,
    InternalCaretAuthorityResultV1, InternalCaretStopAuthorityV1, UnsupportedInternalCaretV1,
    resolve_internal_carets_ltr,
};
pub use resolve::{
    BoundedLayoutEnvironment, BoundedResolvedScene, ResolveBlocked, ResolveDiagnostic,
    ResolveSeverity, ResolvedPhysicalNode, ResolvedSurface, SceneOriginMapping,
    resolve_bounded_geometry,
};
pub use shaped_flow::{
    BoundedShapedFlowDescriptor, BoundedShapedFlowError, BoundedShapedFlowRuntime,
    BoundedShapedFlowScene, BoundedShapedLine, ShapedLineOriginMapping,
    resolve_bounded_shaped_flow,
};
pub use shaping::{
    BOUNDED_SHAPER_REVISION, BoundedShapeError, BoundedShapedGlyph, BoundedShapedText,
    BoundedShapingDescriptor, BoundedShapingRuntime, font_fingerprint_sha256, shape_bounded_ltr,
    shape_bounded_ltr_segment,
};
pub use table_layout::{
    BoundedResolvedTableCells, BoundedTableResolveError, BoundedUniformTableMetrics,
    ResolvedTableCell, TableCellOriginMapping, resolve_bounded_uniform_table_cells,
};
pub use text_flow::{
    BoundedTextFlowEnvironment, BoundedTextFlowScene, BoundedTextMetrics, ResolvedTextFragment,
    TextFragmentOriginMapping, resolve_bounded_text_flow,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedAuthoringSlice {
    pub pages: Vec<Page>,
    pub node_geometry: Vec<BoundedNodeGeometryInput>,
    pub stories: Vec<Story>,
    pub story_frames: Vec<StoryFrame<StoryId, NodeId>>,
    pub tables: Vec<BoundedTableInput>,
    pub guides: Vec<BoundedGuideInput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unknown_layout_state: Vec<UnknownLayoutState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedNodeGeometryInput {
    pub node_id: NodeId,
    pub parent_origin: CanonicalId,
    pub bounds: RectEmu,
    pub transform: Affine2D,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedTableInput {
    pub node_id: NodeId,
    pub table: SimpleRectangularTable<TableCellId>,
}

pub fn bounded_table_input_from_effective_grid(
    grid: &EffectiveTableGridV1,
) -> Option<BoundedTableInput> {
    grid.validate().ok()?;
    let table = SimpleRectangularTable::new(
        u32::try_from(grid.rows.len()).ok()?,
        u32::try_from(grid.columns.len()).ok()?,
        grid.cells
            .iter()
            .map(|cell| SimpleTableCell {
                id: cell.id,
                address: cell.address,
            })
            .collect(),
    )
    .ok()?;
    Some(BoundedTableInput {
        node_id: grid.table_id,
        table,
    })
}

pub fn uniform_metrics_from_effective_grid(
    grid: &EffectiveTableGridV1,
) -> Option<BoundedUniformTableMetrics> {
    grid.validate().ok()?;
    let first_row = grid.rows.first()?.extent?;
    let first_column = grid.columns.first()?.extent?;
    if !grid.rows.iter().all(|row| row.extent == Some(first_row))
        || !grid
            .columns
            .iter()
            .all(|column| column.extent == Some(first_column))
    {
        return None;
    }
    Some(BoundedUniformTableMetrics {
        table_origin: grid.table_id,
        cell_width: first_column,
        row_pitch: first_row,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedGuideInput {
    pub page_id: PageId,
    pub guide: RulerGuide<LengthEmu>,
    /// Semantic provenance only. Raw source carriers remain upstream.
    pub provenance: PublisherGuideRole,
}

impl From<GroundedRulerGuide<LengthEmu>> for BoundedGuideInput {
    fn from(value: GroundedRulerGuide<LengthEmu>) -> Self {
        Self {
            page_id: value.page_id,
            guide: value.guide,
            provenance: value.role,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownLayoutState {
    pub origin: CanonicalId,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedLayoutProjection {
    pub pages: Vec<ProjectedPage>,
    pub node_geometry: Vec<ProjectedNodeGeometry>,
    pub stories: Vec<ProjectedStory>,
    pub story_frames: Vec<ProjectedStoryFrame>,
    pub tables: Vec<ProjectedTable>,
    pub guides: Vec<ProjectedGuide>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<ProjectionDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedPage {
    pub origin: PageId,
    pub size: Size2D,
    pub bleed: Option<BoxEdges>,
    pub margins: Option<BoxEdges>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedNodeGeometry {
    pub origin: NodeId,
    pub parent_origin: CanonicalId,
    pub bounds: RectEmu,
    pub transform: Affine2D,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedStory {
    pub origin: StoryId,
    pub text: String,
    pub paragraph_origins: Vec<ParagraphId>,
    pub run_origins: Vec<TextRunId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedStoryFrame {
    pub story_origin: StoryId,
    pub frame_origin: NodeId,
    pub ordinal: u32,
    pub previous_frame_origin: Option<NodeId>,
    pub next_frame_origin: Option<NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedTable {
    pub origin: NodeId,
    pub rows: u32,
    pub columns: u32,
    pub cells: Vec<ProjectedTableCell>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedTableCell {
    pub origin: TableCellId,
    pub address: TableCellAddress,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedGuide {
    pub page_origin: PageId,
    pub axis: RulerGuideAxis,
    pub position: LengthEmu,
    pub provenance: PublisherGuideRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionSeverity {
    Error,
    FidelityWarning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionDiagnostic {
    pub code: String,
    pub severity: ProjectionSeverity,
    pub origin: CanonicalId,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProjectionMembershipStats {
    pub story_frame_count: u64,
    pub story_membership_comparisons: u64,
    pub frame_membership_comparisons: u64,
    pub additional_index_bytes: u64,
}

impl ProjectionMembershipStats {
    pub fn total_membership_comparisons(self) -> u64 {
        self.story_membership_comparisons
            .saturating_add(self.frame_membership_comparisons)
    }
}

/// Deterministically projects a deliberately small authoring slice.
///
/// Input vector order is not semantic. Output is normalized by stable canonical
/// identities and semantic coordinates so repeated projections are comparable.
pub fn project_bounded(input: BoundedAuthoringSlice) -> BoundedLayoutProjection {
    project_bounded_with_membership_stats(input).0
}

pub fn project_bounded_with_membership_stats(
    mut input: BoundedAuthoringSlice,
) -> (BoundedLayoutProjection, ProjectionMembershipStats) {
    input.pages.sort_by_key(|page| page.id);
    input.node_geometry.sort_by_key(|node| node.node_id);
    input.stories.sort_by_key(|story| story.id);
    input.story_frames.sort_by(|left, right| {
        (left.story_id, left.ordinal, left.frame_id).cmp(&(
            right.story_id,
            right.ordinal,
            right.frame_id,
        ))
    });
    input.tables.sort_by_key(|table| table.node_id);
    input.guides.sort_by(|left, right| {
        (
            left.page_id,
            guide_role_order(left.provenance),
            guide_axis_order(left.guide.axis),
            left.guide.position,
        )
            .cmp(&(
                right.page_id,
                guide_role_order(right.provenance),
                guide_axis_order(right.guide.axis),
                right.guide.position,
            ))
    });
    input.unknown_layout_state.sort_by(|left, right| {
        (left.origin, &left.description).cmp(&(right.origin, &right.description))
    });

    let mut diagnostics = Vec::new();

    let pages = input
        .pages
        .into_iter()
        .map(|page| {
            if let Err(error) = page.validate() {
                diagnostics.push(page_validation_diagnostic(&page, error));
            }

            ProjectedPage {
                origin: page.id,
                size: page.size,
                bleed: page.bleed,
                margins: page.margins,
            }
        })
        .collect();

    let node_geometry: Vec<_> = input
        .node_geometry
        .into_iter()
        .map(|node| ProjectedNodeGeometry {
            origin: node.node_id,
            parent_origin: node.parent_origin,
            bounds: node.bounds,
            transform: node.transform,
        })
        .collect();

    let stories: Vec<_> = input
        .stories
        .into_iter()
        .map(|story| ProjectedStory {
            origin: story.id,
            text: story.text,
            paragraph_origins: story.paragraphs,
            run_origins: story.runs,
        })
        .collect();

    let mut membership_stats = ProjectionMembershipStats {
        story_frame_count: input.story_frames.len() as u64,
        ..ProjectionMembershipStats::default()
    };

    for frame in &input.story_frames {
        if !sorted_story_contains(
            &stories,
            frame.story_id,
            &mut membership_stats.story_membership_comparisons,
        ) {
            diagnostics.push(ProjectionDiagnostic {
                code: "missing_story_content".into(),
                severity: ProjectionSeverity::Error,
                origin: frame.story_id.into_canonical(),
                message: "story frame references story content absent from projection".into(),
            });
        }

        if !sorted_node_geometry_contains(
            &node_geometry,
            frame.frame_id,
            &mut membership_stats.frame_membership_comparisons,
        ) {
            diagnostics.push(ProjectionDiagnostic {
                code: "missing_frame_geometry".into(),
                severity: ProjectionSeverity::Error,
                origin: frame.frame_id.into_canonical(),
                message: "story frame has no projected authored geometry".into(),
            });
        }
    }

    let story_frames = input
        .story_frames
        .into_iter()
        .map(|frame| ProjectedStoryFrame {
            story_origin: frame.story_id,
            frame_origin: frame.frame_id,
            ordinal: frame.ordinal,
            previous_frame_origin: frame.previous,
            next_frame_origin: frame.next,
        })
        .collect();

    let tables = input
        .tables
        .into_iter()
        .map(|table| {
            let mut cells: Vec<_> = table
                .table
                .cells
                .into_iter()
                .map(|cell| ProjectedTableCell {
                    origin: cell.id,
                    address: cell.address,
                })
                .collect();
            cells.sort_by_key(|cell| (cell.address.row, cell.address.column, cell.origin));

            ProjectedTable {
                origin: table.node_id,
                rows: table.table.rows,
                columns: table.table.columns,
                cells,
            }
        })
        .collect();

    let guides = input
        .guides
        .into_iter()
        .map(|guide| ProjectedGuide {
            page_origin: guide.page_id,
            axis: guide.guide.axis,
            position: guide.guide.position,
            provenance: guide.provenance,
        })
        .collect();

    diagnostics.extend(input.unknown_layout_state.into_iter().map(|unknown| {
        ProjectionDiagnostic {
            code: "unknown_layout_affecting_state".into(),
            severity: ProjectionSeverity::FidelityWarning,
            origin: unknown.origin,
            message: unknown.description,
        }
    }));

    (
        BoundedLayoutProjection {
            pages,
            node_geometry,
            stories,
            story_frames,
            tables,
            guides,
            diagnostics,
        },
        membership_stats,
    )
}

fn sorted_story_contains(
    stories: &[ProjectedStory],
    target: StoryId,
    comparisons: &mut u64,
) -> bool {
    let mut low = 0usize;
    let mut high = stories.len();
    while low < high {
        let mid = low + (high - low) / 2;
        *comparisons = (*comparisons).saturating_add(1);
        match stories[mid].origin.cmp(&target) {
            std::cmp::Ordering::Less => low = mid + 1,
            std::cmp::Ordering::Equal => return true,
            std::cmp::Ordering::Greater => high = mid,
        }
    }
    false
}

fn sorted_node_geometry_contains(
    nodes: &[ProjectedNodeGeometry],
    target: NodeId,
    comparisons: &mut u64,
) -> bool {
    let mut low = 0usize;
    let mut high = nodes.len();
    while low < high {
        let mid = low + (high - low) / 2;
        *comparisons = (*comparisons).saturating_add(1);
        match nodes[mid].origin.cmp(&target) {
            std::cmp::Ordering::Less => low = mid + 1,
            std::cmp::Ordering::Equal => return true,
            std::cmp::Ordering::Greater => high = mid,
        }
    }
    false
}

fn guide_role_order(role: PublisherGuideRole) -> u8 {
    match role {
        PublisherGuideRole::PublicationLayoutGuides => 0,
        PublisherGuideRole::PageRulerGuide => 1,
    }
}

fn guide_axis_order(axis: RulerGuideAxis) -> u8 {
    match axis {
        RulerGuideAxis::Horizontal => 0,
        RulerGuideAxis::Vertical => 1,
    }
}

fn page_validation_diagnostic(
    page: &Page,
    error: pub_model::PageValidationError,
) -> ProjectionDiagnostic {
    let message = match error {
        pub_model::PageValidationError::NonPositiveSize { width, height } => {
            format!(
                "page has non-positive size: width={} EMU height={} EMU",
                width.get(),
                height.get()
            )
        }
        pub_model::PageValidationError::DuplicateChild { child_id } => {
            format!("page contains duplicate child {}", child_id.as_canonical())
        }
    };

    ProjectionDiagnostic {
        code: "invalid_page_semantics".into(),
        severity: ProjectionSeverity::Error,
        origin: page.id.into_canonical(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pub_model::{CanonicalId, EMU_PER_MILLIMETER, SimpleTableCell, TableCellAddress};

    fn id(byte: u8) -> CanonicalId {
        CanonicalId::from_bytes([byte; 16])
    }

    fn page_id(byte: u8) -> PageId {
        PageId::from_canonical(id(byte))
    }

    fn node_id(byte: u8) -> NodeId {
        NodeId::from_canonical(id(byte))
    }

    fn story_id(byte: u8) -> StoryId {
        StoryId::from_canonical(id(byte))
    }

    fn table_cell_id(byte: u8) -> TableCellId {
        TableCellId::from_canonical(id(byte))
    }

    fn page(byte: u8) -> Page {
        Page {
            id: page_id(byte),
            size: Size2D::new(
                LengthEmu::new(90 * EMU_PER_MILLIMETER),
                LengthEmu::new(120 * EMU_PER_MILLIMETER),
            ),
            bleed: None,
            margins: None,
            children: Vec::new(),
            extensions: Vec::new(),
        }
    }

    fn table_input() -> BoundedTableInput {
        let table = SimpleRectangularTable::new(
            3,
            2,
            vec![
                SimpleTableCell {
                    id: table_cell_id(10),
                    address: TableCellAddress { row: 0, column: 0 },
                },
                SimpleTableCell {
                    id: table_cell_id(11),
                    address: TableCellAddress { row: 0, column: 1 },
                },
                SimpleTableCell {
                    id: table_cell_id(12),
                    address: TableCellAddress { row: 1, column: 0 },
                },
                SimpleTableCell {
                    id: table_cell_id(13),
                    address: TableCellAddress { row: 1, column: 1 },
                },
                SimpleTableCell {
                    id: table_cell_id(14),
                    address: TableCellAddress { row: 2, column: 0 },
                },
                SimpleTableCell {
                    id: table_cell_id(15),
                    address: TableCellAddress { row: 2, column: 1 },
                },
            ],
        )
        .expect("grounded 3x2 table");

        BoundedTableInput {
            node_id: node_id(9),
            table,
        }
    }

    fn fixture(order_reversed: bool) -> BoundedAuthoringSlice {
        let mut pages = vec![page(2), page(1)];
        let mut node_geometry = vec![
            BoundedNodeGeometryInput {
                node_id: node_id(31),
                parent_origin: page_id(1).into_canonical(),
                bounds: RectEmu::new(
                    LengthEmu::new(50 * EMU_PER_MILLIMETER),
                    LengthEmu::new(10 * EMU_PER_MILLIMETER),
                    LengthEmu::new(30 * EMU_PER_MILLIMETER),
                    LengthEmu::new(20 * EMU_PER_MILLIMETER),
                ),
                transform: Affine2D::identity(),
            },
            BoundedNodeGeometryInput {
                node_id: node_id(30),
                parent_origin: page_id(1).into_canonical(),
                bounds: RectEmu::new(
                    LengthEmu::new(10 * EMU_PER_MILLIMETER),
                    LengthEmu::new(10 * EMU_PER_MILLIMETER),
                    LengthEmu::new(30 * EMU_PER_MILLIMETER),
                    LengthEmu::new(20 * EMU_PER_MILLIMETER),
                ),
                transform: Affine2D::identity(),
            },
            BoundedNodeGeometryInput {
                node_id: node_id(29),
                parent_origin: page_id(1).into_canonical(),
                bounds: RectEmu::new(
                    LengthEmu::new(10 * EMU_PER_MILLIMETER),
                    LengthEmu::new(40 * EMU_PER_MILLIMETER),
                    LengthEmu::new(70 * EMU_PER_MILLIMETER),
                    LengthEmu::new(20 * EMU_PER_MILLIMETER),
                ),
                transform: Affine2D::identity(),
            },
        ];
        let mut stories = vec![Story {
            id: story_id(7),
            text: "Alpha beta gamma".into(),
            paragraphs: Vec::new(),
            runs: Vec::new(),
            fields: Vec::new(),
            hyperlinks: Vec::new(),
            source_refs: Vec::new(),
        }];
        let mut story_frames = vec![
            StoryFrame {
                story_id: story_id(7),
                frame_id: node_id(31),
                ordinal: 2,
                previous: Some(node_id(29)),
                next: None,
            },
            StoryFrame {
                story_id: story_id(7),
                frame_id: node_id(30),
                ordinal: 0,
                previous: None,
                next: Some(node_id(29)),
            },
            StoryFrame {
                story_id: story_id(7),
                frame_id: node_id(29),
                ordinal: 1,
                previous: Some(node_id(30)),
                next: Some(node_id(31)),
            },
        ];
        let mut guides = vec![
            BoundedGuideInput {
                page_id: page_id(1),
                guide: RulerGuide {
                    axis: RulerGuideAxis::Vertical,
                    position: LengthEmu::new(20 * EMU_PER_MILLIMETER),
                },
                provenance: PublisherGuideRole::PageRulerGuide,
            },
            BoundedGuideInput {
                page_id: page_id(1),
                guide: RulerGuide {
                    axis: RulerGuideAxis::Horizontal,
                    position: LengthEmu::new(15 * EMU_PER_MILLIMETER),
                },
                provenance: PublisherGuideRole::PublicationLayoutGuides,
            },
        ];

        if order_reversed {
            pages.reverse();
            node_geometry.reverse();
            stories.reverse();
            story_frames.reverse();
            guides.reverse();
        }

        BoundedAuthoringSlice {
            pages,
            node_geometry,
            stories,
            story_frames,
            tables: vec![table_input()],
            guides,
            unknown_layout_state: vec![UnknownLayoutState {
                origin: id(90),
                description: "unknown visual property may affect layout".into(),
            }],
        }
    }

    fn legacy_linear_membership_diagnostics(
        mut input: BoundedAuthoringSlice,
    ) -> Vec<ProjectionDiagnostic> {
        input.stories.sort_by_key(|story| story.id);
        input.node_geometry.sort_by_key(|node| node.node_id);
        input.story_frames.sort_by(|left, right| {
            (left.story_id, left.ordinal, left.frame_id).cmp(&(
                right.story_id,
                right.ordinal,
                right.frame_id,
            ))
        });

        let mut diagnostics = Vec::new();
        for frame in &input.story_frames {
            if !input.stories.iter().any(|story| story.id == frame.story_id) {
                diagnostics.push(ProjectionDiagnostic {
                    code: "missing_story_content".into(),
                    severity: ProjectionSeverity::Error,
                    origin: frame.story_id.into_canonical(),
                    message: "story frame references story content absent from projection".into(),
                });
            }
            if !input
                .node_geometry
                .iter()
                .any(|node| node.node_id == frame.frame_id)
            {
                diagnostics.push(ProjectionDiagnostic {
                    code: "missing_frame_geometry".into(),
                    severity: ProjectionSeverity::Error,
                    origin: frame.frame_id.into_canonical(),
                    message: "story frame has no projected authored geometry".into(),
                });
            }
        }
        diagnostics
    }

    #[test]
    fn indexed_membership_preserves_legacy_projection_bytes_and_diagnostics() {
        let mut input = fixture(false);
        input.stories.clear();
        input
            .node_geometry
            .retain(|node| node.node_id != node_id(29));

        let legacy_membership = legacy_linear_membership_diagnostics(input.clone());
        let (indexed, stats) = project_bounded_with_membership_stats(input);
        let indexed_membership: Vec<_> = indexed
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                matches!(
                    diagnostic.code.as_str(),
                    "missing_story_content" | "missing_frame_geometry"
                )
            })
            .cloned()
            .collect();
        assert_eq!(indexed_membership, legacy_membership);
        assert_eq!(stats.additional_index_bytes, 0);

        let mut legacy_projection = indexed.clone();
        let tail: Vec<_> = indexed
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                !matches!(
                    diagnostic.code.as_str(),
                    "missing_story_content" | "missing_frame_geometry"
                )
            })
            .cloned()
            .collect();
        legacy_projection.diagnostics = legacy_membership;
        legacy_projection.diagnostics.extend(tail);
        assert_eq!(
            serde_json::to_vec(&indexed).expect("serialize indexed projection"),
            serde_json::to_vec(&legacy_projection).expect("serialize legacy projection")
        );
    }

    #[test]
    fn indexed_membership_comparison_count_is_logarithmically_bounded() {
        let count = 200u8;
        let mut input = BoundedAuthoringSlice {
            pages: Vec::new(),
            node_geometry: Vec::new(),
            stories: Vec::new(),
            story_frames: Vec::new(),
            tables: Vec::new(),
            guides: Vec::new(),
            unknown_layout_state: Vec::new(),
        };
        for value in 1..=count {
            input.stories.push(Story {
                id: story_id(value),
                text: String::new(),
                paragraphs: Vec::new(),
                runs: Vec::new(),
                fields: Vec::new(),
                hyperlinks: Vec::new(),
                source_refs: Vec::new(),
            });
            input.node_geometry.push(BoundedNodeGeometryInput {
                node_id: node_id(value),
                parent_origin: id(250),
                bounds: RectEmu::new(
                    LengthEmu::new(0),
                    LengthEmu::new(0),
                    LengthEmu::new(1),
                    LengthEmu::new(1),
                ),
                transform: Affine2D::identity(),
            });
            input.story_frames.push(StoryFrame {
                story_id: story_id(value),
                frame_id: node_id(value),
                ordinal: 0,
                previous: None,
                next: None,
            });
        }

        let (_, stats) = project_bounded_with_membership_stats(input);
        let per_lookup_bound = 9u64;
        assert_eq!(stats.story_frame_count, u64::from(count));
        assert_eq!(stats.additional_index_bytes, 0);
        assert!(stats.total_membership_comparisons() <= u64::from(count) * per_lookup_bound * 2);
        assert!(stats.total_membership_comparisons() < u64::from(count) * u64::from(count));
    }

    #[test]
    fn projection_is_deterministic_across_input_order() {
        assert_eq!(
            project_bounded(fixture(false)),
            project_bounded(fixture(true))
        );
    }

    #[test]
    fn linked_story_projection_preserves_explicit_relations() {
        let projection = project_bounded(fixture(false));

        assert_eq!(projection.story_frames.len(), 3);
        assert_eq!(projection.story_frames[0].frame_origin, node_id(30));
        assert_eq!(
            projection.story_frames[0].next_frame_origin,
            Some(node_id(29))
        );
        assert_eq!(projection.story_frames[1].frame_origin, node_id(29));
        assert_eq!(
            projection.story_frames[1].previous_frame_origin,
            Some(node_id(30))
        );
        assert_eq!(
            projection.story_frames[1].next_frame_origin,
            Some(node_id(31))
        );
    }

    #[test]
    fn logical_story_content_crosses_without_source_refs() {
        let projection = project_bounded(fixture(false));

        assert_eq!(projection.stories.len(), 1);
        assert_eq!(projection.stories[0].origin, story_id(7));
        assert_eq!(projection.stories[0].text, "Alpha beta gamma");
        assert!(projection.stories[0].paragraph_origins.is_empty());
        assert!(projection.stories[0].run_origins.is_empty());
    }

    #[test]
    fn missing_story_content_is_an_explicit_error() {
        let mut input = fixture(false);
        input.stories.clear();

        let projection = project_bounded(input);

        assert!(projection.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "missing_story_content"
                && diagnostic.origin == story_id(7).into_canonical()
                && diagnostic.severity == ProjectionSeverity::Error
        }));
    }

    #[test]
    fn authored_frame_geometry_crosses_with_parent_origin() {
        let projection = project_bounded(fixture(false));

        let frame = projection
            .node_geometry
            .iter()
            .find(|node| node.origin == node_id(30))
            .expect("frame geometry should cross the layout boundary");

        assert_eq!(frame.parent_origin, page_id(1).into_canonical());
        assert_eq!(frame.bounds.x, LengthEmu::new(10 * EMU_PER_MILLIMETER));
        assert_eq!(frame.bounds.y, LengthEmu::new(10 * EMU_PER_MILLIMETER));
        assert_eq!(frame.transform, Affine2D::identity());
    }

    #[test]
    fn missing_story_frame_geometry_is_an_explicit_error() {
        let mut input = fixture(false);
        input
            .node_geometry
            .retain(|node| node.node_id != node_id(29));

        let projection = project_bounded(input);

        assert!(projection.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "missing_frame_geometry"
                && diagnostic.origin == node_id(29).into_canonical()
                && diagnostic.severity == ProjectionSeverity::Error
        }));
    }

    #[test]
    fn simple_table_projection_preserves_semantic_topology() {
        let projection = project_bounded(fixture(false));
        let table = &projection.tables[0];

        assert_eq!((table.rows, table.columns), (3, 2));
        assert_eq!(
            table
                .cells
                .iter()
                .map(|cell| (cell.address.row, cell.address.column))
                .collect::<Vec<_>>(),
            vec![(0, 0), (0, 1), (1, 0), (1, 1), (2, 0), (2, 1)]
        );
    }

    #[test]
    fn grounded_publication_and_page_guides_cross_with_semantic_provenance() {
        let projection = project_bounded(fixture(false));

        assert_eq!(projection.guides.len(), 2);
        assert_eq!(
            projection.guides[0].provenance,
            PublisherGuideRole::PublicationLayoutGuides
        );
        assert_eq!(projection.guides[0].axis, RulerGuideAxis::Horizontal);
        assert_eq!(
            projection.guides[0].position,
            LengthEmu::new(15 * EMU_PER_MILLIMETER)
        );
        assert_eq!(
            projection.guides[1].provenance,
            PublisherGuideRole::PageRulerGuide
        );
        assert_eq!(projection.guides[1].axis, RulerGuideAxis::Vertical);
        assert_eq!(
            projection.guides[1].position,
            LengthEmu::new(20 * EMU_PER_MILLIMETER)
        );
    }

    #[test]
    fn unknown_layout_state_becomes_diagnostic_not_raw_escape_hatch() {
        let projection = project_bounded(fixture(false));

        assert_eq!(projection.diagnostics.len(), 1);
        assert_eq!(
            projection.diagnostics[0].code,
            "unknown_layout_affecting_state"
        );
        assert_eq!(
            projection.diagnostics[0].severity,
            ProjectionSeverity::FidelityWarning
        );
    }

    #[test]
    fn projection_json_contains_origins_but_no_source_carriers() {
        let json = serde_json::to_string(&project_bounded(fixture(false)))
            .expect("bounded projection should serialize");

        assert!(json.contains("story_origin"));
        assert!(json.contains("frame_origin"));
        assert!(!json.contains("source_refs"));
        assert!(!json.contains("byte_range"));
        assert!(!json.contains("private_state_ref"));
    }
}

#[cfg(test)]
mod effective_grid_projection_tests {
    use super::*;
    use pub_model::{
        CanonicalId, EFFECTIVE_TABLE_GRID_V1, EffectiveTableCellV1, EffectiveTableGridV1,
        EffectiveTableTrackV1, TableColumnId, TableRowId,
    };

    fn canonical(byte: u8) -> CanonicalId {
        CanonicalId::from_bytes([byte; 16])
    }

    fn node_id(byte: u8) -> NodeId {
        NodeId::from_canonical(canonical(byte))
    }

    fn row_id(byte: u8) -> TableRowId {
        TableRowId::from_canonical(canonical(byte))
    }

    fn column_id(byte: u8) -> TableColumnId {
        TableColumnId::from_canonical(canonical(byte))
    }

    fn cell_id(byte: u8) -> TableCellId {
        TableCellId::from_canonical(canonical(byte))
    }

    fn grid(row_extent: Option<i64>, column_extent: Option<i64>) -> EffectiveTableGridV1 {
        let rows = vec![
            EffectiveTableTrackV1 {
                id: row_id(1),
                index: 0,
                extent: row_extent.map(LengthEmu::new),
            },
            EffectiveTableTrackV1 {
                id: row_id(2),
                index: 1,
                extent: row_extent.map(LengthEmu::new),
            },
        ];
        let columns = vec![
            EffectiveTableTrackV1 {
                id: column_id(3),
                index: 0,
                extent: column_extent.map(LengthEmu::new),
            },
            EffectiveTableTrackV1 {
                id: column_id(4),
                index: 1,
                extent: column_extent.map(LengthEmu::new),
            },
        ];
        let cells = vec![
            EffectiveTableCellV1 {
                id: cell_id(10),
                row_id: rows[0].id,
                column_id: columns[0].id,
                address: TableCellAddress { row: 0, column: 0 },
                row_span: 1,
                column_span: 1,
                story_id: None,
                utf16_start: None,
                utf16_end: None,
            },
            EffectiveTableCellV1 {
                id: cell_id(11),
                row_id: rows[0].id,
                column_id: columns[1].id,
                address: TableCellAddress { row: 0, column: 1 },
                row_span: 1,
                column_span: 1,
                story_id: None,
                utf16_start: None,
                utf16_end: None,
            },
            EffectiveTableCellV1 {
                id: cell_id(12),
                row_id: rows[1].id,
                column_id: columns[0].id,
                address: TableCellAddress { row: 1, column: 0 },
                row_span: 1,
                column_span: 1,
                story_id: None,
                utf16_start: None,
                utf16_end: None,
            },
            EffectiveTableCellV1 {
                id: cell_id(13),
                row_id: rows[1].id,
                column_id: columns[1].id,
                address: TableCellAddress { row: 1, column: 1 },
                row_span: 1,
                column_span: 1,
                story_id: None,
                utf16_start: None,
                utf16_end: None,
            },
        ];
        EffectiveTableGridV1 {
            version: EFFECTIVE_TABLE_GRID_V1.into(),
            table_id: node_id(7),
            rows,
            columns,
            cells,
        }
    }

    #[test]
    fn effective_grid_projects_without_changing_cell_identity() {
        let grid = grid(Some(200), Some(300));
        let bounded = bounded_table_input_from_effective_grid(&grid).expect("valid grid");
        assert_eq!(bounded.node_id, node_id(7));
        assert_eq!(bounded.table.rows, 2);
        assert_eq!(bounded.table.columns, 2);
        assert!(
            bounded
                .table
                .cells
                .iter()
                .any(|cell| cell.id == cell_id(13))
        );
    }

    #[test]
    fn uniform_known_extents_bridge_to_existing_table_metrics() {
        let grid = grid(Some(200), Some(300));
        let metrics = uniform_metrics_from_effective_grid(&grid).expect("known uniform metrics");
        assert_eq!(metrics.table_origin, node_id(7));
        assert_eq!(metrics.row_pitch, LengthEmu::new(200));
        assert_eq!(metrics.cell_width, LengthEmu::new(300));
    }

    #[test]
    fn unknown_track_metrics_do_not_become_equal_split_inference() {
        let grid = grid(None, None);
        assert!(uniform_metrics_from_effective_grid(&grid).is_none());
    }

    #[test]
    fn nonuniform_tracks_do_not_claim_uniform_layout_metrics() {
        let mut grid = grid(Some(200), Some(300));
        grid.columns[1].extent = Some(LengthEmu::new(301));
        assert!(uniform_metrics_from_effective_grid(&grid).is_none());
    }
}
