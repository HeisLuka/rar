use std::collections::{BTreeMap, BTreeSet};

use crate::prepared_paragraph::{
    PreparedParagraphBindingV1, PreparedStoryIndexV1,
};
use crate::runtime::{IntervalPolicyV1, LineRegionV1};
use crate::{FingerprintV1, fingerprint_v1};

pub const FRAME_CONTINUATION_SCHEMA_V1: &str = "frame-continuation-v1";
pub const LINKED_STORY_FLOW_STAGE_V1: &str = "linked-story-flow-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContinuationTerminalV1 {
    Continue,
    Complete,
    Overset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameContinuationStateV1 {
    pub schema_version: String,
    pub story_id: String,
    pub next_scalar: u32,
    pub paragraph_id: Option<String>,
    pub prepared_boundary_fingerprint: Option<FingerprintV1>,
    pub layout_environment_fingerprint: FingerprintV1,
    pub flow_policy_fingerprint: FingerprintV1,
    pub terminal: ContinuationTerminalV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownstreamDependencyStateV1 {
    pub prepared_suffix_fingerprint: FingerprintV1,
    pub frame_suffix_fingerprint: FingerprintV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameBoundaryStateV1 {
    pub continuation: FrameContinuationStateV1,
    pub downstream: DownstreamDependencyStateV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedFrameInputV1 {
    pub frame_id: String,
    pub previous_frame_id: Option<String>,
    pub next_frame_id: Option<String>,
    pub region: LineRegionV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedFlowLineV1 {
    pub row_index: u32,
    pub x0_emu: i64,
    pub x1_emu: i64,
    pub paragraph_id: String,
    pub scalar_start: u32,
    pub scalar_end: u32,
    pub consumed_scalar_end: u32,
    pub measured_width_emu: i64,
    pub mandatory_break: bool,
    pub content_fingerprint: FingerprintV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedLinkedFrameV1 {
    pub frame_id: String,
    pub frame_dependency_fingerprint: FingerprintV1,
    pub dependency_fingerprint: FingerprintV1,
    pub output_fingerprint: FingerprintV1,
    pub input_continuation: FrameContinuationStateV1,
    pub output_boundary: FrameBoundaryStateV1,
    pub lines: Vec<LinkedFlowLineV1>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedStoryFlowV1 {
    pub story_id: String,
    pub layout_environment_fingerprint: FingerprintV1,
    pub flow_policy_fingerprint: FingerprintV1,
    pub output_fingerprint: FingerprintV1,
    pub frames: Vec<ResolvedLinkedFrameV1>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IncrementalExecutionModeV1 {
    Bounded,
    FullFallback,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncrementalLinkedStoryFlowV1 {
    pub mode: IncrementalExecutionModeV1,
    pub requested_start_frame_id: String,
    pub actual_start_frame_id: String,
    pub recomputed_frame_ids: Vec<String>,
    pub convergence_after_frame_id: Option<String>,
    pub reused_suffix_from_frame_id: Option<String>,
    pub flow: LinkedStoryFlowV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkedFlowErrorV1 {
    MissingFrames,
    DuplicateFrameId { frame_id: String },
    RegionFrameMismatch { frame_id: String, region_frame_id: String },
    InvalidHeadCount { count: usize },
    MissingLinkedFrame { frame_id: String },
    BrokenPreviousLink {
        frame_id: String,
        expected_previous: String,
        actual_previous: Option<String>,
    },
    CycleOrDisconnectedChain,
    StoryIdMismatch,
    RequestedStartFrameMissing { frame_id: String },
    MissingMetricAtCursor {
        paragraph_id: String,
        scalar: u32,
    },
    CursorOutsidePreparedStory { scalar: u32 },
}

pub fn resolve_linked_story_full_v1(
    prepared: &PreparedStoryIndexV1,
    frames: &[LinkedFrameInputV1],
    layout_environment_fingerprint: FingerprintV1,
    flow_policy_fingerprint: FingerprintV1,
    interval_policy: IntervalPolicyV1,
) -> Result<LinkedStoryFlowV1, LinkedFlowErrorV1> {
    let ordered = ordered_frame_indices(frames)?;
    let ordered_frames = ordered
        .iter()
        .map(|index| &frames[*index])
        .collect::<Vec<_>>();

    let story_end = story_end_scalar(prepared);
    let initial_terminal = if story_end == 0 {
        ContinuationTerminalV1::Complete
    } else {
        ContinuationTerminalV1::Continue
    };
    let mut continuation = continuation_at_cursor_v1(
        prepared,
        0,
        layout_environment_fingerprint,
        flow_policy_fingerprint,
        initial_terminal,
    )?;

    let mut resolved = Vec::with_capacity(ordered_frames.len());

    for (frame_index, frame) in ordered_frames.iter().enumerate() {
        let result = resolve_one_frame_v1(
            prepared,
            &ordered_frames,
            frame_index,
            continuation,
            layout_environment_fingerprint,
            flow_policy_fingerprint,
            interval_policy,
        )?;
        continuation = result.output_boundary.continuation.clone();
        resolved.push(result);
    }

    Ok(finish_linked_flow_v1(
        prepared,
        layout_environment_fingerprint,
        flow_policy_fingerprint,
        resolved,
    ))
}

pub fn resolve_linked_story_incremental_v1(
    prepared: &PreparedStoryIndexV1,
    frames: &[LinkedFrameInputV1],
    old: &LinkedStoryFlowV1,
    earliest_affected_frame_id: &str,
    layout_environment_fingerprint: FingerprintV1,
    flow_policy_fingerprint: FingerprintV1,
    interval_policy: IntervalPolicyV1,
) -> Result<IncrementalLinkedStoryFlowV1, LinkedFlowErrorV1> {
    let ordered = ordered_frame_indices(frames)?;
    let ordered_frames = ordered
        .iter()
        .map(|index| &frames[*index])
        .collect::<Vec<_>>();

    if old.story_id != prepared.story_id {
        return Err(LinkedFlowErrorV1::StoryIdMismatch);
    }

    let requested_start = ordered_frames
        .iter()
        .position(|frame| frame.frame_id == earliest_affected_frame_id)
        .ok_or_else(|| LinkedFlowErrorV1::RequestedStartFrameMissing {
            frame_id: earliest_affected_frame_id.to_owned(),
        })?;

    let old_chain_matches = old.frames.len() == ordered_frames.len()
        && old
            .frames
            .iter()
            .zip(&ordered_frames)
            .all(|(old_frame, new_frame)| old_frame.frame_id == new_frame.frame_id);

    let environment_matches =
        old.layout_environment_fingerprint == layout_environment_fingerprint;
    let policy_matches = old.flow_policy_fingerprint == flow_policy_fingerprint;

    let mut actual_start = if old_chain_matches && environment_matches && policy_matches {
        requested_start
    } else {
        0
    };
    let mut mode = if actual_start == requested_start {
        IncrementalExecutionModeV1::Bounded
    } else {
        IncrementalExecutionModeV1::FullFallback
    };

    if old_chain_matches {
        for index in 0..actual_start {
            let current_dependency = frame_dependency_fingerprint_v1(ordered_frames[index]);
            if current_dependency != old.frames[index].frame_dependency_fingerprint {
                actual_start = index;
                mode = IncrementalExecutionModeV1::FullFallback;
                break;
            }
        }
    }

    if actual_start > 0 && old_chain_matches && environment_matches && policy_matches {
        let old_input = old.frames[actual_start].input_continuation.clone();
        let rebuilt_input = continuation_at_cursor_v1(
            prepared,
            old_input.next_scalar,
            layout_environment_fingerprint,
            flow_policy_fingerprint,
            old_input.terminal,
        )?;
        if rebuilt_input != old_input {
            actual_start = 0;
            mode = IncrementalExecutionModeV1::FullFallback;
        }
    }

    let actual_start_frame_id = ordered_frames[actual_start].frame_id.clone();

    let story_end = story_end_scalar(prepared);
    let mut continuation = if actual_start == 0 {
        continuation_at_cursor_v1(
            prepared,
            0,
            layout_environment_fingerprint,
            flow_policy_fingerprint,
            if story_end == 0 {
                ContinuationTerminalV1::Complete
            } else {
                ContinuationTerminalV1::Continue
            },
        )?
    } else {
        old.frames[actual_start - 1]
            .output_boundary
            .continuation
            .clone()
    };

    let mut assembled = if actual_start == 0 {
        Vec::new()
    } else {
        old.frames[..actual_start].to_vec()
    };
    let mut recomputed_frame_ids = Vec::new();
    let mut convergence_after_frame_id = None;
    let mut reused_suffix_from_frame_id = None;

    for index in actual_start..ordered_frames.len() {
        let resolved = resolve_one_frame_v1(
            prepared,
            &ordered_frames,
            index,
            continuation,
            layout_environment_fingerprint,
            flow_policy_fingerprint,
            interval_policy,
        )?;
        continuation = resolved.output_boundary.continuation.clone();
        recomputed_frame_ids.push(resolved.frame_id.clone());

        let can_converge = old_chain_matches
            && environment_matches
            && policy_matches
            && old
                .frames
                .get(index)
                .is_some_and(|old_frame| {
                    resolved.output_boundary == old_frame.output_boundary
                });

        assembled.push(resolved);

        if can_converge && index + 1 < ordered_frames.len() {
            convergence_after_frame_id = Some(ordered_frames[index].frame_id.clone());
            reused_suffix_from_frame_id = Some(ordered_frames[index + 1].frame_id.clone());
            assembled.extend_from_slice(&old.frames[index + 1..]);
            break;
        }
    }

    let flow = finish_linked_flow_v1(
        prepared,
        layout_environment_fingerprint,
        flow_policy_fingerprint,
        assembled,
    );

    Ok(IncrementalLinkedStoryFlowV1 {
        mode,
        requested_start_frame_id: earliest_affected_frame_id.to_owned(),
        actual_start_frame_id,
        recomputed_frame_ids,
        convergence_after_frame_id,
        reused_suffix_from_frame_id,
        flow,
    })
}

fn resolve_one_frame_v1(
    prepared: &PreparedStoryIndexV1,
    ordered_frames: &[&LinkedFrameInputV1],
    frame_index: usize,
    input_continuation: FrameContinuationStateV1,
    layout_environment_fingerprint: FingerprintV1,
    flow_policy_fingerprint: FingerprintV1,
    interval_policy: IntervalPolicyV1,
) -> Result<ResolvedLinkedFrameV1, LinkedFlowErrorV1> {
    let frame = ordered_frames[frame_index];
    let frame_dependency_fingerprint = frame_dependency_fingerprint_v1(frame);
    let story_end = story_end_scalar(prepared);
    let mut cursor = input_continuation.next_scalar;
    let mut lines = Vec::new();

    if input_continuation.terminal == ContinuationTerminalV1::Continue {
        for band in &frame.region.bands {
            if cursor >= story_end {
                break;
            }

            let Some(interval) = choose_interval(&band.intervals, interval_policy) else {
                continue;
            };

            let paragraph = paragraph_at_cursor(prepared, cursor)
                .ok_or(LinkedFlowErrorV1::CursorOutsidePreparedStory { scalar: cursor })?;

            if cursor == paragraph.content_scalar_end
                && paragraph.terminator_scalar == Some(cursor)
            {
                let consumed_scalar_end = cursor + 1;
                lines.push(LinkedFlowLineV1 {
                    row_index: band.row_index,
                    x0_emu: interval.x0_emu,
                    x1_emu: interval.x1_emu,
                    paragraph_id: paragraph.paragraph_id.clone(),
                    scalar_start: cursor,
                    scalar_end: cursor,
                    consumed_scalar_end,
                    measured_width_emu: 0,
                    mandatory_break: true,
                    content_fingerprint: empty_line_fingerprint_v1(paragraph),
                });
                cursor = consumed_scalar_end;
                continue;
            }

            let local_cursor = cursor - paragraph.scalar_base;
            let start_index = paragraph
                .prepared
                .metrics
                .iter()
                .position(|metric| metric.local_scalar_start == local_cursor)
                .ok_or_else(|| LinkedFlowErrorV1::MissingMetricAtCursor {
                    paragraph_id: paragraph.paragraph_id.clone(),
                    scalar: cursor,
                })?;

            let capacity = interval.x1_emu - interval.x0_emu;
            let mut width = 0_i64;
            let mut probe = start_index;
            let mut last_break = None;

            while probe < paragraph.prepared.metrics.len() {
                let metric = &paragraph.prepared.metrics[probe];
                let next = width.saturating_add(metric.advance_emu);
                if next > capacity {
                    break;
                }
                width = next;
                probe += 1;
                if metric.break_after {
                    last_break = Some(probe);
                }
            }

            let reaches_paragraph_end = probe == paragraph.prepared.metrics.len();
            let end_index = if reaches_paragraph_end {
                probe
            } else if let Some(last_break) = last_break {
                last_break
            } else {
                continue;
            };

            if end_index <= start_index {
                continue;
            }

            let selected = &paragraph.prepared.metrics[start_index..end_index];
            let scalar_end = paragraph.scalar_base
                + selected
                    .last()
                    .expect("non-empty selected metrics")
                    .local_scalar_end;
            let measured_width_emu = selected.iter().map(|metric| metric.advance_emu).sum();
            let mandatory_break =
                end_index == paragraph.prepared.metrics.len()
                    && paragraph.terminator_scalar.is_some();
            let consumed_scalar_end = if mandatory_break {
                paragraph
                    .terminator_scalar
                    .expect("mandatory break requires terminator")
                    + 1
            } else {
                scalar_end
            };

            lines.push(LinkedFlowLineV1 {
                row_index: band.row_index,
                x0_emu: interval.x0_emu,
                x1_emu: interval.x1_emu,
                paragraph_id: paragraph.paragraph_id.clone(),
                scalar_start: cursor,
                scalar_end,
                consumed_scalar_end,
                measured_width_emu,
                mandatory_break,
                content_fingerprint: selected_metrics_fingerprint_v1(selected),
            });
            cursor = consumed_scalar_end;
        }
    }

    let is_last = frame_index + 1 == ordered_frames.len();
    let terminal = if cursor >= story_end {
        ContinuationTerminalV1::Complete
    } else if is_last {
        ContinuationTerminalV1::Overset
    } else {
        ContinuationTerminalV1::Continue
    };

    let output_continuation = continuation_at_cursor_v1(
        prepared,
        cursor,
        layout_environment_fingerprint,
        flow_policy_fingerprint,
        terminal,
    )?;
    let downstream = downstream_dependency_state_v1(
        prepared,
        ordered_frames,
        frame_index,
        cursor,
    );

    let mut dependency_payload = Vec::new();
    dependency_payload.extend_from_slice(&frame_dependency_fingerprint);
    append_continuation(&mut dependency_payload, &input_continuation);
    dependency_payload.extend_from_slice(&prepared_suffix_fingerprint_v1(prepared, cursor));
    dependency_payload.extend_from_slice(&layout_environment_fingerprint);
    dependency_payload.extend_from_slice(&flow_policy_fingerprint);

    let dependency_fingerprint =
        fingerprint_v1("linked-frame-flow-input-v1", &[&dependency_payload]);

    let output_boundary = FrameBoundaryStateV1 {
        continuation: output_continuation,
        downstream,
    };
    let output_fingerprint =
        frame_output_fingerprint_v1(&lines, &output_boundary);

    Ok(ResolvedLinkedFrameV1 {
        frame_id: frame.frame_id.clone(),
        frame_dependency_fingerprint,
        dependency_fingerprint,
        output_fingerprint,
        input_continuation,
        output_boundary,
        lines,
    })
}

fn continuation_at_cursor_v1(
    prepared: &PreparedStoryIndexV1,
    next_scalar: u32,
    layout_environment_fingerprint: FingerprintV1,
    flow_policy_fingerprint: FingerprintV1,
    terminal: ContinuationTerminalV1,
) -> Result<FrameContinuationStateV1, LinkedFlowErrorV1> {
    let story_end = story_end_scalar(prepared);

    if next_scalar > story_end {
        return Err(LinkedFlowErrorV1::CursorOutsidePreparedStory {
            scalar: next_scalar,
        });
    }

    let (paragraph_id, prepared_boundary_fingerprint) =
        if terminal == ContinuationTerminalV1::Complete || next_scalar == story_end {
            (None, None)
        } else {
            let paragraph = paragraph_at_cursor(prepared, next_scalar)
                .ok_or(LinkedFlowErrorV1::CursorOutsidePreparedStory {
                    scalar: next_scalar,
                })?;
            let local_offset = next_scalar - paragraph.scalar_base;
            let fingerprint = fingerprint_v1(
                "prepared-boundary-v1",
                &[
                    &paragraph.prepared.output_fingerprint,
                    &local_offset.to_be_bytes(),
                ],
            );
            (Some(paragraph.paragraph_id.clone()), Some(fingerprint))
        };

    Ok(FrameContinuationStateV1 {
        schema_version: FRAME_CONTINUATION_SCHEMA_V1.to_owned(),
        story_id: prepared.story_id.clone(),
        next_scalar,
        paragraph_id,
        prepared_boundary_fingerprint,
        layout_environment_fingerprint,
        flow_policy_fingerprint,
        terminal,
    })
}

fn downstream_dependency_state_v1(
    prepared: &PreparedStoryIndexV1,
    ordered_frames: &[&LinkedFrameInputV1],
    frame_index: usize,
    cursor: u32,
) -> DownstreamDependencyStateV1 {
    let mut frame_payload = Vec::new();
    for frame in ordered_frames.iter().skip(frame_index + 1) {
        frame_payload.extend_from_slice(&frame_dependency_fingerprint_v1(frame));
    }

    DownstreamDependencyStateV1 {
        prepared_suffix_fingerprint: prepared_suffix_fingerprint_v1(prepared, cursor),
        frame_suffix_fingerprint: fingerprint_v1("frame-suffix-v1", &[&frame_payload]),
    }
}

fn prepared_suffix_fingerprint_v1(
    prepared: &PreparedStoryIndexV1,
    cursor: u32,
) -> FingerprintV1 {
    let mut payload = Vec::new();
    push_bytes(&mut payload, prepared.story_id.as_bytes());
    push_u32(&mut payload, cursor);

    for paragraph in &prepared.paragraphs {
        if paragraph.scalar_end <= cursor {
            continue;
        }

        push_bytes(&mut payload, paragraph.paragraph_id.as_bytes());
        payload.extend_from_slice(&paragraph.position_fingerprint);
        payload.extend_from_slice(&paragraph.prepared.output_fingerprint);

        let local_offset = if cursor > paragraph.scalar_base {
            cursor - paragraph.scalar_base
        } else {
            0
        };
        push_u32(&mut payload, local_offset);
    }

    fingerprint_v1("prepared-suffix-v1", &[&payload])
}

fn frame_dependency_fingerprint_v1(frame: &LinkedFrameInputV1) -> FingerprintV1 {
    let mut payload = Vec::new();
    push_bytes(&mut payload, frame.frame_id.as_bytes());
    push_optional_string(&mut payload, frame.previous_frame_id.as_deref());
    push_optional_string(&mut payload, frame.next_frame_id.as_deref());
    payload.extend_from_slice(&frame.region.output_fingerprint);
    fingerprint_v1("linked-frame-dependency-v1", &[&payload])
}

fn frame_output_fingerprint_v1(
    lines: &[LinkedFlowLineV1],
    boundary: &FrameBoundaryStateV1,
) -> FingerprintV1 {
    let mut payload = Vec::new();

    for line in lines {
        push_u32(&mut payload, line.row_index);
        payload.extend_from_slice(&line.x0_emu.to_be_bytes());
        payload.extend_from_slice(&line.x1_emu.to_be_bytes());
        push_bytes(&mut payload, line.paragraph_id.as_bytes());
        push_u32(&mut payload, line.scalar_start);
        push_u32(&mut payload, line.scalar_end);
        push_u32(&mut payload, line.consumed_scalar_end);
        payload.extend_from_slice(&line.measured_width_emu.to_be_bytes());
        payload.push(u8::from(line.mandatory_break));
        payload.extend_from_slice(&line.content_fingerprint);
    }

    append_continuation(&mut payload, &boundary.continuation);
    payload.extend_from_slice(&boundary.downstream.prepared_suffix_fingerprint);
    payload.extend_from_slice(&boundary.downstream.frame_suffix_fingerprint);

    fingerprint_v1("linked-frame-output-v1", &[&payload])
}

fn finish_linked_flow_v1(
    prepared: &PreparedStoryIndexV1,
    layout_environment_fingerprint: FingerprintV1,
    flow_policy_fingerprint: FingerprintV1,
    frames: Vec<ResolvedLinkedFrameV1>,
) -> LinkedStoryFlowV1 {
    let mut payload = Vec::new();
    for frame in &frames {
        push_bytes(&mut payload, frame.frame_id.as_bytes());
        payload.extend_from_slice(&frame.output_fingerprint);
    }

    LinkedStoryFlowV1 {
        story_id: prepared.story_id.clone(),
        layout_environment_fingerprint,
        flow_policy_fingerprint,
        output_fingerprint: fingerprint_v1(LINKED_STORY_FLOW_STAGE_V1, &[&payload]),
        frames,
    }
}

fn ordered_frame_indices(
    frames: &[LinkedFrameInputV1],
) -> Result<Vec<usize>, LinkedFlowErrorV1> {
    if frames.is_empty() {
        return Err(LinkedFlowErrorV1::MissingFrames);
    }

    let mut by_id = BTreeMap::new();
    for (index, frame) in frames.iter().enumerate() {
        if frame.region.frame_id != frame.frame_id {
            return Err(LinkedFlowErrorV1::RegionFrameMismatch {
                frame_id: frame.frame_id.clone(),
                region_frame_id: frame.region.frame_id.clone(),
            });
        }
        if by_id.insert(frame.frame_id.clone(), index).is_some() {
            return Err(LinkedFlowErrorV1::DuplicateFrameId {
                frame_id: frame.frame_id.clone(),
            });
        }
    }

    let heads = frames
        .iter()
        .enumerate()
        .filter(|(_, frame)| frame.previous_frame_id.is_none())
        .map(|(index, _)| index)
        .collect::<Vec<_>>();

    if heads.len() != 1 {
        return Err(LinkedFlowErrorV1::InvalidHeadCount { count: heads.len() });
    }

    let mut ordered = Vec::with_capacity(frames.len());
    let mut visited = BTreeSet::new();
    let mut current_index = heads[0];

    loop {
        let current = &frames[current_index];
        if !visited.insert(current.frame_id.clone()) {
            return Err(LinkedFlowErrorV1::CycleOrDisconnectedChain);
        }
        ordered.push(current_index);

        let Some(next_id) = current.next_frame_id.as_deref() else {
            break;
        };
        let next_index = *by_id
            .get(next_id)
            .ok_or_else(|| LinkedFlowErrorV1::MissingLinkedFrame {
                frame_id: next_id.to_owned(),
            })?;
        let next = &frames[next_index];
        if next.previous_frame_id.as_deref() != Some(current.frame_id.as_str()) {
            return Err(LinkedFlowErrorV1::BrokenPreviousLink {
                frame_id: next.frame_id.clone(),
                expected_previous: current.frame_id.clone(),
                actual_previous: next.previous_frame_id.clone(),
            });
        }
        current_index = next_index;
    }

    if visited.len() != frames.len() {
        return Err(LinkedFlowErrorV1::CycleOrDisconnectedChain);
    }

    Ok(ordered)
}

fn paragraph_at_cursor(
    prepared: &PreparedStoryIndexV1,
    cursor: u32,
) -> Option<&PreparedParagraphBindingV1> {
    prepared
        .paragraphs
        .iter()
        .find(|paragraph| cursor >= paragraph.scalar_base && cursor < paragraph.scalar_end)
}

fn story_end_scalar(prepared: &PreparedStoryIndexV1) -> u32 {
    prepared
        .paragraphs
        .last()
        .map_or(0, |paragraph| paragraph.scalar_end)
}

fn choose_interval(
    intervals: &[crate::runtime::IntervalV1],
    policy: IntervalPolicyV1,
) -> Option<crate::runtime::IntervalV1> {
    match policy {
        IntervalPolicyV1::LargestOnly => intervals.iter().copied().max_by(|left, right| {
            let left_width = left.x1_emu - left.x0_emu;
            let right_width = right.x1_emu - right.x0_emu;
            left_width
                .cmp(&right_width)
                .then_with(|| right.x0_emu.cmp(&left.x0_emu))
        }),
    }
}

fn selected_metrics_fingerprint_v1(
    metrics: &[crate::prepared_paragraph::PreparedLocalScalarMetricV1],
) -> FingerprintV1 {
    let mut payload = Vec::new();
    for metric in metrics {
        push_u32(&mut payload, metric.local_scalar_start);
        push_u32(&mut payload, metric.local_scalar_end);
        payload.extend_from_slice(&metric.advance_emu.to_be_bytes());
        payload.push(u8::from(metric.break_after));
        payload.extend_from_slice(&metric.semantic_fingerprint);
    }
    fingerprint_v1("linked-line-content-v1", &[&payload])
}

fn empty_line_fingerprint_v1(paragraph: &PreparedParagraphBindingV1) -> FingerprintV1 {
    fingerprint_v1(
        "linked-empty-line-v1",
        &[paragraph.paragraph_id.as_bytes(), &paragraph.prepared.output_fingerprint],
    )
}

fn append_continuation(payload: &mut Vec<u8>, continuation: &FrameContinuationStateV1) {
    push_bytes(payload, continuation.schema_version.as_bytes());
    push_bytes(payload, continuation.story_id.as_bytes());
    push_u32(payload, continuation.next_scalar);
    push_optional_string(payload, continuation.paragraph_id.as_deref());
    match continuation.prepared_boundary_fingerprint {
        Some(fingerprint) => {
            payload.push(1);
            payload.extend_from_slice(&fingerprint);
        }
        None => payload.push(0),
    }
    payload.extend_from_slice(&continuation.layout_environment_fingerprint);
    payload.extend_from_slice(&continuation.flow_policy_fingerprint);
    payload.push(match continuation.terminal {
        ContinuationTerminalV1::Continue => 1,
        ContinuationTerminalV1::Complete => 2,
        ContinuationTerminalV1::Overset => 3,
    });
}

fn push_optional_string(payload: &mut Vec<u8>, value: Option<&str>) {
    match value {
        Some(value) => {
            payload.push(1);
            push_bytes(payload, value.as_bytes());
        }
        None => payload.push(0),
    }
}

fn push_bytes(payload: &mut Vec<u8>, bytes: &[u8]) {
    let len = u64::try_from(bytes.len()).expect("bounded linked-flow input");
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

    use crate::prepared_paragraph::prepare_projected_story_v1;
    use crate::runtime::{
        FrameGeometryV1, ResolvedScalarMetricV1, resolve_line_regions_v1,
    };

    use super::*;

    fn fp(domain: &str, value: &str) -> FingerprintV1 {
        fingerprint_v1(domain, &[value.as_bytes()])
    }

    fn prepared_single_paragraph(
        scalar_count: u32,
        environment: FingerprintV1,
        policy: FingerprintV1,
    ) -> PreparedStoryIndexV1 {
        let text = (0..scalar_count)
            .map(|index| char::from(b'a' + u8::try_from(index % 26).expect("bounded alphabet")))
            .collect::<String>();

        let projected = project_story_text_v1(StoryProjectionInputV1 {
            story_id: "s1".into(),
            text,
            paragraph_ids: vec!["p1".into()],
            shaping_runs: vec![ResolvedShapingRunInputV1 {
                scalar_start: 0,
                scalar_end: scalar_count,
                resolved_shaping_fingerprint: "style-a".into(),
            }],
        });

        let metrics = (0..scalar_count)
            .map(|scalar| ResolvedScalarMetricV1 {
                scalar_start: scalar,
                scalar_end: scalar + 1,
                advance_emu: 20,
                break_after: true,
                semantic_fingerprint: fp("semantic-v1", &format!("s{scalar}")),
            })
            .collect::<Vec<_>>();

        prepare_projected_story_v1(&projected, &metrics, environment, policy)
            .expect("prepare fixture")
    }

    fn frame_region(frame_id: &str, width_emu: i64) -> LineRegionV1 {
        resolve_line_regions_v1(
            &FrameGeometryV1 {
                frame_id: frame_id.into(),
                page_id: format!("page-{frame_id}"),
                width_emu,
                height_emu: 20,
                line_height_emu: 20,
            },
            &[],
        )
        .expect("region")
    }

    fn chain(widths: &[i64]) -> Vec<LinkedFrameInputV1> {
        widths
            .iter()
            .enumerate()
            .map(|(index, width)| {
                let frame_id = format!("f{}", index + 1);
                LinkedFrameInputV1 {
                    frame_id: frame_id.clone(),
                    previous_frame_id: (index > 0).then(|| format!("f{index}")),
                    next_frame_id: (index + 1 < widths.len())
                        .then(|| format!("f{}", index + 2)),
                    region: frame_region(&frame_id, *width),
                }
            })
            .collect()
    }

    fn env() -> FingerprintV1 {
        fp("env-v1", "font-a")
    }

    fn policy() -> FingerprintV1 {
        fp("flow-policy-v1", "largest")
    }

    #[test]
    fn explicit_chain_order_is_derived_from_links_not_input_enumeration() {
        let prepared = prepared_single_paragraph(6, env(), policy());
        let mut frames = chain(&[40, 40, 40]);
        frames.reverse();

        let flow = resolve_linked_story_full_v1(
            &prepared,
            &frames,
            env(),
            policy(),
            IntervalPolicyV1::LargestOnly,
        )
        .expect("flow");

        assert_eq!(
            flow.frames
                .iter()
                .map(|frame| frame.frame_id.as_str())
                .collect::<Vec<_>>(),
            vec!["f1", "f2", "f3"]
        );
    }

    #[test]
    fn scalar_difference_never_converges() {
        let prepared = prepared_single_paragraph(20, env(), policy());
        let baseline_frames = chain(&[40; 10]);
        let baseline = resolve_linked_story_full_v1(
            &prepared,
            &baseline_frames,
            env(),
            policy(),
            IntervalPolicyV1::LargestOnly,
        )
        .expect("baseline");

        let changed_frames = chain(&[20, 40, 40, 40, 40, 40, 40, 40, 40, 40]);
        let incremental = resolve_linked_story_incremental_v1(
            &prepared,
            &changed_frames,
            &baseline,
            "f1",
            env(),
            policy(),
            IntervalPolicyV1::LargestOnly,
        )
        .expect("incremental");
        let clean = resolve_linked_story_full_v1(
            &prepared,
            &changed_frames,
            env(),
            policy(),
            IntervalPolicyV1::LargestOnly,
        )
        .expect("clean");

        assert_eq!(incremental.flow, clean);
        assert_eq!(incremental.recomputed_frame_ids.len(), 10);
        assert_eq!(incremental.convergence_after_frame_id, None);
        assert_eq!(
            incremental
                .flow
                .frames
                .last()
                .expect("last frame")
                .output_boundary
                .continuation
                .terminal,
            ContinuationTerminalV1::Overset
        );
    }

    #[test]
    fn matching_scalar_is_not_enough_when_environment_differs() {
        let prepared = prepared_single_paragraph(20, env(), policy());
        let frames = chain(&[40; 10]);
        let baseline = resolve_linked_story_full_v1(
            &prepared,
            &frames,
            env(),
            policy(),
            IntervalPolicyV1::LargestOnly,
        )
        .expect("baseline");

        let other_env = fp("env-v1", "font-b");
        let incremental = resolve_linked_story_incremental_v1(
            &prepared,
            &frames,
            &baseline,
            "f1",
            other_env,
            policy(),
            IntervalPolicyV1::LargestOnly,
        )
        .expect("incremental");
        let clean = resolve_linked_story_full_v1(
            &prepared,
            &frames,
            other_env,
            policy(),
            IntervalPolicyV1::LargestOnly,
        )
        .expect("clean");

        assert_eq!(incremental.flow, clean);
        assert_eq!(
            incremental.mode,
            IncrementalExecutionModeV1::FullFallback
        );
        assert_eq!(incremental.recomputed_frame_ids.len(), 10);
        assert_eq!(incremental.convergence_after_frame_id, None);
    }

    #[test]
    fn downstream_frame_dependency_prevents_early_false_convergence() {
        let prepared = prepared_single_paragraph(20, env(), policy());
        let baseline_frames = chain(&[40; 10]);
        let baseline = resolve_linked_story_full_v1(
            &prepared,
            &baseline_frames,
            env(),
            policy(),
            IntervalPolicyV1::LargestOnly,
        )
        .expect("baseline");

        let changed_frames = chain(&[40, 40, 60, 20, 40, 40, 40, 40, 40, 40]);
        let incremental = resolve_linked_story_incremental_v1(
            &prepared,
            &changed_frames,
            &baseline,
            "f1",
            env(),
            policy(),
            IntervalPolicyV1::LargestOnly,
        )
        .expect("incremental");
        let clean = resolve_linked_story_full_v1(
            &prepared,
            &changed_frames,
            env(),
            policy(),
            IntervalPolicyV1::LargestOnly,
        )
        .expect("clean");

        assert_eq!(incremental.flow, clean);
        assert_eq!(
            incremental.recomputed_frame_ids,
            vec!["f1", "f2", "f3", "f4"]
        );
        assert_eq!(
            incremental.convergence_after_frame_id.as_deref(),
            Some("f4")
        );
        assert_eq!(
            incremental.reused_suffix_from_frame_id.as_deref(),
            Some("f5")
        );
    }

    #[test]
    fn ten_frame_story_converges_after_two_frames_and_reuses_eight() {
        let prepared = prepared_single_paragraph(20, env(), policy());
        let baseline_frames = chain(&[40; 10]);
        let baseline = resolve_linked_story_full_v1(
            &prepared,
            &baseline_frames,
            env(),
            policy(),
            IntervalPolicyV1::LargestOnly,
        )
        .expect("baseline");

        let changed_frames = chain(&[20, 60, 40, 40, 40, 40, 40, 40, 40, 40]);
        let incremental = resolve_linked_story_incremental_v1(
            &prepared,
            &changed_frames,
            &baseline,
            "f1",
            env(),
            policy(),
            IntervalPolicyV1::LargestOnly,
        )
        .expect("incremental");
        let clean = resolve_linked_story_full_v1(
            &prepared,
            &changed_frames,
            env(),
            policy(),
            IntervalPolicyV1::LargestOnly,
        )
        .expect("clean");

        assert_eq!(incremental.flow, clean);
        assert_eq!(incremental.recomputed_frame_ids, vec!["f1", "f2"]);
        assert_eq!(
            incremental.convergence_after_frame_id.as_deref(),
            Some("f2")
        );
        assert_eq!(
            incremental.reused_suffix_from_frame_id.as_deref(),
            Some("f3")
        );
        assert_eq!(
            incremental
                .flow
                .frames
                .iter()
                .skip(2)
                .map(|frame| frame.output_fingerprint)
                .collect::<Vec<_>>(),
            baseline
                .frames
                .iter()
                .skip(2)
                .map(|frame| frame.output_fingerprint)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn broken_chain_fails_closed_instead_of_using_ordinal_like_order() {
        let prepared = prepared_single_paragraph(4, env(), policy());
        let mut frames = chain(&[40, 40]);
        frames[1].previous_frame_id = None;

        let error = resolve_linked_story_full_v1(
            &prepared,
            &frames,
            env(),
            policy(),
            IntervalPolicyV1::LargestOnly,
        )
        .expect_err("two heads must fail");

        assert_eq!(error, LinkedFlowErrorV1::InvalidHeadCount { count: 2 });
    }

    #[test]
    fn mandatory_paragraph_boundary_is_consumed_without_fake_width() {
        let text = "A\rB";
        let projected = project_story_text_v1(StoryProjectionInputV1 {
            story_id: "s1".into(),
            text: text.into(),
            paragraph_ids: vec!["p1".into(), "p2".into()],
            shaping_runs: vec![ResolvedShapingRunInputV1 {
                scalar_start: 0,
                scalar_end: 3,
                resolved_shaping_fingerprint: "style-a".into(),
            }],
        });
        let metrics = vec![
            ResolvedScalarMetricV1 {
                scalar_start: 0,
                scalar_end: 1,
                advance_emu: 20,
                break_after: false,
                semantic_fingerprint: fp("semantic-v1", "A"),
            },
            ResolvedScalarMetricV1 {
                scalar_start: 2,
                scalar_end: 3,
                advance_emu: 20,
                break_after: false,
                semantic_fingerprint: fp("semantic-v1", "B"),
            },
        ];
        let prepared =
            prepare_projected_story_v1(&projected, &metrics, env(), policy()).expect("prepare");
        let frames = chain(&[100, 100]);

        let flow = resolve_linked_story_full_v1(
            &prepared,
            &frames,
            env(),
            policy(),
            IntervalPolicyV1::LargestOnly,
        )
        .expect("flow");

        assert_eq!(flow.frames[0].lines.len(), 1);
        assert_eq!(flow.frames[0].lines[0].scalar_start, 0);
        assert_eq!(flow.frames[0].lines[0].scalar_end, 1);
        assert_eq!(flow.frames[0].lines[0].consumed_scalar_end, 2);
        assert!(flow.frames[0].lines[0].mandatory_break);
        assert_eq!(flow.frames[0].lines[0].measured_width_emu, 20);

        assert_eq!(flow.frames[1].lines[0].scalar_start, 2);
        assert_eq!(flow.frames[1].lines[0].scalar_end, 3);
    }
}
