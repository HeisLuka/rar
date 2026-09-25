use crate::{
    ContentsFamily, ContentsReadError, Legacy0x22TableSlice, Legacy0x22TableTextMap,
    Legacy0x22TableTextReadError, Legacy0x22TextInfoMap, detect_family,
    parse_legacy_0x22_table_text_map,
};
use pub_core::{RawSpan, StreamPath};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const LEGACY_0X22_TRAILER_POINTER_OFFSET: usize = 0x16;
pub const LEGACY_0X22_TABLE_CHUNK_TYPE: u16 = 0x0001;

const LEGACY_DIRECTORY_ENTRY_SIZE: usize = 10;
const LEGACY_TABLE_HEADER_MIN_SIZE: usize = 62;
const LEGACY_TABLE_LIST_HEADER_SIZE: usize = 10;
const LEGACY_TABLE_AXIS_RECORD_SIZE: u16 = 14;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22TableListHeader {
    pub count: u16,
    pub max_count: u16,
    pub record_size: u16,
    pub observed_value_0: u16,
    pub observed_value_1: u16,
    pub source: RawSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22TableAxisSegment {
    /// Cumulative axis extent stored at the beginning of the 14-byte record.
    pub cumulative_emu: u32,
    pub cumulative_source: RawSpan,
    /// Difference from the previous cumulative value on the same axis.
    pub extent_emu: u32,
    /// Complete 14-byte source record; the remaining ten bytes are still raw.
    pub record_source: RawSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22TableChunk {
    pub directory_index: u16,
    pub directory_entry_source: RawSpan,
    pub chunk_id: u16,
    pub parent_id: u16,
    pub chunk_offset: u32,
    pub chunk_offset_source: RawSpan,
    pub chunk_type_source: RawSpan,
    /// Relative byte stored at chunk+3. The absolute data offset is
    /// chunk_offset + data_offset_delta.
    pub data_offset_delta: u8,
    pub data_offset_delta_source: RawSpan,
    pub data_offset: u32,
    /// Publisher 2 low-family text selector before the historical +65536
    /// synthetic namespace adjustment.
    pub local_text_index: u16,
    pub local_text_index_source: RawSpan,
    /// Default low-family text id used when no separate text-info override is
    /// present. This is not a universal cross-version effective text identifier.
    pub default_text_id: u32,
    /// Historical parser comment calls this "data size ?"; keep it observed.
    pub observed_header_u16_at_48: u16,
    pub observed_header_u16_at_48_source: RawSpan,
    pub column_count: u16,
    pub column_count_source: RawSpan,
    pub row_count: u16,
    pub row_count_source: RawSpan,
    pub width_emu: u32,
    pub width_source: RawSpan,
    pub height_emu: u32,
    pub height_source: RawSpan,
    pub list_header: Legacy0x22TableListHeader,
    pub columns: Vec<Legacy0x22TableAxisSegment>,
    pub rows: Vec<Legacy0x22TableAxisSegment>,
}

impl Legacy0x22TableChunk {
    pub fn is_materialized_grid(&self) -> bool {
        self.column_count != 0 && self.row_count != 0
    }

    pub fn cell_slot_count(&self) -> usize {
        usize::from(self.column_count) * usize::from(self.row_count)
    }

    pub fn columns_sum_to_declared_width(&self) -> bool {
        self.columns
            .last()
            .map(|segment| segment.cumulative_emu == self.width_emu)
            .unwrap_or(self.width_emu == 0)
    }

    pub fn rows_sum_to_declared_height(&self) -> bool {
        self.rows
            .last()
            .map(|segment| segment.cumulative_emu == self.height_emu)
            .unwrap_or(self.height_emu == 0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22TableCatalog {
    pub trailer_offset: u32,
    pub trailer_offset_source: RawSpan,
    pub directory_count: u16,
    pub directory_count_source: RawSpan,
    /// All type-0x0001 chunks, including zero-sized placeholders observed in
    /// the native corpus.
    pub table_chunks: Vec<Legacy0x22TableChunk>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22HorizontalMerge {
    pub row: u16,
    pub start_column: u16,
    pub end_column: u16,
    pub start_cell_index: u32,
    pub covered_cell_indices: Vec<u32>,
}

impl Legacy0x22HorizontalMerge {
    pub fn column_span(&self) -> u16 {
        self.end_column - self.start_column + 1
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22ResolvedTable {
    pub table_index: u32,
    pub effective_text_id: u32,
    pub object: Legacy0x22TableChunk,
    pub text: Legacy0x22TableSlice,
    /// Horizontal merge ranges recovered from grounded cell-style bits.
    ///
    /// Source cell slices remain intact, including covered slots. This layer
    /// records ownership/coverage without collapsing or moving their text.
    pub horizontal_merges: Vec<Legacy0x22HorizontalMerge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22ResolvedTableCatalog {
    pub tables: Vec<Legacy0x22ResolvedTable>,
    pub placeholder_chunk_indices: Vec<usize>,
    pub unapplied_style_boundary_indices: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Legacy0x22TableCatalogReadError {
    Contents(ContentsReadError),
    UnexpectedFamily(ContentsFamily),
    TrailerPointerOutOfBounds {
        offset: u32,
        stream_len: usize,
    },
    DirectoryOutOfBounds {
        trailer_offset: u32,
        count: u16,
        stream_len: usize,
    },
    ChunkOffsetOutOfBounds {
        directory_index: u16,
        chunk_id: u16,
        chunk_offset: u32,
        stream_len: usize,
    },
    TableHeaderTooShort {
        chunk_id: u16,
        chunk_offset: u32,
        data_offset: u32,
    },
    TableDataOffsetOutOfBounds {
        chunk_id: u16,
        data_offset: u32,
        chunk_end: u32,
    },
    UnsupportedExtendedListHeader {
        chunk_id: u16,
        count: u16,
        max_count: u16,
    },
    UnexpectedTableRecordSize {
        chunk_id: u16,
        found: u16,
    },
    TooFewTableRecords {
        chunk_id: u16,
        count: u16,
        required: u16,
    },
    TableRecordOutOfBounds {
        chunk_id: u16,
        record_index: u16,
        offset: u32,
        chunk_end: u32,
    },
    AxisNotMonotonic {
        chunk_id: u16,
        axis: &'static str,
        index: usize,
        previous: u32,
        current: u32,
    },
    Text(Legacy0x22TableTextReadError),
    TableCountMismatch {
        object_tables: usize,
        text_tables: usize,
    },
    DuplicateTextIdentity {
        effective_text_id: u32,
    },
    DuplicateObjectTextIdentity {
        effective_text_id: u32,
        first_chunk_id: u16,
        second_chunk_id: u16,
    },
    MissingTableTextIdentity {
        chunk_id: u16,
        effective_text_id: u32,
    },
    UnmatchedTableTextIdentities {
        text_ids: Vec<u32>,
    },
    TableCellCountMismatch {
        table_index: usize,
        rows: u16,
        columns: u16,
        cell_slots: usize,
        text_cells: usize,
    },
}

impl fmt::Display for Legacy0x22TableCatalogReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contents(error) => error.fmt(f),
            Self::UnexpectedFamily(found) => {
                write!(f, "expected legacy Contents family 0x22, found {found:?}")
            }
            Self::TrailerPointerOutOfBounds { offset, stream_len } => write!(
                f,
                "legacy 0x22 trailer pointer {offset:#x} is outside Contents length {stream_len}"
            ),
            Self::DirectoryOutOfBounds {
                trailer_offset,
                count,
                stream_len,
            } => write!(
                f,
                "legacy 0x22 directory at {trailer_offset:#x} with {count} entries exceeds Contents length {stream_len}"
            ),
            Self::ChunkOffsetOutOfBounds {
                directory_index,
                chunk_id,
                chunk_offset,
                stream_len,
            } => write!(
                f,
                "legacy 0x22 directory entry {directory_index} chunk {chunk_id} points to {chunk_offset:#x} outside Contents length {stream_len}"
            ),
            Self::TableHeaderTooShort {
                chunk_id,
                chunk_offset,
                data_offset,
            } => write!(
                f,
                "legacy 0x22 table chunk {chunk_id} header {chunk_offset:#x}..{data_offset:#x} is too short"
            ),
            Self::TableDataOffsetOutOfBounds {
                chunk_id,
                data_offset,
                chunk_end,
            } => write!(
                f,
                "legacy 0x22 table chunk {chunk_id} data offset {data_offset:#x} is outside chunk end {chunk_end:#x}"
            ),
            Self::UnsupportedExtendedListHeader {
                chunk_id,
                count,
                max_count,
            } => write!(
                f,
                "legacy 0x22 table chunk {chunk_id} uses unsupported extended list header count={count} max={max_count}"
            ),
            Self::UnexpectedTableRecordSize { chunk_id, found } => write!(
                f,
                "legacy 0x22 table chunk {chunk_id} axis record size is {found}, expected 14"
            ),
            Self::TooFewTableRecords {
                chunk_id,
                count,
                required,
            } => write!(
                f,
                "legacy 0x22 table chunk {chunk_id} has {count} axis records, needs at least {required}"
            ),
            Self::TableRecordOutOfBounds {
                chunk_id,
                record_index,
                offset,
                chunk_end,
            } => write!(
                f,
                "legacy 0x22 table chunk {chunk_id} axis record {record_index} at {offset:#x} exceeds chunk end {chunk_end:#x}"
            ),
            Self::AxisNotMonotonic {
                chunk_id,
                axis,
                index,
                previous,
                current,
            } => write!(
                f,
                "legacy 0x22 table chunk {chunk_id} {axis} cumulative record {index} decreases from {previous} to {current}"
            ),
            Self::Text(error) => write!(f, "legacy 0x22 table text map: {error}"),
            Self::TableCountMismatch {
                object_tables,
                text_tables,
            } => write!(
                f,
                "legacy 0x22 table object/text count mismatch: objects={object_tables}, text groups={text_tables}"
            ),
            Self::DuplicateTextIdentity { effective_text_id } => write!(
                f,
                "legacy 0x22 table text map contains duplicate effective text id {effective_text_id}"
            ),
            Self::DuplicateObjectTextIdentity {
                effective_text_id,
                first_chunk_id,
                second_chunk_id,
            } => write!(
                f,
                "legacy 0x22 table chunks {first_chunk_id} and {second_chunk_id} resolve to the same effective text id {effective_text_id}"
            ),
            Self::MissingTableTextIdentity {
                chunk_id,
                effective_text_id,
            } => write!(
                f,
                "legacy 0x22 table chunk {chunk_id} resolves to effective text id {effective_text_id}, but no table text group has that identity"
            ),
            Self::UnmatchedTableTextIdentities { text_ids } => write!(
                f,
                "legacy 0x22 table text identities remain unmatched after object resolution: {text_ids:?}"
            ),
            Self::TableCellCountMismatch {
                table_index,
                rows,
                columns,
                cell_slots,
                text_cells,
            } => write!(
                f,
                "legacy 0x22 table {table_index} grid {rows}x{columns} has {cell_slots} slots but text map has {text_cells} cells"
            ),
        }
    }
}

impl std::error::Error for Legacy0x22TableCatalogReadError {}

impl From<ContentsReadError> for Legacy0x22TableCatalogReadError {
    fn from(value: ContentsReadError) -> Self {
        Self::Contents(value)
    }
}

impl From<Legacy0x22TableTextReadError> for Legacy0x22TableCatalogReadError {
    fn from(value: Legacy0x22TableTextReadError) -> Self {
        Self::Text(value)
    }
}

fn span(stream: &StreamPath, offset: usize, len: usize) -> RawSpan {
    RawSpan {
        stream: stream.clone(),
        offset: offset as u64,
        len: len as u64,
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let raw = bytes.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes([raw[0], raw[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let raw = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

struct AxisReader<'a> {
    stream: &'a StreamPath,
    bytes: &'a [u8],
    chunk_id: u16,
    chunk_end: usize,
    data_offset: usize,
    record_size: usize,
}

impl AxisReader<'_> {
    fn segments(
        &self,
        first_record: usize,
        count: usize,
        axis: &'static str,
    ) -> Result<Vec<Legacy0x22TableAxisSegment>, Legacy0x22TableCatalogReadError> {
        let mut result = Vec::with_capacity(count);
        let mut previous = 0u32;

        for index in 0..count {
            let record_index = first_record + index;
            let record_offset =
                self.data_offset + LEGACY_TABLE_LIST_HEADER_SIZE + record_index * self.record_size;
            let record_end = record_offset + self.record_size;
            if record_end > self.chunk_end || record_end > self.bytes.len() {
                return Err(Legacy0x22TableCatalogReadError::TableRecordOutOfBounds {
                    chunk_id: self.chunk_id,
                    record_index: record_index as u16,
                    offset: record_offset as u32,
                    chunk_end: self.chunk_end as u32,
                });
            }

            let current = read_u32(self.bytes, record_offset).expect("record bounds checked above");
            if current < previous {
                return Err(Legacy0x22TableCatalogReadError::AxisNotMonotonic {
                    chunk_id: self.chunk_id,
                    axis,
                    index,
                    previous,
                    current,
                });
            }
            let extent = current - previous;
            previous = current;

            result.push(Legacy0x22TableAxisSegment {
                cumulative_emu: current,
                cumulative_source: span(self.stream, record_offset, 4),
                extent_emu: extent,
                record_source: span(self.stream, record_offset, self.record_size),
            });
        }

        Ok(result)
    }
}

/// Parse type-0x0001 Publisher 2 table chunks from the low-family Contents
/// directory.
///
/// The supported path is deliberately narrow: the verified 16-bit list header
/// and 14-byte cumulative geometry records used by the pinned native
/// Publisher 2 corpus. Zero-sized type-0x0001 placeholders are preserved.
pub fn parse_legacy_0x22_table_catalog(
    stream: StreamPath,
    bytes: &[u8],
) -> Result<Legacy0x22TableCatalog, Legacy0x22TableCatalogReadError> {
    let family = detect_family(bytes)?;
    if family != ContentsFamily::Family0x22 {
        return Err(Legacy0x22TableCatalogReadError::UnexpectedFamily(family));
    }

    let trailer_offset = read_u32(bytes, LEGACY_0X22_TRAILER_POINTER_OFFSET).ok_or(
        Legacy0x22TableCatalogReadError::TrailerPointerOutOfBounds {
            offset: u32::MAX,
            stream_len: bytes.len(),
        },
    )?;
    let trailer = trailer_offset as usize;
    if trailer + 2 > bytes.len() {
        return Err(Legacy0x22TableCatalogReadError::TrailerPointerOutOfBounds {
            offset: trailer_offset,
            stream_len: bytes.len(),
        });
    }

    let directory_count = read_u16(bytes, trailer).expect("two trailer bytes checked above");
    let directory_end = trailer
        .checked_add(2)
        .and_then(|value| {
            value.checked_add(usize::from(directory_count) * LEGACY_DIRECTORY_ENTRY_SIZE)
        })
        .ok_or(Legacy0x22TableCatalogReadError::DirectoryOutOfBounds {
            trailer_offset,
            count: directory_count,
            stream_len: bytes.len(),
        })?;
    if directory_end > bytes.len() {
        return Err(Legacy0x22TableCatalogReadError::DirectoryOutOfBounds {
            trailer_offset,
            count: directory_count,
            stream_len: bytes.len(),
        });
    }

    #[derive(Clone)]
    struct Entry {
        directory_index: u16,
        source: RawSpan,
        chunk_id: u16,
        parent_id: u16,
        chunk_offset: u32,
        chunk_offset_source: RawSpan,
        chunk_type: u16,
        chunk_type_source: RawSpan,
    }

    let mut entries = Vec::with_capacity(usize::from(directory_count));
    for index in 0..usize::from(directory_count) {
        let entry_offset = trailer + 2 + index * LEGACY_DIRECTORY_ENTRY_SIZE;
        let chunk_id = read_u16(bytes, entry_offset + 2).expect("directory bounds checked");
        let parent_id = read_u16(bytes, entry_offset + 4).expect("directory bounds checked");
        let chunk_offset = read_u32(bytes, entry_offset + 6).expect("directory bounds checked");
        let chunk = chunk_offset as usize;
        if chunk + 2 > bytes.len() {
            return Err(Legacy0x22TableCatalogReadError::ChunkOffsetOutOfBounds {
                directory_index: index as u16,
                chunk_id,
                chunk_offset,
                stream_len: bytes.len(),
            });
        }
        entries.push(Entry {
            directory_index: index as u16,
            source: span(&stream, entry_offset, LEGACY_DIRECTORY_ENTRY_SIZE),
            chunk_id,
            parent_id,
            chunk_offset,
            chunk_offset_source: span(&stream, entry_offset + 6, 4),
            chunk_type: read_u16(bytes, chunk).expect("chunk type bounds checked"),
            chunk_type_source: span(&stream, chunk, 2),
        });
    }

    let mut sorted_offsets = entries
        .iter()
        .map(|entry| entry.chunk_offset as usize)
        .collect::<Vec<_>>();
    sorted_offsets.sort_unstable();
    sorted_offsets.dedup();

    let mut table_chunks = Vec::new();
    for entry in entries
        .iter()
        .filter(|entry| entry.chunk_type == LEGACY_0X22_TABLE_CHUNK_TYPE)
    {
        let chunk = entry.chunk_offset as usize;
        let chunk_end = sorted_offsets
            .iter()
            .copied()
            .find(|offset| *offset > chunk)
            .unwrap_or(bytes.len());

        let data_offset_delta =
            *bytes
                .get(chunk + 3)
                .ok_or(Legacy0x22TableCatalogReadError::TableHeaderTooShort {
                    chunk_id: entry.chunk_id,
                    chunk_offset: entry.chunk_offset,
                    data_offset: entry.chunk_offset,
                })?;
        let data_offset = chunk.checked_add(usize::from(data_offset_delta)).ok_or(
            Legacy0x22TableCatalogReadError::TableHeaderTooShort {
                chunk_id: entry.chunk_id,
                chunk_offset: entry.chunk_offset,
                data_offset: u32::MAX,
            },
        )?;

        if data_offset < chunk + LEGACY_TABLE_HEADER_MIN_SIZE {
            return Err(Legacy0x22TableCatalogReadError::TableHeaderTooShort {
                chunk_id: entry.chunk_id,
                chunk_offset: entry.chunk_offset,
                data_offset: data_offset as u32,
            });
        }
        if data_offset + LEGACY_TABLE_LIST_HEADER_SIZE > chunk_end
            || data_offset + LEGACY_TABLE_LIST_HEADER_SIZE > bytes.len()
        {
            return Err(
                Legacy0x22TableCatalogReadError::TableDataOffsetOutOfBounds {
                    chunk_id: entry.chunk_id,
                    data_offset: data_offset as u32,
                    chunk_end: chunk_end as u32,
                },
            );
        }

        let local_text_index = read_u16(bytes, chunk + 46).expect("table header size checked");
        let observed_header_u16_at_48 =
            read_u16(bytes, chunk + 48).expect("table header size checked");
        let column_count = read_u16(bytes, chunk + 50).expect("table header size checked");
        let row_count = read_u16(bytes, chunk + 52).expect("table header size checked");
        let width_emu = read_u32(bytes, chunk + 54).expect("table header size checked");
        let height_emu = read_u32(bytes, chunk + 58).expect("table header size checked");

        let count = read_u16(bytes, data_offset).expect("list header bounds checked");
        let max_count = read_u16(bytes, data_offset + 2).expect("list header bounds checked");
        if max_count < count {
            return Err(
                Legacy0x22TableCatalogReadError::UnsupportedExtendedListHeader {
                    chunk_id: entry.chunk_id,
                    count,
                    max_count,
                },
            );
        }
        let record_size = read_u16(bytes, data_offset + 4).expect("list header bounds checked");
        if record_size != LEGACY_TABLE_AXIS_RECORD_SIZE {
            return Err(Legacy0x22TableCatalogReadError::UnexpectedTableRecordSize {
                chunk_id: entry.chunk_id,
                found: record_size,
            });
        }
        let observed_value_0 =
            read_u16(bytes, data_offset + 6).expect("list header bounds checked");
        let observed_value_1 =
            read_u16(bytes, data_offset + 8).expect("list header bounds checked");

        let required = column_count.saturating_add(row_count);
        if count < required {
            return Err(Legacy0x22TableCatalogReadError::TooFewTableRecords {
                chunk_id: entry.chunk_id,
                count,
                required,
            });
        }

        let axis_reader = AxisReader {
            stream: &stream,
            bytes,
            chunk_id: entry.chunk_id,
            chunk_end,
            data_offset,
            record_size: usize::from(record_size),
        };
        let columns = axis_reader.segments(0, usize::from(column_count), "column")?;
        let rows =
            axis_reader.segments(usize::from(column_count), usize::from(row_count), "row")?;

        table_chunks.push(Legacy0x22TableChunk {
            directory_index: entry.directory_index,
            directory_entry_source: entry.source.clone(),
            chunk_id: entry.chunk_id,
            parent_id: entry.parent_id,
            chunk_offset: entry.chunk_offset,
            chunk_offset_source: entry.chunk_offset_source.clone(),
            chunk_type_source: entry.chunk_type_source.clone(),
            data_offset_delta,
            data_offset_delta_source: span(&stream, chunk + 3, 1),
            data_offset: data_offset as u32,
            local_text_index,
            local_text_index_source: span(&stream, chunk + 46, 2),
            default_text_id: 65536 + u32::from(local_text_index),
            observed_header_u16_at_48,
            observed_header_u16_at_48_source: span(&stream, chunk + 48, 2),
            column_count,
            column_count_source: span(&stream, chunk + 50, 2),
            row_count,
            row_count_source: span(&stream, chunk + 52, 2),
            width_emu,
            width_source: span(&stream, chunk + 54, 4),
            height_emu,
            height_source: span(&stream, chunk + 58, 4),
            list_header: Legacy0x22TableListHeader {
                count,
                max_count,
                record_size,
                observed_value_0,
                observed_value_1,
                source: span(&stream, data_offset, LEGACY_TABLE_LIST_HEADER_SIZE),
            },
            columns,
            rows,
        });
    }

    Ok(Legacy0x22TableCatalog {
        trailer_offset,
        trailer_offset_source: span(&stream, LEGACY_0X22_TRAILER_POINTER_OFFSET, 4),
        directory_count,
        directory_count_source: span(&stream, trailer, 2),
        table_chunks,
    })
}

fn object_effective_text_id(
    object: &Legacy0x22TableChunk,
    text_info: Option<&Legacy0x22TextInfoMap>,
) -> u32 {
    if text_info
        .and_then(|map| map.end_for_owner(object.chunk_id))
        .is_some()
    {
        u32::from(object.chunk_id)
    } else {
        object.default_text_id
    }
}

fn horizontal_merges(
    object: &Legacy0x22TableChunk,
    text: &Legacy0x22TableSlice,
) -> Vec<Legacy0x22HorizontalMerge> {
    let columns = usize::from(object.column_count);
    let rows = usize::from(object.row_count);
    let mut result = Vec::new();

    for row in 0..rows {
        let mut column = 0usize;
        while column < columns {
            let start_index = row * columns + column;
            let starts = text.cells[start_index]
                .style_boundary
                .style
                .as_ref()
                .map(|style| style.starts_horizontal_merge())
                .unwrap_or(false);

            if !starts {
                column += 1;
                continue;
            }

            let mut end_column = column;
            let mut covered_cell_indices = Vec::new();
            while end_column + 1 < columns {
                let next_index = row * columns + end_column + 1;
                let continues = text.cells[next_index]
                    .style_boundary
                    .style
                    .as_ref()
                    .map(|style| style.continues_horizontal_merge())
                    .unwrap_or(false);
                if !continues {
                    break;
                }

                end_column += 1;
                covered_cell_indices.push(text.cells[next_index].cell_index);
            }

            // Match the historical low-family parser exactly: bit 0 only
            // changes layout when at least one immediately following cell in
            // the same row carries bit 2.
            if end_column > column {
                result.push(Legacy0x22HorizontalMerge {
                    row: row as u16,
                    start_column: column as u16,
                    end_column: end_column as u16,
                    start_cell_index: text.cells[start_index].cell_index,
                    covered_cell_indices,
                });
            }

            column = end_column + 1;
        }
    }

    result
}

/// Join materialized Publisher 2 table objects to cell-text groups by the
/// verified low-family effective text identity.
///
/// A table object normally addresses the synthetic text key
/// `65536 + local index`. When TEXT_INFO has an owner/frame boundary for that
/// table object's chunk id, the historical Publisher consumer instead uses the
/// explicit chunk id as the effective text key. The text parser applies the
/// same rule at that boundary, so this resolver does not depend on incidental
/// directory/text ordering and does not claim the chunk id is a native story id.
///
/// Each matched grid must still have exactly rows×columns 0x0F-delimited
/// slots. Zero-sized type-1 placeholder chunks are preserved by index but
/// excluded from the join.
pub fn parse_legacy_0x22_resolved_tables(
    stream: StreamPath,
    bytes: &[u8],
) -> Result<Legacy0x22ResolvedTableCatalog, Legacy0x22TableCatalogReadError> {
    let catalog = parse_legacy_0x22_table_catalog(stream.clone(), bytes)?;
    let text = parse_legacy_0x22_table_text_map(stream, bytes)?;
    let Legacy0x22TableTextMap {
        tables: text_tables,
        text_info,
        unapplied_style_boundary_indices,
    } = text;

    let mut materialized = Vec::new();
    let mut placeholders = Vec::new();
    for (index, table) in catalog.table_chunks.iter().enumerate() {
        if table.is_materialized_grid() {
            materialized.push(table.clone());
        } else {
            placeholders.push(index);
        }
    }

    if materialized.len() != text_tables.len() {
        return Err(Legacy0x22TableCatalogReadError::TableCountMismatch {
            object_tables: materialized.len(),
            text_tables: text_tables.len(),
        });
    }

    let mut text_by_id = BTreeMap::<u32, Legacy0x22TableSlice>::new();
    for table in text_tables {
        let effective_text_id = table.effective_text_id;
        if text_by_id.insert(effective_text_id, table).is_some() {
            return Err(Legacy0x22TableCatalogReadError::DuplicateTextIdentity {
                effective_text_id,
            });
        }
    }

    let mut object_text_chunks = BTreeMap::<u32, u16>::new();
    let mut seen_object_text_ids = BTreeSet::new();
    let mut tables = Vec::with_capacity(materialized.len());

    for (index, object) in materialized.into_iter().enumerate() {
        let effective_text_id = object_effective_text_id(&object, text_info.as_ref());
        if !seen_object_text_ids.insert(effective_text_id) {
            let first_chunk_id = object_text_chunks[&effective_text_id];
            return Err(
                Legacy0x22TableCatalogReadError::DuplicateObjectTextIdentity {
                    effective_text_id,
                    first_chunk_id,
                    second_chunk_id: object.chunk_id,
                },
            );
        }
        object_text_chunks.insert(effective_text_id, object.chunk_id);

        let text_table = text_by_id.remove(&effective_text_id).ok_or(
            Legacy0x22TableCatalogReadError::MissingTableTextIdentity {
                chunk_id: object.chunk_id,
                effective_text_id,
            },
        )?;

        let slots = object.cell_slot_count();
        if slots != text_table.cells.len() {
            return Err(Legacy0x22TableCatalogReadError::TableCellCountMismatch {
                table_index: index,
                rows: object.row_count,
                columns: object.column_count,
                cell_slots: slots,
                text_cells: text_table.cells.len(),
            });
        }

        let horizontal_merges = horizontal_merges(&object, &text_table);
        tables.push(Legacy0x22ResolvedTable {
            table_index: index as u32,
            effective_text_id,
            object,
            text: text_table,
            horizontal_merges,
        });
    }

    if !text_by_id.is_empty() {
        return Err(
            Legacy0x22TableCatalogReadError::UnmatchedTableTextIdentities {
                text_ids: text_by_id.into_keys().collect(),
            },
        );
    }

    Ok(Legacy0x22ResolvedTableCatalog {
        tables,
        placeholder_chunk_indices: placeholders,
        unapplied_style_boundary_indices,
    })
}
