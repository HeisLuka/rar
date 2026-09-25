use pub_editor::{
    EDITOR_PROJECT_VERSION_V0_6, EDITOR_PROJECT_VERSION_V0_7, EditorProject, EditorProjectError,
    EditorSession,
};
use pub_model::{
    Affine2D, CanonicalId, Document, DocumentId, LengthEmu, Node, NodeHeader, NodeId, NodeKind,
    Page, PageId, RectEmu, ResolvedGraph, Sha256Digest, SimpleRectangularTable, SimpleTableCell,
    Size2D, SourceDescriptor, Story, StoryId, TableCellAddress, TableCellId,
};
use pub_reader::{
    PubExplicitShapePaintSource, PubResolvedGraph, PubResolvedNodePayload, PubTableCellCoordinates,
    PubTableCellSource, PubTableLayoutMetricsSource, PubTableSource,
};
use std::collections::BTreeMap;

fn canonical(byte: u8) -> CanonicalId {
    CanonicalId::from_bytes([byte; 16])
}

fn source_hash() -> Sha256Digest {
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        .parse()
        .unwrap()
}

fn graph() -> PubResolvedGraph {
    let source_hash = source_hash();
    let page_id = PageId::from_canonical(canonical(1));
    let table_id = NodeId::from_canonical(canonical(2));
    let story_id = StoryId::from_canonical(canonical(3));
    let cell0 = TableCellId::from_canonical(canonical(10));
    let cell1 = TableCellId::from_canonical(canonical(11));

    let simple = SimpleRectangularTable::new(
        1,
        2,
        vec![
            SimpleTableCell {
                id: cell0,
                address: TableCellAddress { row: 0, column: 0 },
            },
            SimpleTableCell {
                id: cell1,
                address: TableCellAddress { row: 0, column: 1 },
            },
        ],
    )
    .unwrap();

    let table = PubTableSource {
        text_id: 77,
        story_id: Some(story_id),
        rows: 1,
        columns: 2,
        cells_seq_num: 88,
        tcd_story_ordinal: 0,
        cells: vec![
            PubTableCellSource {
                id: cell0,
                stored_record_index: 0,
                coordinates: Some(PubTableCellCoordinates {
                    start_row: 0,
                    end_row: 0,
                    start_column: 0,
                    end_column: 0,
                }),
                utf16_start: 0,
                utf16_end: 1,
                source_refs: Vec::new(),
            },
            PubTableCellSource {
                id: cell1,
                stored_record_index: 1,
                coordinates: Some(PubTableCellCoordinates {
                    start_row: 0,
                    end_row: 0,
                    start_column: 1,
                    end_column: 1,
                }),
                utf16_start: 1,
                utf16_end: 4,
                source_refs: Vec::new(),
            },
        ],
        simple_table: Some(simple),
        layout_metrics: Some(PubTableLayoutMetricsSource {
            story_layout_key: 99,
            cell_width: LengthEmu::new(1_000),
            row_pitch: LengthEmu::new(500),
            source_refs: Vec::new(),
        }),
        source_refs: Vec::new(),
    };

    let source = SourceDescriptor {
        format: "pub-mature-0x2c".into(),
        format_version: Some("0x2c".into()),
        adapter_version: "pub-rs".into(),
        source_hash,
    };

    ResolvedGraph {
        cdm_version: "0.1".into(),
        resolver_version: "test".into(),
        source,
        document: Document {
            id: DocumentId::from_canonical(canonical(4)),
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
                size: Size2D::new(LengthEmu::new(10_000), LengthEmu::new(10_000)),
                bleed: None,
                margins: None,
                children: vec![table_id],
                extensions: Vec::new(),
            },
        )]),
        nodes: BTreeMap::from([(
            table_id,
            Node {
                kind: NodeKind::Table,
                header: NodeHeader {
                    id: table_id,
                    parent_id: page_id.into_canonical(),
                    bounds: RectEmu::new(
                        LengthEmu::new(0),
                        LengthEmu::new(0),
                        LengthEmu::new(2_000),
                        LengthEmu::new(500),
                    ),
                    transform: Affine2D::identity(),
                    source_refs: Vec::new(),
                    extensions: Vec::new(),
                },
                payload: PubResolvedNodePayload {
                    contents_seq_num: 123,
                    officeart_shape_type: None,
                    officeart_spid: None,
                    image_slot: None,
                    explicit_image_crop: None,
                    explicit_paint: PubExplicitShapePaintSource::default(),
                    story_frame: None,
                    table_story: None,
                    table: Some(table),
                },
            },
        )]),
        stories: BTreeMap::from([(
            story_id,
            Story {
                id: story_id,
                text: "A\rB\r".into(),
                paragraphs: Vec::new(),
                runs: Vec::new(),
                fields: Vec::new(),
                hyperlinks: Vec::new(),
                source_refs: Vec::new(),
            },
        )]),
        paragraphs: BTreeMap::new(),
        text_runs: BTreeMap::new(),
        resources: BTreeMap::new(),
        styles: BTreeMap::new(),
        extensions: BTreeMap::new(),
    }
}

#[test]
fn effective_grid_roundtrips_and_replays_exactly() {
    let mut session = EditorSession::new(graph()).unwrap();
    let baseline = session.project();

    assert_eq!(baseline.schema_version, EDITOR_PROJECT_VERSION_V0_6);
    assert_eq!(baseline.table_grids.len(), 1);
    let grid = &baseline.table_grids[0];
    assert_eq!(grid.rows.len(), 1);
    assert_eq!(grid.columns.len(), 2);
    assert_eq!(grid.rows[0].extent, Some(LengthEmu::new(500)));
    assert_eq!(grid.columns[0].extent, Some(LengthEmu::new(1_000)));
    assert_eq!(grid.cells[0].id, TableCellId::from_canonical(canonical(10)));

    let json = serde_json::to_string(&baseline).unwrap();
    let decoded: EditorProject = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, baseline);

    let mut replay = EditorSession::new(graph()).unwrap();
    replay.apply_project(&decoded).unwrap();
    assert_eq!(replay.project(), baseline);

    let row_id = grid.rows[0].id;
    let column0_id = grid.columns[0].id;
    let cell0_id = grid.cells[0].id;

    session
        .replace_table_cell_text(NodeId::from_canonical(canonical(2)), cell0_id, "Alpha")
        .unwrap();

    let edited = session.project();
    assert_eq!(edited.table_grids[0].rows[0].id, row_id);
    assert_eq!(edited.table_grids[0].columns[0].id, column0_id);
    assert_eq!(edited.table_grids[0].cells[0].id, cell0_id);
    assert_ne!(
        edited.table_grids[0].cells[1].utf16_start,
        baseline.table_grids[0].cells[1].utf16_start
    );

    session.undo().unwrap();
    let undone = session.project();
    assert_eq!(undone.table_grids, baseline.table_grids);

    session.redo().unwrap();
    assert_eq!(session.project().table_grids, edited.table_grids);

    let mut replay_edited = EditorSession::new(graph()).unwrap();
    replay_edited.apply_project(&edited).unwrap();
    assert_eq!(replay_edited.project(), edited);
}

#[test]
fn legacy_v0_5_project_replays_on_table_source_without_v0_6_grid_payload() {
    let baseline_session = EditorSession::new(graph()).unwrap();
    let mut legacy = baseline_session.project();
    legacy.schema_version = "pub-editor-v0.5".into();
    legacy.table_grids.clear();

    let mut replay = EditorSession::new(graph()).unwrap();
    replay.apply_project(&legacy).unwrap();

    let current = replay.project();
    assert_eq!(current.schema_version, EDITOR_PROJECT_VERSION_V0_6);
    assert_eq!(current.table_grids.len(), 1);
}

#[test]
fn v0_7_inherits_table_grid_replay_integrity() {
    let baseline_session = EditorSession::new(graph()).unwrap();
    let mut project = baseline_session.project();
    project.schema_version = EDITOR_PROJECT_VERSION_V0_7.into();
    project.table_grids[0].rows[0].extent = Some(LengthEmu::new(501));

    let mut replay = EditorSession::new(graph()).unwrap();
    let error = replay
        .apply_project(&project)
        .expect_err("v0.7 must not bypass v0.6 table-grid integrity");

    assert!(matches!(error, EditorProjectError::TableGridMismatch));
}
