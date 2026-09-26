use chaptera_desktop_fallback_font_resource as fallback_resource;
use chaptera_desktop_shaped_flow_runtime::{
    DesktopStoryLayoutV1, ExplicitDesktopFontResourceV1, build_current_story_layout_v1,
};
use chaptera_text_caret_map_adapter::{CaretStopV1, resolve_story_position_v1};
use chaptera_text_input_adapter::{
    TextInputCommitV1, apply_keyboard_command_and_lower_v1,
    domain::{
        StoryEditDomainV1, StoryProvenanceHintV1, derive_editor_story_edit_domain_v1,
        to_interaction_domain_v1,
    },
    keyboard::KeyboardCommandV1,
    replace_selection_with_external_text_v1,
};
use chaptera_text_interaction_adapter::{
    SELECTION_VERSION_V1, TextEditSessionV1, TextEntryCandidateV1, TextPointerTargetV1,
    TextSelectionStateV1, activate_explicit_edit_text_v1, activate_pointer_text_v1,
    exit_desktop_text_mode_v1, rebind_text_edit_session_authority_v1,
};
use pub_editor::{EditorSession, NodeId, StoryId};

#[derive(Debug, Clone)]
pub struct DesktopTextMode {
    pub story_id: StoryId,
    pub frame_id: NodeId,
    pub domain: StoryEditDomainV1,
    pub layout: DesktopStoryLayoutV1,
    pub session: TextEditSessionV1,
}

fn fallback_font_resource() -> ExplicitDesktopFontResourceV1<'static> {
    ExplicitDesktopFontResourceV1 {
        resource_id: fallback_resource::RESOURCE_ID,
        expected_sha256: fallback_resource::EXPECTED_SHA256,
        face_index: 0,
        font_size_emu: pub_editor::LengthEmu::new(fallback_resource::FONT_SIZE_EMU),
        line_height_emu: pub_editor::LengthEmu::new(fallback_resource::LINE_HEIGHT_EMU),
        bytes: fallback_resource::bytes(),
    }
}

fn revision_id(editor: &EditorSession) -> String {
    editor.project().state_id_v1()
}

fn document_id(editor: &EditorSession) -> Result<String, String> {
    editor
        .project()
        .identity
        .map(|identity| identity.document_id)
        .ok_or_else(|| "current EditorProject has no durable document identity".to_owned())
}

fn candidate(story_id: StoryId, frame_id: NodeId) -> TextEntryCandidateV1 {
    TextEntryCandidateV1 {
        target_id: frame_id.as_canonical().to_string(),
        story_id: story_id.as_canonical().to_string(),
        frame_id: Some(frame_id.as_canonical().to_string()),
        capability: "editable".to_owned(),
        reason: None,
    }
}

fn build_layout(
    editor: &EditorSession,
    story_id: StoryId,
    revision: &str,
) -> Result<DesktopStoryLayoutV1, String> {
    build_current_story_layout_v1(editor, story_id, revision, &fallback_font_resource())
        .map_err(|error| error.to_string())
}

fn derive_domain(editor: &EditorSession, story_id: StoryId) -> Result<StoryEditDomainV1, String> {
    derive_editor_story_edit_domain_v1(editor, story_id, StoryProvenanceHintV1::ImportedAuto)
        .map_err(|error| error.to_string())
}

fn resolve_post_edit_stop(
    layout: &DesktopStoryLayoutV1,
    scalar_boundary: u32,
    preferred_frame_id: NodeId,
) -> Result<CaretStopV1, String> {
    match resolve_story_position_v1(
        &layout.caret_map,
        scalar_boundary,
        None,
        Some(&layout.layout_revision_id),
    ) {
        Ok(stop) => Ok(stop),
        Err(error) if error.code == "caret_affinity_required" => {
            let preferred_frame = preferred_frame_id.as_canonical().to_string();
            let mut candidates = layout.caret_map.caret_stops.iter().filter(|stop| {
                stop.scalar_boundary == scalar_boundary && stop.frame_id == preferred_frame
            });
            let first = candidates.next().cloned();
            if first.is_none() || candidates.next().is_some() {
                return Err(format!(
                    "post-edit caret affinity is ambiguous at scalar {scalar_boundary}: {error}"
                ));
            }
            let first = first.expect("checked above");
            resolve_story_position_v1(
                &layout.caret_map,
                scalar_boundary,
                Some(&first.stop_id),
                Some(&layout.layout_revision_id),
            )
            .map_err(|resolve_error| resolve_error.to_string())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn rebind_after_commit(
    editor: &EditorSession,
    mode: &mut DesktopTextMode,
    commit: &TextInputCommitV1,
) -> Result<(), String> {
    if commit.post_edit_selection.story_id != mode.story_id.as_canonical().to_string() {
        return Err("post-edit selection targets another Story".to_owned());
    }
    if commit.post_edit_selection.anchor_scalar != commit.post_edit_selection.focus_scalar {
        return Err("bounded Desktop V0 expects a collapsed post-edit caret".to_owned());
    }

    let revision = revision_id(editor);
    let domain = derive_domain(editor, mode.story_id)?;
    let interaction_domain = to_interaction_domain_v1(&domain);
    let layout = build_layout(editor, mode.story_id, &revision)?;
    let stop = resolve_post_edit_stop(
        &layout,
        commit.post_edit_selection.focus_scalar,
        mode.frame_id,
    )?;
    if stop.frame_id != mode.frame_id.as_canonical().to_string() {
        return Err(
            "post-edit caret moved outside the current bounded TextFrame; linked-flow handoff is not admitted in Desktop V0"
                .to_owned(),
        );
    }

    let selection = TextSelectionStateV1 {
        protocol_version: SELECTION_VERSION_V1.to_owned(),
        story_id: interaction_domain.story_id.clone(),
        anchor_scalar: stop.scalar_boundary,
        focus_scalar: stop.scalar_boundary,
        revision_id: revision.clone(),
        edit_domain_id: interaction_domain.domain_id.clone(),
        projection_state: "projected".to_owned(),
        layout_revision_id: Some(layout.layout_revision_id.clone()),
        anchor_visual_stop_id: Some(stop.stop_id.clone()),
        focus_visual_stop_id: Some(stop.stop_id),
        preferred_inline_x_emu: None,
    };
    let rebound = rebind_text_edit_session_authority_v1(
        &mode.session,
        &revision,
        &interaction_domain,
        &layout.caret_map,
        &layout.layout_revision_id,
        selection,
    )
    .map_err(|error| error.to_string())?;

    mode.domain = domain;
    mode.layout = layout;
    mode.session = rebound.session;
    Ok(())
}

pub fn enter_explicit_text_mode(
    editor: &EditorSession,
    story_id: StoryId,
    frame_id: NodeId,
) -> Result<DesktopTextMode, String> {
    editor
        .can_replace_story_text(story_id)
        .map_err(|error| error.to_string())?;
    let revision = revision_id(editor);
    let domain = derive_domain(editor, story_id)?;
    let interaction_domain = to_interaction_domain_v1(&domain);
    let layout = build_layout(editor, story_id, &revision)?;
    let document_id = document_id(editor)?;
    let session_id = format!("chaptera.desktop.text-session:{document_id}");
    let activation = activate_explicit_edit_text_v1(
        &session_id,
        &document_id,
        &revision,
        &candidate(story_id, frame_id),
        &interaction_domain,
        &layout.caret_map,
        &layout.layout_revision_id,
        None,
        None,
    )
    .map_err(|error| error.to_string())?;
    let session = activation.active_session.ok_or_else(|| {
        activation
            .reason
            .unwrap_or_else(|| "text activation produced no session".to_owned())
    })?;

    Ok(DesktopTextMode {
        story_id,
        frame_id,
        domain,
        layout,
        session,
    })
}

pub fn replace_external_text(
    editor: &mut EditorSession,
    mode: &mut DesktopTextMode,
    text: &str,
) -> Result<(), String> {
    let commit = replace_selection_with_external_text_v1(
        editor,
        mode.story_id,
        &mode.domain,
        &mode.session.selection,
        &mode.session.revision_id,
        text,
    )
    .map_err(|error| error.to_string())?;
    rebind_after_commit(editor, mode, &commit)
}

pub fn apply_keyboard_command(
    editor: &mut EditorSession,
    mode: &mut DesktopTextMode,
    command: KeyboardCommandV1,
) -> Result<(), String> {
    let result = apply_keyboard_command_and_lower_v1(
        editor,
        mode.story_id,
        &mode.domain,
        &mode.session.selection,
        &mode.layout.caret_map,
        &mode.session.revision_id,
        command,
    )
    .map_err(|error| error.to_string())?;

    if let Some(commit) = result.commit {
        rebind_after_commit(editor, mode, &commit)?;
    } else if let Some(selection) = result.decision.selection {
        mode.session.selection = selection;
    }
    Ok(())
}

pub fn reposition_pointer(
    mode: &mut DesktopTextMode,
    page_id: &str,
    page_x_emu: i64,
    page_y_emu: i64,
) -> Result<(), String> {
    let interaction_domain = to_interaction_domain_v1(&mode.domain);
    let activation = activate_pointer_text_v1(
        &candidate(mode.story_id, mode.frame_id),
        &mode.session.revision_id,
        &interaction_domain,
        &mode.layout.caret_map,
        &mode.layout.layout_revision_id,
        &TextPointerTargetV1 {
            page_id: page_id.to_owned(),
            page_x_emu,
            page_y_emu,
        },
        Some(&mode.session),
        true,
        1,
        &mode.session.session_id,
        &mode.session.document_id,
        None,
    )
    .map_err(|error| error.to_string())?;
    if let Some(session) = activation.active_session {
        mode.session = session;
        Ok(())
    } else {
        Err(activation
            .reason
            .unwrap_or_else(|| "pointer text activation produced no active session".to_owned()))
    }
}

pub fn exit_text_mode(mode: &DesktopTextMode, trigger: &str) -> Result<(), String> {
    let result =
        exit_desktop_text_mode_v1(Some(&mode.session), trigger, "story_text", None, &[], false)
            .map_err(|error| error.to_string())?;
    if result.active_session.is_none() {
        Ok(())
    } else {
        Err(result
            .reason
            .unwrap_or_else(|| "text session did not exit".to_owned()))
    }
}

pub fn focus_caret(mode: &DesktopTextMode) -> Option<&CaretStopV1> {
    let stop_id = mode.session.selection.focus_visual_stop_id.as_deref()?;
    mode.layout
        .caret_map
        .caret_stops
        .iter()
        .find(|stop| stop.stop_id == stop_id)
}


#[cfg(test)]
mod tests {
    use super::*;
    use pub_editor::{EditOperation, Sha256Digest, open_mature_0x2c_editor};
    use sha2::{Digest, Sha256};
    use std::{env, fs};

    #[test]
    fn real_sample_newsletter_direct_text_session_enters_types_rebinds_and_exits() {
        let Some(path) = env::var_os("CHAPTERA_SAMPLE_NEWSLETTER") else {
            eprintln!(
                "CHAPTERA_SAMPLE_NEWSLETTER not set; dedicated direct-text gate owns real evidence"
            );
            return;
        };

        let original = fs::read(&path).expect("read pinned SampleNewsletter");
        let digest = Sha256::digest(&original);
        let mut digest_bytes = [0_u8; 32];
        digest_bytes.copy_from_slice(&digest);
        let source_hash = Sha256Digest::from_bytes(digest_bytes);
        let mut editor =
            open_mature_0x2c_editor(&original, source_hash).expect("open real SampleNewsletter");
        let visual = pub_viewer::open_mature_0x2c_geometry(
            &original,
            pub_viewer::viewer_geometry_environment_v0_1(),
        )
        .expect("open real SampleNewsletter Viewer geometry");

        let (story_id, frame_id) = visual
            .text_fragments
            .iter()
            .find_map(|fragment| {
                editor
                    .can_replace_story_text(fragment.story_id)
                    .ok()
                    .and_then(|_| {
                        enter_explicit_text_mode(&editor, fragment.story_id, fragment.frame_id)
                            .ok()
                            .map(|_| (fragment.story_id, fragment.frame_id))
                    })
            })
            .expect("real fixture should expose one capability-safe placed TextFrame");

        let operation_count_before = editor.operations().len();
        let mut mode =
            enter_explicit_text_mode(&editor, story_id, frame_id).expect("enter direct text mode");
        assert_eq!(
            editor.operations().len(),
            operation_count_before,
            "entering text mode must not create a document revision"
        );
        assert!(focus_caret(&mode).is_some());

        replace_external_text(&mut editor, &mut mode, "X")
            .expect("type one scalar through canonical Story range authority");
        assert_eq!(editor.operations().len(), operation_count_before + 1);
        assert!(matches!(
            editor.operations().last(),
            Some(EditOperation::ReplaceStoryRange { .. })
        ));
        assert_eq!(mode.session.revision_id, editor.project().state_id_v1());
        assert_eq!(
            mode.layout.layout_revision_id,
            editor.project().state_id_v1(),
            "caret layout must be rebound to the current authoring revision"
        );
        assert!(focus_caret(&mode).is_some());

        exit_text_mode(&mode, "escape").expect("Escape exits direct text mode");
        assert_eq!(
            editor.operations().len(),
            operation_count_before + 1,
            "exit must not create another document revision"
        );
        assert_eq!(editor.source_hash(), source_hash);
        assert_eq!(
            fs::read(path).expect("re-read source PUB"),
            original,
            "direct text editing must not mutate source PUB bytes"
        );
    }
}
