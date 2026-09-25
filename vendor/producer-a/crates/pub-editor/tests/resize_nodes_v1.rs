use std::collections::BTreeMap;

use pub_editor::{
    EDITOR_PROJECT_VERSION_CURRENT, EDITOR_PROJECT_VERSION_V0_9, EDITOR_PROJECT_VERSION_V0_10,
    EditOperation, EditorError, EditorProject, EditorProjectError, EditorSession, LengthEmu,
    RectEmu, ResizeNodeBatchEntry,
};
use pub_model::{
    Affine2D, Document, DocumentId, Node, NodeHeader, NodeId, NodeKind, Page, PageId,
    ResolvedGraph, Sha256Digest, Size2D, SourceDescriptor,
};
use pub_reader::{PubExplicitShapePaintSource, PubResolvedGraph, PubResolvedNodePayload};

fn canonical_id<T: serde::de::DeserializeOwned>(value: &str) -> T {
    serde_json::from_str(&format!("\"{value}\"")).expect("canonical typed id")
}

fn rect(x: i64, y: i64, width: i64, height: i64) -> RectEmu {
    RectEmu::new(
        LengthEmu::new(x),
        LengthEmu::new(y),
        LengthEmu::new(width),
        LengthEmu::new(height),
    )
}

fn source_hash() -> Sha256Digest {
    "2222222222222222222222222222222222222222222222222222222222222222"
        .parse()
        .expect("sha")
}

fn payload(seq: u32) -> PubResolvedNodePayload {
    PubResolvedNodePayload {
        contents_seq_num: seq,
        officeart_shape_type: Some(1),
        officeart_spid: Some(seq),
        image_slot: None,
        explicit_image_crop: None,
        explicit_paint: PubExplicitShapePaintSource::default(),
        story_frame: None,
        table_story: None,
        table: None,
    }
}

fn ids() -> (PageId, NodeId, NodeId) {
    (
        canonical_id("11000000-0000-4000-8000-000000000001"),
        canonical_id("22000000-0000-4000-8000-000000000001"),
        canonical_id("22000000-0000-4000-8000-000000000002"),
    )
}

fn graph() -> PubResolvedGraph {
    let (page_id, node_a, node_b) = ids();
    let source_hash = source_hash();
    let mut pages = BTreeMap::new();
    pages.insert(
        page_id,
        Page {
            id: page_id,
            size: Size2D::new(LengthEmu::new(5_000_000), LengthEmu::new(5_000_000)),
            bleed: None,
            margins: None,
            children: vec![node_a, node_b],
            extensions: Vec::new(),
        },
    );

    let mut nodes = BTreeMap::new();
    for (node_id, bounds, seq) in [
        (node_a, rect(100_000, 200_000, 300_000, 400_000), 1),
        (node_b, rect(900_000, 800_000, 500_000, 600_000), 2),
    ] {
        nodes.insert(
            node_id,
            Node {
                kind: NodeKind::Shape,
                header: NodeHeader {
                    id: node_id,
                    parent_id: page_id.into_canonical(),
                    bounds,
                    transform: Affine2D::identity(),
                    source_refs: Vec::new(),
                    extensions: Vec::new(),
                },
                payload: payload(seq),
            },
        );
    }

    ResolvedGraph {
        cdm_version: "0.1".into(),
        resolver_version: "resize-nodes-runtime-test".into(),
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
        nodes,
        stories: BTreeMap::new(),
        paragraphs: BTreeMap::new(),
        text_runs: BTreeMap::new(),
        resources: BTreeMap::new(),
        styles: BTreeMap::new(),
        extensions: BTreeMap::new(),
    }
}

fn entries(base: &PubResolvedGraph) -> Vec<ResizeNodeBatchEntry> {
    let (_, node_a, node_b) = ids();
    let before_a = base.nodes[&node_a].header.bounds;
    let before_b = base.nodes[&node_b].header.bounds;
    vec![
        ResizeNodeBatchEntry {
            node_id: node_b,
            before: before_b,
            after: rect(
                before_b.x.get() - 25_000,
                before_b.y.get() + 75_000,
                before_b.width.get() + 100_000,
                before_b.height.get() + 120_000,
            ),
        },
        ResizeNodeBatchEntry {
            node_id: node_a,
            before: before_a,
            // One member may translate without resizing as long as the batch
            // contains a genuine size change elsewhere. This mirrors the
            // canonical Python ResizeNodesV1 law used by aggregate planning.
            after: rect(
                before_a.x.get() + 50_000,
                before_a.y.get() - 10_000,
                before_a.width.get(),
                before_a.height.get(),
            ),
        },
    ]
}

#[test]
fn canonical_batch_is_one_history_and_project_replay_unit() {
    assert_eq!(EDITOR_PROJECT_VERSION_CURRENT, EDITOR_PROJECT_VERSION_V0_10);
    let base = graph();
    let (page_id, node_a, node_b) = ids();
    let mut batch = entries(&base);
    batch.sort_by_key(|entry| entry.node_id);
    let operation = EditOperation::ResizeNodes {
        page_id,
        entries: batch.clone(),
    };
    let project = EditorProject {
        schema_version: EDITOR_PROJECT_VERSION_V0_9.to_owned(),
        source_hash: source_hash(),
        assets: Vec::new(),
        table_grids: Vec::new(),
        operations: vec![operation.clone()],
    };
    let after_a = batch
        .iter()
        .find(|entry| entry.node_id == node_a)
        .unwrap()
        .after;
    let after_b = batch
        .iter()
        .find(|entry| entry.node_id == node_b)
        .unwrap()
        .after;

    let mut session = EditorSession::new(base.clone()).expect("session");
    session
        .apply_project(&project)
        .expect("canonical batch replay");
    assert_eq!(session.operations(), std::slice::from_ref(&operation));
    assert_eq!(session.graph().nodes[&node_a].header.bounds, after_a);
    assert_eq!(session.graph().nodes[&node_b].header.bounds, after_b);
    assert_eq!(session.project(), project);
    assert_eq!(session.persistence_requirements().len(), 2);

    session.undo().expect("one batch undo");
    assert_eq!(
        session.graph().nodes[&node_a].header.bounds,
        base.nodes[&node_a].header.bounds
    );
    assert_eq!(
        session.graph().nodes[&node_b].header.bounds,
        base.nodes[&node_b].header.bounds
    );
    assert!(matches!(session.undo(), Err(EditorError::NothingToUndo)));

    session.redo().expect("one batch redo");
    assert_eq!(session.graph().nodes[&node_a].header.bounds, after_a);
    assert_eq!(session.graph().nodes[&node_b].header.bounds, after_b);

    let mut reopened = EditorSession::new(base).expect("reopen");
    reopened
        .apply_project(&project)
        .expect("save/reopen replay");
    assert_eq!(reopened.project(), project);
    assert_eq!(reopened.graph().nodes[&node_a].header.bounds, after_a);
    assert_eq!(reopened.graph().nodes[&node_b].header.bounds, after_b);
}

#[test]
fn stale_later_member_rejects_whole_batch_without_partial_mutation() {
    let base = graph();
    let (page_id, node_a, node_b) = ids();
    let before_a = base.nodes[&node_a].header.bounds;
    let before_b = base.nodes[&node_b].header.bounds;
    let mut batch = entries(&base);
    batch.sort_by_key(|entry| entry.node_id);
    let stale = batch
        .iter_mut()
        .find(|entry| entry.node_id == node_b)
        .unwrap();
    stale.before.x = LengthEmu::new(stale.before.x.get() + 1);
    let project = EditorProject {
        schema_version: EDITOR_PROJECT_VERSION_V0_9.to_owned(),
        source_hash: source_hash(),
        assets: Vec::new(),
        table_grids: Vec::new(),
        operations: vec![EditOperation::ResizeNodes {
            page_id,
            entries: batch,
        }],
    };
    let mut session = EditorSession::new(base).expect("session");

    assert!(matches!(
        session.apply_project(&project),
        Err(EditorProjectError::Operation {
            index: 0,
            error: EditorError::StaleNodeResize { node_id }
        }) if node_id == node_b
    ));
    assert_eq!(session.graph().nodes[&node_a].header.bounds, before_a);
    assert_eq!(session.graph().nodes[&node_b].header.bounds, before_b);
    assert!(session.operations().is_empty());
}

#[test]
fn count_duplicate_translation_only_and_wrong_page_fail_closed() {
    let base = graph();
    let (page_id, node_a, _) = ids();
    let one = entries(&base)
        .into_iter()
        .find(|entry| entry.node_id == node_a)
        .unwrap();

    let too_small = EditorProject {
        schema_version: EDITOR_PROJECT_VERSION_V0_9.to_owned(),
        source_hash: source_hash(),
        assets: Vec::new(),
        table_grids: Vec::new(),
        operations: vec![EditOperation::ResizeNodes {
            page_id,
            entries: vec![one.clone()],
        }],
    };
    let mut session = EditorSession::new(base.clone()).expect("session");
    assert!(matches!(
        session.apply_project(&too_small),
        Err(EditorProjectError::Operation {
            index: 0,
            error: EditorError::ResizeNodesInvalidCount { found: 1 }
        })
    ));

    let duplicate = EditorProject {
        schema_version: EDITOR_PROJECT_VERSION_V0_9.to_owned(),
        source_hash: source_hash(),
        assets: Vec::new(),
        table_grids: Vec::new(),
        operations: vec![EditOperation::ResizeNodes {
            page_id,
            entries: vec![one.clone(), one.clone()],
        }],
    };
    let mut session = EditorSession::new(base.clone()).expect("session");
    assert!(matches!(
        session.apply_project(&duplicate),
        Err(EditorProjectError::Operation {
            index: 0,
            error: EditorError::ResizeNodesDuplicate { node_id }
        }) if node_id == node_a
    ));

    let mut translation_only = entries(&base);
    for entry in &mut translation_only {
        entry.after.width = entry.before.width;
        entry.after.height = entry.before.height;
        if entry.after == entry.before {
            entry.after.x = LengthEmu::new(entry.before.x.get() + 1);
        }
    }
    translation_only.sort_by_key(|entry| entry.node_id);
    let translation_only_project = EditorProject {
        schema_version: EDITOR_PROJECT_VERSION_V0_9.to_owned(),
        source_hash: source_hash(),
        assets: Vec::new(),
        table_grids: Vec::new(),
        operations: vec![EditOperation::ResizeNodes {
            page_id,
            entries: translation_only,
        }],
    };
    let mut session = EditorSession::new(base.clone()).expect("session");
    assert!(matches!(
        session.apply_project(&translation_only_project),
        Err(EditorProjectError::Operation {
            index: 0,
            error: EditorError::ResizeNodesNoSizeChange
        })
    ));

    let other_page: PageId = canonical_id("11000000-0000-4000-8000-000000000099");
    let mut batch = entries(&base);
    batch.sort_by_key(|entry| entry.node_id);
    let wrong_page = EditorProject {
        schema_version: EDITOR_PROJECT_VERSION_V0_9.to_owned(),
        source_hash: source_hash(),
        assets: Vec::new(),
        table_grids: Vec::new(),
        operations: vec![EditOperation::ResizeNodes {
            page_id: other_page,
            entries: batch,
        }],
    };
    let mut session = EditorSession::new(base).expect("session");
    assert!(matches!(
        session.apply_project(&wrong_page),
        Err(EditorProjectError::Operation {
            index: 0,
            error: EditorError::ResizeNodesPageMismatch { page_id: found, .. }
        }) if found == other_page
    ));
}

#[test]
fn v0_8_project_cannot_smuggle_resize_nodes_history_shape() {
    let base = graph();
    let (page_id, _, _) = ids();
    let mut batch = entries(&base);
    batch.sort_by_key(|entry| entry.node_id);
    let project = EditorProject {
        schema_version: "pub-editor-v0.8".to_owned(),
        source_hash: source_hash(),
        assets: Vec::new(),
        table_grids: Vec::new(),
        operations: vec![EditOperation::ResizeNodes {
            page_id,
            entries: batch,
        }],
    };
    let mut session = EditorSession::new(base).expect("session");
    assert!(matches!(
        session.apply_project(&project),
        Err(EditorProjectError::LegacyProjectCarriesResizeNodesOperation { index: 0 })
    ));
    assert!(session.operations().is_empty());
}

#[test]
fn resize_nodes_wire_and_error_codes_are_stable() {
    let base = graph();
    let (page_id, node_a, _) = ids();
    let mut batch = entries(&base);
    batch.sort_by_key(|entry| entry.node_id);
    let operation = EditOperation::ResizeNodes {
        page_id,
        entries: batch,
    };
    let value = serde_json::to_value(operation).expect("serialize ResizeNodes");
    assert_eq!(value["kind"], "resize_nodes");
    assert_eq!(
        value["entries"][0]["node_id"],
        serde_json::to_value(node_a).unwrap()
    );

    assert_eq!(
        EditorError::ResizeNodesInvalidCount { found: 1 }.code(),
        "resize_nodes_invalid_count"
    );
    assert_eq!(
        EditorError::ResizeNodesDuplicate { node_id: node_a }.code(),
        "resize_nodes_duplicate"
    );
    assert_eq!(
        EditorError::ResizeNodesNoSizeChange.code(),
        "resize_nodes_no_size_change"
    );
}
