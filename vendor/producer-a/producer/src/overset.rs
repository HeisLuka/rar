use anyhow::{Context, Result, anyhow, bail};
use pub_editor::{EditOperation, EditorEditableTarget, EditorProject, EditorSession};
use pub_layout::{
    BoundedAuthoringSlice, BoundedLayoutEnvironment, BoundedNodeGeometryInput,
    BoundedShapedFlowRuntime, BoundedShapingRuntime, font_fingerprint_sha256, project_bounded,
    resolve_bounded_shaped_flow,
};
use pub_model::{
    Affine2D, CanonicalId, Document, DocumentId, EMU_PER_POINT, LengthEmu, Node, NodeHeader,
    NodeId, NodeKind, Page, PageId, RectEmu, ResolvedGraph, Sha256Digest, Size2D, SourceDescriptor,
    Story, StoryFrame, StoryId,
};
use pub_reader::{
    PubExplicitShapePaintSource, PubResolvedGraph, PubResolvedNodePayload, PubResolvedStoryFrame,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};

const BASELINE_TEXT: &str = "Hi";
const REPLACEMENT_TEXT: &str = "Hfi Hfi Hfi Hfi Hfi Hfi Hfi Hfi";
const FRAME_WIDTH_EMU: i64 = 600_000;
const FRAME_HEIGHT_EMU: i64 = 200_000;
const LINE_HEIGHT_EMU: i64 = 200_000;
const FONT_SIZE_EMU: i64 = 12 * EMU_PER_POINT;

fn canonical(byte: u8) -> CanonicalId {
    CanonicalId::from_bytes([byte; 16])
}

fn document_id() -> DocumentId {
    DocumentId::from_canonical(canonical(0x10))
}

fn page_id() -> PageId {
    PageId::from_canonical(canonical(0x20))
}

fn frame_id() -> NodeId {
    NodeId::from_canonical(canonical(0x30))
}

fn story_id() -> StoryId {
    StoryId::from_canonical(canonical(0x40))
}

fn text_hash(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("hex write");
    }
    out
}

fn hash_id(text: &str) -> String {
    format!("sha256:{}", text_hash(text))
}

fn parse_source_hash(value: &str) -> Result<Sha256Digest> {
    value
        .parse()
        .map_err(|_| anyhow!("source_hash must be lowercase SHA-256"))
}

fn synthetic_graph(source_hash_text: &str) -> Result<PubResolvedGraph> {
    let source_hash = parse_source_hash(source_hash_text)?;
    let page_id = page_id();
    let frame_id = frame_id();
    let story_id = story_id();

    let source = SourceDescriptor {
        format: "synthetic-authoring-overset".into(),
        format_version: Some("v1".into()),
        adapter_version: "rar-authoring-overset-producer-v1".into(),
        source_hash,
    };
    let document = Document {
        id: document_id(),
        format_origin: "synthetic-authoring-overset".into(),
        source_hash,
        pages: vec![page_id],
        resources: Vec::new(),
        styles: Vec::new(),
    };
    let page = Page {
        id: page_id,
        size: Size2D::new(LengthEmu::new(2_000_000), LengthEmu::new(2_000_000)),
        bleed: None,
        margins: None,
        children: vec![frame_id],
        extensions: Vec::new(),
    };
    let node = Node {
        kind: NodeKind::TextFrame,
        header: NodeHeader {
            id: frame_id,
            parent_id: page_id.into_canonical(),
            bounds: RectEmu::new(
                LengthEmu::new(100_000),
                LengthEmu::new(100_000),
                LengthEmu::new(FRAME_WIDTH_EMU),
                LengthEmu::new(FRAME_HEIGHT_EMU),
            ),
            transform: Affine2D::identity(),
            source_refs: Vec::new(),
            extensions: Vec::new(),
        },
        payload: PubResolvedNodePayload {
            contents_seq_num: 1,
            officeart_shape_type: Some(202),
            officeart_spid: Some(1),
            image_slot: None,
            explicit_image_crop: None,
            explicit_paint: PubExplicitShapePaintSource::default(),
            story_frame: Some(PubResolvedStoryFrame {
                story_id: Some(story_id),
                ordinal: 0,
                previous_frame: None,
                next_frame: None,
            }),
            table_story: None,
            table: None,
        },
    };
    let story = Story {
        id: story_id,
        text: BASELINE_TEXT.into(),
        paragraphs: Vec::new(),
        runs: Vec::new(),
        fields: Vec::new(),
        hyperlinks: Vec::new(),
        source_refs: Vec::new(),
    };

    Ok(ResolvedGraph {
        cdm_version: "0.1".into(),
        resolver_version: "rar-authoring-overset-synthetic-v1".into(),
        source,
        document,
        pages: BTreeMap::from([(page_id, page)]),
        nodes: BTreeMap::from([(frame_id, node)]),
        stories: BTreeMap::from([(story_id, story)]),
        paragraphs: BTreeMap::new(),
        text_runs: BTreeMap::new(),
        resources: BTreeMap::new(),
        styles: BTreeMap::new(),
        extensions: BTreeMap::new(),
    })
}

fn baseline_session(source_hash: &str) -> Result<EditorSession> {
    EditorSession::new(synthetic_graph(source_hash)?).map_err(|error| anyhow!(error.to_string()))
}

fn project_from(value: &Value) -> Result<EditorProject> {
    serde_json::from_value(value.clone()).context("parse canonical EditorProject")
}

fn project_value(session: &EditorSession) -> Result<Value> {
    serde_json::to_value(session.project()).context("serialize canonical EditorProject")
}

fn story_text(session: &EditorSession) -> Result<&str> {
    session
        .graph()
        .stories
        .get(&story_id())
        .map(|story| story.text.as_str())
        .ok_or_else(|| anyhow!("synthetic Story missing"))
}

fn authoring_slice(session: &EditorSession) -> BoundedAuthoringSlice {
    let graph = session.graph();
    let node_geometry = graph
        .nodes
        .values()
        .map(|node| BoundedNodeGeometryInput {
            node_id: node.header.id,
            parent_origin: node.header.parent_id,
            bounds: node.header.bounds,
            transform: node.header.transform.clone(),
        })
        .collect();

    let story_frames = graph
        .nodes
        .values()
        .filter_map(|node| {
            let frame = node.payload.story_frame.as_ref()?;
            Some(StoryFrame {
                story_id: frame.story_id?,
                frame_id: node.header.id,
                ordinal: frame.ordinal,
                previous: frame.previous_frame,
                next: frame.next_frame,
            })
        })
        .collect();

    BoundedAuthoringSlice {
        pages: graph.pages.values().cloned().collect(),
        node_geometry,
        stories: graph.stories.values().cloned().collect(),
        story_frames,
        tables: Vec::new(),
        guides: Vec::new(),
        unknown_layout_state: Vec::new(),
    }
}

fn layout_state(session: &EditorSession) -> Result<Value> {
    let font = font_test_data::NOTOSERIF_AUTOHINT_SHAPING;
    let runtime = BoundedShapedFlowRuntime {
        shaping: BoundedShapingRuntime {
            layout: BoundedLayoutEnvironment {
                engine_revision: "authoring-overset-v1".into(),
                font_set_fingerprint: font_fingerprint_sha256(font),
                resource_fingerprint: "resources:none".into(),
            },
            face_index: 0,
            font_size_emu: LengthEmu::new(FONT_SIZE_EMU),
            font_bytes: font,
        },
        line_height: LengthEmu::new(LINE_HEIGHT_EMU),
    };
    let projection = project_bounded(authoring_slice(session));
    let scene = resolve_bounded_shaped_flow(&projection, &runtime)
        .map_err(|error| anyhow!("shaped flow failed: {error}"))?;

    let environment_bytes =
        serde_json::to_vec(&scene.environment).context("serialize layout environment")?;
    let environment_hash = hash_id(
        &environment_bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
    );
    let overset = scene
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "story_overset");
    let text = story_text(session)?;

    Ok(json!({
        "story_hash": hash_id(text),
        "scalar_count": text.chars().count(),
        "state": if overset { "overset" } else { "fits" },
        "reason_code": Value::Null,
        "environment_authoritative": true,
        "layout_environment_hash": environment_hash,
    }))
}

fn layout_unknown_state(session: &EditorSession) -> Result<Value> {
    let text = story_text(session)?;
    Ok(json!({
        "story_hash": hash_id(text),
        "scalar_count": text.chars().count(),
        "state": "layout_unknown",
        "reason_code": "layout.environment_unavailable",
        "environment_authoritative": false,
        "layout_environment_hash": Value::Null,
    }))
}

fn consequences(key: &str) -> Value {
    json!([{"key": key, "state": "supported", "note": Value::Null}])
}

fn canonical_story_operation(
    command: &Value,
    before: &str,
    after: &str,
    edit: &EditOperation,
) -> Result<Value> {
    let EditOperation::ReplaceStoryRange {
        story_id: actual_story_id,
        start_scalar,
        end_scalar,
        expected_before,
        replacement_text,
        before_story_state_id,
        after_story_state_id,
    } = edit
    else {
        bail!("EditorSession returned non-range operation");
    };

    let command_story = command
        .get("story_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("command.story_id missing"))?;
    if command_story != actual_story_id.as_canonical().to_string()
        || command.get("start_scalar").and_then(Value::as_u64) != Some(u64::from(*start_scalar))
        || command.get("end_scalar").and_then(Value::as_u64) != Some(u64::from(*end_scalar))
        || command.get("expected_before").and_then(Value::as_str) != Some(expected_before)
        || command.get("replacement_text").and_then(Value::as_str) != Some(replacement_text)
    {
        bail!("EditorSession canonical operation differs from accepted Story intent");
    }

    let replacement_scalar_len = u32::try_from(replacement_text.chars().count())
        .map_err(|_| anyhow!("replacement scalar count overflow"))?;
    let inverse_end = start_scalar
        .checked_add(replacement_scalar_len)
        .ok_or_else(|| anyhow!("inverse scalar range overflow"))?;

    Ok(json!({
        "protocol_version": "chaptera.replace-story-range.v1",
        "kind": "replace_story_range",
        "story_id": actual_story_id.as_canonical().to_string(),
        "start_scalar": start_scalar,
        "end_scalar": end_scalar,
        "expected_before": expected_before,
        "replacement_text": replacement_text,
        "inverse": {
            "start_scalar": start_scalar,
            "end_scalar": inverse_end,
            "expected_before": replacement_text,
            "replacement_text": expected_before,
        },
        "before_text_hash": text_hash(before),
        "after_text_hash": text_hash(after),
        "before_story_state_id": before_story_state_id,
        "after_story_state_id": after_story_state_id,
    }))
}

fn apply_fixed_edit(session: &mut EditorSession) -> Result<EditOperation> {
    session
        .replace_story_range(
            story_id(),
            0,
            u32::try_from(BASELINE_TEXT.chars().count()).expect("bounded baseline"),
            BASELINE_TEXT,
            REPLACEMENT_TEXT,
        )
        .map_err(|error| anyhow!(error.to_string()))
}

fn baseline(source_hash: &str) -> Result<Value> {
    let session = baseline_session(source_hash)?;
    Ok(json!({
        "source_hash": source_hash,
        "baseline_project": project_value(&session)?,
        "story_id": story_id().as_canonical().to_string(),
        "frame_node_id": frame_id().as_canonical().to_string(),
        "baseline_layout_state": layout_state(&session)?,
        "edit_intent": {
            "start_scalar": 0,
            "end_scalar": BASELINE_TEXT.chars().count(),
            "expected_before": BASELINE_TEXT,
            "replacement_text": REPLACEMENT_TEXT,
        },
    }))
}

fn commit(payload: &Value, source_hash: &str) -> Result<Value> {
    let base_project = project_from(
        payload
            .get("base_project")
            .ok_or_else(|| anyhow!("base_project missing"))?,
    )?;
    let command = payload
        .get("command")
        .ok_or_else(|| anyhow!("command missing"))?;
    let mut session = baseline_session(source_hash)?;
    session
        .apply_project(&base_project)
        .map_err(|error| anyhow!(error.to_string()))?;

    let before = story_text(&session)?.to_owned();
    let start = u32::try_from(
        command
            .get("start_scalar")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("command.start_scalar missing"))?,
    )
    .map_err(|_| anyhow!("command.start_scalar overflow"))?;
    let end = u32::try_from(
        command
            .get("end_scalar")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("command.end_scalar missing"))?,
    )
    .map_err(|_| anyhow!("command.end_scalar overflow"))?;
    let expected_before = command
        .get("expected_before")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("command.expected_before missing"))?;
    let replacement = command
        .get("replacement_text")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("command.replacement_text missing"))?;

    let operation = session
        .replace_story_range(story_id(), start, end, expected_before, replacement)
        .map_err(|error| anyhow!(error.to_string()))?;
    let after = story_text(&session)?.to_owned();

    Ok(json!({
        "canonical_operation": canonical_story_operation(command, &before, &after, &operation)?,
        "resulting_project": project_value(&session)?,
        "consequences": consequences("story.text"),
        "accepted_layout_state": layout_state(&session)?,
        "source_hash_after": source_hash,
    }))
}

fn history(payload: &Value, source_hash: &str) -> Result<Value> {
    let kind = payload
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("history kind missing"))?;
    let base_project = project_from(
        payload
            .get("base_project")
            .ok_or_else(|| anyhow!("base_project missing"))?,
    )?;
    let mut session = baseline_session(source_hash)?;

    match kind {
        "undo" => {
            session
                .apply_project(&base_project)
                .map_err(|error| anyhow!(error.to_string()))?;
            session.undo().map_err(|error| anyhow!(error.to_string()))?;
        }
        "redo" => {
            session
                .apply_project(&base_project)
                .map_err(|error| anyhow!(error.to_string()))?;
            if !session.operations().is_empty() {
                bail!("redo base project must be the restored baseline");
            }
            apply_fixed_edit(&mut session)?;
            session.undo().map_err(|error| anyhow!(error.to_string()))?;
            session.redo().map_err(|error| anyhow!(error.to_string()))?;
        }
        _ => bail!("unsupported history kind {kind:?}"),
    }

    Ok(json!({
        "resulting_project": project_value(&session)?,
        "layout_state": layout_state(&session)?,
        "consequences": consequences(&format!("history.{kind}")),
        "source_hash_after": source_hash,
    }))
}

fn replay(payload: &Value, source_hash: &str) -> Result<Value> {
    let project = project_from(
        payload
            .get("project")
            .ok_or_else(|| anyhow!("project missing"))?,
    )?;
    let mut session = baseline_session(source_hash)?;
    session
        .apply_project(&project)
        .map_err(|error| anyhow!(error.to_string()))?;
    let layout = layout_state(&session)?;
    let text = story_text(&session)?.to_owned();

    let export = session
        .export_editable(EditorEditableTarget::Idml, "synthetic-one-frame")
        .map_err(|error| anyhow!("editable export failed: {error}"))?;
    if export.bytes.is_empty() {
        bail!("editable export unexpectedly empty");
    }

    Ok(json!({
        "replayed_project": project_value(&session)?,
        "layout_state": layout,
        "editable_export_story_hash": text_hash(&text),
        "fixed_output_outcome": "explicit_overset_loss",
        "source_hash_after": source_hash,
    }))
}

fn layout_unknown_probe(payload: &Value, source_hash: &str) -> Result<Value> {
    let project = project_from(
        payload
            .get("project")
            .ok_or_else(|| anyhow!("project missing"))?,
    )?;
    let mut session = baseline_session(source_hash)?;
    session
        .apply_project(&project)
        .map_err(|error| anyhow!(error.to_string()))?;
    Ok(json!({
        "layout_state": layout_unknown_state(&session)?,
        "source_hash_after": source_hash,
    }))
}

pub fn run() -> Result<()> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("read authoring overset producer request")?;
    let payload: Value = serde_json::from_str(&input).context("parse authoring overset request")?;
    let action = payload
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("action missing"))?;
    let source_hash = payload
        .get("source_hash")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("source_hash missing"))?;
    parse_source_hash(source_hash)?;

    let output = match action {
        "baseline" => baseline(source_hash)?,
        "commit" => commit(&payload, source_hash)?,
        "history" => history(&payload, source_hash)?,
        "replay" => replay(&payload, source_hash)?,
        "layout_unknown_probe" => layout_unknown_probe(&payload, source_hash)?,
        _ => bail!("unsupported authoring overset action {action:?}"),
    };

    serde_json::to_writer(std::io::stdout(), &output).context("write producer response")?;
    std::io::stdout()
        .flush()
        .context("flush producer response")?;
    Ok(())
}
