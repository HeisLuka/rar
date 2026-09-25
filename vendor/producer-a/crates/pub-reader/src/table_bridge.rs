use super::*;
use pub_contents::{CONTENTS_RAW_TYPE_CELLS, MatureCellCoordinates, parse_confirmed_mature_cells};
use pub_model::{SimpleRectangularTable, SimpleTableCell, Story, TableCellAddress, TableCellId};
use pub_quill::{QuillMcldChunk, QuillStoryCatalog, bounded_mcld_table_metrics};

pub const RAW_TYPE_TABLE: u16 = 0x10;
pub const TABLE_NUM_ROWS_ID: u16 = 0x66;
pub const TABLE_NUM_COLUMNS_ID: u16 = 0x67;
pub const TABLE_CELLS_SEQ_NUM_ID: u16 = 0x6B;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubTableCellSource {
    pub id: TableCellId,
    pub stored_record_index: u32,
    pub coordinates: Option<PubTableCellCoordinates>,
    pub utf16_start: u32,
    pub utf16_end: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<SourceRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubTableCellCoordinates {
    pub start_row: u32,
    pub end_row: u32,
    pub start_column: u32,
    pub end_column: u32,
}

impl From<MatureCellCoordinates> for PubTableCellCoordinates {
    fn from(value: MatureCellCoordinates) -> Self {
        Self {
            start_row: value.start_row,
            end_row: value.end_row,
            start_column: value.start_column,
            end_column: value.end_column,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PubMaterializedTableCell {
    pub id: TableCellId,
    pub address: TableCellAddress,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PubTableTextError {
    NotSimpleRectangular,
    StoryMismatch {
        expected: Option<StoryId>,
        found: StoryId,
    },
    MissingSourceCell {
        id: TableCellId,
    },
    InvalidRange {
        id: TableCellId,
        start: u32,
        end: u32,
        story_len_utf16: usize,
    },
    MissingLeadingCellSeparator {
        id: TableCellId,
        start: u32,
    },
    InvalidUtf16 {
        id: TableCellId,
    },
}

pub fn materialize_bounded_simple_table_cells(
    table: &PubTableSource,
    story: &Story,
) -> Result<Vec<PubMaterializedTableCell>, PubTableTextError> {
    let simple = table
        .simple_table
        .as_ref()
        .ok_or(PubTableTextError::NotSimpleRectangular)?;

    if table.story_id != Some(story.id) {
        return Err(PubTableTextError::StoryMismatch {
            expected: table.story_id,
            found: story.id,
        });
    }

    let story_utf16 = story.text.encode_utf16().collect::<Vec<_>>();
    let mut semantic_cells = simple.cells.clone();
    semantic_cells.sort_by_key(|cell| (cell.address.row, cell.address.column, cell.id));

    semantic_cells
        .into_iter()
        .map(|semantic| {
            let source = table
                .cells
                .iter()
                .find(|cell| cell.id == semantic.id)
                .ok_or(PubTableTextError::MissingSourceCell { id: semantic.id })?;

            let start = usize::try_from(source.utf16_start).map_err(|_| {
                PubTableTextError::InvalidRange {
                    id: semantic.id,
                    start: source.utf16_start,
                    end: source.utf16_end,
                    story_len_utf16: story_utf16.len(),
                }
            })?;
            let end =
                usize::try_from(source.utf16_end).map_err(|_| PubTableTextError::InvalidRange {
                    id: semantic.id,
                    start: source.utf16_start,
                    end: source.utf16_end,
                    story_len_utf16: story_utf16.len(),
                })?;
            if start > end || end > story_utf16.len() {
                return Err(PubTableTextError::InvalidRange {
                    id: semantic.id,
                    start: source.utf16_start,
                    end: source.utf16_end,
                    story_len_utf16: story_utf16.len(),
                });
            }

            let mut cell_start = start;
            let mut cell_end = end;

            // The grounded Publisher TCD convention used by our mature bridge
            // leaves the inter-cell CR at the start of every cell after the
            // first source slice. The table structure itself represents that
            // boundary, so it must not become cell content.
            if start > 0 {
                if story_utf16.get(start) != Some(&0x000D) {
                    return Err(PubTableTextError::MissingLeadingCellSeparator {
                        id: semantic.id,
                        start: source.utf16_start,
                    });
                }
                cell_start += 1;
            }

            // The final Quill Story paragraph terminator is not table-cell
            // content. Internal CRs are preserved for multi-paragraph cells.
            if end == story_utf16.len()
                && cell_end > cell_start
                && story_utf16.get(cell_end - 1) == Some(&0x000D)
            {
                cell_end -= 1;
            }

            let text = String::from_utf16(&story_utf16[cell_start..cell_end])
                .map_err(|_| PubTableTextError::InvalidUtf16 { id: semantic.id })?;

            Ok(PubMaterializedTableCell {
                id: semantic.id,
                address: semantic.address,
                text,
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubTableLayoutMetricsSource {
    pub story_layout_key: u32,
    pub cell_width: LengthEmu,
    pub row_pitch: LengthEmu,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<SourceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubTableStoryOwnershipSource {
    pub text_id: u32,
    pub story_id: Option<StoryId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<SourceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubTableSource {
    pub text_id: u32,
    pub story_id: Option<StoryId>,
    pub rows: u32,
    pub columns: u32,
    pub cells_seq_num: u32,
    pub tcd_story_ordinal: u16,
    pub cells: Vec<PubTableCellSource>,
    /// Present only for a complete, unmerged, unambiguous rectangular grid.
    pub simple_table: Option<SimpleRectangularTable<TableCellId>>,
    pub layout_metrics: Option<PubTableLayoutMetricsSource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<SourceRef>,
}

pub(crate) struct TableBridgeContext<'a> {
    pub source: &'a SourceDescriptor,
    pub contents_stream: &'a StreamPath,
    pub contents: &'a [u8],
    pub references: &'a BTreeMap<u32, Contents0x2cChunkReference>,
    pub quill_catalog: &'a QuillStoryCatalog,
    pub story_by_syid: &'a BTreeMap<u32, StoryId>,
    pub story_layout_keys: &'a BTreeMap<u32, (u32, RawSpan)>,
    pub mcld: Option<&'a QuillMcldChunk>,
}

pub(crate) fn build_table_story_ownership_source(
    context: &TableBridgeContext<'_>,
    table_seq_num: u32,
    table_chunk: &Contents0x2cChunk,
) -> Result<Option<PubTableStoryOwnershipSource>> {
    let tail_scalars = scan_table_tail_scalars(context, table_chunk)?;
    let Some((text_id, text_id_source)) =
        unique_table_scalar(table_chunk, &tail_scalars, FIELD_STORY_ID)?
    else {
        return Ok(None);
    };

    Ok(Some(PubTableStoryOwnershipSource {
        text_id,
        story_id: context.story_by_syid.get(&text_id).copied(),
        source_refs: vec![source_ref(
            context.source,
            &text_id_source,
            Some(contents_object_key(table_seq_num)),
            Some("TABLE/textId".into()),
            SourceRole::Relation,
            AuthorityClass::Authoritative,
            ReadConfidence::Exact,
        )],
    }))
}

pub(crate) fn build_table_source(
    context: &TableBridgeContext<'_>,
    table_seq_num: u32,
    table_chunk: &Contents0x2cChunk,
    diagnostics: &mut Vec<PubBridgeDiagnostic>,
) -> Result<Option<PubTableSource>> {
    let tail_scalars = scan_table_tail_scalars(context, table_chunk)?;
    let Some((text_id, text_id_source)) =
        unique_table_scalar(table_chunk, &tail_scalars, FIELD_STORY_ID)?
    else {
        diagnostics.push(PubBridgeDiagnostic::TableMissingRequiredField {
            seq_num: table_seq_num,
            field_id: FIELD_STORY_ID,
        });
        return Ok(None);
    };
    let Some((rows, rows_source)) =
        unique_table_scalar(table_chunk, &tail_scalars, TABLE_NUM_ROWS_ID)?
    else {
        diagnostics.push(PubBridgeDiagnostic::TableMissingRequiredField {
            seq_num: table_seq_num,
            field_id: TABLE_NUM_ROWS_ID,
        });
        return Ok(None);
    };
    let Some((columns, columns_source)) =
        unique_table_scalar(table_chunk, &tail_scalars, TABLE_NUM_COLUMNS_ID)?
    else {
        diagnostics.push(PubBridgeDiagnostic::TableMissingRequiredField {
            seq_num: table_seq_num,
            field_id: TABLE_NUM_COLUMNS_ID,
        });
        return Ok(None);
    };
    let Some((cells_seq_num, cells_seq_source)) =
        unique_table_scalar(table_chunk, &tail_scalars, TABLE_CELLS_SEQ_NUM_ID)?
    else {
        diagnostics.push(PubBridgeDiagnostic::TableMissingRequiredField {
            seq_num: table_seq_num,
            field_id: TABLE_CELLS_SEQ_NUM_ID,
        });
        return Ok(None);
    };

    let story_id = context.story_by_syid.get(&text_id).copied();
    if story_id.is_none() {
        diagnostics.push(PubBridgeDiagnostic::MissingQuillStory {
            seq_num: table_seq_num,
            text_id,
        });
    }

    let mut tcd_matches = context
        .quill_catalog
        .tcd
        .iter()
        .filter(|tcd| tcd.story_syid.value.0 == text_id);
    let Some(tcd) = tcd_matches.next() else {
        diagnostics.push(PubBridgeDiagnostic::TableMissingTcd {
            seq_num: table_seq_num,
            text_id,
        });
        return Ok(None);
    };
    if tcd_matches.next().is_some() {
        diagnostics.push(PubBridgeDiagnostic::TableAmbiguousTcd {
            seq_num: table_seq_num,
            text_id,
        });
        return Ok(None);
    }

    let Some(cells_reference) = context.references.get(&cells_seq_num) else {
        diagnostics.push(PubBridgeDiagnostic::TableMissingCellsObject {
            seq_num: table_seq_num,
            cells_seq_num,
        });
        return Ok(None);
    };
    if single_raw_type(cells_reference) != Some(CONTENTS_RAW_TYPE_CELLS) {
        diagnostics.push(PubBridgeDiagnostic::TableCellsWrongRawType {
            seq_num: table_seq_num,
            cells_seq_num,
            raw_type: single_raw_type(cells_reference),
        });
        return Ok(None);
    }
    if single_parent_seq(cells_reference) != Some(table_seq_num) {
        diagnostics.push(PubBridgeDiagnostic::TableCellsWrongParent {
            seq_num: table_seq_num,
            cells_seq_num,
            parent_seq_num: single_parent_seq(cells_reference),
        });
    }

    let cells_chunk = chunk_for_reference(
        context.contents_stream.clone(),
        context.contents,
        cells_reference,
    )
    .context("parse TABLE-owned CELLS chunk")?;
    let cells = parse_confirmed_mature_cells(context.contents, &cells_chunk)
        .context("parse TABLE-owned mature CELLS")?;

    if cells.records.len() != tcd.cell_end_offsets_utf16.len() {
        diagnostics.push(PubBridgeDiagnostic::TableCellCountMismatch {
            seq_num: table_seq_num,
            cells_records: cells.records.len(),
            tcd_boundaries: tcd.cell_end_offsets_utf16.len(),
        });
        return Ok(None);
    }

    let mut joined_cells = Vec::with_capacity(cells.records.len());
    let mut previous_end = 0_u32;
    let mut monotonic = true;

    for (record, end) in cells.records.iter().zip(&tcd.cell_end_offsets_utf16) {
        if end.value < previous_end || end.value > tcd.story_utf16_code_units.value {
            monotonic = false;
            diagnostics.push(PubBridgeDiagnostic::TableCellTextRangeInvalid {
                seq_num: table_seq_num,
                stored_record_index: record.record_index,
                previous_end,
                end: end.value,
                story_len: tcd.story_utf16_code_units.value,
            });
        }

        let coordinates = record.effective_coordinates().map(Into::into);
        if coordinates.is_none() {
            diagnostics.push(PubBridgeDiagnostic::TableCellCoordinatesAmbiguous {
                seq_num: table_seq_num,
                stored_record_index: record.record_index,
            });
        }

        let id = TableCellId::from_canonical(derive_pub_id(
            &context.source.source_hash,
            &format!(
                "contents/0x2c/seq/{table_seq_num}/cells/stored/{}",
                record.record_index
            ),
            "cdm.table_cell",
        )?);

        joined_cells.push(PubTableCellSource {
            id,
            stored_record_index: record.record_index,
            coordinates,
            utf16_start: previous_end,
            utf16_end: end.value,
            source_refs: vec![
                source_ref(
                    context.source,
                    &record.source,
                    Some(contents_object_key(cells_seq_num)),
                    Some(format!("CELLS/record/{}", record.record_index)),
                    SourceRole::Relation,
                    AuthorityClass::Authoritative,
                    ReadConfidence::Exact,
                ),
                source_ref(
                    context.source,
                    &end.source,
                    Some(quill_story_object_key(text_id)),
                    Some(format!("TCD/cell_end/{}", record.record_index)),
                    SourceRole::Relation,
                    AuthorityClass::Authoritative,
                    ReadConfidence::Exact,
                ),
            ],
        });
        previous_end = end.value;
    }

    if monotonic && previous_end != tcd.story_utf16_code_units.value {
        diagnostics.push(PubBridgeDiagnostic::TableStoryLengthMismatch {
            seq_num: table_seq_num,
            tcd_last_end: previous_end,
            story_len: tcd.story_utf16_code_units.value,
        });
    }

    let simple_table = build_simple_table(rows, columns, &joined_cells);
    let layout_metrics = build_table_layout_metrics(context, table_seq_num, text_id, diagnostics);

    Ok(Some(PubTableSource {
        text_id,
        story_id,
        rows,
        columns,
        cells_seq_num,
        tcd_story_ordinal: tcd.story_ordinal.value,
        cells: joined_cells,
        simple_table,
        layout_metrics,
        source_refs: vec![
            source_ref(
                context.source,
                &text_id_source,
                Some(contents_object_key(table_seq_num)),
                Some("TABLE/textId".into()),
                SourceRole::Relation,
                AuthorityClass::Authoritative,
                ReadConfidence::Exact,
            ),
            source_ref(
                context.source,
                &rows_source,
                Some(contents_object_key(table_seq_num)),
                Some("TABLE/rows".into()),
                SourceRole::Semantic,
                AuthorityClass::Authoritative,
                ReadConfidence::Exact,
            ),
            source_ref(
                context.source,
                &columns_source,
                Some(contents_object_key(table_seq_num)),
                Some("TABLE/columns".into()),
                SourceRole::Semantic,
                AuthorityClass::Authoritative,
                ReadConfidence::Exact,
            ),
            source_ref(
                context.source,
                &cells_seq_source,
                Some(contents_object_key(table_seq_num)),
                Some("TABLE/cellsSeqNum".into()),
                SourceRole::Relation,
                AuthorityClass::Authoritative,
                ReadConfidence::Exact,
            ),
            source_ref(
                context.source,
                &tcd.source,
                Some(quill_story_object_key(text_id)),
                Some("TCD".into()),
                SourceRole::Relation,
                AuthorityClass::Authoritative,
                ReadConfidence::Exact,
            ),
            source_ref(
                context.source,
                &cells.source,
                Some(contents_object_key(cells_seq_num)),
                Some("CELLS".into()),
                SourceRole::Relation,
                AuthorityClass::Authoritative,
                ReadConfidence::Exact,
            ),
        ],
    }))
}

fn build_table_layout_metrics(
    context: &TableBridgeContext<'_>,
    table_seq_num: u32,
    text_id: u32,
    diagnostics: &mut Vec<PubBridgeDiagnostic>,
) -> Option<PubTableLayoutMetricsSource> {
    let Some((layout_key, layout_key_source)) = context.story_layout_keys.get(&text_id) else {
        diagnostics.push(PubBridgeDiagnostic::TableLayoutMetricsUnavailable {
            seq_num: table_seq_num,
            text_id,
            layout_key: None,
            reason: "story catalog has no unique layout key".into(),
        });
        return None;
    };

    let Some(mcld) = context.mcld else {
        diagnostics.push(PubBridgeDiagnostic::TableLayoutMetricsUnavailable {
            seq_num: table_seq_num,
            text_id,
            layout_key: Some(*layout_key),
            reason: "usable bounded Quill MCLD is unavailable".into(),
        });
        return None;
    };

    let metrics = match bounded_mcld_table_metrics(mcld, *layout_key) {
        Ok(metrics) => metrics,
        Err(error) => {
            diagnostics.push(PubBridgeDiagnostic::TableLayoutMetricsUnavailable {
                seq_num: table_seq_num,
                text_id,
                layout_key: Some(*layout_key),
                reason: error.to_string(),
            });
            return None;
        }
    };

    let mut source_refs = vec![source_ref(
        context.source,
        layout_key_source,
        Some(format!("contents/0x65/story/{text_id}")),
        Some("story/layout_key".into()),
        SourceRole::Relation,
        AuthorityClass::Authoritative,
        ReadConfidence::Exact,
    )];

    source_refs.extend(metrics.cell_width_emu.sources.iter().map(|source| {
        source_ref(
            context.source,
            source,
            Some(quill_story_object_key(text_id)),
            Some("MCLD/table/child/field04".into()),
            SourceRole::Projection,
            AuthorityClass::Authoritative,
            ReadConfidence::Exact,
        )
    }));
    source_refs.extend(metrics.row_pitch_emu.sources.iter().map(|source| {
        source_ref(
            context.source,
            source,
            Some(quill_story_object_key(text_id)),
            Some("MCLD/table/child/field05".into()),
            SourceRole::Projection,
            AuthorityClass::Authoritative,
            ReadConfidence::Exact,
        )
    }));

    Some(PubTableLayoutMetricsSource {
        story_layout_key: *layout_key,
        cell_width: LengthEmu::new(i64::from(metrics.cell_width_emu.value)),
        row_pitch: LengthEmu::new(i64::from(metrics.row_pitch_emu.value)),
        source_refs,
    })
}

fn build_simple_table(
    rows: u32,
    columns: u32,
    cells: &[PubTableCellSource],
) -> Option<SimpleRectangularTable<TableCellId>> {
    let simple_cells = cells
        .iter()
        .map(|cell| {
            let coordinates = cell.coordinates?;
            if coordinates.start_row != coordinates.end_row
                || coordinates.start_column != coordinates.end_column
            {
                return None;
            }
            Some(SimpleTableCell {
                id: cell.id,
                address: TableCellAddress {
                    row: coordinates.start_row,
                    column: coordinates.start_column,
                },
            })
        })
        .collect::<Option<Vec<_>>>()?;

    SimpleRectangularTable::new(rows, columns, simple_cells).ok()
}

type TableTailScalars = BTreeMap<u16, Vec<(u32, RawSpan)>>;

fn unique_table_scalar(
    chunk: &Contents0x2cChunk,
    tail_scalars: &TableTailScalars,
    id: u16,
) -> Result<Option<(u32, RawSpan)>> {
    let mut values = Vec::new();

    for field in chunk.fields.iter().filter(|field| field.id == id) {
        match &field.body {
            RawContentsBlockBody::U16 {
                value,
                value_source,
            } => values.push((u32::from(*value), value_source.clone())),
            RawContentsBlockBody::U32 {
                value,
                value_source,
            } => values.push((*value, value_source.clone())),
            _ => bail!(
                "TABLE field 0x{id:02X} at {} is not a confirmed integer/reference body",
                field.source.offset
            ),
        }
    }

    if let Some(tail_values) = tail_scalars.get(&id) {
        values.extend(tail_values.iter().cloned());
    }

    match values.as_slice() {
        [] => Ok(None),
        [value] => Ok(Some(value.clone())),
        _ => bail!("duplicate TABLE scalar field 0x{id:02X}"),
    }
}

/// Scans only the opaque suffix of a mature TABLE chunk using the physical
/// block-length grammar independently implemented by libmspub's
/// parseBlock(..., true). The scanner keeps only 2/4-byte scalar observations;
/// variable-length blocks are skipped as opaque payloads.
///
/// This does not promote the rest of the tail to semantic state.
fn scan_table_tail_scalars(
    context: &TableBridgeContext<'_>,
    chunk: &Contents0x2cChunk,
) -> Result<TableTailScalars> {
    let Some(tail) = chunk.unsupported_tail.as_ref() else {
        return Ok(BTreeMap::new());
    };

    let start = usize::try_from(tail.offset)
        .map_err(|_| anyhow!("TABLE opaque tail offset does not fit usize"))?;
    let len = usize::try_from(tail.len)
        .map_err(|_| anyhow!("TABLE opaque tail length does not fit usize"))?;
    let end = start
        .checked_add(len)
        .filter(|end| *end <= context.contents.len())
        .ok_or_else(|| anyhow!("TABLE opaque tail is outside Contents stream"))?;

    let bytes = context.contents;
    let mut position = start;
    let mut scalars = BTreeMap::<u16, Vec<(u32, RawSpan)>>::new();

    while position < end {
        if end - position < 2 {
            bail!("truncated TABLE tail block header at {position}");
        }

        let raw_tag = [bytes[position], bytes[position + 1]];
        let (id, wire_type) = pub_contents::decode_packed_field_tag(raw_tag);
        position += 2;

        match wire_type {
            0x00 | 0x08 | 0x78 => {}
            0x10 | 0x18 => {
                if end - position < 2 {
                    bail!("truncated TABLE tail u16 field 0x{id:02X} at {position}");
                }
                let value = u16::from_le_bytes([bytes[position], bytes[position + 1]]);
                scalars.entry(id).or_default().push((
                    u32::from(value),
                    RawSpan {
                        stream: tail.stream.clone(),
                        offset: position as u64,
                        len: 2,
                    },
                ));
                position += 2;
            }
            0x20 | 0x58 | 0x68 | 0x70 | 0xB8 => {
                if end - position < 4 {
                    bail!("truncated TABLE tail u32 field 0x{id:02X} at {position}");
                }
                let value = u32::from_le_bytes([
                    bytes[position],
                    bytes[position + 1],
                    bytes[position + 2],
                    bytes[position + 3],
                ]);
                scalars.entry(id).or_default().push((
                    value,
                    RawSpan {
                        stream: tail.stream.clone(),
                        offset: position as u64,
                        len: 4,
                    },
                ));
                position += 4;
            }
            0x28 => {
                position = checked_skip(position, 8, end, id, wire_type)?;
            }
            0x38 => {
                position = checked_skip(position, 16, end, id, wire_type)?;
            }
            0x48 => {
                position = checked_skip(position, 24, end, id, wire_type)?;
            }
            0x80 | 0x82 | 0x88 | 0x8A | 0x90 | 0x98 | 0xA0 | 0xC0 => {
                if end - position < 4 {
                    bail!("truncated TABLE tail variable field 0x{id:02X} length at {position}");
                }
                let declared_length = u32::from_le_bytes([
                    bytes[position],
                    bytes[position + 1],
                    bytes[position + 2],
                    bytes[position + 3],
                ]);
                if declared_length < 4 {
                    bail!("invalid TABLE tail variable field 0x{id:02X} length {declared_length}");
                }
                position = checked_skip(
                    position,
                    usize::try_from(declared_length)
                        .map_err(|_| anyhow!("TABLE tail declared length does not fit usize"))?,
                    end,
                    id,
                    wire_type,
                )?;
            }
            other => {
                bail!(
                    "unsupported TABLE tail wire type 0x{other:02X} for field 0x{id:02X} at {}",
                    position - 2
                );
            }
        }
    }

    Ok(scalars)
}

fn checked_skip(
    position: usize,
    length: usize,
    end: usize,
    id: u8,
    wire_type: u8,
) -> Result<usize> {
    position
        .checked_add(length)
        .filter(|next| *next <= end)
        .ok_or_else(|| {
            anyhow!("TABLE tail field 0x{id:02X}/type 0x{wire_type:02X} exceeds bounded tail")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_cell_id(byte: u8) -> TableCellId {
        TableCellId::from_canonical(CanonicalId::from_bytes([byte; 16]))
    }

    #[test]
    fn simple_view_preserves_stored_ids_but_uses_visual_coordinates() {
        let cells = vec![
            PubTableCellSource {
                id: table_cell_id(2),
                stored_record_index: 2,
                coordinates: Some(PubTableCellCoordinates {
                    start_row: 1,
                    end_row: 1,
                    start_column: 0,
                    end_column: 0,
                }),
                utf16_start: 0,
                utf16_end: 1,
                source_refs: Vec::new(),
            },
            PubTableCellSource {
                id: table_cell_id(0),
                stored_record_index: 0,
                coordinates: Some(PubTableCellCoordinates {
                    start_row: 0,
                    end_row: 0,
                    start_column: 0,
                    end_column: 0,
                }),
                utf16_start: 1,
                utf16_end: 2,
                source_refs: Vec::new(),
            },
            PubTableCellSource {
                id: table_cell_id(3),
                stored_record_index: 3,
                coordinates: Some(PubTableCellCoordinates {
                    start_row: 1,
                    end_row: 1,
                    start_column: 1,
                    end_column: 1,
                }),
                utf16_start: 2,
                utf16_end: 3,
                source_refs: Vec::new(),
            },
            PubTableCellSource {
                id: table_cell_id(1),
                stored_record_index: 1,
                coordinates: Some(PubTableCellCoordinates {
                    start_row: 0,
                    end_row: 0,
                    start_column: 1,
                    end_column: 1,
                }),
                utf16_start: 3,
                utf16_end: 4,
                source_refs: Vec::new(),
            },
        ];

        let simple = build_simple_table(2, 2, &cells).expect("complete unmerged grid");

        assert_eq!(
            simple.cells.iter().map(|cell| cell.id).collect::<Vec<_>>(),
            vec![
                table_cell_id(2),
                table_cell_id(0),
                table_cell_id(3),
                table_cell_id(1),
            ],
            "promotion must not reorder stored record identity"
        );
        assert!(!simple.cells_are_row_major());
    }

    #[test]
    fn spanning_cell_is_not_flattened_to_simple_subset() {
        let cells = vec![PubTableCellSource {
            id: table_cell_id(0),
            stored_record_index: 0,
            coordinates: Some(PubTableCellCoordinates {
                start_row: 0,
                end_row: 0,
                start_column: 0,
                end_column: 1,
            }),
            utf16_start: 0,
            utf16_end: 1,
            source_refs: Vec::new(),
        }];

        assert!(build_simple_table(1, 2, &cells).is_none());
    }
}
