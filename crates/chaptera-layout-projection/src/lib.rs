//! Source-neutral Story -> paragraph/shaping projection for
//! LAYOUT-PARAGRAPH-PROJECTION-01.
//!
//! This crate intentionally does not read PUB/Quill/BTE carriers and does not
//! shape text. It projects canonical Story Unicode-scalar semantics plus already
//! resolved shaping-run fingerprints into a deterministic layout-facing view.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const PARAGRAPH_PROJECTION_SCHEMA_V1: &str = "chaptera.layout-paragraph-projection.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedShapingRunInputV1 {
    pub scalar_start: u32,
    pub scalar_end: u32,
    pub resolved_shaping_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryProjectionInputV1 {
    pub story_id: String,
    pub text: String,
    /// Canonical paragraph identities in semantic Story order.
    pub paragraph_ids: Vec<String>,
    /// Effective shaping-affecting runs. Input enumeration order is not
    /// semantic; scalar coordinates are.
    pub shaping_runs: Vec<ResolvedShapingRunInputV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedParagraphV1 {
    pub origin: String,
    /// Inclusive start of the paragraph-owned canonical Story range.
    pub scalar_start: u32,
    /// Exclusive end of the paragraph-owned canonical Story range.
    ///
    /// When a U+000D terminator exists, this includes that scalar. The final
    /// paragraph without a terminator ends at Story scalar_len.
    pub scalar_end: u32,
    /// Exclusive end of logical paragraph content before U+000D.
    pub content_scalar_end: u32,
    /// Canonical Story scalar index of U+000D, when this paragraph owns one.
    pub terminator_scalar: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedShapingRunV1 {
    pub paragraph_origin: String,
    pub scalar_start: u32,
    pub scalar_end: u32,
    pub resolved_shaping_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionSeverityV1 {
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionDiagnosticV1 {
    pub code: String,
    pub severity: ProjectionSeverityV1,
    pub story_id: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedStoryTextV1 {
    pub schema_version: String,
    pub story_id: String,
    pub scalar_len: u32,
    pub paragraphs: Vec<ProjectedParagraphV1>,
    /// Effective shaping spans normalized into scalar order and split at
    /// paragraph ownership boundaries. No output span crosses a paragraph.
    pub shaping_runs: Vec<ProjectedShapingRunV1>,
    pub diagnostics: Vec<ProjectionDiagnosticV1>,
    pub output_fingerprint: String,
}

impl ProjectedStoryTextV1 {
    pub fn is_usable_for_preparation(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParagraphExtentV1 {
    scalar_start: u32,
    scalar_end: u32,
    content_scalar_end: u32,
    terminator_scalar: Option<u32>,
}

pub fn project_story_text_v1(input: StoryProjectionInputV1) -> ProjectedStoryTextV1 {
    let scalar_len = match u32::try_from(input.text.chars().count()) {
        Ok(value) => value,
        Err(_) => {
            let diagnostics = vec![diagnostic(
                &input.story_id,
                "story_scalar_count_overflow",
                "Story scalar count does not fit the V1 u32 coordinate space",
            )];
            return finish_projection(input.story_id, 0, Vec::new(), Vec::new(), diagnostics);
        }
    };

    let extents = paragraph_extents_v1(&input.text);
    let mut diagnostics = Vec::new();

    let paragraph_identity_valid = validate_paragraph_identities(
        &input.story_id,
        &input.paragraph_ids,
        extents.len(),
        &mut diagnostics,
    );

    let paragraphs = if paragraph_identity_valid {
        input
            .paragraph_ids
            .iter()
            .zip(extents.iter())
            .map(|(origin, extent)| ProjectedParagraphV1 {
                origin: origin.clone(),
                scalar_start: extent.scalar_start,
                scalar_end: extent.scalar_end,
                content_scalar_end: extent.content_scalar_end,
                terminator_scalar: extent.terminator_scalar,
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let normalized_inputs = normalize_shaping_runs(
        &input.story_id,
        scalar_len,
        input.shaping_runs,
        &mut diagnostics,
    );

    let shaping_runs = if paragraph_identity_valid && normalized_inputs.is_some() {
        split_shaping_runs_at_paragraphs(
            normalized_inputs.as_deref().expect("checked Some"),
            &paragraphs,
        )
    } else {
        Vec::new()
    };

    finish_projection(
        input.story_id,
        scalar_len,
        paragraphs,
        shaping_runs,
        diagnostics,
    )
}

fn paragraph_extents_v1(text: &str) -> Vec<ParagraphExtentV1> {
    let mut extents = Vec::new();
    let mut start = 0_u32;
    let mut scalar = 0_u32;

    for ch in text.chars() {
        if ch == '\r' {
            extents.push(ParagraphExtentV1 {
                scalar_start: start,
                scalar_end: scalar + 1,
                content_scalar_end: scalar,
                terminator_scalar: Some(scalar),
            });
            start = scalar + 1;
        }
        scalar += 1;
    }

    // Story end always terminates the final Chaptera paragraph. This deliberately
    // yields an empty final paragraph for an empty Story or a Story ending in CR.
    extents.push(ParagraphExtentV1 {
        scalar_start: start,
        scalar_end: scalar,
        content_scalar_end: scalar,
        terminator_scalar: None,
    });

    extents
}

fn validate_paragraph_identities(
    story_id: &str,
    paragraph_ids: &[String],
    expected_count: usize,
    diagnostics: &mut Vec<ProjectionDiagnosticV1>,
) -> bool {
    let mut valid = true;

    if paragraph_ids.len() != expected_count {
        diagnostics.push(diagnostic(
            story_id,
            "paragraph_topology_mismatch",
            &format!(
                "canonical text implies {expected_count} paragraphs but {} ParagraphIds were supplied",
                paragraph_ids.len()
            ),
        ));
        valid = false;
    }

    if paragraph_ids.iter().any(String::is_empty) {
        diagnostics.push(diagnostic(
            story_id,
            "empty_paragraph_identity",
            "ParagraphId must not be empty",
        ));
        valid = false;
    }

    let unique = paragraph_ids.iter().collect::<BTreeSet<_>>();
    if unique.len() != paragraph_ids.len() {
        diagnostics.push(diagnostic(
            story_id,
            "duplicate_paragraph_identity",
            "ParagraphIds must be unique inside one Story projection",
        ));
        valid = false;
    }

    valid
}

fn normalize_shaping_runs(
    story_id: &str,
    scalar_len: u32,
    mut runs: Vec<ResolvedShapingRunInputV1>,
    diagnostics: &mut Vec<ProjectionDiagnosticV1>,
) -> Option<Vec<ResolvedShapingRunInputV1>> {
    runs.sort_by(|left, right| {
        (
            left.scalar_start,
            left.scalar_end,
            &left.resolved_shaping_fingerprint,
        )
            .cmp(&(
                right.scalar_start,
                right.scalar_end,
                &right.resolved_shaping_fingerprint,
            ))
    });

    let diagnostics_before = diagnostics.len();

    if scalar_len == 0 {
        if !runs.is_empty() {
            diagnostics.push(diagnostic(
                story_id,
                "shaping_run_out_of_bounds",
                "empty Story must not contain non-empty shaping runs",
            ));
        }
        return (diagnostics.len() == diagnostics_before).then_some(Vec::new());
    }

    let mut cursor = 0_u32;

    for run in &runs {
        if run.resolved_shaping_fingerprint.is_empty() {
            diagnostics.push(diagnostic(
                story_id,
                "empty_shaping_fingerprint",
                "effective shaping run must carry a stable resolved fingerprint",
            ));
        }

        if run.scalar_start >= run.scalar_end || run.scalar_end > scalar_len {
            diagnostics.push(diagnostic(
                story_id,
                "shaping_run_out_of_bounds",
                &format!(
                    "invalid shaping range [{}..{}) for Story scalar_len={scalar_len}",
                    run.scalar_start, run.scalar_end
                ),
            ));
            continue;
        }

        if run.scalar_start > cursor {
            diagnostics.push(diagnostic(
                story_id,
                "shaping_run_gap",
                &format!(
                    "no effective shaping run covers canonical scalar range [{cursor}..{})",
                    run.scalar_start
                ),
            ));
        } else if run.scalar_start < cursor {
            diagnostics.push(diagnostic(
                story_id,
                "shaping_run_overlap",
                &format!(
                    "effective shaping run [{}..{}) overlaps already covered scalars ending at {cursor}",
                    run.scalar_start, run.scalar_end
                ),
            ));
        }

        cursor = cursor.max(run.scalar_end);
    }

    if cursor < scalar_len {
        diagnostics.push(diagnostic(
            story_id,
            "shaping_run_gap",
            &format!(
                "no effective shaping run covers canonical scalar range [{cursor}..{scalar_len})"
            ),
        ));
    }

    (diagnostics.len() == diagnostics_before).then_some(runs)
}

fn split_shaping_runs_at_paragraphs(
    runs: &[ResolvedShapingRunInputV1],
    paragraphs: &[ProjectedParagraphV1],
) -> Vec<ProjectedShapingRunV1> {
    let mut output = Vec::new();

    for paragraph in paragraphs {
        for run in runs {
            let scalar_start = run.scalar_start.max(paragraph.scalar_start);
            let scalar_end = run.scalar_end.min(paragraph.scalar_end);
            if scalar_start < scalar_end {
                output.push(ProjectedShapingRunV1 {
                    paragraph_origin: paragraph.origin.clone(),
                    scalar_start,
                    scalar_end,
                    resolved_shaping_fingerprint: run.resolved_shaping_fingerprint.clone(),
                });
            }
        }
    }

    output.sort_by(|left, right| {
        (
            left.scalar_start,
            left.scalar_end,
            &left.paragraph_origin,
            &left.resolved_shaping_fingerprint,
        )
            .cmp(&(
                right.scalar_start,
                right.scalar_end,
                &right.paragraph_origin,
                &right.resolved_shaping_fingerprint,
            ))
    });

    output
}

fn finish_projection(
    story_id: String,
    scalar_len: u32,
    paragraphs: Vec<ProjectedParagraphV1>,
    shaping_runs: Vec<ProjectedShapingRunV1>,
    mut diagnostics: Vec<ProjectionDiagnosticV1>,
) -> ProjectedStoryTextV1 {
    diagnostics.sort_by(|left, right| {
        (&left.code, &left.message, &left.story_id).cmp(&(
            &right.code,
            &right.message,
            &right.story_id,
        ))
    });
    diagnostics.dedup();

    let output_fingerprint = projection_fingerprint_v1(
        &story_id,
        scalar_len,
        &paragraphs,
        &shaping_runs,
        &diagnostics,
    );

    ProjectedStoryTextV1 {
        schema_version: PARAGRAPH_PROJECTION_SCHEMA_V1.to_owned(),
        story_id,
        scalar_len,
        paragraphs,
        shaping_runs,
        diagnostics,
        output_fingerprint,
    }
}

fn projection_fingerprint_v1(
    story_id: &str,
    scalar_len: u32,
    paragraphs: &[ProjectedParagraphV1],
    shaping_runs: &[ProjectedShapingRunV1],
    diagnostics: &[ProjectionDiagnosticV1],
) -> String {
    let mut hasher = Sha256::new();
    hash_bytes(&mut hasher, PARAGRAPH_PROJECTION_SCHEMA_V1.as_bytes());
    hash_bytes(&mut hasher, story_id.as_bytes());
    hasher.update(scalar_len.to_be_bytes());

    for paragraph in paragraphs {
        hash_bytes(&mut hasher, paragraph.origin.as_bytes());
        hasher.update(paragraph.scalar_start.to_be_bytes());
        hasher.update(paragraph.scalar_end.to_be_bytes());
        hasher.update(paragraph.content_scalar_end.to_be_bytes());
        match paragraph.terminator_scalar {
            Some(value) => {
                hasher.update([1]);
                hasher.update(value.to_be_bytes());
            }
            None => hasher.update([0]),
        }
    }

    for run in shaping_runs {
        hash_bytes(&mut hasher, run.paragraph_origin.as_bytes());
        hasher.update(run.scalar_start.to_be_bytes());
        hasher.update(run.scalar_end.to_be_bytes());
        hash_bytes(&mut hasher, run.resolved_shaping_fingerprint.as_bytes());
    }

    for item in diagnostics {
        hash_bytes(&mut hasher, item.code.as_bytes());
        hash_bytes(&mut hasher, item.story_id.as_bytes());
        hash_bytes(&mut hasher, item.message.as_bytes());
    }

    format!("sha256:{:x}", hasher.finalize())
}

fn hash_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    let len = u64::try_from(bytes.len()).expect("bounded projection input");
    hasher.update(len.to_be_bytes());
    hasher.update(bytes);
}

fn diagnostic(story_id: &str, code: &str, message: &str) -> ProjectionDiagnosticV1 {
    ProjectionDiagnosticV1 {
        code: code.to_owned(),
        severity: ProjectionSeverityV1::Error,
        story_id: story_id.to_owned(),
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(start: u32, end: u32, fp: &str) -> ResolvedShapingRunInputV1 {
        ResolvedShapingRunInputV1 {
            scalar_start: start,
            scalar_end: end,
            resolved_shaping_fingerprint: fp.to_owned(),
        }
    }

    fn project(
        text: &str,
        paragraph_ids: &[&str],
        shaping_runs: Vec<ResolvedShapingRunInputV1>,
    ) -> ProjectedStoryTextV1 {
        project_story_text_v1(StoryProjectionInputV1 {
            story_id: "story-1".into(),
            text: text.into(),
            paragraph_ids: paragraph_ids.iter().map(|id| (*id).to_owned()).collect(),
            shaping_runs,
        })
    }

    #[test]
    fn final_paragraph_without_terminal_cr_uses_story_end() {
        let output = project("Alpha", &["p1"], vec![run(0, 5, "style-a")]);

        assert!(output.is_usable_for_preparation());
        assert_eq!(output.scalar_len, 5);
        assert_eq!(
            output.paragraphs,
            vec![ProjectedParagraphV1 {
                origin: "p1".into(),
                scalar_start: 0,
                scalar_end: 5,
                content_scalar_end: 5,
                terminator_scalar: None,
            }]
        );
    }

    #[test]
    fn cr_delimited_paragraphs_own_their_terminators() {
        let output = project("A\rB\rC", &["p1", "p2", "p3"], vec![run(0, 5, "style-a")]);

        assert!(output.is_usable_for_preparation());
        assert_eq!(
            output.paragraphs,
            vec![
                ProjectedParagraphV1 {
                    origin: "p1".into(),
                    scalar_start: 0,
                    scalar_end: 2,
                    content_scalar_end: 1,
                    terminator_scalar: Some(1),
                },
                ProjectedParagraphV1 {
                    origin: "p2".into(),
                    scalar_start: 2,
                    scalar_end: 4,
                    content_scalar_end: 3,
                    terminator_scalar: Some(3),
                },
                ProjectedParagraphV1 {
                    origin: "p3".into(),
                    scalar_start: 4,
                    scalar_end: 5,
                    content_scalar_end: 5,
                    terminator_scalar: None,
                },
            ]
        );
    }

    #[test]
    fn trailing_cr_creates_empty_final_paragraph() {
        let output = project("A\r", &["p1", "p2"], vec![run(0, 2, "style-a")]);

        assert!(output.is_usable_for_preparation());
        assert_eq!(
            output.paragraphs[1],
            ProjectedParagraphV1 {
                origin: "p2".into(),
                scalar_start: 2,
                scalar_end: 2,
                content_scalar_end: 2,
                terminator_scalar: None,
            }
        );
    }

    #[test]
    fn consecutive_cr_preserves_empty_middle_paragraph() {
        let output = project("A\r\rB", &["p1", "p2", "p3"], vec![run(0, 4, "style-a")]);

        assert!(output.is_usable_for_preparation());
        assert_eq!(
            output.paragraphs[1],
            ProjectedParagraphV1 {
                origin: "p2".into(),
                scalar_start: 2,
                scalar_end: 3,
                content_scalar_end: 2,
                terminator_scalar: Some(2),
            }
        );
    }

    #[test]
    fn empty_story_is_one_empty_final_paragraph() {
        let output = project("", &["p1"], vec![]);

        assert!(output.is_usable_for_preparation());
        assert_eq!(
            output.paragraphs,
            vec![ProjectedParagraphV1 {
                origin: "p1".into(),
                scalar_start: 0,
                scalar_end: 0,
                content_scalar_end: 0,
                terminator_scalar: None,
            }]
        );
        assert!(output.shaping_runs.is_empty());
    }

    #[test]
    fn paragraph_identity_count_mismatch_fails_closed_without_guessing_pairing() {
        let output = project("A\rB", &["p1"], vec![run(0, 3, "style-a")]);

        assert!(!output.is_usable_for_preparation());
        assert!(output.paragraphs.is_empty());
        assert!(output.shaping_runs.is_empty());
        assert!(
            output
                .diagnostics
                .iter()
                .any(|item| item.code == "paragraph_topology_mismatch")
        );
    }

    #[test]
    fn shaping_run_crossing_paragraph_boundary_is_split_explicitly() {
        let output = project("A\rB", &["p1", "p2"], vec![run(0, 3, "style-a")]);

        assert!(output.is_usable_for_preparation());
        assert_eq!(
            output.shaping_runs,
            vec![
                ProjectedShapingRunV1 {
                    paragraph_origin: "p1".into(),
                    scalar_start: 0,
                    scalar_end: 2,
                    resolved_shaping_fingerprint: "style-a".into(),
                },
                ProjectedShapingRunV1 {
                    paragraph_origin: "p2".into(),
                    scalar_start: 2,
                    scalar_end: 3,
                    resolved_shaping_fingerprint: "style-a".into(),
                },
            ]
        );
    }

    #[test]
    fn shaping_gap_or_overlap_is_explicit_and_not_projected() {
        let gap = project("ABCD", &["p1"], vec![run(0, 2, "a"), run(3, 4, "b")]);
        assert!(!gap.is_usable_for_preparation());
        assert!(gap.shaping_runs.is_empty());
        assert!(
            gap.diagnostics
                .iter()
                .any(|item| item.code == "shaping_run_gap")
        );

        let overlap = project("ABCD", &["p1"], vec![run(0, 3, "a"), run(2, 4, "b")]);
        assert!(!overlap.is_usable_for_preparation());
        assert!(overlap.shaping_runs.is_empty());
        assert!(
            overlap
                .diagnostics
                .iter()
                .any(|item| item.code == "shaping_run_overlap")
        );
    }

    #[test]
    fn shaping_run_input_enumeration_order_is_not_semantic() {
        let left = project("ABCD", &["p1"], vec![run(2, 4, "b"), run(0, 2, "a")]);
        let right = project("ABCD", &["p1"], vec![run(0, 2, "a"), run(2, 4, "b")]);

        assert_eq!(left, right);
        assert_eq!(
            serde_json::to_vec(&left).expect("serialize"),
            serde_json::to_vec(&right).expect("serialize")
        );
    }

    #[test]
    fn unicode_coordinates_are_story_global_scalar_indices_not_utf16() {
        let output = project("A😀\rB", &["p1", "p2"], vec![run(0, 4, "style-a")]);

        assert!(output.is_usable_for_preparation());
        assert_eq!(output.scalar_len, 4);
        assert_eq!(output.paragraphs[0].terminator_scalar, Some(2));
        assert_eq!(output.paragraphs[0].scalar_end, 3);
        assert_eq!(output.paragraphs[1].scalar_start, 3);
    }

    #[test]
    fn split_and_merge_states_follow_canonical_paragraph_identity_law() {
        let base = project("AB", &["p-upstream"], vec![run(0, 2, "style-a")]);
        let split = project("A\rB", &["p-upstream", "p-new"], vec![run(0, 3, "style-a")]);
        let merged = project("AB", &["p-upstream"], vec![run(0, 2, "style-a")]);

        assert!(base.is_usable_for_preparation());
        assert!(split.is_usable_for_preparation());
        assert!(merged.is_usable_for_preparation());
        assert_eq!(base.paragraphs[0].origin, "p-upstream");
        assert_eq!(split.paragraphs[0].origin, "p-upstream");
        assert_eq!(split.paragraphs[1].origin, "p-new");
        assert_eq!(merged.paragraphs[0].origin, "p-upstream");
        assert_eq!(base.output_fingerprint, merged.output_fingerprint);
    }

    #[test]
    fn duplicate_paragraph_identity_fails_closed() {
        let output = project("A\rB", &["p1", "p1"], vec![run(0, 3, "style-a")]);

        assert!(!output.is_usable_for_preparation());
        assert!(output.paragraphs.is_empty());
        assert!(
            output
                .diagnostics
                .iter()
                .any(|item| item.code == "duplicate_paragraph_identity")
        );
    }
}
