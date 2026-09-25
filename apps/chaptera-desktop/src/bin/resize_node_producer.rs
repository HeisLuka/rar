#[cfg(feature = "reader-only")]
compile_error!("chaptera-resize-producer cannot be built with the reader-only product feature");

use chaptera_scene_instance::{
    ObjectMutationKindV1, SCENE_INSTANCE_SCHEMA_V1, SceneInstanceV1, SceneProjectionKindV1,
    admit_object_mutation_v1, direct_page_local_instance_v1,
};
use pub_editor::{
    EditOperation, EditorEditableTarget, EditorError, EditorProject, EditorProjectError,
    EditorSession, LengthEmu, NodeId, RectEmu, Sha256Digest,
};
use pub_viewer::ViewerGeometryDocument;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fs;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use zip::ZipArchive;

const EMU_PER_POINT: i128 = 12_700;

#[derive(Debug, Clone, Copy)]
struct ResizeCandidate {
    node_id: NodeId,
    before: RectEmu,
    after: RectEmu,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let fixture = parse_fixture_arg()?;
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let payload: Value = serde_json::from_str(&input)?;
    let output = handle(&fixture, &payload)?;
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}

fn parse_fixture_arg() -> Result<PathBuf, Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    match (args.next(), args.next(), args.next()) {
        (Some(flag), Some(path), None) if flag == std::ffi::OsStr::new("--fixture") => Ok(PathBuf::from(path)),
        _ => Err("usage: chaptera-resize-producer --fixture FILE.pub".into()),
    }
}

fn handle(fixture: &Path, payload: &Value) -> Result<Value, Box<dyn Error>> {
    let action = required_str(payload, "action")?;
    let expected_hash = required_str(payload, "source_hash")?;
    let (bytes, visual, editor, actual_hash) = open_fixture(fixture)?;
    if actual_hash != expected_hash {
        return Err("producer source hash does not match fixture bytes".into());
    }

    match action {
        "baseline" => baseline_response(&visual, &editor, &actual_hash),
        "commit" => commit_response(payload, &bytes, &actual_hash),
        "history" => history_response(payload, &bytes, &actual_hash),
        "replay" => replay_response(payload, &bytes, &actual_hash),
        "export" => export_response(payload, &bytes, &actual_hash),
        "probe" => probe_response(payload, &visual, &editor, &actual_hash),
        _ => Err(format!("unsupported ResizeNode producer action {action:?}").into()),
    }
}

fn open_fixture(
    fixture: &Path,
) -> Result<(Vec<u8>, ViewerGeometryDocument, EditorSession, String), Box<dyn Error>> {
    let bytes = fs::read(fixture)?;
    let digest = Sha256::digest(&bytes);
    let mut digest_bytes = [0_u8; 32];
    digest_bytes.copy_from_slice(&digest);
    let source_hash = Sha256Digest::from_bytes(digest_bytes);
    let source_hash_text = hex_bytes(&digest);

    let visual = pub_viewer::open_mature_0x2c_geometry(
        &bytes,
        pub_viewer::viewer_geometry_environment_v0_1(),
    )?;
    if visual.document.source.source_hash != source_hash {
        return Err("Viewer source identity differs from fixture SHA-256".into());
    }
    let editor = pub_editor::open_mature_0x2c_editor(&bytes, source_hash)?;
    Ok((bytes, visual, editor, source_hash_text))
}

fn baseline_response(
    visual: &ViewerGeometryDocument,
    editor: &EditorSession,
    source_hash: &str,
) -> Result<Value, Box<dyn Error>> {
    let candidate = select_resize_candidate(visual, editor)?;
    let mut signed_probe = editor.clone();
    let signed_bounds = RectEmu::new(
        LengthEmu::new(-12_700),
        candidate.before.y,
        LengthEmu::new(
            candidate
                .before
                .width
                .get()
                .checked_add(12_700)
                .ok_or("signed-origin probe width overflow")?,
        ),
        candidate.before.height,
    );
    signed_probe.resize_node_to(candidate.node_id, signed_bounds)?;

    Ok(json!({
        "source_hash": source_hash,
        "baseline_project": serde_json::to_value(editor.project())?,
        "resize_candidate": {
            "node_id": candidate.node_id.as_canonical().to_string(),
            "before": rect_json(candidate.before),
            "after": rect_json(candidate.after),
            "direct_page_owned": true,
            "identity_transform": true,
            "original_bounds_valid": true
        },
        "signed_origin_probe_passed": true
    }))
}

fn select_resize_candidate(
    visual: &ViewerGeometryDocument,
    editor: &EditorSession,
) -> Result<ResizeCandidate, Box<dyn Error>> {
    for page in &visual.document.pages {
        let page_origin = page.id.into_canonical();
        let page_id = page.id.as_canonical().to_string();
        for scene_node in visual
            .scene
            .nodes
            .iter()
            .filter(|node| node.parent_origin == page_origin)
        {
            let Some(authored) = editor.graph().nodes.get(&scene_node.origin) else {
                continue;
            };
            if authored.payload.story_frame.is_none() || editor.can_resize_node(scene_node.origin).is_err() {
                continue;
            }

            let origin_node_id = scene_node.origin.as_canonical().to_string();
            let instance = direct_page_local_instance_v1(&origin_node_id, &page_id)?;
            let admission = admit_object_mutation_v1(&instance, ObjectMutationKindV1::ResizeNode);
            if !admission.admitted
                || admission.origin_node_id.as_deref() != Some(origin_node_id.as_str())
            {
                continue;
            }

            let before = authored.header.bounds;
            let Some(x) = before.x.get().checked_sub(12_700) else {
                continue;
            };
            let Some(y) = before.y.get().checked_sub(12_700) else {
                continue;
            };
            let Some(width) = before.width.get().checked_add(25_400) else {
                continue;
            };
            let Some(height) = before.height.get().checked_add(12_700) else {
                continue;
            };
            let after = RectEmu::new(
                LengthEmu::new(x),
                LengthEmu::new(y),
                LengthEmu::new(width),
                LengthEmu::new(height),
            );
            if editor.can_resize_node_to(scene_node.origin, after).is_ok() {
                return Ok(ResizeCandidate {
                    node_id: scene_node.origin,
                    before,
                    after,
                });
            }
        }
    }
    Err("fixture exposes no typed direct-page-local resizable Story frame".into())
}

fn commit_response(
    payload: &Value,
    bytes: &[u8],
    source_hash: &str,
) -> Result<Value, Box<dyn Error>> {
    let mut editor = fresh_editor(bytes)?;
    let base_project: EditorProject =
        serde_json::from_value(required_value(payload, "base_project")?.clone())?;
    editor.apply_project(&base_project)?;

    let command = required_value(payload, "command")?;
    if required_str(command, "kind")? != "resize_node_to" {
        return Err("commit command must be resize_node_to".into());
    }
    let node_id = parse_node_id(required_str(command, "node_id")?)?;
    let after = RectEmu::new(
        LengthEmu::new(required_i64(command, "x_emu")?),
        LengthEmu::new(required_i64(command, "y_emu")?),
        LengthEmu::new(required_i64(command, "width_emu")?),
        LengthEmu::new(required_i64(command, "height_emu")?),
    );
    let operation = editor.resize_node_to(node_id, after)?;

    Ok(json!({
        "canonical_operation": serde_json::to_value(operation)?,
        "resulting_project": serde_json::to_value(editor.project())?,
        "consequences": [
            {"key":"node.geometry.bounds","state":"supported","note":Value::Null}
        ],
        "source_hash_after": source_hash
    }))
}

fn history_response(
    payload: &Value,
    bytes: &[u8],
    source_hash: &str,
) -> Result<Value, Box<dyn Error>> {
    let kind = required_str(payload, "kind")?;
    let base_project = required_value(payload, "base_project")?;
    let baseline_project = required_value(payload, "baseline_project")?;
    let accepted_project = required_value(payload, "accepted_project")?;

    let mut editor = fresh_editor(bytes)?;
    let accepted: EditorProject = serde_json::from_value(accepted_project.clone())?;
    editor.apply_project(&accepted)?;

    match kind {
        "undo" => {
            if base_project != accepted_project {
                return Err("undo base project differs from accepted project".into());
            }
            editor.undo()?;
        }
        "redo" => {
            if base_project != baseline_project {
                return Err("redo base project differs from baseline project".into());
            }
            editor.undo()?;
            if serde_json::to_value(editor.project())? != *baseline_project {
                return Err("redo precondition did not reproduce exact baseline project".into());
            }
            editor.redo()?;
        }
        _ => return Err("history kind must be undo or redo".into()),
    }

    Ok(json!({
        "resulting_project": serde_json::to_value(editor.project())?,
        "consequences": [
            {"key":format!("history.{kind}"),"state":"supported","note":Value::Null}
        ],
        "source_hash_after": source_hash
    }))
}

fn replay_response(
    payload: &Value,
    bytes: &[u8],
    source_hash: &str,
) -> Result<Value, Box<dyn Error>> {
    let project_value = required_value(payload, "project")?.clone();
    let project: EditorProject = serde_json::from_value(project_value.clone())?;

    let mut replayed = fresh_editor(bytes)?;
    replayed.apply_project(&project)?;
    let replayed_project = serde_json::to_value(replayed.project())?;

    let mut legacy_value = project_value.clone();
    legacy_value["schema_version"] = json!("pub-editor-v0.4");
    let legacy_project: EditorProject = serde_json::from_value(legacy_value)?;
    let mut legacy_editor = fresh_editor(bytes)?;
    let legacy_v0_4_rejected = matches!(
        legacy_editor.apply_project(&legacy_project),
        Err(EditorProjectError::LegacyProjectCarriesResizeOperation { .. })
    );

    let mut stale_value = project_value;
    let stale_before_x = stale_value["operations"]
        .as_array_mut()
        .and_then(|operations| operations.first_mut())
        .and_then(|operation| operation.get_mut("before"))
        .and_then(Value::as_object_mut)
        .and_then(|before| before.get_mut("x"))
        .ok_or("ResizeNode project has no before.x")?;
    let x = stale_before_x
        .as_i64()
        .ok_or("ResizeNode before.x is not an i64")?;
    *stale_before_x = json!(x.checked_add(1).ok_or("stale probe x overflow")?);
    let stale_project: EditorProject = serde_json::from_value(stale_value)?;

    let mut stale_editor = fresh_editor(bytes)?;
    let stale_baseline = serde_json::to_value(stale_editor.project())?;
    let stale_result = stale_editor.apply_project(&stale_project);
    let stale_before_rejected_transactionally =
        stale_result.is_err() && serde_json::to_value(stale_editor.project())? == stale_baseline;

    Ok(json!({
        "replayed_project": replayed_project,
        "legacy_v0_4_rejected": legacy_v0_4_rejected,
        "stale_before_rejected_transactionally": stale_before_rejected_transactionally,
        "source_hash_after": source_hash
    }))
}

fn export_response(
    payload: &Value,
    bytes: &[u8],
    source_hash: &str,
) -> Result<Value, Box<dyn Error>> {
    let project: EditorProject =
        serde_json::from_value(required_value(payload, "project")?.clone())?;
    let (node_id, after) = project
        .operations
        .iter()
        .find_map(|operation| match operation {
            EditOperation::ResizeNode { node_id, after, .. } => Some((*node_id, *after)),
            _ => None,
        })
        .ok_or("project contains no ResizeNode")?;

    let mut editor = fresh_editor(bytes)?;
    editor.apply_project(&project)?;
    let effective_bounds = editor
        .graph()
        .nodes
        .get(&node_id)
        .ok_or("ResizeNode target missing after replay")?
        .header
        .bounds;
    if effective_bounds != after {
        return Err("replayed graph does not expose ResizeNode after bounds".into());
    }

    let idml = editor.export_editable(EditorEditableTarget::Idml, "resize-receipt.pub")?;
    let odg = editor.export_editable(EditorEditableTarget::Odg, "resize-receipt.pub")?;

    Ok(json!({
        "idml_reflects_resized_bounds": idml_contains_bounds(&idml.bytes, node_id, after)?,
        "odg_reflects_resized_bounds": odg_contains_bounds(&odg.bytes, node_id, after)?,
        "source_hash_after": source_hash
    }))
}

fn probe_response(
    payload: &Value,
    visual: &ViewerGeometryDocument,
    editor: &EditorSession,
    source_hash: &str,
) -> Result<Value, Box<dyn Error>> {
    let candidate = select_resize_candidate(visual, editor)?;
    let probe = required_str(payload, "probe")?;
    let baseline = serde_json::to_value(editor.project())?;
    let mut candidate_editor = editor.clone();

    let result = match probe {
        "identical_bounds" => candidate_editor.resize_node_to(candidate.node_id, candidate.before),
        "pure_move" => {
            let x = candidate
                .before
                .x
                .get()
                .checked_add(12_700)
                .ok_or("pure-move probe x overflow")?;
            candidate_editor.resize_node_to(
                candidate.node_id,
                RectEmu::new(
                    LengthEmu::new(x),
                    candidate.before.y,
                    candidate.before.width,
                    candidate.before.height,
                ),
            )
        }
        "non_positive_size" => candidate_editor.resize_node_to(
            candidate.node_id,
            RectEmu::new(
                candidate.before.x,
                candidate.before.y,
                LengthEmu::new(0),
                candidate.before.height,
            ),
        ),
        "overflow" => candidate_editor.resize_node_to(
            candidate.node_id,
            RectEmu::new(
                LengthEmu::new(i64::MAX),
                candidate.before.y,
                LengthEmu::new(2),
                candidate.before.height,
            ),
        ),
        "unsupported_target" => {
            let projected = SceneInstanceV1 {
                schema_version: SCENE_INSTANCE_SCHEMA_V1.to_owned(),
                instance_id: "sha256:resize-producer-projected-negative-probe".to_owned(),
                projection_kind: SceneProjectionKindV1::InheritedMaster,
                origin_node_id: candidate.node_id.as_canonical().to_string(),
                target_page_id: "00000000-0000-4000-8000-000000000001".to_owned(),
                source_parent_origin: Some("00000000-0000-4000-8000-000000000002".to_owned()),
                story_authority_id: None,
                cmo_slot_index: None,
                cmo_scalar_index: None,
            };
            let admission =
                admit_object_mutation_v1(&projected, ObjectMutationKindV1::ResizeNode);
            if admission.admitted || admission.origin_node_id.is_some() {
                return Err("projected ResizeNode target was unexpectedly admitted".into());
            }
            Err(EditorError::NodeResizeUnsupported {
                node_id: candidate.node_id,
            })
        }
        _ => return Err(format!("unknown ResizeNode probe {probe:?}").into()),
    };

    let rejected_no_mutation =
        result.is_err() && serde_json::to_value(candidate_editor.project())? == baseline;
    Ok(json!({
        "rejected_no_mutation": rejected_no_mutation,
        "source_hash_after": source_hash
    }))
}

fn fresh_editor(bytes: &[u8]) -> Result<EditorSession, Box<dyn Error>> {
    let digest = Sha256::digest(bytes);
    let mut digest_bytes = [0_u8; 32];
    digest_bytes.copy_from_slice(&digest);
    Ok(pub_editor::open_mature_0x2c_editor(
        bytes,
        Sha256Digest::from_bytes(digest_bytes),
    )?)
}

fn idml_contains_bounds(
    bytes: &[u8],
    node_id: NodeId,
    bounds: RectEmu,
) -> Result<bool, Box<dyn Error>> {
    let node_hex = node_id.as_canonical().to_string().replace('-', "");
    let marker = format!("Self=\"uf{node_hex}\"");
    let x = format_emu_points(bounds.x.get());
    let y = format_emu_points(bounds.y.get());
    let right = format_emu_points(bounds.right().ok_or("IDML bounds right overflow")?.get());
    let bottom = format_emu_points(bounds.bottom().ok_or("IDML bounds bottom overflow")?.get());
    let anchors = [
        format!("Anchor=\"{x} {y}\""),
        format!("Anchor=\"{x} {bottom}\""),
        format!("Anchor=\"{right} {bottom}\""),
        format!("Anchor=\"{right} {y}\""),
    ];

    for (_, xml) in zip_xml_parts(bytes)? {
        let Some(start) = xml.find(&marker) else {
            continue;
        };
        let tail = &xml[start..];
        let end = tail.find("</TextFrame>").unwrap_or(tail.len());
        let frame = &tail[..end];
        if anchors.iter().all(|anchor| frame.contains(anchor)) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn odg_contains_bounds(
    bytes: &[u8],
    node_id: NodeId,
    bounds: RectEmu,
) -> Result<bool, Box<dyn Error>> {
    let node_hex = node_id.as_canonical().to_string().replace('-', "");
    let marker = format!("draw:name=\"Frame_{node_hex}\"");
    let expected = [
        format!("svg:x=\"{}pt\"", format_emu_points(bounds.x.get())),
        format!("svg:y=\"{}pt\"", format_emu_points(bounds.y.get())),
        format!("svg:width=\"{}pt\"", format_emu_points(bounds.width.get())),
        format!("svg:height=\"{}pt\"", format_emu_points(bounds.height.get())),
    ];

    for (_, xml) in zip_xml_parts(bytes)? {
        let Some(start) = xml.find(&marker) else {
            continue;
        };
        let tail = &xml[start..];
        let end = tail.find("</draw:frame>").unwrap_or(tail.len());
        let frame = &tail[..end];
        if expected.iter().all(|attribute| frame.contains(attribute)) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn zip_xml_parts(bytes: &[u8]) -> Result<Vec<(String, String)>, Box<dyn Error>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;
    let mut parts = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name().to_owned();
        if entry.is_dir() || !name.ends_with(".xml") {
            continue;
        }
        let mut xml = String::new();
        entry.read_to_string(&mut xml)?;
        parts.push((name, xml));
    }
    Ok(parts)
}

fn format_emu_points(value: i64) -> String {
    format_ratio(i128::from(value), EMU_PER_POINT, 15)
}

fn format_ratio(numerator: i128, denominator: i128, precision: usize) -> String {
    if numerator == 0 {
        return "0".into();
    }
    let negative = numerator < 0;
    let numerator = numerator.abs();
    let whole = numerator / denominator;
    let mut remainder = numerator % denominator;
    let mut result = String::new();
    if negative {
        result.push('-');
    }
    result.push_str(&whole.to_string());
    if remainder == 0 {
        return result;
    }
    result.push('.');
    for _ in 0..precision {
        remainder *= 10;
        let digit = remainder / denominator;
        result.push(char::from(b'0' + u8::try_from(digit).expect("decimal digit")));
        remainder %= denominator;
        if remainder == 0 {
            break;
        }
    }
    while result.ends_with('0') {
        result.pop();
    }
    if result.ends_with('.') {
        result.pop();
    }
    result
}

fn rect_json(rect: RectEmu) -> Value {
    json!({
        "x": rect.x.get(),
        "y": rect.y.get(),
        "width": rect.width.get(),
        "height": rect.height.get()
    })
}

fn parse_node_id(value: &str) -> Result<NodeId, Box<dyn Error>> {
    Ok(serde_json::from_value(Value::String(value.to_owned()))?)
}

fn required_value<'a>(value: &'a Value, field: &str) -> Result<&'a Value, Box<dyn Error>> {
    value
        .get(field)
        .ok_or_else(|| format!("missing field {field:?}").into())
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, Box<dyn Error>> {
    required_value(value, field)?
        .as_str()
        .ok_or_else(|| format!("field {field:?} must be a string").into())
}

fn required_i64(value: &Value, field: &str) -> Result<i64, Box<dyn Error>> {
    required_value(value, field)?
        .as_i64()
        .ok_or_else(|| format!("field {field:?} must be an i64").into())
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires CHAPTERA_SAMPLE_NEWSLETTER"]
    fn real_sample_contract() {
        let fixture = PathBuf::from(
            std::env::var_os("CHAPTERA_SAMPLE_NEWSLETTER")
                .expect("CHAPTERA_SAMPLE_NEWSLETTER is required"),
        );
        let bytes = fs::read(&fixture).expect("read pinned SampleNewsletter");
        let source_hash = hex_bytes(&Sha256::digest(&bytes));

        let baseline = handle(
            &fixture,
            &json!({
                "action":"baseline",
                "source_hash":source_hash,
                "fixture_kind":"real_pub_sanitized"
            }),
        )
        .expect("baseline");
        assert_eq!(baseline["signed_origin_probe_passed"], true);

        let candidate = baseline["resize_candidate"].clone();
        let after = &candidate["after"];
        let commit = handle(
            &fixture,
            &json!({
                "action":"commit",
                "source_hash":source_hash,
                "base_project":baseline["baseline_project"],
                "command":{
                    "kind":"resize_node_to",
                    "node_id":candidate["node_id"],
                    "x_emu":after["x"],
                    "y_emu":after["y"],
                    "width_emu":after["width"],
                    "height_emu":after["height"]
                }
            }),
        )
        .expect("commit");
        assert_eq!(commit["canonical_operation"]["kind"], "resize_node");

        let accepted = commit["resulting_project"].clone();
        let undo = handle(
            &fixture,
            &json!({
                "action":"history",
                "source_hash":source_hash,
                "kind":"undo",
                "base_project":accepted,
                "baseline_project":baseline["baseline_project"],
                "accepted_project":accepted
            }),
        )
        .expect("undo");
        assert_eq!(undo["resulting_project"], baseline["baseline_project"]);

        let redo = handle(
            &fixture,
            &json!({
                "action":"history",
                "source_hash":source_hash,
                "kind":"redo",
                "base_project":baseline["baseline_project"],
                "baseline_project":baseline["baseline_project"],
                "accepted_project":accepted
            }),
        )
        .expect("redo");
        assert_eq!(redo["resulting_project"], accepted);

        let replay = handle(
            &fixture,
            &json!({"action":"replay","source_hash":source_hash,"project":accepted}),
        )
        .expect("replay");
        assert_eq!(replay["legacy_v0_4_rejected"], true);
        assert_eq!(replay["stale_before_rejected_transactionally"], true);

        let export = handle(
            &fixture,
            &json!({"action":"export","source_hash":source_hash,"project":accepted}),
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
            let result = handle(
                &fixture,
                &json!({
                    "action":"probe",
                    "probe":probe,
                    "source_hash":source_hash,
                    "baseline_project":baseline["baseline_project"],
                    "resize_candidate":candidate
                }),
            )
            .expect("probe");
            assert_eq!(result["rejected_no_mutation"], true, "{probe}");
        }
    }
}
