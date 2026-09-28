use anyhow::{Context, Result};
use pub_model::{NodeId, Sha256Digest, StoryId};
use pub_reader::{build_mature_0x2c_source_graph, materialize_bounded_simple_table_cells};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{env, fs, io::Cursor, path::PathBuf};

const SCHEMA: &str = "chaptera.table-story-corpus-receipt.v1";

#[derive(Debug, Serialize)]
struct CellReceipt {
    stored_record_index: u32,
    utf16_start: u32,
    utf16_end: u32,
    bounded: bool,
    contiguous_with_previous: bool,
    leading_separator_cr: Option<bool>,
}

#[derive(Debug, Serialize)]
struct TableReceipt {
    node_id: NodeId,
    contents_seq_num: u32,
    text_id: u32,
    story_id: Option<StoryId>,
    story_utf16_len: Option<u32>,
    rows: u32,
    columns: u32,
    cell_count: usize,
    simple_rectangular: bool,
    monotonic_and_bounded: bool,
    contiguous_coverage: bool,
    leading_cr_convention: bool,
    final_story_terminator_cr: Option<bool>,
    materialized_simple_cells: Option<usize>,
    materialization_error: Option<String>,
    cells: Vec<CellReceipt>,
}

#[derive(Debug, Serialize)]
struct Receipt {
    schema: &'static str,
    source: String,
    source_sha256: Sha256Digest,
    status: &'static str,
    parse_error: Option<String>,
    table_count: usize,
    simple_table_count: usize,
    non_simple_table_count: usize,
    tables_with_valid_ranges: usize,
    tables_with_contiguous_story_coverage: usize,
    simple_tables_materialized: usize,
    tables: Vec<TableReceipt>,
}

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let source = PathBuf::from(
        args.next()
            .context("usage: table-story-receipt SOURCE.pub [OUTPUT.json]")?,
    );
    let output = args.next().map(PathBuf::from);

    let bytes = fs::read(&source).with_context(|| format!("read {}", source.display()))?;
    let digest = Sha256::digest(&bytes);
    let mut hash = [0_u8; 32];
    hash.copy_from_slice(&digest);
    let source_hash = Sha256Digest::from_bytes(hash);

    let build = match build_mature_0x2c_source_graph(Cursor::new(bytes.as_slice()), source_hash) {
        Ok(build) => build,
        Err(error) => {
            let receipt = Receipt {
                schema: SCHEMA,
                source: source.display().to_string(),
                source_sha256: source_hash,
                status: "unsupported_or_parse_error",
                parse_error: Some(format!("{error:#}")),
                table_count: 0,
                simple_table_count: 0,
                non_simple_table_count: 0,
                tables_with_valid_ranges: 0,
                tables_with_contiguous_story_coverage: 0,
                simple_tables_materialized: 0,
                tables: Vec::new(),
            };
            return write_receipt(receipt, output);
        }
    };

    let mut tables = Vec::new();

    for (node_id, node) in &build.graph.nodes {
        let Some(table) = node.payload.table.as_ref() else {
            continue;
        };

        let story = table
            .story_id
            .and_then(|story_id| build.graph.stories.get(&story_id));
        let story_utf16 = story
            .map(|story| story.text.encode_utf16().collect::<Vec<_>>());
        let story_len = story_utf16
            .as_ref()
            .and_then(|units| u32::try_from(units.len()).ok());

        let mut previous_end = 0_u32;
        let mut monotonic_and_bounded = story_len.is_some();
        let mut contiguous_coverage = story_len.is_some();
        let mut leading_cr_convention = story_utf16.is_some();
        let mut cells = Vec::with_capacity(table.cells.len());

        for (index, cell) in table.cells.iter().enumerate() {
            let bounded = story_len.is_some_and(|len| {
                cell.utf16_start <= cell.utf16_end && cell.utf16_end <= len
            });
            let contiguous = cell.utf16_start == previous_end;
            monotonic_and_bounded &= bounded && cell.utf16_end >= previous_end;
            contiguous_coverage &= contiguous;

            let leading_separator_cr = if index == 0 {
                None
            } else {
                let is_cr = story_utf16
                    .as_ref()
                    .and_then(|units| usize::try_from(cell.utf16_start).ok().and_then(|i| units.get(i)))
                    .is_some_and(|unit| *unit == 0x000D);
                leading_cr_convention &= is_cr;
                Some(is_cr)
            };

            cells.push(CellReceipt {
                stored_record_index: cell.stored_record_index,
                utf16_start: cell.utf16_start,
                utf16_end: cell.utf16_end,
                bounded,
                contiguous_with_previous: contiguous,
                leading_separator_cr,
            });

            previous_end = cell.utf16_end;
        }

        contiguous_coverage &= table.cells.first().is_none_or(|cell| cell.utf16_start == 0);
        contiguous_coverage &= story_len.is_some_and(|len| previous_end == len);

        let final_story_terminator_cr = story_utf16
            .as_ref()
            .and_then(|units| units.last())
            .map(|unit| *unit == 0x000D);

        let (materialized_simple_cells, materialization_error) =
            if table.simple_table.is_some() {
                match story {
                    Some(story) => match materialize_bounded_simple_table_cells(table, story) {
                        Ok(cells) => (Some(cells.len()), None),
                        Err(error) => (None, Some(format!("{error:?}"))),
                    },
                    None => (None, Some("missing owning Story".to_owned())),
                }
            } else {
                (None, None)
            };

        tables.push(TableReceipt {
            node_id: *node_id,
            contents_seq_num: node.payload.contents_seq_num,
            text_id: table.text_id,
            story_id: table.story_id,
            story_utf16_len: story_len,
            rows: table.rows,
            columns: table.columns,
            cell_count: table.cells.len(),
            simple_rectangular: table.simple_table.is_some(),
            monotonic_and_bounded,
            contiguous_coverage,
            leading_cr_convention,
            final_story_terminator_cr,
            materialized_simple_cells,
            materialization_error,
            cells,
        });
    }

    let receipt = Receipt {
        schema: SCHEMA,
        source: source.display().to_string(),
        source_sha256: source_hash,
        status: "analyzed",
        parse_error: None,
        table_count: tables.len(),
        simple_table_count: tables.iter().filter(|table| table.simple_rectangular).count(),
        non_simple_table_count: tables.iter().filter(|table| !table.simple_rectangular).count(),
        tables_with_valid_ranges: tables
            .iter()
            .filter(|table| table.monotonic_and_bounded && table.leading_cr_convention)
            .count(),
        tables_with_contiguous_story_coverage: tables
            .iter()
            .filter(|table| table.contiguous_coverage)
            .count(),
        simple_tables_materialized: tables
            .iter()
            .filter(|table| table.materialized_simple_cells.is_some())
            .count(),
        tables,
    };

    write_receipt(receipt, output)
}

fn write_receipt(receipt: Receipt, output: Option<PathBuf>) -> Result<()> {
    let json = serde_json::to_string_pretty(&receipt)? + "\n";
    match output {
        Some(path) => fs::write(&path, json)
            .with_context(|| format!("write {}", path.display()))?,
        None => print!("{json}"),
    }
    Ok(())
}
