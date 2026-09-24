use chaptera_layout_projection::{
    ProjectedParagraphV1, ProjectedShapingRunV1, ProjectedStoryTextV1,
};

use crate::runtime::ResolvedScalarMetricV1;
use crate::{FingerprintV1, fingerprint_v1};

pub const PREPARED_PARAGRAPH_STAGE_V1: &str = "prepared-paragraph-v1";
pub const PREPARED_STORY_INDEX_STAGE_V1: &str = "prepared-story-index-v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedLocalScalarMetricV1 {
    pub local_scalar_start: u32,
    pub local_scalar_end: u32,
    pub advance_emu: i64,
    pub break_after: bool,
    pub semantic_fingerprint: FingerprintV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedLocalShapingRunV1 {
    pub local_scalar_start: u32,
    pub local_scalar_end: u32,
    pub resolved_shaping_fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedParagraphCoreV1 {
    pub story_id: String,
    pub paragraph_id: String,
    pub content_scalar_len: u32,
    pub owned_scalar_len: u32,
    pub has_mandatory_paragraph_break: bool,
    pub dependency_fingerprint: FingerprintV1,
    pub output_fingerprint: FingerprintV1,
    pub metrics: Vec<PreparedLocalScalarMetricV1>,
    pub shaping_runs: Vec<PreparedLocalShapingRunV1>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedParagraphBindingV1 {
    pub paragraph_id: String,
    pub scalar_base: u32,
    pub scalar_end: u32,
    pub content_scalar_end: u32,
    pub terminator_scalar: Option<u32>,
    /// Position-only identity. A paragraph may reuse its prepared core while this
    /// fingerprint changes because an earlier paragraph changed length.
    pub position_fingerprint: FingerprintV1,
    pub prepared: PreparedParagraphCoreV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedStoryIndexV1 {
    pub story_id: String,
    pub projection_output_fingerprint: String,
    pub output_fingerprint: FingerprintV1,
    pub paragraphs: Vec<PreparedParagraphBindingV1>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreparedParagraphErrorV1 {
    ProjectionNotUsable,
    InvalidMetricRange {
        scalar_start: u32,
        scalar_end: u32,
    },
    NonPositiveAdvance {
        scalar_start: u32,
    },
    MetricOutsideParagraphContent {
        scalar_start: u32,
        scalar_end: u32,
    },
    MetricCoverageGap {
        paragraph_id: String,
        expected_scalar: u32,
        actual_scalar: u32,
    },
    MetricCoverageIncomplete {
        paragraph_id: String,
        expected_end: u32,
        actual_end: u32,
    },
    ShapingSpanOutsideParagraph {
        paragraph_id: String,
        scalar_start: u32,
        scalar_end: u32,
    },
}

pub fn prepare_projected_story_v1(
    projected: &ProjectedStoryTextV1,
    metrics: &[ResolvedScalarMetricV1],
    environment_fingerprint: FingerprintV1,
    shaping_policy_fingerprint: FingerprintV1,
) -> Result<PreparedStoryIndexV1, PreparedParagraphErrorV1> {
    if !projected.is_usable_for_preparation() {
        return Err(PreparedParagraphErrorV1::ProjectionNotUsable);
    }

    validate_metrics(metrics)?;

    let mut assigned = vec![false; metrics.len()];
    let mut paragraphs = Vec::with_capacity(projected.paragraphs.len());

    for paragraph in &projected.paragraphs {
        let mut paragraph_metrics = Vec::new();

        for (index, metric) in metrics.iter().enumerate() {
            if metric.scalar_start >= paragraph.scalar_start
                && metric.scalar_end <= paragraph.content_scalar_end
            {
                assigned[index] = true;
                paragraph_metrics.push(metric.clone());
            }
        }

        paragraph_metrics.sort_by_key(|metric| (metric.scalar_start, metric.scalar_end));
        validate_metric_coverage(paragraph, &paragraph_metrics)?;

        let local_metrics = paragraph_metrics
            .into_iter()
            .map(|metric| PreparedLocalScalarMetricV1 {
                local_scalar_start: metric.scalar_start - paragraph.scalar_start,
                local_scalar_end: metric.scalar_end - paragraph.scalar_start,
                advance_emu: metric.advance_emu,
                break_after: metric.break_after,
                semantic_fingerprint: metric.semantic_fingerprint,
            })
            .collect::<Vec<_>>();

        let local_shaping_runs = local_shaping_runs(projected, paragraph)?;

        let content_scalar_len = paragraph.content_scalar_end - paragraph.scalar_start;
        let owned_scalar_len = paragraph.scalar_end - paragraph.scalar_start;
        let has_mandatory_paragraph_break = paragraph.terminator_scalar.is_some();

        let dependency_fingerprint = prepared_dependency_fingerprint_v1(
            &projected.story_id,
            &paragraph.origin,
            content_scalar_len,
            owned_scalar_len,
            has_mandatory_paragraph_break,
            &local_metrics,
            &local_shaping_runs,
            environment_fingerprint,
            shaping_policy_fingerprint,
        );

        let output_fingerprint = prepared_output_fingerprint_v1(
            content_scalar_len,
            owned_scalar_len,
            has_mandatory_paragraph_break,
            &local_metrics,
            &local_shaping_runs,
        );

        let position_fingerprint = paragraph_position_fingerprint_v1(paragraph);

        paragraphs.push(PreparedParagraphBindingV1 {
            paragraph_id: paragraph.origin.clone(),
            scalar_base: paragraph.scalar_start,
            scalar_end: paragraph.scalar_end,
            content_scalar_end: paragraph.content_scalar_end,
            terminator_scalar: paragraph.terminator_scalar,
            position_fingerprint,
            prepared: PreparedParagraphCoreV1 {
                story_id: projected.story_id.clone(),
                paragraph_id: paragraph.origin.clone(),
                content_scalar_len,
                owned_scalar_len,
                has_mandatory_paragraph_break,
                dependency_fingerprint,
                output_fingerprint,
                metrics: local_metrics,
                shaping_runs: local_shaping_runs,
            },
        });
    }

    if let Some(metric) = metrics
        .iter()
        .enumerate()
        .find(|(index, _)| !assigned[*index])
        .map(|(_, metric)| metric)
    {
        return Err(PreparedParagraphErrorV1::MetricOutsideParagraphContent {
            scalar_start: metric.scalar_start,
            scalar_end: metric.scalar_end,
        });
    }

    let output_fingerprint = story_index_output_fingerprint_v1(&paragraphs);

    Ok(PreparedStoryIndexV1 {
        story_id: projected.story_id.clone(),
        projection_output_fingerprint: projected.output_fingerprint.clone(),
        output_fingerprint,
        paragraphs,
    })
}

fn validate_metrics(metrics: &[ResolvedScalarMetricV1]) -> Result<(), PreparedParagraphErrorV1> {
    for metric in metrics {
        if metric.scalar_end <= metric.scalar_start {
            return Err(PreparedParagraphErrorV1::InvalidMetricRange {
                scalar_start: metric.scalar_start,
                scalar_end: metric.scalar_end,
            });
        }
        if metric.advance_emu <= 0 {
            return Err(PreparedParagraphErrorV1::NonPositiveAdvance {
                scalar_start: metric.scalar_start,
            });
        }
    }
    Ok(())
}

fn validate_metric_coverage(
    paragraph: &ProjectedParagraphV1,
    metrics: &[ResolvedScalarMetricV1],
) -> Result<(), PreparedParagraphErrorV1> {
    let mut cursor = paragraph.scalar_start;

    for metric in metrics {
        if metric.scalar_start != cursor {
            return Err(PreparedParagraphErrorV1::MetricCoverageGap {
                paragraph_id: paragraph.origin.clone(),
                expected_scalar: cursor,
                actual_scalar: metric.scalar_start,
            });
        }
        cursor = metric.scalar_end;
    }

    if cursor != paragraph.content_scalar_end {
        return Err(PreparedParagraphErrorV1::MetricCoverageIncomplete {
            paragraph_id: paragraph.origin.clone(),
            expected_end: paragraph.content_scalar_end,
            actual_end: cursor,
        });
    }

    Ok(())
}

fn local_shaping_runs(
    projected: &ProjectedStoryTextV1,
    paragraph: &ProjectedParagraphV1,
) -> Result<Vec<PreparedLocalShapingRunV1>, PreparedParagraphErrorV1> {
    let mut runs = projected
        .shaping_runs
        .iter()
        .filter(|run| run.paragraph_origin == paragraph.origin)
        .cloned()
        .collect::<Vec<ProjectedShapingRunV1>>();

    runs.sort_by_key(|run| (run.scalar_start, run.scalar_end));

    runs.into_iter()
        .map(|run| {
            if run.scalar_start < paragraph.scalar_start || run.scalar_end > paragraph.scalar_end {
                return Err(PreparedParagraphErrorV1::ShapingSpanOutsideParagraph {
                    paragraph_id: paragraph.origin.clone(),
                    scalar_start: run.scalar_start,
                    scalar_end: run.scalar_end,
                });
            }

            Ok(PreparedLocalShapingRunV1 {
                local_scalar_start: run.scalar_start - paragraph.scalar_start,
                local_scalar_end: run.scalar_end - paragraph.scalar_start,
                resolved_shaping_fingerprint: run.resolved_shaping_fingerprint,
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn prepared_dependency_fingerprint_v1(
    story_id: &str,
    paragraph_id: &str,
    content_scalar_len: u32,
    owned_scalar_len: u32,
    has_mandatory_paragraph_break: bool,
    metrics: &[PreparedLocalScalarMetricV1],
    shaping_runs: &[PreparedLocalShapingRunV1],
    environment_fingerprint: FingerprintV1,
    shaping_policy_fingerprint: FingerprintV1,
) -> FingerprintV1 {
    let mut payload = Vec::new();
    push_bytes(&mut payload, story_id.as_bytes());
    push_bytes(&mut payload, paragraph_id.as_bytes());
    push_u32(&mut payload, content_scalar_len);
    push_u32(&mut payload, owned_scalar_len);
    payload.push(u8::from(has_mandatory_paragraph_break));
    payload.extend_from_slice(&environment_fingerprint);
    payload.extend_from_slice(&shaping_policy_fingerprint);
    append_local_metrics(&mut payload, metrics);
    append_local_runs(&mut payload, shaping_runs);
    fingerprint_v1(PREPARED_PARAGRAPH_STAGE_V1, &[&payload])
}

fn prepared_output_fingerprint_v1(
    content_scalar_len: u32,
    owned_scalar_len: u32,
    has_mandatory_paragraph_break: bool,
    metrics: &[PreparedLocalScalarMetricV1],
    shaping_runs: &[PreparedLocalShapingRunV1],
) -> FingerprintV1 {
    let mut payload = Vec::new();
    push_u32(&mut payload, content_scalar_len);
    push_u32(&mut payload, owned_scalar_len);
    payload.push(u8::from(has_mandatory_paragraph_break));
    append_local_metrics(&mut payload, metrics);
    append_local_runs(&mut payload, shaping_runs);
    fingerprint_v1("prepared-paragraph-output-v1", &[&payload])
}

fn paragraph_position_fingerprint_v1(paragraph: &ProjectedParagraphV1) -> FingerprintV1 {
    let mut payload = Vec::new();
    push_u32(&mut payload, paragraph.scalar_start);
    push_u32(&mut payload, paragraph.scalar_end);
    push_u32(&mut payload, paragraph.content_scalar_end);
    match paragraph.terminator_scalar {
        Some(value) => {
            payload.push(1);
            push_u32(&mut payload, value);
        }
        None => payload.push(0),
    }
    fingerprint_v1("prepared-paragraph-position-v1", &[&payload])
}

fn story_index_output_fingerprint_v1(paragraphs: &[PreparedParagraphBindingV1]) -> FingerprintV1 {
    let mut payload = Vec::new();
    for paragraph in paragraphs {
        push_bytes(&mut payload, paragraph.paragraph_id.as_bytes());
        payload.extend_from_slice(&paragraph.prepared.output_fingerprint);
        payload.extend_from_slice(&paragraph.position_fingerprint);
    }
    fingerprint_v1(PREPARED_STORY_INDEX_STAGE_V1, &[&payload])
}

fn append_local_metrics(payload: &mut Vec<u8>, metrics: &[PreparedLocalScalarMetricV1]) {
    push_u32(
        payload,
        u32::try_from(metrics.len()).expect("bounded prepared paragraph metric count"),
    );
    for metric in metrics {
        push_u32(payload, metric.local_scalar_start);
        push_u32(payload, metric.local_scalar_end);
        payload.extend_from_slice(&metric.advance_emu.to_be_bytes());
        payload.push(u8::from(metric.break_after));
        payload.extend_from_slice(&metric.semantic_fingerprint);
    }
}

fn append_local_runs(payload: &mut Vec<u8>, runs: &[PreparedLocalShapingRunV1]) {
    push_u32(
        payload,
        u32::try_from(runs.len()).expect("bounded prepared paragraph run count"),
    );
    for run in runs {
        push_u32(payload, run.local_scalar_start);
        push_u32(payload, run.local_scalar_end);
        push_bytes(payload, run.resolved_shaping_fingerprint.as_bytes());
    }
}

fn push_bytes(payload: &mut Vec<u8>, bytes: &[u8]) {
    let len = u64::try_from(bytes.len()).expect("bounded prepared paragraph input");
    payload.extend_from_slice(&len.to_be_bytes());
    payload.extend_from_slice(bytes);
}

fn push_u32(payload: &mut Vec<u8>, value: u32) {
    payload.extend_from_slice(&value.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use chaptera_layout_projection::{
        ResolvedShapingRunInputV1, StoryProjectionInputV1, project_story_text_v1,
    };

    use super::*;

    fn fp(domain: &str, value: &str) -> FingerprintV1 {
        fingerprint_v1(domain, &[value.as_bytes()])
    }

    fn metric(start: u32, end: u32, label: &str) -> ResolvedScalarMetricV1 {
        ResolvedScalarMetricV1 {
            scalar_start: start,
            scalar_end: end,
            advance_emu: 20,
            break_after: label.ends_with(' '),
            semantic_fingerprint: fp("metric-semantic-v1", label),
        }
    }

    fn projection(
        text: &str,
        paragraph_ids: &[&str],
        shaping_fingerprint: &str,
    ) -> ProjectedStoryTextV1 {
        let scalar_len = u32::try_from(text.chars().count()).expect("small fixture");
        let shaping_runs = if scalar_len == 0 {
            vec![]
        } else {
            vec![ResolvedShapingRunInputV1 {
                scalar_start: 0,
                scalar_end: scalar_len,
                resolved_shaping_fingerprint: shaping_fingerprint.to_owned(),
            }]
        };

        project_story_text_v1(StoryProjectionInputV1 {
            story_id: "s1".into(),
            text: text.into(),
            paragraph_ids: paragraph_ids.iter().map(|id| (*id).to_owned()).collect(),
            shaping_runs,
        })
    }

    fn prepare(
        projected: &ProjectedStoryTextV1,
        metrics: &[ResolvedScalarMetricV1],
    ) -> PreparedStoryIndexV1 {
        prepare_projected_story_v1(
            projected,
            metrics,
            fp("env-v1", "font-a"),
            fp("policy-v1", "p"),
        )
        .expect("prepare")
    }

    #[test]
    fn prepares_paragraph_local_metrics_and_keeps_cr_as_boundary_not_metric() {
        let projected = projection("A\rBC", &["p1", "p2"], "style-a");
        let prepared = prepare(
            &projected,
            &[metric(0, 1, "A"), metric(2, 3, "B"), metric(3, 4, "C")],
        );

        assert_eq!(prepared.paragraphs.len(), 2);

        let first = &prepared.paragraphs[0];
        assert_eq!(first.scalar_base, 0);
        assert_eq!(first.terminator_scalar, Some(1));
        assert_eq!(first.prepared.content_scalar_len, 1);
        assert_eq!(first.prepared.owned_scalar_len, 2);
        assert_eq!(first.prepared.metrics.len(), 1);
        assert_eq!(first.prepared.metrics[0].local_scalar_start, 0);
        assert_eq!(first.prepared.metrics[0].local_scalar_end, 1);

        let second = &prepared.paragraphs[1];
        assert_eq!(second.scalar_base, 2);
        assert_eq!(second.terminator_scalar, None);
        assert_eq!(second.prepared.content_scalar_len, 2);
        assert_eq!(
            second
                .prepared
                .metrics
                .iter()
                .map(|metric| (metric.local_scalar_start, metric.local_scalar_end))
                .collect::<Vec<_>>(),
            vec![(0, 1), (1, 2)]
        );
    }

    #[test]
    fn downstream_paragraph_core_reuses_after_earlier_paragraph_length_shift() {
        let before_projection = projection("A\rBC", &["p1", "p2"], "style-a");
        let before = prepare(
            &before_projection,
            &[metric(0, 1, "A"), metric(2, 3, "B"), metric(3, 4, "C")],
        );

        let after_projection = projection("AAAA\rBC", &["p1", "p2"], "style-a");
        let after = prepare(
            &after_projection,
            &[
                metric(0, 1, "A"),
                metric(1, 2, "A"),
                metric(2, 3, "A"),
                metric(3, 4, "A"),
                metric(5, 6, "B"),
                metric(6, 7, "C"),
            ],
        );

        let before_p2 = &before.paragraphs[1];
        let after_p2 = &after.paragraphs[1];

        assert_eq!(before_p2.paragraph_id, "p2");
        assert_eq!(before_p2.scalar_base, 2);
        assert_eq!(after_p2.scalar_base, 5);
        assert_ne!(
            before_p2.position_fingerprint,
            after_p2.position_fingerprint
        );
        assert_eq!(
            before_p2.prepared.dependency_fingerprint,
            after_p2.prepared.dependency_fingerprint
        );
        assert_eq!(
            before_p2.prepared.output_fingerprint,
            after_p2.prepared.output_fingerprint
        );

        assert_ne!(
            before.paragraphs[0].prepared.output_fingerprint,
            after.paragraphs[0].prepared.output_fingerprint
        );
        assert_ne!(before.output_fingerprint, after.output_fingerprint);
    }

    #[test]
    fn shaping_style_change_invalidates_only_actual_paragraph_consumer() {
        let projected_a = project_story_text_v1(StoryProjectionInputV1 {
            story_id: "s1".into(),
            text: "A\rBC".into(),
            paragraph_ids: vec!["p1".into(), "p2".into()],
            shaping_runs: vec![
                ResolvedShapingRunInputV1 {
                    scalar_start: 0,
                    scalar_end: 2,
                    resolved_shaping_fingerprint: "style-a".into(),
                },
                ResolvedShapingRunInputV1 {
                    scalar_start: 2,
                    scalar_end: 4,
                    resolved_shaping_fingerprint: "style-b".into(),
                },
            ],
        });
        let projected_b = project_story_text_v1(StoryProjectionInputV1 {
            story_id: "s1".into(),
            text: "A\rBC".into(),
            paragraph_ids: vec!["p1".into(), "p2".into()],
            shaping_runs: vec![
                ResolvedShapingRunInputV1 {
                    scalar_start: 0,
                    scalar_end: 2,
                    resolved_shaping_fingerprint: "style-a2".into(),
                },
                ResolvedShapingRunInputV1 {
                    scalar_start: 2,
                    scalar_end: 4,
                    resolved_shaping_fingerprint: "style-b".into(),
                },
            ],
        });
        let metrics = [metric(0, 1, "A"), metric(2, 3, "B"), metric(3, 4, "C")];

        let a = prepare(&projected_a, &metrics);
        let b = prepare(&projected_b, &metrics);

        assert_ne!(
            a.paragraphs[0].prepared.dependency_fingerprint,
            b.paragraphs[0].prepared.dependency_fingerprint
        );
        assert_ne!(
            a.paragraphs[0].prepared.output_fingerprint,
            b.paragraphs[0].prepared.output_fingerprint
        );
        assert_eq!(
            a.paragraphs[1].prepared.dependency_fingerprint,
            b.paragraphs[1].prepared.dependency_fingerprint
        );
        assert_eq!(
            a.paragraphs[1].prepared.output_fingerprint,
            b.paragraphs[1].prepared.output_fingerprint
        );
    }

    #[test]
    fn paragraph_split_changes_topology_instead_of_reusing_old_whole_paragraph() {
        let before_projection = projection("AB", &["p1"], "style-a");
        let before = prepare(&before_projection, &[metric(0, 1, "A"), metric(1, 2, "B")]);

        let after_projection = projection("A\rB", &["p1", "p-new"], "style-a");
        let after = prepare(&after_projection, &[metric(0, 1, "A"), metric(2, 3, "B")]);

        assert_eq!(before.paragraphs.len(), 1);
        assert_eq!(after.paragraphs.len(), 2);
        assert_eq!(after.paragraphs[0].paragraph_id, "p1");
        assert_eq!(after.paragraphs[1].paragraph_id, "p-new");
        assert_ne!(
            before.paragraphs[0].prepared.output_fingerprint,
            after.paragraphs[0].prepared.output_fingerprint
        );
    }

    #[test]
    fn empty_paragraph_is_a_valid_prepared_unit_without_fake_glyph_metric() {
        let projected = projection("A\r\rB", &["p1", "p2", "p3"], "style-a");
        let prepared = prepare(&projected, &[metric(0, 1, "A"), metric(3, 4, "B")]);

        let empty = &prepared.paragraphs[1];
        assert_eq!(empty.prepared.content_scalar_len, 0);
        assert_eq!(empty.prepared.owned_scalar_len, 1);
        assert!(empty.prepared.has_mandatory_paragraph_break);
        assert!(empty.prepared.metrics.is_empty());
        assert_eq!(empty.terminator_scalar, Some(2));
    }

    #[test]
    fn metric_for_cr_or_crossing_paragraph_boundary_is_rejected() {
        let projected = projection("A\rB", &["p1", "p2"], "style-a");

        let error = prepare_projected_story_v1(
            &projected,
            &[metric(0, 1, "A"), metric(1, 2, "CR"), metric(2, 3, "B")],
            fp("env-v1", "font-a"),
            fp("policy-v1", "p"),
        )
        .expect_err("CR is a boundary, not a content metric");

        assert_eq!(
            error,
            PreparedParagraphErrorV1::MetricOutsideParagraphContent {
                scalar_start: 1,
                scalar_end: 2,
            }
        );

        let crossing = prepare_projected_story_v1(
            &projected,
            &[metric(0, 3, "cross")],
            fp("env-v1", "font-a"),
            fp("policy-v1", "p"),
        )
        .expect_err("crossing metric must fail");

        assert!(matches!(
            crossing,
            PreparedParagraphErrorV1::MetricCoverageIncomplete { .. }
                | PreparedParagraphErrorV1::MetricOutsideParagraphContent { .. }
        ));
    }

    #[test]
    fn unusable_projection_is_rejected_before_preparation() {
        let projected = projection("A\rB", &["only-one-id"], "style-a");
        assert!(!projected.is_usable_for_preparation());

        let error = prepare_projected_story_v1(
            &projected,
            &[metric(0, 1, "A"), metric(2, 3, "B")],
            fp("env-v1", "font-a"),
            fp("policy-v1", "p"),
        )
        .expect_err("invalid projection must fail closed");

        assert_eq!(error, PreparedParagraphErrorV1::ProjectionNotUsable);
    }
}
