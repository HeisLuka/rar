//! Deterministic source-neutral horizontal placement for already-resolved shaped lines.
//!
//! LAYOUT-PARAGRAPH-ALIGN-01 deliberately starts after shaping and line breaking.
//! It cannot alter Story ranges, glyph advances, frame geometry, break opportunities,
//! or overset. It only resolves the horizontal origin of each admitted visible line
//! inside its already-known content box.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{error::Error, fmt};

pub const PARAGRAPH_LINE_PLACEMENT_CONTRACT_V1: &str =
    "chaptera.layout-paragraph-line-placement.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParagraphAlignmentV1 {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutPlacementContextV1 {
    pub authoring_revision: String,
    pub layout_environment_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedLineInputV1 {
    pub line_index: usize,
    pub story_id: String,
    pub frame_node_id: String,
    pub frame_line_index: u32,
    pub scalar_start: u32,
    pub scalar_end: u32,
    pub content_leading_x_emu: i64,
    pub content_width_emu: i64,
    pub measured_width_emu: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParagraphLinePlacementInputV1 {
    pub context: LayoutPlacementContextV1,
    pub alignment: ParagraphAlignmentV1,
    pub story_overset: bool,
    pub lines: Vec<ResolvedLineInputV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedLinePlacementV1 {
    pub line_index: usize,
    pub story_id: String,
    pub frame_node_id: String,
    pub frame_line_index: u32,
    pub scalar_start: u32,
    pub scalar_end: u32,
    pub content_leading_x_emu: i64,
    pub content_width_emu: i64,
    pub measured_width_emu: i64,
    pub line_origin_x_emu: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParagraphLinePlacementSceneV1 {
    pub contract_version: String,
    pub context: LayoutPlacementContextV1,
    pub alignment: ParagraphAlignmentV1,
    pub story_overset: bool,
    pub lines: Vec<ResolvedLinePlacementV1>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinePlacementError {
    EmptyContextField(&'static str),
    NonCanonicalLineIndex { expected: usize, actual: usize },
    InvalidScalarRange { line_index: usize },
    NonPositiveContentWidth { line_index: usize, width_emu: i64 },
    NegativeMeasuredWidth { line_index: usize, width_emu: i64 },
    LineWiderThanContent {
        line_index: usize,
        measured_width_emu: i64,
        content_width_emu: i64,
    },
    OriginOverflow { line_index: usize },
    Serialization,
}

impl fmt::Display for LinePlacementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyContextField(field) => write!(f, "{field} must be non-empty"),
            Self::NonCanonicalLineIndex { expected, actual } => write!(
                f,
                "line index {actual} is not canonical position {expected}"
            ),
            Self::InvalidScalarRange { line_index } => {
                write!(f, "line {line_index} has scalar_end < scalar_start")
            }
            Self::NonPositiveContentWidth {
                line_index,
                width_emu,
            } => write!(
                f,
                "line {line_index} content width must be positive, got {width_emu} EMU"
            ),
            Self::NegativeMeasuredWidth {
                line_index,
                width_emu,
            } => write!(
                f,
                "line {line_index} measured width must be non-negative, got {width_emu} EMU"
            ),
            Self::LineWiderThanContent {
                line_index,
                measured_width_emu,
                content_width_emu,
            } => write!(
                f,
                "line {line_index} measured width {measured_width_emu} exceeds content width {content_width_emu}"
            ),
            Self::OriginOverflow { line_index } => {
                write!(f, "line {line_index} horizontal origin overflowed")
            }
            Self::Serialization => write!(f, "canonical placement serialization failed"),
        }
    }
}

impl Error for LinePlacementError {}

/// Resolve horizontal origins for already-admitted shaped lines.
///
/// Center uses integer floor toward the leading edge:
/// `floor((content_width - measured_width) / 2)`.
/// Because the remaining width is validated as non-negative, ordinary integer
/// division implements that rule exactly. Right uses the full remaining width.
/// Left preserves the content leading origin.
///
/// The input is a bounded one-paragraph placement contract: one explicit
/// alignment value is applied to every visible continuation line, including
/// lines in later linked frames. Overset is passed through unchanged.
pub fn resolve_paragraph_line_placement_v1(
    input: &ParagraphLinePlacementInputV1,
) -> Result<ParagraphLinePlacementSceneV1, LinePlacementError> {
    if input.context.authoring_revision.is_empty() {
        return Err(LinePlacementError::EmptyContextField("authoring_revision"));
    }
    if input.context.layout_environment_fingerprint.is_empty() {
        return Err(LinePlacementError::EmptyContextField(
            "layout_environment_fingerprint",
        ));
    }

    let mut lines = Vec::with_capacity(input.lines.len());
    for (expected, line) in input.lines.iter().enumerate() {
        if line.line_index != expected {
            return Err(LinePlacementError::NonCanonicalLineIndex {
                expected,
                actual: line.line_index,
            });
        }
        if line.scalar_end < line.scalar_start {
            return Err(LinePlacementError::InvalidScalarRange {
                line_index: line.line_index,
            });
        }
        if line.content_width_emu <= 0 {
            return Err(LinePlacementError::NonPositiveContentWidth {
                line_index: line.line_index,
                width_emu: line.content_width_emu,
            });
        }
        if line.measured_width_emu < 0 {
            return Err(LinePlacementError::NegativeMeasuredWidth {
                line_index: line.line_index,
                width_emu: line.measured_width_emu,
            });
        }
        if line.measured_width_emu > line.content_width_emu {
            return Err(LinePlacementError::LineWiderThanContent {
                line_index: line.line_index,
                measured_width_emu: line.measured_width_emu,
                content_width_emu: line.content_width_emu,
            });
        }

        let remaining = line.content_width_emu - line.measured_width_emu;
        let offset = match input.alignment {
            ParagraphAlignmentV1::Left => 0,
            ParagraphAlignmentV1::Center => remaining / 2,
            ParagraphAlignmentV1::Right => remaining,
        };
        let line_origin_x_emu = line
            .content_leading_x_emu
            .checked_add(offset)
            .ok_or(LinePlacementError::OriginOverflow {
                line_index: line.line_index,
            })?;

        lines.push(ResolvedLinePlacementV1 {
            line_index: line.line_index,
            story_id: line.story_id.clone(),
            frame_node_id: line.frame_node_id.clone(),
            frame_line_index: line.frame_line_index,
            scalar_start: line.scalar_start,
            scalar_end: line.scalar_end,
            content_leading_x_emu: line.content_leading_x_emu,
            content_width_emu: line.content_width_emu,
            measured_width_emu: line.measured_width_emu,
            line_origin_x_emu,
        });
    }

    Ok(ParagraphLinePlacementSceneV1 {
        contract_version: PARAGRAPH_LINE_PLACEMENT_CONTRACT_V1.to_owned(),
        context: input.context.clone(),
        alignment: input.alignment,
        story_overset: input.story_overset,
        lines,
    })
}

pub fn placement_scene_hash_v1(
    scene: &ParagraphLinePlacementSceneV1,
) -> Result<String, LinePlacementError> {
    let bytes = serde_json::to_vec(scene).map_err(|_| LinePlacementError::Serialization)?;
    let digest = Sha256::digest(bytes);
    Ok(format!("sha256:{digest:x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(
        index: usize,
        frame: &str,
        row: u32,
        start: u32,
        end: u32,
        leading: i64,
        content: i64,
        measured: i64,
    ) -> ResolvedLineInputV1 {
        ResolvedLineInputV1 {
            line_index: index,
            story_id: "story-1".to_owned(),
            frame_node_id: frame.to_owned(),
            frame_line_index: row,
            scalar_start: start,
            scalar_end: end,
            content_leading_x_emu: leading,
            content_width_emu: content,
            measured_width_emu: measured,
        }
    }

    fn input(
        alignment: ParagraphAlignmentV1,
        lines: Vec<ResolvedLineInputV1>,
    ) -> ParagraphLinePlacementInputV1 {
        ParagraphLinePlacementInputV1 {
            context: LayoutPlacementContextV1 {
                authoring_revision: "rev-42".to_owned(),
                layout_environment_fingerprint: "layout-env:fixture-v1".to_owned(),
            },
            alignment,
            story_overset: false,
            lines,
        }
    }

    #[test]
    fn left_preserves_leading_origin_and_exact_fit_is_zero_offset() {
        let scene = resolve_paragraph_line_placement_v1(&input(
            ParagraphAlignmentV1::Left,
            vec![line(0, "frame-a", 0, 0, 4, 100, 1000, 1000)],
        ))
        .unwrap();

        assert_eq!(scene.lines[0].line_origin_x_emu, 100);
        assert_eq!(scene.lines[0].scalar_start, 0);
        assert_eq!(scene.lines[0].scalar_end, 4);
        assert_eq!(scene.lines[0].measured_width_emu, 1000);
    }

    #[test]
    fn center_uses_explicit_floor_toward_leading_edge_for_odd_and_even_space() {
        let odd = resolve_paragraph_line_placement_v1(&input(
            ParagraphAlignmentV1::Center,
            vec![line(0, "frame-a", 0, 0, 4, 10, 101, 100)],
        ))
        .unwrap();
        let even = resolve_paragraph_line_placement_v1(&input(
            ParagraphAlignmentV1::Center,
            vec![line(0, "frame-a", 0, 0, 4, 10, 102, 100)],
        ))
        .unwrap();

        assert_eq!(odd.lines[0].line_origin_x_emu, 10);
        assert_eq!(even.lines[0].line_origin_x_emu, 11);
    }

    #[test]
    fn right_consumes_exact_remaining_width() {
        let scene = resolve_paragraph_line_placement_v1(&input(
            ParagraphAlignmentV1::Right,
            vec![line(0, "frame-a", 0, 0, 4, -20, 150, 100)],
        ))
        .unwrap();

        assert_eq!(scene.lines[0].line_origin_x_emu, 30);
    }

    #[test]
    fn multi_line_and_linked_frame_continuation_share_one_alignment_law() {
        let scene = resolve_paragraph_line_placement_v1(&input(
            ParagraphAlignmentV1::Right,
            vec![
                line(0, "frame-a", 0, 0, 5, 100, 500, 300),
                line(1, "frame-a", 1, 5, 9, 100, 500, 450),
                line(2, "frame-b", 0, 9, 13, 800, 700, 500),
            ],
        ))
        .unwrap();

        assert_eq!(
            scene
                .lines
                .iter()
                .map(|line| line.line_origin_x_emu)
                .collect::<Vec<_>>(),
            vec![300, 150, 1000]
        );
        assert_eq!(scene.lines[2].frame_node_id, "frame-b");
        assert_eq!((scene.lines[2].scalar_start, scene.lines[2].scalar_end), (9, 13));
    }

    #[test]
    fn overset_and_resolved_ranges_are_passed_through_not_recomputed() {
        let mut fixture = input(
            ParagraphAlignmentV1::Center,
            vec![
                line(0, "frame-a", 0, 0, 5, 0, 500, 300),
                line(1, "frame-b", 0, 5, 9, 0, 500, 400),
            ],
        );
        fixture.story_overset = true;

        let scene = resolve_paragraph_line_placement_v1(&fixture).unwrap();

        assert!(scene.story_overset);
        assert_eq!(
            scene
                .lines
                .iter()
                .map(|line| (line.scalar_start, line.scalar_end, line.measured_width_emu))
                .collect::<Vec<_>>(),
            vec![(0, 5, 300), (5, 9, 400)]
        );
    }

    #[test]
    fn same_revision_environment_and_input_produce_same_scene_hash() {
        let fixture = input(
            ParagraphAlignmentV1::Center,
            vec![line(0, "frame-a", 0, 0, 4, 20, 1000, 501)],
        );

        let left = resolve_paragraph_line_placement_v1(&fixture).unwrap();
        let right = resolve_paragraph_line_placement_v1(&fixture).unwrap();

        assert_eq!(left, right);
        assert_eq!(
            placement_scene_hash_v1(&left).unwrap(),
            placement_scene_hash_v1(&right).unwrap()
        );
    }

    #[test]
    fn wider_than_content_fails_closed_instead_of_changing_line_breaks() {
        let err = resolve_paragraph_line_placement_v1(&input(
            ParagraphAlignmentV1::Center,
            vec![line(0, "frame-a", 0, 0, 4, 0, 100, 101)],
        ))
        .unwrap_err();

        assert!(matches!(
            err,
            LinePlacementError::LineWiderThanContent {
                line_index: 0,
                measured_width_emu: 101,
                content_width_emu: 100
            }
        ));
    }
}
