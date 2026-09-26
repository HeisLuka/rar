use crate::{QuillStoryCatalog, QuillStorySlice};
use pub_core::{QuillSyid, RawSpan};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

const FONT: [u8; 4] = *b"FONT";
const FDPC: [u8; 4] = *b"FDPC";

const VARIABLE_BLOCK_TYPES: [u8; 8] = [0xC0, 0x80, 0x82, 0x88, 0x8A, 0x90, 0x98, 0xA0];
const GENERAL_CONTAINER: u8 = 0x88;
const FONT_INDEX_CONTAINER_ID: u8 = 0x24;
const TEXT_SIZE_ID: u8 = 0x0C;

pub const QUILL_TEXT_SIZE_EMU_PER_POINT: u32 = 12_700;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillTypographyCatalog {
    pub font_names: Vec<String>,
    pub ranges: Vec<QuillTypographyRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explicit_runs: Vec<QuillExplicitTypographyRun>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unknown_block_types_assumed_zero_length: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillTypographyRange {
    pub global_start_utf16: u32,
    pub global_end_utf16: u32,
    pub fdpc_descriptor_ordinal: u32,
    pub fdpc_style_ordinal: u32,
    pub fdpc_style_source: RawSpan,
    pub text_offset_source: RawSpan,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub font_indices: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub font_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_sizes_emu: Vec<u32>,
    pub story_intersections: Vec<QuillTypographyStoryIntersection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillTypographyStoryIntersection {
    pub story_index: u32,
    pub story_syid: QuillSyid,
    pub global_start_utf16: u32,
    pub global_end_utf16: u32,
    pub story_start_utf16: u32,
    pub story_end_utf16: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillExplicitTypographyRun {
    pub story_index: u32,
    pub story_syid: QuillSyid,
    pub story_start_utf16: u32,
    pub story_end_utf16: u32,
    pub font_index: u32,
    pub font_name: String,
    pub text_size_emu: u32,
    pub fdpc_descriptor_ordinal: u32,
    pub fdpc_style_ordinal: u32,
    pub fdpc_style_source: RawSpan,
}

impl QuillExplicitTypographyRun {
    pub fn text_size_points_exact(&self) -> Option<u32> {
        (self.text_size_emu % QUILL_TEXT_SIZE_EMU_PER_POINT == 0)
            .then_some(self.text_size_emu / QUILL_TEXT_SIZE_EMU_PER_POINT)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuillTypographyReadError {
    message: String,
}

impl QuillTypographyReadError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for QuillTypographyReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for QuillTypographyReadError {}

#[derive(Debug, Clone)]
struct StyleObservation {
    fdpc_descriptor_ordinal: u32,
    fdpc_style_ordinal: u32,
    absolute_text_end: u32,
    text_offset_source: RawSpan,
    style_source: RawSpan,
    font_indices: Vec<u32>,
    font_names: Vec<String>,
    text_sizes_emu: Vec<u32>,
}

#[derive(Debug, Clone, Copy)]
struct BlockObservation {
    id: u8,
    block_type: u8,
    data_offset: usize,
    end: usize,
    value: Option<u32>,
}

pub fn parse_bounded_typography(
    bytes: &[u8],
    story_catalog: &QuillStoryCatalog,
) -> Result<QuillTypographyCatalog, QuillTypographyReadError> {
    let descriptors = story_catalog
        .descriptor_nodes
        .iter()
        .flat_map(|node| node.descriptors.iter())
        .enumerate()
        .collect::<Vec<_>>();

    let font_names = parse_font_catalog(bytes, &descriptors)?;
    let mut unknown_block_types = BTreeSet::new();
    let mut styles = parse_fdpc_styles(
        bytes,
        story_catalog,
        &descriptors,
        &font_names,
        &mut unknown_block_types,
    )?;

    styles.sort_by_key(|style| style.absolute_text_end);

    let text_start = u32::try_from(story_catalog.text.source.offset)
        .map_err(|_| QuillTypographyReadError::new("TEXT offset exceeds u32"))?;
    let text_len = u32::try_from(story_catalog.text.source.len)
        .map_err(|_| QuillTypographyReadError::new("TEXT length exceeds u32"))?;
    let text_end = text_start
        .checked_add(text_len)
        .ok_or_else(|| QuillTypographyReadError::new("TEXT span overflows u32"))?;

    let story_extents = build_story_extents(&story_catalog.stories)?;

    let total_utf16 = story_extents
        .last()
        .map(|extent| extent.global_end_utf16)
        .unwrap_or(0);
    let expected_text_bytes = total_utf16
        .checked_mul(2)
        .ok_or_else(|| QuillTypographyReadError::new("aggregate Story UTF-16 length overflows"))?;
    if expected_text_bytes != text_len {
        return Err(QuillTypographyReadError::new(format!(
            "aggregate Story UTF-16 bytes {expected_text_bytes} do not match TEXT length {text_len}"
        )));
    }

    let mut previous_end_utf16 = 0_u32;
    let mut ranges = Vec::new();
    for style in styles {
        if style.absolute_text_end < text_start || style.absolute_text_end > text_end {
            return Err(QuillTypographyReadError::new(format!(
                "FDPC text offset 0x{:x} is outside TEXT [0x{text_start:x}, 0x{text_end:x}]",
                style.absolute_text_end
            )));
        }

        let byte_delta = style.absolute_text_end - text_start;
        if byte_delta % 2 != 0 {
            return Err(QuillTypographyReadError::new(format!(
                "FDPC text offset 0x{:x} is not aligned to UTF-16LE code units",
                style.absolute_text_end
            )));
        }
        let global_end_utf16 = byte_delta / 2;
        if global_end_utf16 < previous_end_utf16 {
            return Err(QuillTypographyReadError::new(format!(
                "FDPC range end regressed from {previous_end_utf16} to {global_end_utf16}"
            )));
        }

        if global_end_utf16 == previous_end_utf16 {
            continue;
        }

        let intersections = story_extents
            .iter()
            .filter_map(|extent| {
                let start = previous_end_utf16.max(extent.global_start_utf16);
                let end = global_end_utf16.min(extent.global_end_utf16);
                (start < end).then(|| QuillTypographyStoryIntersection {
                    story_index: extent.story_index,
                    story_syid: extent.story_syid,
                    global_start_utf16: start,
                    global_end_utf16: end,
                    story_start_utf16: start - extent.global_start_utf16,
                    story_end_utf16: end - extent.global_start_utf16,
                })
            })
            .collect::<Vec<_>>();

        ranges.push(QuillTypographyRange {
            global_start_utf16: previous_end_utf16,
            global_end_utf16,
            fdpc_descriptor_ordinal: style.fdpc_descriptor_ordinal,
            fdpc_style_ordinal: style.fdpc_style_ordinal,
            fdpc_style_source: style.style_source,
            text_offset_source: style.text_offset_source,
            font_indices: style.font_indices,
            font_names: style.font_names,
            text_sizes_emu: style.text_sizes_emu,
            story_intersections: intersections,
        });
        previous_end_utf16 = global_end_utf16;
    }

    if previous_end_utf16 != total_utf16 {
        return Err(QuillTypographyReadError::new(format!(
            "FDPC terminal UTF-16 boundary {previous_end_utf16} does not close Story corpus at {total_utf16}"
        )));
    }

    let mut explicit_runs = Vec::new();
    for range in &ranges {
        let mut font_pairs = range
            .font_indices
            .iter()
            .copied()
            .zip(range.font_names.iter().cloned())
            .collect::<Vec<_>>();
        font_pairs.sort();
        font_pairs.dedup();

        let mut text_sizes = range.text_sizes_emu.clone();
        text_sizes.sort_unstable();
        text_sizes.dedup();

        let ([(font_index, font_name)], [text_size_emu]) =
            (font_pairs.as_slice(), text_sizes.as_slice())
        else {
            continue;
        };

        for intersection in &range.story_intersections {
            explicit_runs.push(QuillExplicitTypographyRun {
                story_index: intersection.story_index,
                story_syid: intersection.story_syid,
                story_start_utf16: intersection.story_start_utf16,
                story_end_utf16: intersection.story_end_utf16,
                font_index: *font_index,
                font_name: font_name.clone(),
                text_size_emu: *text_size_emu,
                fdpc_descriptor_ordinal: range.fdpc_descriptor_ordinal,
                fdpc_style_ordinal: range.fdpc_style_ordinal,
                fdpc_style_source: range.fdpc_style_source.clone(),
            });
        }
    }

    Ok(QuillTypographyCatalog {
        font_names,
        ranges,
        explicit_runs,
        unknown_block_types_assumed_zero_length: unknown_block_types.into_iter().collect(),
    })
}

#[derive(Debug, Clone, Copy)]
struct StoryExtent {
    story_index: u32,
    story_syid: QuillSyid,
    global_start_utf16: u32,
    global_end_utf16: u32,
}

fn build_story_extents(
    stories: &[QuillStorySlice],
) -> Result<Vec<StoryExtent>, QuillTypographyReadError> {
    let mut cursor = 0_u32;
    let mut extents = Vec::with_capacity(stories.len());
    for story in stories {
        let end = cursor
            .checked_add(story.utf16_code_units)
            .ok_or_else(|| QuillTypographyReadError::new("Story UTF-16 extent overflows u32"))?;
        extents.push(StoryExtent {
            story_index: story.index,
            story_syid: story.syid,
            global_start_utf16: cursor,
            global_end_utf16: end,
        });
        cursor = end;
    }
    Ok(extents)
}

fn parse_font_catalog(
    bytes: &[u8],
    descriptors: &[(usize, &crate::QuillChunkDescriptor)],
) -> Result<Vec<String>, QuillTypographyReadError> {
    let mut names = Vec::new();
    for (_, descriptor) in descriptors
        .iter()
        .copied()
        .filter(|(_, descriptor)| descriptor.name.value == FONT)
    {
        let start = to_usize(descriptor.data_offset.value, "FONT offset")?;
        let len = to_usize(descriptor.data_length.value, "FONT length")?;
        let end = checked_end(start, len, bytes.len(), "FONT chunk")?;
        if start + 8 > end {
            return Err(QuillTypographyReadError::new(
                "FONT chunk is shorter than fixed prefix",
            ));
        }

        let count = read_u32(bytes, start + 4, end)?;
        let count_usize = to_usize(count, "FONT count")?;
        let index_bytes = count_usize
            .checked_mul(4)
            .ok_or_else(|| QuillTypographyReadError::new("FONT index table overflows usize"))?;
        let mut cursor = start
            .checked_add(20)
            .and_then(|value| value.checked_add(index_bytes))
            .ok_or_else(|| QuillTypographyReadError::new("FONT records offset overflows usize"))?;
        if cursor > end {
            return Err(QuillTypographyReadError::new(
                "FONT index table exceeds chunk",
            ));
        }

        for _ in 0..count {
            let name_units = usize::from(read_u16(bytes, cursor, end)?);
            cursor += 2;
            let name_bytes_len = name_units
                .checked_mul(2)
                .ok_or_else(|| QuillTypographyReadError::new("FONT name length overflows usize"))?;
            let name_end = cursor
                .checked_add(name_bytes_len)
                .ok_or_else(|| QuillTypographyReadError::new("FONT name end overflows usize"))?;
            let trailing_end = name_end
                .checked_add(4)
                .ok_or_else(|| QuillTypographyReadError::new("FONT record end overflows usize"))?;
            if trailing_end > end {
                return Err(QuillTypographyReadError::new("FONT record exceeds chunk"));
            }

            let mut units = Vec::with_capacity(name_units);
            for pair in bytes[cursor..name_end].chunks_exact(2) {
                units.push(u16::from_le_bytes([pair[0], pair[1]]));
            }
            let name = String::from_utf16(&units).map_err(|error| {
                QuillTypographyReadError::new(format!("FONT name is invalid UTF-16: {error}"))
            })?;
            names.push(name);
            cursor = trailing_end;
        }

        if cursor != end {
            return Err(QuillTypographyReadError::new(format!(
                "FONT records leave {} trailing bytes",
                end - cursor
            )));
        }
    }

    if names.is_empty() {
        return Err(QuillTypographyReadError::new("no FONT records"));
    }
    Ok(names)
}

fn parse_fdpc_styles(
    bytes: &[u8],
    story_catalog: &QuillStoryCatalog,
    descriptors: &[(usize, &crate::QuillChunkDescriptor)],
    font_names: &[String],
    unknown_block_types: &mut BTreeSet<u8>,
) -> Result<Vec<StyleObservation>, QuillTypographyReadError> {
    let stream = story_catalog.text.source.stream.clone();
    let mut styles = Vec::new();

    for (descriptor_ordinal, descriptor) in descriptors
        .iter()
        .copied()
        .filter(|(_, descriptor)| descriptor.name.value == FDPC)
    {
        let start = to_usize(descriptor.data_offset.value, "FDPC offset")?;
        let len = to_usize(descriptor.data_length.value, "FDPC length")?;
        let end = checked_end(start, len, bytes.len(), "FDPC chunk")?;
        if start + 8 > end {
            return Err(QuillTypographyReadError::new(
                "FDPC chunk is shorter than fixed prefix",
            ));
        }

        let count = usize::from(read_u16(bytes, start, end)?);
        let offsets_start = start + 8;
        let chunk_offsets_start = offsets_start
            .checked_add(count.checked_mul(4).ok_or_else(|| {
                QuillTypographyReadError::new("FDPC text offset table overflows usize")
            })?)
            .ok_or_else(|| QuillTypographyReadError::new("FDPC text offset table end overflows"))?;
        let body_start = chunk_offsets_start
            .checked_add(count.checked_mul(2).ok_or_else(|| {
                QuillTypographyReadError::new("FDPC style offset table overflows usize")
            })?)
            .ok_or_else(|| {
                QuillTypographyReadError::new("FDPC style offset table end overflows")
            })?;
        if body_start > end {
            return Err(QuillTypographyReadError::new("FDPC tables exceed chunk"));
        }

        for style_ordinal in 0..count {
            let text_offset_pos = offsets_start + style_ordinal * 4;
            let absolute_text_end = read_u32(bytes, text_offset_pos, end)?;
            let relative_style_offset = usize::from(read_u16(
                bytes,
                chunk_offsets_start + style_ordinal * 2,
                end,
            )?);
            let style_start = start
                .checked_add(relative_style_offset)
                .ok_or_else(|| QuillTypographyReadError::new("FDPC style offset overflows"))?;
            let style_len_u32 = read_u32(bytes, style_start, end)?;
            let style_len = to_usize(style_len_u32, "FDPC style length")?;
            if style_len < 4 {
                return Err(QuillTypographyReadError::new(
                    "FDPC style length is smaller than header",
                ));
            }
            let style_end = checked_end(style_start, style_len, end, "FDPC style")?;
            let mut cursor = style_start + 4;
            let mut font_indices = Vec::new();
            let mut text_sizes_emu = Vec::new();

            while cursor < style_end {
                let (block, next) = parse_block(bytes, cursor, style_end, unknown_block_types)?;
                if block.id == FONT_INDEX_CONTAINER_ID {
                    if let Some(index) =
                        extract_primary_font_index(bytes, block, unknown_block_types)?
                    {
                        let index_usize = to_usize(index, "FDPC font index")?;
                        if index_usize >= font_names.len() {
                            return Err(QuillTypographyReadError::new(format!(
                                "FDPC font index {index} is outside FONT catalog of {} records",
                                font_names.len()
                            )));
                        }
                        font_indices.push(index);
                    }
                }
                if block.id == TEXT_SIZE_ID {
                    if let Some(value) = block.value {
                        text_sizes_emu.push(value);
                    }
                }
                cursor = next;
            }
            if cursor != style_end {
                return Err(QuillTypographyReadError::new(
                    "FDPC style did not close exactly",
                ));
            }

            let joined_names = font_indices
                .iter()
                .map(|index| font_names[*index as usize].clone())
                .collect::<Vec<_>>();

            styles.push(StyleObservation {
                fdpc_descriptor_ordinal: u32::try_from(descriptor_ordinal)
                    .map_err(|_| QuillTypographyReadError::new("descriptor ordinal exceeds u32"))?,
                fdpc_style_ordinal: u32::try_from(style_ordinal)
                    .map_err(|_| QuillTypographyReadError::new("style ordinal exceeds u32"))?,
                absolute_text_end,
                text_offset_source: RawSpan {
                    stream: stream.clone(),
                    offset: text_offset_pos as u64,
                    len: 4,
                },
                style_source: RawSpan {
                    stream: stream.clone(),
                    offset: style_start as u64,
                    len: style_len as u64,
                },
                font_indices,
                font_names: joined_names,
                text_sizes_emu,
            });
        }
    }

    if styles.is_empty() {
        return Err(QuillTypographyReadError::new("no FDPC styles"));
    }
    Ok(styles)
}

fn extract_primary_font_index(
    bytes: &[u8],
    block: BlockObservation,
    unknown_block_types: &mut BTreeSet<u8>,
) -> Result<Option<u32>, QuillTypographyReadError> {
    if !VARIABLE_BLOCK_TYPES.contains(&block.block_type) {
        return Ok(None);
    }

    let mut cursor = block
        .data_offset
        .checked_add(4)
        .ok_or_else(|| QuillTypographyReadError::new("font container payload offset overflows"))?;
    while cursor < block.end {
        let (child, next) = parse_block(bytes, cursor, block.end, unknown_block_types)?;
        if child.block_type == GENERAL_CONTAINER {
            let inner_start = child.data_offset.checked_add(4).ok_or_else(|| {
                QuillTypographyReadError::new("general container payload overflows")
            })?;
            if inner_start >= child.end {
                return Ok(None);
            }
            let (value_block, _) = parse_block(bytes, inner_start, child.end, unknown_block_types)?;
            return Ok(value_block.value);
        }
        cursor = next;
    }
    Ok(None)
}

fn parse_block(
    bytes: &[u8],
    start: usize,
    limit: usize,
    unknown_block_types: &mut BTreeSet<u8>,
) -> Result<(BlockObservation, usize), QuillTypographyReadError> {
    if start + 2 > limit {
        return Err(QuillTypographyReadError::new(format!(
            "Quill style block header exceeds limit at 0x{start:x}"
        )));
    }
    let id = bytes[start];
    let block_type = bytes[start + 1];
    let data_offset = start + 2;

    let (end, value) = if VARIABLE_BLOCK_TYPES.contains(&block_type) {
        let declared = to_usize(
            read_u32(bytes, data_offset, limit)?,
            "variable block length",
        )?;
        if declared < 4 {
            return Err(QuillTypographyReadError::new(format!(
                "variable block at 0x{start:x} has length {declared}"
            )));
        }
        let end = data_offset
            .checked_add(declared)
            .ok_or_else(|| QuillTypographyReadError::new("variable block end overflows"))?;
        if end > limit {
            return Err(QuillTypographyReadError::new(format!(
                "variable block at 0x{start:x} exceeds style boundary"
            )));
        }
        (end, None)
    } else {
        let data_len = match block_type {
            0x78 | 0x05 | 0x08 | 0x0A => 0,
            0x10 | 0x12 | 0x18 | 0x1A | 0x07 => 2,
            0x20 | 0x22 | 0x58 | 0x68 | 0x70 | 0xB8 => 4,
            0x28 => 8,
            0x38 => 16,
            0x48 => 24,
            other => {
                unknown_block_types.insert(other);
                0
            }
        };
        let end = data_offset
            .checked_add(data_len)
            .ok_or_else(|| QuillTypographyReadError::new("fixed block end overflows"))?;
        if end > limit {
            return Err(QuillTypographyReadError::new(format!(
                "fixed block at 0x{start:x} exceeds style boundary"
            )));
        }
        let value = match data_len {
            2 => Some(u32::from(read_u16(bytes, data_offset, limit)?)),
            4 => Some(read_u32(bytes, data_offset, limit)?),
            _ => None,
        };
        (end, value)
    };

    Ok((
        BlockObservation {
            id,
            block_type,
            data_offset,
            end,
            value,
        },
        end,
    ))
}

fn read_u16(bytes: &[u8], offset: usize, limit: usize) -> Result<u16, QuillTypographyReadError> {
    if offset + 2 > limit || offset + 2 > bytes.len() {
        return Err(QuillTypographyReadError::new(format!(
            "u16 read out of bounds at 0x{offset:x}"
        )));
    }
    Ok(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]))
}

fn read_u32(bytes: &[u8], offset: usize, limit: usize) -> Result<u32, QuillTypographyReadError> {
    if offset + 4 > limit || offset + 4 > bytes.len() {
        return Err(QuillTypographyReadError::new(format!(
            "u32 read out of bounds at 0x{offset:x}"
        )));
    }
    Ok(u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]))
}

fn to_usize(value: u32, label: &str) -> Result<usize, QuillTypographyReadError> {
    usize::try_from(value)
        .map_err(|_| QuillTypographyReadError::new(format!("{label} exceeds usize")))
}

fn checked_end(
    start: usize,
    len: usize,
    limit: usize,
    label: &str,
) -> Result<usize, QuillTypographyReadError> {
    let end = start
        .checked_add(len)
        .ok_or_else(|| QuillTypographyReadError::new(format!("{label} end overflows usize")))?;
    if end > limit {
        return Err(QuillTypographyReadError::new(format!(
            "{label} exceeds enclosing boundary"
        )));
    }
    Ok(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_point_conversion_is_fail_closed() {
        let exact = QuillExplicitTypographyRun {
            story_index: 0,
            story_syid: QuillSyid(1),
            story_start_utf16: 0,
            story_end_utf16: 1,
            font_index: 0,
            font_name: "Test".to_owned(),
            text_size_emu: 24 * QUILL_TEXT_SIZE_EMU_PER_POINT,
            fdpc_descriptor_ordinal: 0,
            fdpc_style_ordinal: 0,
            fdpc_style_source: RawSpan {
                stream: pub_core::StreamPath("/Quill/QuillSub/CONTENTS".into()),
                offset: 0,
                len: 4,
            },
        };
        assert_eq!(exact.text_size_points_exact(), Some(24));

        let mut non_exact = exact;
        non_exact.text_size_emu += 1;
        assert_eq!(non_exact.text_size_points_exact(), None);
    }

    #[test]
    fn fixed_block_parser_preserves_known_size_value() {
        let bytes = [TEXT_SIZE_ID, 0x20, 0x40, 0xa6, 0x04, 0x00];
        let mut unknown = BTreeSet::new();
        let (block, end) = parse_block(&bytes, 0, bytes.len(), &mut unknown).expect("parse block");
        assert_eq!(block.id, TEXT_SIZE_ID);
        assert_eq!(block.value, Some(304_704));
        assert_eq!(end, bytes.len());
        assert!(unknown.is_empty());
    }
}
