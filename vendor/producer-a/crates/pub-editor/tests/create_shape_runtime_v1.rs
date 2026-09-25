use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
};

use pub_editor::{
    AuthoredEntityProvenanceV1, AuthoredShapeKindV1, AuthoredShapePaintV1,
    AuthoredShapeTransformV1, AuthoredSolidFillV1, AuthoredSolidStrokeV1,
    EDITOR_PROJECT_VERSION_CURRENT, EDITOR_PROJECT_VERSION_V0_10, EditOperation,
    EditorEditableTarget, EditorError, EditorProject, EditorProjectError, EditorSession, LengthEmu,
    RectEmu, Srgb8V1, mature_0x2c_pub_persistence_target,
};
use pub_export::{CapabilityLevel, PersistenceCompatibilityState, WriterCapabilityManifest};
use pub_model::{
    Affine2D, Document, DocumentId, EMU_PER_POINT, Node, NodeHeader, NodeId, NodeKind, Page,
    PageId, ResolvedGraph, Sha256Digest, Size2D, SourceDescriptor,
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
    "5555555555555555555555555555555555555555555555555555555555555555"
        .parse()
        .expect("sha")
}

fn page_id() -> PageId {
    canonical_id("11000000-0000-4000-8000-000000000001")
}

fn source_node_id() -> NodeId {
    canonical_id("22000000-0000-4000-8000-000000000001")
}

fn authored_node_id() -> NodeId {
    canonical_id("01890f47-0c00-7abc-8def-0123456789ab")
}

fn other_authored_node_id() -> NodeId {
    canonical_id("01890f47-0c01-7abc-8def-0123456789ab")
}

fn payload() -> PubResolvedNodePayload {
    PubResolvedNodePayload {
        contents_seq_num: 7,
        officeart_shape_type: Some(1),
        officeart_spid: Some(7),
        image_slot: None,
        explicit_image_crop: None,
        explicit_paint: PubExplicitShapePaintSource::default(),
        story_frame: None,
        table_story: None,
        table: None,
    }
}

fn graph() -> PubResolvedGraph {
    let page_id = page_id();
    let source_node = source_node_id();
    let source_hash = source_hash();

    let mut pages = BTreeMap::new();
    pages.insert(
        page_id,
        Page {
            id: page_id,
            size: Size2D::new(LengthEmu::new(8_000_000), LengthEmu::new(10_000_000)),
            bleed: None,
            margins: None,
            children: vec![source_node],
            extensions: Vec::new(),
        },
    );

    let mut nodes = BTreeMap::new();
    nodes.insert(
        source_node,
        Node {
            kind: NodeKind::Shape,
            header: NodeHeader {
                id: source_node,
                parent_id: page_id.into_canonical(),
                bounds: rect(10_000, 20_000, 30_000, 40_000),
                transform: Affine2D::identity(),
                source_refs: Vec::new(),
                extensions: Vec::new(),
            },
            payload: payload(),
        },
    );

    ResolvedGraph {
        cdm_version: "0.1".into(),
        resolver_version: "create-shape-runtime-test".into(),
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

fn paint() -> AuthoredShapePaintV1 {
    AuthoredShapePaintV1 {
        fill: AuthoredSolidFillV1 {
            visible: true,
            color: Srgb8V1 {
                r: 10,
                g: 20,
                b: 30,
            },
        },
        stroke: AuthoredSolidStrokeV1 {
            visible: true,
            color: Srgb8V1 {
                r: 40,
                g: 50,
                b: 60,
            },
            width_emu: 12_700,
        },
        provenance: AuthoredEntityProvenanceV1::AuthorCreated,
    }
}

#[test]
fn create_shape_is_one_v0_10_history_unit_and_source_graph_stays_immutable() {
    assert_eq!(EDITOR_PROJECT_VERSION_CURRENT, EDITOR_PROJECT_VERSION_V0_10);
    let base = graph();
    let mut session = EditorSession::new(base.clone()).expect("session");
    let node_id = authored_node_id();

    let operation = session
        .create_shape(
            node_id,
            page_id(),
            rect(-100_000, 250_000, 900_000, 600_000),
            paint(),
        )
        .expect("CreateShape");

    assert!(matches!(operation, EditOperation::CreateShape { .. }));
    assert_eq!(
        session.graph(),
        &base,
        "CreateShape must not mutate imported graph"
    );
    assert!(!session.graph().nodes.contains_key(&node_id));

    let authored = session
        .authored_shape(node_id)
        .expect("authored overlay entity");
    assert_eq!(authored.node_id, node_id);
    assert_eq!(authored.page_id, page_id());
    assert_eq!(authored.parent_id, page_id());
    assert_eq!(authored.shape_kind, AuthoredShapeKindV1::Rectangle);
    assert_eq!(authored.transform, AuthoredShapeTransformV1::Identity);
    assert_eq!(
        authored.provenance,
        AuthoredEntityProvenanceV1::AuthorCreated
    );
    assert_eq!(
        authored.paint.provenance,
        AuthoredEntityProvenanceV1::AuthorCreated
    );

    let project = session.project();
    assert_eq!(project.schema_version, EDITOR_PROJECT_VERSION_V0_10);
    assert_eq!(project.operations, vec![operation.clone()]);
    assert_eq!(session.persistence_requirements().len(), 3);

    session.undo().expect("one undo");
    assert!(session.authored_shape(node_id).is_none());
    assert_eq!(session.graph(), &base);
    assert!(matches!(session.undo(), Err(EditorError::NothingToUndo)));

    session.redo().expect("one redo");
    assert_eq!(
        session.authored_shape(node_id),
        Some(authored_from_operation(&operation).as_ref())
    );

    let mut reopened = EditorSession::new(base).expect("reopen");
    reopened.apply_project(&project).expect("replay");
    assert_eq!(reopened.project(), project);
    assert_eq!(
        reopened.authored_shape(node_id),
        session.authored_shape(node_id),
        "save/reopen must preserve exact authored identity/state"
    );
}

fn authored_from_operation(operation: &EditOperation) -> Box<pub_editor::AuthoredShapeRuntimeV1> {
    let EditOperation::CreateShape {
        node_id,
        page_id,
        parent_id,
        shape_kind,
        bounds,
        transform,
        paint,
        provenance,
    } = operation
    else {
        panic!("expected CreateShape")
    };
    Box::new(pub_editor::AuthoredShapeRuntimeV1 {
        node_id: *node_id,
        page_id: *page_id,
        parent_id: *parent_id,
        shape_kind: *shape_kind,
        bounds: *bounds,
        transform: *transform,
        paint: paint.clone(),
        provenance: *provenance,
    })
}

#[test]
fn create_shape_wire_carries_explicit_typed_semantics() {
    let mut session = EditorSession::new(graph()).expect("session");
    let operation = session
        .create_shape(authored_node_id(), page_id(), rect(1, 2, 3, 4), paint())
        .expect("create");
    let value = serde_json::to_value(operation).expect("wire");
    assert_eq!(value["kind"], "create_shape");
    assert_eq!(value["shape_kind"], "rectangle");
    assert_eq!(value["transform"]["kind"], "identity");
    assert_eq!(value["provenance"]["kind"], "author_created");
    assert_eq!(value["paint"]["provenance"]["kind"], "author_created");
    assert!(value.get("contents_seq_num").is_none());
    assert!(value.get("officeart_spid").is_none());
    assert!(value.get("oid").is_none());
}

#[test]
fn invalid_page_identity_bounds_paint_and_collisions_fail_closed() {
    let mut session = EditorSession::new(graph()).expect("session");

    let missing_page: PageId = canonical_id("11000000-0000-4000-8000-000000000099");
    assert!(matches!(
        session.create_shape(
            authored_node_id(),
            missing_page,
            rect(0, 0, 10, 10),
            paint(),
        ),
        Err(EditorError::CreateShapePageMissing { .. })
    ));

    assert!(matches!(
        session.create_shape(source_node_id(), page_id(), rect(0, 0, 10, 10), paint(),),
        Err(EditorError::CreateShapeIdCollision { .. })
    ));

    let non_v7: NodeId = canonical_id("22000000-0000-4000-8000-000000000099");
    assert!(matches!(
        session.create_shape(non_v7, page_id(), rect(0, 0, 10, 10), paint()),
        Err(EditorError::CreateShapeInvalidNodeId { .. })
    ));

    assert!(matches!(
        session.create_shape(authored_node_id(), page_id(), rect(0, 0, 0, 10), paint(),),
        Err(EditorError::CreateShapeInvalidBounds { .. })
    ));

    let mut bad_paint = paint();
    bad_paint.stroke.width_emu = 0;
    assert!(matches!(
        session.create_shape(authored_node_id(), page_id(), rect(0, 0, 10, 10), bad_paint,),
        Err(EditorError::CreateShapeInvalidPaint { .. })
    ));

    session
        .create_shape(authored_node_id(), page_id(), rect(0, 0, 10, 10), paint())
        .expect("first create");
    assert!(matches!(
        session.create_shape(authored_node_id(), page_id(), rect(20, 20, 10, 10), paint(),),
        Err(EditorError::CreateShapeIdCollision { .. })
    ));
}

#[test]
fn persisted_source_backed_provenance_is_rejected_and_v0_9_cannot_smuggle_create_shape() {
    let node_id = authored_node_id();
    let operation = EditOperation::CreateShape {
        node_id,
        page_id: page_id(),
        parent_id: page_id(),
        shape_kind: AuthoredShapeKindV1::Rectangle,
        bounds: rect(0, 0, 100, 100),
        transform: AuthoredShapeTransformV1::Identity,
        paint: AuthoredShapePaintV1 {
            provenance: AuthoredEntityProvenanceV1::SourceBacked,
            ..paint()
        },
        provenance: AuthoredEntityProvenanceV1::AuthorCreated,
    };

    let current = EditorProject {
        schema_version: EDITOR_PROJECT_VERSION_V0_10.to_owned(),
        source_hash: source_hash(),
        assets: Vec::new(),
        table_grids: Vec::new(),
        operations: vec![operation.clone()],
    };
    let mut session = EditorSession::new(graph()).expect("session");
    assert!(matches!(
        session.apply_project(&current),
        Err(EditorProjectError::Operation {
            index: 0,
            error: EditorError::CreateShapeInvalidProvenance { .. }
        })
    ));
    assert!(session.authored_shape(node_id).is_none());

    let legacy = EditorProject {
        schema_version: "pub-editor-v0.9".to_owned(),
        ..current
    };
    let mut session = EditorSession::new(graph()).expect("legacy session");
    assert!(matches!(
        session.apply_project(&legacy),
        Err(EditorProjectError::LegacyProjectCarriesCreateShapeOperation { index: 0 })
    ));
}

#[test]
fn native_pub_persistence_does_not_overclaim_created_shape_support() {
    let mut session = EditorSession::new(graph()).expect("session");
    session
        .create_shape(
            other_authored_node_id(),
            page_id(),
            rect(0, 0, 100, 200),
            paint(),
        )
        .expect("create");

    let assessment = session
        .assess_mature_0x2c_pub_persistence(&WriterCapabilityManifest {
            target: mature_0x2c_pub_persistence_target(),
            writer_version: "test-no-create-shape-writer".into(),
            features: BTreeMap::new(),
            scoped: Vec::new(),
        })
        .expect("assessment");

    assert_eq!(
        assessment.state,
        PersistenceCompatibilityState::NotEvaluated
    );
    assert!(assessment.items.iter().any(|item| {
        item.requirement.feature == "node.created_identity"
            && item.state == PersistenceCompatibilityState::NotEvaluated
            && item.format.is_none()
    }));
    assert!(assessment.items.iter().any(|item| {
        item.requirement.feature == "shape.paint"
            && item.state == PersistenceCompatibilityState::NotEvaluated
            && item.format.is_none()
    }));
}

fn zip_text(bytes: &[u8], path: &str) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("valid editable package zip");
    let mut entry = archive.by_name(path).expect("expected package part");
    let mut text = String::new();
    entry.read_to_string(&mut text).expect("UTF-8 XML part");
    text
}

fn zip_first_text_matching(bytes: &[u8], predicate: impl Fn(&str) -> bool) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("valid editable package zip");
    let path = (0..archive.len())
        .find_map(|index| {
            let entry = archive.by_index(index).ok()?;
            predicate(entry.name()).then(|| entry.name().to_owned())
        })
        .expect("matching package part");
    let mut entry = archive.by_name(&path).expect("matching package part");
    let mut text = String::new();
    entry.read_to_string(&mut text).expect("UTF-8 XML part");
    text
}

#[test]
fn authored_rectangle_survives_reopen_and_materializes_in_idml_and_odg_with_explicit_order_loss() {
    let base = graph();
    let mut session = EditorSession::new(base.clone()).expect("session");
    let node_id = authored_node_id();
    let bounds = rect(
        10 * EMU_PER_POINT,
        20 * EMU_PER_POINT,
        30 * EMU_PER_POINT,
        40 * EMU_PER_POINT,
    );
    session
        .create_shape(node_id, page_id(), bounds, paint())
        .expect("CreateShape");

    let project = session.project();
    let mut reopened = EditorSession::new(base.clone()).expect("reopen");
    reopened.apply_project(&project).expect("replay project");
    assert_eq!(
        reopened.graph(),
        &base,
        "editable export must not mutate source graph"
    );
    assert_eq!(
        reopened.authored_shape(node_id),
        session.authored_shape(node_id)
    );

    for target in [EditorEditableTarget::Idml, EditorEditableTarget::Odg] {
        let preview = reopened
            .preview_editable_export(target, "fixture.pub")
            .expect("preview authored rectangle export");
        let export = reopened
            .export_editable(target, "fixture.pub")
            .expect("serialize authored rectangle export");
        assert_eq!(preview.report, export.report);
        assert!(export.report.can_serialize);

        for feature in [
            "node.created_identity",
            "node.geometry.bounds",
            "shape.paint",
        ] {
            let item = export
                .report
                .items
                .iter()
                .find(|item| {
                    item.feature == feature && item.origin == Some(node_id.into_canonical())
                })
                .expect("authored shape report item");
            assert_eq!(item.disposition, CapabilityLevel::Preserved);
            assert!(item.loss_kind.is_none());
        }

        let order = export
            .report
            .items
            .iter()
            .find(|item| {
                item.feature == "page.object_order"
                    && item.origin == Some(page_id().into_canonical())
            })
            .expect("explicit authored ordering loss");
        assert_ne!(order.disposition, CapabilityLevel::Preserved);
        assert!(order.loss_kind.is_some());

        match target {
            EditorEditableTarget::Idml => {
                let spread = zip_first_text_matching(&export.bytes, |path| {
                    path.starts_with("Spreads/Spread_") && path.ends_with(".xml")
                });
                assert!(spread.contains("01890f470c007abc8def0123456789ab"));
                assert!(spread.contains("Anchor=\"10 20\""));
                assert!(spread.contains("Anchor=\"10 60\""));
                assert!(spread.contains("Anchor=\"40 60\""));
                assert!(spread.contains("Anchor=\"40 20\""));
                assert!(spread.contains("FillColor=\"Color/Chaptera_RGB_10_20_30\""));
                assert!(spread.contains("StrokeColor=\"Color/Chaptera_RGB_40_50_60\""));
                assert!(spread.contains("StrokeWeight=\"1\""));

                let graphic = zip_text(&export.bytes, "Resources/Graphic.xml");
                assert!(graphic.contains("ColorValue=\"10 20 30\""));
                assert!(graphic.contains("ColorValue=\"40 50 60\""));
                let designmap = zip_text(&export.bytes, "designmap.xml");
                assert!(designmap.contains("src=\"Resources/Graphic.xml\""));
            }
            EditorEditableTarget::Odg => {
                let content = zip_text(&export.bytes, "content.xml");
                assert!(content.contains("Shape_01890f470c007abc8def0123456789ab"));
                assert!(content.contains("svg:x=\"10pt\""));
                assert!(content.contains("svg:y=\"20pt\""));
                assert!(content.contains("svg:width=\"30pt\""));
                assert!(content.contains("svg:height=\"40pt\""));
                assert!(content.contains("draw:fill-color=\"#0a141e\""));
                assert!(content.contains("svg:stroke-color=\"#28323c\""));
                assert!(content.contains("svg:stroke-width=\"1pt\""));
                let rect_start = content
                    .find("<draw:rect")
                    .expect("authored rectangle element");
                let rect_end = content[rect_start..]
                    .find("/>")
                    .map(|offset| rect_start + offset)
                    .expect("authored rectangle close");
                assert!(
                    !content[rect_start..rect_end].contains("draw:z-index"),
                    "authored ordering remains explicit/unknown, not guessed"
                );
            }
        }
    }
}
