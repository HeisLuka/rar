//! Source-neutral derived caret/selection geometry for authoritative resolved text layout.
//!
//! This crate does not shape text, break lines, parse Publisher bytes, or persist
//! selection. It consumes already-resolved horizontal-LTR line records with
//! Story-global Unicode-scalar cluster provenance and derives interaction-facing
//! caret stops and selection geometry.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const CARET_MAP_VERSION_V1: &str = "chaptera.resolved-text-caret-map.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmuPointV1 {
    pub x: i64,
    pub y: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmuRectV1 {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScalarRangeV1 {
    pub start: u32,
    pub end: u32,
}

impl ScalarRangeV1 {
    pub fn new(start: u32, end: u32) -> Result<Self, CaretMapError> {
        if start > end {
            return Err(CaretMapError::InvalidScalarRange { start, end });
        }
        Ok(Self { start, end })
    }

    fn is_empty(self) -> bool {
        self.start == self.end
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryExtentInputV1 {
    pub story_id: String,
    pub scalar_len: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedGlyphInputV1 {
    /// Story-global Unicode-scalar cluster start.
    pub cluster_start: u32,
    pub x_advance_emu: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedTextLineInputV1 {
    pub story_id: String,
    pub page_id: String,
    pub frame_id: String,
    /// Authoritative Story-flow order. Geometry sorting is never used as flow authority.
    pub story_line_order: u32,
    pub frame_line_index: u32,
    /// Visible shaped scalar range, excluding an unpainted mandatory delimiter.
    pub scalar_start: u32,
    pub scalar_end: u32,
    /// Logical consumed position, including a mandatory paragraph delimiter.
    pub consumed_scalar_end: u32,
    /// Exact page-local line origin from authoritative layout.
    pub page_origin_emu: EmuPointV1,
    /// Exact page-local frame origin, used only to derive frame-local output.
    pub frame_origin_emu: EmuPointV1,
    /// Exact authoritative line slot extent.
    pub line_extent_width_emu: i64,
    pub line_height_emu: i64,
    /// Sum of shaped glyph advances for the visible line.
    pub measured_width_emu: i64,
    /// Glyph stream in authoritative visual order for horizontal LTR V1.
    pub glyphs: Vec<ResolvedGlyphInputV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedTextLayoutInputV1 {
    pub layout_environment_id: String,
    pub stories: Vec<StoryExtentInputV1>,
    pub lines: Vec<ResolvedTextLineInputV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaretAffinityV1 {
    Upstream,
    Downstream,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaretStopV1 {
    pub stop_id: String,
    pub story_id: String,
    pub page_id: String,
    pub frame_id: String,
    pub line_id: String,
    pub story_line_order: u32,
    pub scalar: u32,
    pub affinity: CaretAffinityV1,
    pub logical_only: bool,
    pub page_position_emu: EmuPointV1,
    pub frame_position_emu: EmuPointV1,
    pub line_height_emu: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedCaretLineV1 {
    pub line_id: String,
    pub story_id: String,
    pub page_id: String,
    pub frame_id: String,
    pub story_line_order: u32,
    pub frame_line_index: u32,
    pub previous_line_id: Option<String>,
    pub next_line_id: Option<String>,
    pub scalar_start: u32,
    pub scalar_end: u32,
    pub consumed_scalar_end: u32,
    pub page_extent_emu: EmuRectV1,
    pub frame_extent_emu: EmuRectV1,
    pub measured_width_emu: i64,
    /// Scalar boundaries inside a shaping cluster that V1 must not fabricate.
    pub unsupported_internal_scalars: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedTextCaretMapV1 {
    pub version: String,
    pub layout_environment_id: String,
    pub stories: Vec<StoryExtentInputV1>,
    pub lines: Vec<ResolvedCaretLineV1>,
    pub stops: Vec<CaretStopV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryPositionV1 {
    pub story_id: String,
    pub scalar: u32,
    pub line_id: String,
    pub stop_id: String,
    pub affinity: CaretAffinityV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionCoverageStateV1 {
    Complete,
    Partial,
    Unplaced,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionFragmentV1 {
    pub story_id: String,
    pub page_id: String,
    pub frame_id: String,
    pub line_id: String,
    pub story_line_order: u32,
    pub scalar_range: ScalarRangeV1,
    pub page_rect_emu: EmuRectV1,
    pub frame_rect_emu: EmuRectV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionGeometryV1 {
    pub story_id: String,
    pub semantic_range: ScalarRangeV1,
    pub coverage_state: SelectionCoverageStateV1,
    pub fragments: Vec<SelectionFragmentV1>,
    pub covered_ranges: Vec<ScalarRangeV1>,
    pub unplaced_ranges: Vec<ScalarRangeV1>,
    pub unsupported_internal_boundaries: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaretMapError {
    InvalidIdentifier {
        field: &'static str,
        value: String,
    },
    EmptyLayoutEnvironment,
    DuplicateStory {
        story_id: String,
    },
    UnknownStory {
        story_id: String,
    },
    DuplicateStoryLineOrder {
        story_id: String,
        order: u32,
    },
    NonContiguousStoryLineOrder {
        story_id: String,
        expected: u32,
        actual: u32,
    },
    InvalidScalarRange {
        start: u32,
        end: u32,
    },
    LineOutsideStory {
        story_id: String,
    },
    InvalidLineMetrics,
    InvalidGlyphAdvance,
    InvalidGlyphClusterOrder,
    GlyphClusterOutsideLine,
    MeasuredWidthMismatch {
        expected: i64,
        actual: i64,
    },
    MetricOverflow,
    NoLineOnPage {
        page_id: String,
    },
    StoryPositionUnplaced {
        story_id: String,
        scalar: u32,
    },
    InternalClusterUnsupported {
        story_id: String,
        scalar: u32,
    },
    CaretAffinityRequired {
        story_id: String,
        scalar: u32,
    },
    CaretAffinityUnavailable {
        story_id: String,
        scalar: u32,
    },
}

impl fmt::Display for CaretMapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier { field, value } => {
                write!(
                    formatter,
                    "{field} is not a canonical lowercase UUID: {value}"
                )
            }
            Self::EmptyLayoutEnvironment => write!(formatter, "layout environment id is required"),
            Self::DuplicateStory { story_id } => write!(formatter, "duplicate story {story_id}"),
            Self::UnknownStory { story_id } => write!(formatter, "unknown story {story_id}"),
            Self::DuplicateStoryLineOrder { story_id, order } => {
                write!(formatter, "duplicate story line order {story_id}:{order}")
            }
            Self::NonContiguousStoryLineOrder {
                story_id,
                expected,
                actual,
            } => {
                write!(
                    formatter,
                    "story line order is not contiguous for {story_id}: expected {expected}, got {actual}"
                )
            }
            Self::InvalidScalarRange { start, end } => {
                write!(formatter, "invalid scalar range {start}..{end}")
            }
            Self::LineOutsideStory { story_id } => {
                write!(formatter, "resolved line range exceeds story {story_id}")
            }
            Self::InvalidLineMetrics => write!(formatter, "resolved line metrics must be positive"),
            Self::InvalidGlyphAdvance => {
                write!(formatter, "LTR glyph advance must be non-negative")
            }
            Self::InvalidGlyphClusterOrder => {
                write!(formatter, "glyph cluster order must be non-decreasing")
            }
            Self::GlyphClusterOutsideLine => {
                write!(
                    formatter,
                    "glyph cluster lies outside the visible line range"
                )
            }
            Self::MeasuredWidthMismatch { expected, actual } => {
                write!(
                    formatter,
                    "measured line width mismatch: expected {expected} EMU, got {actual} EMU"
                )
            }
            Self::MetricOverflow => write!(formatter, "EMU metric arithmetic overflowed"),
            Self::NoLineOnPage { page_id } => {
                write!(formatter, "no resolved text line on page {page_id}")
            }
            Self::StoryPositionUnplaced { story_id, scalar } => {
                write!(formatter, "story position is unplaced: {story_id}:{scalar}")
            }
            Self::InternalClusterUnsupported { story_id, scalar } => {
                write!(
                    formatter,
                    "story position is inside a shaping cluster without internal caret authority: {story_id}:{scalar}"
                )
            }
            Self::CaretAffinityRequired { story_id, scalar } => {
                write!(
                    formatter,
                    "multiple physical caret stops exist; affinity is required: {story_id}:{scalar}"
                )
            }
            Self::CaretAffinityUnavailable { story_id, scalar } => {
                write!(
                    formatter,
                    "requested affinity has no physical caret stop: {story_id}:{scalar}"
                )
            }
        }
    }
}

impl std::error::Error for CaretMapError {}

fn canonical_uuid(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }
    value.bytes().enumerate().all(|(index, byte)| {
        if matches!(index, 8 | 13 | 18 | 23) {
            byte == b'-'
        } else {
            byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
        }
    })
}

fn require_uuid(field: &'static str, value: &str) -> Result<(), CaretMapError> {
    if canonical_uuid(value) {
        Ok(())
    } else {
        Err(CaretMapError::InvalidIdentifier {
            field,
            value: value.to_owned(),
        })
    }
}

fn line_id(story_id: &str, order: u32) -> String {
    format!("story-line-v1:{story_id}:{order:08}")
}

fn stop_id(line_id: &str, scalar: u32, affinity: CaretAffinityV1, logical_only: bool) -> String {
    let affinity = match affinity {
        CaretAffinityV1::Upstream => "upstream",
        CaretAffinityV1::Downstream => "downstream",
    };
    let kind = if logical_only { "logical" } else { "visual" };
    format!("{line_id}:{scalar:08}:{affinity}:{kind}")
}

fn checked_sub(left: i64, right: i64) -> Result<i64, CaretMapError> {
    left.checked_sub(right).ok_or(CaretMapError::MetricOverflow)
}

fn checked_add(left: i64, right: i64) -> Result<i64, CaretMapError> {
    left.checked_add(right).ok_or(CaretMapError::MetricOverflow)
}

fn make_stop(
    line: &ResolvedTextLineInputV1,
    line_id_value: &str,
    scalar: u32,
    affinity: CaretAffinityV1,
    logical_only: bool,
    x_offset_emu: i64,
) -> Result<CaretStopV1, CaretMapError> {
    let page_x = checked_add(line.page_origin_emu.x, x_offset_emu)?;
    let frame_x = checked_sub(page_x, line.frame_origin_emu.x)?;
    let frame_y = checked_sub(line.page_origin_emu.y, line.frame_origin_emu.y)?;
    Ok(CaretStopV1 {
        stop_id: stop_id(line_id_value, scalar, affinity, logical_only),
        story_id: line.story_id.clone(),
        page_id: line.page_id.clone(),
        frame_id: line.frame_id.clone(),
        line_id: line_id_value.to_owned(),
        story_line_order: line.story_line_order,
        scalar,
        affinity,
        logical_only,
        page_position_emu: EmuPointV1 {
            x: page_x,
            y: line.page_origin_emu.y,
        },
        frame_position_emu: EmuPointV1 {
            x: frame_x,
            y: frame_y,
        },
        line_height_emu: line.line_height_emu,
    })
}

fn line_stops(
    line: &ResolvedTextLineInputV1,
    line_id_value: &str,
) -> Result<(Vec<CaretStopV1>, Vec<u32>), CaretMapError> {
    if line.line_extent_width_emu <= 0 || line.line_height_emu <= 0 || line.measured_width_emu < 0 {
        return Err(CaretMapError::InvalidLineMetrics);
    }
    if line.scalar_start > line.scalar_end || line.scalar_end > line.consumed_scalar_end {
        return Err(CaretMapError::InvalidScalarRange {
            start: line.scalar_start,
            end: line.consumed_scalar_end,
        });
    }

    if line.scalar_start == line.scalar_end {
        if !line.glyphs.is_empty() || line.measured_width_emu != 0 {
            return Err(CaretMapError::MeasuredWidthMismatch {
                expected: 0,
                actual: line.measured_width_emu,
            });
        }
        let mut stops = vec![make_stop(
            line,
            line_id_value,
            line.scalar_start,
            CaretAffinityV1::Downstream,
            false,
            0,
        )?];
        if line.consumed_scalar_end > line.scalar_end {
            stops.push(make_stop(
                line,
                line_id_value,
                line.consumed_scalar_end,
                CaretAffinityV1::Upstream,
                true,
                0,
            )?);
        }
        return Ok((stops, Vec::new()));
    }

    if line.glyphs.is_empty() {
        return Err(CaretMapError::MeasuredWidthMismatch {
            expected: line.measured_width_emu,
            actual: 0,
        });
    }

    let mut groups: Vec<(u32, i64)> = Vec::new();
    let mut previous_cluster = None;
    let mut measured = 0_i64;

    for glyph in &line.glyphs {
        if glyph.x_advance_emu < 0 {
            return Err(CaretMapError::InvalidGlyphAdvance);
        }
        if glyph.cluster_start < line.scalar_start || glyph.cluster_start >= line.scalar_end {
            return Err(CaretMapError::GlyphClusterOutsideLine);
        }
        if let Some(previous) = previous_cluster {
            if glyph.cluster_start < previous {
                return Err(CaretMapError::InvalidGlyphClusterOrder);
            }
        }
        previous_cluster = Some(glyph.cluster_start);
        measured = checked_add(measured, glyph.x_advance_emu)?;

        match groups.last_mut() {
            Some((cluster, advance)) if *cluster == glyph.cluster_start => {
                *advance = checked_add(*advance, glyph.x_advance_emu)?;
            }
            _ => groups.push((glyph.cluster_start, glyph.x_advance_emu)),
        }
    }

    if measured != line.measured_width_emu {
        return Err(CaretMapError::MeasuredWidthMismatch {
            expected: line.measured_width_emu,
            actual: measured,
        });
    }
    if groups.first().map(|group| group.0) != Some(line.scalar_start) {
        return Err(CaretMapError::GlyphClusterOutsideLine);
    }

    let mut stops = Vec::new();
    let mut unsupported = Vec::new();
    let mut x = 0_i64;

    for (index, (cluster_start, advance)) in groups.iter().enumerate() {
        let cluster_end = groups.get(index + 1).map_or(line.scalar_end, |next| next.0);
        if cluster_end <= *cluster_start || cluster_end > line.scalar_end {
            return Err(CaretMapError::InvalidGlyphClusterOrder);
        }

        stops.push(make_stop(
            line,
            line_id_value,
            *cluster_start,
            CaretAffinityV1::Downstream,
            false,
            x,
        )?);

        for scalar in cluster_start.saturating_add(1)..cluster_end {
            unsupported.push(scalar);
        }

        x = checked_add(x, *advance)?;
    }

    // Internal cluster boundaries appear as the start stop of the next cluster.
    // The final visible line boundary is explicitly upstream so a soft-wrap
    // continuation can expose a distinct downstream stop at the same scalar.
    stops.retain(|stop| stop.scalar != line.scalar_end);
    stops.push(make_stop(
        line,
        line_id_value,
        line.scalar_end,
        CaretAffinityV1::Upstream,
        false,
        x,
    )?);

    if line.consumed_scalar_end > line.scalar_end {
        stops.push(make_stop(
            line,
            line_id_value,
            line.consumed_scalar_end,
            CaretAffinityV1::Upstream,
            true,
            x,
        )?);
    }

    Ok((stops, unsupported))
}

pub fn build_caret_map(
    mut input: ResolvedTextLayoutInputV1,
) -> Result<ResolvedTextCaretMapV1, CaretMapError> {
    if input.layout_environment_id.is_empty() {
        return Err(CaretMapError::EmptyLayoutEnvironment);
    }

    input
        .stories
        .sort_by(|left, right| left.story_id.cmp(&right.story_id));
    let mut story_lengths = BTreeMap::new();
    for story in &input.stories {
        require_uuid("story_id", &story.story_id)?;
        if story_lengths
            .insert(story.story_id.clone(), story.scalar_len)
            .is_some()
        {
            return Err(CaretMapError::DuplicateStory {
                story_id: story.story_id.clone(),
            });
        }
    }

    input.lines.sort_by(|left, right| {
        (
            &left.story_id,
            left.story_line_order,
            &left.frame_id,
            left.frame_line_index,
        )
            .cmp(&(
                &right.story_id,
                right.story_line_order,
                &right.frame_id,
                right.frame_line_index,
            ))
    });

    let mut by_story_orders: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    for line in &input.lines {
        require_uuid("story_id", &line.story_id)?;
        require_uuid("page_id", &line.page_id)?;
        require_uuid("frame_id", &line.frame_id)?;
        let Some(story_len) = story_lengths.get(&line.story_id) else {
            return Err(CaretMapError::UnknownStory {
                story_id: line.story_id.clone(),
            });
        };
        if line.scalar_start > line.scalar_end
            || line.scalar_end > line.consumed_scalar_end
            || line.consumed_scalar_end > *story_len
        {
            return Err(CaretMapError::LineOutsideStory {
                story_id: line.story_id.clone(),
            });
        }
        let inserted = by_story_orders
            .entry(line.story_id.clone())
            .or_default()
            .insert(line.story_line_order);
        if !inserted {
            return Err(CaretMapError::DuplicateStoryLineOrder {
                story_id: line.story_id.clone(),
                order: line.story_line_order,
            });
        }
    }

    for (story_id, orders) in &by_story_orders {
        for (expected, actual) in (0_u32..).zip(orders.iter().copied()) {
            if expected != actual {
                return Err(CaretMapError::NonContiguousStoryLineOrder {
                    story_id: story_id.clone(),
                    expected,
                    actual,
                });
            }
        }
    }

    let mut story_line_ids: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in &input.lines {
        story_line_ids
            .entry(line.story_id.clone())
            .or_default()
            .push(line_id(&line.story_id, line.story_line_order));
    }

    let mut lines = Vec::with_capacity(input.lines.len());
    let mut stops = Vec::new();

    for line in &input.lines {
        let id = line_id(&line.story_id, line.story_line_order);
        let ids =
            story_line_ids
                .get(&line.story_id)
                .ok_or_else(|| CaretMapError::UnknownStory {
                    story_id: line.story_id.clone(),
                })?;
        let order =
            usize::try_from(line.story_line_order).map_err(|_| CaretMapError::MetricOverflow)?;
        let previous_line_id = order
            .checked_sub(1)
            .and_then(|index| ids.get(index))
            .cloned();
        let next_line_id = ids.get(order.saturating_add(1)).cloned();
        let (mut line_stops, unsupported_internal_scalars) = line_stops(line, &id)?;

        let frame_x = checked_sub(line.page_origin_emu.x, line.frame_origin_emu.x)?;
        let frame_y = checked_sub(line.page_origin_emu.y, line.frame_origin_emu.y)?;

        lines.push(ResolvedCaretLineV1 {
            line_id: id,
            story_id: line.story_id.clone(),
            page_id: line.page_id.clone(),
            frame_id: line.frame_id.clone(),
            story_line_order: line.story_line_order,
            frame_line_index: line.frame_line_index,
            previous_line_id,
            next_line_id,
            scalar_start: line.scalar_start,
            scalar_end: line.scalar_end,
            consumed_scalar_end: line.consumed_scalar_end,
            page_extent_emu: EmuRectV1 {
                x: line.page_origin_emu.x,
                y: line.page_origin_emu.y,
                width: line.line_extent_width_emu,
                height: line.line_height_emu,
            },
            frame_extent_emu: EmuRectV1 {
                x: frame_x,
                y: frame_y,
                width: line.line_extent_width_emu,
                height: line.line_height_emu,
            },
            measured_width_emu: line.measured_width_emu,
            unsupported_internal_scalars,
        });
        stops.append(&mut line_stops);
    }

    stops.sort_by(|left, right| {
        (
            &left.story_id,
            left.story_line_order,
            left.scalar,
            affinity_rank(left.affinity),
            left.logical_only,
        )
            .cmp(&(
                &right.story_id,
                right.story_line_order,
                right.scalar,
                affinity_rank(right.affinity),
                right.logical_only,
            ))
    });

    Ok(ResolvedTextCaretMapV1 {
        version: CARET_MAP_VERSION_V1.to_owned(),
        layout_environment_id: input.layout_environment_id,
        stories: input.stories,
        lines,
        stops,
    })
}

fn affinity_rank(affinity: CaretAffinityV1) -> u8 {
    match affinity {
        CaretAffinityV1::Upstream => 0,
        CaretAffinityV1::Downstream => 1,
    }
}

pub fn resolve_story_position(
    map: &ResolvedTextCaretMapV1,
    story_id: &str,
    scalar: u32,
    affinity: Option<CaretAffinityV1>,
) -> Result<StoryPositionV1, CaretMapError> {
    let mut candidates: Vec<_> = map
        .stops
        .iter()
        .filter(|stop| stop.story_id == story_id && stop.scalar == scalar)
        .collect();

    if candidates.is_empty() {
        let unsupported = map.lines.iter().any(|line| {
            line.story_id == story_id && line.unsupported_internal_scalars.contains(&scalar)
        });
        return Err(if unsupported {
            CaretMapError::InternalClusterUnsupported {
                story_id: story_id.to_owned(),
                scalar,
            }
        } else {
            CaretMapError::StoryPositionUnplaced {
                story_id: story_id.to_owned(),
                scalar,
            }
        });
    }

    if let Some(requested) = affinity {
        candidates.retain(|stop| stop.affinity == requested);
        if candidates.len() != 1 {
            return Err(CaretMapError::CaretAffinityUnavailable {
                story_id: story_id.to_owned(),
                scalar,
            });
        }
    } else if candidates.len() != 1 {
        return Err(CaretMapError::CaretAffinityRequired {
            story_id: story_id.to_owned(),
            scalar,
        });
    }

    let stop = candidates[0];
    Ok(StoryPositionV1 {
        story_id: stop.story_id.clone(),
        scalar: stop.scalar,
        line_id: stop.line_id.clone(),
        stop_id: stop.stop_id.clone(),
        affinity: stop.affinity,
    })
}

fn point_to_rect_distance(point: EmuPointV1, rect: EmuRectV1) -> i128 {
    let right = i128::from(rect.x) + i128::from(rect.width);
    let bottom = i128::from(rect.y) + i128::from(rect.height);
    let px = i128::from(point.x);
    let py = i128::from(point.y);
    let left = i128::from(rect.x);
    let top = i128::from(rect.y);

    let dx = if px < left {
        left - px
    } else if px > right {
        px - right
    } else {
        0
    };
    let dy = if py < top {
        top - py
    } else if py > bottom {
        py - bottom
    } else {
        0
    };
    dx + dy
}

pub fn hit_test(
    map: &ResolvedTextCaretMapV1,
    page_id: &str,
    point_emu: EmuPointV1,
) -> Result<StoryPositionV1, CaretMapError> {
    let line = map
        .lines
        .iter()
        .filter(|line| line.page_id == page_id)
        .min_by_key(|line| {
            (
                point_to_rect_distance(point_emu, line.page_extent_emu),
                line.story_id.clone(),
                line.story_line_order,
            )
        })
        .ok_or_else(|| CaretMapError::NoLineOnPage {
            page_id: page_id.to_owned(),
        })?;

    let stop = map
        .stops
        .iter()
        .filter(|stop| stop.line_id == line.line_id)
        .min_by_key(|stop| {
            (
                i128::from(stop.page_position_emu.x)
                    .saturating_sub(i128::from(point_emu.x))
                    .abs(),
                stop.scalar,
                affinity_rank(stop.affinity),
                stop.logical_only,
            )
        })
        .ok_or_else(|| CaretMapError::StoryPositionUnplaced {
            story_id: line.story_id.clone(),
            scalar: line.scalar_start,
        })?;

    Ok(StoryPositionV1 {
        story_id: stop.story_id.clone(),
        scalar: stop.scalar,
        line_id: stop.line_id.clone(),
        stop_id: stop.stop_id.clone(),
        affinity: stop.affinity,
    })
}

fn merge_ranges(mut ranges: Vec<ScalarRangeV1>) -> Vec<ScalarRangeV1> {
    ranges.retain(|range| !range.is_empty());
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<ScalarRangeV1> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => {
                last.end = last.end.max(range.end);
            }
            _ => merged.push(range),
        }
    }
    merged
}

fn intersect(left: ScalarRangeV1, right: ScalarRangeV1) -> Option<ScalarRangeV1> {
    let start = left.start.max(right.start);
    let end = left.end.min(right.end);
    (start < end).then_some(ScalarRangeV1 { start, end })
}

fn covered_within(semantic: ScalarRangeV1, placed: &[ScalarRangeV1]) -> Vec<ScalarRangeV1> {
    merge_ranges(
        placed
            .iter()
            .filter_map(|range| intersect(semantic, *range))
            .collect(),
    )
}

fn subtract_covered(semantic: ScalarRangeV1, covered: &[ScalarRangeV1]) -> Vec<ScalarRangeV1> {
    if semantic.is_empty() {
        return Vec::new();
    }
    let mut cursor = semantic.start;
    let mut result = Vec::new();
    for range in covered {
        if range.start > cursor {
            result.push(ScalarRangeV1 {
                start: cursor,
                end: range.start.min(semantic.end),
            });
        }
        cursor = cursor.max(range.end);
        if cursor >= semantic.end {
            break;
        }
    }
    if cursor < semantic.end {
        result.push(ScalarRangeV1 {
            start: cursor,
            end: semantic.end,
        });
    }
    result
}

fn stop_x(map: &ResolvedTextCaretMapV1, line_id_value: &str, scalar: u32) -> Option<(i64, i64)> {
    map.stops
        .iter()
        .find(|stop| stop.line_id == line_id_value && stop.scalar == scalar && !stop.logical_only)
        .map(|stop| (stop.page_position_emu.x, stop.frame_position_emu.x))
}

pub fn selection_geometry(
    map: &ResolvedTextCaretMapV1,
    story_id: &str,
    semantic_range: ScalarRangeV1,
) -> Result<SelectionGeometryV1, CaretMapError> {
    let story = map
        .stories
        .iter()
        .find(|story| story.story_id == story_id)
        .ok_or_else(|| CaretMapError::UnknownStory {
            story_id: story_id.to_owned(),
        })?;
    if semantic_range.start > semantic_range.end || semantic_range.end > story.scalar_len {
        return Err(CaretMapError::InvalidScalarRange {
            start: semantic_range.start,
            end: semantic_range.end,
        });
    }
    if semantic_range.is_empty() {
        return Ok(SelectionGeometryV1 {
            story_id: story_id.to_owned(),
            semantic_range,
            coverage_state: SelectionCoverageStateV1::Complete,
            fragments: Vec::new(),
            covered_ranges: Vec::new(),
            unplaced_ranges: Vec::new(),
            unsupported_internal_boundaries: Vec::new(),
        });
    }

    let unsupported: BTreeSet<u32> = map
        .lines
        .iter()
        .filter(|line| line.story_id == story_id)
        .flat_map(|line| line.unsupported_internal_scalars.iter().copied())
        .filter(|scalar| *scalar == semantic_range.start || *scalar == semantic_range.end)
        .collect();

    let placed_ranges = merge_ranges(
        map.lines
            .iter()
            .filter(|line| line.story_id == story_id)
            .filter_map(|line| {
                (line.scalar_start < line.consumed_scalar_end).then_some(ScalarRangeV1 {
                    start: line.scalar_start,
                    end: line.consumed_scalar_end,
                })
            })
            .collect(),
    );
    let covered_ranges = covered_within(semantic_range, &placed_ranges);
    let unplaced_ranges = subtract_covered(semantic_range, &covered_ranges);

    if !unsupported.is_empty() {
        return Ok(SelectionGeometryV1 {
            story_id: story_id.to_owned(),
            semantic_range,
            coverage_state: SelectionCoverageStateV1::Unsupported,
            fragments: Vec::new(),
            covered_ranges,
            unplaced_ranges,
            unsupported_internal_boundaries: unsupported.into_iter().collect(),
        });
    }

    let mut fragments = Vec::new();
    for line in map.lines.iter().filter(|line| line.story_id == story_id) {
        let visible = ScalarRangeV1 {
            start: line.scalar_start,
            end: line.scalar_end,
        };
        let Some(range) = intersect(semantic_range, visible) else {
            continue;
        };
        let Some((page_left, frame_left)) = stop_x(map, &line.line_id, range.start) else {
            return Ok(SelectionGeometryV1 {
                story_id: story_id.to_owned(),
                semantic_range,
                coverage_state: SelectionCoverageStateV1::Unsupported,
                fragments: Vec::new(),
                covered_ranges,
                unplaced_ranges,
                unsupported_internal_boundaries: vec![range.start],
            });
        };
        let Some((page_right, frame_right)) = stop_x(map, &line.line_id, range.end) else {
            return Ok(SelectionGeometryV1 {
                story_id: story_id.to_owned(),
                semantic_range,
                coverage_state: SelectionCoverageStateV1::Unsupported,
                fragments: Vec::new(),
                covered_ranges,
                unplaced_ranges,
                unsupported_internal_boundaries: vec![range.end],
            });
        };

        let page_x = page_left.min(page_right);
        let frame_x = frame_left.min(frame_right);
        let width = page_left.abs_diff(page_right);
        let width = i64::try_from(width).map_err(|_| CaretMapError::MetricOverflow)?;

        fragments.push(SelectionFragmentV1 {
            story_id: story_id.to_owned(),
            page_id: line.page_id.clone(),
            frame_id: line.frame_id.clone(),
            line_id: line.line_id.clone(),
            story_line_order: line.story_line_order,
            scalar_range: range,
            page_rect_emu: EmuRectV1 {
                x: page_x,
                y: line.page_extent_emu.y,
                width,
                height: line.page_extent_emu.height,
            },
            frame_rect_emu: EmuRectV1 {
                x: frame_x,
                y: line.frame_extent_emu.y,
                width,
                height: line.frame_extent_emu.height,
            },
        });
    }

    fragments.sort_by_key(|fragment| fragment.story_line_order);

    let coverage_state = if covered_ranges.is_empty() {
        SelectionCoverageStateV1::Unplaced
    } else if unplaced_ranges.is_empty() {
        SelectionCoverageStateV1::Complete
    } else {
        SelectionCoverageStateV1::Partial
    };

    Ok(SelectionGeometryV1 {
        story_id: story_id.to_owned(),
        semantic_range,
        coverage_state,
        fragments,
        covered_ranges,
        unplaced_ranges,
        unsupported_internal_boundaries: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STORY: &str = "30000000-0000-4000-8000-000000000001";
    const PAGE: &str = "20000000-0000-4000-8000-000000000001";
    const FRAME_A: &str = "10000000-0000-4000-8000-000000000001";
    const FRAME_B: &str = "10000000-0000-4000-8000-000000000002";

    fn glyph(cluster_start: u32, advance: i64) -> ResolvedGlyphInputV1 {
        ResolvedGlyphInputV1 {
            cluster_start,
            x_advance_emu: advance,
        }
    }

    fn line(
        frame_id: &str,
        order: u32,
        frame_line_index: u32,
        start: u32,
        end: u32,
        consumed: u32,
        page_x: i64,
        page_y: i64,
        glyphs: Vec<ResolvedGlyphInputV1>,
    ) -> ResolvedTextLineInputV1 {
        let measured = glyphs.iter().map(|item| item.x_advance_emu).sum();
        ResolvedTextLineInputV1 {
            story_id: STORY.to_owned(),
            page_id: PAGE.to_owned(),
            frame_id: frame_id.to_owned(),
            story_line_order: order,
            frame_line_index,
            scalar_start: start,
            scalar_end: end,
            consumed_scalar_end: consumed,
            page_origin_emu: EmuPointV1 {
                x: page_x,
                y: page_y,
            },
            frame_origin_emu: EmuPointV1 {
                x: if frame_id == FRAME_A { 1000 } else { 5000 },
                y: 2000,
            },
            line_extent_width_emu: 2000,
            line_height_emu: 400,
            measured_width_emu: measured,
            glyphs,
        }
    }

    fn build(story_len: u32, lines: Vec<ResolvedTextLineInputV1>) -> ResolvedTextCaretMapV1 {
        build_caret_map(ResolvedTextLayoutInputV1 {
            layout_environment_id: "layout:test-v1".to_owned(),
            stories: vec![StoryExtentInputV1 {
                story_id: STORY.to_owned(),
                scalar_len: story_len,
            }],
            lines,
        })
        .unwrap()
    }

    #[test]
    fn ascii_hit_test_before_inside_after_uses_authoritative_stops() {
        let map = build(
            3,
            vec![line(
                FRAME_A,
                0,
                0,
                0,
                3,
                3,
                1000,
                2000,
                vec![glyph(0, 100), glyph(1, 100), glyph(2, 100)],
            )],
        );

        assert_eq!(
            hit_test(&map, PAGE, EmuPointV1 { x: 900, y: 2100 })
                .unwrap()
                .scalar,
            0
        );
        assert_eq!(
            hit_test(&map, PAGE, EmuPointV1 { x: 1110, y: 2100 })
                .unwrap()
                .scalar,
            1
        );
        assert_eq!(
            hit_test(&map, PAGE, EmuPointV1 { x: 1400, y: 2100 })
                .unwrap()
                .scalar,
            3
        );
    }

    #[test]
    fn soft_wrap_preserves_same_scalar_multi_stop_and_requires_affinity() {
        let map = build(
            6,
            vec![
                line(
                    FRAME_A,
                    0,
                    0,
                    0,
                    3,
                    3,
                    1000,
                    2000,
                    vec![glyph(0, 100), glyph(1, 100), glyph(2, 100)],
                ),
                line(
                    FRAME_A,
                    1,
                    1,
                    3,
                    6,
                    6,
                    1000,
                    2400,
                    vec![glyph(3, 100), glyph(4, 100), glyph(5, 100)],
                ),
            ],
        );

        assert_eq!(
            resolve_story_position(&map, STORY, 3, None),
            Err(CaretMapError::CaretAffinityRequired {
                story_id: STORY.to_owned(),
                scalar: 3,
            })
        );
        let upstream =
            resolve_story_position(&map, STORY, 3, Some(CaretAffinityV1::Upstream)).unwrap();
        let downstream =
            resolve_story_position(&map, STORY, 3, Some(CaretAffinityV1::Downstream)).unwrap();
        assert_ne!(upstream.line_id, downstream.line_id);
        assert_eq!(
            map.lines[0].next_line_id,
            Some(map.lines[1].line_id.clone())
        );
        assert_eq!(
            map.lines[1].previous_line_id,
            Some(map.lines[0].line_id.clone())
        );
    }

    #[test]
    fn ligature_and_combining_clusters_do_not_invent_internal_caret_geometry() {
        let ligature = build(
            3,
            vec![line(
                FRAME_A,
                0,
                0,
                0,
                3,
                3,
                1000,
                2000,
                vec![glyph(0, 200), glyph(2, 100)],
            )],
        );
        assert_eq!(
            resolve_story_position(&ligature, STORY, 1, None),
            Err(CaretMapError::InternalClusterUnsupported {
                story_id: STORY.to_owned(),
                scalar: 1,
            })
        );
        let full_cluster =
            selection_geometry(&ligature, STORY, ScalarRangeV1::new(0, 2).unwrap()).unwrap();
        assert_eq!(
            full_cluster.coverage_state,
            SelectionCoverageStateV1::Complete
        );
        assert_eq!(full_cluster.fragments[0].page_rect_emu.width, 200);

        let inside_cluster =
            selection_geometry(&ligature, STORY, ScalarRangeV1::new(1, 2).unwrap()).unwrap();
        assert_eq!(
            inside_cluster.coverage_state,
            SelectionCoverageStateV1::Unsupported
        );
        assert_eq!(inside_cluster.unsupported_internal_boundaries, vec![1]);

        let combining = build(
            2,
            vec![line(
                FRAME_A,
                0,
                0,
                0,
                2,
                2,
                1000,
                2000,
                vec![glyph(0, 100)],
            )],
        );
        assert_eq!(
            resolve_story_position(&combining, STORY, 1, None),
            Err(CaretMapError::InternalClusterUnsupported {
                story_id: STORY.to_owned(),
                scalar: 1,
            })
        );
    }

    #[test]
    fn supplementary_unicode_scalar_is_one_canonical_position() {
        let map = build(
            2,
            vec![line(
                FRAME_A,
                0,
                0,
                0,
                2,
                2,
                1000,
                2000,
                vec![glyph(0, 150), glyph(1, 100)],
            )],
        );
        assert_eq!(
            resolve_story_position(&map, STORY, 1, None).unwrap().scalar,
            1
        );
    }

    #[test]
    fn mandatory_delimiter_remains_logical_without_painted_rectangle() {
        let map = build(
            2,
            vec![line(
                FRAME_A,
                0,
                0,
                0,
                1,
                2,
                1000,
                2000,
                vec![glyph(0, 100)],
            )],
        );
        let after_delimiter =
            resolve_story_position(&map, STORY, 2, Some(CaretAffinityV1::Upstream)).unwrap();
        assert_eq!(after_delimiter.scalar, 2);

        let delimiter = selection_geometry(&map, STORY, ScalarRangeV1::new(1, 2).unwrap()).unwrap();
        assert_eq!(delimiter.coverage_state, SelectionCoverageStateV1::Complete);
        assert!(delimiter.fragments.is_empty());
        assert_eq!(
            delimiter.covered_ranges,
            vec![ScalarRangeV1 { start: 1, end: 2 }]
        );
    }

    #[test]
    fn linked_frame_line_order_comes_from_story_flow_not_geometry_sort() {
        let map = build(
            4,
            vec![
                line(
                    FRAME_A,
                    0,
                    0,
                    0,
                    2,
                    2,
                    9000,
                    8000,
                    vec![glyph(0, 100), glyph(1, 100)],
                ),
                line(
                    FRAME_B,
                    1,
                    0,
                    2,
                    4,
                    4,
                    1000,
                    1000,
                    vec![glyph(2, 100), glyph(3, 100)],
                ),
            ],
        );

        assert_eq!(map.lines[0].story_line_order, 0);
        assert_eq!(map.lines[0].frame_id, FRAME_A);
        assert_eq!(map.lines[1].story_line_order, 1);
        assert_eq!(map.lines[1].frame_id, FRAME_B);
        assert_eq!(
            map.lines[0].next_line_id,
            Some(map.lines[1].line_id.clone())
        );
    }

    #[test]
    fn selection_reports_complete_partial_and_unplaced_without_fabrication() {
        let map = build(
            10,
            vec![line(
                FRAME_A,
                0,
                0,
                0,
                5,
                5,
                1000,
                2000,
                vec![
                    glyph(0, 100),
                    glyph(1, 100),
                    glyph(2, 100),
                    glyph(3, 100),
                    glyph(4, 100),
                ],
            )],
        );

        let complete = selection_geometry(&map, STORY, ScalarRangeV1::new(1, 4).unwrap()).unwrap();
        assert_eq!(complete.coverage_state, SelectionCoverageStateV1::Complete);
        assert_eq!(
            complete.covered_ranges,
            vec![ScalarRangeV1 { start: 1, end: 4 }]
        );
        assert!(complete.unplaced_ranges.is_empty());

        let partial = selection_geometry(&map, STORY, ScalarRangeV1::new(3, 8).unwrap()).unwrap();
        assert_eq!(partial.coverage_state, SelectionCoverageStateV1::Partial);
        assert_eq!(
            partial.covered_ranges,
            vec![ScalarRangeV1 { start: 3, end: 5 }]
        );
        assert_eq!(
            partial.unplaced_ranges,
            vec![ScalarRangeV1 { start: 5, end: 8 }]
        );

        let unplaced = selection_geometry(&map, STORY, ScalarRangeV1::new(6, 9).unwrap()).unwrap();
        assert_eq!(unplaced.coverage_state, SelectionCoverageStateV1::Unplaced);
        assert!(unplaced.fragments.is_empty());
        assert!(unplaced.covered_ranges.is_empty());
        assert_eq!(
            unplaced.unplaced_ranges,
            vec![ScalarRangeV1 { start: 6, end: 9 }]
        );
    }

    #[test]
    fn fixed_layout_input_serializes_byte_identically() {
        let input = ResolvedTextLayoutInputV1 {
            layout_environment_id: "layout:test-v1".to_owned(),
            stories: vec![StoryExtentInputV1 {
                story_id: STORY.to_owned(),
                scalar_len: 3,
            }],
            lines: vec![line(
                FRAME_A,
                0,
                0,
                0,
                3,
                3,
                1000,
                2000,
                vec![glyph(0, 100), glyph(1, 100), glyph(2, 100)],
            )],
        };
        let left = serde_json::to_vec(&build_caret_map(input.clone()).unwrap()).unwrap();
        let right = serde_json::to_vec(&build_caret_map(input).unwrap()).unwrap();
        assert_eq!(left, right);
    }

    #[test]
    fn frame_local_geometry_is_derived_from_authoritative_page_and_frame_origins() {
        let map = build(
            1,
            vec![line(
                FRAME_A,
                0,
                0,
                0,
                1,
                1,
                1300,
                2400,
                vec![glyph(0, 100)],
            )],
        );
        assert_eq!(map.lines[0].frame_extent_emu.x, 300);
        assert_eq!(map.lines[0].frame_extent_emu.y, 400);
        let start = &map.stops[0];
        assert_eq!(start.frame_position_emu, EmuPointV1 { x: 300, y: 400 });
    }
}
