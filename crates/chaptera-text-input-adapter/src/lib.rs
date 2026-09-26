//! Native bounded text-input bridge for Chaptera Desktop.
//!
//! Canonical semantic donors remain the service-side V1 contracts. This crate
//! only makes those contracts consumable by the Rust desktop and lowers an
//! admitted edit to the existing pub-editor ReplaceStoryRange authority.

pub mod domain;
pub mod ingress;
pub mod keyboard;

use chaptera_text_caret_map_adapter::ResolvedTextCaretMapV1;
use chaptera_text_interaction_adapter::TextSelectionStateV1;
use domain::{
    StoryEditDomainError, StoryEditDomainV1, edit_domain_id_v1, validate_ordinary_story_range_v1,
};
use ingress::{TextIngressError, normalize_external_text_v1};
use keyboard::{
    KeyboardCommandV1, TextKeyboardDecisionV1, TextKeyboardPolicyError,
    apply_text_keyboard_policy_v1,
};
use pub_editor::{EditOperation, EditorError, EditorSession, StoryId};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextInputAdapterError {
    pub code: String,
    pub message: String,
}

impl TextInputAdapterError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for TextInputAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for TextInputAdapterError {}

impl From<StoryEditDomainError> for TextInputAdapterError {
    fn from(error: StoryEditDomainError) -> Self {
        Self::new(error.code, error.message)
    }
}

impl From<TextIngressError> for TextInputAdapterError {
    fn from(error: TextIngressError) -> Self {
        Self::new("text_ingress_rejected", error.message)
    }
}

impl From<TextKeyboardPolicyError> for TextInputAdapterError {
    fn from(error: TextKeyboardPolicyError) -> Self {
        Self::new(error.code, error.message)
    }
}

impl From<EditorError> for TextInputAdapterError {
    fn from(error: EditorError) -> Self {
        Self::new("editor_operation_rejected", error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostEditScalarSelectionV1 {
    pub story_id: String,
    pub anchor_scalar: u32,
    pub focus_scalar: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextInputCommitV1 {
    pub operation: EditOperation,
    pub post_edit_selection: PostEditScalarSelectionV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextKeyboardApplicationV1 {
    pub decision: TextKeyboardDecisionV1,
    pub commit: Option<TextInputCommitV1>,
}

fn scalar_byte_offset(text: &str, scalar_index: u32) -> Option<usize> {
    let target = usize::try_from(scalar_index).ok()?;
    if target == text.chars().count() {
        return Some(text.len());
    }
    text.char_indices().nth(target).map(|(offset, _)| offset)
}

fn scalar_slice(text: &str, start_scalar: u32, end_scalar: u32) -> Option<&str> {
    if end_scalar < start_scalar {
        return None;
    }
    let start = scalar_byte_offset(text, start_scalar)?;
    let end = scalar_byte_offset(text, end_scalar)?;
    text.get(start..end)
}

fn current_story_text(
    session: &EditorSession,
    story_id: StoryId,
) -> Result<String, TextInputAdapterError> {
    session
        .graph()
        .stories
        .get(&story_id)
        .map(|story| story.text.clone())
        .ok_or_else(|| {
            TextInputAdapterError::new("missing_story", "Story is absent from EditorSession")
        })
}

fn validate_selection_authority(
    story_id: StoryId,
    domain: &StoryEditDomainV1,
    selection: &TextSelectionStateV1,
    expected_revision_id: &str,
) -> Result<(), TextInputAdapterError> {
    let canonical_story_id = story_id.as_canonical().to_string();
    if domain.story_id != canonical_story_id || selection.story_id != canonical_story_id {
        return Err(TextInputAdapterError::new(
            "story_mismatch",
            "StoryId, edit domain and selection do not identify the same Story",
        ));
    }
    if selection.revision_id != expected_revision_id {
        return Err(TextInputAdapterError::new(
            "stale_selection_revision",
            "selection belongs to a different canonical revision",
        ));
    }
    if selection.edit_domain_id != edit_domain_id_v1(domain) {
        return Err(TextInputAdapterError::new(
            "selection_reconcile_required",
            "selection was projected against a different Story edit domain",
        ));
    }
    validate_ordinary_story_range_v1(
        domain,
        selection.anchor_scalar.min(selection.focus_scalar),
        selection.anchor_scalar.max(selection.focus_scalar),
    )?;
    Ok(())
}

/// Normalize external platform text once, replace the currently admitted
/// selection/caret through pub-editor, and return only scalar selection for
/// authoritative re-layout/rebind.
pub fn replace_selection_with_external_text_v1(
    session: &mut EditorSession,
    story_id: StoryId,
    domain: &StoryEditDomainV1,
    selection: &TextSelectionStateV1,
    expected_revision_id: &str,
    external_text: &str,
) -> Result<TextInputCommitV1, TextInputAdapterError> {
    validate_selection_authority(story_id, domain, selection, expected_revision_id)?;
    let before = current_story_text(session, story_id)?;
    let current_len = u32::try_from(before.chars().count()).map_err(|_| {
        TextInputAdapterError::new("stale_story", "Story scalar length overflows u32")
    })?;
    if current_len != domain.raw_scalar_len {
        return Err(TextInputAdapterError::new(
            "stale_story",
            "current Story text length disagrees with StoryEditDomainV1",
        ));
    }

    let start = selection.anchor_scalar.min(selection.focus_scalar);
    let end = selection.anchor_scalar.max(selection.focus_scalar);
    validate_ordinary_story_range_v1(domain, start, end)?;
    let expected_before = scalar_slice(&before, start, end)
        .ok_or_else(|| {
            TextInputAdapterError::new("invalid_range", "selection is not a valid scalar range")
        })?
        .to_owned();
    let replacement = normalize_external_text_v1(external_text)?;
    let post = start.checked_add(replacement.scalar_len).ok_or_else(|| {
        TextInputAdapterError::new("scalar_overflow", "post-edit caret boundary overflows u32")
    })?;

    let operation =
        session.replace_story_range(story_id, start, end, expected_before, replacement.text)?;

    Ok(TextInputCommitV1 {
        operation,
        post_edit_selection: PostEditScalarSelectionV1 {
            story_id: domain.story_id.clone(),
            anchor_scalar: post,
            focus_scalar: post,
        },
    })
}

/// Apply canonical logical keyboard policy. Movement/extension is transient.
/// Delete decisions lower to exactly one pub-editor ReplaceStoryRange operation.
pub fn apply_keyboard_command_and_lower_v1(
    session: &mut EditorSession,
    story_id: StoryId,
    domain: &StoryEditDomainV1,
    selection: &TextSelectionStateV1,
    caret_map: &ResolvedTextCaretMapV1,
    expected_revision_id: &str,
    command: KeyboardCommandV1,
) -> Result<TextKeyboardApplicationV1, TextInputAdapterError> {
    validate_selection_authority(story_id, domain, selection, expected_revision_id)?;
    let before = current_story_text(session, story_id)?;
    let decision = apply_text_keyboard_policy_v1(
        command,
        &before,
        domain,
        selection,
        caret_map,
        expected_revision_id,
    )?;

    let Some(intent) = decision.delete_intent.as_ref() else {
        return Ok(TextKeyboardApplicationV1 {
            decision,
            commit: None,
        });
    };

    if intent.story_id != story_id.as_canonical().to_string() {
        return Err(TextInputAdapterError::new(
            "story_mismatch",
            "keyboard delete intent targets a different Story",
        ));
    }
    validate_ordinary_story_range_v1(domain, intent.start_scalar, intent.end_scalar)?;
    let expected_before = scalar_slice(&before, intent.start_scalar, intent.end_scalar)
        .ok_or_else(|| {
            TextInputAdapterError::new("invalid_range", "delete intent is not a valid scalar range")
        })?
        .to_owned();

    let operation = session.replace_story_range(
        story_id,
        intent.start_scalar,
        intent.end_scalar,
        expected_before,
        intent.replacement_text.clone(),
    )?;
    let post = intent.start_scalar;
    let commit = TextInputCommitV1 {
        operation,
        post_edit_selection: PostEditScalarSelectionV1 {
            story_id: domain.story_id.clone(),
            anchor_scalar: post,
            focus_scalar: post,
        },
    };

    Ok(TextKeyboardApplicationV1 {
        decision,
        commit: Some(commit),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{StoryProvenanceV1, derive_story_edit_domain_v1};
    use ingress::normalize_external_text_v1;

    #[test]
    fn scalar_slice_uses_unicode_scalar_boundaries_not_bytes() {
        assert_eq!(scalar_slice("A😀B", 1, 2), Some("😀"));
        assert_eq!(scalar_slice("e\u{301}x", 0, 2), Some("e\u{301}"));
    }

    #[test]
    fn external_newline_normalization_does_not_append_terminal_cr() {
        let value = normalize_external_text_v1("A\r\nB\nC").unwrap();
        assert_eq!(value.text, "A\rB\rC");
        assert!(!value.text.ends_with('\r'));
    }

    #[test]
    fn mature_terminal_cr_domain_admits_insert_before_protected_suffix() {
        let domain = derive_story_edit_domain_v1(
            "story:q",
            "ABC\r",
            StoryProvenanceV1::ImportedMatureQuillTerminalCr,
        )
        .unwrap();
        validate_ordinary_story_range_v1(&domain, 3, 3).unwrap();
        assert!(validate_ordinary_story_range_v1(&domain, 3, 4).is_err());
    }

    #[test]
    fn real_sample_newsletter_commit_is_one_replace_range_and_undo_exact() {
        use crate::domain::{StoryProvenanceHintV1, derive_editor_story_edit_domain_v1};
        use pub_editor::{Sha256Digest, open_mature_0x2c_editor};
        use sha2::{Digest, Sha256};
        use std::{env, fs};

        let Some(path) = env::var_os("CHAPTERA_SAMPLE_NEWSLETTER") else {
            eprintln!(
                "CHAPTERA_SAMPLE_NEWSLETTER not set; dedicated real-fixture gate owns this test"
            );
            return;
        };
        let bytes = fs::read(path).expect("read pinned SampleNewsletter");
        let digest = Sha256::digest(&bytes);
        let mut digest_bytes = [0_u8; 32];
        digest_bytes.copy_from_slice(&digest);
        let source_hash = Sha256Digest::from_bytes(digest_bytes);
        let mut editor = open_mature_0x2c_editor(&bytes, source_hash)
            .expect("open real SampleNewsletter editor");

        let story_id = editor
            .graph()
            .stories
            .keys()
            .copied()
            .find(|story_id| {
                editor.can_replace_story_text(*story_id).is_ok()
                    && derive_editor_story_edit_domain_v1(
                        &editor,
                        *story_id,
                        StoryProvenanceHintV1::ImportedAuto,
                    )
                    .is_ok_and(|domain| domain.status == "known")
            })
            .expect("real fixture should expose one provenance-qualified editable Story");
        let before = editor.graph().stories[&story_id].text.clone();
        let domain = derive_editor_story_edit_domain_v1(
            &editor,
            story_id,
            StoryProvenanceHintV1::ImportedAuto,
        )
        .expect("derive mature Quill edit domain");
        let caret = domain.caret_end_boundary.expect("known caret end");
        let selection = TextSelectionStateV1 {
            protocol_version: "chaptera.text-selection-state.v1".to_owned(),
            story_id: domain.story_id.clone(),
            anchor_scalar: caret,
            focus_scalar: caret,
            revision_id: "rev:real-fixture".to_owned(),
            edit_domain_id: edit_domain_id_v1(&domain),
            projection_state: "layout_pending".to_owned(),
            layout_revision_id: None,
            anchor_visual_stop_id: None,
            focus_visual_stop_id: None,
            preferred_inline_x_emu: None,
        };

        let commit = replace_selection_with_external_text_v1(
            &mut editor,
            story_id,
            &domain,
            &selection,
            "rev:real-fixture",
            "X",
        )
        .expect("one bounded real Story insertion");
        assert!(matches!(
            commit.operation,
            EditOperation::ReplaceStoryRange { .. }
        ));
        assert_eq!(editor.operations().len(), 1);
        assert_eq!(editor.source_hash(), source_hash);
        assert_ne!(editor.graph().stories[&story_id].text, before);

        editor.undo().expect("undo real Story insertion");
        assert_eq!(editor.graph().stories[&story_id].text, before);
        assert_eq!(editor.source_hash(), source_hash);
    }
}
