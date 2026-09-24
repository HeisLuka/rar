use serde::{Deserialize, Serialize};

/// Семантическое представление одного text-frame внутри Publisher Story.
///
/// Типы идентификаторов намеренно являются параметрами: source-specific
/// пространства Contents/Quill/Escher нельзя склеивать здесь в один native ID.
///
/// Для подтверждённой linked-story модели:
/// - ordinal 0 является обычным эффективным значением, даже если raw поле
///   отсутствует;
/// - принадлежность к одной Story не означает наличие explicit flow chain;
/// - previous/next — отдельные отношения и не выводятся сортировкой frame ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryFrame<StoryId, FrameId> {
    pub story_id: StoryId,
    pub frame_id: FrameId,
    pub ordinal: u32,
    pub previous: Option<FrameId>,
    pub next: Option<FrameId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowDirection {
    Previous,
    Next,
}

/// Ошибка семантического графа linked text frames.
///
/// Валидатор кодирует только invariants, подтверждённые для Publisher story
/// model. Он не требует наличия links у нескольких frames одной Story.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoryFlowError<StoryId, FrameId> {
    DuplicateFrameId {
        frame_id: FrameId,
    },
    DuplicateOrdinal {
        story_id: StoryId,
        ordinal: u32,
    },
    MissingTarget {
        frame_id: FrameId,
        direction: FlowDirection,
        target: FrameId,
    },
    CrossStoryLink {
        frame_id: FrameId,
        direction: FlowDirection,
        target: FrameId,
    },
    NonReciprocalLink {
        frame_id: FrameId,
        direction: FlowDirection,
        target: FrameId,
    },
    NonAdjacentOrdinal {
        frame_id: FrameId,
        direction: FlowDirection,
        target: FrameId,
        frame_ordinal: u32,
        target_ordinal: u32,
    },
}

/// Проверяет подтверждённые отношения Publisher Story ↔ TextFrame.
///
/// Важно: shared Story без explicit previous/next links является допустимым
/// состоянием и не превращается валидатором в flow chain.
pub fn validate_story_frames<StoryId, FrameId>(
    frames: &[StoryFrame<StoryId, FrameId>],
) -> Vec<StoryFlowError<StoryId, FrameId>>
where
    StoryId: Clone + Eq,
    FrameId: Clone + Eq,
{
    let mut errors = Vec::new();

    for (index, frame) in frames.iter().enumerate() {
        if frames[..index]
            .iter()
            .any(|other| other.frame_id == frame.frame_id)
        {
            errors.push(StoryFlowError::DuplicateFrameId {
                frame_id: frame.frame_id.clone(),
            });
        }

        if frames[..index]
            .iter()
            .any(|other| other.story_id == frame.story_id && other.ordinal == frame.ordinal)
        {
            errors.push(StoryFlowError::DuplicateOrdinal {
                story_id: frame.story_id.clone(),
                ordinal: frame.ordinal,
            });
        }

        validate_link(
            frames,
            frame,
            FlowDirection::Previous,
            frame.previous.as_ref(),
            &mut errors,
        );
        validate_link(
            frames,
            frame,
            FlowDirection::Next,
            frame.next.as_ref(),
            &mut errors,
        );
    }

    errors
}

fn validate_link<StoryId, FrameId>(
    frames: &[StoryFrame<StoryId, FrameId>],
    frame: &StoryFrame<StoryId, FrameId>,
    direction: FlowDirection,
    target_id: Option<&FrameId>,
    errors: &mut Vec<StoryFlowError<StoryId, FrameId>>,
) where
    StoryId: Clone + Eq,
    FrameId: Clone + Eq,
{
    let Some(target_id) = target_id else {
        return;
    };

    let Some(target) = frames
        .iter()
        .find(|candidate| &candidate.frame_id == target_id)
    else {
        errors.push(StoryFlowError::MissingTarget {
            frame_id: frame.frame_id.clone(),
            direction,
            target: target_id.clone(),
        });
        return;
    };

    if target.story_id != frame.story_id {
        errors.push(StoryFlowError::CrossStoryLink {
            frame_id: frame.frame_id.clone(),
            direction,
            target: target_id.clone(),
        });
        return;
    }

    let reciprocal = match direction {
        FlowDirection::Previous => target.next.as_ref(),
        FlowDirection::Next => target.previous.as_ref(),
    };
    if reciprocal != Some(&frame.frame_id) {
        errors.push(StoryFlowError::NonReciprocalLink {
            frame_id: frame.frame_id.clone(),
            direction,
            target: target_id.clone(),
        });
    }

    let adjacent = match direction {
        FlowDirection::Previous => frame.ordinal.checked_sub(1) == Some(target.ordinal),
        FlowDirection::Next => frame.ordinal.checked_add(1) == Some(target.ordinal),
    };
    if !adjacent {
        errors.push(StoryFlowError::NonAdjacentOrdinal {
            frame_id: frame.frame_id.clone(),
            direction,
            target: target_id.clone(),
            frame_ordinal: frame.ordinal,
            target_ordinal: target.ordinal,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        story_id: u32,
        frame_id: u32,
        ordinal: u32,
        previous: Option<u32>,
        next: Option<u32>,
    ) -> StoryFrame<u32, u32> {
        StoryFrame {
            story_id,
            frame_id,
            ordinal,
            previous,
            next,
        }
    }

    #[test]
    fn shared_story_without_explicit_links_is_valid() {
        let frames = [frame(7, 311, 0, None, None), frame(7, 312, 1, None, None)];

        assert!(validate_story_frames(&frames).is_empty());
    }

    #[test]
    fn non_monotonic_frame_ids_are_valid_when_flow_relations_agree() {
        let frames = [
            frame(22, 330, 0, None, Some(329)),
            frame(22, 329, 1, Some(330), Some(331)),
            frame(22, 331, 2, Some(329), None),
        ];

        assert!(validate_story_frames(&frames).is_empty());
    }

    #[test]
    fn cross_story_link_is_rejected() {
        let frames = [
            frame(8, 298, 0, None, Some(299)),
            frame(9, 299, 1, Some(298), None),
        ];

        let errors = validate_story_frames(&frames);
        assert!(errors.iter().any(|error| {
            matches!(
                error,
                StoryFlowError::CrossStoryLink {
                    frame_id: 298,
                    direction: FlowDirection::Next,
                    target: 299,
                }
            )
        }));
    }

    #[test]
    fn ordinal_does_not_replace_explicit_link_validation() {
        let frames = [
            frame(8, 298, 0, None, Some(300)),
            frame(8, 299, 1, None, None),
            frame(8, 300, 2, Some(298), None),
        ];

        let errors = validate_story_frames(&frames);
        assert!(errors.iter().any(|error| {
            matches!(
                error,
                StoryFlowError::NonAdjacentOrdinal {
                    frame_id: 298,
                    direction: FlowDirection::Next,
                    target: 300,
                    ..
                }
            )
        }));
    }
}
