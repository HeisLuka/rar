use crate::domain::{StoryEditDomainV1, edit_domain_id_v1, validate_ordinary_story_range_v1};
use chaptera_text_caret_map_adapter::{ResolvedTextCaretMapV1, resolve_story_position_v1};
use chaptera_text_interaction_adapter::TextSelectionStateV1;
use serde::{Deserialize, Serialize};
use std::fmt;
use unicode_segmentation::UnicodeSegmentation;

pub const UNICODE_GRAPHEME_VERSION: &str = "15.0.0";
pub const KEYBOARD_POLICY_VERSION_V1: &str = "chaptera.text-keyboard-policy.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyboardCommandV1 {
    MovePrevious,
    MoveNext,
    ExtendPrevious,
    ExtendNext,
    DeleteBackward,
    DeleteForward,
}

impl KeyboardCommandV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MovePrevious => "move_previous",
            Self::MoveNext => "move_next",
            Self::ExtendPrevious => "extend_previous",
            Self::ExtendNext => "extend_next",
            Self::DeleteBackward => "delete_backward",
            Self::DeleteForward => "delete_forward",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplaceStoryRangeIntentV1 {
    pub protocol_version: String,
    pub story_id: String,
    pub start_scalar: u32,
    pub end_scalar: u32,
    pub replacement_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextKeyboardDecisionV1 {
    pub protocol_version: String,
    pub command: KeyboardCommandV1,
    pub action: String,
    pub selection: Option<TextSelectionStateV1>,
    pub delete_intent: Option<ReplaceStoryRangeIntentV1>,
    pub grapheme_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextKeyboardPolicyError {
    pub code: String,
    pub message: String,
}

impl TextKeyboardPolicyError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for TextKeyboardPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for TextKeyboardPolicyError {}

pub fn grapheme_boundaries_v1(text: &str) -> Result<Vec<u32>, TextKeyboardPolicyError> {
    let mut boundaries = Vec::new();
    boundaries.push(0);
    let mut scalar_cursor = 0_u32;
    for grapheme in UnicodeSegmentation::graphemes(text, true) {
        let count = u32::try_from(grapheme.chars().count()).map_err(|_| {
            TextKeyboardPolicyError::new("invalid_story", "grapheme scalar length overflows u32")
        })?;
        scalar_cursor = scalar_cursor.checked_add(count).ok_or_else(|| {
            TextKeyboardPolicyError::new("invalid_story", "Story scalar length overflows u32")
        })?;
        boundaries.push(scalar_cursor);
    }
    Ok(boundaries)
}

fn require_known_domain(domain: &StoryEditDomainV1) -> Result<(u32, u32), TextKeyboardPolicyError> {
    if domain.status != "known" {
        return Err(TextKeyboardPolicyError::new(
            "edit_domain_unknown",
            "ordinary Story keyboard domain is unknown",
        ));
    }
    let floor = domain.caret_start_boundary.ok_or_else(|| {
        TextKeyboardPolicyError::new("edit_domain_unknown", "caret start is unavailable")
    })?;
    let ceiling = domain.caret_end_boundary.ok_or_else(|| {
        TextKeyboardPolicyError::new("edit_domain_unknown", "caret end is unavailable")
    })?;
    Ok((floor, ceiling))
}

fn require_admitted_boundary(
    domain: &StoryEditDomainV1,
    scalar: u32,
    label: &str,
) -> Result<(), TextKeyboardPolicyError> {
    let (floor, ceiling) = require_known_domain(domain)?;
    if scalar < floor || scalar > ceiling {
        return Err(TextKeyboardPolicyError::new(
            "selection_reconcile_required",
            format!("{label} lies outside the current ordinary Story edit domain"),
        ));
    }
    Ok(())
}

fn validate_selection_state_v1(
    selection: &TextSelectionStateV1,
    domain: &StoryEditDomainV1,
    expected_revision_id: &str,
) -> Result<(), TextKeyboardPolicyError> {
    if selection.protocol_version != "chaptera.text-selection-state.v1" {
        return Err(TextKeyboardPolicyError::new(
            "invalid_selection_state",
            "selection protocol_version mismatch",
        ));
    }
    if selection.story_id != domain.story_id {
        return Err(TextKeyboardPolicyError::new(
            "selection_story_mismatch",
            "selection and edit domain target different Stories",
        ));
    }
    if selection.edit_domain_id != edit_domain_id_v1(domain) {
        return Err(TextKeyboardPolicyError::new(
            "selection_reconcile_required",
            "selection was projected against a different Story edit domain",
        ));
    }
    if selection.revision_id != expected_revision_id {
        return Err(TextKeyboardPolicyError::new(
            "stale_selection_revision",
            "selection belongs to a different canonical revision",
        ));
    }
    require_admitted_boundary(domain, selection.anchor_scalar, "anchor_scalar")?;
    require_admitted_boundary(domain, selection.focus_scalar, "focus_scalar")?;

    match selection.projection_state.as_str() {
        "layout_pending" => {
            if selection.layout_revision_id.is_some()
                || selection.anchor_visual_stop_id.is_some()
                || selection.focus_visual_stop_id.is_some()
            {
                return Err(TextKeyboardPolicyError::new(
                    "invalid_selection_state",
                    "layout-pending selection carries stale geometry",
                ));
            }
        }
        "projected" => {
            if selection
                .layout_revision_id
                .as_deref()
                .is_none_or(str::is_empty)
                || selection
                    .anchor_visual_stop_id
                    .as_deref()
                    .is_none_or(str::is_empty)
                || selection
                    .focus_visual_stop_id
                    .as_deref()
                    .is_none_or(str::is_empty)
            {
                return Err(TextKeyboardPolicyError::new(
                    "invalid_selection_state",
                    "projected selection requires layout revision and endpoint stops",
                ));
            }
            if selection.anchor_scalar == selection.focus_scalar
                && selection.anchor_visual_stop_id != selection.focus_visual_stop_id
            {
                return Err(TextKeyboardPolicyError::new(
                    "invalid_selection_state",
                    "collapsed selection must use one physical caret stop",
                ));
            }
        }
        _ => {
            return Err(TextKeyboardPolicyError::new(
                "invalid_selection_state",
                "unsupported projection_state",
            ));
        }
    }
    Ok(())
}

fn require_physical_boundary(
    caret_map: &ResolvedTextCaretMapV1,
    scalar_boundary: u32,
    stop_id: Option<&str>,
) -> Result<(), TextKeyboardPolicyError> {
    resolve_story_position_v1(
        caret_map,
        scalar_boundary,
        stop_id,
        Some(&caret_map.layout_revision_id),
    )
    .map(|_| ())
    .map_err(|error| {
        let code = if matches!(
            error.code,
            "internal_cluster_unsupported"
                | "unplaced_story_position"
                | "caret_affinity_required"
                | "invalid_caret_affinity"
                | "stale_layout_map"
        ) {
            "caret_geometry_unsupported".to_owned()
        } else {
            error.code.to_owned()
        };
        TextKeyboardPolicyError::new(code, error.to_string())
    })
}

fn previous_boundary(boundaries: &[u32], scalar: u32, floor: u32) -> u32 {
    boundaries
        .iter()
        .copied()
        .filter(|item| floor <= *item && *item < scalar)
        .max()
        .unwrap_or(scalar)
}

fn next_boundary(boundaries: &[u32], scalar: u32, ceiling: u32) -> u32 {
    boundaries
        .iter()
        .copied()
        .filter(|item| scalar < *item && *item <= ceiling)
        .min()
        .unwrap_or(scalar)
}

fn layout_pending_selection(
    domain: &StoryEditDomainV1,
    revision_id: &str,
    anchor_scalar: u32,
    focus_scalar: u32,
) -> Result<TextSelectionStateV1, TextKeyboardPolicyError> {
    require_admitted_boundary(domain, anchor_scalar, "anchor_scalar")?;
    require_admitted_boundary(domain, focus_scalar, "focus_scalar")?;
    Ok(TextSelectionStateV1 {
        protocol_version: "chaptera.text-selection-state.v1".to_owned(),
        story_id: domain.story_id.clone(),
        anchor_scalar,
        focus_scalar,
        revision_id: revision_id.to_owned(),
        edit_domain_id: edit_domain_id_v1(domain),
        projection_state: "layout_pending".to_owned(),
        layout_revision_id: None,
        anchor_visual_stop_id: None,
        focus_visual_stop_id: None,
        preferred_inline_x_emu: None,
    })
}

fn selection_decision(
    command: KeyboardCommandV1,
    domain: &StoryEditDomainV1,
    revision_id: &str,
    anchor: u32,
    focus: u32,
) -> Result<TextKeyboardDecisionV1, TextKeyboardPolicyError> {
    Ok(TextKeyboardDecisionV1 {
        protocol_version: "chaptera.text-keyboard-decision.v1".to_owned(),
        command,
        action: "selection".to_owned(),
        selection: Some(layout_pending_selection(
            domain,
            revision_id,
            anchor,
            focus,
        )?),
        delete_intent: None,
        grapheme_version: UNICODE_GRAPHEME_VERSION.to_owned(),
    })
}

fn delete_decision(
    command: KeyboardCommandV1,
    domain: &StoryEditDomainV1,
    start: u32,
    end: u32,
) -> Result<TextKeyboardDecisionV1, TextKeyboardPolicyError> {
    if start == end {
        return Ok(TextKeyboardDecisionV1 {
            protocol_version: "chaptera.text-keyboard-decision.v1".to_owned(),
            command,
            action: "boundary_noop".to_owned(),
            selection: None,
            delete_intent: None,
            grapheme_version: UNICODE_GRAPHEME_VERSION.to_owned(),
        });
    }
    validate_ordinary_story_range_v1(domain, start, end)
        .map_err(|error| TextKeyboardPolicyError::new(error.code, error.to_string()))?;
    Ok(TextKeyboardDecisionV1 {
        protocol_version: "chaptera.text-keyboard-decision.v1".to_owned(),
        command,
        action: "delete".to_owned(),
        selection: None,
        delete_intent: Some(ReplaceStoryRangeIntentV1 {
            protocol_version: "chaptera.replace-story-range-intent.v1".to_owned(),
            story_id: domain.story_id.clone(),
            start_scalar: start,
            end_scalar: end,
            replacement_text: String::new(),
        }),
        grapheme_version: UNICODE_GRAPHEME_VERSION.to_owned(),
    })
}

pub fn apply_text_keyboard_policy_v1(
    command: KeyboardCommandV1,
    story_text: &str,
    domain: &StoryEditDomainV1,
    selection: &TextSelectionStateV1,
    caret_map: &ResolvedTextCaretMapV1,
    expected_revision_id: &str,
) -> Result<TextKeyboardDecisionV1, TextKeyboardPolicyError> {
    let raw_len = u32::try_from(story_text.chars().count()).map_err(|_| {
        TextKeyboardPolicyError::new("stale_story", "Story scalar length overflows u32")
    })?;
    if domain.story_id != selection.story_id || caret_map.story_id != selection.story_id {
        return Err(TextKeyboardPolicyError::new(
            "story_mismatch",
            "keyboard inputs target different Stories",
        ));
    }
    if raw_len != domain.raw_scalar_len {
        return Err(TextKeyboardPolicyError::new(
            "stale_story",
            "Story text length disagrees with StoryEditDomainV1",
        ));
    }
    if caret_map.story_scalar_len != domain.raw_scalar_len {
        return Err(TextKeyboardPolicyError::new(
            "stale_layout_map",
            "caret map Story length disagrees with current domain",
        ));
    }
    validate_selection_state_v1(selection, domain, expected_revision_id)?;
    let boundaries = grapheme_boundaries_v1(story_text)?;
    let (floor, ceiling) = require_known_domain(domain)?;

    require_physical_boundary(
        caret_map,
        selection.focus_scalar,
        selection.focus_visual_stop_id.as_deref(),
    )?;

    let start = selection.anchor_scalar.min(selection.focus_scalar);
    let end = selection.anchor_scalar.max(selection.focus_scalar);
    let collapsed = selection.anchor_scalar == selection.focus_scalar;

    match command {
        KeyboardCommandV1::MovePrevious => {
            let target = if collapsed {
                previous_boundary(&boundaries, selection.focus_scalar, floor)
            } else {
                start
            };
            require_physical_boundary(caret_map, target, None)?;
            selection_decision(command, domain, expected_revision_id, target, target)
        }
        KeyboardCommandV1::MoveNext => {
            let target = if collapsed {
                next_boundary(&boundaries, selection.focus_scalar, ceiling)
            } else {
                end
            };
            require_physical_boundary(caret_map, target, None)?;
            selection_decision(command, domain, expected_revision_id, target, target)
        }
        KeyboardCommandV1::ExtendPrevious | KeyboardCommandV1::ExtendNext => {
            let target = if command == KeyboardCommandV1::ExtendPrevious {
                previous_boundary(&boundaries, selection.focus_scalar, floor)
            } else {
                next_boundary(&boundaries, selection.focus_scalar, ceiling)
            };
            require_physical_boundary(caret_map, target, None)?;
            selection_decision(
                command,
                domain,
                expected_revision_id,
                selection.anchor_scalar,
                target,
            )
        }
        KeyboardCommandV1::DeleteBackward | KeyboardCommandV1::DeleteForward if !collapsed => {
            delete_decision(command, domain, start, end)
        }
        KeyboardCommandV1::DeleteBackward => {
            let target = previous_boundary(&boundaries, selection.focus_scalar, floor);
            if target == selection.focus_scalar {
                delete_decision(command, domain, target, target)
            } else {
                require_physical_boundary(caret_map, target, None)?;
                delete_decision(command, domain, target, selection.focus_scalar)
            }
        }
        KeyboardCommandV1::DeleteForward => {
            let target = next_boundary(&boundaries, selection.focus_scalar, ceiling);
            if target == selection.focus_scalar {
                delete_decision(command, domain, target, target)
            } else {
                require_physical_boundary(caret_map, target, None)?;
                delete_decision(command, domain, selection.focus_scalar, target)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{StoryProvenanceV1, derive_story_edit_domain_v1};

    #[test]
    fn pinned_unicode_15_graphemes_cover_v0_cases() {
        assert_eq!(grapheme_boundaries_v1("abc").unwrap(), vec![0, 1, 2, 3]);
        assert_eq!(grapheme_boundaries_v1("a\u{301}b").unwrap(), vec![0, 2, 3]);
        let family = "👨‍👩‍👧‍👦";
        assert_eq!(
            grapheme_boundaries_v1(family).unwrap(),
            vec![0, u32::try_from(family.chars().count()).unwrap()]
        );
        assert_eq!(grapheme_boundaries_v1("🇺🇸🇨🇦").unwrap(), vec![0, 2, 4]);
        assert_eq!(grapheme_boundaries_v1("a\r\nb").unwrap(), vec![0, 1, 3, 4]);
    }

    #[test]
    fn protected_terminal_end_is_a_keyboard_boundary() {
        let domain = derive_story_edit_domain_v1(
            "story:q",
            "abc\r",
            StoryProvenanceV1::ImportedMatureQuillTerminalCr,
        )
        .unwrap();
        assert_eq!(require_known_domain(&domain).unwrap(), (0, 3));
    }
}
