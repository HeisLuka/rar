use anyhow::{Context, Result};
use pub_editor::{open_mature_0x2c_editor, EditorEditableTarget};
use pub_model::{LengthEmu, NodeId, Sha256Digest, StoryId, TableCellId};
use pub_reader::materialize_bounded_simple_table_cells;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs;

#[derive(Debug, Serialize)]
struct Capability {
    ok: bool,
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct StoryRow {
    story_id: String,
    non_empty: bool,
    utf16_len: usize,
    paragraph_count: usize,
    run_count: usize,
    field_count: usize,
    hyperlink_count: usize,
    frame_count: usize,
    frame_node_ids: Vec<String>,
    frame_page_indices: Vec<u32>,
    table_owned: bool,
    replace_story_text: Capability,
}

#[derive(Debug, Serialize)]
struct NodeRow {
    node_id: String,
    kind: String,
    parent_id: String,
    page_index: Option<u32>,
    x: i64,
    y: i64,
    width: i64,
    height: i64,
    identity_transform: bool,
    story_id: Option<String>,
    image_slot: Option<u32>,
    explicit_crop: bool,
    table: bool,
    table_story_owner: bool,
    move_node: Capability,
    replace_image: Capability,
}

#[derive(Debug, Serialize)]
struct TableRow {
    node_id: String,
    page_index: Option<u32>,
    story_id: Option<String>,
    rows: u32,
    columns: u32,
    source_cell_records: usize,
    merged_or_spanning_cell_records: usize,
    simple_table_present: bool,
    simple_cell_count: usize,
    layout_metrics_present: bool,
    materialization_ok: bool,
    materialization_error: Option<String>,
    editable_cell_count: usize,
    editable_cell_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct Smoke {
    story_id: Option<String>,
    node_id: Option<String>,
    operation_count: usize,
    undo_redo_ok: bool,
    project_replay_ok: bool,
    idml_export_ok: bool,
    idml_bytes: Option<usize>,
    idml_error: Option<String>,
    odg_export_ok: bool,
    odg_bytes: Option<usize>,
    odg_error: Option<String>,
    table_cell_id: Option<String>,
    table_edit_replay_ok: Option<bool>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct Report {
    schema: &'static str,
    source_sha256: String,
    source_bytes: usize,
    viewer_page_count: usize,
    viewer_story_count: usize,
    viewer_diagnostic_codes: BTreeMap<String, usize>,
    editor_document_page_count: usize,
    editor_page_registry_count: usize,
    node_count: usize,
    story_count: usize,
    non_empty_story_count: usize,
    framed_story_count: usize,
    editable_story_count: usize,
    editable_non_empty_framed_story_count: usize,
    story_rejection_reasons: BTreeMap<String, usize>,
    page_node_counts: BTreeMap<u32, usize>,
    movable_node_count: usize,
    image_slot_node_count: usize,
    replaceable_image_node_count: usize,
    table_node_count: usize,
    simple_table_node_count: usize,
    editable_table_cell_count: usize,
    stories: Vec<StoryRow>,
    nodes: Vec<NodeRow>,
    tables: Vec<TableRow>,
    smoke: Smoke,
}

fn capability(result: Result<(), pub_editor::EditorError>) -> Capability {
    match result {
        Ok(()) => Capability { ok: true, reason: None },
        Err(error) => Capability {
            ok: false,
            reason: Some(error.code().to_owned()),
        },
    }
}

fn source_hash(bytes: &[u8]) -> Sha256Digest {
    let digest = Sha256::digest(bytes);
    let mut raw = [0_u8; 32];
    raw.copy_from_slice(&digest);
    Sha256Digest::from_bytes(raw)
}

fn main() -> Result<()> {
    let path = env::args().nth(1).context("usage: carlton-census FILE.pub")?;
    let bytes = fs::read(&path).with_context(|| format!("read {path}"))?;
    let hash = source_hash(&bytes);
    let mut session = open_mature_0x2c_editor(&bytes, hash)
        .context("open exact PUB in current Editor")?;

    // Current ImageReplace predicate requires a registered PNG/JPEG asset.
    // The capability gate validates only non-empty bytes + MIME signature.
    let dummy_png = b"\x89PNG\r\n\x1a\n".to_vec();
    let dummy_asset = session
        .import_replacement_asset("image/png", dummy_png)
        .context("register census-only PNG signature asset")?;

    let viewer = pub_viewer::open_mature_0x2c(&bytes)
        .context("open exact PUB in current Viewer")?;
    let mut viewer_diagnostic_codes = BTreeMap::new();
    for diagnostic in &viewer.diagnostics {
        *viewer_diagnostic_codes.entry(diagnostic.code.clone()).or_insert(0usize) += 1;
    }

    let graph = session.graph();
    let mut page_index_by_id = BTreeMap::new();
    for (zero, page_id) in graph.document.pages.iter().enumerate() {
        page_index_by_id.insert(page_id.as_canonical().to_string(), (zero + 1) as u32);
    }

    let mut stories = Vec::new();
    let mut story_rejection_reasons = BTreeMap::new();
    let mut editable_story_count = 0usize;
    let mut editable_non_empty_framed_story_count = 0usize;
    let mut framed_story_count = 0usize;
    let mut non_empty_story_count = 0usize;
    let mut first_safe_story: Option<StoryId> = None;

    for (story_id, story) in &graph.stories {
        let mut frame_node_ids = Vec::new();
        let mut frame_page_indices = Vec::new();
        for (node_id, node) in &graph.nodes {
            let Some(frame) = node.payload.story_frame.as_ref() else { continue };
            if frame.story_id != Some(*story_id) { continue; }
            frame_node_ids.push(node_id.as_canonical().to_string());
            if let Some(index) = page_index_by_id.get(&node.header.parent_id.to_string()) {
                frame_page_indices.push(*index);
            }
        }
        frame_node_ids.sort();
        frame_page_indices.sort();
        frame_page_indices.dedup();

        let non_empty = !story.text.is_empty();
        if non_empty { non_empty_story_count += 1; }
        if !frame_node_ids.is_empty() { framed_story_count += 1; }

        let table_owned = graph.nodes.values().any(|node| {
            node.payload.table_story.as_ref().is_some_and(|owner| owner.story_id == Some(*story_id))
                || node.payload.table.as_ref().is_some_and(|table| table.story_id == Some(*story_id))
        });

        let cap = capability(session.can_replace_story_text(*story_id));
        if cap.ok {
            editable_story_count += 1;
            if non_empty && !frame_node_ids.is_empty() {
                editable_non_empty_framed_story_count += 1;
                if first_safe_story.is_none() {
                    first_safe_story = Some(*story_id);
                }
            }
        } else if let Some(reason) = cap.reason.as_ref() {
            *story_rejection_reasons.entry(reason.clone()).or_insert(0usize) += 1;
        }

        stories.push(StoryRow {
            story_id: story_id.as_canonical().to_string(),
            non_empty,
            utf16_len: story.text.encode_utf16().count(),
            paragraph_count: story.paragraphs.len(),
            run_count: story.runs.len(),
            field_count: story.fields.len(),
            hyperlink_count: story.hyperlinks.len(),
            frame_count: frame_node_ids.len(),
            frame_node_ids,
            frame_page_indices,
            table_owned,
            replace_story_text: cap,
        });
    }
    stories.sort_by(|a, b| a.story_id.cmp(&b.story_id));

    let mut nodes = Vec::new();
    let mut page_node_counts = BTreeMap::new();
    let mut movable_node_count = 0usize;
    let mut image_slot_node_count = 0usize;
    let mut replaceable_image_node_count = 0usize;
    let mut first_movable_node: Option<NodeId> = None;

    for (node_id, node) in &graph.nodes {
        let page_index = page_index_by_id.get(&node.header.parent_id.to_string()).copied();
        if let Some(index) = page_index {
            *page_node_counts.entry(index).or_insert(0usize) += 1;
        }

        let move_cap = capability(session.can_move_node_to(
            *node_id,
            node.header.bounds.x,
            node.header.bounds.y,
        ));
        if move_cap.ok {
            movable_node_count += 1;
            if first_movable_node.is_none() {
                first_movable_node = Some(*node_id);
            }
        }

        let image_cap = capability(session.can_replace_image(*node_id, dummy_asset));
        if node.payload.image_slot.is_some() {
            image_slot_node_count += 1;
            if image_cap.ok { replaceable_image_node_count += 1; }
        }

        nodes.push(NodeRow {
            node_id: node_id.as_canonical().to_string(),
            kind: format!("{:?}", node.kind),
            parent_id: node.header.parent_id.to_string(),
            page_index,
            x: node.header.bounds.x.get(),
            y: node.header.bounds.y.get(),
            width: node.header.bounds.width.get(),
            height: node.header.bounds.height.get(),
            identity_transform: node.header.transform == pub_model::Affine2D::identity(),
            story_id: node.payload.story_frame.as_ref()
                .and_then(|frame| frame.story_id)
                .map(|id| id.as_canonical().to_string()),
            image_slot: node.payload.image_slot,
            explicit_crop: node.payload.explicit_image_crop.is_some(),
            table: node.payload.table.is_some(),
            table_story_owner: node.payload.table_story.is_some(),
            move_node: move_cap,
            replace_image: image_cap,
        });
    }
    nodes.sort_by(|a, b| a.node_id.cmp(&b.node_id));

    let mut tables = Vec::new();
    let mut simple_table_node_count = 0usize;
    let mut editable_table_cell_count = 0usize;
    let mut first_table_cell: Option<(NodeId, StoryId, TableCellId, String)> = None;

    for (node_id, node) in &graph.nodes {
        let Some(table) = node.payload.table.as_ref() else { continue };
        let page_index = page_index_by_id.get(&node.header.parent_id.to_string()).copied();
        let merged_or_spanning = table.cells.iter().filter(|cell| {
            cell.coordinates.is_some_and(|c| {
                c.start_row != c.end_row || c.start_column != c.end_column
            })
        }).count();

        let mut materialization_ok = false;
        let mut materialization_error = None;
        let mut editable_cell_ids = Vec::new();

        if table.simple_table.is_some() {
            simple_table_node_count += 1;
        }

        if let Some(story_id) = table.story_id {
            if let Some(story) = graph.stories.get(&story_id) {
                match materialize_bounded_simple_table_cells(table, story) {
                    Ok(cells) => {
                        materialization_ok = true;
                        for cell in cells {
                            if session.can_replace_table_cell_text(*node_id, cell.id).is_ok() {
                                editable_table_cell_count += 1;
                                editable_cell_ids.push(cell.id.as_canonical().to_string());
                                if first_table_cell.is_none() {
                                    first_table_cell = Some((*node_id, story_id, cell.id, cell.text));
                                }
                            }
                        }
                    }
                    Err(error) => {
                        materialization_error = Some(format!("{error:?}"));
                    }
                }
            } else {
                materialization_error = Some("story_missing".to_owned());
            }
        } else {
            materialization_error = Some("story_identity_missing".to_owned());
        }

        editable_cell_ids.sort();
        tables.push(TableRow {
            node_id: node_id.as_canonical().to_string(),
            page_index,
            story_id: table.story_id.map(|id| id.as_canonical().to_string()),
            rows: table.rows,
            columns: table.columns,
            source_cell_records: table.cells.len(),
            merged_or_spanning_cell_records: merged_or_spanning,
            simple_table_present: table.simple_table.is_some(),
            simple_cell_count: table.simple_table.as_ref().map_or(0, |simple| simple.cells.len()),
            layout_metrics_present: table.layout_metrics.is_some(),
            materialization_ok,
            materialization_error,
            editable_cell_count: editable_cell_ids.len(),
            editable_cell_ids,
        });
    }
    tables.sort_by(|a, b| a.node_id.cmp(&b.node_id));

    let mut smoke = Smoke {
        story_id: first_safe_story.map(|id| id.as_canonical().to_string()),
        node_id: first_movable_node.map(|id| id.as_canonical().to_string()),
        operation_count: 0,
        undo_redo_ok: false,
        project_replay_ok: false,
        idml_export_ok: false,
        idml_bytes: None,
        idml_error: None,
        odg_export_ok: false,
        odg_bytes: None,
        odg_error: None,
        table_cell_id: first_table_cell.as_ref().map(|(_, _, id, _)| id.as_canonical().to_string()),
        table_edit_replay_ok: None,
        error: None,
    };

    if let (Some(story_id), Some(node_id)) = (first_safe_story, first_movable_node) {
        let result = (|| -> Result<()> {
            let mut probe = open_mature_0x2c_editor(&bytes, hash)?;
            let replacement = format!("CHAPTERA CARLTON SMOKE {}", story_id.as_canonical());
            probe.replace_story_text(story_id, replacement.clone())?;

            let before = probe.graph().nodes.get(&node_id)
                .context("smoke move node missing")?.header.bounds;
            let x = before.x.get().checked_add(9_144).context("smoke x overflow")?;
            let new_x = LengthEmu::new(x);
            probe.move_node_to(node_id, new_x, before.y)?;

            let project = probe.project();
            smoke.operation_count = project.operations.len();

            let undo_a = probe.undo().is_ok();
            let undo_b = probe.undo().is_ok();
            let redo_a = probe.redo().is_ok();
            let redo_b = probe.redo().is_ok();
            smoke.undo_redo_ok = undo_a && undo_b && redo_a && redo_b;

            let mut replay = open_mature_0x2c_editor(&bytes, hash)?;
            replay.apply_project(&project)?;
            let story_ok = replay.graph().stories.get(&story_id)
                .is_some_and(|story| story.text == replacement);
            let move_ok = replay.graph().nodes.get(&node_id)
                .is_some_and(|node| node.header.bounds.x == new_x);
            smoke.project_replay_ok = story_ok && move_ok;

            match replay.export_editable(EditorEditableTarget::Idml, "carlton-march-census") {
                Ok(export) => {
                    smoke.idml_export_ok = true;
                    smoke.idml_bytes = Some(export.bytes.len());
                }
                Err(error) => smoke.idml_error = Some(error.to_string()),
            }
            match replay.export_editable(EditorEditableTarget::Odg, "carlton-march-census") {
                Ok(export) => {
                    smoke.odg_export_ok = true;
                    smoke.odg_bytes = Some(export.bytes.len());
                }
                Err(error) => smoke.odg_error = Some(error.to_string()),
            }
            Ok(())
        })();
        if let Err(error) = result {
            smoke.error = Some(error.to_string());
        }
    } else {
        smoke.error = Some("no safe ordinary Story and/or movable Node candidate".to_owned());
    }

    if let Some((node_id, story_id, cell_id, current_text)) = first_table_cell {
        let result = (|| -> Result<bool> {
            let mut probe = open_mature_0x2c_editor(&bytes, hash)?;
            let replacement = if current_text == "X" { "Y".to_owned() } else { "X".to_owned() };
            probe.replace_table_cell_text(node_id, cell_id, replacement.clone())?;
            let project = probe.project();
            let undo_ok = probe.undo().is_ok();
            let redo_ok = probe.redo().is_ok();

            let mut replay = open_mature_0x2c_editor(&bytes, hash)?;
            replay.apply_project(&project)?;
            let found = replay.editable_table_cells_for_story(story_id)
                .into_iter()
                .find(|cell| cell.node_id == node_id && cell.cell_id == cell_id)
                .is_some_and(|cell| cell.text == replacement);
            Ok(undo_ok && redo_ok && found)
        })();
        smoke.table_edit_replay_ok = Some(result.unwrap_or(false));
    }

    let report = Report {
        schema: "chaptera.carlton-march-editor-census.v1",
        source_sha256: hash.to_string(),
        source_bytes: bytes.len(),
        viewer_page_count: viewer.pages.len(),
        viewer_story_count: viewer.stories.len(),
        viewer_diagnostic_codes,
        editor_document_page_count: graph.document.pages.len(),
        editor_page_registry_count: graph.pages.len(),
        node_count: graph.nodes.len(),
        story_count: graph.stories.len(),
        non_empty_story_count,
        framed_story_count,
        editable_story_count,
        editable_non_empty_framed_story_count,
        story_rejection_reasons,
        page_node_counts,
        movable_node_count,
        image_slot_node_count,
        replaceable_image_node_count,
        table_node_count: tables.len(),
        simple_table_node_count,
        editable_table_cell_count,
        stories,
        nodes,
        tables,
        smoke,
    };

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
