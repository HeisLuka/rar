use std::collections::BTreeMap;

use pub_editor::{
    AuthoringTextPresetV1, EDITOR_PROJECT_VERSION_CURRENT, EditOperation, EditorError,
    EditorSession, LengthEmu, RectEmu,
};
use pub_model::{
    Document, DocumentId, Page, PageId, ResolvedGraph, Sha256Digest, Size2D, SourceDescriptor,
};
use pub_reader::PubResolvedGraph;

fn canonical_id<T: serde::de::DeserializeOwned>(value: &str) -> T {
    serde_json::from_str(&format!("\"{value}\"")).expect("canonical typed id")
}

fn source_hash() -> Sha256Digest {
    "7777777777777777777777777777777777777777777777777777777777777777"
        .parse()
        .expect("sha")
}

fn page_id() -> PageId {
    canonical_id("11000000-0000-4000-8000-000000000001")
}

fn text_node_id() -> pub_editor::NodeId {
    canonical_id("01890f47-0c00-7abc-8def-0123456789ab")
}

fn story_id() -> pub_editor::StoryId {
    canonical_id("01890f47-0c01-7abc-8def-0123456789ab")
}

fn rect() -> RectEmu {
    RectEmu::new(
        LengthEmu::new(100_000),
        LengthEmu::new(200_000),
        LengthEmu::new(2_400_000),
        LengthEmu::new(900_000),
    )
}

fn preset() -> AuthoringTextPresetV1 {
    AuthoringTextPresetV1 {
        resource_id: "chaptera.desktop.fallback-font.ubuntu-light.v1".into(),
        font_fingerprint_sha256:
            "80307b8da7649aa4ee4d484b232140e3ce1ec0ca093073d3c53c8f5a5ced7a70".into(),
        face_index: 0,
        font_size_emu: LengthEmu::new(114_300),
        line_height_emu: LengthEmu::new(142_875),
    }
}

fn graph() -> PubResolvedGraph {
    let source_hash = source_hash();
    let page_id = page_id();
    let mut pages = BTreeMap::new();
    pages.insert(
        page_id,
        Page {
            id: page_id,
            size: Size2D::new(LengthEmu::new(8_000_000), LengthEmu::new(10_000_000)),
            bleed: None,
            margins: None,
            children: Vec::new(),
            extensions: Vec::new(),
        },
    );

    ResolvedGraph {
        cdm_version: "0.1".into(),
        resolver_version: "create-textbox-rust-test".into(),
        source: SourceDescriptor {
            format: "pub".into(),
            format_version: Some("0x2c".into()),
            adapter_version: "pub-rs/test".into(),
            source_hash,
        },
        document: Document {
            id: canonical_id::<DocumentId>("33000000-0000-4000-8000-000000000001"),
            format_origin: "pub".into(),
            source_hash,
            pages: vec![page_id],
            resources: Vec::new(),
            styles: Vec::new(),
        },
        pages,
        nodes: BTreeMap::new(),
        stories: BTreeMap::new(),
        paragraphs: BTreeMap::new(),
        text_runs: BTreeMap::new(),
        resources: BTreeMap::new(),
        styles: BTreeMap::new(),
        extensions: BTreeMap::new(),
    }
}

#[test]
fn create_text_box_is_atomic_then_uses_existing_story_edit_history() {
    let base = graph();
    let mut session = EditorSession::new(base.clone()).expect("session");

    let create = session
        .create_text_box(text_node_id(), story_id(), page_id(), rect(), preset())
        .expect("CreateTextBox");
    assert!(matches!(create, EditOperation::CreateTextBox { .. }));
    assert_eq!(session.operations().len(), 1);

    let node = session.graph().nodes.get(&text_node_id()).expect("text frame");
    assert_eq!(node.kind, pub_model::NodeKind::TextFrame);
    assert_eq!(node.header.bounds, rect());
    assert_eq!(
        node.payload
            .story_frame
            .as_ref()
            .and_then(|frame| frame.story_id),
        Some(story_id())
    );
    assert_eq!(
        session.graph().stories.get(&story_id()).expect("Story").text,
        ""
    );
    assert_eq!(
        session.graph().pages[&page_id()].children,
        vec![text_node_id()]
    );

    let edit = session
        .replace_story_range(story_id(), 0, 0, "", "Hello")
        .expect("ordinary Story edit");
    assert!(matches!(edit, EditOperation::ReplaceStoryRange { .. }));
    assert_eq!(
        session.graph().stories.get(&story_id()).expect("Story").text,
        "Hello"
    );
    assert_eq!(session.operations().len(), 2);

    session.undo().expect("undo typing");
    assert_eq!(session.graph().stories[&story_id()].text, "");
    session.undo().expect("undo create");
    assert!(!session.graph().nodes.contains_key(&text_node_id()));
    assert!(!session.graph().stories.contains_key(&story_id()));
    assert!(session.graph().pages[&page_id()].children.is_empty());

    session.redo().expect("redo create");
    session.redo().expect("redo typing");
    assert_eq!(session.graph().stories[&story_id()].text, "Hello");

    let project = session.project();
    assert_eq!(project.schema_version, EDITOR_PROJECT_VERSION_CURRENT);
    let mut reopened = EditorSession::new(base).expect("fresh session");
    reopened.apply_project(&project).expect("project replay");
    assert_eq!(reopened.project(), project);
    assert_eq!(reopened.graph().stories[&story_id()].text, "Hello");
    assert_eq!(
        reopened.graph().nodes[&text_node_id()].header.bounds,
        rect()
    );
}

#[test]
fn create_text_box_rejects_invalid_identity_bounds_and_collisions() {
    let mut session = EditorSession::new(graph()).expect("session");
    let non_v7_node = canonical_id("22000000-0000-4000-8000-000000000099");
    assert!(matches!(
        session.create_text_box(non_v7_node, story_id(), page_id(), rect(), preset()),
        Err(EditorError::CreateTextBoxInvalidNodeId { .. })
    ));

    let bad_story = canonical_id("44000000-0000-4000-8000-000000000099");
    assert!(matches!(
        session.create_text_box(text_node_id(), bad_story, page_id(), rect(), preset()),
        Err(EditorError::CreateTextBoxInvalidStoryId { .. })
    ));

    let zero = RectEmu::new(
        LengthEmu::ZERO,
        LengthEmu::ZERO,
        LengthEmu::ZERO,
        LengthEmu::new(100),
    );
    assert!(matches!(
        session.create_text_box(text_node_id(), story_id(), page_id(), zero, preset()),
        Err(EditorError::CreateTextBoxInvalidBounds { .. })
    ));

    session
        .create_text_box(text_node_id(), story_id(), page_id(), rect(), preset())
        .expect("first create");
    assert!(matches!(
        session.create_text_box(text_node_id(), canonical_id("01890f47-0c02-7abc-8def-0123456789ab"), page_id(), rect(), preset()),
        Err(EditorError::CreateTextBoxNodeIdCollision { .. })
    ));
}

#[test]
fn create_text_box_wire_persists_explicit_text_preset() {
    let mut session = EditorSession::new(graph()).expect("session");
    let operation = session
        .create_text_box(text_node_id(), story_id(), page_id(), rect(), preset())
        .expect("create");
    let value = serde_json::to_value(operation).expect("wire");
    assert_eq!(value["kind"], "create_text_box");
    assert_eq!(
        value["text_preset"]["resource_id"],
        "chaptera.desktop.fallback-font.ubuntu-light.v1"
    );
    assert_eq!(value["text_preset"]["face_index"], 0);
    assert_eq!(value["text_preset"]["font_size_emu"], 114_300);
    assert_eq!(value["text_preset"]["line_height_emu"], 142_875);
}
