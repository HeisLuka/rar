//! Native Rust conformance/runtime adapter for Chaptera text interaction V0.
//!
//! Canonical semantics remain service-side:
//! - services/editor-api/text_selection_state_v1.py
//! - services/editor-api/text_edit_session_v1.py
//! - services/editor-api/editor_text_session_gestures_v1.py
//!
//! This crate exposes only the bounded native Desktop subset needed to restore
//! direct in-canvas text activation without making egui/widget state semantic
//! authority. Durable Story mutation remains owned by the existing editor core.

use chaptera_text_caret_map_adapter::{
    CaretMapError, CaretStopV1, ResolvedTextCaretMapV1, caret_map_hash_v1,
    hit_test_story_position_v1, resolve_story_position_v1,
};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const SELECTION_VERSION_V1: &str = "chaptera.text-selection-state.v1";
pub const SESSION_VERSION_V1: &str = "chaptera.text-edit-session.v1";
pub const TRANSITION_VERSION_V1: &str = "chaptera.text-session-transition.v1";
pub const EXIT_VERSION_V1: &str = "chaptera.text-session-exit.v1";
pub const ACTIVATION_VERSION_V1: &str = "chaptera.desktop-text-activation-result.v1";
pub const DESKTOP_EXIT_VERSION_V1: &str = "chaptera.desktop-text-exit-result.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextInteractionError {
    pub code: &'static str,
    pub message: String,
}

impl TextInteractionError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn from_caret(error: CaretMapError) -> Self {
        Self::new(error.code, error.message)
    }
}

impl fmt::Display for TextInteractionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for TextInteractionError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryEditDomainV1 {
    pub story_id: String,
    pub raw_scalar_len: u32,
    pub status: String,
    pub caret_start_boundary: Option<u32>,
    pub caret_end_boundary: Option<u32>,
    pub domain_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextEntryCandidateV1 {
    pub target_id: String,
    pub story_id: String,
    pub frame_id: Option<String>,
    pub capability: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextPointerEntryContextV1 {
    pub page_x_emu: i64,
    pub page_y_emu: i64,
    #[serde(default)]
    pub page_id_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextPointerTargetV1 {
    pub page_id: String,
    pub page_x_emu: i64,
    pub page_y_emu: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextInitialPositionV1 {
    pub scalar_boundary: u32,
    pub visual_stop_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextSelectionStateV1 {
    pub protocol_version: String,
    pub story_id: String,
    pub anchor_scalar: u32,
    pub focus_scalar: u32,
    pub revision_id: String,
    pub edit_domain_id: String,
    pub projection_state: String,
    pub layout_revision_id: Option<String>,
    pub anchor_visual_stop_id: Option<String>,
    pub focus_visual_stop_id: Option<String>,
    pub preferred_inline_x_emu: Option<i64>,
}

impl TextSelectionStateV1 {
    pub fn is_collapsed(&self) -> bool {
        self.anchor_scalar == self.focus_scalar
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextEditSessionV1 {
    pub protocol_version: String,
    pub session_id: String,
    pub incarnation: u32,
    pub document_id: String,
    pub story_id: String,
    pub revision_id: String,
    pub edit_domain_id: String,
    pub layout_revision_id: String,
    pub caret_map_hash: String,
    pub entry_frame_id: Option<String>,
    pub current_frame_id: Option<String>,
    pub focus_owner: String,
    pub selection: TextSelectionStateV1,
    pub composition_active: bool,
    pub pending_interaction_metadata: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextSessionTransitionV1 {
    pub protocol_version: String,
    pub kind: String,
    pub previous_story_id: Option<String>,
    pub current_story_id: String,
    pub session: TextEditSessionV1,
    pub focus_context_discontinuity: bool,
    pub undo_group_boundary: bool,
    pub lifecycle_document_mutation_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextSessionExitV1 {
    pub protocol_version: String,
    pub session_id: String,
    pub incarnation: u32,
    pub closed_story_id: String,
    pub reason: String,
    pub composition_resolution: Option<String>,
    pub focus_context_discontinuity: bool,
    pub undo_group_boundary: bool,
    pub lifecycle_document_mutation_count: u32,
    pub still_pending_operation_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopTextActivationResultV1 {
    pub protocol_version: String,
    pub status: String,
    pub transition: Option<TextSessionTransitionV1>,
    pub active_session: Option<TextEditSessionV1>,
    pub story_shortcuts_owned: bool,
    pub canvas_object_shortcuts_owned: bool,
    pub document_mutation_count: u32,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopTextExitResultV1 {
    pub protocol_version: String,
    pub status: String,
    pub exit_receipt: Option<TextSessionExitV1>,
    pub active_session: Option<TextEditSessionV1>,
    pub story_shortcuts_owned: bool,
    pub canvas_object_shortcuts_owned: bool,
    pub transient_text_state_cleared: bool,
    pub document_mutation_count: u32,
    pub reason: Option<String>,
}

fn required(value: &str, code: &'static str, message: &str) -> Result<(), TextInteractionError> {
    if value.is_empty() {
        Err(TextInteractionError::new(code, message))
    } else {
        Ok(())
    }
}

fn validate_candidate(candidate: &TextEntryCandidateV1) -> Result<(), TextInteractionError> {
    required(
        &candidate.target_id,
        "entry_owner_invalid",
        "entry target_id is required",
    )?;
    required(
        &candidate.story_id,
        "entry_owner_invalid",
        "entry story_id is required",
    )?;
    match candidate.capability.as_str() {
        "editable" => Ok(()),
        "read_only" => Err(TextInteractionError::new(
            "read_only_story",
            candidate
                .reason
                .clone()
                .unwrap_or_else(|| "resolved Story is read-only".to_owned()),
        )),
        "unsupported" => Err(TextInteractionError::new(
            "story_editing_unsupported",
            candidate
                .reason
                .clone()
                .unwrap_or_else(|| "resolved Story is not editable by this capability".to_owned()),
        )),
        _ => Err(TextInteractionError::new(
            "entry_owner_invalid",
            "entry capability is invalid",
        )),
    }
}

fn validate_domain(
    domain: &StoryEditDomainV1,
    story_id: &str,
) -> Result<(u32, u32), TextInteractionError> {
    if domain.story_id != story_id {
        return Err(TextInteractionError::new(
            "entry_story_mismatch",
            "entry owner and edit domain target different Stories",
        ));
    }
    if domain.status != "known" {
        return Err(TextInteractionError::new(
            "edit_domain_unknown",
            "Story ordinary edit domain is unavailable",
        ));
    }
    let Some(start) = domain.caret_start_boundary else {
        return Err(TextInteractionError::new(
            "edit_domain_unknown",
            "ordinary Story caret start boundary is unavailable",
        ));
    };
    let Some(end) = domain.caret_end_boundary else {
        return Err(TextInteractionError::new(
            "edit_domain_unknown",
            "ordinary Story caret end boundary is unavailable",
        ));
    };
    if start > end || end > domain.raw_scalar_len {
        return Err(TextInteractionError::new(
            "invalid_edit_domain",
            "Story edit domain boundaries are invalid",
        ));
    }
    Ok((start, end))
}

fn validate_authoritative_context(
    candidate: &TextEntryCandidateV1,
    revision_id: &str,
    domain: &StoryEditDomainV1,
    caret_map: &ResolvedTextCaretMapV1,
    expected_layout_revision_id: &str,
) -> Result<(u32, u32), TextInteractionError> {
    validate_candidate(candidate)?;
    required(revision_id, "invalid_session", "revision_id is required")?;
    required(
        expected_layout_revision_id,
        "invalid_session",
        "expected_layout_revision_id is required",
    )?;
    let boundaries = validate_domain(domain, &candidate.story_id)?;
    if caret_map.story_id != candidate.story_id {
        return Err(TextInteractionError::new(
            "entry_story_mismatch",
            "caret map targets a different Story",
        ));
    }
    if caret_map.story_scalar_len != domain.raw_scalar_len {
        return Err(TextInteractionError::new(
            "stale_layout_map",
            "caret map Story extent differs from edit domain",
        ));
    }
    if caret_map.layout_revision_id != expected_layout_revision_id {
        return Err(TextInteractionError::new(
            "stale_layout_map",
            "caret map belongs to a different layout revision",
        ));
    }
    Ok(boundaries)
}

fn ensure_admitted(scalar: u32, domain: &StoryEditDomainV1) -> Result<(), TextInteractionError> {
    let (start, end) = validate_domain(domain, &domain.story_id)?;
    if scalar < start || scalar > end {
        return Err(TextInteractionError::new(
            "selection_reconcile_required",
            "selection lies outside the current ordinary Story edit domain",
        ));
    }
    Ok(())
}

fn projected_selection(
    revision_id: &str,
    domain: &StoryEditDomainV1,
    map: &ResolvedTextCaretMapV1,
    stop: &CaretStopV1,
) -> Result<TextSelectionStateV1, TextInteractionError> {
    ensure_admitted(stop.scalar_boundary, domain)?;
    Ok(TextSelectionStateV1 {
        protocol_version: SELECTION_VERSION_V1.to_owned(),
        story_id: domain.story_id.clone(),
        anchor_scalar: stop.scalar_boundary,
        focus_scalar: stop.scalar_boundary,
        revision_id: revision_id.to_owned(),
        edit_domain_id: domain.domain_id.clone(),
        projection_state: "projected".to_owned(),
        layout_revision_id: Some(map.layout_revision_id.clone()),
        anchor_visual_stop_id: Some(stop.stop_id.clone()),
        focus_visual_stop_id: Some(stop.stop_id.clone()),
        preferred_inline_x_emu: None,
    })
}

fn initial_selection(
    revision_id: &str,
    domain: &StoryEditDomainV1,
    map: &ResolvedTextCaretMapV1,
    initial: &TextInitialPositionV1,
) -> Result<TextSelectionStateV1, TextInteractionError> {
    ensure_admitted(initial.scalar_boundary, domain)?;
    if domain.raw_scalar_len == 0 && initial.scalar_boundary == 0 && map.caret_stops.is_empty() {
        if initial.visual_stop_id.is_some() {
            return Err(TextInteractionError::new(
                "invalid_caret_affinity",
                "empty Story has no materialized visual caret stop",
            ));
        }
        return Ok(TextSelectionStateV1 {
            protocol_version: SELECTION_VERSION_V1.to_owned(),
            story_id: domain.story_id.clone(),
            anchor_scalar: 0,
            focus_scalar: 0,
            revision_id: revision_id.to_owned(),
            edit_domain_id: domain.domain_id.clone(),
            projection_state: "layout_pending".to_owned(),
            layout_revision_id: None,
            anchor_visual_stop_id: None,
            focus_visual_stop_id: None,
            preferred_inline_x_emu: None,
        });
    }
    let stop = resolve_story_position_v1(
        map,
        initial.scalar_boundary,
        initial.visual_stop_id.as_deref(),
        Some(&map.layout_revision_id),
    )
    .map_err(TextInteractionError::from_caret)?;
    projected_selection(revision_id, domain, map, &stop)
}

fn pointer_selection(
    revision_id: &str,
    domain: &StoryEditDomainV1,
    map: &ResolvedTextCaretMapV1,
    pointer: &TextPointerTargetV1,
    expected_layout_revision_id: &str,
) -> Result<(TextSelectionStateV1, CaretStopV1), TextInteractionError> {
    if map.layout_revision_id != expected_layout_revision_id {
        return Err(TextInteractionError::new(
            "stale_layout_map",
            "caret map belongs to a different layout revision",
        ));
    }
    let stop = hit_test_story_position_v1(
        map,
        &pointer.page_id,
        pointer.page_x_emu,
        pointer.page_y_emu,
        Some(expected_layout_revision_id),
    )
    .map_err(TextInteractionError::from_caret)?;
    ensure_admitted(stop.scalar_boundary, domain)?;
    let selection = projected_selection(revision_id, domain, map, &stop)?;
    Ok((selection, stop))
}

fn canonical_metadata(
    mut metadata: Vec<(String, String)>,
) -> Result<Vec<(String, String)>, TextInteractionError> {
    metadata.sort_by(|a, b| a.0.cmp(&b.0));
    for pair in metadata.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(TextInteractionError::new(
                "invalid_session",
                "interaction metadata keys must be unique",
            ));
        }
    }
    if metadata.iter().any(|(key, _)| key.is_empty()) {
        return Err(TextInteractionError::new(
            "invalid_session",
            "interaction metadata key is required",
        ));
    }
    Ok(metadata)
}

// Mirrors the canonical V1 contract shape; keep authority parameters explicit.
#[allow(clippy::too_many_arguments)]
fn build_session(
    session_id: &str,
    incarnation: u32,
    document_id: &str,
    candidate: &TextEntryCandidateV1,
    revision_id: &str,
    domain: &StoryEditDomainV1,
    caret_map: &ResolvedTextCaretMapV1,
    selection: TextSelectionStateV1,
    metadata: Vec<(String, String)>,
) -> Result<TextEditSessionV1, TextInteractionError> {
    required(session_id, "invalid_session", "session_id is required")?;
    required(document_id, "invalid_session", "document_id is required")?;
    Ok(TextEditSessionV1 {
        protocol_version: SESSION_VERSION_V1.to_owned(),
        session_id: session_id.to_owned(),
        incarnation,
        document_id: document_id.to_owned(),
        story_id: candidate.story_id.clone(),
        revision_id: revision_id.to_owned(),
        edit_domain_id: domain.domain_id.clone(),
        layout_revision_id: caret_map.layout_revision_id.clone(),
        caret_map_hash: caret_map_hash_v1(caret_map),
        entry_frame_id: candidate.frame_id.clone(),
        current_frame_id: candidate.frame_id.clone(),
        focus_owner: "story_text".to_owned(),
        selection,
        composition_active: false,
        pending_interaction_metadata: canonical_metadata(metadata)?,
    })
}

// Mirrors the canonical V1 contract shape; keep authority parameters explicit.
#[allow(clippy::too_many_arguments)]
pub fn enter_text_edit_session_v1(
    session_id: &str,
    incarnation: u32,
    document_id: &str,
    candidate: &TextEntryCandidateV1,
    revision_id: &str,
    domain: &StoryEditDomainV1,
    caret_map: &ResolvedTextCaretMapV1,
    expected_layout_revision_id: &str,
    pointer_context: Option<&TextPointerTargetV1>,
    initial_position: Option<&TextInitialPositionV1>,
    metadata: Vec<(String, String)>,
) -> Result<TextSessionTransitionV1, TextInteractionError> {
    validate_authoritative_context(
        candidate,
        revision_id,
        domain,
        caret_map,
        expected_layout_revision_id,
    )?;
    if pointer_context.is_some() == initial_position.is_some() {
        return Err(TextInteractionError::new(
            "entry_position_required",
            "entry requires exactly one pointer context or explicit initial position",
        ));
    }
    let selection = if let Some(pointer) = pointer_context {
        pointer_selection(
            revision_id,
            domain,
            caret_map,
            pointer,
            expected_layout_revision_id,
        )?
        .0
    } else {
        initial_selection(
            revision_id,
            domain,
            caret_map,
            initial_position.expect("checked above"),
        )?
    };
    let session = build_session(
        session_id,
        incarnation,
        document_id,
        candidate,
        revision_id,
        domain,
        caret_map,
        selection,
        metadata,
    )?;
    Ok(TextSessionTransitionV1 {
        protocol_version: TRANSITION_VERSION_V1.to_owned(),
        kind: "enter".to_owned(),
        previous_story_id: None,
        current_story_id: session.story_id.clone(),
        session,
        focus_context_discontinuity: false,
        undo_group_boundary: false,
        lifecycle_document_mutation_count: 0,
    })
}

fn require_composition_resolved(
    session: &TextEditSessionV1,
    resolution: Option<&str>,
) -> Result<(), TextInteractionError> {
    if !session.composition_active {
        if resolution.is_some() {
            return Err(TextInteractionError::new(
                "invalid_composition_resolution",
                "no active composition requires resolution",
            ));
        }
        return Ok(());
    }
    match resolution {
        Some("cancelled" | "committed" | "reconciled") => Ok(()),
        _ => Err(TextInteractionError::new(
            "composition_transition_required",
            "active composition must commit/cancel/reconcile before focus context changes",
        )),
    }
}

pub fn handoff_same_story_frame_v1(
    session: &TextEditSessionV1,
    candidate: &TextEntryCandidateV1,
    domain: &StoryEditDomainV1,
    caret_map: &ResolvedTextCaretMapV1,
    expected_layout_revision_id: &str,
    pointer_context: Option<&TextPointerTargetV1>,
    composition_resolution: Option<&str>,
) -> Result<TextSessionTransitionV1, TextInteractionError> {
    validate_candidate(candidate)?;
    if candidate.story_id != session.story_id {
        return Err(TextInteractionError::new(
            "story_switch_required",
            "different Story ownership requires explicit session switch",
        ));
    }
    validate_authoritative_context(
        candidate,
        &session.revision_id,
        domain,
        caret_map,
        expected_layout_revision_id,
    )?;
    if domain.domain_id != session.edit_domain_id {
        return Err(TextInteractionError::new(
            "selection_reconcile_required",
            "session edit-domain receipt is stale",
        ));
    }
    let mut updated = session.clone();
    if let Some(pointer) = pointer_context {
        require_composition_resolved(session, composition_resolution)?;
        let (selection, _) = pointer_selection(
            &session.revision_id,
            domain,
            caret_map,
            pointer,
            expected_layout_revision_id,
        )?;
        updated.selection = selection;
        updated.composition_active = false;
    } else if caret_map.layout_revision_id != session.layout_revision_id {
        let anchor = resolve_story_position_v1(
            caret_map,
            session.selection.anchor_scalar,
            session.selection.anchor_visual_stop_id.as_deref(),
            Some(expected_layout_revision_id),
        )
        .map_err(TextInteractionError::from_caret)?;
        let focus = resolve_story_position_v1(
            caret_map,
            session.selection.focus_scalar,
            session.selection.focus_visual_stop_id.as_deref(),
            Some(expected_layout_revision_id),
        )
        .map_err(TextInteractionError::from_caret)?;
        updated.selection = TextSelectionStateV1 {
            protocol_version: SELECTION_VERSION_V1.to_owned(),
            story_id: session.story_id.clone(),
            anchor_scalar: anchor.scalar_boundary,
            focus_scalar: focus.scalar_boundary,
            revision_id: session.revision_id.clone(),
            edit_domain_id: domain.domain_id.clone(),
            projection_state: "projected".to_owned(),
            layout_revision_id: Some(caret_map.layout_revision_id.clone()),
            anchor_visual_stop_id: Some(anchor.stop_id),
            focus_visual_stop_id: Some(focus.stop_id),
            preferred_inline_x_emu: None,
        };
    }
    updated.layout_revision_id = caret_map.layout_revision_id.clone();
    updated.caret_map_hash = caret_map_hash_v1(caret_map);
    updated.current_frame_id = candidate.frame_id.clone();
    Ok(TextSessionTransitionV1 {
        protocol_version: TRANSITION_VERSION_V1.to_owned(),
        kind: "same_story_handoff".to_owned(),
        previous_story_id: Some(session.story_id.clone()),
        current_story_id: session.story_id.clone(),
        session: updated,
        focus_context_discontinuity: false,
        undo_group_boundary: false,
        lifecycle_document_mutation_count: 0,
    })
}

// Mirrors the canonical V1 contract shape; keep authority parameters explicit.
#[allow(clippy::too_many_arguments)]
pub fn switch_text_edit_session_v1(
    session: &TextEditSessionV1,
    candidate: &TextEntryCandidateV1,
    revision_id: &str,
    domain: &StoryEditDomainV1,
    caret_map: &ResolvedTextCaretMapV1,
    expected_layout_revision_id: &str,
    pointer_context: Option<&TextPointerTargetV1>,
    initial_position: Option<&TextInitialPositionV1>,
    composition_resolution: Option<&str>,
    metadata: Vec<(String, String)>,
) -> Result<TextSessionTransitionV1, TextInteractionError> {
    if candidate.story_id == session.story_id {
        return Err(TextInteractionError::new(
            "same_story_handoff_required",
            "same Story must use frame handoff rather than session restart",
        ));
    }
    require_composition_resolved(session, composition_resolution)?;
    let mut entered = enter_text_edit_session_v1(
        &session.session_id,
        session.incarnation + 1,
        &session.document_id,
        candidate,
        revision_id,
        domain,
        caret_map,
        expected_layout_revision_id,
        pointer_context,
        initial_position,
        metadata,
    )?;
    entered.kind = "story_switch".to_owned();
    entered.previous_story_id = Some(session.story_id.clone());
    entered.focus_context_discontinuity = true;
    entered.undo_group_boundary = true;
    Ok(entered)
}

pub fn rebind_text_edit_session_authority_v1(
    session: &TextEditSessionV1,
    revision_id: &str,
    domain: &StoryEditDomainV1,
    caret_map: &ResolvedTextCaretMapV1,
    expected_layout_revision_id: &str,
    selection: TextSelectionStateV1,
) -> Result<TextSessionTransitionV1, TextInteractionError> {
    if session.story_id != domain.story_id || session.story_id != caret_map.story_id {
        return Err(TextInteractionError::new(
            "entry_story_mismatch",
            "session rebind authorities target different Stories",
        ));
    }
    if caret_map.layout_revision_id != expected_layout_revision_id {
        return Err(TextInteractionError::new(
            "stale_layout_map",
            "caret map belongs to a different layout revision",
        ));
    }
    if selection.story_id != session.story_id
        || selection.revision_id != revision_id
        || selection.edit_domain_id != domain.domain_id
    {
        return Err(TextInteractionError::new(
            "reconcile_required",
            "selection does not belong to rebound session authority",
        ));
    }
    let mut updated = session.clone();
    updated.revision_id = revision_id.to_owned();
    updated.edit_domain_id = domain.domain_id.clone();
    updated.layout_revision_id = caret_map.layout_revision_id.clone();
    updated.caret_map_hash = caret_map_hash_v1(caret_map);
    updated.selection = selection;
    updated.composition_active = false;
    Ok(TextSessionTransitionV1 {
        protocol_version: TRANSITION_VERSION_V1.to_owned(),
        kind: "authority_rebind".to_owned(),
        previous_story_id: Some(session.story_id.clone()),
        current_story_id: session.story_id.clone(),
        session: updated,
        focus_context_discontinuity: false,
        undo_group_boundary: false,
        lifecycle_document_mutation_count: 0,
    })
}

pub fn exit_text_edit_session_v1(
    session: &TextEditSessionV1,
    reason: &str,
    composition_resolution: Option<&str>,
    submitted_durable_operation_ids: &[String],
) -> Result<TextSessionExitV1, TextInteractionError> {
    required(reason, "invalid_session_exit", "exit reason is required")?;
    require_composition_resolved(session, composition_resolution)?;
    Ok(TextSessionExitV1 {
        protocol_version: EXIT_VERSION_V1.to_owned(),
        session_id: session.session_id.clone(),
        incarnation: session.incarnation,
        closed_story_id: session.story_id.clone(),
        reason: reason.to_owned(),
        composition_resolution: composition_resolution.map(str::to_owned),
        focus_context_discontinuity: true,
        undo_group_boundary: true,
        lifecycle_document_mutation_count: 0,
        still_pending_operation_ids: submitted_durable_operation_ids.to_vec(),
    })
}

pub fn story_shortcut_admitted_v1(
    active_session: Option<&TextEditSessionV1>,
    focus_owner: &str,
) -> Result<bool, TextInteractionError> {
    match focus_owner {
        "canvas" | "story_text" | "inspector" | "find_replace" | "modal" => {}
        _ => {
            return Err(TextInteractionError::new(
                "invalid_focus_owner",
                "unsupported desktop focus owner",
            ));
        }
    }
    Ok(active_session.is_some() && focus_owner == "story_text")
}

fn first_admitted_stop_in_frame<'a>(
    candidate: &TextEntryCandidateV1,
    domain: &StoryEditDomainV1,
    map: &'a ResolvedTextCaretMapV1,
) -> Result<Option<&'a CaretStopV1>, TextInteractionError> {
    let (start, end) = validate_domain(domain, &candidate.story_id)?;
    if map.story_id != candidate.story_id {
        return Err(TextInteractionError::new(
            "entry_story_mismatch",
            "candidate/domain/caret map target different Stories",
        ));
    }
    Ok(map
        .caret_stops
        .iter()
        .filter(|stop| {
            start <= stop.scalar_boundary
                && stop.scalar_boundary <= end
                && candidate
                    .frame_id
                    .as_ref()
                    .is_none_or(|frame| frame == &stop.frame_id)
        })
        .min_by_key(|stop| {
            (
                stop.flow_ordinal,
                stop.scalar_boundary,
                stop.page_y_top_emu,
                stop.page_x_emu,
                stop.stop_id.as_str(),
            )
        }))
}

fn explicit_activation_position(
    candidate: &TextEntryCandidateV1,
    domain: &StoryEditDomainV1,
    map: &ResolvedTextCaretMapV1,
) -> Result<(Option<TextPointerTargetV1>, Option<TextInitialPositionV1>), TextInteractionError> {
    if let Some(stop) = first_admitted_stop_in_frame(candidate, domain, map)? {
        return Ok((
            Some(TextPointerTargetV1 {
                page_id: stop.page_id.clone(),
                page_x_emu: stop.page_x_emu,
                page_y_emu: (stop.page_y_top_emu + stop.page_y_bottom_emu) / 2,
            }),
            None,
        ));
    }
    if domain.raw_scalar_len == 0
        && domain.caret_start_boundary == Some(0)
        && domain.caret_end_boundary == Some(0)
    {
        return Ok((
            None,
            Some(TextInitialPositionV1 {
                scalar_boundary: 0,
                visual_stop_id: None,
            }),
        ));
    }
    Err(TextInteractionError::new(
        "direct_edit_unavailable",
        "selected TextFrame/Story has no authoritative placed caret stop",
    ))
}

fn preflight_pointer_target(
    candidate: &TextEntryCandidateV1,
    domain: &StoryEditDomainV1,
    map: &ResolvedTextCaretMapV1,
    pointer: &TextPointerTargetV1,
    expected_layout_revision_id: &str,
) -> Result<CaretStopV1, TextInteractionError> {
    validate_authoritative_context(
        candidate,
        "preflight",
        domain,
        map,
        expected_layout_revision_id,
    )?;
    let stop = hit_test_story_position_v1(
        map,
        &pointer.page_id,
        pointer.page_x_emu,
        pointer.page_y_emu,
        Some(expected_layout_revision_id),
    )
    .map_err(TextInteractionError::from_caret)?;
    if candidate
        .frame_id
        .as_ref()
        .is_some_and(|frame| frame != &stop.frame_id)
    {
        return Err(TextInteractionError::new(
            "direct_edit_unavailable",
            "requested TextFrame has no authoritative caret hit at the pointer location",
        ));
    }
    ensure_admitted(stop.scalar_boundary, domain)?;
    Ok(stop)
}

fn activation_from_transition(
    transition: TextSessionTransitionV1,
) -> Result<DesktopTextActivationResultV1, TextInteractionError> {
    let status = match transition.kind.as_str() {
        "enter" => "entered",
        "same_story_handoff" => "same_story_handoff",
        "story_switch" => "story_switch",
        _ => {
            return Err(TextInteractionError::new(
                "invalid_session_transition",
                "gesture activation received non-entry session transition",
            ));
        }
    };
    Ok(DesktopTextActivationResultV1 {
        protocol_version: ACTIVATION_VERSION_V1.to_owned(),
        status: status.to_owned(),
        active_session: Some(transition.session.clone()),
        transition: Some(transition),
        story_shortcuts_owned: true,
        canvas_object_shortcuts_owned: false,
        document_mutation_count: 0,
        reason: None,
    })
}

// Mirrors the canonical V1 contract shape; keep authority parameters explicit.
#[allow(clippy::too_many_arguments)]
pub fn activate_explicit_edit_text_v1(
    session_id: &str,
    document_id: &str,
    revision_id: &str,
    candidate: &TextEntryCandidateV1,
    domain: &StoryEditDomainV1,
    caret_map: &ResolvedTextCaretMapV1,
    expected_layout_revision_id: &str,
    active_session: Option<&TextEditSessionV1>,
    composition_resolution: Option<&str>,
) -> Result<DesktopTextActivationResultV1, TextInteractionError> {
    let (pointer, initial) = explicit_activation_position(candidate, domain, caret_map)?;
    let transition = match active_session {
        None => enter_text_edit_session_v1(
            session_id,
            0,
            document_id,
            candidate,
            revision_id,
            domain,
            caret_map,
            expected_layout_revision_id,
            pointer.as_ref(),
            initial.as_ref(),
            vec![("entry_reason".to_owned(), "explicit_edit_text".to_owned())],
        )?,
        Some(active) if active.story_id == candidate.story_id => handoff_same_story_frame_v1(
            active,
            candidate,
            domain,
            caret_map,
            expected_layout_revision_id,
            pointer.as_ref(),
            composition_resolution,
        )?,
        Some(active) => switch_text_edit_session_v1(
            active,
            candidate,
            revision_id,
            domain,
            caret_map,
            expected_layout_revision_id,
            pointer.as_ref(),
            initial.as_ref(),
            composition_resolution,
            vec![("entry_reason".to_owned(), "explicit_edit_text".to_owned())],
        )?,
    };
    activation_from_transition(transition)
}

// Mirrors the canonical V1 contract shape; keep authority parameters explicit.
#[allow(clippy::too_many_arguments)]
pub fn activate_pointer_text_v1(
    candidate: &TextEntryCandidateV1,
    revision_id: &str,
    domain: &StoryEditDomainV1,
    caret_map: &ResolvedTextCaretMapV1,
    expected_layout_revision_id: &str,
    pointer: &TextPointerTargetV1,
    active_session: Option<&TextEditSessionV1>,
    explicit_edit_requested: bool,
    click_count: u32,
    session_id: &str,
    document_id: &str,
    composition_resolution: Option<&str>,
) -> Result<DesktopTextActivationResultV1, TextInteractionError> {
    if click_count == 0 {
        return Err(TextInteractionError::new(
            "invalid_click_count",
            "click_count must be positive integer",
        ));
    }
    if click_count != 1 {
        return Ok(DesktopTextActivationResultV1 {
            protocol_version: ACTIVATION_VERSION_V1.to_owned(),
            status: "multiclick_unavailable".to_owned(),
            transition: None,
            active_session: active_session.cloned(),
            story_shortcuts_owned: active_session.is_some(),
            canvas_object_shortcuts_owned: active_session.is_none(),
            document_mutation_count: 0,
            reason: Some(
                "double/triple-click text semantics are gated by EXP-TEXT-MULTICLICK-SELECTION-01"
                    .to_owned(),
            ),
        });
    }
    if active_session.is_none() && !explicit_edit_requested {
        return Ok(DesktopTextActivationResultV1 {
            protocol_version: ACTIVATION_VERSION_V1.to_owned(),
            status: "canvas_object_selection".to_owned(),
            transition: None,
            active_session: None,
            story_shortcuts_owned: false,
            canvas_object_shortcuts_owned: true,
            document_mutation_count: 0,
            reason: Some("inactive single-click remains canvas object selection".to_owned()),
        });
    }

    if let Err(error) = preflight_pointer_target(
        candidate,
        domain,
        caret_map,
        pointer,
        expected_layout_revision_id,
    ) {
        if matches!(
            error.code,
            "direct_edit_unavailable"
                | "unplaced_hit_test"
                | "unplaced_story_position"
                | "internal_cluster_unsupported"
        ) {
            return Ok(DesktopTextActivationResultV1 {
                protocol_version: ACTIVATION_VERSION_V1.to_owned(),
                status: "direct_edit_unavailable".to_owned(),
                transition: None,
                active_session: active_session.cloned(),
                story_shortcuts_owned: active_session.is_some(),
                canvas_object_shortcuts_owned: active_session.is_none(),
                document_mutation_count: 0,
                reason: Some(error.to_string()),
            });
        }
        return Err(error);
    }

    let transition = match active_session {
        None => enter_text_edit_session_v1(
            session_id,
            0,
            document_id,
            candidate,
            revision_id,
            domain,
            caret_map,
            expected_layout_revision_id,
            Some(pointer),
            None,
            vec![("entry_reason".to_owned(), "pointer_edit_text".to_owned())],
        )?,
        Some(active) if active.story_id == candidate.story_id => handoff_same_story_frame_v1(
            active,
            candidate,
            domain,
            caret_map,
            expected_layout_revision_id,
            Some(pointer),
            composition_resolution,
        )?,
        Some(active) => switch_text_edit_session_v1(
            active,
            candidate,
            revision_id,
            domain,
            caret_map,
            expected_layout_revision_id,
            Some(pointer),
            None,
            composition_resolution,
            vec![("entry_reason".to_owned(), "pointer_edit_text".to_owned())],
        )?,
    };
    activation_from_transition(transition)
}

pub fn exit_desktop_text_mode_v1(
    active_session: Option<&TextEditSessionV1>,
    trigger: &str,
    focus_owner: &str,
    composition_resolution: Option<&str>,
    submitted_durable_operation_ids: &[String],
    host_consumed: bool,
) -> Result<DesktopTextExitResultV1, TextInteractionError> {
    match trigger {
        "escape" | "canvas_non_text_click" | "non_text_tool" | "explicit_exit" => {}
        _ => {
            return Err(TextInteractionError::new(
                "invalid_exit_trigger",
                "unsupported text-session exit trigger",
            ));
        }
    }
    match focus_owner {
        "canvas" | "story_text" | "inspector" | "find_replace" | "modal" => {}
        _ => {
            return Err(TextInteractionError::new(
                "invalid_focus_owner",
                "unsupported desktop focus owner",
            ));
        }
    }
    let Some(session) = active_session else {
        return Ok(DesktopTextExitResultV1 {
            protocol_version: DESKTOP_EXIT_VERSION_V1.to_owned(),
            status: "no_active_session".to_owned(),
            exit_receipt: None,
            active_session: None,
            story_shortcuts_owned: false,
            canvas_object_shortcuts_owned: true,
            transient_text_state_cleared: false,
            document_mutation_count: 0,
            reason: None,
        });
    };
    if trigger == "escape"
        && (host_consumed || matches!(focus_owner, "inspector" | "find_replace" | "modal"))
    {
        return Ok(DesktopTextExitResultV1 {
            protocol_version: DESKTOP_EXIT_VERSION_V1.to_owned(),
            status: "host_focus_owned".to_owned(),
            exit_receipt: None,
            active_session: Some(session.clone()),
            story_shortcuts_owned: false,
            canvas_object_shortcuts_owned: false,
            transient_text_state_cleared: false,
            document_mutation_count: 0,
            reason: Some("focused host control owns Escape/input".to_owned()),
        });
    }
    match exit_text_edit_session_v1(
        session,
        trigger,
        composition_resolution,
        submitted_durable_operation_ids,
    ) {
        Ok(receipt) => Ok(DesktopTextExitResultV1 {
            protocol_version: DESKTOP_EXIT_VERSION_V1.to_owned(),
            status: "exited".to_owned(),
            exit_receipt: Some(receipt),
            active_session: None,
            story_shortcuts_owned: false,
            canvas_object_shortcuts_owned: true,
            transient_text_state_cleared: true,
            document_mutation_count: 0,
            reason: None,
        }),
        Err(error) if error.code == "composition_transition_required" => {
            Ok(DesktopTextExitResultV1 {
                protocol_version: DESKTOP_EXIT_VERSION_V1.to_owned(),
                status: "composition_resolution_required".to_owned(),
                exit_receipt: None,
                active_session: Some(session.clone()),
                story_shortcuts_owned: true,
                canvas_object_shortcuts_owned: false,
                transient_text_state_cleared: false,
                document_mutation_count: 0,
                reason: Some(error.to_string()),
            })
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chaptera_text_caret_map_adapter::{
        CaretMapBuildInputV1, ResolvedClusterV1, ResolvedLineFragmentV1,
        build_resolved_text_caret_map_v1,
    };

    fn cluster(start: u32, end: u32, x0: i64, x1: i64) -> ResolvedClusterV1 {
        ResolvedClusterV1 {
            start_scalar: start,
            end_scalar: end,
            page_x_start_emu: x0,
            page_x_end_emu: x1,
            frame_x_start_emu: x0,
            frame_x_end_emu: x1,
            painted: true,
            internal_caret_stops: Vec::new(),
        }
    }

    // Mirrors the canonical conformance fixture shape; keep arguments explicit.
    #[allow(clippy::too_many_arguments)]
    fn line(
        story: &str,
        id: &str,
        ordinal: u32,
        frame: &str,
        prev: Option<&str>,
        next: Option<&str>,
        y: i64,
        clusters: Vec<ResolvedClusterV1>,
    ) -> ResolvedLineFragmentV1 {
        ResolvedLineFragmentV1 {
            story_id: story.to_owned(),
            page_id: "page:1".to_owned(),
            frame_id: frame.to_owned(),
            line_id: id.to_owned(),
            flow_ordinal: ordinal,
            previous_line_id: prev.map(str::to_owned),
            next_line_id: next.map(str::to_owned),
            page_y_top_emu: y,
            page_y_bottom_emu: y + 20,
            frame_y_top_emu: y,
            frame_y_bottom_emu: y + 20,
            clusters,
        }
    }

    fn context(
        story: &str,
        text_len: u32,
        linked: bool,
    ) -> (StoryEditDomainV1, ResolvedTextCaretMapV1) {
        let lines = if text_len == 0 {
            Vec::new()
        } else if linked && text_len >= 2 {
            vec![
                line(
                    story,
                    "l1",
                    0,
                    "frame:A",
                    None,
                    Some("l2"),
                    0,
                    vec![cluster(0, 1, 0, 10)],
                ),
                line(
                    story,
                    "l2",
                    1,
                    "frame:B",
                    Some("l1"),
                    None,
                    30,
                    (1..text_len)
                        .map(|i| cluster(i, i + 1, ((i - 1) * 10) as i64, (i * 10) as i64))
                        .collect(),
                ),
            ]
        } else {
            vec![line(
                story,
                "l1",
                0,
                "frame:A",
                None,
                None,
                0,
                (0..text_len)
                    .map(|i| cluster(i, i + 1, (i * 10) as i64, ((i + 1) * 10) as i64))
                    .collect(),
            )]
        };
        let map = build_resolved_text_caret_map_v1(CaretMapBuildInputV1 {
            layout_revision_id: "layout:1".to_owned(),
            story_id: story.to_owned(),
            story_scalar_len: text_len,
            lines,
        })
        .unwrap();
        (
            StoryEditDomainV1 {
                story_id: story.to_owned(),
                raw_scalar_len: text_len,
                status: "known".to_owned(),
                caret_start_boundary: Some(0),
                caret_end_boundary: Some(text_len),
                domain_id: format!("domain:{story}:{text_len}"),
            },
            map,
        )
    }

    fn candidate(story: &str, frame: &str) -> TextEntryCandidateV1 {
        TextEntryCandidateV1 {
            target_id: frame.to_owned(),
            story_id: story.to_owned(),
            frame_id: Some(frame.to_owned()),
            capability: "editable".to_owned(),
            reason: None,
        }
    }

    #[test]
    fn pointer_entry_matches_canonical_focus_behavior() {
        let (domain, map) = context("story:1", 3, false);
        let result = enter_text_edit_session_v1(
            "session:1",
            0,
            "doc:1",
            &candidate("story:1", "frame:A"),
            "rev:1",
            &domain,
            &map,
            "layout:1",
            Some(&TextPointerTargetV1 {
                page_id: "page:1".to_owned(),
                page_x_emu: 19,
                page_y_emu: 10,
            }),
            None,
            Vec::new(),
        )
        .unwrap();
        assert_eq!(result.session.selection.focus_scalar, 2);
        assert_eq!(result.lifecycle_document_mutation_count, 0);
        assert!(!result.undo_group_boundary);
    }

    #[test]
    fn empty_story_enters_semantic_zero_without_fake_stop() {
        let (domain, map) = context("story:new", 0, false);
        let result = enter_text_edit_session_v1(
            "session:new",
            0,
            "doc:1",
            &candidate("story:new", "frame:new"),
            "rev:1",
            &domain,
            &map,
            "layout:1",
            None,
            Some(&TextInitialPositionV1 {
                scalar_boundary: 0,
                visual_stop_id: None,
            }),
            Vec::new(),
        )
        .unwrap();
        assert_eq!(result.session.selection.focus_scalar, 0);
        assert_eq!(result.session.selection.projection_state, "layout_pending");
        assert_eq!(result.lifecycle_document_mutation_count, 0);
    }

    #[test]
    fn linked_frame_handoff_preserves_session_identity() {
        let (domain, map) = context("story:1", 2, true);
        let active = enter_text_edit_session_v1(
            "session:1",
            4,
            "doc:1",
            &candidate("story:1", "frame:A"),
            "rev:1",
            &domain,
            &map,
            "layout:1",
            Some(&TextPointerTargetV1 {
                page_id: "page:1".to_owned(),
                page_x_emu: 0,
                page_y_emu: 10,
            }),
            None,
            Vec::new(),
        )
        .unwrap()
        .session;
        let handoff = handoff_same_story_frame_v1(
            &active,
            &candidate("story:1", "frame:B"),
            &domain,
            &map,
            "layout:1",
            Some(&TextPointerTargetV1 {
                page_id: "page:1".to_owned(),
                page_x_emu: 10,
                page_y_emu: 40,
            }),
            None,
        )
        .unwrap();
        assert_eq!(handoff.session.session_id, "session:1");
        assert_eq!(handoff.session.incarnation, 4);
        assert_eq!(handoff.session.current_frame_id.as_deref(), Some("frame:B"));
        assert_eq!(handoff.session.selection.focus_scalar, 2);
    }

    #[test]
    fn story_switch_increments_incarnation_and_closes_undo_context() {
        let (domain1, map1) = context("story:1", 1, false);
        let active = enter_text_edit_session_v1(
            "session:1",
            2,
            "doc:1",
            &candidate("story:1", "frame:A"),
            "rev:1",
            &domain1,
            &map1,
            "layout:1",
            None,
            Some(&TextInitialPositionV1 {
                scalar_boundary: 1,
                visual_stop_id: None,
            }),
            Vec::new(),
        )
        .unwrap()
        .session;
        let (domain2, map2) = context("story:2", 1, false);
        let switched = switch_text_edit_session_v1(
            &active,
            &candidate("story:2", "frame:A"),
            "rev:2",
            &domain2,
            &map2,
            "layout:1",
            None,
            Some(&TextInitialPositionV1 {
                scalar_boundary: 0,
                visual_stop_id: None,
            }),
            None,
            Vec::new(),
        )
        .unwrap();
        assert_eq!(switched.kind, "story_switch");
        assert_eq!(switched.session.incarnation, 3);
        assert!(switched.focus_context_discontinuity);
        assert!(switched.undo_group_boundary);
        assert_eq!(switched.lifecycle_document_mutation_count, 0);
    }

    #[test]
    fn inactive_single_click_stays_canvas_selection() {
        let (domain, map) = context("story:1", 3, false);
        let result = activate_pointer_text_v1(
            &candidate("story:1", "frame:A"),
            "rev:1",
            &domain,
            &map,
            "layout:1",
            &TextPointerTargetV1 {
                page_id: "page:1".to_owned(),
                page_x_emu: 19,
                page_y_emu: 10,
            },
            None,
            false,
            1,
            "session:1",
            "doc:1",
            None,
        )
        .unwrap();
        assert_eq!(result.status, "canvas_object_selection");
        assert!(result.active_session.is_none());
        assert!(result.canvas_object_shortcuts_owned);
        assert_eq!(result.document_mutation_count, 0);
    }

    #[test]
    fn exact_frame_pointer_mismatch_fails_direct_edit_without_snap() {
        let (domain, map) = context("story:1", 1, false);
        let result = activate_pointer_text_v1(
            &candidate("story:1", "frame:B"),
            "rev:1",
            &domain,
            &map,
            "layout:1",
            &TextPointerTargetV1 {
                page_id: "page:1".to_owned(),
                page_x_emu: 5,
                page_y_emu: 200,
            },
            None,
            true,
            1,
            "session:1",
            "doc:1",
            None,
        )
        .unwrap();
        assert_eq!(result.status, "direct_edit_unavailable");
        assert!(result.active_session.is_none());
    }

    #[test]
    fn escape_focus_fence_and_exit_are_zero_mutation() {
        let (domain, map) = context("story:1", 1, false);
        let active = activate_explicit_edit_text_v1(
            "session:1",
            "doc:1",
            "rev:1",
            &candidate("story:1", "frame:A"),
            &domain,
            &map,
            "layout:1",
            None,
            None,
        )
        .unwrap()
        .active_session
        .unwrap();
        let fenced =
            exit_desktop_text_mode_v1(Some(&active), "escape", "find_replace", None, &[], false)
                .unwrap();
        assert_eq!(fenced.status, "host_focus_owned");
        assert!(fenced.active_session.is_some());
        let exited = exit_desktop_text_mode_v1(
            Some(&active),
            "escape",
            "story_text",
            None,
            &["op:already-submitted".to_owned()],
            false,
        )
        .unwrap();
        assert_eq!(exited.status, "exited");
        assert!(exited.active_session.is_none());
        assert_eq!(
            exited
                .exit_receipt
                .as_ref()
                .unwrap()
                .still_pending_operation_ids,
            vec!["op:already-submitted".to_owned()]
        );
        assert_eq!(exited.document_mutation_count, 0);
    }
}
