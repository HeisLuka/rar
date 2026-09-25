use chaptera_scene_instance::{
    ObjectMutationKindV1, SCENE_INSTANCE_SCHEMA_V1, SceneInstanceV1, SceneProjectionKindV1,
    admit_object_mutation_v1, direct_page_local_instance_v1,
};
use pub_editor::{
    EditOperation, EditorEditableTarget, EditorError, EditorProject, EditorSession, LengthEmu,
    NodeId, RectEmu, Sha256Digest,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read};
use std::path::Path;

#[derive(Debug)]
struct Fixture {
    bytes: Vec<u8>,
    source_hash: String,
    digest: Sha256Digest,
}

#[derive(Debug, Clone, Copy)]
struct ResizeCandidate {
    node_id: NodeId,
    before: RectEmu,
    after: RectEmu,
}

#[derive(Debug)]
struct CandidateContext {
    candidate: ResizeCandidate,
    page_id: String,
}

pub fn run_stdio(fixture: &Path) -> Result<(), String> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|_| "could not read ResizeNode producer request".to_owned())?;
    let request: Value =
        serde_json::from_str(&input).map_err(|_| "ResizeNode producer request is not JSON".to_owned())?;
    let response = handle_request(fixture, &request)?;
    let encoded = serde_json::to_string(&response)
        .map_err(|_| "could not serialize ResizeNode producer response".to_owned())?;
    println!("{encoded}");
    Ok(())
}

fn handle_request(fixture_path: &Path, request: &Value) -> Result<Value, String> {
    let expected_source_hash = request
        .get("source_hash")
        .and_then(Value::as_str)
        .ok_or_else(|| "ResizeNode producer request is missing source_hash".to_owned())?;
    let fixture = read_fixture(fixture_path, expected_source_hash)?;
    let action = request
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| "ResizeNode producer request is missing action".to_owned())?;

    let response = match action {
        "baseline" => baseline_response(&fixture)?,
        "commit" => commit_response(&fixture, request)?,
        "history" => history_response(&fixture, request)?,
        "replay" => replay_response(&fixture, request)?,
        "export" => export_response(&fixture, request)?,
        "probe" => probe_response(&fixture, request)?,
        _ => return Err("unsupported ResizeNode producer action".to_owned()),
    };

    ensure_fixture_unchanged(fixture_path, &fixture.source_hash)?;
    Ok(response)
}

fn read_fixture(path: &Path, expected_source_hash: &str) -> Result<Fixture, String> {
    if expected_source_hash.len() != 64
        || !expected_source_hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("ResizeNode producer source_hash must be lowercase SHA-256".to_owned());
    }

    let bytes = fs::read(path).map_err(|_| "could not read ResizeNode producer fixture".to_owned())?;
    let digest_bytes = Sha256::digest(&bytes);
    let source_hash = hex_lower(&digest_bytes);
    if source_hash != expected_source_hash {
        return Err("ResizeNode producer fixture identity mismatch".to_owned());
    }
    let mut raw = [0_u8; 32];
    raw.copy_from_slice(&digest_bytes);
    Ok(Fixture {
        bytes,
        source_hash,
        digest: Sha256Digest::from_bytes(raw),
    })
}

fn ensure_fixture_unchanged(path: &Path, expected_source_hash: &str) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|_| "could not re-read ResizeNode producer fixture".to_owned())?;
    if hex_lower(&Sha256::digest(&bytes)) != expected_source_hash {
        return Err("ResizeNode producer mutated immutable source fixture".to_owned());
    }
    Ok(())
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("write to String cannot fail");
    }
    encoded
}

fn fresh_editor(fixture: &Fixture) -> Result<EditorSession, String> {
    pub_editor::open_mature_0x2c_editor(&fixture.bytes, fixture.digest)
        .map_err(|_| "could not open ResizeNode producer fixture in current editor".to_owned())
}

fn select_candidate(editor: &EditorSession) -> Result<CandidateContext, String> {
    for (node_id, node) in &editor.graph().nodes {
        if node.payload.story_frame.is_none() || editor.can_resize_node(*node_id).is_err() {
            continue;
        }
        let Some(page_id) = editor
            .graph()
            .pages
            .keys()
            .find(|page_id| (**page_id).into_canonical() == node.header.parent_id)
        else {
            continue;
        };
        let page_id_text = page_id.as_canonical().to_string();
        let node_id_text = node_id.as_canonical().to_string();
        let Ok(instance) = direct_page_local_instance_v1(&node_id_text, &page_id_text) else {
            continue;
        };
        let admission = admit_object_mutation_v1(&instance, ObjectMutationKindV1::ResizeNode);
        if !admission.admitted || admission.origin_node_id.as_deref() != Some(node_id_text.as_str()) {
            continue;
        }

        let before = node.header.bounds;
        for delta in [12_700_i64, 1_i64, 127_000_i64] {
            let (Some(width), Some(height)) = (
                before.width.get().checked_add(delta),
                before.height.get().checked_add(delta),
            ) else {
                continue;
            };
            let after = RectEmu::new(
                LengthEmu::new(-delta),
                before.y,
                LengthEmu::new(width),
                LengthEmu::new(height),
            );
            if editor.can_resize_node_to(*node_id, after).is_ok() {
                return Ok(CandidateContext {
                    candidate: ResizeCandidate {
                        node_id: *node_id,
                        before,
                        after,
                    },
                    page_id: page_id_text,
                });
            }
        }
    }
    Err("real fixture exposes no direct page-local exportable ResizeNode target".to_owned())
}

fn baseline_response(fixture: &Fixture) -> Result<Value, String> {
    let editor = fresh_editor(fixture)?;
    let context = select_candidate(&editor)?;
    Ok(json!({
        "source_hash": fixture.source_hash,
        "baseline_project": editor.project(),
        "resize_candidate": {
            "node_id": context.candidate.node_id.as_canonical().to_string(),
            "before": rect_json(context.candidate.before),
            "after": rect_json(context.candidate.after),
            "direct_page_owned": true,
            "identity_transform": true,
            "original_bounds_valid": bounds_valid(context.candidate.before),
        },
        "signed_origin_probe_passed":
            (context.candidate.after.x.get() < 0 || context.candidate.after.y.get() < 0)
            && editor.can_resize_node_to(context.candidate.node_id, context.candidate.after).is_ok(),
    }))
}

fn commit_response(fixture: &Fixture, request: &Value) -> Result<Value, String> {
    let mut editor = fresh_editor(fixture)?;
    let base_project = project_field(request, "base_project")?;
    editor
        .apply_project(&base_project)
        .map_err(|_| "ResizeNode producer base project replay failed".to_owned())?;
    let context = select_candidate(&editor)?;

    let command = request
        .get("command")
        .and_then(Value::as_object)
        .ok_or_else(|| "ResizeNode commit request is missing command".to_owned())?;
    let candidate_node_id = context.candidate.node_id.as_canonical().to_string();
    if command.get("kind").and_then(Value::as_str) != Some("resize_node_to")
        || command.get("node_id").and_then(Value::as_str) != Some(candidate_node_id.as_str())
    {
        return Err("ResizeNode commit request targets a different canonical object".to_owned());
    }
    let after = RectEmu::new(
        LengthEmu::new(command_i64(command, "x_emu")?),
        LengthEmu::new(command_i64(command, "y_emu")?),
        LengthEmu::new(command_i64(command, "width_emu")?),
        LengthEmu::new(command_i64(command, "height_emu")?),
    );
    let operation = editor
        .resize_node_to(context.candidate.node_id, after)
        .map_err(|_| "current editor rejected ResizeNode commit".to_owned())?;
    Ok(json!({
        "canonical_operation": operation,
        "resulting_project": editor.project(),
        "consequences": [
            {"key": "node.geometry.bounds", "state": "supported", "note": null}
        ],
        "source_hash_after": fixture.source_hash,
    }))
}

fn history_response(fixture: &Fixture, request: &Value) -> Result<Value, String> {
    let kind = request
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| "ResizeNode history request is missing kind".to_owned())?;
    let baseline_project = project_field(request, "baseline_project")?;
    let accepted_project = project_field(request, "accepted_project")?;
    let mut editor = fresh_editor(fixture)?;
    editor
        .apply_project(&accepted_project)
        .map_err(|_| "ResizeNode accepted project replay failed".to_owned())?;

    match kind {
        "undo" => {
            editor
                .undo()
                .map_err(|_| "ResizeNode producer undo failed".to_owned())?;
            if editor.project() != baseline_project {
                return Err("ResizeNode producer undo did not restore baseline".to_owned());
            }
        }
        "redo" => {
            editor
                .undo()
                .map_err(|_| "ResizeNode producer redo setup failed".to_owned())?;
            editor
                .redo()
                .map_err(|_| "ResizeNode producer redo failed".to_owned())?;
            if editor.project() != accepted_project {
                return Err("ResizeNode producer redo did not restore accepted project".to_owned());
            }
        }
        _ => return Err("unsupported ResizeNode history transition".to_owned()),
    }

    Ok(json!({
        "resulting_project": editor.project(),
        "consequences": [
            {"key": format!("history.{kind}"), "state": "supported", "note": null}
        ],
        "source_hash_after": fixture.source_hash,
    }))
}

fn replay_response(fixture: &Fixture, request: &Value) -> Result<Value, String> {
    let project = project_field(request, "project")?;
    let mut replayed = fresh_editor(fixture)?;
    replayed
        .apply_project(&project)
        .map_err(|_| "fresh ResizeNode producer replay failed".to_owned())?;
    let replayed_project = replayed.project();

    let mut legacy = project.clone();
    legacy.schema_version = "pub-editor-v0.4".to_owned();
    let mut legacy_session = fresh_editor(fixture)?;
    let legacy_before = legacy_session.project();
    let legacy_v0_4_rejected =
        legacy_session.apply_project(&legacy).is_err() && legacy_session.project() == legacy_before;

    let mut stale = project.clone();
    let stale_operation = stale
        .operations
        .iter_mut()
        .find(|operation| matches!(operation, EditOperation::ResizeNode { .. }))
        .ok_or_else(|| "ResizeNode replay project contains no ResizeNode operation".to_owned())?;
    if let EditOperation::ResizeNode { before, .. } = stale_operation {
        let shifted_x = before
            .x
            .get()
            .checked_add(1)
            .or_else(|| before.x.get().checked_sub(1))
            .ok_or_else(|| "could not build stale ResizeNode probe".to_owned())?;
        *before = RectEmu::new(
            LengthEmu::new(shifted_x),
            before.y,
            before.width,
            before.height,
        );
    }
    let mut stale_session = fresh_editor(fixture)?;
    let stale_before = stale_session.project();
    let stale_before_rejected_transactionally =
        stale_session.apply_project(&stale).is_err() && stale_session.project() == stale_before;

    Ok(json!({
        "replayed_project": replayed_project,
        "legacy_v0_4_rejected": legacy_v0_4_rejected,
        "stale_before_rejected_transactionally": stale_before_rejected_transactionally,
        "source_hash_after": fixture.source_hash,
    }))
}

fn export_response(fixture: &Fixture, request: &Value) -> Result<Value, String> {
    let project = project_field(request, "project")?;
    let (node_id, expected_after) = resize_operation(&project)?;

    let baseline = fresh_editor(fixture)?;
    let baseline_idml = baseline
        .export_editable(EditorEditableTarget::Idml, "receipt.pub")
        .map_err(|_| "baseline IDML export failed".to_owned())?;
    let baseline_odg = baseline
        .export_editable(EditorEditableTarget::Odg, "receipt.pub")
        .map_err(|_| "baseline ODG export failed".to_owned())?;

    let mut resized = fresh_editor(fixture)?;
    resized
        .apply_project(&project)
        .map_err(|_| "resized project replay failed before export".to_owned())?;
    let graph_after = resized
        .graph()
        .nodes
        .get(&node_id)
        .map(|node| node.header.bounds)
        .ok_or_else(|| "resized export target disappeared".to_owned())?;
    if graph_after != expected_after {
        return Err("resized graph does not contain canonical after bounds".to_owned());
    }

    let idml = resized
        .export_editable(EditorEditableTarget::Idml, "receipt.pub")
        .map_err(|_| "resized IDML export failed".to_owned())?;
    let odg = resized
        .export_editable(EditorEditableTarget::Odg, "receipt.pub")
        .map_err(|_| "resized ODG export failed".to_owned())?;
    let idml_repeat = resized
        .export_editable(EditorEditableTarget::Idml, "receipt.pub")
        .map_err(|_| "repeat resized IDML export failed".to_owned())?;
    let odg_repeat = resized
        .export_editable(EditorEditableTarget::Odg, "receipt.pub")
        .map_err(|_| "repeat resized ODG export failed".to_owned())?;

    let idml_reflects_resized_bounds = idml.report.can_serialize
        && idml.bytes == idml_repeat.bytes
        && idml.bytes != baseline_idml.bytes;
    let odg_reflects_resized_bounds = odg.report.can_serialize
        && odg.bytes == odg_repeat.bytes
        && odg.bytes != baseline_odg.bytes;

    Ok(json!({
        "idml_reflects_resized_bounds": idml_reflects_resized_bounds,
        "odg_reflects_resized_bounds": odg_reflects_resized_bounds,
        "source_hash_after": fixture.source_hash,
    }))
}

fn probe_response(fixture: &Fixture, request: &Value) -> Result<Value, String> {
    let probe = request
        .get("probe")
        .and_then(Value::as_str)
        .ok_or_else(|| "ResizeNode probe request is missing probe".to_owned())?;
    let editor = fresh_editor(fixture)?;
    let baseline_project = project_field(request, "baseline_project")?;
    if editor.project() != baseline_project {
        return Err("ResizeNode probe baseline differs from current fixture".to_owned());
    }
    let context = select_candidate(&editor)?;
    verify_candidate_payload(request, context.candidate)?;

    let rejected = match probe {
        "identical_bounds" => matches!(
            editor.can_resize_node_to(context.candidate.node_id, context.candidate.before),
            Err(EditorError::NodeResizeNoChange { .. })
        ),
        "pure_move" => {
            let shifted_x = context
                .candidate
                .before
                .x
                .get()
                .checked_add(1)
                .or_else(|| context.candidate.before.x.get().checked_sub(1))
                .ok_or_else(|| "could not build pure-move ResizeNode probe".to_owned())?;
            let pure_move = RectEmu::new(
                LengthEmu::new(shifted_x),
                context.candidate.before.y,
                context.candidate.before.width,
                context.candidate.before.height,
            );
            matches!(
                editor.can_resize_node_to(context.candidate.node_id, pure_move),
                Err(EditorError::NodeResizeNoSizeChange { .. })
            )
        }
        "non_positive_size" => {
            let invalid = RectEmu::new(
                context.candidate.before.x,
                context.candidate.before.y,
                LengthEmu::new(0),
                context.candidate.before.height,
            );
            matches!(
                editor.can_resize_node_to(context.candidate.node_id, invalid),
                Err(EditorError::NodeResizeNonPositive { .. })
            )
        }
        "overflow" => {
            let invalid = RectEmu::new(
                LengthEmu::new(i64::MAX),
                context.candidate.before.y,
                context.candidate.before.width,
                context.candidate.before.height,
            );
            matches!(
                editor.can_resize_node_to(context.candidate.node_id, invalid),
                Err(EditorError::NodeResizeOverflow { .. })
            )
        }
        "unsupported_target" => {
            let projected = SceneInstanceV1 {
                schema_version: SCENE_INSTANCE_SCHEMA_V1.to_owned(),
                instance_id: "sha256:resize-producer-negative-probe".to_owned(),
                projection_kind: SceneProjectionKindV1::InheritedMaster,
                origin_node_id: context.candidate.node_id.as_canonical().to_string(),
                target_page_id: context.page_id.clone(),
                source_parent_origin: Some(context.page_id.clone()),
                story_authority_id: None,
                cmo_slot_index: None,
                cmo_scalar_index: None,
            };
            !admit_object_mutation_v1(&projected, ObjectMutationKindV1::ResizeNode).admitted
        }
        _ => return Err("unsupported ResizeNode negative probe".to_owned()),
    };

    Ok(json!({
        "rejected_no_mutation": rejected && editor.project() == baseline_project,
        "source_hash_after": fixture.source_hash,
    }))
}

fn project_field(request: &Value, key: &str) -> Result<EditorProject, String> {
    let value = request
        .get(key)
        .cloned()
        .ok_or_else(|| format!("ResizeNode producer request is missing {key}"))?;
    serde_json::from_value(value)
        .map_err(|_| format!("ResizeNode producer {key} is not a current EditorProject"))
}

fn resize_operation(project: &EditorProject) -> Result<(NodeId, RectEmu), String> {
    let mut resize = project.operations.iter().filter_map(|operation| match operation {
        EditOperation::ResizeNode { node_id, after, .. } => Some((*node_id, *after)),
        _ => None,
    });
    let first = resize
        .next()
        .ok_or_else(|| "ResizeNode project contains no resize operation".to_owned())?;
    if resize.next().is_some() {
        return Err("ResizeNode producer expects exactly one resize operation".to_owned());
    }
    Ok(first)
}

fn command_i64(command: &serde_json::Map<String, Value>, key: &str) -> Result<i64, String> {
    command
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("ResizeNode command is missing integer {key}"))
}

fn rect_json(rect: RectEmu) -> Value {
    json!({
        "x": rect.x.get(),
        "y": rect.y.get(),
        "width": rect.width.get(),
        "height": rect.height.get(),
    })
}

fn bounds_valid(rect: RectEmu) -> bool {
    rect.width.get() > 0
        && rect.height.get() > 0
        && rect.right().is_some()
        && rect.bottom().is_some()
}

fn verify_candidate_payload(request: &Value, candidate: ResizeCandidate) -> Result<(), String> {
    let value = request
        .get("resize_candidate")
        .ok_or_else(|| "ResizeNode probe is missing resize_candidate".to_owned())?;
    if value.get("node_id").and_then(Value::as_str)
        != Some(candidate.node_id.as_canonical().to_string().as_str())
        || value.get("before") != Some(&rect_json(candidate.before))
        || value.get("after") != Some(&rect_json(candidate.after))
    {
        return Err("ResizeNode probe candidate drifted from current fixture".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires pinned CHAPTERA_SAMPLE_NEWSLETTER runtime fixture"]
    fn real_fixture_resize_producer_actions_are_self_consistent() {
        let fixture = std::env::var_os("CHAPTERA_SAMPLE_NEWSLETTER")
            .map(std::path::PathBuf::from)
            .expect("CHAPTERA_SAMPLE_NEWSLETTER");
        let bytes = fs::read(&fixture).expect("fixture bytes");
        let source_hash = hex_lower(&Sha256::digest(&bytes));

        let baseline = handle_request(
            &fixture,
            &json!({"action": "baseline", "source_hash": source_hash, "fixture_kind": "real_pub_sanitized"}),
        )
        .expect("baseline");
        assert_eq!(baseline["signed_origin_probe_passed"], true);

        let candidate = baseline["resize_candidate"].clone();
        let after = &candidate["after"];
        let commit = handle_request(
            &fixture,
            &json!({
                "action": "commit",
                "source_hash": source_hash,
                "base_project": baseline["baseline_project"].clone(),
                "command": {
                    "kind": "resize_node_to",
                    "node_id": candidate["node_id"].clone(),
                    "x_emu": after["x"].clone(),
                    "y_emu": after["y"].clone(),
                    "width_emu": after["width"].clone(),
                    "height_emu": after["height"].clone(),
                }
            }),
        )
        .expect("commit");
        let accepted = commit["resulting_project"].clone();

        let undo = handle_request(
            &fixture,
            &json!({
                "action": "history",
                "source_hash": source_hash,
                "kind": "undo",
                "base_project": accepted.clone(),
                "baseline_project": baseline["baseline_project"].clone(),
                "accepted_project": accepted.clone(),
            }),
        )
        .expect("undo");
        assert_eq!(undo["resulting_project"], baseline["baseline_project"]);

        let redo = handle_request(
            &fixture,
            &json!({
                "action": "history",
                "source_hash": source_hash,
                "kind": "redo",
                "base_project": baseline["baseline_project"].clone(),
                "baseline_project": baseline["baseline_project"].clone(),
                "accepted_project": accepted.clone(),
            }),
        )
        .expect("redo");
        assert_eq!(redo["resulting_project"], accepted);

        let replay = handle_request(
            &fixture,
            &json!({"action": "replay", "source_hash": source_hash, "project": accepted.clone()}),
        )
        .expect("replay");
        assert_eq!(replay["legacy_v0_4_rejected"], true);
        assert_eq!(replay["stale_before_rejected_transactionally"], true);

        let export = handle_request(
            &fixture,
            &json!({"action": "export", "source_hash": source_hash, "project": accepted}),
        )
        .expect("export");
        assert_eq!(export["idml_reflects_resized_bounds"], true);
        assert_eq!(export["odg_reflects_resized_bounds"], true);

        for probe in [
            "identical_bounds",
            "pure_move",
            "non_positive_size",
            "overflow",
            "unsupported_target",
        ] {
            let result = handle_request(
                &fixture,
                &json!({
                    "action": "probe",
                    "probe": probe,
                    "source_hash": source_hash,
                    "baseline_project": baseline["baseline_project"].clone(),
                    "resize_candidate": candidate.clone(),
                }),
            )
            .expect("negative probe");
            assert_eq!(result["rejected_no_mutation"], true, "{probe}");
        }
    }
}
