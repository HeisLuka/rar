use pub_editor::{
    EDITOR_PROJECT_VERSION_V0_6, EditOperation, EditorEditableTarget, EditorError, EditorSession,
};
use pub_model::{
    Affine2D, CanonicalId, Document, DocumentId, LengthEmu, Node, NodeHeader, NodeId, NodeKind,
    Page, PageId, RectEmu, ResolvedGraph, Sha256Digest, Size2D, SourceDescriptor, Story, StoryId,
};
use pub_reader::{
    PubExplicitShapePaintSource, PubResolvedGraph, PubResolvedNodePayload, PubResolvedStoryFrame,
};
use std::collections::BTreeMap;

fn canonical(byte: u8) -> CanonicalId {
    CanonicalId::from_bytes([byte; 16])
}

fn document_id() -> DocumentId {
    DocumentId::from_canonical(canonical(0x10))
}

fn page_id() -> PageId {
    PageId::from_canonical(canonical(0x20))
}

fn frame_id(byte: u8) -> NodeId {
    NodeId::from_canonical(canonical(byte))
}

fn source_story_id() -> StoryId {
    StoryId::from_canonical(canonical(0x40))
}

fn new_story_id() -> StoryId {
    serde_json::from_str("\"01890f47-0c00-7abc-8def-0123456789ab\"")
        .expect("valid UUIDv7 StoryId")
}

fn other_new_story_id() -> StoryId {
    serde_json::from_str("\"01890f47-0c00-7abc-8def-0123456789ac\"")
        .expect("valid UUIDv7 StoryId")
}

fn source_hash() -> Sha256Digest {
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        .parse()
        .expect("valid source hash")
}

fn story() -> Story {
    Story {
        id: source_story_id(),
        text: "This text must remain entirely in the upstream Story after the split.".into(),
        paragraphs: Vec::new(),
        runs: Vec::new(),
        fields: Vec::new(),
        hyperlinks: Vec::new(),
        source_refs: Vec::new(),
    }
}

fn text_frame(
    id: NodeId,
    ordinal: u32,
    previous: Option<NodeId>,
    next: Option<NodeId>,
    x: i64,
) -> Node<PubResolvedNodePayload> {
    Node {
        kind: NodeKind::TextFrame,
        header: NodeHeader {
            id,
            parent_id: page_id().into_canonical(),
            bounds: RectEmu::new(
                LengthEmu::new(x),
                LengthEmu::new(100_000),
                LengthEmu::new(800_000),
                LengthEmu::new(300_000),
            ),
            transform: Affine2D::identity(),
            source_refs: Vec::new(),
            extensions: Vec::new(),
        },
        payload: PubResolvedNodePayload {
            contents_seq_num: 100 + ordinal,
            officeart_shape_type: Some(202),
            officeart_spid: Some(100 + ordinal),
            image_slot: None,
            explicit_image_crop: None,
            explicit_paint: PubExplicitShapePaintSource::default(),
            story_frame: Some(PubResolvedStoryFrame {
                story_id: Some(source_story_id()),
                ordinal,
                previous_frame: previous,
                next_frame: next,
            }),
            table_story: None,
            table: None,
        },
    }
}

fn graph() -> PubResolvedGraph {
    let a = frame_id(0x31);
    let b = frame_id(0x32);
    let c = frame_id(0x33);
    let source = SourceDescriptor {
        format: "synthetic-break-link".into(),
        format_version: Some("v1".into()),
        adapter_version: "break-link-test-v1".into(),
        source_hash: source_hash(),
    };
    let document = Document {
        id: document_id(),
        format_origin: "synthetic-break-link".into(),
        source_hash: source_hash(),
        pages: vec![page_id()],
        resources: Vec::new(),
        styles: Vec::new(),
    };
    let page = Page {
        id: page_id(),
        size: Size2D::new(LengthEmu::new(4_000_000), LengthEmu::new(2_000_000)),
        bleed: None,
        margins: None,
        children: vec![a, b, c],
        extensions: Vec::new(),
    };

    ResolvedGraph {
        cdm_version: "0.1".into(),
        resolver_version: "break-link-test-v1".into(),
        source,
        document,
        pages: BTreeMap::from([(page_id(), page)]),
        nodes: BTreeMap::from([
            (a, text_frame(a, 0, None, Some(b), 100_000)),
            (b, text_frame(b, 1, Some(a), Some(c), 1_100_000)),
            (c, text_frame(c, 2, Some(b), None, 2_100_000)),
        ]),
        stories: BTreeMap::from([(source_story_id(), story())]),
        paragraphs: BTreeMap::new(),
        text_runs: BTreeMap::new(),
        resources: BTreeMap::new(),
        styles: BTreeMap::new(),
        extensions: BTreeMap::new(),
    }
}

fn frame(graph: &PubResolvedGraph, id: NodeId) -> &PubResolvedStoryFrame {
    graph.nodes[&id]
        .payload
        .story_frame
        .as_ref()
        .expect("text frame")
}

#[test]
fn break_link_splits_topology_without_moving_or_copying_story_text() {
    let baseline = graph();
    let baseline_nodes = baseline.nodes.clone();
    let source_story = baseline.stories[&source_story_id()].clone();
    let source_hash_before = baseline.source.source_hash;
    let document_source_hash_before = baseline.document.source_hash;
    let mut session = EditorSession::new(baseline.clone()).expect("open editor");
    let a = frame_id(0x31);
    let b = frame_id(0x32);
    let c = frame_id(0x33);

    let operation = session
        .break_text_frame_forward_link(a, b, new_story_id())
        .expect("break explicit A->B");

    let EditOperation::BreakTextFrameForwardLink {
        story_id,
        upstream_frame_id,
        downstream_frame_id,
        new_story_id: actual_new_story_id,
        before_frames,
        after_frames,
    } = &operation
    else {
        panic!("expected BreakTextFrameForwardLink");
    };
    assert_eq!(*story_id, source_story_id());
    assert_eq!(*upstream_frame_id, a);
    assert_eq!(*downstream_frame_id, b);
    assert_eq!(*actual_new_story_id, new_story_id());
    assert_eq!(before_frames.len(), 3);
    assert_eq!(after_frames.len(), 3);

    let current = session.graph();
    assert_eq!(current.stories[&source_story_id()], source_story);
    assert_eq!(current.source.source_hash, source_hash_before);
    assert_eq!(current.document.source_hash, document_source_hash_before);
    assert_eq!(current.stories[&new_story_id()].text, "");
    assert!(current.stories[&new_story_id()].paragraphs.is_empty());
    assert!(current.stories[&new_story_id()].runs.is_empty());
    assert!(current.stories[&new_story_id()].source_refs.is_empty());

    assert_eq!(frame(current, a).story_id, Some(source_story_id()));
    assert_eq!(frame(current, a).next_frame, None);
    assert_eq!(frame(current, b).story_id, Some(new_story_id()));
    assert_eq!(frame(current, b).previous_frame, None);
    assert_eq!(frame(current, b).next_frame, Some(c));
    assert_eq!(frame(current, c).story_id, Some(new_story_id()));
    assert_eq!(frame(current, c).previous_frame, Some(b));
    assert_eq!(frame(current, c).next_frame, None);

    for id in [a, b, c] {
        assert_eq!(current.nodes[&id].header, baseline_nodes[&id].header);
        assert_eq!(
            current.nodes[&id].payload.contents_seq_num,
            baseline_nodes[&id].payload.contents_seq_num
        );
        assert_eq!(
            current.nodes[&id].payload.explicit_paint,
            baseline_nodes[&id].payload.explicit_paint
        );
    }

    let project = session.project();
    assert_eq!(project.schema_version, EDITOR_PROJECT_VERSION_V0_6);
    assert_eq!(project.operations, vec![operation]);
    let requirements = session.persistence_requirements();
    assert!(requirements.iter().any(|item| item.feature == "story.linked_frames"));
    assert!(requirements.iter().any(|item| item.feature == "story.created_identity"));
}

#[test]
fn undo_redo_and_fresh_project_replay_are_exact_and_reuse_story_id() {
    let baseline = graph();
    let mut session = EditorSession::new(baseline.clone()).expect("open editor");
    let a = frame_id(0x31);
    let b = frame_id(0x32);

    let operation = session
        .break_text_frame_forward_link(a, b, new_story_id())
        .expect("break explicit A->B");
    let split_graph = session.graph().clone();
    let project = session.project();

    session.undo().expect("undo");
    assert_eq!(session.graph(), &baseline);
    assert!(!session.graph().stories.contains_key(&new_story_id()));

    session.redo().expect("redo");
    assert_eq!(session.graph(), &split_graph);
    assert_eq!(session.project().operations, vec![operation.clone()]);

    let mut reopened = EditorSession::new(baseline).expect("fresh editor");
    reopened.apply_project(&project).expect("replay project");
    assert_eq!(reopened.graph(), &split_graph);
    assert_eq!(reopened.project(), project);

    let EditOperation::BreakTextFrameForwardLink {
        new_story_id: replayed_id,
        ..
    } = &reopened.project().operations[0]
    else {
        panic!("expected persisted break operation");
    };
    assert_eq!(*replayed_id, new_story_id());
}

#[test]
fn idml_and_odg_exports_serialize_the_split_graph() {
    let mut session = EditorSession::new(graph()).expect("open editor");
    session
        .break_text_frame_forward_link(frame_id(0x31), frame_id(0x32), new_story_id())
        .expect("break explicit A->B");

    let idml = session
        .export_editable(EditorEditableTarget::Idml, "break-link-test")
        .expect("IDML export");
    let odg = session
        .export_editable(EditorEditableTarget::Odg, "break-link-test")
        .expect("ODG export");

    assert!(!idml.bytes.is_empty());
    assert!(!odg.bytes.is_empty());
    assert!(idml.report.can_serialize);
    assert!(odg.report.can_serialize);
}

#[test]
fn ambiguous_or_non_explicit_topology_fails_closed_without_mutation() {
    let mut ambiguous = graph();
    let d = frame_id(0x34);
    ambiguous.pages.get_mut(&page_id()).unwrap().children.push(d);
    ambiguous.nodes.insert(
        d,
        text_frame(d, 3, None, None, 3_100_000),
    );
    let baseline = ambiguous.clone();
    let mut session = EditorSession::new(ambiguous).expect("open editor");

    let error = session
        .break_text_frame_forward_link(frame_id(0x31), frame_id(0x32), new_story_id())
        .expect_err("multiple heads must be ambiguous");
    assert!(matches!(error, EditorError::BreakLinkUnsupported { .. }));
    assert_eq!(session.graph(), &baseline);

    let mut no_edge = EditorSession::new(graph()).expect("open editor");
    let error = no_edge
        .break_text_frame_forward_link(frame_id(0x31), frame_id(0x33), new_story_id())
        .expect_err("A->C is not an explicit edge");
    assert!(matches!(error, EditorError::BreakLinkUnsupported { .. }));
}

#[test]
fn new_story_identity_must_be_fresh_uuid_v7() {
    let mut invalid = EditorSession::new(graph()).expect("open editor");
    let not_v7 = StoryId::from_canonical(canonical(0x55));
    let error = invalid
        .break_text_frame_forward_link(frame_id(0x31), frame_id(0x32), not_v7)
        .expect_err("non-v7 StoryId must fail");
    assert!(matches!(error, EditorError::NewStoryIdInvalid { .. }));

    let mut conflict_graph = graph();
    conflict_graph
        .stories
        .insert(new_story_id(), Story {
            id: new_story_id(),
            text: String::new(),
            paragraphs: Vec::new(),
            runs: Vec::new(),
            fields: Vec::new(),
            hyperlinks: Vec::new(),
            source_refs: Vec::new(),
        });
    let mut conflict = EditorSession::new(conflict_graph).expect("open editor");
    let error = conflict
        .break_text_frame_forward_link(frame_id(0x31), frame_id(0x32), new_story_id())
        .expect_err("existing StoryId must fail");
    assert!(matches!(error, EditorError::NewStoryIdConflict { .. }));

    // A different preallocated UUIDv7 remains admissible.
    let mut good = EditorSession::new(graph()).expect("open editor");
    good.break_text_frame_forward_link(
        frame_id(0x31),
        frame_id(0x32),
        other_new_story_id(),
    )
    .expect("fresh UUIDv7");
}
