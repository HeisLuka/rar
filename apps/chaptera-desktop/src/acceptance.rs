use chaptera_scene_instance::{
    GeometrySyncPolicyV1, ObjectMutationKindV1, SceneInstanceV1, SceneProjectionKindV1,
    admit_object_mutation_v1, direct_page_local_instance_v1, geometry_sync_policy_v1,
};
use pub_editor::{
    EditOperation, EditorEditableTarget, EditorProject, EditorSession, LengthEmu, NodeId,
    RectEmu, StoryId, story_state_id_v1,
};
use pub_interaction::{DocumentPoint, MoveTransaction};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::Path;

const PROTOCOL_VERSION: &str = "chaptera.editor-desktop-vertical-observation.v1";
const REPLACEMENT_WITNESS: &str = "ChapteraV0";

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn state_id(editor: &EditorSession) -> Result<String, String> {
    let bytes = serde_json::to_vec(&editor.project())
        .map_err(|error| format!("serialize effective editor state: {error}"))?;
    Ok(format!("sha256:{}", sha256_hex(&bytes)))
}

fn rect_json(rect: RectEmu) -> Value {
    json!({
        "x": rect.x.get(),
        "y": rect.y.get(),
        "width": rect.width.get(),
        "height": rect.height.get(),
    })
}

fn direct_instance(
    editor: &EditorSession,
    target_page_id: &str,
    node_id: NodeId,
) -> Option<SceneInstanceV1> {
    let authored = editor.graph().nodes.get(&node_id)?;
    if authored.header.parent_id.to_string() != target_page_id {
        return None;
    }
    direct_page_local_instance_v1(&node_id.as_canonical().to_string(), target_page_id).ok()
}

fn select_story_edit(editor: &EditorSession) -> Option<(StoryId, u32, String)> {
    for (story_id, story) in &editor.graph().stories {
        if editor.can_replace_story_text(*story_id).is_err() {
            continue;
        }
        let Some((index, ch)) = story
            .text
            .chars()
            .enumerate()
            .find(|(_, ch)| *ch != '\r')
        else {
            continue;
        };
        let Ok(start) = u32::try_from(index) else {
            continue;
        };
        return Some((*story_id, start, ch.to_string()));
    }
    None
}

fn select_move(
    editor: &EditorSession,
    visual: &pub_viewer::ViewerGeometryDocument,
) -> Option<(SceneInstanceV1, NodeId, RectEmu, MoveTransaction)> {
    const DELTAS: &[(i64, i64)] = &[
        (127_000, 254_000),
        (-127_000, 254_000),
        (127_000, -254_000),
        (-127_000, -254_000),
    ];

    for page in &visual.document.pages {
        let page_origin = page.id.into_canonical();
        let target_page_id = page.id.as_canonical().to_string();
        for scene_node in visual
            .scene
            .nodes
            .iter()
            .filter(|node| node.parent_origin == page_origin)
        {
            let Some(instance) = direct_instance(editor, &target_page_id, scene_node.origin) else {
                continue;
            };
            let admission = admit_object_mutation_v1(&instance, ObjectMutationKindV1::MoveNode);
            let origin_node_id = scene_node.origin.as_canonical().to_string();
            if !admission.admitted
                || admission.origin_node_id.as_deref() != Some(origin_node_id.as_str())
                || geometry_sync_policy_v1(&instance)
                    != GeometrySyncPolicyV1::ApplyAuthoredOriginGeometry
            {
                continue;
            }

            let Some(authored) = editor.graph().nodes.get(&scene_node.origin) else {
                continue;
            };
            let before = authored.header.bounds;
            for &(dx, dy) in DELTAS {
                let Ok(mut drag) = MoveTransaction::begin(
                    scene_node.origin,
                    before,
                    DocumentPoint::new(LengthEmu::ZERO, LengthEmu::ZERO),
                ) else {
                    continue;
                };
                let Ok(preview) = drag.update(DocumentPoint::new(
                    LengthEmu::new(dx),
                    LengthEmu::new(dy),
                )) else {
                    continue;
                };
                if editor
                    .can_move_node_to(scene_node.origin, preview.x, preview.y)
                    .is_ok()
                {
                    return Some((instance, scene_node.origin, before, drag));
                }
            }
        }
    }
    None
}

fn story_state(editor: &EditorSession, story_id: StoryId) -> Result<String, String> {
    let story = editor
        .graph()
        .stories
        .get(&story_id)
        .ok_or_else(|| "accepted Story disappeared from editor graph".to_owned())?;
    Ok(story_state_id_v1(story_id, &story.text))
}

fn explicit_loss_flags(report: &pub_export::ExportReport) -> Result<(u64, bool, bool), String> {
    let value = serde_json::to_value(report)
        .map_err(|error| format!("serialize export report for acceptance: {error}"))?;
    let counts = value
        .get("counts")
        .and_then(Value::as_object)
        .ok_or_else(|| "export report counts are unavailable".to_owned())?;
    let blocking = counts
        .get("blocking")
        .and_then(Value::as_u64)
        .ok_or_else(|| "export report blocking count is unavailable".to_owned())?;
    let approximated = counts
        .get("approximated")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let unsupported = counts
        .get("unsupported")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| "export report items are unavailable".to_owned())?;

    let explicit_approximated = items
        .iter()
        .filter(|item| item.get("disposition").and_then(Value::as_str) == Some("approximated"))
        .filter(|item| item.get("loss_kind").is_some() && item.get("code").is_some())
        .count() as u64;
    let explicit_unsupported = items
        .iter()
        .filter(|item| item.get("disposition").and_then(Value::as_str) == Some("unsupported"))
        .filter(|item| item.get("loss_kind").is_some() && item.get("code").is_some())
        .count() as u64;

    Ok((
        blocking,
        approximated == explicit_approximated,
        unsupported == explicit_unsupported,
    ))
}

fn export_target(path: &Path) -> Result<EditorEditableTarget, String> {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("idml") => Ok(EditorEditableTarget::Idml),
        Some("odg") => Ok(EditorEditableTarget::Odg),
        _ => Err("acceptance export path must end in .idml or .odg".to_owned()),
    }
}

fn projected_move_is_denied(direct: &SceneInstanceV1) -> bool {
    let projected = SceneInstanceV1 {
        schema_version: direct.schema_version.clone(),
        instance_id: direct.instance_id.clone(),
        projection_kind: SceneProjectionKindV1::InheritedMaster,
        origin_node_id: direct.origin_node_id.clone(),
        target_page_id: direct.target_page_id.clone(),
        source_parent_origin: direct.source_parent_origin.clone(),
        story_authority_id: None,
        cmo_slot_index: None,
        cmo_scalar_index: None,
    };
    let decision = admit_object_mutation_v1(&projected, ObjectMutationKindV1::MoveNode);
    !decision.admitted
        && decision.origin_node_id.is_none()
        && geometry_sync_policy_v1(&projected) == GeometrySyncPolicyV1::ReprojectFromContext
}

pub fn run(fixture: &Path, project_path: &Path, export_path: &Path) -> Result<Value, String> {
    if env::var("CHAPTERA_DESKTOP_ACCEPTANCE_V1").as_deref() != Ok("1") {
        return Err("CHAPTERA_DESKTOP_ACCEPTANCE_V1=1 is required".to_owned());
    }
    let rar_commit = env::var("CHAPTERA_RAR_COMMIT")
        .map_err(|_| "CHAPTERA_RAR_COMMIT is required".to_owned())?;
    let expected_source_hash = env::var("CHAPTERA_SOURCE_HASH")
        .map_err(|_| "CHAPTERA_SOURCE_HASH is required".to_owned())?;

    let source_before =
        fs::read(fixture).map_err(|error| format!("read {}: {error}", fixture.display()))?;
    if sha256_hex(&source_before) != expected_source_hash {
        return Err("fixture SHA-256 differs from CHAPTERA_SOURCE_HASH".to_owned());
    }

    let visual = pub_viewer::open_mature_0x2c_geometry(
        &source_before,
        pub_viewer::viewer_geometry_environment_v0_1(),
    )
    .map_err(|error| format!("open Viewer geometry: {error:#}"))?;
    if visual.document.source.source_hash.to_string() != expected_source_hash {
        return Err("Viewer source identity differs from acceptance binding".to_owned());
    }

    let source_hash = visual.document.source.source_hash;
    let mut editor = pub_editor::open_mature_0x2c_editor(&source_before, source_hash)
        .map_err(|error| format!("open EditorSession: {error}"))?;

    let (story_id, start_scalar, expected_before) =
        select_story_edit(&editor).ok_or_else(|| "no capability-approved ordinary Story".to_owned())?;
    let end_scalar = start_scalar
        .checked_add(1)
        .ok_or_else(|| "Story scalar range overflow".to_owned())?;
    let story_operation = editor
        .replace_story_range(
            story_id,
            start_scalar,
            end_scalar,
            expected_before,
            REPLACEMENT_WITNESS,
        )
        .map_err(|error| format!("replace Story range: {error}"))?;
    let (before_story_state_id, after_story_state_id) = match &story_operation {
        EditOperation::ReplaceStoryRange {
            before_story_state_id,
            after_story_state_id,
            ..
        } => (before_story_state_id.clone(), after_story_state_id.clone()),
        _ => return Err("EditorSession did not emit ReplaceStoryRange".to_owned()),
    };
    let after_story_state_id_effective = state_id(&editor)?;

    let operations_before_drag = editor.operations().len();
    let (instance, moved_node_id, before_rect, mut drag) =
        select_move(&editor, &visual).ok_or_else(|| "no admitted direct page-local MoveNode target".to_owned())?;
    let after_rect = drag.preview_bounds();
    if editor.operations().len() != operations_before_drag {
        return Err("transient drag emitted a durable editor operation".to_owned());
    }
    let move_admission = admit_object_mutation_v1(&instance, ObjectMutationKindV1::MoveNode);
    if !move_admission.admitted {
        return Err("selected SceneInstance lost MoveNode admission".to_owned());
    }
    editor
        .move_node_to(moved_node_id, after_rect.x, after_rect.y)
        .map_err(|error| format!("commit MoveNode: {error}"))?;
    if editor.operations().len() != operations_before_drag + 1 {
        return Err("drag release did not emit exactly one durable MoveNode".to_owned());
    }
    let after_move_state_id = state_id(&editor)?;
    let story_state_after_move = story_state(&editor, story_id)?;

    editor.undo().map_err(|error| format!("undo MoveNode: {error}"))?;
    let undo_state_id = state_id(&editor)?;
    let story_state_after_undo = story_state(&editor, story_id)?;

    editor.redo().map_err(|error| format!("redo MoveNode: {error}"))?;
    let redo_state_id = state_id(&editor)?;
    let story_state_after_redo = story_state(&editor, story_id)?;

    let project = editor.project();
    if project.operations.len() != 2 {
        return Err("Desktop V0 project does not contain exactly two operations".to_owned());
    }
    let project_bytes = serde_json::to_vec_pretty(&project)
        .map_err(|error| format!("serialize EditorProject: {error}"))?;
    fs::write(project_path, &project_bytes)
        .map_err(|error| format!("write {}: {error}", project_path.display()))?;

    let persisted: EditorProject = serde_json::from_slice(&project_bytes)
        .map_err(|error| format!("parse persisted EditorProject: {error}"))?;
    let mut reopened = pub_editor::open_mature_0x2c_editor(&source_before, source_hash)
        .map_err(|error| format!("open fresh EditorSession: {error}"))?;
    reopened
        .apply_project(&persisted)
        .map_err(|error| format!("replay persisted EditorProject: {error}"))?;
    let reopened_state_id = state_id(&reopened)?;
    let story_state_reopened = story_state(&reopened, story_id)?;

    let target = export_target(export_path)?;
    let preview = reopened
        .preview_editable_export(target, "desktop-v0.pub")
        .map_err(|error| format!("preview edited export: {error}"))?;
    let (blocking_loss_count, approximations_explicit, unsupported_explicit) =
        explicit_loss_flags(&preview.report)?;
    if !preview.report.can_serialize || blocking_loss_count != 0 {
        return Err("edited export is blocked by capability/loss preview".to_owned());
    }
    let export = reopened
        .export_editable(target, "desktop-v0.pub")
        .map_err(|error| format!("export edited package: {error}"))?;
    fs::write(export_path, &export.bytes)
        .map_err(|error| format!("write {}: {error}", export_path.display()))?;

    let reopened_story_matches =
        story_state_reopened == after_story_state_id;
    let moved_geometry_matches = reopened
        .graph()
        .nodes
        .get(&moved_node_id)
        .is_some_and(|node| node.header.bounds == after_rect);

    let source_after =
        fs::read(fixture).map_err(|error| format!("re-read {}: {error}", fixture.display()))?;
    if source_before != source_after || sha256_hex(&source_after) != expected_source_hash {
        return Err("source PUB changed during desktop acceptance".to_owned());
    }

    let projected_denied = projected_move_is_denied(&instance);
    if !projected_denied {
        return Err("projected SceneInstance unexpectedly admits MoveNode".to_owned());
    }

    Ok(json!({
        "protocol_version": PROTOCOL_VERSION,
        "source_hash": expected_source_hash,
        "rar_commit": rar_commit,
        "story_edit": {
            "story_id": story_id.as_canonical().to_string(),
            "operation_kind": "replace_story_range",
            "capability_admitted": true,
            "before_state_id": before_story_state_id,
            "after_state_id": after_story_state_id,
        },
        "object_move": {
            "instance_id": instance.instance_id,
            "projection_kind": "direct_page_local",
            "origin_node_id": moved_node_id.as_canonical().to_string(),
            "capability_admitted": move_admission.admitted,
            "geometry_sync_policy": "apply_authored_origin_geometry",
            "before": rect_json(before_rect),
            "after": rect_json(after_rect),
            "durable_move_count": 1,
            "transient_geometry_operation_count": 0,
        },
        "history": {
            "after_story_state_id": after_story_state_id_effective,
            "after_move_state_id": after_move_state_id,
            "undo_state_id": undo_state_id,
            "redo_state_id": redo_state_id,
            "reopened_state_id": reopened_state_id,
            "story_state_after_move": story_state_after_move,
            "story_state_after_undo": story_state_after_undo,
            "story_state_after_redo": story_state_after_redo,
            "story_state_reopened": story_state_reopened,
        },
        "capability_loss": {
            "observed_before_export": true,
            "blocking_loss_count": blocking_loss_count,
            "approximations_explicit": approximations_explicit,
            "unsupported_partial_semantics_explicit": unsupported_explicit,
        },
        "export": {
            "format": target.extension(),
            "edited_story_present": reopened_story_matches,
            "moved_geometry_present": moved_geometry_matches,
        },
        "invariants": {
            "native_pub_write_used": false,
            "no_hidden_network_upload": true,
            "direct_page_local_gate_used": move_admission.admitted,
            "projected_object_mutation_fails_closed": projected_denied,
            "reopen_used_fresh_session": true,
        },
    }))
}
