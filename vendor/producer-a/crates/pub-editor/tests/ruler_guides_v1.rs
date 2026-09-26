use std::collections::BTreeMap;

use pub_editor::{
    EDITOR_PROJECT_VERSION_CURRENT, EDITOR_PROJECT_VERSION_V0_11, EDITOR_PROJECT_VERSION_V0_12,
    EditOperation, EditorError, EditorProjectError, EditorSession, LengthEmu, RulerGuideAxis,
};
use pub_model::{
    Document, DocumentId, Page, PageId, ResolvedGraph, Sha256Digest, Size2D, SourceDescriptor,
};
use pub_reader::{PubResolvedGraph, PubResolvedNodePayload};

fn canonical_id<T: serde::de::DeserializeOwned>(value: &str) -> T {
    serde_json::from_str(&format!("\"{value}\"")).expect("canonical typed id")
}

fn source_hash() -> Sha256Digest {
    Sha256Digest::from_bytes([0x61; 32])
}

fn page_id() -> PageId {
    canonical_id("11000000-0000-4000-8000-000000000001")
}

fn graph() -> PubResolvedGraph {
    let source_hash = source_hash();
    let page_id = page_id();

    ResolvedGraph {
        cdm_version: "0.1".into(),
        resolver_version: "ruler-guide-authoring-test".into(),
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
                size: Size2D::new(LengthEmu::new(8_000_000), LengthEmu::new(10_000_000)),
                bleed: None,
                margins: None,
                children: Vec::new(),
                extensions: Vec::new(),
            },
        )]),
        nodes: BTreeMap::<_, pub_model::Node<PubResolvedNodePayload>>::new(),
        stories: BTreeMap::new(),
        paragraphs: BTreeMap::new(),
        text_runs: BTreeMap::new(),
        resources: BTreeMap::new(),
        styles: BTreeMap::new(),
        extensions: BTreeMap::new(),
    }
}

#[test]
fn add_move_delete_are_canonical_history_units_with_exact_emu_state() {
    assert_eq!(EDITOR_PROJECT_VERSION_CURRENT, EDITOR_PROJECT_VERSION_V0_12);

    let base = graph();
    let mut session = EditorSession::new(base.clone()).expect("session");

    let add = session
        .add_ruler_guide(
            page_id(),
            RulerGuideAxis::Vertical,
            LengthEmu::new(1_000_000),
        )
        .expect("add guide");
    let guide = match add {
        EditOperation::AddRulerGuide { guide } => guide,
        other => panic!("unexpected add operation: {other:?}"),
    };
    let id_bytes = guide.guide_id.as_bytes();
    assert_eq!(id_bytes[6] >> 4, 0x7, "guide identity must be UUIDv7");
    assert_eq!(guide.page_id, page_id());
    assert_eq!(guide.axis, RulerGuideAxis::Vertical);
    assert_eq!(guide.position, LengthEmu::new(1_000_000));
    assert_eq!(
        session.graph(),
        &base,
        "guides live in authored overlay state"
    );
    assert_eq!(session.authored_ruler_guide(guide.guide_id), Some(&guide));
    assert_eq!(session.operations().len(), 1);

    let moved = session
        .move_ruler_guide(guide.guide_id, LengthEmu::new(2_000_000))
        .expect("move guide");
    assert!(matches!(
        moved,
        EditOperation::MoveRulerGuide {
            before_position,
            after_position,
            ..
        } if before_position == LengthEmu::new(1_000_000)
            && after_position == LengthEmu::new(2_000_000)
    ));
    assert_eq!(
        session
            .authored_ruler_guide(guide.guide_id)
            .expect("moved guide")
            .position,
        LengthEmu::new(2_000_000)
    );

    session.undo().expect("undo move");
    assert_eq!(
        session
            .authored_ruler_guide(guide.guide_id)
            .expect("restored guide")
            .position,
        LengthEmu::new(1_000_000)
    );
    session.redo().expect("redo move");
    assert_eq!(
        session
            .authored_ruler_guide(guide.guide_id)
            .expect("moved again")
            .position,
        LengthEmu::new(2_000_000)
    );

    let delete = session
        .delete_ruler_guide(guide.guide_id)
        .expect("delete guide");
    assert!(matches!(delete, EditOperation::DeleteRulerGuide { .. }));
    assert!(session.authored_ruler_guide(guide.guide_id).is_none());

    session.undo().expect("undo delete");
    assert_eq!(
        session
            .authored_ruler_guide(guide.guide_id)
            .expect("restored after delete")
            .position,
        LengthEmu::new(2_000_000)
    );
}

#[test]
fn v0_12_project_replay_preserves_guide_identity_state_and_operations() {
    let base = graph();
    let mut session = EditorSession::new(base.clone()).expect("session");
    let add = session
        .add_ruler_guide(
            page_id(),
            RulerGuideAxis::Horizontal,
            LengthEmu::new(1_500_000),
        )
        .expect("add guide");
    let guide_id = match add {
        EditOperation::AddRulerGuide { guide } => guide.guide_id,
        _ => unreachable!(),
    };
    session
        .move_ruler_guide(guide_id, LengthEmu::new(3_000_000))
        .expect("move guide");

    let project = session.project();
    assert_eq!(project.schema_version, EDITOR_PROJECT_VERSION_V0_12);
    assert_eq!(project.operations.len(), 2);
    assert!(
        session
            .persistence_requirements()
            .iter()
            .any(|item| item.feature == "guide.page_ruler")
    );

    let json = serde_json::to_vec(&project).expect("serialize");
    let decoded = serde_json::from_slice(&json).expect("deserialize");

    let mut reopened = EditorSession::new(base).expect("fresh session");
    reopened
        .apply_project(&decoded)
        .expect("replay guide project");
    assert_eq!(reopened.project(), project);
    assert_eq!(
        reopened
            .authored_ruler_guide(guide_id)
            .expect("replayed guide")
            .position,
        LengthEmu::new(3_000_000)
    );
}

#[test]
fn invalid_noop_and_legacy_smuggling_fail_closed() {
    let base = graph();
    let mut session = EditorSession::new(base.clone()).expect("session");

    assert!(matches!(
        session.add_ruler_guide(
            page_id(),
            RulerGuideAxis::Vertical,
            LengthEmu::new(8_000_001),
        ),
        Err(EditorError::RulerGuideOutOfRange { .. })
    ));
    assert!(session.operations().is_empty());

    let add = session
        .add_ruler_guide(page_id(), RulerGuideAxis::Vertical, LengthEmu::new(750_000))
        .expect("valid add");
    let guide_id = match add {
        EditOperation::AddRulerGuide { guide } => guide.guide_id,
        _ => unreachable!(),
    };
    assert!(matches!(
        session.move_ruler_guide(guide_id, LengthEmu::new(750_000)),
        Err(EditorError::RulerGuideNoChange { .. })
    ));

    let mut legacy = session.project();
    legacy.schema_version = EDITOR_PROJECT_VERSION_V0_11.to_owned();

    let mut replay = EditorSession::new(base).expect("replay session");
    assert!(matches!(
        replay.apply_project(&legacy),
        Err(EditorProjectError::LegacyProjectCarriesRulerGuideOperation { .. })
    ));
}

#[test]
fn delete_replay_finishes_with_no_authored_guide_and_preserves_history() {
    let base = graph();
    let mut session = EditorSession::new(base.clone()).expect("session");
    let add = session
        .add_ruler_guide(
            page_id(),
            RulerGuideAxis::Horizontal,
            LengthEmu::new(2_000_000),
        )
        .expect("add");
    let guide_id = match add {
        EditOperation::AddRulerGuide { guide } => guide.guide_id,
        _ => unreachable!(),
    };
    session
        .delete_ruler_guide(guide_id)
        .expect("delete authored guide");
    let project = session.project();

    let mut reopened = EditorSession::new(base).expect("reopen");
    reopened.apply_project(&project).expect("replay add+delete");
    assert!(reopened.authored_ruler_guide(guide_id).is_none());
    assert_eq!(reopened.operations(), project.operations.as_slice());
}
