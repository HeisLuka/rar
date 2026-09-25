use pub_editor::Sha256Digest;
use pub_editor::{
    EDITOR_PROJECT_VERSION_V0_5, EditOperation, EditorProject, EditorProjectAsset,
    EditorProjectForkProvenance, EditorProjectIdentity,
};

fn project_with_identity() -> EditorProject {
    EditorProject {
        schema_version: EDITOR_PROJECT_VERSION_V0_5.to_owned(),
        source_hash: Sha256Digest::from_bytes([0x11; 32]),
        identity: Some(EditorProjectIdentity {
            project_id: "018f0000-0000-7000-8000-000000000001".to_owned(),
            document_id: "018f0000-0000-7000-8000-000000000002".to_owned(),
            history_id: "018f0000-0000-7000-8000-000000000003".to_owned(),
            genesis_revision_id: "018f0000-0000-7000-8000-000000000004".to_owned(),
            forked_from: None,
        }),
        assets: Vec::new(),
        operations: Vec::new(),
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
        schema_version: "pub-editor-v0.4".to_owned(),
        source_hash: Sha256Digest::from_bytes([0x33; 32]),
        identity: None,
        assets: Vec::new(),
        operations: Vec::<EditOperation>::new(),
    };

    assert!(project.fork_next_issue().is_err());
}
