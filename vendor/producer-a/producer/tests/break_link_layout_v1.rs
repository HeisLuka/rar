use pub_editor::EditorSession;
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
fn story_id() -> StoryId {
    StoryId::from_canonical(canonical(0x40))
}
fn new_story_id() -> StoryId {
    serde_json::from_str("\"01890f47-0c00-7abc-8def-0123456789ab\"").unwrap()
}
fn source_hash() -> Sha256Digest {
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        .parse()
        .unwrap()
}

fn frame(
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
                LengthEmu::new(2_500_000),
                LengthEmu::new(220_000),
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
                story_id: Some(story_id()),
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
        format: "synthetic-break-link-layout".into(),
        format_version: Some("v1".into()),
        adapter_version: "break-link-layout-v1".into(),
        source_hash: source_hash(),
    };
    let document = Document {
        id: document_id(),
        format_origin: "synthetic-break-link-layout".into(),
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
    let story = Story {
        id: story_id(),
        // Long enough to require more than one frame but short enough for A+B+C.
        text: "Chaptera linked text keeps every scalar upstream when a forward link is broken."
            .into(),
        paragraphs: Vec::new(),
        runs: Vec::new(),
        fields: Vec::new(),
        hyperlinks: Vec::new(),
        source_refs: Vec::new(),
    };

    ResolvedGraph {
        cdm_version: "0.1".into(),
        resolver_version: "break-link-layout-v1".into(),
        source,
        document,
        pages: BTreeMap::from([(page_id(), page)]),
        nodes: BTreeMap::from([
            (a, frame(a, 0, None, Some(b), 100_000)),
            (b, frame(b, 1, Some(a), Some(c), 1_000_000)),
            (c, frame(c, 2, Some(b), None, 1_900_000)),
        ]),
        stories: BTreeMap::from([(story_id(), story)]),
        paragraphs: BTreeMap::new(),
        text_runs: BTreeMap::new(),
        resources: BTreeMap::new(),
        styles: BTreeMap::new(),
        extensions: BTreeMap::new(),
    }
}

fn authoring_slice(graph: &PubResolvedGraph) -> BoundedAuthoringSlice {
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

fn has_overset(graph: &PubResolvedGraph) -> bool {
    let font = font_test_data::NOTOSERIF_AUTOHINT_SHAPING;
    let runtime = BoundedShapedFlowRuntime {
        shaping: BoundedShapingRuntime {
            layout: BoundedLayoutEnvironment {
                engine_revision: "break-link-layout-v1".into(),
                font_set_fingerprint: font_fingerprint_sha256(font),
                resource_fingerprint: "resources:none".into(),
            },
            face_index: 0,
            font_size_emu: LengthEmu::new(12 * EMU_PER_POINT),
            font_bytes: font,
        },
        line_height: LengthEmu::new(220_000),
    };
    let projected = project_bounded(authoring_slice(graph));
    let scene = resolve_bounded_shaped_flow(&projected, &runtime).expect("resolve shaped flow");
    scene
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "story_overset")
}

#[test]
fn breaking_capacity_recomputes_upstream_as_overset_without_moving_text() {
    let mut session = EditorSession::new(graph()).expect("open editor");
    let original_text = session.graph().stories[&story_id()].text.clone();

    assert!(
        !has_overset(session.graph()),
        "three-frame baseline should fit the bounded test Story"
    );

    session
        .break_text_frame_forward_link(frame_id(0x31), frame_id(0x32), new_story_id())
        .expect("break A->B");

    assert_eq!(session.graph().stories[&story_id()].text, original_text);
    assert_eq!(session.graph().stories[&new_story_id()].text, "");
    assert!(
        has_overset(session.graph()),
        "losing B+C capacity must expose upstream overset"
    );
}
