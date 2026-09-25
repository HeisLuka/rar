use chaptera_scene_instance::{
    GeometrySyncPolicyV1, ObjectMutationKindV1, SceneInstanceV1, admit_object_mutation_v1,
    direct_page_local_instance_v1, geometry_sync_policy_v1,
};
use pub_editor::{
    EditOperation, EditorEditableTarget, EditorProject, EditorSession, LengthEmu, NodeId, RectEmu,
    StoryId,
};
use pub_viewer::{ViewerFidelityStatus, ViewerGeometryDocument};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

const PROTOCOL_VERSION: &str = "chaptera.agent-control.v1";

struct AgentSession {
    source_path: PathBuf,
    source_bytes: Vec<u8>,
    visual: ViewerGeometryDocument,
    editor: EditorSession,
    session_id: String,
}

pub fn run_stdio() -> Result<(), String> {
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    let mut server = AgentServer::default();

    for line in stdin.lock().lines() {
        let line = line.map_err(|error| format!("read agent command: {error}"))?;
        if line.trim().is_empty() {
            continue;
        }

        let responses = server.handle_line(&line);
        for response in responses {
            serde_json::to_writer(&mut stdout, &response)
                .map_err(|error| format!("serialize agent response: {error}"))?;
            stdout
                .write_all(b"\n")
                .map_err(|error| format!("write agent response: {error}"))?;
        }
        stdout
            .flush()
            .map_err(|error| format!("flush agent response: {error}"))?;

        if server.shutdown {
            break;
        }
    }

    Ok(())
}

#[derive(Default)]
struct AgentServer {
    session: Option<AgentSession>,
    trace_subscribed: bool,
    session_generation: u64,
    event_index: u64,
    shutdown: bool,
}

impl AgentServer {
    fn handle_line(&mut self, line: &str) -> Vec<Value> {
        let request = match serde_json::from_str::<Value>(line) {
            Ok(value) => value,
            Err(error) => {
                return vec![error_envelope(
                    None,
                    None,
                    None,
                    None,
                    "invalid_json",
                    &format!("request is not valid JSON: {error}"),
                )];
            }
        };

        let Some(object) = request.as_object() else {
            return vec![error_envelope(
                None,
                None,
                None,
                None,
                "invalid_request",
                "request must be a JSON object",
            )];
        };
        let request_id = object
            .get("request_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let Some(request_id_value) = request_id.as_deref() else {
            return vec![error_envelope(
                None,
                None,
                None,
                None,
                "missing_request_id",
                "request_id must be a non-empty string",
            )];
        };
        if request_id_value.is_empty() {
            return vec![error_envelope(
                request_id.as_deref(),
                None,
                None,
                None,
                "missing_request_id",
                "request_id must be a non-empty string",
            )];
        }
        let Some(command) = object.get("command").and_then(Value::as_str) else {
            return vec![error_envelope(
                request_id.as_deref(),
                None,
                None,
                None,
                "missing_command",
                "command must be a string",
            )];
        };

        let before = self.session.as_ref().and_then(session_state_summary);
        let handled = self.handle_command(command, object);
        let after = self.session.as_ref().and_then(session_state_summary);

        match handled {
            Ok((result, trace_kinds)) => {
                let mut responses = vec![success_envelope(
                    request_id_value,
                    command,
                    self.session.as_ref(),
                    result,
                )];
                if self.trace_subscribed {
                    for (kind, payload) in trace_kinds {
                        self.event_index = self.event_index.saturating_add(1);
                        responses.push(trace_envelope(
                            self.event_index,
                            request_id_value,
                            command,
                            kind,
                            self.session.as_ref(),
                            before.as_ref(),
                            after.as_ref(),
                            payload,
                        ));
                    }
                }
                responses
            }
            Err((code, message)) => vec![error_envelope(
                request_id.as_deref(),
                Some(command),
                self.session.as_ref().map(|session| session.session_id.as_str()),
                self.session
                    .as_ref()
                    .map(|session| session.visual.document.source.source_hash.to_string())
                    .as_deref(),
                code,
                &message,
            )],
        }
    }

    fn handle_command(
        &mut self,
        command: &str,
        object: &Map<String, Value>,
    ) -> Result<(Value, Vec<(&'static str, Value)>), (&'static str, String)> {
        match command {
            "protocol.describe" => Ok((
                json!({
                    "protocol_version": PROTOCOL_VERSION,
                    "transport": "ndjson_stdio",
                    "commands": [
                        "protocol.describe",
                        "open",
                        "document.describe",
                        "pages.list",
                        "stories.list",
                        "story.inspect",
                        "story.read_local",
                        "scene.instances.list",
                        "instance.inspect",
                        "capabilities.get",
                        "edit.apply",
                        "undo",
                        "redo",
                        "project.save",
                        "project.reopen",
                        "loss.preview",
                        "export",
                        "snapshot.get",
                        "trace.subscribe",
                        "diagnostics.deep",
                        "shutdown"
                    ],
                    "privacy_default": "source_free",
                    "native_pub_write": false,
                    "deep_diagnostics_provider": "operation_blast_radius_v1_local_receipt"
                }),
                vec![("observed", json!({"surface":"protocol"}))],
            )),
            "open" => {
                let path = required_string(object, "path")?;
                self.open_document(Path::new(path))?;
                Ok((
                    self.snapshot_payload()?,
                    vec![("opened", json!({"source_immutable": true}))],
                ))
            }
            "document.describe" => {
                let session = self.require_session()?;
                let fidelity = fidelity_label(session.visual.document.fidelity_status());
                Ok((
                    json!({
                        "format": session.visual.document.source.format,
                        "format_version": session.visual.document.source.format_version,
                        "source_hash": session.visual.document.source.source_hash.to_string(),
                        "byte_len": session.visual.document.source.byte_len,
                        "page_count": session.visual.document.pages.len(),
                        "story_count": session.editor.graph().stories.len(),
                        "scene_node_count": session.visual.scene.nodes.len(),
                        "direct_scene_instance_count": direct_instances(session, None).len(),
                        "fidelity": fidelity,
                        "diagnostic_codes": diagnostic_codes(&session.visual),
                        "engine_revision": session.visual.scene.environment.engine_revision,
                        "native_pub_write": false
                    }),
                    vec![("observed", json!({"surface":"document"}))],
                ))
            }
            "pages.list" => {
                let session = self.require_session()?;
                let pages = session
                    .visual
                    .document
                    .pages
                    .iter()
                    .map(|page| {
                        json!({
                            "index": page.index,
                            "page_id": page.id.as_canonical().to_string(),
                            "width_emu": page.width_emu,
                            "height_emu": page.height_emu
                        })
                    })
                    .collect::<Vec<_>>();
                Ok((json!({"pages":pages}), vec![("observed", json!({"surface":"pages"}))]))
            }
            "stories.list" => {
                let session = self.require_session()?;
                let stories = session
                    .editor
                    .graph()
                    .stories
                    .iter()
                    .map(|(story_id, story)| {
                        let capability = session.editor.can_replace_story_text(*story_id);
                        json!({
                            "story_id": story_id.as_canonical().to_string(),
                            "scalar_len": story.text.chars().count(),
                            "text_sha256": sha256_hex(story.text.as_bytes()),
                            "editable": capability.is_ok(),
                            "capability_reason": capability.err().map(|error| error.code().to_owned())
                        })
                    })
                    .collect::<Vec<_>>();
                Ok((json!({"stories":stories}), vec![("observed", json!({"surface":"stories"}))]))
            }
            "story.inspect" => {
                let story_id = parse_story_id(required_string(object, "story_id")?)?;
                let session = self.require_session()?;
                let story = session
                    .editor
                    .graph()
                    .stories
                    .get(&story_id)
                    .ok_or(("story_not_found", "story_id is not present in the current graph".to_owned()))?;
                let capability = session.editor.can_replace_story_text(story_id);
                let frame_count = session
                    .visual
                    .story_frames
                    .iter()
                    .filter(|frame| frame.story_id == story_id)
                    .count();
                Ok((
                    json!({
                        "story_id": story_id.as_canonical().to_string(),
                        "scalar_len": story.text.chars().count(),
                        "byte_len": story.text.len(),
                        "text_sha256": sha256_hex(story.text.as_bytes()),
                        "paragraph_count": story.paragraphs.len(),
                        "run_count": story.runs.len(),
                        "field_count": story.fields.len(),
                        "hyperlink_count": story.hyperlinks.len(),
                        "frame_count": frame_count,
                        "editable": capability.is_ok(),
                        "capability_reason": capability.err().map(|error| error.code().to_owned()),
                        "content": "redacted_use_story.read_local"
                    }),
                    vec![("observed", json!({"surface":"story","story_id":story_id.as_canonical().to_string()}))],
                ))
            }
            "story.read_local" => {
                if object.get("allow_content").and_then(Value::as_bool) != Some(true) {
                    return Err((
                        "content_consent_required",
                        "story.read_local requires allow_content=true".to_owned(),
                    ));
                }
                let story_id = parse_story_id(required_string(object, "story_id")?)?;
                let session = self.require_session()?;
                let story = session
                    .editor
                    .graph()
                    .stories
                    .get(&story_id)
                    .ok_or(("story_not_found", "story_id is not present in the current graph".to_owned()))?;
                Ok((
                    json!({
                        "story_id": story_id.as_canonical().to_string(),
                        "text": story.text,
                        "text_sha256": sha256_hex(story.text.as_bytes()),
                        "local_only": true
                    }),
                    vec![("content_read_local", json!({"story_id":story_id.as_canonical().to_string()}))],
                ))
            }
            "scene.instances.list" => {
                let page_id = object.get("page_id").and_then(Value::as_str);
                let session = self.require_session()?;
                let direct = direct_instances(session, page_id);
                let direct_origins = direct
                    .iter()
                    .filter_map(|item| item.get("origin_node_id").and_then(Value::as_str))
                    .collect::<std::collections::BTreeSet<_>>();
                let visual_nodes_in_scope = session
                    .visual
                    .scene
                    .nodes
                    .iter()
                    .filter(|node| {
                        page_id.is_none_or(|wanted| node.parent_origin.to_string() == wanted)
                    })
                    .count();
                let unresolved = visual_nodes_in_scope.saturating_sub(direct_origins.len());
                Ok((
                    json!({
                        "instances": direct,
                        "unclassified_or_projected_visual_node_count": unresolved,
                        "mutation_policy": "only_direct_page_local_instances_are_addressable_for_object_mutation"
                    }),
                    vec![("observed", json!({"surface":"scene_instances"}))],
                ))
            }
            "instance.inspect" => {
                let instance_id = required_string(object, "instance_id")?;
                let session = self.require_session()?;
                let Some(view) = find_direct_instance_view(session, instance_id) else {
                    return Err((
                        "instance_not_found_or_projected_read_only",
                        "instance is not an admitted direct_page_local SceneInstance".to_owned(),
                    ));
                };
                Ok((view, vec![("observed", json!({"surface":"instance","instance_id":instance_id}))]))
            }
            "capabilities.get" => {
                let target_kind = required_string(object, "target_kind")?;
                let target_id = required_string(object, "target_id")?;
                let session = self.require_session()?;
                let payload = match target_kind {
                    "story" => {
                        let story_id = parse_story_id(target_id)?;
                        match session.editor.can_replace_story_text(story_id) {
                            Ok(()) => json!({
                                "target_kind":"story",
                                "target_id":target_id,
                                "replace_story_range":{"admitted":true,"reason":"editor_story_capability"}
                            }),
                            Err(error) => json!({
                                "target_kind":"story",
                                "target_id":target_id,
                                "replace_story_range":{"admitted":false,"reason":error.code()}
                            }),
                        }
                    }
                    "instance" => {
                        if let Some((instance, node_id, bounds)) = find_direct_instance(session, target_id) {
                            let move_scene = admit_object_mutation_v1(&instance, ObjectMutationKindV1::MoveNode);
                            let move_editor = session.editor.can_move_node_to(node_id, bounds.x, bounds.y);
                            let replace_scene = admit_object_mutation_v1(&instance, ObjectMutationKindV1::ReplaceImage);
                            json!({
                                "target_kind":"instance",
                                "target_id":target_id,
                                "projection_kind":"direct_page_local",
                                "move_node":{
                                    "scene_admitted":move_scene.admitted,
                                    "editor_geometry_supported":move_editor.is_ok(),
                                    "reason":move_editor.err().map(|error| error.code().to_owned()).unwrap_or(move_scene.reason)
                                },
                                "replace_image":{
                                    "scene_admitted":replace_scene.admitted,
                                    "reason":"replacement_asset_required_for_editor_preflight"
                                }
                            })
                        } else {
                            json!({
                                "target_kind":"instance",
                                "target_id":target_id,
                                "move_node":{"scene_admitted":false,"reason":"unknown_or_projected_instance_read_only"},
                                "replace_image":{"scene_admitted":false,"reason":"unknown_or_projected_instance_read_only"}
                            })
                        }
                    }
                    _ => {
                        return Err((
                            "unsupported_target_kind",
                            "target_kind must be story or instance".to_owned(),
                        ))
                    }
                };
                Ok((payload, vec![("observed", json!({"surface":"capabilities","target_kind":target_kind}))]))
            }
            "edit.apply" => {
                let operation = object
                    .get("operation")
                    .and_then(Value::as_object)
                    .ok_or(("missing_operation", "operation must be an object".to_owned()))?;
                let kind = operation
                    .get("kind")
                    .and_then(Value::as_str)
                    .ok_or(("missing_operation_kind", "operation.kind must be a string".to_owned()))?;
                let intent = json!({"operation_kind":kind});
                let (payload, commit_payload) = match kind {
                    "replace_story_range" => self.apply_story_range(operation)?,
                    "move_node" => self.apply_move_node(operation)?,
                    _ => {
                        return Err((
                            "unsupported_operation",
                            "agent V1 supports replace_story_range and move_node".to_owned(),
                        ))
                    }
                };
                Ok((
                    payload,
                    vec![
                        ("intent", intent),
                        ("durable_commit", commit_payload.clone()),
                        ("projection_updated", commit_payload),
                    ],
                ))
            }
            "undo" => {
                let session = self.require_session_mut()?;
                let operation = session
                    .editor
                    .undo()
                    .map_err(|error| (error.code(), error.to_string()))?
                    .clone();
                Ok((
                    json!({"operation":operation_summary(&operation),"state":session_state_summary(session)}),
                    vec![("undo", json!({"operation_id":operation_id(&operation)}))],
                ))
            }
            "redo" => {
                let session = self.require_session_mut()?;
                let operation = session
                    .editor
                    .redo()
                    .map_err(|error| (error.code(), error.to_string()))?
                    .clone();
                Ok((
                    json!({"operation":operation_summary(&operation),"state":session_state_summary(session)}),
                    vec![("redo", json!({"operation_id":operation_id(&operation)}))],
                ))
            }
            "project.save" => {
                let path = required_string(object, "path")?;
                let session = self.require_session()?;
                verify_source_immutable(session)?;
                let bytes = serde_json::to_vec_pretty(&session.editor.project())
                    .map_err(|error| ("project_serialize_failed", error.to_string()))?;
                fs::write(path, &bytes)
                    .map_err(|error| ("project_write_failed", format!("write project: {error}")))?;
                verify_source_immutable(session)?;
                Ok((
                    json!({
                        "artifact":{"kind":"editor_project","sha256":sha256_hex(&bytes),"byte_len":bytes.len()},
                        "operation_count":session.editor.operations().len()
                    }),
                    vec![("project_persisted", json!({"sha256":sha256_hex(&bytes),"byte_len":bytes.len()}))],
                ))
            }
            "project.reopen" => {
                let path = required_string(object, "path")?;
                let session = self.require_session_mut()?;
                verify_source_immutable(session)?;
                let bytes = fs::read(path)
                    .map_err(|error| ("project_read_failed", format!("read project: {error}")))?;
                let project: EditorProject = serde_json::from_slice(&bytes)
                    .map_err(|error| ("project_parse_failed", error.to_string()))?;
                let source_hash = session.visual.document.source.source_hash;
                let mut reopened = pub_editor::open_mature_0x2c_editor(&session.source_bytes, source_hash)
                    .map_err(|error| ("editor_reopen_failed", error.to_string()))?;
                reopened
                    .apply_project(&project)
                    .map_err(|error| ("project_replay_failed", error.to_string()))?;
                session.editor = reopened;
                verify_source_immutable(session)?;
                Ok((
                    json!({
                        "artifact":{"kind":"editor_project","sha256":sha256_hex(&bytes),"byte_len":bytes.len()},
                        "state":session_state_summary(session),
                        "fresh_session":true
                    }),
                    vec![("fresh_reopen", json!({"project_sha256":sha256_hex(&bytes)}))],
                ))
            }
            "loss.preview" => {
                let target = parse_export_target(required_string(object, "target")?)?;
                let session = self.require_session()?;
                let preview = session
                    .editor
                    .preview_editable_export(target, "agent.pub")
                    .map_err(|error| ("loss_preview_failed", error.to_string()))?;
                let report = serde_json::to_value(&preview.report)
                    .map_err(|error| ("loss_preview_serialize_failed", error.to_string()))?;
                Ok((
                    json!({"target":target.extension(),"report":report}),
                    vec![("loss_previewed", json!({"target":target.extension()}))],
                ))
            }
            "export" => {
                let target = parse_export_target(required_string(object, "target")?)?;
                let path = required_string(object, "path")?;
                let session = self.require_session()?;
                verify_source_immutable(session)?;
                let preview = session
                    .editor
                    .preview_editable_export(target, "agent.pub")
                    .map_err(|error| ("loss_preview_failed", error.to_string()))?;
                if !preview.report.can_serialize {
                    return Err(("export_blocked", "editable export is blocked by loss preview".to_owned()));
                }
                let export = session
                    .editor
                    .export_editable(target, "agent.pub")
                    .map_err(|error| ("export_failed", error.to_string()))?;
                fs::write(path, &export.bytes)
                    .map_err(|error| ("export_write_failed", format!("write export: {error}")))?;
                verify_source_immutable(session)?;
                Ok((
                    json!({
                        "artifact":{
                            "kind":"editable_export",
                            "format":target.extension(),
                            "sha256":sha256_hex(&export.bytes),
                            "byte_len":export.bytes.len()
                        },
                        "loss_report":serde_json::to_value(&export.report).map_err(|error| ("export_report_serialize_failed", error.to_string()))?
                    }),
                    vec![("exported", json!({"format":target.extension(),"sha256":sha256_hex(&export.bytes),"byte_len":export.bytes.len()}))],
                ))
            }
            "snapshot.get" => Ok((
                self.snapshot_payload()?,
                vec![("observed", json!({"surface":"snapshot"}))],
            )),
            "trace.subscribe" => {
                self.trace_subscribed = object
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                Ok((
                    json!({"subscribed":self.trace_subscribed}),
                    Vec::new(),
                ))
            }
            "diagnostics.deep" => {
                let Some(receipt_path) = object.get("receipt_path").and_then(Value::as_str) else {
                    return Ok((
                        json!({
                            "available":false,
                            "reason":"deep_diagnostics_receipt_not_loaded",
                            "provider":"operation_blast_radius_v1_local_receipt"
                        }),
                        vec![("observed", json!({"surface":"diagnostics_deep","available":false}))],
                    ));
                };
                if receipt_path.is_empty() {
                    return Err((
                        "invalid_argument",
                        "receipt_path must be a non-empty string".to_owned(),
                    ));
                }
                if object.get("allow_local_file").and_then(Value::as_bool) != Some(true) {
                    return Err((
                        "local_file_consent_required",
                        "diagnostics.deep requires allow_local_file=true to read a local diagnostic receipt"
                            .to_owned(),
                    ));
                }
                let session = self.require_session()?;
                let source_hash = session.visual.document.source.source_hash.to_string();
                let summary =
                    deep_diagnostics_summary(Path::new(receipt_path), &source_hash)?;
                Ok((
                    summary,
                    vec![("observed", json!({"surface":"diagnostics_deep","available":true}))],
                ))
            },
            "shutdown" => {
                self.shutdown = true;
                Ok((json!({"shutdown":true}), vec![("shutdown", json!({}))]))
            }
            _ => Err(("unknown_command", format!("unsupported command {command:?}"))),
        }
    }

    fn open_document(&mut self, path: &Path) -> Result<(), (&'static str, String)> {
        let source_bytes =
            fs::read(path).map_err(|error| ("source_read_failed", format!("read source: {error}")))?;
        let visual = pub_viewer::open_mature_0x2c_geometry(
            &source_bytes,
            pub_viewer::viewer_geometry_environment_v0_1(),
        )
        .map_err(|error| ("viewer_open_failed", error.to_string()))?;
        let source_hash = visual.document.source.source_hash;
        let editor = pub_editor::open_mature_0x2c_editor(&source_bytes, source_hash)
            .map_err(|error| ("editor_open_failed", error.to_string()))?;

        self.session_generation = self.session_generation.saturating_add(1);
        let session_id = format!(
            "sha256:{}",
            sha256_hex(
                format!(
                    "{PROTOCOL_VERSION}|{}|{}",
                    source_hash, self.session_generation
                )
                .as_bytes()
            )
        );
        let session = AgentSession {
            source_path: path.to_path_buf(),
            source_bytes,
            visual,
            editor,
            session_id,
        };
        verify_source_immutable(&session)?;
        self.session = Some(session);
        Ok(())
    }

    fn snapshot_payload(&self) -> Result<Value, (&'static str, String)> {
        let session = self.require_session()?;
        Ok(json!({
            "session_id":session.session_id,
            "source_hash":session.visual.document.source.source_hash.to_string(),
            "state":session_state_summary(session),
            "fidelity":fidelity_label(session.visual.document.fidelity_status()),
            "diagnostic_codes":diagnostic_codes(&session.visual),
            "counts":{
                "pages":session.visual.document.pages.len(),
                "stories":session.editor.graph().stories.len(),
                "scene_nodes":session.visual.scene.nodes.len(),
                "direct_scene_instances":direct_instances(session,None).len(),
                "operations":session.editor.operations().len()
            },
            "invariants":{
                "source_immutable":true,
                "native_pub_write":false,
                "projected_object_mutation":"fail_closed",
                "default_output":"source_free"
            }
        }))
    }

    fn apply_story_range(
        &mut self,
        operation: &Map<String, Value>,
    ) -> Result<(Value, Value), (&'static str, String)> {
        let story_id = parse_story_id(required_string(operation, "story_id")?)?;
        let start_scalar = required_u32(operation, "start_scalar")?;
        let end_scalar = required_u32(operation, "end_scalar")?;
        let expected_before = required_string(operation, "expected_before")?.to_owned();
        let replacement_text = required_string(operation, "replacement_text")?.to_owned();

        let session = self.require_session_mut()?;
        let before_state_id = state_id(&session.editor)?;
        let before_diagnostics = diagnostic_codes(&session.visual);
        let applied = session
            .editor
            .replace_story_range(
                story_id,
                start_scalar,
                end_scalar,
                expected_before,
                replacement_text,
            )
            .map_err(|error| (error.code(), error.to_string()))?;
        let after_state_id = state_id(&session.editor)?;
        let after_diagnostics = diagnostic_codes(&session.visual);
        let (added, cleared) = set_delta(&before_diagnostics, &after_diagnostics);
        let operation_id = operation_id(&applied);
        let summary = operation_summary(&applied);
        let commit = json!({
            "operation_id":operation_id,
            "operation":summary,
            "semantic_delta":{"story_changed":true,"geometry_changed":false},
            "scene_delta":{"geometry_changed":false,"projection_status":"semantic_text_state_updated"},
            "diagnostics_delta":{"added":added,"cleared":cleared},
            "before_state_id":before_state_id,
            "after_state_id":after_state_id
        });
        Ok((
            json!({
                "accepted":true,
                "operation_id":operation_id,
                "operation":summary,
                "before_state_id":before_state_id,
                "after_state_id":after_state_id,
                "diagnostics_delta":{"added":added,"cleared":cleared}
            }),
            commit,
        ))
    }

    fn apply_move_node(
        &mut self,
        operation: &Map<String, Value>,
    ) -> Result<(Value, Value), (&'static str, String)> {
        let instance_id = required_string(operation, "instance_id")?.to_owned();
        let x = required_i64(operation, "x")?;
        let y = required_i64(operation, "y")?;

        let session = self.require_session_mut()?;
        let Some((instance, node_id, before_bounds)) = find_direct_instance(session, &instance_id)
        else {
            return Err((
                "projected_or_unknown_instance_read_only",
                "MoveNode requires an admitted direct_page_local SceneInstance".to_owned(),
            ));
        };
        let admission = admit_object_mutation_v1(&instance, ObjectMutationKindV1::MoveNode);
        if !admission.admitted
            || geometry_sync_policy_v1(&instance) != GeometrySyncPolicyV1::ApplyAuthoredOriginGeometry
        {
            return Err((
                "scene_instance_mutation_denied",
                admission.reason,
            ));
        }

        let before_state_id = state_id(&session.editor)?;
        let applied = session
            .editor
            .move_node_to(node_id, LengthEmu::new(x), LengthEmu::new(y))
            .map_err(|error| (error.code(), error.to_string()))?;
        let after_state_id = state_id(&session.editor)?;
        let after_bounds = session
            .editor
            .graph()
            .nodes
            .get(&node_id)
            .map(|node| node.header.bounds)
            .ok_or(("node_missing_after_move", "moved node disappeared from graph".to_owned()))?;
        let operation_id = operation_id(&applied);
        let summary = operation_summary(&applied);
        let commit = json!({
            "operation_id":operation_id,
            "operation":summary,
            "semantic_delta":{"story_changed":false,"geometry_changed":true},
            "scene_delta":{
                "geometry_changed":true,
                "instance_id":instance.instance_id,
                "origin_node_id":node_id.as_canonical().to_string(),
                "before":rect_json(before_bounds),
                "after":rect_json(after_bounds),
                "geometry_sync_policy":"apply_authored_origin_geometry"
            },
            "diagnostics_delta":{"added":[],"cleared":[]},
            "before_state_id":before_state_id,
            "after_state_id":after_state_id
        });
        Ok((
            json!({
                "accepted":true,
                "operation_id":operation_id,
                "operation":summary,
                "before_state_id":before_state_id,
                "after_state_id":after_state_id,
                "scene_delta":commit["scene_delta"]
            }),
            commit,
        ))
    }

    fn require_session(&self) -> Result<&AgentSession, (&'static str, String)> {
        self.session
            .as_ref()
            .ok_or(("no_open_document", "open a PUB before this command".to_owned()))
    }

    fn require_session_mut(&mut self) -> Result<&mut AgentSession, (&'static str, String)> {
        self.session
            .as_mut()
            .ok_or(("no_open_document", "open a PUB before this command".to_owned()))
    }
}

const BLAST_RADIUS_SCHEMA_VERSION: &str = "chaptera.operation-blast-radius.v1";

fn deep_diagnostics_summary(
    path: &Path,
    current_source_hash: &str,
) -> Result<Value, (&'static str, String)> {
    let bytes = fs::read(path)
        .map_err(|error| ("deep_diagnostics_read_failed", format!("read diagnostic receipt: {error}")))?;
    let value = serde_json::from_slice::<Value>(&bytes)
        .map_err(|error| ("deep_diagnostics_invalid_json", error.to_string()))?;
    let root = value
        .as_object()
        .ok_or(("deep_diagnostics_invalid_receipt", "receipt must be a JSON object".to_owned()))?;

    require_exact_object_keys(
        root,
        &[
            "schema_version",
            "operation",
            "artifacts",
            "cfb",
            "parsed_record_family_delta",
            "semantic_graph_delta",
            "parser_outcomes",
            "second_save_convergence",
            "classification_counts",
            "invariants",
        ],
        "receipt",
    )?;

    if root.get("schema_version").and_then(Value::as_str) != Some(BLAST_RADIUS_SCHEMA_VERSION) {
        return Err((
            "deep_diagnostics_schema_mismatch",
            "receipt is not OperationBlastRadiusV1".to_owned(),
        ));
    }

    let artifacts = required_object_value(root, "artifacts", "receipt")?;
    require_exact_object_keys(artifacts, &["source", "control", "mutation"], "artifacts")?;
    let source = diagnostic_artifact_summary(
        required_object_value(artifacts, "source", "artifacts")?,
        "source",
    )?;
    let control = diagnostic_artifact_summary(
        required_object_value(artifacts, "control", "artifacts")?,
        "control",
    )?;
    let mutation = diagnostic_artifact_summary(
        required_object_value(artifacts, "mutation", "artifacts")?,
        "mutation",
    )?;

    let receipt_source_hash = source
        .get("sha256")
        .and_then(Value::as_str)
        .expect("diagnostic_artifact_summary always returns sha256");
    if receipt_source_hash != current_source_hash {
        return Err((
            "deep_diagnostics_source_mismatch",
            "diagnostic receipt belongs to a different source PUB".to_owned(),
        ));
    }

    let invariants = required_object_value(root, "invariants", "receipt")?;
    require_exact_object_keys(
        invariants,
        &[
            "raw_byte_inequality_is_not_semantic_evidence",
            "matched_noop_control_used",
            "unexplained_collateral_preserved",
            "public_receipt_contains_raw_document_bytes",
            "native_pub_writer_capability_granted",
        ],
        "invariants",
    )?;
    require_bool(invariants, "matched_noop_control_used", true, "invariants")?;
    require_bool(
        invariants,
        "unexplained_collateral_preserved",
        true,
        "invariants",
    )?;
    require_bool(
        invariants,
        "public_receipt_contains_raw_document_bytes",
        false,
        "invariants",
    )?;
    require_bool(
        invariants,
        "native_pub_writer_capability_granted",
        false,
        "invariants",
    )?;

    let counts = diagnostic_classification_counts(
        required_object_value(root, "classification_counts", "receipt")?,
    )?;
    let cfb = required_object_value(root, "cfb", "receipt")?;
    require_exact_object_keys(
        cfb,
        &[
            "source_control_topology_delta",
            "control_mutation_topology_delta",
            "source_control_stream_delta",
            "control_mutation_stream_delta",
            "control_mutation_byte_ranges",
        ],
        "cfb",
    )?;

    let operation = required_object_value(root, "operation", "receipt")?;
    let operation_summary = allowlisted_object(
        operation,
        &["kind", "operation_id", "node_id"],
    );

    let parser_outcomes =
        diagnostic_parser_outcomes(required_object_value(root, "parser_outcomes", "receipt")?)?;
    let second_save = diagnostic_second_save_summary(
        required_object_value(root, "second_save_convergence", "receipt")?,
    )?;

    let parsed_record_family_delta = diagnostic_classified_array(
        root.get("parsed_record_family_delta")
            .ok_or(("deep_diagnostics_invalid_receipt", "missing parsed_record_family_delta".to_owned()))?,
        &["family", "id", "before_sha256", "after_sha256", "classification"],
        "parsed_record_family_delta",
    )?;
    let semantic_graph_delta = diagnostic_classified_array(
        root.get("semantic_graph_delta")
            .ok_or(("deep_diagnostics_invalid_receipt", "missing semantic_graph_delta".to_owned()))?,
        &["kind", "id", "before_sha256", "after_sha256", "classification"],
        "semantic_graph_delta",
    )?;

    let source_control_topology_delta = diagnostic_classified_array(
        cfb.get("source_control_topology_delta")
            .ok_or(("deep_diagnostics_invalid_receipt", "missing source_control_topology_delta".to_owned()))?,
        &["stream_id", "change", "classification"],
        "source_control_topology_delta",
    )?;
    let control_mutation_topology_delta = diagnostic_classified_array(
        cfb.get("control_mutation_topology_delta")
            .ok_or(("deep_diagnostics_invalid_receipt", "missing control_mutation_topology_delta".to_owned()))?,
        &["stream_id", "change", "classification"],
        "control_mutation_topology_delta",
    )?;
    let source_control_stream_delta = diagnostic_classified_array(
        cfb.get("source_control_stream_delta")
            .ok_or(("deep_diagnostics_invalid_receipt", "missing source_control_stream_delta".to_owned()))?,
        &[
            "stream_id",
            "before_sha256",
            "after_sha256",
            "before_size",
            "after_size",
            "classification",
        ],
        "source_control_stream_delta",
    )?;
    let control_mutation_stream_delta = diagnostic_classified_array(
        cfb.get("control_mutation_stream_delta")
            .ok_or(("deep_diagnostics_invalid_receipt", "missing control_mutation_stream_delta".to_owned()))?,
        &[
            "stream_id",
            "before_sha256",
            "after_sha256",
            "before_size",
            "after_size",
            "classification",
        ],
        "control_mutation_stream_delta",
    )?;
    let control_mutation_byte_ranges = diagnostic_classified_array(
        cfb.get("control_mutation_byte_ranges")
            .ok_or(("deep_diagnostics_invalid_receipt", "missing control_mutation_byte_ranges".to_owned()))?,
        &["offset", "length", "physical_label", "classification"],
        "control_mutation_byte_ranges",
    )?;

    Ok(json!({
        "available":true,
        "provider":"operation_blast_radius_v1_local_receipt",
        "receipt_sha256":sha256_hex(&bytes),
        "schema_version":BLAST_RADIUS_SCHEMA_VERSION,
        "source_hash":current_source_hash,
        "operation":operation_summary,
        "artifacts":{
            "source":source,
            "control":control,
            "mutation":mutation
        },
        "classification_counts":counts,
        "cfb":{
            "source_control_topology_delta":source_control_topology_delta,
            "control_mutation_topology_delta":control_mutation_topology_delta,
            "source_control_stream_delta":source_control_stream_delta,
            "control_mutation_stream_delta":control_mutation_stream_delta,
            "control_mutation_byte_ranges":control_mutation_byte_ranges
        },
        "parsed_record_family_delta":parsed_record_family_delta,
        "semantic_graph_delta":semantic_graph_delta,
        "parser_outcomes":parser_outcomes,
        "second_save_convergence":second_save,
        "invariants":{
            "source_bound":true,
            "matched_noop_control_used":true,
            "unexplained_collateral_preserved":true,
            "raw_document_content_emitted":false,
            "local_path_emitted":false,
            "native_pub_write":false
        }
    }))
}

fn required_object_value<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<&'a Map<String, Value>, (&'static str, String)> {
    object
        .get(field)
        .and_then(Value::as_object)
        .ok_or((
            "deep_diagnostics_invalid_receipt",
            format!("{label}.{field} must be an object"),
        ))
}

fn require_exact_object_keys(
    object: &Map<String, Value>,
    expected: &[&str],
    label: &str,
) -> Result<(), (&'static str, String)> {
    let actual = object.keys().map(String::as_str).collect::<std::collections::BTreeSet<_>>();
    let wanted = expected.iter().copied().collect::<std::collections::BTreeSet<_>>();
    if actual != wanted {
        return Err((
            "deep_diagnostics_invalid_receipt",
            format!("{label} fields do not match OperationBlastRadiusV1"),
        ));
    }
    Ok(())
}

fn require_bool(
    object: &Map<String, Value>,
    field: &str,
    expected: bool,
    label: &str,
) -> Result<(), (&'static str, String)> {
    if object.get(field).and_then(Value::as_bool) != Some(expected) {
        return Err((
            "deep_diagnostics_invalid_receipt",
            format!("{label}.{field} must be {expected}"),
        ));
    }
    Ok(())
}

fn diagnostic_artifact_summary(
    artifact: &Map<String, Value>,
    label: &str,
) -> Result<Value, (&'static str, String)> {
    let hash = artifact
        .get("sha256")
        .and_then(Value::as_str)
        .ok_or((
            "deep_diagnostics_invalid_receipt",
            format!("artifacts.{label}.sha256 missing"),
        ))?;
    if !is_sha256_hex(hash) {
        return Err((
            "deep_diagnostics_invalid_receipt",
            format!("artifacts.{label}.sha256 invalid"),
        ));
    }
    let byte_len = artifact
        .get("byte_len")
        .and_then(Value::as_u64)
        .ok_or((
            "deep_diagnostics_invalid_receipt",
            format!("artifacts.{label}.byte_len invalid"),
        ))?;
    Ok(json!({"sha256":hash,"byte_len":byte_len}))
}

fn diagnostic_classification_counts(
    object: &Map<String, Value>,
) -> Result<Value, (&'static str, String)> {
    let fields = [
        "requested_semantic",
        "save_normalization",
        "expected_derived",
        "unexplained_collateral",
        "unavailable",
    ];
    require_exact_object_keys(object, &fields, "classification_counts")?;
    let mut out = Map::new();
    for field in fields {
        let value = object.get(field).and_then(Value::as_u64).ok_or((
            "deep_diagnostics_invalid_receipt",
            format!("classification_counts.{field} must be a non-negative integer"),
        ))?;
        out.insert(field.to_owned(), Value::from(value));
    }
    Ok(Value::Object(out))
}

fn diagnostic_classified_array(
    value: &Value,
    allowlist: &[&str],
    label: &str,
) -> Result<Value, (&'static str, String)> {
    let items = value.as_array().ok_or((
        "deep_diagnostics_invalid_receipt",
        format!("{label} must be an array"),
    ))?;
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let object = item.as_object().ok_or((
            "deep_diagnostics_invalid_receipt",
            format!("{label}[{index}] must be an object"),
        ))?;
        let classification = object.get("classification").and_then(Value::as_str).ok_or((
            "deep_diagnostics_invalid_receipt",
            format!("{label}[{index}].classification missing"),
        ))?;
        if !matches!(
            classification,
            "requested_semantic"
                | "save_normalization"
                | "expected_derived"
                | "unexplained_collateral"
                | "unavailable"
        ) {
            return Err((
                "deep_diagnostics_invalid_receipt",
                format!("{label}[{index}].classification invalid"),
            ));
        }
        out.push(allowlisted_object(object, allowlist));
    }
    Ok(Value::Array(out))
}

fn diagnostic_parser_outcomes(
    object: &Map<String, Value>,
) -> Result<Value, (&'static str, String)> {
    require_exact_object_keys(object, &["source", "control", "mutation"], "parser_outcomes")?;
    let mut out = Map::new();
    for arm in ["source", "control", "mutation"] {
        let value = required_object_value(object, arm, "parser_outcomes")?;
        require_exact_object_keys(value, &["status", "diagnostic_codes"], &format!("parser_outcomes.{arm}"))?;
        let status = value.get("status").and_then(Value::as_str).ok_or((
            "deep_diagnostics_invalid_receipt",
            format!("parser_outcomes.{arm}.status missing"),
        ))?;
        if !matches!(status, "accepted" | "rejected" | "unavailable") {
            return Err((
                "deep_diagnostics_invalid_receipt",
                format!("parser_outcomes.{arm}.status invalid"),
            ));
        }
        let codes = value.get("diagnostic_codes").and_then(Value::as_array).ok_or((
            "deep_diagnostics_invalid_receipt",
            format!("parser_outcomes.{arm}.diagnostic_codes invalid"),
        ))?;
        if !codes.iter().all(|value| value.as_str().is_some()) {
            return Err((
                "deep_diagnostics_invalid_receipt",
                format!("parser_outcomes.{arm}.diagnostic_codes must be strings"),
            ));
        }
        out.insert(arm.to_owned(), json!({"status":status,"diagnostic_codes":codes}));
    }
    Ok(Value::Object(out))
}

fn diagnostic_second_save_summary(
    object: &Map<String, Value>,
) -> Result<Value, (&'static str, String)> {
    let status = object.get("status").and_then(Value::as_str).ok_or((
        "deep_diagnostics_invalid_receipt",
        "second_save_convergence.status missing".to_owned(),
    ))?;
    if !matches!(status, "unavailable" | "converged" | "changed") {
        return Err((
            "deep_diagnostics_invalid_receipt",
            "second_save_convergence.status invalid".to_owned(),
        ));
    }
    let mut out = Map::new();
    out.insert("status".to_owned(), Value::String(status.to_owned()));
    for field in ["changed_stream_count", "different_byte_count"] {
        if let Some(value) = object.get(field) {
            let count = value.as_u64().ok_or((
                "deep_diagnostics_invalid_receipt",
                format!("second_save_convergence.{field} invalid"),
            ))?;
            out.insert(field.to_owned(), Value::from(count));
        }
    }
    if let Some(artifact) = object.get("artifact") {
        let artifact = artifact.as_object().ok_or((
            "deep_diagnostics_invalid_receipt",
            "second_save_convergence.artifact invalid".to_owned(),
        ))?;
        out.insert(
            "artifact".to_owned(),
            diagnostic_artifact_summary(artifact, "second_save")?,
        );
    }
    Ok(Value::Object(out))
}

fn allowlisted_object(object: &Map<String, Value>, fields: &[&str]) -> Value {
    let mut out = Map::new();
    for field in fields {
        if let Some(value) = object.get(*field) {
            out.insert((*field).to_owned(), value.clone());
        }
    }
    Value::Object(out)
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &'static str,
) -> Result<&'a str, (&'static str, String)> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(("invalid_argument", format!("{field} must be a non-empty string")))
}

fn required_u32(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<u32, (&'static str, String)> {
    let value = object
        .get(field)
        .and_then(Value::as_u64)
        .ok_or(("invalid_argument", format!("{field} must be an unsigned integer")))?;
    u32::try_from(value).map_err(|_| ("invalid_argument", format!("{field} does not fit u32")))
}

fn required_i64(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<i64, (&'static str, String)> {
    object
        .get(field)
        .and_then(Value::as_i64)
        .ok_or(("invalid_argument", format!("{field} must be an i64")))
}

fn parse_story_id(value: &str) -> Result<StoryId, (&'static str, String)> {
    serde_json::from_value(Value::String(value.to_owned()))
        .map_err(|error| ("invalid_story_id", error.to_string()))
}

fn parse_node_id(value: &str) -> Result<NodeId, (&'static str, String)> {
    serde_json::from_value(Value::String(value.to_owned()))
        .map_err(|error| ("invalid_node_id", error.to_string()))
}

fn parse_export_target(value: &str) -> Result<EditorEditableTarget, (&'static str, String)> {
    match value {
        "idml" => Ok(EditorEditableTarget::Idml),
        "odg" => Ok(EditorEditableTarget::Odg),
        _ => Err(("unsupported_export_target", "target must be idml or odg".to_owned())),
    }
}

fn direct_instances(session: &AgentSession, page_filter: Option<&str>) -> Vec<Value> {
    let mut result = Vec::new();
    for page in &session.visual.document.pages {
        let page_id = page.id.as_canonical().to_string();
        if page_filter.is_some_and(|wanted| wanted != page_id) {
            continue;
        }
        let page_origin = page.id.into_canonical();
        for scene_node in session
            .visual
            .scene
            .nodes
            .iter()
            .filter(|node| node.parent_origin == page_origin)
        {
            let Some(authored) = session.editor.graph().nodes.get(&scene_node.origin) else {
                continue;
            };
            if authored.header.parent_id.to_string() != page_id {
                continue;
            }
            let Ok(instance) = direct_page_local_instance_v1(
                &scene_node.origin.as_canonical().to_string(),
                &page_id,
            ) else {
                continue;
            };
            let admission = admit_object_mutation_v1(&instance, ObjectMutationKindV1::MoveNode);
            let editor_move = session.editor.can_move_node_to(
                scene_node.origin,
                authored.header.bounds.x,
                authored.header.bounds.y,
            );
            result.push(json!({
                "instance_id":instance.instance_id,
                "projection_kind":"direct_page_local",
                "origin_node_id":scene_node.origin.as_canonical().to_string(),
                "target_page_id":page_id,
                "bounds":rect_json(authored.header.bounds),
                "move_node_scene_admitted":admission.admitted,
                "move_node_editor_supported":editor_move.is_ok(),
                "move_node_reason":editor_move.err().map(|error| error.code().to_owned()).unwrap_or(admission.reason),
                "geometry_sync_policy":"apply_authored_origin_geometry"
            }));
        }
    }
    result.sort_by(|left, right| {
        left.get("instance_id")
            .and_then(Value::as_str)
            .cmp(&right.get("instance_id").and_then(Value::as_str))
    });
    result
}

fn find_direct_instance(
    session: &AgentSession,
    instance_id: &str,
) -> Option<(SceneInstanceV1, NodeId, RectEmu)> {
    for page in &session.visual.document.pages {
        let page_id = page.id.as_canonical().to_string();
        let page_origin = page.id.into_canonical();
        for scene_node in session
            .visual
            .scene
            .nodes
            .iter()
            .filter(|node| node.parent_origin == page_origin)
        {
            let Some(authored) = session.editor.graph().nodes.get(&scene_node.origin) else {
                continue;
            };
            if authored.header.parent_id.to_string() != page_id {
                continue;
            }
            let Ok(instance) = direct_page_local_instance_v1(
                &scene_node.origin.as_canonical().to_string(),
                &page_id,
            ) else {
                continue;
            };
            if instance.instance_id == instance_id {
                return Some((instance, scene_node.origin, authored.header.bounds));
            }
        }
    }
    None
}

fn find_direct_instance_view(session: &AgentSession, instance_id: &str) -> Option<Value> {
    let (instance, node_id, bounds) = find_direct_instance(session, instance_id)?;
    let move_admission = admit_object_mutation_v1(&instance, ObjectMutationKindV1::MoveNode);
    Some(json!({
        "instance":instance,
        "effective_bounds":rect_json(bounds),
        "origin_node_id":node_id.as_canonical().to_string(),
        "move_node_admission":move_admission,
        "geometry_sync_policy":geometry_sync_policy_v1(&instance)
    }))
}

fn session_state_summary(session: &AgentSession) -> Option<Value> {
    let state_id = state_id(&session.editor).ok()?;
    Some(json!({
        "state_id":state_id,
        "revision_index":session.editor.operations().len(),
        "operation_count":session.editor.operations().len()
    }))
}

fn state_id(editor: &EditorSession) -> Result<String, (&'static str, String)> {
    let bytes = serde_json::to_vec(&editor.project())
        .map_err(|error| ("state_serialize_failed", error.to_string()))?;
    Ok(format!("sha256:{}", sha256_hex(&bytes)))
}

fn operation_id(operation: &EditOperation) -> String {
    let bytes = serde_json::to_vec(operation).expect("EditOperation JSON serialization is infallible");
    format!("sha256:{}", sha256_hex(&bytes))
}

fn operation_summary(operation: &EditOperation) -> Value {
    match operation {
        EditOperation::ReplaceStoryRange {
            story_id,
            start_scalar,
            end_scalar,
            before_story_state_id,
            after_story_state_id,
            ..
        } => json!({
            "kind":"replace_story_range",
            "story_id":story_id.as_canonical().to_string(),
            "start_scalar":start_scalar,
            "end_scalar":end_scalar,
            "before_story_state_id":before_story_state_id,
            "after_story_state_id":after_story_state_id
        }),
        EditOperation::ReplaceStoryText { story_id, before, after } => json!({
            "kind":"replace_story_text",
            "story_id":story_id.as_canonical().to_string(),
            "before_text_sha256":sha256_hex(before.as_bytes()),
            "after_text_sha256":sha256_hex(after.as_bytes())
        }),
        EditOperation::ReplaceTableCellText { node_id, story_id, cell_id, .. } => json!({
            "kind":"replace_table_cell_text",
            "node_id":node_id.as_canonical().to_string(),
            "story_id":story_id.as_canonical().to_string(),
            "cell_id":cell_id.as_canonical().to_string()
        }),
        EditOperation::ReplaceImage { node_id, before_asset, after_asset } => json!({
            "kind":"replace_image",
            "node_id":node_id.as_canonical().to_string(),
            "before_asset":before_asset.as_ref().map(|value| value.to_string()),
            "after_asset":after_asset.to_string()
        }),
        EditOperation::MoveNode { node_id, before, after } => json!({
            "kind":"move_node",
            "node_id":node_id.as_canonical().to_string(),
            "before":rect_json(*before),
            "after":rect_json(*after)
        }),
    }
}

fn rect_json(rect: RectEmu) -> Value {
    json!({
        "x":rect.x.get(),
        "y":rect.y.get(),
        "width":rect.width.get(),
        "height":rect.height.get()
    })
}

fn diagnostic_codes(visual: &ViewerGeometryDocument) -> Vec<String> {
    visual
        .document
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.clone())
        .collect()
}

fn set_delta(before: &[String], after: &[String]) -> (Vec<String>, Vec<String>) {
    let before_set = before.iter().cloned().collect::<std::collections::BTreeSet<_>>();
    let after_set = after.iter().cloned().collect::<std::collections::BTreeSet<_>>();
    (
        after_set.difference(&before_set).cloned().collect(),
        before_set.difference(&after_set).cloned().collect(),
    )
}

fn fidelity_label(value: ViewerFidelityStatus) -> &'static str {
    match value {
        ViewerFidelityStatus::Supported => "supported",
        ViewerFidelityStatus::Partial => "partial",
        ViewerFidelityStatus::Unsupported => "unsupported",
    }
}

fn verify_source_immutable(session: &AgentSession) -> Result<(), (&'static str, String)> {
    let current = fs::read(&session.source_path)
        .map_err(|error| ("source_recheck_failed", format!("re-read source: {error}")))?;
    if current != session.source_bytes {
        return Err((
            "source_identity_changed",
            "source PUB bytes changed outside the agent session".to_owned(),
        ));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("hex write cannot fail");
    }
    encoded
}

fn success_envelope(
    request_id: &str,
    command: &str,
    session: Option<&AgentSession>,
    result: Value,
) -> Value {
    json!({
        "protocol_version":PROTOCOL_VERSION,
        "message_type":"result",
        "request_id":request_id,
        "command":command,
        "ok":true,
        "session_id":session.map(|value| value.session_id.as_str()),
        "source_hash":session.map(|value| value.visual.document.source.source_hash.to_string()),
        "state":session.and_then(session_state_summary),
        "result":result
    })
}

fn error_envelope(
    request_id: Option<&str>,
    command: Option<&str>,
    session_id: Option<&str>,
    source_hash: Option<&str>,
    code: &str,
    message: &str,
) -> Value {
    json!({
        "protocol_version":PROTOCOL_VERSION,
        "message_type":"result",
        "request_id":request_id,
        "command":command,
        "ok":false,
        "session_id":session_id,
        "source_hash":source_hash,
        "error":{"code":code,"message":message}
    })
}

fn trace_envelope(
    event_index: u64,
    request_id: &str,
    command: &str,
    event_kind: &str,
    session: Option<&AgentSession>,
    before: Option<&Value>,
    after: Option<&Value>,
    payload: Value,
) -> Value {
    json!({
        "protocol_version":PROTOCOL_VERSION,
        "message_type":"trace",
        "event_index":event_index,
        "request_id":request_id,
        "command":command,
        "event_kind":event_kind,
        "session_id":session.map(|value| value.session_id.as_str()),
        "source_hash":session.map(|value| value.visual.document.source.source_hash.to_string()),
        "before_state":before,
        "after_state":after,
        "payload":payload
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_json_is_a_machine_error_not_a_panic() {
        let mut server = AgentServer::default();
        let responses = server.handle_line("{");
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0]["ok"], false);
        assert_eq!(responses[0]["error"]["code"], "invalid_json");
    }

    #[test]
    fn protocol_describe_is_available_before_open() {
        let mut server = AgentServer::default();
        let responses = server.handle_line(r#"{"request_id":"r1","command":"protocol.describe"}"#);
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0]["ok"], true);
        assert_eq!(
            responses[0]["result"]["protocol_version"],
            PROTOCOL_VERSION
        );
        assert_eq!(responses[0]["result"]["native_pub_write"], false);
    }

    #[test]
    fn content_read_requires_explicit_local_consent() {
        let mut server = AgentServer::default();
        let responses = server.handle_line(
            r#"{"request_id":"r1","command":"story.read_local","story_id":"00112233-4455-6677-8899-aabbccddeeff"}"#,
        );
        assert_eq!(responses[0]["ok"], false);
        assert_eq!(
            responses[0]["error"]["code"],
            "content_consent_required"
        );
    }

    #[test]
    fn id_parser_reuses_canonical_wire_format() {
        let id = "00112233-4455-6677-8899-aabbccddeeff";
        assert_eq!(
            parse_story_id(id)
                .expect("valid StoryId")
                .as_canonical()
                .to_string(),
            id
        );
        assert_eq!(
            parse_node_id(id)
                .expect("valid NodeId")
                .as_canonical()
                .to_string(),
            id
        );
    }

    fn deep_receipt(source_hash: &str) -> Value {
        json!({
            "schema_version":"chaptera.operation-blast-radius.v1",
            "operation":{
                "kind":"MoveNode",
                "operation_id":"op-1",
                "node_id":"node-1",
                "raw_text":"SECRET-MUST-NOT-ESCAPE"
            },
            "artifacts":{
                "source":{
                    "sha256":source_hash,
                    "byte_len":1000,
                    "producer":{"local_path":"C:\\private\\source.pub"}
                },
                "control":{"sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","byte_len":1000},
                "mutation":{"sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","byte_len":1000}
            },
            "cfb":{
                "source_control_topology_delta":[],
                "control_mutation_topology_delta":[],
                "source_control_stream_delta":[],
                "control_mutation_stream_delta":[{
                    "stream_id":"dir:1:Contents",
                    "before_sha256":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                    "after_sha256":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                    "before_size":100,
                    "after_size":100,
                    "classification":"requested_semantic",
                    "raw_bytes":"SECRET"
                }],
                "control_mutation_byte_ranges":[{
                    "offset":10,
                    "length":4,
                    "physical_label":"stream_payload:Contents",
                    "classification":"requested_semantic"
                }]
            },
            "parsed_record_family_delta":[{
                "family":"Escher",
                "id":"shape-1",
                "before_sha256":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                "after_sha256":"1111111111111111111111111111111111111111111111111111111111111111",
                "classification":"requested_semantic"
            }],
            "semantic_graph_delta":[{
                "kind":"node",
                "id":"node-1",
                "before_sha256":"2222222222222222222222222222222222222222222222222222222222222222",
                "after_sha256":"3333333333333333333333333333333333333333333333333333333333333333",
                "classification":"requested_semantic"
            }],
            "parser_outcomes":{
                "source":{"status":"accepted","diagnostic_codes":[]},
                "control":{"status":"accepted","diagnostic_codes":["save.normalized"]},
                "mutation":{"status":"accepted","diagnostic_codes":[]}
            },
            "second_save_convergence":{"status":"unavailable"},
            "classification_counts":{
                "requested_semantic":4,
                "save_normalization":1,
                "expected_derived":0,
                "unexplained_collateral":0,
                "unavailable":0
            },
            "invariants":{
                "raw_byte_inequality_is_not_semantic_evidence":true,
                "matched_noop_control_used":true,
                "unexplained_collateral_preserved":true,
                "public_receipt_contains_raw_document_bytes":false,
                "native_pub_writer_capability_granted":false
            }
        })
    }

    fn write_deep_receipt(value: &Value, suffix: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "chaptera-agent-deep-{}-{}-{suffix}.json",
            std::process::id(),
            sha256_hex(suffix.as_bytes())
        ));
        fs::write(
            &path,
            serde_json::to_vec(value).expect("serialize diagnostic fixture"),
        )
        .expect("write diagnostic fixture");
        path
    }

    #[test]
    fn deep_diagnostics_requires_explicit_local_file_consent() {
        let mut server = AgentServer::default();
        let responses = server.handle_line(
            r#"{"request_id":"r1","command":"diagnostics.deep","receipt_path":"receipt.json"}"#,
        );
        assert_eq!(responses[0]["ok"], false);
        assert_eq!(
            responses[0]["error"]["code"],
            "local_file_consent_required"
        );
    }

    #[test]
    fn deep_diagnostics_without_receipt_is_explicitly_not_loaded() {
        let mut server = AgentServer::default();
        let responses =
            server.handle_line(r#"{"request_id":"r1","command":"diagnostics.deep"}"#);
        assert_eq!(responses[0]["ok"], true);
        assert_eq!(responses[0]["result"]["available"], false);
        assert_eq!(
            responses[0]["result"]["reason"],
            "deep_diagnostics_receipt_not_loaded"
        );
    }

    #[test]
    fn deep_diagnostics_summary_is_source_bound_and_source_free() {
        let source_hash =
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let fixture = deep_receipt(source_hash);
        let path = write_deep_receipt(&fixture, "allowlist");
        let summary =
            deep_diagnostics_summary(&path, source_hash).expect("valid deep receipt");
        let encoded = serde_json::to_string(&summary).expect("serialize summary");
        assert_eq!(summary["available"], true);
        assert_eq!(summary["source_hash"], source_hash);
        assert_eq!(
            summary["operation"]["kind"],
            "MoveNode"
        );
        assert!(!encoded.contains("SECRET"));
        assert!(!encoded.contains("private"));
        assert!(!encoded.contains(path.to_string_lossy().as_ref()));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn deep_diagnostics_rejects_receipt_for_another_pub() {
        let receipt_hash =
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let current_hash =
            "9999999999999999999999999999999999999999999999999999999999999999";
        let fixture = deep_receipt(receipt_hash);
        let path = write_deep_receipt(&fixture, "mismatch");
        let error = deep_diagnostics_summary(&path, current_hash)
            .expect_err("different source must fail");
        assert_eq!(error.0, "deep_diagnostics_source_mismatch");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn deep_diagnostics_rejects_writer_capability_escalation() {
        let source_hash =
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let mut fixture = deep_receipt(source_hash);
        fixture["invariants"]["native_pub_writer_capability_granted"] = Value::Bool(true);
        let path = write_deep_receipt(&fixture, "writer-escalation");
        let error = deep_diagnostics_summary(&path, source_hash)
            .expect_err("writer capability escalation must fail");
        assert_eq!(error.0, "deep_diagnostics_invalid_receipt");
        let _ = fs::remove_file(path);
    }
}
