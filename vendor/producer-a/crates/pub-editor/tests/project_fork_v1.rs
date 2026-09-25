use std::collections::BTreeMap;

use pub_editor::{
    EDITOR_PROJECT_VERSION_V0_11, EditOperation, EditorEditableTarget, EditorProject,
    EditorProjectAsset, EditorProjectForkProvenance, EditorProjectIdentity, EditorSession,
    LengthEmu, RectEmu, Sha256Digest,
};
use pub_model::{
    Affine2D, Document, DocumentId, Node, NodeHeader, NodeId, NodeKind, Page, PageId,
    ResolvedGraph, Size2D, SourceDescriptor,
};
use pub_reader::{PubExplicitShapePaintSource, PubResolvedGraph, PubResolvedNodePayload};

fn project_with_identity() -> EditorProject {
    EditorProject {
        schema_version: EDITOR_PROJECT_VERSION_V0_11.to_owned(),
        source_hash: Sha256Digest::from_bytes([0x11; 32]),
        identity: Some(EditorProjectIdentity {
            project_id: "018f0000-0000-7000-8000-000000000001".to_owned(),
            document_id: "018f0000-0000-7000-8000-000000000002".to_owned(),
            history_id: "018f0000-0000-7000-8000-000000000003".to_owned(),
            genesis_revision_id: "018f0000-0000-7000-8000-000000000004".to_owned(),
            forked_from: None,
        }),
        assets: Vec::new(),
        table_grids: Vec::new(),
        operations: Vec::new(),
    }
}

fn canonical_id<T: serde::de::DeserializeOwned>(value: &str) -> T {
    serde_json::from_str(&format!("\"{value}\"")).expect("canonical typed id")
}

fn graph() -> PubResolvedGraph {
    let source_hash = Sha256Digest::from_bytes([0x44; 32]);
    let page_id = canonical_id::<PageId>("11000000-0000-4000-8000-000000000001");
    let node_id = canonical_id::<NodeId>("22000000-0000-4000-8000-000000000001");

    ResolvedGraph {
        cdm_version: "0.1".into(),
        resolver_version: "project-fork-test".into(),
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
        pages: BTreeMap::from([(
            page_id,
            Page {
                id: page_id,
                size: Size2D::new(LengthEmu::new(5_000_000), LengthEmu::new(5_000_000)),
                bleed: None,
                margins: None,
                children: vec![node_id],
                extensions: Vec::new(),
            },
        )]),
        nodes: BTreeMap::from([(
            node_id,
            Node {
                kind: NodeKind::Shape,
                header: NodeHeader {
                    id: node_id,
                    parent_id: page_id.into_canonical(),
                    bounds: RectEmu::new(
                        LengthEmu::new(100_000),
                        LengthEmu::new(200_000),
                        LengthEmu::new(300_000),
                        LengthEmu::new(400_000),
                    ),
                    transform: Affine2D::identity(),
                    source_refs: Vec::new(),
                    extensions: Vec::new(),
                },
                payload: PubResolvedNodePayload {
                    contents_seq_num: 1,
                    officeart_shape_type: Some(1),
                    officeart_spid: Some(1),
                    image_slot: None,
                    explicit_image_crop: None,
                    explicit_paint: PubExplicitShapePaintSource::default(),
                    story_frame: None,
                    table_story: None,
                    table: None,
                },
            },
        )]),
        stories: BTreeMap::new(),
        paragraphs: BTreeMap::new(),
        text_runs: BTreeMap::new(),
        resources: BTreeMap::new(),
        styles: BTreeMap::new(),
        extensions: BTreeMap::new(),
    }
}

#[test]
fn fork_preserves_effective_state_but_rekeys_history_identity() {
    let parent = project_with_identity();
    let parent_state = parent.state_id_v1();
    let fork = parent
        .fork_next_issue()
        .expect("identity-bearing project should fork");
    let fork_identity = fork.identity.as_ref().expect("fork identity");
    let parent_identity = parent.identity.as_ref().expect("parent identity");

    assert_eq!(fork.source_hash, parent.source_hash);
    assert_eq!(fork.assets, parent.assets);
    assert_eq!(fork.table_grids, parent.table_grids);
    assert_eq!(fork.operations, parent.operations);
    assert_eq!(fork.state_id_v1(), parent_state);

    assert_ne!(fork_identity.project_id, parent_identity.project_id);
    assert_ne!(fork_identity.document_id, parent_identity.document_id);
    assert_ne!(fork_identity.history_id, parent_identity.history_id);
    assert_ne!(
        fork_identity.genesis_revision_id,
        parent_identity.genesis_revision_id
    );

    assert_eq!(
        fork_identity.forked_from,
        Some(EditorProjectForkProvenance {
            project_id: parent_identity.project_id.clone(),
            document_id: parent_identity.document_id.clone(),
            history_id: parent_identity.history_id.clone(),
            state_id: parent_state,
        })
    );
}

#[test]
fn fork_value_diverges_without_mutating_parent() {
    let parent = project_with_identity();
    let mut fork = parent.fork_next_issue().expect("fork");

    fork.assets.push(EditorProjectAsset {
        sha256: Sha256Digest::from_bytes([0x22; 32]),
        mime: "image/png".to_owned(),
        byte_len: 123,
    });

    assert!(parent.assets.is_empty());
    assert_eq!(fork.assets.len(), 1);
    assert_ne!(fork.state_id_v1(), parent.state_id_v1());
}

#[test]
fn legacy_identityless_project_fails_closed_for_fork() {
    let project = EditorProject {
        schema_version: "pub-editor-v0.10".to_owned(),
        source_hash: Sha256Digest::from_bytes([0x33; 32]),
        identity: None,
        assets: Vec::new(),
        table_grids: Vec::new(),
        operations: Vec::<EditOperation>::new(),
    };

    assert!(project.fork_next_issue().is_err());
}

#[test]
fn identity_and_fork_provenance_survive_json_roundtrip() {
    let parent = project_with_identity();
    let fork = parent.fork_next_issue().expect("fork");
    let json = serde_json::to_string(&fork).expect("serialize");
    let decoded: EditorProject = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(decoded, fork);
    assert_eq!(decoded.state_id_v1(), fork.state_id_v1());
}

#[test]
fn parent_and_next_issue_reopen_diverge_and_export_independently() {
    let base = graph();
    let node_id = canonical_id::<NodeId>("22000000-0000-4000-8000-000000000001");
    let parent_session = EditorSession::new(base.clone()).expect("parent session");
    let parent = parent_session.project();
    let parent_bytes_before = serde_json::to_vec(&parent).expect("serialize parent");
    let parent_state = parent.state_id_v1();

    let fork = parent_session
        .fork_project_next_issue()
        .expect("fork current project");
    assert_eq!(fork.state_id_v1(), parent_state);
    assert_ne!(
        fork.identity.as_ref().expect("fork identity").project_id,
        parent.identity.as_ref().expect("parent identity").project_id
    );

    let mut next_issue = EditorSession::new(base.clone()).expect("next issue session");
    next_issue.apply_project(&fork).expect("reopen fork");
    next_issue
        .move_node_to(node_id, LengthEmu::new(700_000), LengthEmu::new(800_000))
        .expect("edit only next issue");
    let final_next_issue = next_issue.project();

    assert_ne!(final_next_issue.state_id_v1(), parent_state);
    assert_eq!(
        final_next_issue
            .identity
            .as_ref()
            .expect("next issue identity")
            .project_id,
        fork.identity.as_ref().expect("fork identity").project_id
    );

    let mut reopened_parent = EditorSession::new(base.clone()).expect("reopen parent");
    reopened_parent.apply_project(&parent).expect("apply parent");
    assert_eq!(
        serde_json::to_vec(&reopened_parent.project()).expect("serialize reopened parent"),
        parent_bytes_before,
        "editing the fork must not mutate the parent project"
    );

    let mut reopened_next = EditorSession::new(base).expect("reopen next issue");
    reopened_next
        .apply_project(&final_next_issue)
        .expect("apply final next issue");
    assert_eq!(reopened_next.project(), final_next_issue);

    let parent_export = reopened_parent
        .export_editable(EditorEditableTarget::Odg, "project-fork-parent")
        .expect("parent export");
    let next_export = reopened_next
        .export_editable(EditorEditableTarget::Odg, "project-fork-next")
        .expect("next issue export");
    assert!(!parent_export.bytes.is_empty());
    assert!(!next_export.bytes.is_empty());
    assert_eq!(reopened_parent.source_hash(), reopened_next.source_hash());
}
