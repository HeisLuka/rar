//! Native Rust adapter for Chaptera Desktop text input V0.
//!
//! Canonical behavior remains defined by the service-side V1 contracts:
//! story_edit_domain_v1.py, text_ingress_v1.py and text_keyboard_policy_v1.py.
//! This crate lowers admitted native input to the existing ReplaceStoryRange
//! operation shape. It does not own a second Story mutation engine.

use chaptera_text_caret_map_adapter::{
    ResolvedTextCaretMapV1, resolve_story_position_v1,
};
use chaptera_text_interaction_adapter::TextSelectionStateV1;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use unicode_segmentation::UnicodeSegmentation;

pub const STORY_EDIT_DOMAIN_VERSION_V1: &str = "chaptera.story-edit-domain.v1";
pub const TEXT_INGRESS_VERSION_V1: &str = "chaptera.text-ingress.v1";
pub const KEYBOARD_POLICY_VERSION_V1: &str = "chaptera.text-keyboard-policy.v1";
pub const KEYBOARD_DECISION_VERSION_V1: &str = "chaptera.text-keyboard-decision.v1";
pub const REPLACE_RANGE_INTENT_VERSION_V1: &str = "chaptera.replace-story-range-intent.v1";
pub const UNICODE_GRAPHEME_VERSION: &str = "15.0.0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextInputError {
    pub code: &'static str,
    pub message: String,
}
impl TextInputError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
}
impl fmt::Display for TextInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for TextInputError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryProvenanceV1 {
    ChapteraCreated,
    ImportedMatureQuillTerminalCr,
    ImportedUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatureQuillStoryEvidenceV1 {
    pub confirmed_mature_0x2c_profile: bool,
    pub syid_strs_text_story_slice_confirmed: bool,
    pub source_slice_ends_with_u000d: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectedStoryRangeV1 {
    pub start_scalar: u32,
    pub end_scalar: u32,
    pub reason: String,
    pub provenance: StoryProvenanceV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryEditDomainV1 {
    pub protocol_version: String,
    pub story_id: String,
    pub provenance: StoryProvenanceV1,
    pub status: String,
    pub raw_scalar_len: u32,
    pub editable_start_scalar: Option<u32>,
    pub editable_end_scalar: Option<u32>,
    pub caret_start_boundary: Option<u32>,
    pub caret_end_boundary: Option<u32>,
    pub protected_ranges: Vec<ProtectedStoryRangeV1>,
    pub domain_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalStoryTextV1 {
    pub protocol_version: String,
    pub text: String,
    pub scalar_len: u32,
    pub paragraph_boundary_count: u32,
    pub text_sha256: String,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplaceStoryRangeIntentV1 {
    pub protocol_version: String,
    pub story_id: String,
    pub start_scalar: u32,
    pub end_scalar: u32,
    pub expected_before: String,
    pub replacement_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostEditScalarSelectionV1 {
    pub story_id: String,
    pub anchor_scalar: u32,
    pub focus_scalar: u32,
    pub requires_authority_rebind: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextKeyboardDecisionV1 {
    pub protocol_version: String,
    pub command: KeyboardCommandV1,
    pub action: String,
    pub selection: Option<TextSelectionStateV1>,
    pub replace_intent: Option<ReplaceStoryRangeIntentV1>,
    pub post_edit_selection: Option<PostEditScalarSelectionV1>,
    pub grapheme_version: String,
}

fn scalar_len(text: &str) -> u32 {
    text.chars().count() as u32
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(bytes);
    format!("{:x}", hash.finalize())
}

fn scalar_byte_offset(text: &str, scalar: u32) -> Option<usize> {
    if scalar == scalar_len(text) {
        return Some(text.len());
    }
    text.char_indices().nth(scalar as usize).map(|(byte, _)| byte)
}

fn scalar_slice(text: &str, start: u32, end: u32) -> Result<&str, TextInputError> {
    if start > end || end > scalar_len(text) {
        return Err(TextInputError::new("invalid_range", "Story scalar range is invalid"));
    }
    let a = scalar_byte_offset(text, start)
        .ok_or_else(|| TextInputError::new("invalid_range", "start scalar is invalid"))?;
    let b = scalar_byte_offset(text, end)
        .ok_or_else(|| TextInputError::new("invalid_range", "end scalar is invalid"))?;
    Ok(&text[a..b])
}

fn domain_id_v1(
    story_id: &str,
    provenance: StoryProvenanceV1,
    raw_scalar_len: u32,
    editable_end: Option<u32>,
) -> String {
    let payload = format!(
        "chaptera.story-edit-domain.v1\n{story_id}\n{:?}\n{raw_scalar_len}\n{:?}",
        provenance, editable_end
    );
    format!("sha256:{}", sha256_hex(payload.as_bytes()))
}

pub fn derive_story_edit_domain_v1(
    story_id: &str,
    story_text: &str,
    provenance: StoryProvenanceV1,
    mature_quill_evidence: Option<&MatureQuillStoryEvidenceV1>,
) -> Result<StoryEditDomainV1, TextInputError> {
    if story_id.is_empty() {
        return Err(TextInputError::new("invalid_story", "story_id is required"));
    }
    let raw = scalar_len(story_text);
    let (status, editable_end, protected) = match provenance {
        StoryProvenanceV1::ChapteraCreated => ("known", Some(raw), Vec::new()),
        StoryProvenanceV1::ImportedUnknown => ("edit_domain_unknown", None, Vec::new()),
        StoryProvenanceV1::ImportedMatureQuillTerminalCr => {
            let evidence = mature_quill_evidence.ok_or_else(|| {
                TextInputError::new(
                    "invalid_provenance",
                    "mature Quill terminal-CR provenance requires explicit source evidence",
                )
            })?;
            if !evidence.confirmed_mature_0x2c_profile
                || !evidence.syid_strs_text_story_slice_confirmed
                || !evidence.source_slice_ends_with_u000d
                || !story_text.ends_with('\r')
                || raw == 0
            {
                return Err(TextInputError::new(
                    "invalid_provenance",
                    "terminal CR may be protected only with confirmed mature-0x2C SYID/STRS/TEXT source provenance",
                ));
            }
            let end = raw - 1;
            (
                "known",
                Some(end),
                vec![ProtectedStoryRangeV1 {
                    start_scalar: end,
                    end_scalar: raw,
                    reason: "source_terminal_paragraph_mark".to_owned(),
                    provenance,
                }],
            )
        }
    };

    Ok(StoryEditDomainV1 {
        protocol_version: STORY_EDIT_DOMAIN_VERSION_V1.to_owned(),
        story_id: story_id.to_owned(),
        provenance,
        status: status.to_owned(),
        raw_scalar_len: raw,
        editable_start_scalar: editable_end.map(|_| 0),
        editable_end_scalar: editable_end,
        caret_start_boundary: editable_end.map(|_| 0),
        caret_end_boundary: editable_end,
        protected_ranges: protected,
        domain_id: domain_id_v1(story_id, provenance, raw, editable_end),
    })
}

pub fn validate_ordinary_story_range_v1(
    domain: &StoryEditDomainV1,
    start: u32,
    end: u32,
) -> Result<(), TextInputError> {
    if domain.status != "known" {
        return Err(TextInputError::new(
            "edit_domain_unknown",
            "ordinary edit domain is unknown for imported Story provenance",
        ));
    }
    if start > end || end > domain.raw_scalar_len {
        return Err(TextInputError::new("invalid_range", "ordinary Story scalar range is invalid"));
    }
    let floor = domain.editable_start_scalar.expect("known domain has start");
    let ceiling = domain.editable_end_scalar.expect("known domain has end");
    if start < floor {
        return Err(TextInputError::new("invalid_range", "range starts before editable content"));
    }
    if start == end && start == ceiling {
        return Ok(());
    }
    if start > ceiling || end > ceiling {
        if !domain.protected_ranges.is_empty() {
            return Err(TextInputError::new(
                "protected_story_structure",
                "ordinary Story range overlaps protected source structure",
            ));
        }
        return Err(TextInputError::new("invalid_range", "range exceeds editable content"));
    }
    Ok(())
}

pub fn normalize_external_text_v1(input: &str) -> CanonicalStoryTextV1 {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                out.push('\r');
                if matches!(chars.peek(), Some('\n')) {
                    chars.next();
                }
            }
            '\n' => out.push('\r'),
            other => out.push(other),
        }
    }
    CanonicalStoryTextV1 {
        protocol_version: TEXT_INGRESS_VERSION_V1.to_owned(),
        scalar_len: scalar_len(&out),
        paragraph_boundary_count: out.chars().filter(|ch| *ch == '\r').count() as u32,
        text_sha256: sha256_hex(out.as_bytes()),
        text: out,
    }
}

pub fn validate_canonical_fragment_text_v1(
    text: &str,
) -> Result<CanonicalStoryTextV1, TextInputError> {
    if text.contains('\n') {
        return Err(TextInputError::new(
            "invalid_canonical_text",
            "canonical Story fragment must not contain LF/CRLF delimiters",
        ));
    }
    Ok(CanonicalStoryTextV1 {
        protocol_version: TEXT_INGRESS_VERSION_V1.to_owned(),
        scalar_len: scalar_len(text),
        paragraph_boundary_count: text.chars().filter(|ch| *ch == '\r').count() as u32,
        text_sha256: sha256_hex(text.as_bytes()),
        text: text.to_owned(),
    })
}

pub fn grapheme_boundaries_v1(text: &str) -> Vec<u32> {
    // unicode-segmentation = 1.10.1 is pinned because that release carries
    // Unicode 15.0.0 UAX#29 tables, matching the canonical Python V1 profile.
    let mut boundaries = text
        .grapheme_indices(true)
        .map(|(byte, _)| text[..byte].chars().count() as u32)
        .collect::<Vec<_>>();
    let end = scalar_len(text);
    if boundaries.last().copied() != Some(end) {
        boundaries.push(end);
    }
    if boundaries.is_empty() {
        boundaries.push(0);
    }
    boundaries
}

fn previous_boundary(boundaries: &[u32], scalar: u32, floor: u32) -> u32 {
    boundaries
        .iter()
        .copied()
        .filter(|value| *value >= floor && *value < scalar)
        .max()
        .unwrap_or(scalar)
}
fn next_boundary(boundaries: &[u32], scalar: u32, ceiling: u32) -> u32 {
    boundaries
        .iter()
        .copied()
        .filter(|value| scalar < *value && *value <= ceiling)
        .min()
        .unwrap_or(scalar)
}

fn require_context(
    story_text: &str,
    domain: &StoryEditDomainV1,
    selection: &TextSelectionStateV1,
    caret_map: &ResolvedTextCaretMapV1,
    expected_revision_id: &str,
) -> Result<(), TextInputError> {
    if domain.story_id != selection.story_id || caret_map.story_id != selection.story_id {
        return Err(TextInputError::new("story_mismatch", "keyboard inputs target different Stories"));
    }
    if scalar_len(story_text) != domain.raw_scalar_len {
        return Err(TextInputError::new("stale_story", "Story text length disagrees with edit domain"));
    }
    if caret_map.story_scalar_len != domain.raw_scalar_len {
        return Err(TextInputError::new("stale_layout_map", "caret map Story length disagrees with edit domain"));
    }
    if selection.revision_id != expected_revision_id {
        return Err(TextInputError::new("stale_revision", "selection belongs to another revision"));
    }
    if selection.edit_domain_id != domain.domain_id {
        return Err(TextInputError::new("stale_edit_domain", "selection belongs to another edit domain"));
    }
    validate_ordinary_story_range_v1(domain, selection.anchor_scalar, selection.anchor_scalar)?;
    validate_ordinary_story_range_v1(domain, selection.focus_scalar, selection.focus_scalar)?;
    Ok(())
}

fn require_physical_boundary(
    caret_map: &ResolvedTextCaretMapV1,
    scalar: u32,
    stop_id: Option<&str>,
) -> Result<(), TextInputError> {
    resolve_story_position_v1(
        caret_map,
        scalar,
        stop_id,
        Some(&caret_map.layout_revision_id),
    )
    .map(|_| ())
    .map_err(|error| {
        if matches!(
            error.code,
            "internal_cluster_unsupported"
                | "unplaced_story_position"
                | "caret_affinity_required"
                | "invalid_caret_affinity"
                | "stale_layout_map"
        ) {
            TextInputError::new("caret_geometry_unsupported", error.to_string())
        } else {
            TextInputError::new(error.code, error.to_string())
        }
    })
}

fn selection_state(
    prior: &TextSelectionStateV1,
    anchor: u32,
    focus: u32,
) -> TextSelectionStateV1 {
    TextSelectionStateV1 {
        protocol_version: prior.protocol_version.clone(),
        story_id: prior.story_id.clone(),
        anchor_scalar: anchor,
        focus_scalar: focus,
        revision_id: prior.revision_id.clone(),
        edit_domain_id: prior.edit_domain_id.clone(),
        projection_state: "projected".to_owned(),
        layout_revision_id: prior.layout_revision_id.clone(),
        anchor_visual_stop_id: None,
        focus_visual_stop_id: None,
        preferred_inline_x_emu: prior.preferred_inline_x_emu,
    }
}

fn replacement_intent(
    story_text: &str,
    domain: &StoryEditDomainV1,
    start: u32,
    end: u32,
    replacement_text: String,
) -> Result<(ReplaceStoryRangeIntentV1, PostEditScalarSelectionV1), TextInputError> {
    validate_ordinary_story_range_v1(domain, start, end)?;
    let expected_before = scalar_slice(story_text, start, end)?.to_owned();
    let inserted_len = scalar_len(&replacement_text);
    let focus = start
        .checked_add(inserted_len)
        .ok_or_else(|| TextInputError::new("invalid_range", "post-edit scalar boundary overflow"))?;
    Ok((
        ReplaceStoryRangeIntentV1 {
            protocol_version: REPLACE_RANGE_INTENT_VERSION_V1.to_owned(),
            story_id: domain.story_id.clone(),
            start_scalar: start,
            end_scalar: end,
            expected_before,
            replacement_text,
        },
        PostEditScalarSelectionV1 {
            story_id: domain.story_id.clone(),
            anchor_scalar: focus,
            focus_scalar: focus,
            requires_authority_rebind: true,
        },
    ))
}

pub fn lower_external_insert_v1(
    story_text: &str,
    domain: &StoryEditDomainV1,
    selection: &TextSelectionStateV1,
    input_text: &str,
) -> Result<(ReplaceStoryRangeIntentV1, PostEditScalarSelectionV1), TextInputError> {
    if domain.story_id != selection.story_id || selection.edit_domain_id != domain.domain_id {
        return Err(TextInputError::new("stale_edit_domain", "selection/edit-domain mismatch"));
    }
    let start = selection.anchor_scalar.min(selection.focus_scalar);
    let end = selection.anchor_scalar.max(selection.focus_scalar);
    let normalized = normalize_external_text_v1(input_text);
    replacement_intent(story_text, domain, start, end, normalized.text)
}

pub fn apply_text_keyboard_policy_v1(
    command: KeyboardCommandV1,
    story_text: &str,
    domain: &StoryEditDomainV1,
    selection: &TextSelectionStateV1,
    caret_map: &ResolvedTextCaretMapV1,
    expected_revision_id: &str,
) -> Result<TextKeyboardDecisionV1, TextInputError> {
    require_context(story_text, domain, selection, caret_map, expected_revision_id)?;
    require_physical_boundary(
        caret_map,
        selection.focus_scalar,
        selection.focus_visual_stop_id.as_deref(),
    )?;

    let boundaries = grapheme_boundaries_v1(story_text);
    let floor = domain.caret_start_boundary.expect("known domain");
    let ceiling = domain.caret_end_boundary.expect("known domain");
    let start = selection.anchor_scalar.min(selection.focus_scalar);
    let end = selection.anchor_scalar.max(selection.focus_scalar);
    let collapsed = start == end;

    let selection_decision = |anchor, focus| TextKeyboardDecisionV1 {
        protocol_version: KEYBOARD_DECISION_VERSION_V1.to_owned(),
        command,
        action: "selection".to_owned(),
        selection: Some(selection_state(selection, anchor, focus)),
        replace_intent: None,
        post_edit_selection: None,
        grapheme_version: UNICODE_GRAPHEME_VERSION.to_owned(),
    };
    let noop = || TextKeyboardDecisionV1 {
        protocol_version: KEYBOARD_DECISION_VERSION_V1.to_owned(),
        command,
        action: "boundary_noop".to_owned(),
        selection: None,
        replace_intent: None,
        post_edit_selection: None,
        grapheme_version: UNICODE_GRAPHEME_VERSION.to_owned(),
    };

    match command {
        KeyboardCommandV1::MovePrevious => {
            let target = if collapsed {
                previous_boundary(&boundaries, selection.focus_scalar, floor)
            } else {
                start
            };
            require_physical_boundary(caret_map, target, None)?;
            Ok(selection_decision(target, target))
        }
        KeyboardCommandV1::MoveNext => {
            let target = if collapsed {
                next_boundary(&boundaries, selection.focus_scalar, ceiling)
            } else {
                end
            };
            require_physical_boundary(caret_map, target, None)?;
            Ok(selection_decision(target, target))
        }
        KeyboardCommandV1::ExtendPrevious | KeyboardCommandV1::ExtendNext => {
            let target = if command == KeyboardCommandV1::ExtendPrevious {
                previous_boundary(&boundaries, selection.focus_scalar, floor)
            } else {
                next_boundary(&boundaries, selection.focus_scalar, ceiling)
            };
            require_physical_boundary(caret_map, target, None)?;
            Ok(selection_decision(selection.anchor_scalar, target))
        }
        KeyboardCommandV1::DeleteBackward | KeyboardCommandV1::DeleteForward => {
            let (delete_start, delete_end) = if !collapsed {
                (start, end)
            } else if command == KeyboardCommandV1::DeleteBackward {
                (previous_boundary(&boundaries, selection.focus_scalar, floor), selection.focus_scalar)
            } else {
                (selection.focus_scalar, next_boundary(&boundaries, selection.focus_scalar, ceiling))
            };
            if delete_start == delete_end {
                return Ok(noop());
            }
            require_physical_boundary(caret_map, delete_start, None)?;
            require_physical_boundary(caret_map, delete_end, None)?;
            let (intent, post) = replacement_intent(
                story_text,
                domain,
                delete_start,
                delete_end,
                String::new(),
            )?;
            Ok(TextKeyboardDecisionV1 {
                protocol_version: KEYBOARD_DECISION_VERSION_V1.to_owned(),
                command,
                action: "delete".to_owned(),
                selection: None,
                replace_intent: Some(intent),
                post_edit_selection: Some(post),
                grapheme_version: UNICODE_GRAPHEME_VERSION.to_owned(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chaptera_text_caret_map_adapter::{
        CaretMapBuildInputV1, ResolvedClusterV1, ResolvedLineFragmentV1,
        build_resolved_text_caret_map_v1,
    };

    fn map(story: &str, len: u32) -> ResolvedTextCaretMapV1 {
        let clusters = (0..len)
            .map(|i| ResolvedClusterV1 {
                start_scalar: i,
                end_scalar: i + 1,
                page_x_start_emu: (i * 10) as i64,
                page_x_end_emu: ((i + 1) * 10) as i64,
                frame_x_start_emu: (i * 10) as i64,
                frame_x_end_emu: ((i + 1) * 10) as i64,
                painted: true,
                internal_caret_stops: Vec::new(),
            })
            .collect();
        build_resolved_text_caret_map_v1(CaretMapBuildInputV1 {
            layout_revision_id: "layout:1".to_owned(),
            story_id: story.to_owned(),
            story_scalar_len: len,
            lines: if len == 0 { Vec::new() } else { vec![ResolvedLineFragmentV1 {
                story_id: story.to_owned(), page_id: "page:1".to_owned(),
                frame_id: "frame:1".to_owned(), line_id: "line:1".to_owned(),
                flow_ordinal: 0, previous_line_id: None, next_line_id: None,
                page_y_top_emu: 0, page_y_bottom_emu: 20,
                frame_y_top_emu: 0, frame_y_bottom_emu: 20, clusters,
            }] },
        }).unwrap()
    }

    fn selection(domain: &StoryEditDomainV1, focus: u32) -> TextSelectionStateV1 {
        TextSelectionStateV1 {
            protocol_version: "chaptera.text-selection-state.v1".to_owned(),
            story_id: domain.story_id.clone(),
            anchor_scalar: focus, focus_scalar: focus,
            revision_id: "rev:1".to_owned(), edit_domain_id: domain.domain_id.clone(),
            projection_state: "projected".to_owned(),
            layout_revision_id: Some("layout:1".to_owned()),
            anchor_visual_stop_id: None, focus_visual_stop_id: None,
            preferred_inline_x_emu: None,
        }
    }

    #[test]
    fn ingress_normalizes_newline_spelling_once() {
        let out = normalize_external_text_v1("A\r\nB\nC\rD");
        assert_eq!(out.text, "A\rB\rC\rD");
        assert_eq!(out.paragraph_boundary_count, 3);
    }

    #[test]
    fn trailing_cr_is_not_provenance() {
        let unknown = derive_story_edit_domain_v1(
            "story:1", "hello\r", StoryProvenanceV1::ImportedUnknown, None
        ).unwrap();
        assert_eq!(unknown.status, "edit_domain_unknown");
        assert!(derive_story_edit_domain_v1(
            "story:1", "hello\r", StoryProvenanceV1::ImportedMatureQuillTerminalCr, None
        ).is_err());
    }

    #[test]
    fn confirmed_mature_terminal_cr_is_protected() {
        let evidence = MatureQuillStoryEvidenceV1 {
            confirmed_mature_0x2c_profile: true,
            syid_strs_text_story_slice_confirmed: true,
            source_slice_ends_with_u000d: true,
        };
        let domain = derive_story_edit_domain_v1(
            "story:1", "hello\r", StoryProvenanceV1::ImportedMatureQuillTerminalCr, Some(&evidence)
        ).unwrap();
        assert_eq!(domain.editable_end_scalar, Some(5));
        assert_eq!(
            validate_ordinary_story_range_v1(&domain, 5, 6).unwrap_err().code,
            "protected_story_structure"
        );
    }

    #[test]
    fn unicode15_graphemes_cover_combining_zwj_and_regional_indicator() {
        assert_eq!(grapheme_boundaries_v1("a\u{0301}b"), vec![0, 2, 3]);
        assert_eq!(grapheme_boundaries_v1("👨‍👩‍👧‍👦x"), vec![0, 7, 8]);
        assert_eq!(grapheme_boundaries_v1("🇺🇸x"), vec![0, 2, 3]);
    }

    #[test]
    fn insert_replaces_selection_and_returns_rebind_position() {
        let domain = derive_story_edit_domain_v1(
            "story:1", "abc", StoryProvenanceV1::ChapteraCreated, None
        ).unwrap();
        let mut sel = selection(&domain, 1);
        sel.anchor_scalar = 1;
        sel.focus_scalar = 2;
        let (intent, post) = lower_external_insert_v1("abc", &domain, &sel, "X\nY").unwrap();
        assert_eq!(intent.start_scalar, 1);
        assert_eq!(intent.end_scalar, 2);
        assert_eq!(intent.expected_before, "b");
        assert_eq!(intent.replacement_text, "X\rY");
        assert_eq!(post.focus_scalar, 4);
        assert!(post.requires_authority_rebind);
    }

    #[test]
    fn delete_backward_uses_grapheme_boundary_not_scalar_count() {
        let text = "a\u{0301}b";
        let domain = derive_story_edit_domain_v1(
            "story:1", text, StoryProvenanceV1::ChapteraCreated, None
        ).unwrap();
        let caret_map = map("story:1", 3);
        let sel = selection(&domain, 2);
        let decision = apply_text_keyboard_policy_v1(
            KeyboardCommandV1::DeleteBackward, text, &domain, &sel, &caret_map, "rev:1"
        ).unwrap();
        let intent = decision.replace_intent.unwrap();
        assert_eq!((intent.start_scalar, intent.end_scalar), (0, 2));
        assert_eq!(intent.expected_before, "a\u{0301}");
    }

    #[test]
    fn protected_terminal_boundary_delete_forward_is_noop() {
        let evidence = MatureQuillStoryEvidenceV1 {
            confirmed_mature_0x2c_profile: true,
            syid_strs_text_story_slice_confirmed: true,
            source_slice_ends_with_u000d: true,
        };
        let text = "a\r";
        let domain = derive_story_edit_domain_v1(
            "story:1", text, StoryProvenanceV1::ImportedMatureQuillTerminalCr, Some(&evidence)
        ).unwrap();
        let caret_map = map("story:1", 2);
        let sel = selection(&domain, 1);
        let decision = apply_text_keyboard_policy_v1(
            KeyboardCommandV1::DeleteForward, text, &domain, &sel, &caret_map, "rev:1"
        ).unwrap();
        assert_eq!(decision.action, "boundary_noop");
        assert!(decision.replace_intent.is_none());
    }
}
