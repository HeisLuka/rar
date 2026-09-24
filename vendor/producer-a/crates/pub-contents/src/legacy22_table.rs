use crate::{
    Legacy0x22CellStyleBoundary, Legacy0x22FormattingRunsError, Legacy0x22TextInfoMap,
    Legacy0x22TextInfoOwnerEnd, Legacy0x22TextInfoReadError, parse_legacy_0x22_formatting_runs,
    parse_legacy_0x22_text_info_map,
};
use pub_core::{RawSpan, StreamPath};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const LEGACY_0X22_CELL_SEPARATOR: u8 = 0x0f;
pub const LEGACY_0X22_SHAPE_SEPARATOR: u8 = 0x0c;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22TableCellSlice {
    pub table_index: u32,
    pub cell_index: u32,
    pub global_cell_index: u32,
    /// Raw cell bytes up to, but excluding, the 0x0F cell separator.
    /// This normally includes the terminal CRLF.
    pub raw_cell_bytes: Vec<u8>,
    pub raw_cell_source: RawSpan,
    /// Raw cell payload with one verified terminal CRLF removed.
    ///
    /// No codepage decoding is attempted at this layer.
    pub content_bytes: Vec<u8>,
    pub content_source: RawSpan,
    pub line_end_source: Option<RawSpan>,
    pub separator_source: RawSpan,
    pub style_boundary_index: usize,
    pub style_boundary: Legacy0x22CellStyleBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22TableSlice {
    pub table_index: u32,
    /// Ordinal advanced only by ordinary 0x0C ShapeEnd markers.
    pub ordinary_shape_index: u32,
    /// Synthetic low-family text key used when no explicit TEXT_INFO boundary applies.
    pub default_text_id: u32,
    /// Effective text identity used by the historical Publisher consumer.
    /// A TEXT_INFO owner boundary replaces the synthetic key with its explicit
    /// Contents owner/object id. This is not promoted to a native story-id field.
    pub effective_text_id: u32,
    pub text_boundary_source: Option<RawSpan>,
    pub text_id_source: Option<RawSpan>,
    pub text_end_offset_source: Option<RawSpan>,
    pub cells: Vec<Legacy0x22TableCellSlice>,
    pub source: RawSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22TableTextMap {
    pub tables: Vec<Legacy0x22TableSlice>,
    pub text_info: Option<Legacy0x22TextInfoMap>,
    /// Cell-style records that do not map to an in-range 0x0F separator.
    ///
    /// Publisher 2 non-table stories can still carry one default/allocation
    /// cell-style record whose boundary lies beyond text_end. Keeping these
    /// indices explicit prevents that storage sentinel from becoming a fake
    /// table cell.
    pub unapplied_style_boundary_indices: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Legacy0x22TableTextReadError {
    Formatting(Legacy0x22FormattingRunsError),
    TextInfo(Legacy0x22TextInfoReadError),
    TextInfoOwnerEndOutOfBounds {
        owner_id: u16,
        relative_end_offset: u32,
        text_len: usize,
    },
    DuplicateStyleBoundary {
        position: u32,
        first_index: usize,
        second_index: usize,
    },
    CellSeparatorWithoutStyleBoundary {
        separator_offset: u32,
    },
}

impl fmt::Display for Legacy0x22TableTextReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Formatting(error) => write!(f, "legacy 0x22 formatting: {error}"),
            Self::TextInfo(error) => write!(f, "legacy 0x22 text-info: {error}"),
            Self::TextInfoOwnerEndOutOfBounds {
                owner_id,
                relative_end_offset,
                text_len,
            } => write!(
                f,
                "legacy 0x22 text-info owner {owner_id} ends at relative offset {relative_end_offset}, outside text length {text_len}"
            ),
            Self::DuplicateStyleBoundary {
                position,
                first_index,
                second_index,
            } => write!(
                f,
                "legacy 0x22 cell-style boundaries {first_index} and {second_index} both apply at {position}"
            ),
            Self::CellSeparatorWithoutStyleBoundary { separator_offset } => write!(
                f,
                "legacy 0x22 cell separator at {separator_offset} has no matching cell-style boundary"
            ),
        }
    }
}

impl std::error::Error for Legacy0x22TableTextReadError {}

impl From<Legacy0x22FormattingRunsError> for Legacy0x22TableTextReadError {
    fn from(value: Legacy0x22FormattingRunsError) -> Self {
        Self::Formatting(value)
    }
}

impl From<Legacy0x22TextInfoReadError> for Legacy0x22TableTextReadError {
    fn from(value: Legacy0x22TextInfoReadError) -> Self {
        Self::TextInfo(value)
    }
}

fn raw_span(stream: &StreamPath, offset: usize, len: usize) -> RawSpan {
    RawSpan {
        stream: stream.clone(),
        offset: offset as u64,
        len: len as u64,
    }
}

fn text_info_boundaries(
    text_info: Option<&Legacy0x22TextInfoMap>,
    text_start: usize,
    text_end: usize,
) -> Result<BTreeMap<usize, Legacy0x22TextInfoOwnerEnd>, Legacy0x22TableTextReadError> {
    let mut result = BTreeMap::new();
    let text_len = text_end.saturating_sub(text_start);

    if let Some(text_info) = text_info {
        for end in &text_info.ends {
            let relative = usize::try_from(end.relative_end_offset).map_err(|_| {
                Legacy0x22TableTextReadError::TextInfoOwnerEndOutOfBounds {
                    owner_id: end.owner_id,
                    relative_end_offset: end.relative_end_offset,
                    text_len,
                }
            })?;
            if relative >= text_len {
                return Err(Legacy0x22TableTextReadError::TextInfoOwnerEndOutOfBounds {
                    owner_id: end.owner_id,
                    relative_end_offset: end.relative_end_offset,
                    text_len,
                });
            }
            result.insert(text_start + relative, end.clone());
        }
    }

    Ok(result)
}

fn ordinary_shape_index(bytes: &[u8], text_start: usize, cell_start: usize) -> u32 {
    bytes[text_start..cell_start]
        .iter()
        .filter(|byte| **byte == LEGACY_0X22_SHAPE_SEPARATOR)
        .count() as u32
}

fn finalize_text_identity(
    stream: &StreamPath,
    bytes: &[u8],
    text_end: usize,
    override_boundaries: &BTreeMap<usize, Legacy0x22TextInfoOwnerEnd>,
    table: &mut Legacy0x22TableSlice,
) {
    let Some(last_cell) = table.cells.last() else {
        return;
    };
    let last_separator = last_cell.separator_source.offset as usize;
    let literal_boundary = bytes[last_separator..text_end]
        .iter()
        .position(|byte| *byte == LEGACY_0X22_SHAPE_SEPARATOR)
        .map(|relative| last_separator + relative);
    let override_boundary = override_boundaries
        .range(last_separator..text_end)
        .next()
        .map(|(offset, end)| (*offset, end));

    match (literal_boundary, override_boundary) {
        (Some(literal), Some((override_offset, end))) if override_offset <= literal => {
            table.effective_text_id = u32::from(end.owner_id);
            table.text_boundary_source = Some(raw_span(stream, override_offset, 1));
            table.text_id_source = Some(end.owner_id_source.clone());
            table.text_end_offset_source = Some(end.end_offset_source.clone());
        }
        (None, Some((override_offset, end))) => {
            table.effective_text_id = u32::from(end.owner_id);
            table.text_boundary_source = Some(raw_span(stream, override_offset, 1));
            table.text_id_source = Some(end.owner_id_source.clone());
            table.text_end_offset_source = Some(end.end_offset_source.clone());
        }
        (Some(literal), _) => {
            table.text_boundary_source = Some(raw_span(stream, literal, 1));
        }
        (None, None) => {}
    }
}

/// Reconstruct bounded legacy Publisher 2 table-cell text ownership.
///
/// The low-family text grammar independently identifies 0x0F as CellEnd.
/// The recovered cell-style FKP lane stores each application position two
/// bytes before the following FC boundary. On native table fixtures this
/// position is the LF in the terminal CRLF immediately before 0x0F.
///
/// This parser deliberately stays at the Contents layer:
/// - it matches exact cell-style boundaries to exact 0x0F bytes;
/// - it groups consecutive cells into table slices when 0x0C shape
///   separators divide the text stream;
/// - it preserves raw cell bytes and provenance;
/// - it does not infer rows, columns, merged-cell coordinates, Unicode
///   codepages, or mature 0x2C table semantics.
pub fn parse_legacy_0x22_table_text_map(
    stream: StreamPath,
    bytes: &[u8],
) -> Result<Legacy0x22TableTextMap, Legacy0x22TableTextReadError> {
    let formatting = parse_legacy_0x22_formatting_runs(stream.clone(), bytes)?;
    let text_start = usize::try_from(formatting.descriptor.text_start)
        .expect("validated legacy text offsets fit usize");
    let text_end = usize::try_from(formatting.descriptor.text_end)
        .expect("validated legacy text offsets fit usize");
    let text_info = parse_legacy_0x22_text_info_map(stream.clone(), bytes)?;
    let override_boundaries = text_info_boundaries(text_info.as_ref(), text_start, text_end)?;

    if text_start == text_end {
        return Ok(Legacy0x22TableTextMap {
            tables: Vec::new(),
            text_info,
            unapplied_style_boundary_indices: Vec::new(),
        });
    }

    let mut style_by_separator = BTreeMap::<usize, usize>::new();
    let mut unapplied = Vec::new();

    for (index, boundary) in formatting.cell_style_boundaries.iter().enumerate() {
        let Ok(position) = usize::try_from(boundary.position) else {
            unapplied.push(index);
            continue;
        };
        let Some(separator) = position.checked_add(1) else {
            unapplied.push(index);
            continue;
        };

        if position < text_start
            || separator >= text_end
            || bytes.get(separator).copied() != Some(LEGACY_0X22_CELL_SEPARATOR)
        {
            unapplied.push(index);
            continue;
        }

        if let Some(first_index) = style_by_separator.insert(separator, index) {
            return Err(Legacy0x22TableTextReadError::DuplicateStyleBoundary {
                position: boundary.position,
                first_index,
                second_index: index,
            });
        }
    }

    let separators: Vec<usize> = bytes[text_start..text_end]
        .iter()
        .enumerate()
        .filter_map(|(relative, byte)| {
            (*byte == LEGACY_0X22_CELL_SEPARATOR).then_some(text_start + relative)
        })
        .collect();

    for separator in &separators {
        if !style_by_separator.contains_key(separator) {
            return Err(
                Legacy0x22TableTextReadError::CellSeparatorWithoutStyleBoundary {
                    separator_offset: *separator as u32,
                },
            );
        }
    }

    let mut used_boundaries = BTreeSet::new();
    let mut tables = Vec::<Legacy0x22TableSlice>::new();

    for separator in separators {
        let boundary_index = style_by_separator[&separator];
        used_boundaries.insert(boundary_index);
        let boundary = formatting.cell_style_boundaries[boundary_index].clone();

        let physical_previous = bytes[text_start..separator]
            .iter()
            .rposition(|byte| {
                *byte == LEGACY_0X22_CELL_SEPARATOR || *byte == LEGACY_0X22_SHAPE_SEPARATOR
            })
            .map(|relative| text_start + relative);
        let override_previous = override_boundaries
            .range(text_start..separator)
            .next_back()
            .map(|(offset, _)| *offset);
        let previous_delimiter = match (physical_previous, override_previous) {
            (Some(physical), Some(override_offset)) => Some(physical.max(override_offset)),
            (Some(physical), None) => Some(physical),
            (None, Some(override_offset)) => Some(override_offset),
            (None, None) => None,
        };

        let cell_start = previous_delimiter.map_or(text_start, |offset| offset + 1);
        let raw_cell_end = separator;
        let has_terminal_crlf = raw_cell_end >= cell_start + 2
            && bytes[raw_cell_end - 2] == 0x0d
            && bytes[raw_cell_end - 1] == 0x0a;
        let content_end = if has_terminal_crlf {
            raw_cell_end - 2
        } else {
            raw_cell_end
        };

        let previous_is_text_boundary = previous_delimiter.is_some_and(|offset| {
            override_boundaries.contains_key(&offset)
                || bytes.get(offset).copied() == Some(LEGACY_0X22_SHAPE_SEPARATOR)
        });
        let new_table = tables.is_empty() || previous_is_text_boundary;

        if new_table {
            let table_index = tables.len() as u32;
            let ordinary_shape_index = ordinary_shape_index(bytes, text_start, cell_start);
            let default_text_id = 65_536 + ordinary_shape_index;
            tables.push(Legacy0x22TableSlice {
                table_index,
                ordinary_shape_index,
                default_text_id,
                effective_text_id: default_text_id,
                text_boundary_source: None,
                text_id_source: None,
                text_end_offset_source: None,
                cells: Vec::new(),
                source: raw_span(&stream, cell_start, separator + 1 - cell_start),
            });
        }

        let table = tables
            .last_mut()
            .expect("a table is created before its first cell");
        let cell_index = table.cells.len() as u32;
        let global_cell_index = used_boundaries.len() as u32 - 1;

        let line_end_source = has_terminal_crlf.then(|| raw_span(&stream, raw_cell_end - 2, 2));

        table.cells.push(Legacy0x22TableCellSlice {
            table_index: table.table_index,
            cell_index,
            global_cell_index,
            raw_cell_bytes: bytes[cell_start..raw_cell_end].to_vec(),
            raw_cell_source: raw_span(&stream, cell_start, raw_cell_end - cell_start),
            content_bytes: bytes[cell_start..content_end].to_vec(),
            content_source: raw_span(&stream, cell_start, content_end - cell_start),
            line_end_source,
            separator_source: raw_span(&stream, separator, 1),
            style_boundary_index: boundary_index,
            style_boundary: boundary,
        });

        let table_end = separator + 1;
        table.source.len = table_end as u64 - table.source.offset;
    }

    for table in &mut tables {
        finalize_text_identity(&stream, bytes, text_end, &override_boundaries, table);
    }

    for index in 0..formatting.cell_style_boundaries.len() {
        if !used_boundaries.contains(&index) && !unapplied.contains(&index) {
            unapplied.push(index);
        }
    }

    Ok(Legacy0x22TableTextMap {
        tables,
        text_info,
        unapplied_style_boundary_indices: unapplied,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_separator_starts_a_new_table_group() {
        let stream = StreamPath("/Contents".into());
        let mut bytes = vec![0u8; 0x800];
        bytes[0..4].copy_from_slice(&crate::CONTENTS_0X22_MAGIC);
        bytes[0x12..0x16].copy_from_slice(&0x22u32.to_le_bytes());
        bytes[0x16..0x1a].copy_from_slice(&0x700u32.to_le_bytes());
        bytes[0x700..0x702].copy_from_slice(&0u16.to_le_bytes());

        let descriptor = 0x22 + 14;
        let text_start = 0x50u32;
        let text = b"A\r\n\x0f\x0cB\r\n\x0f\x0c";
        let text_end = text_start + text.len() as u32;
        bytes[descriptor..descriptor + 4].copy_from_slice(&text_start.to_le_bytes());
        bytes[descriptor + 4..descriptor + 8].copy_from_slice(&text_end.to_le_bytes());
        bytes[descriptor + 8..descriptor + 10].copy_from_slice(&1u16.to_le_bytes());
        bytes[descriptor + 10..descriptor + 12].copy_from_slice(&2u16.to_le_bytes());
        bytes[descriptor + 12..descriptor + 14].copy_from_slice(&3u16.to_le_bytes());
        bytes[descriptor + 14..descriptor + 16].copy_from_slice(&4u16.to_le_bytes());
        bytes[text_start as usize..text_end as usize].copy_from_slice(text);

        let ch = 0x200;
        bytes[ch..ch + 4].copy_from_slice(&text_start.to_le_bytes());
        bytes[ch + 4..ch + 8].copy_from_slice(&text_end.to_le_bytes());
        bytes[ch + 8] = 0;
        bytes[ch + 0x1ff] = 1;

        let pap = 0x400;
        bytes[pap..pap + 4].copy_from_slice(&text_start.to_le_bytes());
        bytes[pap + 4..pap + 8].copy_from_slice(&(text_end - 1).to_le_bytes());
        bytes[pap + 8] = 0;
        bytes[pap + 0x1ff] = 1;

        let cell = 0x600;
        let first_separator = text_start + 3;
        let second_separator = text_start + 8;
        bytes[cell..cell + 4].copy_from_slice(&text_start.to_le_bytes());
        bytes[cell + 4..cell + 8].copy_from_slice(&(first_separator + 1).to_le_bytes());
        bytes[cell + 8..cell + 12].copy_from_slice(&(second_separator + 1).to_le_bytes());
        bytes[cell + 12] = 0;
        bytes[cell + 13] = 0;
        bytes[cell + 0x1ff] = 2;

        let parsed = parse_legacy_0x22_table_text_map(stream, &bytes).unwrap();
        assert_eq!(parsed.tables.len(), 2);
        assert_eq!(parsed.tables[0].ordinary_shape_index, 0);
        assert_eq!(parsed.tables[0].default_text_id, 65_536);
        assert_eq!(parsed.tables[0].effective_text_id, 65_536);
        assert_eq!(parsed.tables[0].cells[0].content_bytes, b"A");
        assert_eq!(parsed.tables[1].ordinary_shape_index, 1);
        assert_eq!(parsed.tables[1].default_text_id, 65_537);
        assert_eq!(parsed.tables[1].effective_text_id, 65_537);
        assert_eq!(parsed.tables[1].cells[0].content_bytes, b"B");
        assert!(parsed.text_info.is_none());
        assert!(parsed.unapplied_style_boundary_indices.is_empty());
    }
}
