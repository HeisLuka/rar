use crate::{QuillStoryCatalog, QuillStorySlice};
use pub_core::{QuillSyid, RawSpan};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

const FONT: [u8; 4] = *b"FONT";
const FDPC: [u8; 4] = *b"FDPC";
const FDPP: [u8; 4] = *b"FDPP";
const STSH: [u8; 4] = *b"STSH";

const VARIABLE_BLOCK_TYPES: [u8; 8] = [0xC0, 0x80, 0x82, 0x88, 0x8A, 0x90, 0x98, 0xA0];
const GENERAL_CONTAINER: u8 = 0x88;
const FONT_INDEX_CONTAINER_ID: u8 = 0x24;
const TEXT_SIZE_ID: u8 = 0x0C;
const PARAGRAPH_DEFAULT_CHAR_STYLE_ID: u8 = 0x19;

pub const QUILL_TEXT_SIZE_EMU_PER_POINT: u32 = 12_700;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillTypographyCatalog {
    pub font_names: Vec<String>,
    pub ranges: Vec<QuillTypographyRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explicit_runs: Vec<QuillExplicitTypographyRun>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effective_runs: Vec<QuillEffectiveTypographyRun>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unknown_block_types_assumed_zero_length: Vec<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inheritance_unknown_block_types_assumed_zero_length: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_inheritance_unavailable_reason: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuillTypographyValueSource {
    ExplicitFdpc,
    InheritedStsh1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuillParagraphSelectorSource {
    ExplicitFdpp0x19,
    ImplicitStyleZeroFromBoundedEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillEffectiveTypographyRun {
    pub story_index: u32,
    pub story_syid: QuillSyid,
    pub story_start_utf16: u32,
    pub story_end_utf16: u32,
    pub font_index: u32,
    pub font_name: String,
    pub font_source: QuillTypographyValueSource,
    pub text_size_emu: u32,
    pub text_size_source: QuillTypographyValueSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherited_style_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherited_selector_source: Option<QuillParagraphSelectorSource>,
    pub fdpc_descriptor_ordinal: u32,
    pub fdpc_style_ordinal: u32,
    pub fdpc_style_source: RawSpan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fdpp_style_source: Option<RawSpan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stsh_character_default_source: Option<RawSpan>,
}

impl QuillEffectiveTypographyRun {
    pub fn text_size_points_exact(&self) -> Option<u32> {
        (self.text_size_emu % QUILL_TEXT_SIZE_EMU_PER_POINT == 0)
            .then_some(self.text_size_emu / QUILL_TEXT_SIZE_EMU_PER_POINT)
    }

    pub fn uses_inheritance(&self) -> bool {
        self.font_source == QuillTypographyValueSource::InheritedStsh1
            || self.text_size_source == QuillTypographyValueSource::InheritedStsh1
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

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct ParagraphStyleObservation {
    fdpp_descriptor_ordinal: u32,
    fdpp_style_ordinal: u32,
    absolute_text_end: u32,
    text_offset_source: RawSpan,
    style_source: RawSpan,
    default_style_indices: Vec<u32>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct ParagraphTypographyRange {
    global_start_utf16: u32,
    global_end_utf16: u32,
    fdpp_descriptor_ordinal: u32,
    fdpp_style_ordinal: u32,
    style_source: RawSpan,
    selected_style_index: Option<u32>,
    selector_source: Option<QuillParagraphSelectorSource>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct CharacterDefaultObservation {
    logical_style_index: u32,
    stsh_descriptor_ordinal: u32,
    stsh_record_ordinal: u32,
    style_source: RawSpan,
    font_pairs: Vec<(u32, String)>,
    text_sizes_emu: Vec<u32>,
}

fn validate_monotone_fdpc_text_offsets(
    styles: &[StyleObservation],
) -> Result<(), QuillTypographyReadError> {
    for pair in styles.windows(2) {
        if pair[0].absolute_text_end > pair[1].absolute_text_end {
            return Err(QuillTypographyReadError::new(format!(
                "FDPC text offsets regress in stored order: descriptor/style {}/{} ends at 0x{:x}, then {}/{} ends at 0x{:x}",
                pair[0].fdpc_descriptor_ordinal,
                pair[0].fdpc_style_ordinal,
                pair[0].absolute_text_end,
                pair[1].fdpc_descriptor_ordinal,
                pair[1].fdpc_style_ordinal,
                pair[1].absolute_text_end,
            )));
        }
    }
    Ok(())
}

fn validate_monotone_fdpp_text_offsets(
    styles: &[ParagraphStyleObservation],
) -> Result<(), QuillTypographyReadError> {
    for pair in styles.windows(2) {
        if pair[0].absolute_text_end > pair[1].absolute_text_end {
            return Err(QuillTypographyReadError::new(format!(
                "FDPP text offsets regress in stored order: descriptor/style {}/{} ends at 0x{:x}, then {}/{} ends at 0x{:x}",
                pair[0].fdpp_descriptor_ordinal,
                pair[0].fdpp_style_ordinal,
                pair[0].absolute_text_end,
                pair[1].fdpp_descriptor_ordinal,
                pair[1].fdpp_style_ordinal,
                pair[1].absolute_text_end,
            )));
        }
    }
    Ok(())
}

fn explicit_run_projection_allowed(unknown_block_types: &BTreeSet<u8>) -> bool {
    unknown_block_types.is_empty()
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
    let styles = parse_fdpc_styles(
        bytes,
        story_catalog,
        &descriptors,
        &font_names,
        &mut unknown_block_types,
    )?;

    validate_monotone_fdpc_text_offsets(&styles)?;

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
    // parse_block can keep archaeology moving by assuming an unknown fixed
    // block type has zero payload, but that assumption is not semantic
    // authority. It may shift all later block boundaries. Keep the raw/range
    // observations and diagnostic, but suppress product-facing explicit
    // typography until every physical block width in this catalog is known.
    if explicit_run_projection_allowed(&unknown_block_types) {
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
    }

    let mut effective_runs = Vec::new();
    let mut inheritance_unknown_block_types = BTreeSet::new();
    let mut effective_inheritance_unavailable_reason = None;

    let inheritance_result = (|| {
        let paragraph_styles = parse_fdpp_styles(
            bytes,
            story_catalog,
            &descriptors,
            &mut inheritance_unknown_block_types,
        )?;
        validate_monotone_fdpp_text_offsets(&paragraph_styles)?;
        let paragraph_ranges =
            materialize_paragraph_ranges(&paragraph_styles, text_start, text_end, total_utf16)?;
        let character_defaults = parse_stsh1_character_defaults(
            bytes,
            story_catalog,
            &descriptors,
            &font_names,
            &mut inheritance_unknown_block_types,
        )?;

        if explicit_run_projection_allowed(&unknown_block_types)
            && explicit_run_projection_allowed(&inheritance_unknown_block_types)
        {
            effective_runs = build_effective_runs(
                &ranges,
                &paragraph_ranges,
                &character_defaults,
                &story_extents,
            )?;
        }
        Ok::<(), QuillTypographyReadError>(())
    })();

    if let Err(error) = inheritance_result {
        effective_inheritance_unavailable_reason = Some(error.to_string());
    } else if !inheritance_unknown_block_types.is_empty() {
        effective_inheritance_unavailable_reason = Some(format!(
            "effective typography inheritance suppressed because unknown fixed Quill block widths were observed: {:?}",
            inheritance_unknown_block_types
        ));
    }

    Ok(QuillTypographyCatalog {
        font_names,
        ranges,
        explicit_runs,
        effective_runs,
        unknown_block_types_assumed_zero_length: unknown_block_types.into_iter().collect(),
        inheritance_unknown_block_types_assumed_zero_length: inheritance_unknown_block_types
            .into_iter()
            .collect(),
        effective_inheritance_unavailable_reason,
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

fn parse_fdpp_styles(
    bytes: &[u8],
    story_catalog: &QuillStoryCatalog,
    descriptors: &[(usize, &crate::QuillChunkDescriptor)],
    unknown_block_types: &mut BTreeSet<u8>,
) -> Result<Vec<ParagraphStyleObservation>, QuillTypographyReadError> {
    let stream = story_catalog.text.source.stream.clone();
    let mut styles = Vec::new();

    for (descriptor_ordinal, descriptor) in descriptors
        .iter()
        .copied()
        .filter(|(_, descriptor)| descriptor.name.value == FDPP)
    {
        let start = to_usize(descriptor.data_offset.value, "FDPP offset")?;
        let len = to_usize(descriptor.data_length.value, "FDPP length")?;
        let end = checked_end(start, len, bytes.len(), "FDPP chunk")?;
        if start + 8 > end {
            return Err(QuillTypographyReadError::new(
                "FDPP chunk is shorter than fixed prefix",
            ));
        }

        let count = usize::from(read_u16(bytes, start, end)?);
        let offsets_start = start + 8;
        let chunk_offsets_start = offsets_start
            .checked_add(count.checked_mul(4).ok_or_else(|| {
                QuillTypographyReadError::new("FDPP text offset table overflows usize")
            })?)
            .ok_or_else(|| QuillTypographyReadError::new("FDPP text offset table end overflows"))?;
        let body_start = chunk_offsets_start
            .checked_add(count.checked_mul(2).ok_or_else(|| {
                QuillTypographyReadError::new("FDPP style offset table overflows usize")
            })?)
            .ok_or_else(|| {
                QuillTypographyReadError::new("FDPP style offset table end overflows")
            })?;
        if body_start > end {
            return Err(QuillTypographyReadError::new("FDPP tables exceed chunk"));
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
                .ok_or_else(|| QuillTypographyReadError::new("FDPP style offset overflows"))?;
            if style_start < body_start || style_start.saturating_add(4) > end {
                return Err(QuillTypographyReadError::new(
                    "FDPP style offset points outside style body",
                ));
            }
            let style_len = to_usize(read_u32(bytes, style_start, end)?, "FDPP style length")?;
            if style_len < 4 {
                return Err(QuillTypographyReadError::new(
                    "FDPP style length is smaller than header",
                ));
            }
            let style_end = checked_end(style_start, style_len, end, "FDPP style")?;
            let mut cursor = style_start + 4;
            let mut selectors = Vec::new();

            while cursor < style_end {
                let (block, next) = parse_block(bytes, cursor, style_end, unknown_block_types)?;
                if block.id == PARAGRAPH_DEFAULT_CHAR_STYLE_ID {
                    if let Some(value) = block.value {
                        selectors.push(value);
                    }
                }
                cursor = next;
            }
            if cursor != style_end {
                return Err(QuillTypographyReadError::new(
                    "FDPP style did not close exactly",
                ));
            }

            selectors.sort_unstable();
            selectors.dedup();
            styles.push(ParagraphStyleObservation {
                fdpp_descriptor_ordinal: u32::try_from(descriptor_ordinal)
                    .map_err(|_| QuillTypographyReadError::new("descriptor ordinal exceeds u32"))?,
                fdpp_style_ordinal: u32::try_from(style_ordinal)
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
                default_style_indices: selectors,
            });
        }
    }

    if styles.is_empty() {
        return Err(QuillTypographyReadError::new("no FDPP styles"));
    }
    Ok(styles)
}

fn materialize_paragraph_ranges(
    styles: &[ParagraphStyleObservation],
    text_start: u32,
    text_end: u32,
    total_utf16: u32,
) -> Result<Vec<ParagraphTypographyRange>, QuillTypographyReadError> {
    let mut previous_end_utf16 = 0_u32;
    let mut ranges = Vec::new();

    for style in styles {
        if style.absolute_text_end < text_start || style.absolute_text_end > text_end {
            return Err(QuillTypographyReadError::new(format!(
                "FDPP text offset 0x{:x} is outside TEXT [0x{text_start:x}, 0x{text_end:x}]",
                style.absolute_text_end
            )));
        }
        let byte_delta = style.absolute_text_end - text_start;
        if byte_delta % 2 != 0 {
            return Err(QuillTypographyReadError::new(format!(
                "FDPP text offset 0x{:x} is not aligned to UTF-16LE code units",
                style.absolute_text_end
            )));
        }
        let global_end_utf16 = byte_delta / 2;
        if global_end_utf16 < previous_end_utf16 {
            return Err(QuillTypographyReadError::new(format!(
                "FDPP range end regressed from {previous_end_utf16} to {global_end_utf16}"
            )));
        }
        if global_end_utf16 == previous_end_utf16 {
            continue;
        }

        let (selected_style_index, selector_source) = match style.default_style_indices.as_slice() {
            [] => (None, None),
            [value] => (
                Some(*value),
                Some(QuillParagraphSelectorSource::ExplicitFdpp0x19),
            ),
            _ => (None, None),
        };

        ranges.push(ParagraphTypographyRange {
            global_start_utf16: previous_end_utf16,
            global_end_utf16,
            fdpp_descriptor_ordinal: style.fdpp_descriptor_ordinal,
            fdpp_style_ordinal: style.fdpp_style_ordinal,
            style_source: style.style_source.clone(),
            selected_style_index,
            selector_source,
        });
        previous_end_utf16 = global_end_utf16;
    }

    if previous_end_utf16 != total_utf16 {
        return Err(QuillTypographyReadError::new(format!(
            "FDPP terminal UTF-16 boundary {previous_end_utf16} does not close Story corpus at {total_utf16}"
        )));
    }
    Ok(ranges)
}

fn parse_stsh1_character_defaults(
    bytes: &[u8],
    story_catalog: &QuillStoryCatalog,
    descriptors: &[(usize, &crate::QuillChunkDescriptor)],
    font_names: &[String],
    unknown_block_types: &mut BTreeSet<u8>,
) -> Result<Vec<CharacterDefaultObservation>, QuillTypographyReadError> {
    let stsh = descriptors
        .iter()
        .copied()
        .filter(|(_, descriptor)| descriptor.name.value == STSH)
        .collect::<Vec<_>>();
    if stsh.len() < 2 {
        return Err(QuillTypographyReadError::new(format!(
            "expected second STSH descriptor, got {}",
            stsh.len()
        )));
    }
    let (descriptor_ordinal, descriptor) = stsh[1];
    let start = to_usize(descriptor.data_offset.value, "STSH1 offset")?;
    let len = to_usize(descriptor.data_length.value, "STSH1 length")?;
    let end = checked_end(start, len, bytes.len(), "STSH1 chunk")?;
    if start + 20 > end {
        return Err(QuillTypographyReadError::new(
            "STSH1 chunk is shorter than fixed prefix",
        ));
    }

    let count = to_usize(read_u32(bytes, start + 4, end)?, "STSH1 record count")?;
    if count % 2 != 0 {
        return Err(QuillTypographyReadError::new(format!(
            "STSH1 paired character/paragraph record count is odd: {count}"
        )));
    }
    let offsets_start = start + 20;
    let offsets_end =
        offsets_start
            .checked_add(count.checked_mul(4).ok_or_else(|| {
                QuillTypographyReadError::new("STSH1 offset table overflows usize")
            })?)
            .ok_or_else(|| QuillTypographyReadError::new("STSH1 offset table end overflows"))?;
    if offsets_end > end {
        return Err(QuillTypographyReadError::new(
            "STSH1 offset table exceeds chunk",
        ));
    }

    let mut offsets = Vec::with_capacity(count);
    for ordinal in 0..count {
        offsets.push(to_usize(
            read_u32(bytes, offsets_start + ordinal * 4, end)?,
            "STSH1 record offset",
        )?);
    }
    if offsets.windows(2).any(|pair| pair[0] > pair[1]) {
        return Err(QuillTypographyReadError::new(
            "STSH1 offsets regress in stored order",
        ));
    }

    let stream = story_catalog.text.source.stream.clone();
    let descriptor_ordinal = u32::try_from(descriptor_ordinal)
        .map_err(|_| QuillTypographyReadError::new("descriptor ordinal exceeds u32"))?;
    let mut rows = Vec::new();

    for ordinal in (0..count).step_by(2) {
        let record_start = start
            .checked_add(20)
            .and_then(|value| value.checked_add(offsets[ordinal]))
            .ok_or_else(|| QuillTypographyReadError::new("STSH1 record offset overflows"))?;
        if record_start < offsets_end || record_start.saturating_add(6) > end {
            return Err(QuillTypographyReadError::new(
                "STSH1 character record offset points outside style body",
            ));
        }

        let style_start = record_start + 2;
        let style_len = to_usize(read_u32(bytes, style_start, end)?, "STSH1 style length")?;
        if style_len < 4 {
            return Err(QuillTypographyReadError::new(
                "STSH1 character style length is smaller than header",
            ));
        }
        let style_end = checked_end(style_start, style_len, end, "STSH1 character style")?;
        let mut cursor = style_start + 4;
        let mut font_indices = Vec::new();
        let mut text_sizes_emu = Vec::new();

        while cursor < style_end {
            let (block, next) = parse_block(bytes, cursor, style_end, unknown_block_types)?;
            if block.id == FONT_INDEX_CONTAINER_ID {
                if let Some(index) = extract_primary_font_index(bytes, block, unknown_block_types)?
                {
                    let index_usize = to_usize(index, "STSH1 font index")?;
                    if index_usize >= font_names.len() {
                        return Err(QuillTypographyReadError::new(format!(
                            "STSH1 font index {index} is outside FONT catalog of {} records",
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
                "STSH1 character style did not close exactly",
            ));
        }

        let mut font_pairs = font_indices
            .into_iter()
            .map(|index| (index, font_names[index as usize].clone()))
            .collect::<Vec<_>>();
        font_pairs.sort();
        font_pairs.dedup();
        text_sizes_emu.sort_unstable();
        text_sizes_emu.dedup();

        rows.push(CharacterDefaultObservation {
            logical_style_index: u32::try_from(ordinal / 2)
                .map_err(|_| QuillTypographyReadError::new("logical style index exceeds u32"))?,
            stsh_descriptor_ordinal: descriptor_ordinal,
            stsh_record_ordinal: u32::try_from(ordinal)
                .map_err(|_| QuillTypographyReadError::new("STSH1 record ordinal exceeds u32"))?,
            style_source: RawSpan {
                stream: stream.clone(),
                offset: style_start as u64,
                len: style_len as u64,
            },
            font_pairs,
            text_sizes_emu,
        });
    }

    if rows.is_empty() {
        return Err(QuillTypographyReadError::new(
            "no STSH1 character default rows",
        ));
    }
    Ok(rows)
}

fn build_effective_runs(
    fdpc_ranges: &[QuillTypographyRange],
    paragraph_ranges: &[ParagraphTypographyRange],
    defaults: &[CharacterDefaultObservation],
    story_extents: &[StoryExtent],
) -> Result<Vec<QuillEffectiveTypographyRun>, QuillTypographyReadError> {
    let mut boundaries = BTreeSet::new();
    boundaries.insert(0_u32);
    if let Some(last) = story_extents.last() {
        boundaries.insert(last.global_end_utf16);
    }
    for story in story_extents {
        boundaries.insert(story.global_start_utf16);
        boundaries.insert(story.global_end_utf16);
    }
    for range in fdpc_ranges {
        boundaries.insert(range.global_start_utf16);
        boundaries.insert(range.global_end_utf16);
    }
    for range in paragraph_ranges {
        boundaries.insert(range.global_start_utf16);
        boundaries.insert(range.global_end_utf16);
    }
    let ordered = boundaries.into_iter().collect::<Vec<_>>();

    let mut runs = Vec::new();
    for pair in ordered.windows(2) {
        let [start, end] = pair else {
            continue;
        };
        if start == end {
            continue;
        }

        let story = exactly_one_covering_story(story_extents, *start, *end)?;
        let fdpc = exactly_one_covering_fdpc(fdpc_ranges, *start, *end)?;
        let paragraph = exactly_one_covering_paragraph(paragraph_ranges, *start, *end)?;

        let mut explicit_font_pairs = fdpc
            .font_indices
            .iter()
            .copied()
            .zip(fdpc.font_names.iter().cloned())
            .collect::<Vec<_>>();
        let explicit_font_present = !fdpc.font_indices.is_empty();
        explicit_font_pairs.sort();
        explicit_font_pairs.dedup();

        let mut explicit_sizes = fdpc.text_sizes_emu.clone();
        let explicit_size_present = !explicit_sizes.is_empty();
        explicit_sizes.sort_unstable();
        explicit_sizes.dedup();

        let default = paragraph.selected_style_index.and_then(|style_index| {
            defaults
                .iter()
                .find(|candidate| candidate.logical_style_index == style_index)
        });

        let (font_index, font_name, font_source) =
            if let [(font_index, font_name)] = explicit_font_pairs.as_slice() {
                (
                    *font_index,
                    font_name.clone(),
                    QuillTypographyValueSource::ExplicitFdpc,
                )
            } else if !explicit_font_present {
                let Some(default) = default else {
                    continue;
                };
                let [(font_index, font_name)] = default.font_pairs.as_slice() else {
                    continue;
                };
                (
                    *font_index,
                    font_name.clone(),
                    QuillTypographyValueSource::InheritedStsh1,
                )
            } else {
                continue;
            };

        let (text_size_emu, text_size_source) = if let [text_size_emu] = explicit_sizes.as_slice() {
            (*text_size_emu, QuillTypographyValueSource::ExplicitFdpc)
        } else if !explicit_size_present {
            let Some(default) = default else {
                continue;
            };
            let [text_size_emu] = default.text_sizes_emu.as_slice() else {
                continue;
            };
            (*text_size_emu, QuillTypographyValueSource::InheritedStsh1)
        } else {
            continue;
        };

        if text_size_emu == 0 || font_name.is_empty() {
            continue;
        }

        let uses_inheritance = font_source == QuillTypographyValueSource::InheritedStsh1
            || text_size_source == QuillTypographyValueSource::InheritedStsh1;
        let (
            inherited_style_index,
            inherited_selector_source,
            fdpp_style_source,
            stsh_character_default_source,
        ) = if uses_inheritance {
            let Some(style_index) = paragraph.selected_style_index else {
                continue;
            };
            let Some(selector_source) = paragraph.selector_source else {
                continue;
            };
            let Some(default) = default else {
                continue;
            };
            (
                Some(style_index),
                Some(selector_source),
                Some(paragraph.style_source.clone()),
                Some(default.style_source.clone()),
            )
        } else {
            (None, None, None, None)
        };

        runs.push(QuillEffectiveTypographyRun {
            story_index: story.story_index,
            story_syid: story.story_syid,
            story_start_utf16: *start - story.global_start_utf16,
            story_end_utf16: *end - story.global_start_utf16,
            font_index,
            font_name,
            font_source,
            text_size_emu,
            text_size_source,
            inherited_style_index,
            inherited_selector_source,
            fdpc_descriptor_ordinal: fdpc.fdpc_descriptor_ordinal,
            fdpc_style_ordinal: fdpc.fdpc_style_ordinal,
            fdpc_style_source: fdpc.fdpc_style_source.clone(),
            fdpp_style_source,
            stsh_character_default_source,
        });
    }
    Ok(runs)
}

fn exactly_one_covering_story(
    stories: &[StoryExtent],
    start: u32,
    end: u32,
) -> Result<&StoryExtent, QuillTypographyReadError> {
    let matches = stories
        .iter()
        .filter(|story| story.global_start_utf16 <= start && story.global_end_utf16 >= end)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [story] => Ok(*story),
        _ => Err(QuillTypographyReadError::new(format!(
            "effective typography segment {start}..{end} has {} Story owners",
            matches.len()
        ))),
    }
}

fn exactly_one_covering_fdpc(
    ranges: &[QuillTypographyRange],
    start: u32,
    end: u32,
) -> Result<&QuillTypographyRange, QuillTypographyReadError> {
    let matches = ranges
        .iter()
        .filter(|range| range.global_start_utf16 <= start && range.global_end_utf16 >= end)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [range] => Ok(*range),
        _ => Err(QuillTypographyReadError::new(format!(
            "effective typography segment {start}..{end} has {} FDPC owners",
            matches.len()
        ))),
    }
}

fn exactly_one_covering_paragraph(
    ranges: &[ParagraphTypographyRange],
    start: u32,
    end: u32,
) -> Result<&ParagraphTypographyRange, QuillTypographyReadError> {
    let matches = ranges
        .iter()
        .filter(|range| range.global_start_utf16 <= start && range.global_end_utf16 >= end)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [range] => Ok(*range),
        _ => Err(QuillTypographyReadError::new(format!(
            "effective typography segment {start}..{end} has {} FDPP owners",
            matches.len()
        ))),
    }
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
            if style_start < body_start || style_start.saturating_add(4) > end {
                return Err(QuillTypographyReadError::new(
                    "FDPC style offset points outside style body",
                ));
            }
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

    fn style_observation(end: u32, ordinal: u32) -> StyleObservation {
        StyleObservation {
            fdpc_descriptor_ordinal: 0,
            fdpc_style_ordinal: ordinal,
            absolute_text_end: end,
            text_offset_source: RawSpan {
                stream: pub_core::StreamPath("/Quill/QuillSub/CONTENTS".into()),
                offset: u64::from(ordinal) * 4,
                len: 4,
            },
            style_source: RawSpan {
                stream: pub_core::StreamPath("/Quill/QuillSub/CONTENTS".into()),
                offset: 100 + u64::from(ordinal) * 8,
                len: 8,
            },
            font_indices: Vec::new(),
            font_names: Vec::new(),
            text_sizes_emu: Vec::new(),
        }
    }

    #[test]
    fn fdpc_text_offsets_must_be_monotone_in_stored_order() {
        let monotone = vec![
            style_observation(0x220, 0),
            style_observation(0x240, 1),
            style_observation(0x240, 2),
            style_observation(0x280, 3),
        ];
        validate_monotone_fdpc_text_offsets(&monotone).expect("monotone offsets");

        let regressed = vec![
            style_observation(0x220, 0),
            style_observation(0x280, 1),
            style_observation(0x240, 2),
        ];
        let error = validate_monotone_fdpc_text_offsets(&regressed).unwrap_err();
        assert!(error.to_string().contains("regress in stored order"));
    }

    #[test]
    fn unknown_fixed_block_widths_suppress_explicit_run_projection() {
        let mut unknown = BTreeSet::new();
        assert!(explicit_run_projection_allowed(&unknown));

        unknown.insert(0x33);
        assert!(!explicit_run_projection_allowed(&unknown));
    }

    #[test]
    fn effective_typography_prefers_explicit_property_and_inherits_only_missing_property() {
        let story = StoryExtent {
            story_index: 0,
            story_syid: QuillSyid(7),
            global_start_utf16: 0,
            global_end_utf16: 10,
        };
        let fdpc = QuillTypographyRange {
            global_start_utf16: 0,
            global_end_utf16: 10,
            fdpc_descriptor_ordinal: 1,
            fdpc_style_ordinal: 2,
            fdpc_style_source: RawSpan {
                stream: pub_core::StreamPath("/Quill/QuillSub/CONTENTS".into()),
                offset: 100,
                len: 8,
            },
            text_offset_source: RawSpan {
                stream: pub_core::StreamPath("/Quill/QuillSub/CONTENTS".into()),
                offset: 40,
                len: 4,
            },
            font_indices: vec![3],
            font_names: vec!["Explicit Face".to_owned()],
            text_sizes_emu: Vec::new(),
            story_intersections: Vec::new(),
        };
        let paragraph = ParagraphTypographyRange {
            global_start_utf16: 0,
            global_end_utf16: 10,
            fdpp_descriptor_ordinal: 4,
            fdpp_style_ordinal: 5,
            style_source: RawSpan {
                stream: pub_core::StreamPath("/Quill/QuillSub/CONTENTS".into()),
                offset: 200,
                len: 10,
            },
            selected_style_index: Some(2),
            selector_source: Some(QuillParagraphSelectorSource::ExplicitFdpp0x19),
        };
        let default = CharacterDefaultObservation {
            logical_style_index: 2,
            stsh_descriptor_ordinal: 6,
            stsh_record_ordinal: 4,
            style_source: RawSpan {
                stream: pub_core::StreamPath("/Quill/QuillSub/CONTENTS".into()),
                offset: 300,
                len: 12,
            },
            font_pairs: vec![(9, "Default Face".to_owned())],
            text_sizes_emu: vec![12 * QUILL_TEXT_SIZE_EMU_PER_POINT],
        };

        let runs = build_effective_runs(&[fdpc], &[paragraph], &[default], &[story])
            .expect("effective run");
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert_eq!(run.font_name, "Explicit Face");
        assert_eq!(run.font_source, QuillTypographyValueSource::ExplicitFdpc);
        assert_eq!(run.text_size_emu, 12 * QUILL_TEXT_SIZE_EMU_PER_POINT);
        assert_eq!(
            run.text_size_source,
            QuillTypographyValueSource::InheritedStsh1
        );
        assert_eq!(run.inherited_style_index, Some(2));
        assert_eq!(
            run.inherited_selector_source,
            Some(QuillParagraphSelectorSource::ExplicitFdpp0x19)
        );
        assert!(run.uses_inheritance());
    }

    #[test]
    fn ambiguous_explicit_property_is_not_treated_as_missing() {
        let story = StoryExtent {
            story_index: 0,
            story_syid: QuillSyid(7),
            global_start_utf16: 0,
            global_end_utf16: 10,
        };
        let fdpc = QuillTypographyRange {
            global_start_utf16: 0,
            global_end_utf16: 10,
            fdpc_descriptor_ordinal: 1,
            fdpc_style_ordinal: 2,
            fdpc_style_source: RawSpan {
                stream: pub_core::StreamPath("/Quill/QuillSub/CONTENTS".into()),
                offset: 100,
                len: 8,
            },
            text_offset_source: RawSpan {
                stream: pub_core::StreamPath("/Quill/QuillSub/CONTENTS".into()),
                offset: 40,
                len: 4,
            },
            font_indices: vec![1, 2],
            font_names: vec!["A".to_owned(), "B".to_owned()],
            text_sizes_emu: vec![14 * QUILL_TEXT_SIZE_EMU_PER_POINT],
            story_intersections: Vec::new(),
        };
        let paragraph = ParagraphTypographyRange {
            global_start_utf16: 0,
            global_end_utf16: 10,
            fdpp_descriptor_ordinal: 4,
            fdpp_style_ordinal: 5,
            style_source: RawSpan {
                stream: pub_core::StreamPath("/Quill/QuillSub/CONTENTS".into()),
                offset: 200,
                len: 10,
            },
            selected_style_index: Some(0),
            selector_source: Some(
                QuillParagraphSelectorSource::ImplicitStyleZeroFromBoundedEvidence,
            ),
        };
        let default = CharacterDefaultObservation {
            logical_style_index: 0,
            stsh_descriptor_ordinal: 6,
            stsh_record_ordinal: 0,
            style_source: RawSpan {
                stream: pub_core::StreamPath("/Quill/QuillSub/CONTENTS".into()),
                offset: 300,
                len: 12,
            },
            font_pairs: vec![(9, "Default".to_owned())],
            text_sizes_emu: vec![10 * QUILL_TEXT_SIZE_EMU_PER_POINT],
        };

        let runs = build_effective_runs(&[fdpc], &[paragraph], &[default], &[story])
            .expect("bounded segmentation");
        assert!(runs.is_empty());
    }

    #[test]
    fn paragraph_ranges_promote_only_explicit_selector_until_style_zero_is_reproduced() {
        let stream = pub_core::StreamPath("/Quill/QuillSub/CONTENTS".into());
        let styles = vec![
            ParagraphStyleObservation {
                fdpp_descriptor_ordinal: 1,
                fdpp_style_ordinal: 0,
                absolute_text_end: 120,
                text_offset_source: RawSpan {
                    stream: stream.clone(),
                    offset: 8,
                    len: 4,
                },
                style_source: RawSpan {
                    stream: stream.clone(),
                    offset: 40,
                    len: 8,
                },
                default_style_indices: vec![3],
            },
            ParagraphStyleObservation {
                fdpp_descriptor_ordinal: 1,
                fdpp_style_ordinal: 1,
                absolute_text_end: 140,
                text_offset_source: RawSpan {
                    stream: stream.clone(),
                    offset: 12,
                    len: 4,
                },
                style_source: RawSpan {
                    stream,
                    offset: 48,
                    len: 8,
                },
                default_style_indices: Vec::new(),
            },
        ];
        let ranges = materialize_paragraph_ranges(&styles, 100, 140, 20).expect("paragraph ranges");
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].selected_style_index, Some(3));
        assert_eq!(
            ranges[0].selector_source,
            Some(QuillParagraphSelectorSource::ExplicitFdpp0x19)
        );
        assert_eq!(ranges[1].selected_style_index, None);
        assert_eq!(ranges[1].selector_source, None);
    }

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
