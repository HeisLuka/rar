//! Source-neutral shaped-flow -> fixed text run adapter.
//!
//! This crate deliberately does not shape text and does not serialize PDF.
//! It consumes already-resolved visible line glyphs and materializes the exact
//! fixed-run payload/provenance required by FIXED-PDF-SHAPED-FLOW-01.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{error::Error, fmt};

pub const RECEIPT_VERSION_V1: &str = "chaptera.fixed-pdf-shaped-flow-receipt.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProducerV1 {
    pub implementation: String,
    pub commit_or_build: String,
    pub core_integration: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedGlyphV1 {
    pub glyph_id: u32,
    pub cluster: u32,
    pub x_advance: i64,
    pub y_advance: i64,
    pub x_offset: i64,
    pub y_offset: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapedLineInputV1 {
    pub line_index: usize,
    pub frame_line_index: u32,
    pub frame_node_id: String,
    pub story_id: String,
    pub scalar_start: u32,
    pub scalar_end: u32,
    pub units_per_em: u32,
    pub measured_width: u64,
    pub glyphs: Vec<ResolvedGlyphV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapedFlowInputV1 {
    pub producer: ProducerV1,
    pub source_hash: String,
    pub font_size_emu: i64,
    pub line_height_emu: i64,
    pub story_overset: bool,
    pub lines: Vec<ShapedLineInputV1>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedTextRunV1 {
    pub run_index: usize,
    pub frame_node_id: String,
    pub story_id: String,
    pub scalar_base: u32,
    pub scalar_end: u32,
    pub units_per_em: u32,
    pub measured_width: u64,
    pub baseline_x: i64,
    pub baseline_y: i64,
    pub glyphs: Vec<ResolvedGlyphV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineReceiptV1 {
    pub line_index: usize,
    pub frame_node_id: String,
    pub story_id: String,
    pub scalar_start: u32,
    pub scalar_end: u32,
    pub glyph_count: usize,
    pub glyph_sequence_hash: String,
    pub units_per_em: u32,
    pub measured_width: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunReceiptV1 {
    pub run_index: usize,
    pub frame_node_id: String,
    pub story_id: String,
    pub scalar_base: u32,
    pub scalar_end: u32,
    pub glyph_count: usize,
    pub glyph_sequence_hash: String,
    pub baseline_x: i64,
    pub baseline_y: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantsV1 {
    pub reshaping_calls: u32,
    pub raw_text_emitted: bool,
    pub ascii_gate_applied: bool,
    pub overset_tail_painted: bool,
    pub line_order_preserved: bool,
    pub story_global_clusters_preserved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapedFlowReceiptV1 {
    pub receipt_version: String,
    pub producer: ProducerV1,
    pub source_hash: String,
    pub flow_id: String,
    pub lines: Vec<LineReceiptV1>,
    pub runs: Vec<RunReceiptV1>,
    pub story_overset: bool,
    pub invariants: InvariantsV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixedFlowError {
    InvalidSourceHash,
    InvalidProducerField(&'static str),
    InvalidUuid(&'static str),
    InvalidLineHeight,
    InvalidFontSize,
    NonCanonicalLineIndex {
        expected: usize,
        actual: usize,
    },
    InvalidScalarRange {
        line_index: usize,
    },
    ZeroUnitsPerEm {
        line_index: usize,
    },
    BaselineOverflow {
        line_index: usize,
    },
    ClusterBeforeScalarBase {
        run_index: usize,
        cluster: u32,
        scalar_base: u32,
    },
    ClusterOutsideRun {
        run_index: usize,
        local_cluster: u32,
        logical_scalar_len: u32,
    },
    Serialization,
}

impl fmt::Display for FixedFlowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSourceHash => write!(f, "source_hash must be lowercase SHA-256"),
            Self::InvalidProducerField(field) => write!(f, "invalid producer field {field}"),
            Self::InvalidUuid(field) => write!(f, "{field} must be canonical lowercase UUID"),
            Self::InvalidLineHeight => write!(f, "line_height_emu must be positive"),
            Self::InvalidFontSize => write!(f, "font_size_emu must be positive"),
            Self::NonCanonicalLineIndex { expected, actual } => {
                write!(
                    f,
                    "line index {actual} is not canonical position {expected}"
                )
            }
            Self::InvalidScalarRange { line_index } => {
                write!(f, "line {line_index} has scalar_end < scalar_start")
            }
            Self::ZeroUnitsPerEm { line_index } => {
                write!(f, "line {line_index} has zero units_per_em")
            }
            Self::BaselineOverflow { line_index } => {
                write!(f, "line {line_index} baseline arithmetic overflow")
            }
            Self::ClusterBeforeScalarBase {
                run_index,
                cluster,
                scalar_base,
            } => write!(
                f,
                "run {run_index} glyph cluster {cluster} precedes scalar_base {scalar_base}"
            ),
            Self::ClusterOutsideRun {
                run_index,
                local_cluster,
                logical_scalar_len,
            } => write!(
                f,
                "run {run_index} local cluster {local_cluster} is outside logical scalar length {logical_scalar_len}"
            ),
            Self::Serialization => write!(f, "canonical glyph serialization failed"),
        }
    }
}

impl Error for FixedFlowError {}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

fn valid_token(value: &str, allow_colon: bool, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'_' | b'-')
                || (allow_colon && byte == b':')
        })
}

pub fn glyph_sequence_hash_v1(glyphs: &[ResolvedGlyphV1]) -> Result<String, FixedFlowError> {
    let bytes = serde_json::to_vec(glyphs).map_err(|_| FixedFlowError::Serialization)?;
    let digest = Sha256::digest(bytes);
    Ok(format!("sha256:{digest:x}"))
}

pub fn flow_id_v1(input: &ShapedFlowInputV1) -> Result<String, FixedFlowError> {
    let bytes = serde_json::to_vec(input).map_err(|_| FixedFlowError::Serialization)?;
    let digest = Sha256::digest(bytes);
    Ok(format!("sha256:{digest:x}"))
}

/// Convert a Story-global resolved glyph cluster into the run-local scalar
/// index used only by PDF text slicing/ActualText preparation.
///
/// The glyph object itself remains unchanged; callers must not rebase or rewrite
/// its Story-global cluster.
pub fn run_local_cluster_v1(
    run_index: usize,
    global_cluster: u32,
    scalar_base: u32,
    logical_scalar_len: u32,
) -> Result<u32, FixedFlowError> {
    let local_cluster =
        global_cluster
            .checked_sub(scalar_base)
            .ok_or(FixedFlowError::ClusterBeforeScalarBase {
                run_index,
                cluster: global_cluster,
                scalar_base,
            })?;
    if local_cluster >= logical_scalar_len {
        return Err(FixedFlowError::ClusterOutsideRun {
            run_index,
            local_cluster,
            logical_scalar_len,
        });
    }
    Ok(local_cluster)
}

pub fn materialize_fixed_runs_v1(
    input: &ShapedFlowInputV1,
) -> Result<Vec<FixedTextRunV1>, FixedFlowError> {
    if !is_lower_sha256(&input.source_hash) {
        return Err(FixedFlowError::InvalidSourceHash);
    }
    if !valid_token(&input.producer.implementation, false, 128) {
        return Err(FixedFlowError::InvalidProducerField(
            "producer.implementation",
        ));
    }
    if !valid_token(&input.producer.commit_or_build, true, 160) {
        return Err(FixedFlowError::InvalidProducerField(
            "producer.commit_or_build",
        ));
    }
    if !input.producer.core_integration {
        return Err(FixedFlowError::InvalidProducerField(
            "producer.core_integration",
        ));
    }
    if input.font_size_emu <= 0 {
        return Err(FixedFlowError::InvalidFontSize);
    }
    if input.line_height_emu <= 0 {
        return Err(FixedFlowError::InvalidLineHeight);
    }

    let mut runs = Vec::with_capacity(input.lines.len());
    for (expected_index, line) in input.lines.iter().enumerate() {
        if line.line_index != expected_index {
            return Err(FixedFlowError::NonCanonicalLineIndex {
                expected: expected_index,
                actual: line.line_index,
            });
        }
        if !valid_uuid(&line.frame_node_id) {
            return Err(FixedFlowError::InvalidUuid("frame_node_id"));
        }
        if !valid_uuid(&line.story_id) {
            return Err(FixedFlowError::InvalidUuid("story_id"));
        }
        if line.scalar_end < line.scalar_start {
            return Err(FixedFlowError::InvalidScalarRange {
                line_index: line.line_index,
            });
        }
        if line.units_per_em == 0 {
            return Err(FixedFlowError::ZeroUnitsPerEm {
                line_index: line.line_index,
            });
        }

        let row_offset = i64::from(line.frame_line_index)
            .checked_mul(input.line_height_emu)
            .ok_or(FixedFlowError::BaselineOverflow {
                line_index: line.line_index,
            })?;
        let baseline_y = row_offset.checked_add(input.font_size_emu).ok_or(
            FixedFlowError::BaselineOverflow {
                line_index: line.line_index,
            },
        )?;

        runs.push(FixedTextRunV1 {
            run_index: line.line_index,
            frame_node_id: line.frame_node_id.clone(),
            story_id: line.story_id.clone(),
            scalar_base: line.scalar_start,
            scalar_end: line.scalar_end,
            units_per_em: line.units_per_em,
            measured_width: line.measured_width,
            baseline_x: 0,
            baseline_y,
            glyphs: line.glyphs.clone(),
        });
    }
    Ok(runs)
}

pub fn build_receipt_v1(input: &ShapedFlowInputV1) -> Result<ShapedFlowReceiptV1, FixedFlowError> {
    let runs = materialize_fixed_runs_v1(input)?;

    let mut line_receipts = Vec::with_capacity(input.lines.len());
    let mut run_receipts = Vec::with_capacity(runs.len());
    for (line, run) in input.lines.iter().zip(&runs) {
        let line_hash = glyph_sequence_hash_v1(&line.glyphs)?;
        let run_hash = glyph_sequence_hash_v1(&run.glyphs)?;
        line_receipts.push(LineReceiptV1 {
            line_index: line.line_index,
            frame_node_id: line.frame_node_id.clone(),
            story_id: line.story_id.clone(),
            scalar_start: line.scalar_start,
            scalar_end: line.scalar_end,
            glyph_count: line.glyphs.len(),
            glyph_sequence_hash: line_hash,
            units_per_em: line.units_per_em,
            measured_width: line.measured_width,
        });
        run_receipts.push(RunReceiptV1 {
            run_index: run.run_index,
            frame_node_id: run.frame_node_id.clone(),
            story_id: run.story_id.clone(),
            scalar_base: run.scalar_base,
            scalar_end: run.scalar_end,
            glyph_count: run.glyphs.len(),
            glyph_sequence_hash: run_hash,
            baseline_x: run.baseline_x,
            baseline_y: run.baseline_y,
        });
    }

    Ok(ShapedFlowReceiptV1 {
        receipt_version: RECEIPT_VERSION_V1.to_owned(),
        producer: input.producer.clone(),
        source_hash: input.source_hash.clone(),
        flow_id: flow_id_v1(input)?,
        lines: line_receipts,
        runs: run_receipts,
        story_overset: input.story_overset,
        invariants: InvariantsV1 {
            reshaping_calls: 0,
            raw_text_emitted: false,
            ascii_gate_applied: false,
            overset_tail_painted: false,
            line_order_preserved: true,
            story_global_clusters_preserved: true,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: &str = "10000000-0000-4000-8000-000000000001";
    const STORY: &str = "20000000-0000-4000-8000-000000000001";

    fn glyph(id: u32, cluster: u32) -> ResolvedGlyphV1 {
        ResolvedGlyphV1 {
            glyph_id: id,
            cluster,
            x_advance: 500,
            y_advance: 0,
            x_offset: 0,
            y_offset: 0,
        }
    }

    fn line(
        index: usize,
        row: u32,
        start: u32,
        end: u32,
        glyphs: Vec<ResolvedGlyphV1>,
    ) -> ShapedLineInputV1 {
        ShapedLineInputV1 {
            line_index: index,
            frame_line_index: row,
            frame_node_id: FRAME.to_owned(),
            story_id: STORY.to_owned(),
            scalar_start: start,
            scalar_end: end,
            units_per_em: 1000,
            measured_width: 500 * u64::try_from(glyphs.len()).expect("glyph count"),
            glyphs,
        }
    }

    fn input() -> ShapedFlowInputV1 {
        ShapedFlowInputV1 {
            producer: ProducerV1 {
                implementation: "rar-fixed-flow-adapter".to_owned(),
                commit_or_build: "synthetic-test".to_owned(),
                core_integration: true,
            },
            source_hash: "a".repeat(64),
            font_size_emu: 1000,
            line_height_emu: 1200,
            story_overset: true,
            lines: vec![
                line(0, 0, 0, 1, vec![glyph(10, 0)]),
                line(1, 1, 2, 4, vec![glyph(20, 2), glyph(21, 3)]),
            ],
        }
    }

    #[test]
    fn visible_lines_map_one_to_one_without_reshaping() {
        let input = input();
        let runs = materialize_fixed_runs_v1(&input).expect("runs");
        assert_eq!(runs.len(), input.lines.len());
        assert_eq!(runs[0].glyphs, input.lines[0].glyphs);
        assert_eq!(runs[1].glyphs, input.lines[1].glyphs);
        assert_eq!(runs[0].baseline_y, 1000);
        assert_eq!(runs[1].baseline_y, 2200);
        assert_eq!(runs[1].scalar_base, 2);
    }

    #[test]
    fn later_line_story_global_clusters_are_preserved_but_slice_locally() {
        let input = input();
        let runs = materialize_fixed_runs_v1(&input).expect("runs");
        let second = &runs[1];
        assert_eq!(second.glyphs[0].cluster, 2);
        assert_eq!(second.glyphs[1].cluster, 3);
        assert_eq!(
            run_local_cluster_v1(1, second.glyphs[0].cluster, second.scalar_base, 2).unwrap(),
            0
        );
        assert_eq!(
            run_local_cluster_v1(1, second.glyphs[1].cluster, second.scalar_base, 2).unwrap(),
            1
        );
    }

    #[test]
    fn cluster_underflow_and_out_of_range_fail_closed() {
        assert!(matches!(
            run_local_cluster_v1(1, 1, 2, 2),
            Err(FixedFlowError::ClusterBeforeScalarBase { .. })
        ));
        assert!(matches!(
            run_local_cluster_v1(1, 4, 2, 2),
            Err(FixedFlowError::ClusterOutsideRun { .. })
        ));
    }

    #[test]
    fn receipt_hashes_prove_glyph_sequence_identity_and_no_ascii_gate() {
        let receipt = build_receipt_v1(&input()).expect("receipt");
        assert_eq!(receipt.lines.len(), 2);
        assert_eq!(
            receipt.lines[1].glyph_sequence_hash,
            receipt.runs[1].glyph_sequence_hash
        );
        assert_eq!(receipt.runs[1].scalar_base, 2);
        assert_eq!(receipt.runs[1].baseline_y, 2200);
        assert!(receipt.story_overset);
        assert_eq!(receipt.invariants.reshaping_calls, 0);
        assert!(!receipt.invariants.ascii_gate_applied);
        assert!(!receipt.invariants.overset_tail_painted);
    }

    #[test]
    fn non_ascii_is_not_a_bridge_predicate() {
        // The adapter sees resolved glyph IDs/clusters only. A glyph originating
        // from a supported non-ASCII scalar is indistinguishable here from ASCII,
        // which is the required boundary.
        let mut fixture = input();
        fixture.lines = vec![line(0, 0, 0, 1, vec![glyph(0x410, 0)])];
        assert!(materialize_fixed_runs_v1(&fixture).is_ok());
    }

    #[test]
    fn noncanonical_order_and_invalid_baseline_inputs_fail_closed() {
        let mut bad_order = input();
        bad_order.lines[1].line_index = 9;
        assert!(matches!(
            materialize_fixed_runs_v1(&bad_order),
            Err(FixedFlowError::NonCanonicalLineIndex { .. })
        ));

        let mut bad_baseline = input();
        bad_baseline.line_height_emu = 0;
        assert_eq!(
            materialize_fixed_runs_v1(&bad_baseline),
            Err(FixedFlowError::InvalidLineHeight)
        );
    }
}
