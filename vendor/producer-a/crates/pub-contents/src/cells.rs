use crate::{
    BlockReadError, Contents0x2cChunk, ContentsCursor, ContentsReadError, RawContentsBlock,
    RawContentsBlockBody, parse_confirmed_block,
};
use pub_core::RawSpan;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const CONTENTS_RAW_TYPE_CELLS: u16 = 0x63;
pub const CELLS_DECLARED_COUNT_ID: u16 = 0x01;
pub const CELLS_RECORD_ARRAY_ID: u16 = 0x02;
pub const CELL_START_ROW_ID: u16 = 0x01;
pub const CELL_END_ROW_ID: u16 = 0x02;
pub const CELL_START_COLUMN_ID: u16 = 0x03;
pub const CELL_END_COLUMN_ID: u16 = 0x04;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatureCellsChunk {
    pub source: RawSpan,
    pub fields: Vec<RawContentsBlock>,
    /// Duplicate physical observations are intentionally preserved together
    /// with their actual wire width.
    pub declared_cell_counts: Vec<ObservedCellCount>,
    /// Exact sources of every confirmed record-array container.
    pub record_array_sources: Vec<RawSpan>,
    /// Stored order from Contents. This order must survive until TCD ownership
    /// is joined; it is not a visual row-major ordering.
    pub records: Vec<MatureCellRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedCellScalar {
    pub value: u32,
    pub source: RawSpan,
    pub block_type: u8,
}

pub type ObservedCellCount = ObservedCellScalar;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatureCellCoordinates {
    pub start_row: u32,
    pub end_row: u32,
    pub start_column: u32,
    pub end_column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatureCellRecord {
    pub record_index: u32,
    pub source: RawSpan,
    pub fields: Vec<RawContentsBlock>,
    /// Raw sparse observations only. Absence is not rewritten into a fake block.
    pub start_rows: Vec<ObservedCellScalar>,
    pub end_rows: Vec<ObservedCellScalar>,
    pub start_columns: Vec<ObservedCellScalar>,
    pub end_columns: Vec<ObservedCellScalar>,
}

impl MatureCellRecord {
    /// Mirrors the mature CELLS consumer default: omitted coordinate properties
    /// resolve to zero. Conflicting duplicate physical observations remain
    /// ambiguous and therefore do not produce an effective coordinate tuple.
    pub fn effective_coordinates(&self) -> Option<MatureCellCoordinates> {
        Some(MatureCellCoordinates {
            start_row: unique_scalar_or_zero(&self.start_rows)?,
            end_row: unique_scalar_or_zero(&self.end_rows)?,
            start_column: unique_scalar_or_zero(&self.start_columns)?,
            end_column: unique_scalar_or_zero(&self.end_columns)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellsReadError {
    Contents(ContentsReadError),
    Block(BlockReadError),
    SpanTooLarge { source: RawSpan },
    RecordIndexOverflow,
}

impl fmt::Display for CellsReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contents(error) => error.fmt(f),
            Self::Block(error) => error.fmt(f),
            Self::SpanTooLarge { source } => write!(
                f,
                "CELLS nested span does not fit address space: offset={}, len={}",
                source.offset, source.len
            ),
            Self::RecordIndexOverflow => {
                write!(f, "CELLS stored record index does not fit u32")
            }
        }
    }
}

impl std::error::Error for CellsReadError {}

impl From<ContentsReadError> for CellsReadError {
    fn from(value: ContentsReadError) -> Self {
        Self::Contents(value)
    }
}

impl From<BlockReadError> for CellsReadError {
    fn from(value: BlockReadError) -> Self {
        Self::Block(value)
    }
}

/// Raises only the confirmed mature CELLS topology from an already bounded
/// 0x2C chunk.
///
/// The caller remains responsible for proving that the enclosing directory
/// reference has raw type 0x63. This function deliberately preserves stored
/// record order and leaves all non-coordinate fields raw.
pub fn parse_confirmed_mature_cells(
    bytes: &[u8],
    chunk: &Contents0x2cChunk,
) -> Result<MatureCellsChunk, CellsReadError> {
    let mut declared_cell_counts = Vec::new();
    let mut record_array_sources = Vec::new();
    let mut records = Vec::new();

    for field in &chunk.fields {
        if field.id == CELLS_DECLARED_COUNT_ID {
            match &field.body {
                RawContentsBlockBody::U16 {
                    value,
                    value_source,
                } => declared_cell_counts.push(ObservedCellScalar {
                    value: u32::from(*value),
                    source: value_source.clone(),
                    block_type: field.block_type,
                }),
                RawContentsBlockBody::U32 {
                    value,
                    value_source,
                } => declared_cell_counts.push(ObservedCellScalar {
                    value: *value,
                    source: value_source.clone(),
                    block_type: field.block_type,
                }),
                _ => {}
            }
        }

        if field.id != CELLS_RECORD_ARRAY_ID {
            continue;
        }

        let RawContentsBlockBody::Container { content_source, .. } = &field.body else {
            continue;
        };
        record_array_sources.push(content_source.clone());

        for item in parse_blocks_in_span(bytes, content_source)? {
            if item.id != 0 {
                continue;
            }
            let RawContentsBlockBody::Container {
                content_source: record_source,
                ..
            } = &item.body
            else {
                continue;
            };

            let fields = parse_blocks_in_span(bytes, record_source)?;
            let record_index =
                u32::try_from(records.len()).map_err(|_| CellsReadError::RecordIndexOverflow)?;
            let mut record = MatureCellRecord {
                record_index,
                source: item.source.clone(),
                fields,
                start_rows: Vec::new(),
                end_rows: Vec::new(),
                start_columns: Vec::new(),
                end_columns: Vec::new(),
            };

            for subfield in &record.fields {
                if let Some(value) = promoted_scalar(subfield, CELL_START_ROW_ID) {
                    record.start_rows.push(value);
                }
                if let Some(value) = promoted_scalar(subfield, CELL_END_ROW_ID) {
                    record.end_rows.push(value);
                }
                if let Some(value) = promoted_scalar(subfield, CELL_START_COLUMN_ID) {
                    record.start_columns.push(value);
                }
                if let Some(value) = promoted_scalar(subfield, CELL_END_COLUMN_ID) {
                    record.end_columns.push(value);
                }
            }

            records.push(record);
        }
    }

    Ok(MatureCellsChunk {
        source: chunk.source.clone(),
        fields: chunk.fields.clone(),
        declared_cell_counts,
        record_array_sources,
        records,
    })
}

fn unique_scalar_or_zero(values: &[ObservedCellScalar]) -> Option<u32> {
    let Some(first) = values.first() else {
        return Some(0);
    };
    values
        .iter()
        .all(|value| value.value == first.value)
        .then_some(first.value)
}

fn promoted_scalar(field: &RawContentsBlock, id: u16) -> Option<ObservedCellScalar> {
    if field.id != id {
        return None;
    }

    match &field.body {
        RawContentsBlockBody::Empty => Some(ObservedCellScalar {
            value: 0,
            source: field.source.clone(),
            block_type: field.block_type,
        }),
        RawContentsBlockBody::U16 {
            value,
            value_source,
        } => Some(ObservedCellScalar {
            value: u32::from(*value),
            source: value_source.clone(),
            block_type: field.block_type,
        }),
        RawContentsBlockBody::U32 {
            value,
            value_source,
        } => Some(ObservedCellScalar {
            value: *value,
            source: value_source.clone(),
            block_type: field.block_type,
        }),
        _ => None,
    }
}

fn parse_blocks_in_span(
    bytes: &[u8],
    source: &RawSpan,
) -> Result<Vec<RawContentsBlock>, CellsReadError> {
    let start = usize::try_from(source.offset).map_err(|_| CellsReadError::SpanTooLarge {
        source: source.clone(),
    })?;
    let len = usize::try_from(source.len).map_err(|_| CellsReadError::SpanTooLarge {
        source: source.clone(),
    })?;
    let mut cursor = ContentsCursor::bounded(source.stream.clone(), bytes, start, len)?;
    let mut fields = Vec::new();

    while cursor.remaining() > 0 {
        fields.push(parse_confirmed_block(&mut cursor)?);
    }

    Ok(fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        parse_0x2c_header, parse_confirmed_0x2c_chunk, parse_confirmed_0x2c_trailer_root,
        parse_confirmed_chunk_reference,
    };
    use pub_core::StreamPath;

    fn push_u32_block(out: &mut Vec<u8>, id: u16, value: u32) {
        out.extend_from_slice(
            &crate::encode_packed_field_tag(id, crate::BLOCK_TYPE_U32).expect("test field id"),
        );
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn container_block(id: u16, content: Vec<u8>) -> Vec<u8> {
        let mut out = crate::encode_packed_field_tag(id, crate::BLOCK_TYPE_CONTAINER_90)
            .expect("test field id")
            .to_vec();
        let declared_length = u32::try_from(content.len() + 4).unwrap();
        out.extend_from_slice(&declared_length.to_le_bytes());
        out.extend_from_slice(&content);
        out
    }

    fn cell_item(start_row: u32, end_row: u32, start_column: u32, end_column: u32) -> Vec<u8> {
        let mut content = Vec::new();
        push_u32_block(&mut content, CELL_START_ROW_ID, start_row);
        push_u32_block(&mut content, CELL_END_ROW_ID, end_row);
        push_u32_block(&mut content, CELL_START_COLUMN_ID, start_column);
        push_u32_block(&mut content, CELL_END_COLUMN_ID, end_column);
        push_u32_block(&mut content, 0x09, 1234);
        container_block(0, content)
    }

    fn cells_chunk_bytes() -> Vec<u8> {
        let mut records = Vec::new();
        records.extend_from_slice(&cell_item(0, 0, 0, 0));
        records.extend_from_slice(&cell_item(0, 0, 1, 1));

        let mut fields = Vec::new();
        push_u32_block(&mut fields, CELLS_DECLARED_COUNT_ID, 2);
        fields.extend_from_slice(&container_block(CELLS_RECORD_ARRAY_ID, records));

        let mut out = Vec::new();
        let chunk_length = u32::try_from(fields.len() + 4).unwrap();
        out.extend_from_slice(&chunk_length.to_le_bytes());
        out.extend_from_slice(&fields);
        out
    }

    fn decode_base64(input: &str) -> Vec<u8> {
        let mut output = Vec::with_capacity(input.len() * 3 / 4);
        let mut buffer = 0_u32;
        let mut bits = 0_u8;

        for byte in input.bytes() {
            let value = match byte {
                b'A'..=b'Z' => byte - b'A',
                b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' => break,
                byte if byte.is_ascii_whitespace() => continue,
                other => panic!("unexpected base64 byte: {other:#04x}"),
            };

            buffer = (buffer << 6) | u32::from(value);
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                output.push((buffer >> bits) as u8);
                buffer &= if bits == 0 { 0 } else { (1_u32 << bits) - 1 };
            }
        }

        output
    }

    #[test]
    fn synthetic_cells_preserve_stored_order_and_raw_fields() {
        let bytes = cells_chunk_bytes();
        let chunk = parse_confirmed_0x2c_chunk(StreamPath("/Contents".into()), &bytes, 0)
            .expect("synthetic CELLS chunk");

        let cells = parse_confirmed_mature_cells(&bytes, &chunk).expect("parse CELLS");

        assert_eq!(cells.fields, chunk.fields);
        assert_eq!(cells.declared_cell_counts.len(), 1);
        assert_eq!(cells.declared_cell_counts[0].value, 2);
        assert_eq!(
            cells.declared_cell_counts[0].block_type,
            crate::BLOCK_TYPE_U32
        );
        assert_eq!(cells.records.len(), 2);
        assert_eq!(cells.records[0].record_index, 0);
        assert_eq!(cells.records[1].record_index, 1);
        assert_eq!(cells.records[0].start_rows[0].value, 0);
        assert_eq!(cells.records[0].start_columns[0].value, 0);
        assert_eq!(cells.records[1].start_rows[0].value, 0);
        assert_eq!(cells.records[1].start_columns[0].value, 1);
        assert!(cells.records[0].fields.iter().any(|field| field.id == 0x09));
    }

    #[test]
    fn apache_sample_has_six_stored_cells_in_expected_simple_grid_order() {
        let pub_bytes = decode_base64(include_str!(
            "../../pub-reader/tests/fixtures/Sample.pub.b64"
        ));
        let contents =
            pub_cfb::read_stream_reader(std::io::Cursor::new(pub_bytes.as_slice()), "/Contents")
                .expect("read pinned Apache Sample.pub Contents");
        let stream = StreamPath("/Contents".into());
        let header = parse_0x2c_header(stream.clone(), &contents).expect("0x2c header");
        let trailer = parse_confirmed_0x2c_trailer_root(&contents, &header).expect("0x2c trailer");

        let mut parsed_cells = Vec::new();
        for seq_num in 0..trailer.directory.slots.len() {
            let Some(reference) =
                parse_confirmed_chunk_reference(&contents, &trailer.directory, seq_num)
                    .expect("chunk reference")
            else {
                continue;
            };
            if !reference
                .raw_types
                .iter()
                .any(|field| field.value == CONTENTS_RAW_TYPE_CELLS)
            {
                continue;
            }

            for offset in &reference.chunk_offsets {
                let chunk = parse_confirmed_0x2c_chunk(stream.clone(), &contents, offset.value)
                    .expect("CELLS chunk");
                parsed_cells
                    .push(parse_confirmed_mature_cells(&contents, &chunk).expect("mature CELLS"));
            }
        }

        assert_eq!(parsed_cells.len(), 1);
        let cells = &parsed_cells[0];
        assert_eq!(
            cells
                .declared_cell_counts
                .iter()
                .map(|value| (value.value, value.block_type))
                .collect::<Vec<_>>(),
            vec![(6, crate::BLOCK_TYPE_U16)]
        );
        assert_eq!(cells.records.len(), 6);

        let coordinates: Vec<_> = cells
            .records
            .iter()
            .map(|record| {
                let coordinates = record
                    .effective_coordinates()
                    .expect("clean Sample.pub coordinates must be unambiguous");
                (
                    coordinates.start_row,
                    coordinates.end_row,
                    coordinates.start_column,
                    coordinates.end_column,
                )
            })
            .collect();
        assert!(
            cells.records.iter().any(|record| {
                record.start_rows.is_empty()
                    || record.end_rows.is_empty()
                    || record.start_columns.is_empty()
                    || record.end_columns.is_empty()
            }),
            "clean Sample.pub should exercise sparse omitted-zero coordinates"
        );
        assert_eq!(
            coordinates,
            vec![
                (0, 0, 0, 0),
                (0, 0, 1, 1),
                (1, 1, 0, 0),
                (1, 1, 1, 1),
                (2, 2, 0, 0),
                (2, 2, 1, 1),
            ]
        );
    }
}
