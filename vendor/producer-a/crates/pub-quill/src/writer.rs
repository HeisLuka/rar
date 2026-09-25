use crate::{
    QuillChunkDescriptor, QuillStoryCatalog, QuillStoryReadError, parse_confirmed_story_catalog,
};
use pub_core::{QuillSyid, RawSpan, StreamPath};
use std::collections::BTreeMap;
use std::fmt;

const TEXT: [u8; 4] = *b"TEXT";
const FDPP: [u8; 4] = *b"FDPP";
const FDPC: [u8; 4] = *b"FDPC";
const BTEP: [u8; 4] = *b"BTEP";
const BTEC: [u8; 4] = *b"BTEC";
const TOKN: [u8; 4] = *b"TOKN";
const SERVICE_PAGE_ALIGNMENT: u32 = 0x200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuillStoryTextEdit {
    pub story_syid: QuillSyid,
    pub start_utf16: u32,
    pub delete_utf16: u32,
    pub replacement: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuillStoryTextWritePlan {
    pub output_stream: Vec<u8>,
    pub story_syid: QuillSyid,
    pub delta_utf16: i64,
    pub delta_bytes: i64,
    pub first_aligned_service_anchor: u32,
    pub moved_descriptor_count: usize,
    pub boundary_patch_count: usize,
    pub bte_position_patch_count: usize,
    pub bte_reference_patch_count: usize,
    pub syid_header_before: u32,
    pub syid_header_after: u32,
    pub changed_ranges: Vec<RawSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuillStoryWriteError {
    Read(QuillStoryReadError),
    StoryNotFound(u32),
    DuplicateStoryIdentity(u32),
    TargetIsTableStory(u32),
    TargetHasTokenLayer(u32),
    EditOutOfBounds,
    NonBmpSourceStory,
    NonBmpReplacement,
    ParagraphMarkMutation,
    LengthPreservingEditOutOfScope,
    MissingBoundaryChunk([u8; 4]),
    MalformedBoundaryChunk([u8; 4]),
    BoundaryOutsideText([u8; 4], u32),
    InsertionAtExistingFdBoundary,
    EditTouchesExistingFdBoundary(u32),
    EditCrossesExistingFdBoundary(u32),
    InsertionAtExistingBteBoundary(u32),
    EditTouchesExistingBteBoundary(u32),
    EditCrossesExistingBteBoundary(u32),
    MissingAlignedServiceAnchor,
    InsufficientCapacity,
    PositiveGrowthWouldDiscardNonZeroUnknownAnchorBytes,
    PositiveGrowthWouldDiscardDescriptorPayload([u8; 4]),
    DescriptorMetadataInsideRelocationWindow,
    UnsupportedBteDataSize([u8; 4], u32),
    MalformedBteChunk([u8; 4]),
    SyidHeaderOverflow,
    IntegerOverflow,
    OutputValidationFailed(String),
}

impl fmt::Display for QuillStoryWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => write!(f, "{error}"),
            Self::StoryNotFound(syid) => write!(f, "Quill Story SYID {syid} was not found"),
            Self::DuplicateStoryIdentity(syid) => write!(f, "Quill Story SYID {syid} is ambiguous"),
            Self::TargetIsTableStory(syid) => write!(f, "Quill Story SYID {syid} is table-owned"),
            Self::TargetHasTokenLayer(syid) => {
                write!(f, "Quill Story SYID {syid} has a TOKN layer")
            }
            Self::EditOutOfBounds => write!(f, "Quill edit is out of bounds"),
            Self::NonBmpSourceStory => {
                write!(f, "ordinary Quill writer requires a BMP source story")
            }
            Self::NonBmpReplacement => {
                write!(f, "ordinary Quill writer requires BMP replacement text")
            }
            Self::ParagraphMarkMutation => write!(
                f,
                "paragraph-mark mutation is outside the ordinary-run scope"
            ),
            Self::LengthPreservingEditOutOfScope => write!(
                f,
                "length-preserving edit is outside the variable-length Quill writer scope"
            ),
            Self::MissingBoundaryChunk(name) => write!(
                f,
                "required {} boundary chunk is missing",
                name_string(*name)
            ),
            Self::MalformedBoundaryChunk(name) => {
                write!(f, "malformed {} boundary chunk", name_string(*name))
            }
            Self::BoundaryOutsideText(name, boundary) => write!(
                f,
                "{} boundary {boundary:#x} is outside TEXT",
                name_string(*name)
            ),
            Self::InsertionAtExistingFdBoundary => write!(
                f,
                "insertion at an existing FD boundary is outside the ordinary-run scope"
            ),
            Self::EditTouchesExistingFdBoundary(boundary) => {
                write!(f, "edit starts at existing FD boundary {boundary:#x}")
            }
            Self::EditCrossesExistingFdBoundary(boundary) => {
                write!(f, "edit crosses existing FD boundary {boundary:#x}")
            }
            Self::InsertionAtExistingBteBoundary(boundary) => write!(
                f,
                "insertion at existing BTE partition boundary {boundary:#x} is outside the ordinary-run scope"
            ),
            Self::EditTouchesExistingBteBoundary(boundary) => write!(
                f,
                "edit starts at existing BTE partition boundary {boundary:#x}"
            ),
            Self::EditCrossesExistingBteBoundary(boundary) => write!(
                f,
                "edit crosses existing BTE partition boundary {boundary:#x}"
            ),
            Self::MissingAlignedServiceAnchor => {
                write!(f, "no aligned FDPP/FDPC service anchor follows TEXT")
            }
            Self::InsufficientCapacity => write!(f, "insufficient Quill pre-anchor capacity"),
            Self::PositiveGrowthWouldDiscardNonZeroUnknownAnchorBytes => write!(
                f,
                "positive text growth would discard non-zero unknown anchor bytes"
            ),
            Self::PositiveGrowthWouldDiscardDescriptorPayload(name) => write!(
                f,
                "positive text growth would discard {} payload",
                name_string(*name)
            ),
            Self::DescriptorMetadataInsideRelocationWindow => write!(
                f,
                "descriptor metadata inside relocation window is unsupported"
            ),
            Self::UnsupportedBteDataSize(name, found) => write!(
                f,
                "{} uses unsupported BTE PLC dataSize {found}",
                name_string(*name)
            ),
            Self::MalformedBteChunk(name) => {
                write!(f, "malformed {} Type-4 PLC", name_string(*name))
            }
            Self::SyidHeaderOverflow => write!(f, "SYID lifecycle scalar overflow"),
            Self::IntegerOverflow => write!(f, "integer overflow while planning Quill edit"),
            Self::OutputValidationFailed(reason) => {
                write!(f, "generated Quill stream failed validation: {reason}")
            }
        }
    }
}

impl std::error::Error for QuillStoryWriteError {}

impl From<QuillStoryReadError> for QuillStoryWriteError {
    fn from(value: QuillStoryReadError) -> Self {
        Self::Read(value)
    }
}

#[derive(Debug)]
struct OutputValidationEdit<'a> {
    target_syid: QuillSyid,
    source_units: &'a [u16],
    start: usize,
    delete_end: usize,
    replacement_units: &'a [u16],
}

#[derive(Debug, Clone, Copy)]
struct BoundaryLocation {
    name: [u8; 4],
    field_offset: usize,
    value: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BteWordKind {
    Position,
    Target,
}

#[derive(Debug, Clone, Copy)]
struct BteWord {
    kind: BteWordKind,
    field_offset: usize,
    value: u32,
}

pub fn plan_quill_story_text_edit(
    stream: StreamPath,
    bytes: &[u8],
    edit: &QuillStoryTextEdit,
) -> Result<QuillStoryTextWritePlan, QuillStoryWriteError> {
    let catalog = parse_confirmed_story_catalog(stream.clone(), bytes)?;
    let descriptors = all_descriptors(&catalog);

    let mut stories = catalog
        .stories
        .iter()
        .filter(|story| story.syid == edit.story_syid);
    let story = stories
        .next()
        .ok_or(QuillStoryWriteError::StoryNotFound(edit.story_syid.0))?;
    if stories.next().is_some() {
        return Err(QuillStoryWriteError::DuplicateStoryIdentity(
            edit.story_syid.0,
        ));
    }
    if catalog
        .tcd
        .iter()
        .any(|chunk| chunk.story_syid.value == edit.story_syid)
    {
        return Err(QuillStoryWriteError::TargetIsTableStory(edit.story_syid.0));
    }
    let ordinal = u16::try_from(story.index).ok();
    if descriptors.iter().any(|descriptor| {
        descriptor.name.value == TOKN
            && ordinal.is_some_and(|value| descriptor.option_a.value == value)
    }) {
        return Err(QuillStoryWriteError::TargetHasTokenLayer(edit.story_syid.0));
    }

    let source_units = utf16_units(&story.utf16le)?;
    if source_units
        .iter()
        .any(|unit| (0xd800..=0xdfff).contains(unit))
    {
        return Err(QuillStoryWriteError::NonBmpSourceStory);
    }
    let replacement_units = edit.replacement.encode_utf16().collect::<Vec<_>>();
    if replacement_units
        .iter()
        .any(|unit| (0xd800..=0xdfff).contains(unit))
    {
        return Err(QuillStoryWriteError::NonBmpReplacement);
    }

    let start =
        usize::try_from(edit.start_utf16).map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
    let delete =
        usize::try_from(edit.delete_utf16).map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
    let delete_end = start
        .checked_add(delete)
        .ok_or(QuillStoryWriteError::IntegerOverflow)?;
    if delete_end > source_units.len() {
        return Err(QuillStoryWriteError::EditOutOfBounds);
    }
    if source_units[start..delete_end].contains(&0x000d) || replacement_units.contains(&0x000d) {
        return Err(QuillStoryWriteError::ParagraphMarkMutation);
    }

    let delta_utf16 = i64::try_from(replacement_units.len())
        .map_err(|_| QuillStoryWriteError::IntegerOverflow)?
        .checked_sub(i64::from(edit.delete_utf16))
        .ok_or(QuillStoryWriteError::IntegerOverflow)?;
    if delta_utf16 == 0 {
        return Err(QuillStoryWriteError::LengthPreservingEditOutOfScope);
    }
    let delta_bytes = delta_utf16
        .checked_mul(2)
        .ok_or(QuillStoryWriteError::IntegerOverflow)?;

    let text_descriptor = unique_descriptor(&descriptors, TEXT)?;
    let text_start = usize::try_from(text_descriptor.data_offset.value)
        .map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
    let old_text_len = usize::try_from(text_descriptor.data_length.value)
        .map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
    let old_text_end = text_start
        .checked_add(old_text_len)
        .ok_or(QuillStoryWriteError::IntegerOverflow)?;
    if old_text_end > bytes.len() {
        return Err(QuillStoryWriteError::OutputValidationFailed(
            "TEXT extends beyond stream".into(),
        ));
    }

    let story_start = usize::try_from(story.text_source.offset)
        .map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
    let story_end = usize::try_from(
        story
            .text_source
            .end()
            .ok_or(QuillStoryWriteError::IntegerOverflow)?,
    )
    .map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
    if story_start < text_start || story_end > old_text_end {
        return Err(QuillStoryWriteError::OutputValidationFailed(
            "Story span lies outside TEXT".into(),
        ));
    }

    let edit_start = story_start
        .checked_add(
            start
                .checked_mul(2)
                .ok_or(QuillStoryWriteError::IntegerOverflow)?,
        )
        .ok_or(QuillStoryWriteError::IntegerOverflow)?;
    let edit_end = edit_start
        .checked_add(
            delete
                .checked_mul(2)
                .ok_or(QuillStoryWriteError::IntegerOverflow)?,
        )
        .ok_or(QuillStoryWriteError::IntegerOverflow)?;
    let boundaries = collect_fd_boundaries(bytes, &descriptors, text_start, old_text_end)?;
    if !boundaries.iter().any(|boundary| boundary.name == FDPP) {
        return Err(QuillStoryWriteError::MissingBoundaryChunk(FDPP));
    }
    if !boundaries.iter().any(|boundary| boundary.name == FDPC) {
        return Err(QuillStoryWriteError::MissingBoundaryChunk(FDPC));
    }

    let edit_start_u32 =
        u32::try_from(edit_start).map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
    let edit_end_u32 =
        u32::try_from(edit_end).map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
    for boundary in &boundaries {
        if edit.delete_utf16 == 0 && boundary.value == edit_start_u32 {
            return Err(QuillStoryWriteError::InsertionAtExistingFdBoundary);
        }
        if edit.delete_utf16 != 0 && boundary.value == edit_start_u32 {
            return Err(QuillStoryWriteError::EditTouchesExistingFdBoundary(
                boundary.value,
            ));
        }
        if boundary.value > edit_start_u32 && boundary.value < edit_end_u32 {
            return Err(QuillStoryWriteError::EditCrossesExistingFdBoundary(
                boundary.value,
            ));
        }
    }

    let bte_words = collect_bte_words(bytes, &descriptors, text_start, old_text_end)?;
    for word in &bte_words {
        if word.kind != BteWordKind::Position || word.value == 0 {
            continue;
        }
        if edit.delete_utf16 == 0 && word.value == edit_start_u32 {
            return Err(QuillStoryWriteError::InsertionAtExistingBteBoundary(
                word.value,
            ));
        }
        if edit.delete_utf16 != 0 && word.value == edit_start_u32 {
            return Err(QuillStoryWriteError::EditTouchesExistingBteBoundary(
                word.value,
            ));
        }
        if word.value > edit_start_u32 && word.value < edit_end_u32 {
            return Err(QuillStoryWriteError::EditCrossesExistingBteBoundary(
                word.value,
            ));
        }
    }

    let mut replacement_bytes = Vec::with_capacity(replacement_units.len() * 2);
    for unit in &replacement_units {
        replacement_bytes.extend_from_slice(&unit.to_le_bytes());
    }
    let rel_start = edit_start
        .checked_sub(text_start)
        .ok_or(QuillStoryWriteError::IntegerOverflow)?;
    let rel_end = edit_end
        .checked_sub(text_start)
        .ok_or(QuillStoryWriteError::IntegerOverflow)?;
    let old_text = &bytes[text_start..old_text_end];
    let new_text_capacity = i64::try_from(old_text_len)
        .map_err(|_| QuillStoryWriteError::IntegerOverflow)?
        .checked_add(delta_bytes)
        .ok_or(QuillStoryWriteError::IntegerOverflow)?;
    let mut new_text = Vec::with_capacity(
        usize::try_from(new_text_capacity).map_err(|_| QuillStoryWriteError::IntegerOverflow)?,
    );
    new_text.extend_from_slice(&old_text[..rel_start]);
    new_text.extend_from_slice(&replacement_bytes);
    new_text.extend_from_slice(&old_text[rel_end..]);

    let anchor = first_aligned_service_anchor(&descriptors, old_text_end)?
        .ok_or(QuillStoryWriteError::MissingAlignedServiceAnchor)?;
    let anchor_usize =
        usize::try_from(anchor).map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
    ensure_descriptor_metadata_is_fixed(&catalog, old_text_end, anchor_usize)?;

    let new_text_end_i64 = i64::try_from(old_text_end)
        .map_err(|_| QuillStoryWriteError::IntegerOverflow)?
        .checked_add(delta_bytes)
        .ok_or(QuillStoryWriteError::IntegerOverflow)?;
    let new_text_end =
        usize::try_from(new_text_end_i64).map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
    let mut output = bytes.to_vec();

    if delta_bytes > 0 {
        let growth =
            usize::try_from(delta_bytes).map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
        if new_text_end > anchor_usize {
            return Err(QuillStoryWriteError::InsufficientCapacity);
        }
        let discard_start = anchor_usize
            .checked_sub(growth)
            .ok_or(QuillStoryWriteError::IntegerOverflow)?;
        if discard_start < old_text_end {
            return Err(QuillStoryWriteError::InsufficientCapacity);
        }
        for descriptor in &descriptors {
            let offset = usize::try_from(descriptor.data_offset.value)
                .map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
            let len = usize::try_from(descriptor.data_length.value)
                .map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
            let end = offset
                .checked_add(len)
                .ok_or(QuillStoryWriteError::IntegerOverflow)?;
            if offset < anchor_usize && end > discard_start && descriptor.name.value != TEXT {
                let overlap = &bytes[offset.max(discard_start)..end.min(anchor_usize)];
                if overlap.iter().any(|byte| *byte != 0) {
                    return Err(
                        QuillStoryWriteError::PositiveGrowthWouldDiscardNonZeroUnknownAnchorBytes,
                    );
                }
                return Err(
                    QuillStoryWriteError::PositiveGrowthWouldDiscardDescriptorPayload(
                        descriptor.name.value,
                    ),
                );
            }
        }
        if bytes[discard_start..anchor_usize]
            .iter()
            .any(|byte| *byte != 0)
        {
            return Err(QuillStoryWriteError::PositiveGrowthWouldDiscardNonZeroUnknownAnchorBytes);
        }
        output.copy_within(old_text_end..discard_start, new_text_end);
    } else {
        let shrink =
            usize::try_from(-delta_bytes).map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
        output.copy_within(old_text_end..anchor_usize, new_text_end);
        output[anchor_usize - shrink..anchor_usize].fill(0);
    }
    output[text_start..new_text_end].copy_from_slice(&new_text);

    let mut moved_offsets = BTreeMap::new();
    let mut moved_descriptor_count = 0usize;
    for descriptor in &descriptors {
        let old_offset = descriptor.data_offset.value;
        let old_offset_usize =
            usize::try_from(old_offset).map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
        if old_offset_usize >= old_text_end && old_offset_usize < anchor_usize {
            let new_offset = u32::try_from(
                i64::from(old_offset)
                    .checked_add(delta_bytes)
                    .ok_or(QuillStoryWriteError::IntegerOverflow)?,
            )
            .map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
            moved_offsets.insert(old_offset, new_offset);
            patch_u32(
                &mut output,
                usize::try_from(descriptor.data_offset.source.offset)
                    .map_err(|_| QuillStoryWriteError::IntegerOverflow)?,
                new_offset,
            )?;
            moved_descriptor_count += 1;
        }
    }
    patch_u32(
        &mut output,
        usize::try_from(text_descriptor.data_length.source.offset)
            .map_err(|_| QuillStoryWriteError::IntegerOverflow)?,
        u32::try_from(new_text.len()).map_err(|_| QuillStoryWriteError::IntegerOverflow)?,
    )?;

    let new_story_len = u32::try_from(
        i64::try_from(source_units.len())
            .map_err(|_| QuillStoryWriteError::IntegerOverflow)?
            .checked_add(delta_utf16)
            .ok_or(QuillStoryWriteError::IntegerOverflow)?,
    )
    .map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
    patch_u32(
        &mut output,
        relocated_field_offset(
            usize::try_from(story.length_source.offset)
                .map_err(|_| QuillStoryWriteError::IntegerOverflow)?,
            old_text_end,
            anchor_usize,
            delta_bytes,
        )?,
        new_story_len,
    )?;

    let mut boundary_patch_count = 0usize;
    for boundary in &boundaries {
        let shift = if edit.delete_utf16 == 0 {
            boundary.value > edit_start_u32
        } else {
            boundary.value >= edit_end_u32
        };
        if shift {
            let value = u32::try_from(
                i64::from(boundary.value)
                    .checked_add(delta_bytes)
                    .ok_or(QuillStoryWriteError::IntegerOverflow)?,
            )
            .map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
            patch_u32(
                &mut output,
                relocated_field_offset(
                    boundary.field_offset,
                    old_text_end,
                    anchor_usize,
                    delta_bytes,
                )?,
                value,
            )?;
            boundary_patch_count += 1;
        }
    }

    let mut bte_position_patch_count = 0usize;
    let mut bte_reference_patch_count = 0usize;
    for word in &bte_words {
        match word.kind {
            BteWordKind::Position if word.value != 0 => {
                let shift = if edit.delete_utf16 == 0 {
                    word.value > edit_start_u32
                } else {
                    word.value >= edit_end_u32
                };
                if shift {
                    let value = u32::try_from(
                        i64::from(word.value)
                            .checked_add(delta_bytes)
                            .ok_or(QuillStoryWriteError::IntegerOverflow)?,
                    )
                    .map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
                    patch_u32(
                        &mut output,
                        relocated_field_offset(
                            word.field_offset,
                            old_text_end,
                            anchor_usize,
                            delta_bytes,
                        )?,
                        value,
                    )?;
                    bte_position_patch_count += 1;
                }
            }
            BteWordKind::Target => {
                if let Some(value) = moved_offsets.get(&word.value) {
                    patch_u32(
                        &mut output,
                        relocated_field_offset(
                            word.field_offset,
                            old_text_end,
                            anchor_usize,
                            delta_bytes,
                        )?,
                        *value,
                    )?;
                    bte_reference_patch_count += 1;
                }
            }
            BteWordKind::Position => {}
        }
    }

    let syid_header_before = catalog.syid.header.value;
    let syid_header_after = syid_header_before
        .checked_add(1)
        .ok_or(QuillStoryWriteError::SyidHeaderOverflow)?;
    patch_u32(
        &mut output,
        relocated_field_offset(
            usize::try_from(catalog.syid.header.source.offset)
                .map_err(|_| QuillStoryWriteError::IntegerOverflow)?,
            old_text_end,
            anchor_usize,
            delta_bytes,
        )?,
        syid_header_after,
    )?;

    validate_output(
        stream.clone(),
        &catalog,
        &output,
        OutputValidationEdit {
            target_syid: edit.story_syid,
            source_units: &source_units,
            start,
            delete_end,
            replacement_units: &replacement_units,
        },
    )?;

    Ok(QuillStoryTextWritePlan {
        changed_ranges: changed_ranges(stream, bytes, &output),
        output_stream: output,
        story_syid: edit.story_syid,
        delta_utf16,
        delta_bytes,
        first_aligned_service_anchor: anchor,
        moved_descriptor_count,
        boundary_patch_count,
        bte_position_patch_count,
        bte_reference_patch_count,
        syid_header_before,
        syid_header_after,
    })
}

fn all_descriptors(catalog: &QuillStoryCatalog) -> Vec<&QuillChunkDescriptor> {
    catalog
        .descriptor_nodes
        .iter()
        .flat_map(|node| node.descriptors.iter())
        .collect()
}

fn unique_descriptor<'a>(
    descriptors: &[&'a QuillChunkDescriptor],
    name: [u8; 4],
) -> Result<&'a QuillChunkDescriptor, QuillStoryWriteError> {
    let mut found = descriptors
        .iter()
        .copied()
        .filter(|descriptor| descriptor.name.value == name);
    let first = found.next().ok_or_else(|| {
        QuillStoryWriteError::OutputValidationFailed(format!("missing {}", name_string(name)))
    })?;
    if found.next().is_some() {
        return Err(QuillStoryWriteError::OutputValidationFailed(format!(
            "duplicate {}",
            name_string(name)
        )));
    }
    Ok(first)
}

fn first_aligned_service_anchor(
    descriptors: &[&QuillChunkDescriptor],
    old_text_end: usize,
) -> Result<Option<u32>, QuillStoryWriteError> {
    let old_end = u32::try_from(old_text_end).map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
    Ok(descriptors
        .iter()
        .copied()
        .filter(|descriptor| descriptor.name.value == FDPP || descriptor.name.value == FDPC)
        .map(|descriptor| descriptor.data_offset.value)
        .filter(|offset| *offset >= old_end && *offset % SERVICE_PAGE_ALIGNMENT == 0)
        .min())
}

fn ensure_descriptor_metadata_is_fixed(
    catalog: &QuillStoryCatalog,
    old_text_end: usize,
    anchor: usize,
) -> Result<(), QuillStoryWriteError> {
    for node in &catalog.descriptor_nodes {
        let start = usize::try_from(node.source.offset)
            .map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
        let end = usize::try_from(
            node.source
                .end()
                .ok_or(QuillStoryWriteError::IntegerOverflow)?,
        )
        .map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
        if start < anchor && end > old_text_end {
            return Err(QuillStoryWriteError::DescriptorMetadataInsideRelocationWindow);
        }
    }
    Ok(())
}

fn collect_fd_boundaries(
    bytes: &[u8],
    descriptors: &[&QuillChunkDescriptor],
    text_start: usize,
    text_end: usize,
) -> Result<Vec<BoundaryLocation>, QuillStoryWriteError> {
    let mut result = Vec::new();
    let text_start =
        u32::try_from(text_start).map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
    let text_end = u32::try_from(text_end).map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
    for descriptor in descriptors
        .iter()
        .copied()
        .filter(|descriptor| descriptor.name.value == FDPP || descriptor.name.value == FDPC)
    {
        let name = descriptor.name.value;
        let start = usize::try_from(descriptor.data_offset.value)
            .map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
        let len = usize::try_from(descriptor.data_length.value)
            .map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
        let end = start
            .checked_add(len)
            .ok_or(QuillStoryWriteError::IntegerOverflow)?;
        let chunk = bytes
            .get(start..end)
            .ok_or(QuillStoryWriteError::MalformedBoundaryChunk(name))?;
        if chunk.len() < 8 {
            return Err(QuillStoryWriteError::MalformedBoundaryChunk(name));
        }
        let count = usize::from(read_u16(chunk, 0)?);
        let minimum = 8usize
            .checked_add(
                count
                    .checked_mul(6)
                    .ok_or(QuillStoryWriteError::IntegerOverflow)?,
            )
            .ok_or(QuillStoryWriteError::IntegerOverflow)?;
        if minimum > chunk.len() {
            return Err(QuillStoryWriteError::MalformedBoundaryChunk(name));
        }
        for index in 0..count {
            let relative = 8 + index * 4;
            let value = read_u32(chunk, relative)?;
            if value < text_start || value > text_end {
                return Err(QuillStoryWriteError::BoundaryOutsideText(name, value));
            }
            result.push(BoundaryLocation {
                name,
                field_offset: start + relative,
                value,
            });
        }
    }
    Ok(result)
}

fn collect_bte_words(
    bytes: &[u8],
    descriptors: &[&QuillChunkDescriptor],
    text_start: usize,
    text_end: usize,
) -> Result<Vec<BteWord>, QuillStoryWriteError> {
    let mut result = Vec::new();
    let text_start =
        u32::try_from(text_start).map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
    let text_end = u32::try_from(text_end).map_err(|_| QuillStoryWriteError::IntegerOverflow)?;

    for descriptor in descriptors
        .iter()
        .copied()
        .filter(|descriptor| descriptor.name.value == BTEP || descriptor.name.value == BTEC)
    {
        let name = descriptor.name.value;
        let start = usize::try_from(descriptor.data_offset.value)
            .map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
        let len = usize::try_from(descriptor.data_length.value)
            .map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
        let end = start
            .checked_add(len)
            .ok_or(QuillStoryWriteError::IntegerOverflow)?;
        let chunk = bytes
            .get(start..end)
            .ok_or(QuillStoryWriteError::MalformedBteChunk(name))?;
        if chunk.len() < 16 {
            return Err(QuillStoryWriteError::MalformedBteChunk(name));
        }

        let count = usize::try_from(read_u32(chunk, 0)?)
            .map_err(|_| QuillStoryWriteError::IntegerOverflow)?;
        let data_size = read_u32(chunk, 4)?;
        if data_size != 4 {
            return Err(QuillStoryWriteError::UnsupportedBteDataSize(
                name, data_size,
            ));
        }

        let position_count = count
            .checked_add(1)
            .ok_or(QuillStoryWriteError::IntegerOverflow)?;
        let positions_bytes = position_count
            .checked_mul(4)
            .ok_or(QuillStoryWriteError::IntegerOverflow)?;
        let targets_bytes = count
            .checked_mul(4)
            .ok_or(QuillStoryWriteError::IntegerOverflow)?;
        let required = 12usize
            .checked_add(positions_bytes)
            .and_then(|value| value.checked_add(targets_bytes))
            .ok_or(QuillStoryWriteError::IntegerOverflow)?;
        if required > chunk.len() {
            return Err(QuillStoryWriteError::MalformedBteChunk(name));
        }

        for index in 0..position_count {
            let relative = 12usize
                .checked_add(
                    index
                        .checked_mul(4)
                        .ok_or(QuillStoryWriteError::IntegerOverflow)?,
                )
                .ok_or(QuillStoryWriteError::IntegerOverflow)?;
            let value = read_u32(chunk, relative)?;
            if value != 0 && (value < text_start || value > text_end) {
                return Err(QuillStoryWriteError::BoundaryOutsideText(name, value));
            }
            result.push(BteWord {
                kind: BteWordKind::Position,
                field_offset: start
                    .checked_add(relative)
                    .ok_or(QuillStoryWriteError::IntegerOverflow)?,
                value,
            });
        }

        let targets_start = 12usize
            .checked_add(positions_bytes)
            .ok_or(QuillStoryWriteError::IntegerOverflow)?;
        for index in 0..count {
            let relative = targets_start
                .checked_add(
                    index
                        .checked_mul(4)
                        .ok_or(QuillStoryWriteError::IntegerOverflow)?,
                )
                .ok_or(QuillStoryWriteError::IntegerOverflow)?;
            result.push(BteWord {
                kind: BteWordKind::Target,
                field_offset: start
                    .checked_add(relative)
                    .ok_or(QuillStoryWriteError::IntegerOverflow)?,
                value: read_u32(chunk, relative)?,
            });
        }
    }
    Ok(result)
}

fn relocated_field_offset(
    offset: usize,
    old_text_end: usize,
    anchor: usize,
    delta_bytes: i64,
) -> Result<usize, QuillStoryWriteError> {
    if offset >= old_text_end && offset < anchor {
        let shifted = i64::try_from(offset)
            .map_err(|_| QuillStoryWriteError::IntegerOverflow)?
            .checked_add(delta_bytes)
            .ok_or(QuillStoryWriteError::IntegerOverflow)?;
        usize::try_from(shifted).map_err(|_| QuillStoryWriteError::IntegerOverflow)
    } else {
        Ok(offset)
    }
}

fn validate_output(
    stream: StreamPath,
    source: &QuillStoryCatalog,
    output: &[u8],
    edit: OutputValidationEdit<'_>,
) -> Result<(), QuillStoryWriteError> {
    let parsed = parse_confirmed_story_catalog(stream, output)
        .map_err(|error| QuillStoryWriteError::OutputValidationFailed(error.to_string()))?;
    if parsed.stories.len() != source.stories.len() {
        return Err(QuillStoryWriteError::OutputValidationFailed(
            "Story count changed".into(),
        ));
    }
    let mut expected_units = Vec::new();
    expected_units.extend_from_slice(&edit.source_units[..edit.start]);
    expected_units.extend_from_slice(edit.replacement_units);
    expected_units.extend_from_slice(&edit.source_units[edit.delete_end..]);
    let expected = utf16_bytes(&expected_units);
    for (before, after) in source.stories.iter().zip(&parsed.stories) {
        if before.syid == edit.target_syid {
            if after.utf16le != expected {
                return Err(QuillStoryWriteError::OutputValidationFailed(
                    "edited Story mismatch".into(),
                ));
            }
        } else if before.utf16le != after.utf16le {
            return Err(QuillStoryWriteError::OutputValidationFailed(format!(
                "unrelated Story SYID {} changed",
                before.syid.0
            )));
        }
    }
    Ok(())
}

fn changed_ranges(stream: StreamPath, before: &[u8], after: &[u8]) -> Vec<RawSpan> {
    let mut result = Vec::new();
    let mut index = 0usize;
    while index < before.len().min(after.len()) {
        if before[index] == after[index] {
            index += 1;
            continue;
        }
        let start = index;
        while index < before.len().min(after.len()) && before[index] != after[index] {
            index += 1;
        }
        result.push(RawSpan {
            stream: stream.clone(),
            offset: start as u64,
            len: (index - start) as u64,
        });
    }
    result
}

fn utf16_units(bytes: &[u8]) -> Result<Vec<u16>, QuillStoryWriteError> {
    if bytes.len() % 2 != 0 {
        return Err(QuillStoryWriteError::OutputValidationFailed(
            "odd UTF-16LE byte count".into(),
        ));
    }
    Ok(bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect())
}

fn utf16_bytes(units: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(units.len() * 2);
    for unit in units {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

fn patch_u32(bytes: &mut [u8], offset: usize, value: u32) -> Result<(), QuillStoryWriteError> {
    let slot = bytes
        .get_mut(
            offset
                ..offset
                    .checked_add(4)
                    .ok_or(QuillStoryWriteError::IntegerOverflow)?,
        )
        .ok_or(QuillStoryWriteError::IntegerOverflow)?;
    slot.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, QuillStoryWriteError> {
    let value = bytes
        .get(
            offset
                ..offset
                    .checked_add(2)
                    .ok_or(QuillStoryWriteError::IntegerOverflow)?,
        )
        .ok_or(QuillStoryWriteError::IntegerOverflow)?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, QuillStoryWriteError> {
    let value = bytes
        .get(
            offset
                ..offset
                    .checked_add(4)
                    .ok_or(QuillStoryWriteError::IntegerOverflow)?,
        )
        .ok_or(QuillStoryWriteError::IntegerOverflow)?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn name_string(name: [u8; 4]) -> String {
    String::from_utf8_lossy(&name).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const STREAM: &str = "/Quill/QuillSub/CONTENTS";
    const TEXT_OFFSET: usize = 0x200;
    const TEXT_LEN: usize = 10;
    const STSH_OFFSET: usize = 0x20a;
    const STSH_LEN: usize = 0x14;
    const FDPP_OFFSET: usize = 0x400;
    const FDPC_OFFSET: usize = 0x600;
    const SYID_OFFSET: usize = 0x800;
    const STRS_OFFSET: usize = 0xa00;
    const BTEP_OFFSET: usize = 0xc00;
    const BTEC_OFFSET: usize = 0xe00;

    #[test]
    fn last_proven_capacity_edge_succeeds() {
        let source = synthetic_ordinary_quill();
        let plan = plan_quill_story_text_edit(
            StreamPath(STREAM.into()),
            &source,
            &QuillStoryTextEdit {
                story_syid: QuillSyid(5),
                start_utf16: 1,
                delete_utf16: 0,
                replacement: "X".repeat(241),
            },
        )
        .expect("the historical +482-byte edge must remain writable");

        assert_eq!(plan.delta_utf16, 241);
        assert_eq!(plan.delta_bytes, 482);
        assert_eq!(plan.first_aligned_service_anchor, 0x400);
        assert_eq!(plan.syid_header_before, 8);
        assert_eq!(plan.syid_header_after, 9);
        assert_eq!(plan.bte_position_patch_count, 2);
        assert_eq!(plan.bte_reference_patch_count, 0);

        let parsed = parse_confirmed_story_catalog(StreamPath(STREAM.into()), &plan.output_stream)
            .expect("generated Quill must remain structurally parseable");
        assert_eq!(parsed.stories[0].utf16_code_units, 246);
        let stsh = parsed
            .descriptor_nodes
            .iter()
            .flat_map(|node| node.descriptors.iter())
            .find(|descriptor| descriptor.name.value == *b"STSH")
            .expect("STSH descriptor");
        assert_eq!(stsh.data_offset.value, 0x3ec);
    }

    #[test]
    fn next_capacity_step_fails_closed() {
        let source = synthetic_ordinary_quill();
        let error = plan_quill_story_text_edit(
            StreamPath(STREAM.into()),
            &source,
            &QuillStoryTextEdit {
                story_syid: QuillSyid(5),
                start_utf16: 1,
                delete_utf16: 0,
                replacement: "X".repeat(242),
            },
        )
        .expect_err("the next two bytes would consume non-zero anchored data");

        assert_eq!(
            error,
            QuillStoryWriteError::PositiveGrowthWouldDiscardNonZeroUnknownAnchorBytes
        );
    }

    #[test]
    fn existing_fd_boundary_and_paragraph_mark_remain_closed() {
        let source = synthetic_ordinary_quill();

        let boundary_error = plan_quill_story_text_edit(
            StreamPath(STREAM.into()),
            &source,
            &QuillStoryTextEdit {
                story_syid: QuillSyid(5),
                start_utf16: 5,
                delete_utf16: 0,
                replacement: "X".into(),
            },
        )
        .expect_err("existing FD boundary insertion must stay rejected");
        assert_eq!(
            boundary_error,
            QuillStoryWriteError::InsertionAtExistingFdBoundary
        );

        let cr_error = plan_quill_story_text_edit(
            StreamPath(STREAM.into()),
            &source,
            &QuillStoryTextEdit {
                story_syid: QuillSyid(5),
                start_utf16: 1,
                delete_utf16: 0,
                replacement: "\r".into(),
            },
        )
        .expect_err("paragraph-mark edits are outside this writer profile");
        assert_eq!(cr_error, QuillStoryWriteError::ParagraphMarkMutation);
    }

    #[test]
    fn repeated_plan_is_byte_deterministic_and_preserves_unrelated_bytes() {
        let source = synthetic_ordinary_quill();
        let edit = QuillStoryTextEdit {
            story_syid: QuillSyid(5),
            start_utf16: 1,
            delete_utf16: 1,
            replacement: "XYZ".into(),
        };
        let first =
            plan_quill_story_text_edit(StreamPath(STREAM.into()), &source, &edit).expect("plan 1");
        let second =
            plan_quill_story_text_edit(StreamPath(STREAM.into()), &source, &edit).expect("plan 2");
        assert_eq!(first, second);

        let mut covered = vec![false; source.len()];
        for range in &first.changed_ranges {
            let start = range.offset as usize;
            let end = start + range.len as usize;
            covered[start..end].fill(true);
        }
        for (index, (before, after)) in source.iter().zip(&first.output_stream).enumerate() {
            assert_eq!(
                *before != *after,
                covered[index],
                "changed range mismatch at {index:#x}"
            );
        }
    }

    #[test]
    fn apache_poi_sample3_to_sample4_matches_owned_structural_projection() {
        let source_pub = decode_base64(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/Sample3.pub.b64"
        )));
        let oracle_pub = decode_base64(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/Sample4.pub.b64"
        )));
        let source_quill =
            pub_cfb::read_stream_reader(Cursor::new(source_pub), STREAM).expect("source Quill");
        let oracle_quill =
            pub_cfb::read_stream_reader(Cursor::new(oracle_pub), STREAM).expect("oracle Quill");

        let source_catalog =
            parse_confirmed_story_catalog(StreamPath(STREAM.into()), &source_quill)
                .expect("source Story catalog");
        let oracle_catalog =
            parse_confirmed_story_catalog(StreamPath(STREAM.into()), &oracle_quill)
                .expect("oracle Story catalog");

        let source_story = source_catalog
            .stories
            .iter()
            .find(|story| story.syid == QuillSyid(4))
            .expect("source Story SYID 4");
        let oracle_story = oracle_catalog
            .stories
            .iter()
            .find(|story| story.syid == QuillSyid(4))
            .expect("oracle Story SYID 4");

        let source_units = utf16_units(&source_story.utf16le).expect("source UTF-16");
        let deleted = "345678".encode_utf16().collect::<Vec<_>>();
        let start = source_units
            .windows(deleted.len())
            .position(|window| window == deleted)
            .expect("Sample3 controlled edit marker");
        let mut expected_units = source_units.clone();
        expected_units.drain(start..start + deleted.len());
        assert_eq!(utf16_bytes(&expected_units), oracle_story.utf16le);

        let plan = plan_quill_story_text_edit(
            StreamPath(STREAM.into()),
            &source_quill,
            &QuillStoryTextEdit {
                story_syid: QuillSyid(4),
                start_utf16: u32::try_from(start).expect("start fits u32"),
                delete_utf16: u32::try_from(deleted.len()).expect("length fits u32"),
                replacement: String::new(),
            },
        )
        .expect("controlled POI shrink must be writable");

        assert_eq!(plan.delta_utf16, -6);
        assert_eq!(plan.delta_bytes, -12);
        assert_eq!(plan.bte_position_patch_count, 3);
        assert_eq!(plan.bte_reference_patch_count, 0);

        let generated_catalog =
            parse_confirmed_story_catalog(StreamPath(STREAM.into()), &plan.output_stream)
                .expect("generated Story catalog");

        assert_eq!(
            generated_catalog
                .stories
                .iter()
                .map(|story| (story.syid, story.utf16le.as_slice()))
                .collect::<Vec<_>>(),
            oracle_catalog
                .stories
                .iter()
                .map(|story| (story.syid, story.utf16le.as_slice()))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            generated_catalog
                .strs
                .lengths
                .iter()
                .map(|length| length.value)
                .collect::<Vec<_>>(),
            oracle_catalog
                .strs
                .lengths
                .iter()
                .map(|length| length.value)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            owned_descriptor_projection(&generated_catalog),
            owned_descriptor_projection(&oracle_catalog)
        );
        assert_eq!(
            fd_boundary_projection(&plan.output_stream, &generated_catalog),
            fd_boundary_projection(&oracle_quill, &oracle_catalog)
        );
        assert_eq!(
            bte_reference_projection(&plan.output_stream, &generated_catalog),
            bte_reference_projection(&oracle_quill, &oracle_catalog)
        );

        // The historical pair has save/session-service deltas in opaque STSH/FONT bytes
        // and does not form a native save chain. Those bytes are intentionally not an
        // equality gate for this preservation-first writer regression.
        assert_ne!(plan.output_stream, oracle_quill);
    }

    fn synthetic_ordinary_quill() -> Vec<u8> {
        let mut bytes = vec![0u8; 0x1000];
        let descriptors = [
            (*b"TEXT", *b"TEXT", TEXT_OFFSET as u32, TEXT_LEN as u32),
            (*b"STSH", *b"STSH", STSH_OFFSET as u32, STSH_LEN as u32),
            (*b"FDPP", *b"FDPP", FDPP_OFFSET as u32, 0x20),
            (*b"FDPC", *b"FDPC", FDPC_OFFSET as u32, 0x20),
            (*b"SYID", *b"SYID", SYID_OFFSET as u32, 0x0c),
            (*b"STRS", *b"STRS", STRS_OFFSET as u32, 0x10),
            (*b"BTEP", *b"PLC ", BTEP_OFFSET as u32, 0x18),
            (*b"BTEC", *b"PLC ", BTEC_OFFSET as u32, 0x18),
        ];

        put_u16(&mut bytes, 0x18, 0);
        put_u16(&mut bytes, 0x1a, descriptors.len() as u16);
        put_u32(&mut bytes, 0x1c, 0xffff_ffff);
        for (index, (name, bit_type, offset, len)) in descriptors.iter().enumerate() {
            put_descriptor(
                &mut bytes,
                0x20 + index * 24,
                *name,
                *bit_type,
                *offset,
                *len,
            );
        }

        let text = utf16_bytes(&['A' as u16, 'B' as u16, 'C' as u16, 'D' as u16, 0x000d]);
        bytes[TEXT_OFFSET..TEXT_OFFSET + text.len()].copy_from_slice(&text);
        bytes[STSH_OFFSET..STSH_OFFSET + STSH_LEN].fill(0xa5);

        put_boundary_chunk(&mut bytes, FDPP_OFFSET, (TEXT_OFFSET + TEXT_LEN) as u32);
        put_boundary_chunk(&mut bytes, FDPC_OFFSET, (TEXT_OFFSET + TEXT_LEN) as u32);

        put_u32(&mut bytes, SYID_OFFSET, 8);
        put_u32(&mut bytes, SYID_OFFSET + 4, 1);
        put_u32(&mut bytes, SYID_OFFSET + 8, 5);

        put_u32(&mut bytes, STRS_OFFSET, 1);
        put_u32(&mut bytes, STRS_OFFSET + 4, 8);
        put_u32(&mut bytes, STRS_OFFSET + 12, 5);

        put_bte_chunk(
            &mut bytes,
            BTEP_OFFSET,
            STSH_OFFSET as u32,
            FDPP_OFFSET as u32,
        );
        put_bte_chunk(
            &mut bytes,
            BTEC_OFFSET,
            STSH_OFFSET as u32,
            FDPC_OFFSET as u32,
        );
        bytes
    }

    fn owned_descriptor_projection(catalog: &QuillStoryCatalog) -> Vec<([u8; 4], u32, u32)> {
        catalog
            .descriptor_nodes
            .iter()
            .flat_map(|node| node.descriptors.iter())
            .filter(|descriptor| {
                matches!(
                    descriptor.name.value,
                    TEXT | FDPP | FDPC | BTEP | BTEC | [b'S', b'T', b'S', b'H']
                )
            })
            .map(|descriptor| {
                (
                    descriptor.name.value,
                    descriptor.data_offset.value,
                    descriptor.data_length.value,
                )
            })
            .collect()
    }

    fn fd_boundary_projection(bytes: &[u8], catalog: &QuillStoryCatalog) -> Vec<([u8; 4], u32)> {
        let descriptors = all_descriptors(catalog);
        let text = unique_descriptor(&descriptors, TEXT).expect("TEXT descriptor");
        let text_start = text.data_offset.value as usize;
        let text_end = text_start + text.data_length.value as usize;
        collect_fd_boundaries(bytes, &descriptors, text_start, text_end)
            .expect("FD boundary projection")
            .into_iter()
            .map(|boundary| (boundary.name, boundary.value))
            .collect()
    }

    fn bte_reference_projection(bytes: &[u8], catalog: &QuillStoryCatalog) -> Vec<u32> {
        let descriptors = all_descriptors(catalog);
        let text = unique_descriptor(&descriptors, TEXT).expect("TEXT descriptor");
        let text_start = text.data_offset.value as usize;
        let text_end = text_start + text.data_length.value as usize;
        collect_bte_words(bytes, &descriptors, text_start, text_end)
            .expect("BTE reference projection")
            .into_iter()
            .map(|word| word.value)
            .collect()
    }

    fn decode_base64(text: &str) -> Vec<u8> {
        let cleaned = text
            .bytes()
            .filter(|byte| !byte.is_ascii_whitespace())
            .collect::<Vec<_>>();
        assert_eq!(cleaned.len() % 4, 0, "base64 fixture length");

        let mut output = Vec::with_capacity(cleaned.len() / 4 * 3);
        for quartet in cleaned.chunks_exact(4) {
            let a = base64_value(quartet[0]);
            let b = base64_value(quartet[1]);
            let c = if quartet[2] == b'=' {
                0
            } else {
                base64_value(quartet[2])
            };
            let d = if quartet[3] == b'=' {
                0
            } else {
                base64_value(quartet[3])
            };
            output.push((a << 2) | (b >> 4));
            if quartet[2] != b'=' {
                output.push((b << 4) | (c >> 2));
            }
            if quartet[3] != b'=' {
                output.push((c << 6) | d);
            }
        }
        output
    }

    fn base64_value(byte: u8) -> u8 {
        match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            other => panic!("invalid base64 byte {other:#x}"),
        }
    }

    fn put_descriptor(
        bytes: &mut [u8],
        offset: usize,
        name: [u8; 4],
        bit_type: [u8; 4],
        data_offset: u32,
        data_length: u32,
    ) {
        put_u16(bytes, offset, 0x0018);
        bytes[offset + 2..offset + 6].copy_from_slice(&name);
        bytes[offset + 12..offset + 16].copy_from_slice(&bit_type);
        put_u32(bytes, offset + 16, data_offset);
        put_u32(bytes, offset + 20, data_length);
    }

    fn put_boundary_chunk(bytes: &mut [u8], offset: usize, boundary: u32) {
        put_u16(bytes, offset, 1);
        put_u32(bytes, offset + 8, boundary);
        put_u16(bytes, offset + 12, 0x10);
    }

    fn put_bte_chunk(bytes: &mut [u8], offset: usize, first: u32, second: u32) {
        put_u32(bytes, offset, 1);
        put_u32(bytes, offset + 4, 4);
        put_u32(bytes, offset + 16, first);
        put_u32(bytes, offset + 20, second);
    }

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
}
