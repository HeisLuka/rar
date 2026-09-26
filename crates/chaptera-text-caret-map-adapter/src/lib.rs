//! Rust conformance adapter for the canonical service-side ResolvedTextCaretMapV1.
//!
//! Semantic authority remains services/editor-api/resolved_text_caret_map_v1.py.
//! This crate exists so native Rust products can consume the same contract in-process.
//! It must not use renderer/widget text metrics or define divergent caret semantics.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const CARET_MAP_VERSION_V1: &str = "chaptera.resolved-text-caret-map.v1";
pub const SELECTION_GEOMETRY_VERSION_V1: &str = "chaptera.selection-geometry.v1";
pub const MAX_SAFE_EMU: i64 = 9_007_199_254_740_991;
pub const MIN_SAFE_EMU: i64 = -MAX_SAFE_EMU;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaretMapError {
    pub code: &'static str,
    pub message: String,
}

impl CaretMapError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for CaretMapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for CaretMapError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScalarRangeV1 {
    pub start_scalar: u32,
    pub end_scalar: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InternalCaretStopV1 {
    pub scalar_boundary: u32,
    pub page_x_emu: i64,
    pub frame_x_emu: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedClusterV1 {
    pub start_scalar: u32,
    pub end_scalar: u32,
    pub page_x_start_emu: i64,
    pub page_x_end_emu: i64,
    pub frame_x_start_emu: i64,
    pub frame_x_end_emu: i64,
    #[serde(default = "default_true")]
    pub painted: bool,
    #[serde(default)]
    pub internal_caret_stops: Vec<InternalCaretStopV1>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedLineFragmentV1 {
    pub story_id: String,
    pub page_id: String,
    pub frame_id: String,
    pub line_id: String,
    pub flow_ordinal: u32,
    pub previous_line_id: Option<String>,
    pub next_line_id: Option<String>,
    pub page_y_top_emu: i64,
    pub page_y_bottom_emu: i64,
    pub frame_y_top_emu: i64,
    pub frame_y_bottom_emu: i64,
    pub clusters: Vec<ResolvedClusterV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaretStopV1 {
    pub stop_id: String,
    pub story_id: String,
    pub scalar_boundary: u32,
    pub page_id: String,
    pub frame_id: String,
    pub line_id: String,
    pub flow_ordinal: u32,
    pub page_x_emu: i64,
    pub page_y_top_emu: i64,
    pub page_y_bottom_emu: i64,
    pub frame_x_emu: i64,
    pub frame_y_top_emu: i64,
    pub frame_y_bottom_emu: i64,
    pub affinities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionRectV1 {
    pub page_id: String,
    pub frame_id: String,
    pub line_id: String,
    pub flow_ordinal: u32,
    pub start_scalar: u32,
    pub end_scalar: u32,
    pub page_x_start_emu: i64,
    pub page_x_end_emu: i64,
    pub page_y_top_emu: i64,
    pub page_y_bottom_emu: i64,
    pub frame_x_start_emu: i64,
    pub frame_x_end_emu: i64,
    pub frame_y_top_emu: i64,
    pub frame_y_bottom_emu: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionGeometryV1 {
    pub protocol_version: String,
    pub story_id: String,
    pub start_scalar: u32,
    pub end_scalar: u32,
    pub is_empty: bool,
    pub rectangles: Vec<SelectionRectV1>,
    pub covered_ranges: Vec<ScalarRangeV1>,
    pub unplaced_ranges: Vec<ScalarRangeV1>,
    pub unsupported_ranges: Vec<ScalarRangeV1>,
    pub coverage_state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedTextCaretMapV1 {
    pub protocol_version: String,
    pub layout_revision_id: String,
    pub story_id: String,
    pub story_scalar_len: u32,
    pub lines: Vec<ResolvedLineFragmentV1>,
    pub caret_stops: Vec<CaretStopV1>,
    pub materialized_ranges: Vec<ScalarRangeV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaretMapBuildInputV1 {
    pub layout_revision_id: String,
    pub story_id: String,
    pub story_scalar_len: u32,
    pub lines: Vec<ResolvedLineFragmentV1>,
}

fn safe_emu(value: i64, label: &str) -> Result<i64, CaretMapError> {
    if !(MIN_SAFE_EMU..=MAX_SAFE_EMU).contains(&value) {
        return Err(CaretMapError::new(
            "invalid_geometry",
            format!("{label} must be a JavaScript-safe EMU integer"),
        ));
    }
    Ok(value)
}

fn validate_range(start: u32, end: u32, story_len: u32, label: &str) -> Result<(), CaretMapError> {
    if end < start || end > story_len {
        return Err(CaretMapError::new(
            "invalid_story_range",
            format!("{label} lies outside Story scalar extent"),
        ));
    }
    Ok(())
}

fn merge_ranges(mut ranges: Vec<(u32, u32)>) -> Vec<ScalarRangeV1> {
    ranges.retain(|(a, b)| b > a);
    ranges.sort_unstable();
    let mut merged: Vec<ScalarRangeV1> = Vec::new();
    for (a, b) in ranges {
        match merged.last_mut() {
            Some(last) if a <= last.end_scalar => last.end_scalar = last.end_scalar.max(b),
            _ => merged.push(ScalarRangeV1 {
                start_scalar: a,
                end_scalar: b,
            }),
        }
    }
    merged
}

fn subtract_ranges(start: u32, end: u32, covered: &[ScalarRangeV1]) -> Vec<ScalarRangeV1> {
    if start == end {
        return Vec::new();
    }
    let mut cursor = start;
    let mut out = Vec::new();
    for item in covered {
        let a = start.max(item.start_scalar);
        let b = end.min(item.end_scalar);
        if b <= a {
            continue;
        }
        if cursor < a {
            out.push((cursor, a));
        }
        cursor = cursor.max(b);
    }
    if cursor < end {
        out.push((cursor, end));
    }
    merge_ranges(out)
}

fn validate_cluster(
    mut cluster: ResolvedClusterV1,
    story_len: u32,
    line_id: &str,
    index: usize,
) -> Result<ResolvedClusterV1, CaretMapError> {
    validate_range(
        cluster.start_scalar,
        cluster.end_scalar,
        story_len,
        &format!("{line_id}.clusters[{index}]"),
    )?;
    if cluster.end_scalar <= cluster.start_scalar {
        return Err(CaretMapError::new(
            "invalid_layout",
            "resolved cluster must cover one or more scalars",
        ));
    }
    for (label, value) in [
        ("page_x_start_emu", cluster.page_x_start_emu),
        ("page_x_end_emu", cluster.page_x_end_emu),
        ("frame_x_start_emu", cluster.frame_x_start_emu),
        ("frame_x_end_emu", cluster.frame_x_end_emu),
    ] {
        safe_emu(value, &format!("{line_id}.{label}"))?;
    }
    if cluster.page_x_end_emu < cluster.page_x_start_emu {
        return Err(CaretMapError::new(
            "invalid_layout",
            "horizontal LTR cluster page x must be nondecreasing",
        ));
    }
    if cluster.frame_x_end_emu < cluster.frame_x_start_emu {
        return Err(CaretMapError::new(
            "invalid_layout",
            "horizontal LTR cluster frame x must be nondecreasing",
        ));
    }
    let mut seen = BTreeSet::new();
    for stop in &cluster.internal_caret_stops {
        if !(cluster.start_scalar < stop.scalar_boundary
            && stop.scalar_boundary < cluster.end_scalar)
        {
            return Err(CaretMapError::new(
                "invalid_layout",
                "internal caret authority must lie strictly inside cluster",
            ));
        }
        if !seen.insert(stop.scalar_boundary) {
            return Err(CaretMapError::new(
                "invalid_layout",
                "duplicate internal caret scalar boundary",
            ));
        }
        safe_emu(stop.page_x_emu, "internal.page_x_emu")?;
        safe_emu(stop.frame_x_emu, "internal.frame_x_emu")?;
        if !(cluster.page_x_start_emu..=cluster.page_x_end_emu).contains(&stop.page_x_emu) {
            return Err(CaretMapError::new(
                "invalid_layout",
                "internal page caret lies outside cluster advance",
            ));
        }
        if !(cluster.frame_x_start_emu..=cluster.frame_x_end_emu).contains(&stop.frame_x_emu) {
            return Err(CaretMapError::new(
                "invalid_layout",
                "internal frame caret lies outside cluster advance",
            ));
        }
    }
    cluster
        .internal_caret_stops
        .sort_by_key(|stop| stop.scalar_boundary);
    Ok(cluster)
}

fn validate_line(
    mut line: ResolvedLineFragmentV1,
    story_id: &str,
    story_len: u32,
) -> Result<ResolvedLineFragmentV1, CaretMapError> {
    for (label, value) in [
        ("story_id", line.story_id.as_str()),
        ("page_id", line.page_id.as_str()),
        ("frame_id", line.frame_id.as_str()),
        ("line_id", line.line_id.as_str()),
    ] {
        if value.is_empty() {
            return Err(CaretMapError::new(
                "invalid_layout",
                format!("{label} is required"),
            ));
        }
    }
    if line.story_id != story_id {
        return Err(CaretMapError::new(
            "invalid_layout",
            "resolved line targets different Story",
        ));
    }
    for (label, value) in [
        ("page_y_top_emu", line.page_y_top_emu),
        ("page_y_bottom_emu", line.page_y_bottom_emu),
        ("frame_y_top_emu", line.frame_y_top_emu),
        ("frame_y_bottom_emu", line.frame_y_bottom_emu),
    ] {
        safe_emu(value, &format!("{}.{label}", line.line_id))?;
    }
    if line.page_y_bottom_emu <= line.page_y_top_emu {
        return Err(CaretMapError::new(
            "invalid_layout",
            "line page extent must be positive",
        ));
    }
    if line.frame_y_bottom_emu <= line.frame_y_top_emu {
        return Err(CaretMapError::new(
            "invalid_layout",
            "line frame extent must be positive",
        ));
    }
    if line.clusters.is_empty() {
        return Err(CaretMapError::new(
            "invalid_layout",
            "resolved line requires one or more clusters",
        ));
    }
    let mut canonical = Vec::with_capacity(line.clusters.len());
    for (index, cluster) in line.clusters.into_iter().enumerate() {
        canonical.push(validate_cluster(cluster, story_len, &line.line_id, index)?);
    }
    canonical.sort_by_key(|cluster| (cluster.start_scalar, cluster.end_scalar));
    for pair in canonical.windows(2) {
        if pair[0].end_scalar > pair[1].start_scalar {
            return Err(CaretMapError::new(
                "invalid_layout",
                "resolved clusters overlap within one line",
            ));
        }
    }
    line.clusters = canonical;
    Ok(line)
}

fn build_caret_stops(lines: &[ResolvedLineFragmentV1]) -> Vec<CaretStopV1> {
    let mut stops = Vec::new();
    for line in lines {
        let mut candidates: BTreeMap<(u32, i64, i64), BTreeSet<&'static str>> = BTreeMap::new();
        for cluster in &line.clusters {
            candidates
                .entry((
                    cluster.start_scalar,
                    cluster.page_x_start_emu,
                    cluster.frame_x_start_emu,
                ))
                .or_default()
                .insert("downstream");
            candidates
                .entry((
                    cluster.end_scalar,
                    cluster.page_x_end_emu,
                    cluster.frame_x_end_emu,
                ))
                .or_default()
                .insert("upstream");
            for internal in &cluster.internal_caret_stops {
                candidates
                    .entry((
                        internal.scalar_boundary,
                        internal.page_x_emu,
                        internal.frame_x_emu,
                    ))
                    .or_default()
                    .insert("internal");
            }
        }
        for (ordinal, ((scalar, page_x, frame_x), affinities)) in candidates.into_iter().enumerate()
        {
            let ordered = ["upstream", "downstream", "internal"]
                .into_iter()
                .filter(|v| affinities.contains(v))
                .map(str::to_owned)
                .collect();
            stops.push(CaretStopV1 {
                stop_id: format!("{}:stop:{ordinal}", line.line_id),
                story_id: line.story_id.clone(),
                scalar_boundary: scalar,
                page_id: line.page_id.clone(),
                frame_id: line.frame_id.clone(),
                line_id: line.line_id.clone(),
                flow_ordinal: line.flow_ordinal,
                page_x_emu: page_x,
                page_y_top_emu: line.page_y_top_emu,
                page_y_bottom_emu: line.page_y_bottom_emu,
                frame_x_emu: frame_x,
                frame_y_top_emu: line.frame_y_top_emu,
                frame_y_bottom_emu: line.frame_y_bottom_emu,
                affinities: ordered,
            });
        }
    }
    stops.sort_by(|a, b| {
        (
            a.flow_ordinal,
            a.scalar_boundary,
            a.page_x_emu,
            a.stop_id.as_str(),
        )
            .cmp(&(
                b.flow_ordinal,
                b.scalar_boundary,
                b.page_x_emu,
                b.stop_id.as_str(),
            ))
    });
    stops
}

pub fn build_resolved_text_caret_map_v1(
    input: CaretMapBuildInputV1,
) -> Result<ResolvedTextCaretMapV1, CaretMapError> {
    if input.layout_revision_id.is_empty() {
        return Err(CaretMapError::new(
            "invalid_layout",
            "layout_revision_id is required",
        ));
    }
    if input.story_id.is_empty() {
        return Err(CaretMapError::new("invalid_layout", "story_id is required"));
    }
    let mut lines = Vec::with_capacity(input.lines.len());
    for line in input.lines {
        lines.push(validate_line(
            line,
            &input.story_id,
            input.story_scalar_len,
        )?);
    }
    lines.sort_by_key(|line| line.flow_ordinal);
    let mut ids = BTreeSet::new();
    let mut ords = BTreeSet::new();
    for line in &lines {
        if !ids.insert(line.line_id.clone()) {
            return Err(CaretMapError::new(
                "invalid_layout",
                "line_id must be unique",
            ));
        }
        if !ords.insert(line.flow_ordinal) {
            return Err(CaretMapError::new(
                "invalid_layout",
                "flow_ordinal must be unique",
            ));
        }
    }
    if let Some(first) = lines.first().map(|line| line.flow_ordinal) {
        for (index, line) in lines.iter().enumerate() {
            let expected = first + index as u32;
            if line.flow_ordinal != expected {
                return Err(CaretMapError::new(
                    "invalid_layout",
                    "flow_ordinal sequence must be contiguous",
                ));
            }
            let prev = index.checked_sub(1).map(|i| lines[i].line_id.as_str());
            let next = lines.get(index + 1).map(|v| v.line_id.as_str());
            if line.previous_line_id.as_deref() != prev {
                return Err(CaretMapError::new(
                    "invalid_layout",
                    "previous_line_id disagrees with Story-flow order",
                ));
            }
            if line.next_line_id.as_deref() != next {
                return Err(CaretMapError::new(
                    "invalid_layout",
                    "next_line_id disagrees with Story-flow order",
                ));
            }
        }
    }
    let mut ranges = Vec::new();
    for line in &lines {
        for cluster in &line.clusters {
            ranges.push((cluster.start_scalar, cluster.end_scalar));
        }
    }
    let mut sorted = ranges.clone();
    sorted.sort_unstable();
    for pair in sorted.windows(2) {
        if pair[0].1 > pair[1].0 {
            return Err(CaretMapError::new(
                "invalid_layout",
                "resolved cluster scalar coverage overlaps across Story-flow lines",
            ));
        }
    }
    let caret_stops = build_caret_stops(&lines);
    Ok(ResolvedTextCaretMapV1 {
        protocol_version: CARET_MAP_VERSION_V1.to_owned(),
        layout_revision_id: input.layout_revision_id,
        story_id: input.story_id,
        story_scalar_len: input.story_scalar_len,
        lines,
        caret_stops,
        materialized_ranges: merge_ranges(ranges),
    })
}

fn require_layout(
    map: &ResolvedTextCaretMapV1,
    expected: Option<&str>,
) -> Result<(), CaretMapError> {
    if let Some(expected) = expected
        && expected != map.layout_revision_id
    {
        return Err(CaretMapError::new(
            "stale_layout_map",
            "caret map belongs to a different layout revision",
        ));
    }
    Ok(())
}

pub fn resolve_story_position_v1(
    map: &ResolvedTextCaretMapV1,
    scalar_boundary: u32,
    stop_id: Option<&str>,
    expected_layout_revision_id: Option<&str>,
) -> Result<CaretStopV1, CaretMapError> {
    require_layout(map, expected_layout_revision_id)?;
    if scalar_boundary > map.story_scalar_len {
        return Err(CaretMapError::new(
            "invalid_story_position",
            "scalar boundary lies outside Story",
        ));
    }
    let matches: Vec<&CaretStopV1> = map
        .caret_stops
        .iter()
        .filter(|stop| stop.scalar_boundary == scalar_boundary)
        .collect();
    if let Some(stop_id) = stop_id {
        let selected: Vec<_> = matches
            .iter()
            .copied()
            .filter(|stop| stop.stop_id == stop_id)
            .collect();
        if selected.len() != 1 {
            return Err(CaretMapError::new(
                "invalid_caret_affinity",
                "requested caret stop is not admitted",
            ));
        }
        return Ok(selected[0].clone());
    }
    if matches.len() == 1 {
        return Ok(matches[0].clone());
    }
    if matches.len() > 1 {
        let physical: BTreeSet<_> = matches
            .iter()
            .map(|stop| {
                (
                    stop.page_id.as_str(),
                    stop.frame_id.as_str(),
                    stop.line_id.as_str(),
                    stop.page_x_emu,
                    stop.page_y_top_emu,
                    stop.page_y_bottom_emu,
                )
            })
            .collect();
        if physical.len() > 1 {
            return Err(CaretMapError::new(
                "caret_affinity_required",
                "one Story scalar maps to multiple physical caret stops",
            ));
        }
        return Ok(matches[0].clone());
    }
    for line in &map.lines {
        for cluster in &line.clusters {
            if cluster.start_scalar < scalar_boundary && scalar_boundary < cluster.end_scalar {
                return Err(CaretMapError::new(
                    "internal_cluster_unsupported",
                    "cluster interior has no explicit shaping caret authority",
                ));
            }
        }
    }
    Err(CaretMapError::new(
        "unplaced_story_position",
        "Story position has no resolved caret stop",
    ))
}

pub fn hit_test_story_position_v1(
    map: &ResolvedTextCaretMapV1,
    page_id: &str,
    page_x_emu: i64,
    page_y_emu: i64,
    expected_layout_revision_id: Option<&str>,
) -> Result<CaretStopV1, CaretMapError> {
    require_layout(map, expected_layout_revision_id)?;
    if page_id.is_empty() {
        return Err(CaretMapError::new(
            "invalid_hit_test",
            "page_id is required",
        ));
    }
    safe_emu(page_x_emu, "page_x_emu")?;
    safe_emu(page_y_emu, "page_y_emu")?;
    let lines: Vec<&ResolvedLineFragmentV1> = map
        .lines
        .iter()
        .filter(|line| line.page_id == page_id)
        .collect();
    if lines.is_empty() {
        return Err(CaretMapError::new(
            "unplaced_hit_test",
            "page has no resolved Story lines",
        ));
    }
    let line = lines
        .into_iter()
        .min_by_key(|line| {
            let vertical = if page_y_emu < line.page_y_top_emu {
                line.page_y_top_emu - page_y_emu
            } else if page_y_emu > line.page_y_bottom_emu {
                page_y_emu - line.page_y_bottom_emu
            } else {
                0
            };
            (vertical, line.flow_ordinal)
        })
        .expect("non-empty");
    let stops: Vec<&CaretStopV1> = map
        .caret_stops
        .iter()
        .filter(|stop| stop.line_id == line.line_id)
        .collect();
    if stops.is_empty() {
        return Err(CaretMapError::new(
            "unplaced_hit_test",
            "resolved line has no admitted caret stops",
        ));
    }
    Ok(stops
        .into_iter()
        .min_by_key(|stop| {
            (
                stop.page_x_emu.abs_diff(page_x_emu),
                stop.scalar_boundary,
                stop.stop_id.as_str(),
            )
        })
        .expect("non-empty")
        .clone())
}

fn boundary_x(cluster: &ResolvedClusterV1, scalar: u32) -> Option<(i64, i64)> {
    if scalar == cluster.start_scalar {
        return Some((cluster.page_x_start_emu, cluster.frame_x_start_emu));
    }
    if scalar == cluster.end_scalar {
        return Some((cluster.page_x_end_emu, cluster.frame_x_end_emu));
    }
    cluster
        .internal_caret_stops
        .iter()
        .find(|stop| stop.scalar_boundary == scalar)
        .map(|stop| (stop.page_x_emu, stop.frame_x_emu))
}

pub fn selection_geometry_v1(
    map: &ResolvedTextCaretMapV1,
    start_scalar: u32,
    end_scalar: u32,
    expected_layout_revision_id: Option<&str>,
) -> Result<SelectionGeometryV1, CaretMapError> {
    require_layout(map, expected_layout_revision_id)?;
    validate_range(start_scalar, end_scalar, map.story_scalar_len, "selection")?;
    if start_scalar == end_scalar {
        return Ok(SelectionGeometryV1 {
            protocol_version: SELECTION_GEOMETRY_VERSION_V1.to_owned(),
            story_id: map.story_id.clone(),
            start_scalar,
            end_scalar,
            is_empty: true,
            rectangles: Vec::new(),
            covered_ranges: Vec::new(),
            unplaced_ranges: Vec::new(),
            unsupported_ranges: Vec::new(),
            coverage_state: "complete".to_owned(),
        });
    }
    let mut rectangles = Vec::new();
    let mut covered = Vec::new();
    let mut unsupported = Vec::new();
    let mut materialized = Vec::new();
    for line in &map.lines {
        for cluster in &line.clusters {
            let a = start_scalar.max(cluster.start_scalar);
            let b = end_scalar.min(cluster.end_scalar);
            if b <= a {
                continue;
            }
            materialized.push((a, b));
            let left = boundary_x(cluster, a);
            let right = boundary_x(cluster, b);
            let (Some(left), Some(right)) = (left, right) else {
                unsupported.push((a, b));
                continue;
            };
            covered.push((a, b));
            if cluster.painted && right.0 != left.0 {
                rectangles.push(SelectionRectV1 {
                    page_id: line.page_id.clone(),
                    frame_id: line.frame_id.clone(),
                    line_id: line.line_id.clone(),
                    flow_ordinal: line.flow_ordinal,
                    start_scalar: a,
                    end_scalar: b,
                    page_x_start_emu: left.0.min(right.0),
                    page_x_end_emu: left.0.max(right.0),
                    page_y_top_emu: line.page_y_top_emu,
                    page_y_bottom_emu: line.page_y_bottom_emu,
                    frame_x_start_emu: left.1.min(right.1),
                    frame_x_end_emu: left.1.max(right.1),
                    frame_y_top_emu: line.frame_y_top_emu,
                    frame_y_bottom_emu: line.frame_y_bottom_emu,
                });
            }
        }
    }
    let materialized_ranges = merge_ranges(materialized);
    let unplaced_ranges = subtract_ranges(start_scalar, end_scalar, &materialized_ranges);
    let covered_ranges = merge_ranges(covered);
    let unsupported_ranges = merge_ranges(unsupported);
    rectangles.sort_by_key(|r| {
        (
            r.flow_ordinal,
            r.start_scalar,
            r.end_scalar,
            r.page_x_start_emu,
        )
    });
    let coverage_state = if !unsupported_ranges.is_empty() {
        "unsupported"
    } else if !unplaced_ranges.is_empty() {
        if covered_ranges.is_empty() {
            "unplaced"
        } else {
            "partial"
        }
    } else {
        "complete"
    };
    Ok(SelectionGeometryV1 {
        protocol_version: SELECTION_GEOMETRY_VERSION_V1.to_owned(),
        story_id: map.story_id.clone(),
        start_scalar,
        end_scalar,
        is_empty: false,
        rectangles,
        covered_ranges,
        unplaced_ranges,
        unsupported_ranges,
        coverage_state: coverage_state.to_owned(),
    })
}

pub fn caret_map_to_value_v1(map: &ResolvedTextCaretMapV1) -> Value {
    json!({
        "protocol_version": map.protocol_version,
        "layout_revision_id": map.layout_revision_id,
        "story_id": map.story_id,
        "story_scalar_len": map.story_scalar_len,
        "lines": map.lines,
        "caret_stops": map.caret_stops.iter().map(|stop| json!({
            "stop_id": stop.stop_id,
            "scalar_boundary": stop.scalar_boundary,
            "page_id": stop.page_id,
            "frame_id": stop.frame_id,
            "line_id": stop.line_id,
            "flow_ordinal": stop.flow_ordinal,
            "page_x_emu": stop.page_x_emu,
            "page_y_top_emu": stop.page_y_top_emu,
            "page_y_bottom_emu": stop.page_y_bottom_emu,
            "frame_x_emu": stop.frame_x_emu,
            "frame_y_top_emu": stop.frame_y_top_emu,
            "frame_y_bottom_emu": stop.frame_y_bottom_emu,
            "affinities": stop.affinities,
        })).collect::<Vec<_>>(),
        "materialized_ranges": map.materialized_ranges.iter()
            .map(|r| json!([r.start_scalar,r.end_scalar])).collect::<Vec<_>>(),
    })
}

fn push_python_json_ascii_string(out: &mut String, value: &str) {
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if character <= '\u{001F}' => {
                out.push_str(&format!("\\u{:04x}", character as u32));
            }
            character if character.is_ascii() => out.push(character),
            character => {
                let mut units = [0_u16; 2];
                for unit in character.encode_utf16(&mut units).iter() {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
}

fn push_python_canonical_json(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => out.push_str(&value.to_string()),
        Value::String(value) => push_python_json_ascii_string(out, value),
        Value::Array(values) => {
            out.push('[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    out.push(',');
                }
                push_python_canonical_json(out, value);
            }
            out.push(']');
        }
        Value::Object(values) => {
            out.push('{');
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(left, _)| *left);
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index != 0 {
                    out.push(',');
                }
                push_python_json_ascii_string(out, key);
                out.push(':');
                push_python_canonical_json(out, value);
            }
            out.push('}');
        }
    }
}

pub fn caret_map_hash_v1(map: &ResolvedTextCaretMapV1) -> String {
    let mut canonical = String::new();
    push_python_canonical_json(&mut canonical, &caret_map_to_value_v1(map));
    let mut hash = Sha256::new();
    hash.update(canonical.as_bytes());
    format!("{:x}", hash.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(
        id: &str,
        ordinal: u32,
        prev: Option<&str>,
        next: Option<&str>,
        clusters: Vec<ResolvedClusterV1>,
    ) -> ResolvedLineFragmentV1 {
        ResolvedLineFragmentV1 {
            story_id: "story:1".into(),
            page_id: "page:1".into(),
            frame_id: format!("frame:{ordinal}"),
            line_id: id.into(),
            flow_ordinal: ordinal,
            previous_line_id: prev.map(str::to_owned),
            next_line_id: next.map(str::to_owned),
            page_y_top_emu: (ordinal as i64) * 100,
            page_y_bottom_emu: (ordinal as i64) * 100 + 20,
            frame_y_top_emu: 0,
            frame_y_bottom_emu: 20,
            clusters,
        }
    }
    fn cluster(a: u32, b: u32, x0: i64, x1: i64) -> ResolvedClusterV1 {
        ResolvedClusterV1 {
            start_scalar: a,
            end_scalar: b,
            page_x_start_emu: x0,
            page_x_end_emu: x1,
            frame_x_start_emu: x0,
            frame_x_end_emu: x1,
            painted: true,
            internal_caret_stops: Vec::new(),
        }
    }
    fn build(lines: Vec<ResolvedLineFragmentV1>, len: u32) -> ResolvedTextCaretMapV1 {
        build_resolved_text_caret_map_v1(CaretMapBuildInputV1 {
            layout_revision_id: "layout:1".into(),
            story_id: "story:1".into(),
            story_scalar_len: len,
            lines,
        })
        .unwrap()
    }

    #[test]
    fn canonical_json_hash_uses_python_ascii_escaping_and_sorted_keys() {
        let mut value = serde_json::Map::new();
        value.insert("z".to_owned(), Value::String("é😀".to_owned()));
        value.insert("a".to_owned(), Value::Bool(true));
        let mut canonical = String::new();
        push_python_canonical_json(&mut canonical, &Value::Object(value));
        assert_eq!(canonical, r#"{"a":true,"z":"\u00e9\ud83d\ude00"}"#);
    }

    #[test]
    fn stale_revision_fails_closed() {
        let map = build(
            vec![line("l0", 0, None, None, vec![cluster(0, 1, 0, 10)])],
            1,
        );
        assert_eq!(
            resolve_story_position_v1(&map, 0, None, Some("layout:old"))
                .unwrap_err()
                .code,
            "stale_layout_map"
        );
    }

    #[test]
    fn linked_same_scalar_requires_affinity_by_stop_id() {
        let map = build(
            vec![
                line("l0", 0, None, Some("l1"), vec![cluster(0, 1, 0, 10)]),
                line("l1", 1, Some("l0"), None, vec![cluster(1, 2, 100, 110)]),
            ],
            2,
        );
        assert_eq!(
            resolve_story_position_v1(&map, 1, None, None)
                .unwrap_err()
                .code,
            "caret_affinity_required"
        );
        let stop = map
            .caret_stops
            .iter()
            .find(|s| s.scalar_boundary == 1 && s.line_id == "l1")
            .unwrap();
        assert_eq!(
            resolve_story_position_v1(&map, 1, Some(&stop.stop_id), None)
                .unwrap()
                .line_id,
            "l1"
        );
    }

    #[test]
    fn internal_authority_and_unsupported_are_distinct() {
        let mut c = cluster(0, 2, 0, 20);
        let unsupported = build(vec![line("l0", 0, None, None, vec![c.clone()])], 2);
        assert_eq!(
            resolve_story_position_v1(&unsupported, 1, None, None)
                .unwrap_err()
                .code,
            "internal_cluster_unsupported"
        );
        c.internal_caret_stops.push(InternalCaretStopV1 {
            scalar_boundary: 1,
            page_x_emu: 7,
            frame_x_emu: 7,
        });
        let supported = build(vec![line("l0", 0, None, None, vec![c])], 2);
        let stop = resolve_story_position_v1(&supported, 1, None, None).unwrap();
        assert_eq!(stop.affinities, vec!["internal".to_owned()]);
        assert_eq!(stop.page_x_emu, 7);
    }

    #[test]
    fn selection_preserves_partial_and_unplaced_ranges() {
        let map = build(
            vec![line(
                "l0",
                0,
                None,
                None,
                vec![cluster(0, 1, 0, 10), cluster(1, 2, 10, 20)],
            )],
            5,
        );
        let partial = selection_geometry_v1(&map, 1, 4, None).unwrap();
        assert_eq!(partial.coverage_state, "partial");
        assert_eq!(
            partial.covered_ranges,
            vec![ScalarRangeV1 {
                start_scalar: 1,
                end_scalar: 2
            }]
        );
        assert_eq!(
            partial.unplaced_ranges,
            vec![ScalarRangeV1 {
                start_scalar: 2,
                end_scalar: 4
            }]
        );
        let unplaced = selection_geometry_v1(&map, 3, 5, None).unwrap();
        assert_eq!(unplaced.coverage_state, "unplaced");
        assert!(unplaced.rectangles.is_empty());
    }
}
