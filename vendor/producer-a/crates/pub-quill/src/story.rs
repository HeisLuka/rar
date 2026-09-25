use crate::tokn::{QuillToknChunk, parse_tokn_chunks};
use pub_core::{Decoded, QuillSyid, RawSpan, StreamPath};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

pub const QUILL_DESCRIPTOR_LIST_ROOT_OFFSET: u32 = 0x18;
pub const QUILL_DESCRIPTOR_LIST_END: u32 = 0xffff_ffff;
pub const QUILL_DESCRIPTOR_PRESENCE_MARKER: u16 = 0x0018;
pub const QUILL_DESCRIPTOR_SIZE: usize = 24;

const TEXT: [u8; 4] = *b"TEXT";
const SYID: [u8; 4] = *b"SYID";
const STRS: [u8; 4] = *b"STRS";
const TCD: [u8; 4] = *b"TCD ";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillDescriptorListNode {
    pub source: RawSpan,
    pub service: Decoded<u16>,
    pub count: Decoded<u16>,
    pub next: Decoded<u32>,
    pub descriptors: Vec<QuillChunkDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillChunkDescriptor {
    pub source: RawSpan,
    pub presence_marker: Decoded<u16>,
    pub name: Decoded<[u8; 4]>,
    pub option_a: Decoded<u16>,
    pub option_b: Decoded<u16>,
    pub option_c: Decoded<u16>,
    pub bit_type: Decoded<[u8; 4]>,
    pub data_offset: Decoded<u32>,
    pub data_length: Decoded<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillSyidChunk {
    pub source: RawSpan,
    pub header: Decoded<u32>,
    pub count: Decoded<u32>,
    pub ids: Vec<Decoded<QuillSyid>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillStrsChunk {
    pub source: RawSpan,
    pub count: Decoded<u32>,
    pub service_span: Decoded<u32>,
    pub lengths: Vec<Decoded<u32>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillTextChunk {
    pub source: RawSpan,
    pub bytes: Vec<u8>,
}

/// Raw-backed TCD table-cell boundary chunk.
///
/// `story_ordinal` is the TCD descriptor's type-specific option A. For TCD it
/// is a zero-based index into the parallel SYID/STRS arrays. The stored cell
/// boundaries are preserved verbatim; historical consumer-specific CR fixups
/// are deliberately not applied in this reader.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillTcdChunk {
    pub source: RawSpan,
    pub descriptor_source: RawSpan,
    pub story_ordinal: Decoded<u16>,
    pub story_syid: Decoded<QuillSyid>,
    pub story_utf16_code_units: Decoded<u32>,
    pub stored_cell_count_minus_one: Decoded<u32>,
    pub header_word_1: Decoded<u32>,
    pub header_word_2: Decoded<u32>,
    pub cell_end_offsets_utf16: Vec<Decoded<u32>>,
    pub opaque_tail_source: Option<RawSpan>,
    pub opaque_tail: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillStorySlice {
    pub index: u32,
    pub syid: QuillSyid,
    pub syid_source: RawSpan,
    pub utf16_code_units: u32,
    pub length_source: RawSpan,
    pub text_source: RawSpan,
    pub utf16le: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillStoryCatalog {
    pub descriptor_nodes: Vec<QuillDescriptorListNode>,
    pub syid: QuillSyidChunk,
    pub strs: QuillStrsChunk,
    pub text: QuillTextChunk,
    pub stories: Vec<QuillStorySlice>,
    pub tcd: Vec<QuillTcdChunk>,
    pub tokn: Vec<QuillToknChunk>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuillStoryReadError {
    TooShort {
        offset: usize,
        requested: usize,
        available: usize,
    },
    DescriptorListPointerOutOfBounds {
        offset: u32,
        stream_len: usize,
    },
    DescriptorListCycle {
        offset: u32,
    },
    UnexpectedDescriptorPresenceMarker {
        offset: u64,
        found: u16,
    },
    ChunkOutOfBounds {
        name: [u8; 4],
        offset: u32,
        length: u32,
    },
    MissingRequiredChunk {
        name: [u8; 4],
    },
    DuplicateRequiredChunk {
        name: [u8; 4],
    },
    StrsServiceSpanOutOfBounds {
        service_span: u32,
        chunk_length: u32,
    },
    StoryCountMismatch {
        syid_count: u32,
        strs_count: u32,
    },
    TextLengthOverflow,
    TextLengthMismatch {
        expected_bytes: u64,
        actual_bytes: u32,
    },
    TcdStoryOrdinalOutOfBounds {
        story_ordinal: u16,
        story_count: u32,
    },
    TcdCellCountOverflow {
        stored_cell_count_minus_one: u32,
    },
    ToknStoryOrdinalOutOfBounds {
        story_ordinal: u16,
        story_count: u32,
    },
    ToknUnexpectedPlcType {
        found: u32,
    },
    ToknCountOverflow {
        count: u32,
    },
    ToknNonMonotonicBoundary {
        previous: u32,
        next: u32,
    },
    ToknInvalidBlockLength {
        offset: u64,
        length: u32,
    },
    ToknTokenSpanOverflow {
        start: u32,
        length: u32,
    },
    ToknTokenLengthExceedsBoundary {
        start: u32,
        length: u32,
        boundary: u32,
    },
    ToknTargetSectionOverflow,
}

impl fmt::Display for QuillStoryReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for QuillStoryReadError {}

/// Parses only the grounded Quill descriptor/SYID/STRS/TEXT subset.
///
/// The reader preserves raw field bytes and exact spans, follows descriptor
/// overflow nodes, keeps descriptor option deviations, and never decodes TEXT
/// lossily. TCD is parsed only as a raw-backed Story-scoped cell-boundary relation. TOKN is parsed as a generic raw-backed two-phase token carrier joined to Story identity through descriptor ordinal -> SYID. FDPP/FDPC semantics, CELLS/table geometry, Hyperlink entities and CDM field mapping remain deliberately out of scope.
pub fn parse_confirmed_story_catalog(
    stream: StreamPath,
    bytes: &[u8],
) -> Result<QuillStoryCatalog, QuillStoryReadError> {
    let descriptor_nodes = parse_descriptor_nodes(stream.clone(), bytes)?;
    let descriptors = descriptor_nodes
        .iter()
        .flat_map(|node| node.descriptors.iter())
        .collect::<Vec<_>>();
    let syid_descriptor = required_descriptor(&descriptors, SYID)?;
    let strs_descriptor = required_descriptor(&descriptors, STRS)?;
    let text_descriptor = required_descriptor(&descriptors, TEXT)?;

    let syid = parse_syid(stream.clone(), bytes, syid_descriptor)?;
    let strs = parse_strs(stream.clone(), bytes, strs_descriptor)?;
    let text = parse_text(stream.clone(), bytes, text_descriptor)?;

    if syid.count.value != strs.count.value {
        return Err(QuillStoryReadError::StoryCountMismatch {
            syid_count: syid.count.value,
            strs_count: strs.count.value,
        });
    }

    let expected_bytes = strs.lengths.iter().try_fold(0_u64, |sum, length| {
        u64::from(length.value)
            .checked_mul(2)
            .and_then(|len| sum.checked_add(len))
    });
    let Some(expected_bytes) = expected_bytes else {
        return Err(QuillStoryReadError::TextLengthOverflow);
    };
    if expected_bytes != u64::from(text_descriptor.data_length.value) {
        return Err(QuillStoryReadError::TextLengthMismatch {
            expected_bytes,
            actual_bytes: text_descriptor.data_length.value,
        });
    }

    let text_start = usize::try_from(text_descriptor.data_offset.value)
        .map_err(|_| QuillStoryReadError::TextLengthOverflow)?;
    let mut cursor = text_start;
    let mut stories = Vec::with_capacity(syid.ids.len());
    for (index, (id, length)) in syid.ids.iter().zip(&strs.lengths).enumerate() {
        let byte_len = usize::try_from(length.value)
            .ok()
            .and_then(|units| units.checked_mul(2))
            .ok_or(QuillStoryReadError::TextLengthOverflow)?;
        let end = cursor
            .checked_add(byte_len)
            .ok_or(QuillStoryReadError::TextLengthOverflow)?;
        let utf16le = bytes
            .get(cursor..end)
            .ok_or(QuillStoryReadError::TextLengthOverflow)?
            .to_vec();
        stories.push(QuillStorySlice {
            index: u32::try_from(index).map_err(|_| QuillStoryReadError::TextLengthOverflow)?,
            syid: id.value,
            syid_source: id.source.clone(),
            utf16_code_units: length.value,
            length_source: length.source.clone(),
            text_source: span(stream.clone(), cursor, byte_len),
            utf16le,
        });
        cursor = end;
    }

    let tcd = parse_tcd_chunks(stream.clone(), bytes, &descriptors, &syid, &strs)?;
    let tokn = parse_tokn_chunks(stream.clone(), bytes, &descriptors, &syid.ids)?;

    Ok(QuillStoryCatalog {
        descriptor_nodes,
        syid,
        strs,
        text,
        stories,
        tcd,
        tokn,
    })
}

fn parse_descriptor_nodes(
    stream: StreamPath,
    bytes: &[u8],
) -> Result<Vec<QuillDescriptorListNode>, QuillStoryReadError> {
    let mut current = QUILL_DESCRIPTOR_LIST_ROOT_OFFSET;
    let mut seen = BTreeSet::new();
    let mut nodes = Vec::new();

    while current != QUILL_DESCRIPTOR_LIST_END {
        let start = usize::try_from(current).map_err(|_| {
            QuillStoryReadError::DescriptorListPointerOutOfBounds {
                offset: current,
                stream_len: bytes.len(),
            }
        })?;
        if start >= bytes.len() {
            return Err(QuillStoryReadError::DescriptorListPointerOutOfBounds {
                offset: current,
                stream_len: bytes.len(),
            });
        }
        if !seen.insert(current) {
            return Err(QuillStoryReadError::DescriptorListCycle { offset: current });
        }

        let mut cursor = Cursor::bounded(stream.clone(), bytes, start, bytes.len() - start)?;
        let node_start = cursor.position();
        let service = cursor.u16()?;
        let count = cursor.u16()?;
        let next = cursor.u32()?;
        let descriptor_bytes = usize::from(count.value)
            .checked_mul(QUILL_DESCRIPTOR_SIZE)
            .ok_or(QuillStoryReadError::TextLengthOverflow)?;
        if cursor.remaining() < descriptor_bytes {
            return Err(QuillStoryReadError::TooShort {
                offset: cursor.position(),
                requested: descriptor_bytes,
                available: cursor.remaining(),
            });
        }

        let mut descriptors = Vec::with_capacity(usize::from(count.value));
        for _ in 0..count.value {
            descriptors.push(parse_descriptor(&mut cursor)?);
        }
        nodes.push(QuillDescriptorListNode {
            source: span(stream.clone(), node_start, cursor.position() - node_start),
            service,
            count,
            next: next.clone(),
            descriptors,
        });
        current = next.value;
    }
    Ok(nodes)
}

fn parse_descriptor(cursor: &mut Cursor<'_>) -> Result<QuillChunkDescriptor, QuillStoryReadError> {
    let start = cursor.position();
    let presence_marker = cursor.u16()?;
    if presence_marker.value != QUILL_DESCRIPTOR_PRESENCE_MARKER {
        return Err(QuillStoryReadError::UnexpectedDescriptorPresenceMarker {
            offset: presence_marker.source.offset,
            found: presence_marker.value,
        });
    }
    let name = cursor.name4()?;
    let option_a = cursor.u16()?;
    let option_b = cursor.u16()?;
    let option_c = cursor.u16()?;
    let bit_type = cursor.name4()?;
    let data_offset = cursor.u32()?;
    let data_length = cursor.u32()?;
    Ok(QuillChunkDescriptor {
        source: span(cursor.stream.clone(), start, cursor.position() - start),
        presence_marker,
        name,
        option_a,
        option_b,
        option_c,
        bit_type,
        data_offset,
        data_length,
    })
}

fn required_descriptor<'a>(
    descriptors: &[&'a QuillChunkDescriptor],
    name: [u8; 4],
) -> Result<&'a QuillChunkDescriptor, QuillStoryReadError> {
    let mut found = descriptors
        .iter()
        .copied()
        .filter(|item| item.name.value == name);
    let first = found
        .next()
        .ok_or(QuillStoryReadError::MissingRequiredChunk { name })?;
    if found.next().is_some() {
        return Err(QuillStoryReadError::DuplicateRequiredChunk { name });
    }
    Ok(first)
}

fn chunk_range(
    bytes: &[u8],
    descriptor: &QuillChunkDescriptor,
) -> Result<(usize, usize), QuillStoryReadError> {
    let offset = descriptor.data_offset.value;
    let length = descriptor.data_length.value;
    let start = usize::try_from(offset).map_err(|_| QuillStoryReadError::ChunkOutOfBounds {
        name: descriptor.name.value,
        offset,
        length,
    })?;
    let len = usize::try_from(length).map_err(|_| QuillStoryReadError::ChunkOutOfBounds {
        name: descriptor.name.value,
        offset,
        length,
    })?;
    let end = start
        .checked_add(len)
        .ok_or(QuillStoryReadError::ChunkOutOfBounds {
            name: descriptor.name.value,
            offset,
            length,
        })?;
    if end > bytes.len() {
        return Err(QuillStoryReadError::ChunkOutOfBounds {
            name: descriptor.name.value,
            offset,
            length,
        });
    }
    Ok((start, len))
}

fn parse_syid(
    stream: StreamPath,
    bytes: &[u8],
    descriptor: &QuillChunkDescriptor,
) -> Result<QuillSyidChunk, QuillStoryReadError> {
    let (start, len) = chunk_range(bytes, descriptor)?;
    let mut cursor = Cursor::bounded(stream.clone(), bytes, start, len)?;
    let header = cursor.u32()?;
    let count = cursor.u32()?;
    let required = usize::try_from(count.value)
        .ok()
        .and_then(|count| count.checked_mul(4))
        .ok_or(QuillStoryReadError::TextLengthOverflow)?;
    if cursor.remaining() < required {
        return Err(QuillStoryReadError::TooShort {
            offset: cursor.position(),
            requested: required,
            available: cursor.remaining(),
        });
    }
    let mut ids = Vec::with_capacity(usize::try_from(count.value).unwrap_or(0));
    for _ in 0..count.value {
        let raw = cursor.u32()?;
        ids.push(Decoded {
            value: QuillSyid(raw.value),
            source: raw.source,
            raw: raw.raw,
        });
    }
    Ok(QuillSyidChunk {
        source: span(stream, start, len),
        header,
        count,
        ids,
    })
}

fn parse_strs(
    stream: StreamPath,
    bytes: &[u8],
    descriptor: &QuillChunkDescriptor,
) -> Result<QuillStrsChunk, QuillStoryReadError> {
    let (start, len) = chunk_range(bytes, descriptor)?;
    let mut header = Cursor::bounded(stream.clone(), bytes, start, len)?;
    let count = header.u32()?;
    let service_span = header.u32()?;
    let relative = 4_u64
        .checked_add(u64::from(service_span.value))
        .ok_or(QuillStoryReadError::TextLengthOverflow)?;
    if relative > u64::from(descriptor.data_length.value) {
        return Err(QuillStoryReadError::StrsServiceSpanOutOfBounds {
            service_span: service_span.value,
            chunk_length: descriptor.data_length.value,
        });
    }
    let relative =
        usize::try_from(relative).map_err(|_| QuillStoryReadError::TextLengthOverflow)?;
    let lengths_start = start
        .checked_add(relative)
        .ok_or(QuillStoryReadError::TextLengthOverflow)?;
    let mut cursor = Cursor::bounded(stream.clone(), bytes, lengths_start, len - relative)?;
    let required = usize::try_from(count.value)
        .ok()
        .and_then(|count| count.checked_mul(4))
        .ok_or(QuillStoryReadError::TextLengthOverflow)?;
    if cursor.remaining() < required {
        return Err(QuillStoryReadError::TooShort {
            offset: cursor.position(),
            requested: required,
            available: cursor.remaining(),
        });
    }
    let mut lengths = Vec::with_capacity(usize::try_from(count.value).unwrap_or(0));
    for _ in 0..count.value {
        lengths.push(cursor.u32()?);
    }
    Ok(QuillStrsChunk {
        source: span(stream, start, len),
        count,
        service_span,
        lengths,
    })
}

fn parse_tcd_chunks(
    stream: StreamPath,
    bytes: &[u8],
    descriptors: &[&QuillChunkDescriptor],
    syid: &QuillSyidChunk,
    strs: &QuillStrsChunk,
) -> Result<Vec<QuillTcdChunk>, QuillStoryReadError> {
    let mut chunks = Vec::new();

    for descriptor in descriptors
        .iter()
        .copied()
        .filter(|descriptor| descriptor.name.value == TCD)
    {
        let ordinal = usize::from(descriptor.option_a.value);
        if ordinal >= syid.ids.len() || ordinal >= strs.lengths.len() {
            return Err(QuillStoryReadError::TcdStoryOrdinalOutOfBounds {
                story_ordinal: descriptor.option_a.value,
                story_count: syid.count.value,
            });
        }

        let (start, len) = chunk_range(bytes, descriptor)?;
        let mut cursor = Cursor::bounded(stream.clone(), bytes, start, len)?;
        let stored_cell_count_minus_one = cursor.u32()?;
        let header_word_1 = cursor.u32()?;
        let header_word_2 = cursor.u32()?;
        let boundary_count = stored_cell_count_minus_one.value.checked_add(1).ok_or(
            QuillStoryReadError::TcdCellCountOverflow {
                stored_cell_count_minus_one: stored_cell_count_minus_one.value,
            },
        )?;
        let boundary_count = usize::try_from(boundary_count).map_err(|_| {
            QuillStoryReadError::TcdCellCountOverflow {
                stored_cell_count_minus_one: stored_cell_count_minus_one.value,
            }
        })?;
        let required =
            boundary_count
                .checked_mul(4)
                .ok_or(QuillStoryReadError::TcdCellCountOverflow {
                    stored_cell_count_minus_one: stored_cell_count_minus_one.value,
                })?;
        if cursor.remaining() < required {
            return Err(QuillStoryReadError::TooShort {
                offset: cursor.position(),
                requested: required,
                available: cursor.remaining(),
            });
        }

        let mut cell_end_offsets_utf16 = Vec::with_capacity(boundary_count);
        for _ in 0..boundary_count {
            cell_end_offsets_utf16.push(cursor.u32()?);
        }

        let (opaque_tail_source, opaque_tail) = if cursor.remaining() == 0 {
            (None, Vec::new())
        } else {
            let remaining = cursor.remaining();
            let (tail, source) = cursor.take(remaining)?;
            (Some(source), tail.to_vec())
        };

        chunks.push(QuillTcdChunk {
            source: span(stream.clone(), start, len),
            descriptor_source: descriptor.source.clone(),
            story_ordinal: descriptor.option_a.clone(),
            story_syid: syid.ids[ordinal].clone(),
            story_utf16_code_units: strs.lengths[ordinal].clone(),
            stored_cell_count_minus_one,
            header_word_1,
            header_word_2,
            cell_end_offsets_utf16,
            opaque_tail_source,
            opaque_tail,
        });
    }

    Ok(chunks)
}

fn parse_text(
    stream: StreamPath,
    bytes: &[u8],
    descriptor: &QuillChunkDescriptor,
) -> Result<QuillTextChunk, QuillStoryReadError> {
    let (start, len) = chunk_range(bytes, descriptor)?;
    Ok(QuillTextChunk {
        source: span(stream, start, len),
        bytes: bytes[start..start + len].to_vec(),
    })
}

fn span(stream: StreamPath, start: usize, len: usize) -> RawSpan {
    RawSpan {
        stream,
        offset: start as u64,
        len: len as u64,
    }
}

#[derive(Debug, Clone)]
struct Cursor<'a> {
    stream: StreamPath,
    bytes: &'a [u8],
    position: usize,
    limit: usize,
}

impl<'a> Cursor<'a> {
    fn bounded(
        stream: StreamPath,
        bytes: &'a [u8],
        start: usize,
        len: usize,
    ) -> Result<Self, QuillStoryReadError> {
        let end = start
            .checked_add(len)
            .filter(|end| *end <= bytes.len())
            .ok_or(QuillStoryReadError::TooShort {
                offset: start,
                requested: len,
                available: bytes.len().saturating_sub(start),
            })?;
        Ok(Self {
            stream,
            bytes,
            position: start,
            limit: end,
        })
    }

    fn position(&self) -> usize {
        self.position
    }
    fn remaining(&self) -> usize {
        self.limit.saturating_sub(self.position)
    }

    fn take(&mut self, len: usize) -> Result<(&'a [u8], RawSpan), QuillStoryReadError> {
        let start = self.position;
        let end = start
            .checked_add(len)
            .filter(|end| *end <= self.limit)
            .ok_or(QuillStoryReadError::TooShort {
                offset: start,
                requested: len,
                available: self.remaining(),
            })?;
        self.position = end;
        Ok((
            &self.bytes[start..end],
            span(self.stream.clone(), start, len),
        ))
    }

    fn u16(&mut self) -> Result<Decoded<u16>, QuillStoryReadError> {
        let (bytes, source) = self.take(2)?;
        Ok(Decoded {
            value: u16::from_le_bytes([bytes[0], bytes[1]]),
            source,
            raw: bytes.to_vec(),
        })
    }

    fn u32(&mut self) -> Result<Decoded<u32>, QuillStoryReadError> {
        let (bytes, source) = self.take(4)?;
        Ok(Decoded {
            value: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            source,
            raw: bytes.to_vec(),
        })
    }

    fn name4(&mut self) -> Result<Decoded<[u8; 4]>, QuillStoryReadError> {
        let (bytes, source) = self.take(4)?;
        Ok(Decoded {
            value: [bytes[0], bytes[1], bytes[2], bytes[3]],
            source,
            raw: bytes.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn w32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn descriptor(
        bytes: &mut [u8],
        offset: usize,
        name: [u8; 4],
        option_b: u16,
        option_c: u16,
        data_offset: u32,
        data_length: u32,
    ) {
        w16(bytes, offset, QUILL_DESCRIPTOR_PRESENCE_MARKER);
        bytes[offset + 2..offset + 6].copy_from_slice(&name);
        w16(bytes, offset + 6, 0);
        w16(bytes, offset + 8, option_b);
        w16(bytes, offset + 10, option_c);
        bytes[offset + 12..offset + 16].copy_from_slice(&name);
        w32(bytes, offset + 16, data_offset);
        w32(bytes, offset + 20, data_length);
    }

    fn fixture() -> (StreamPath, Vec<u8>) {
        let stream = StreamPath("/Quill/QuillSub/CONTENTS".into());
        let mut bytes = vec![0; 0x180];
        w16(&mut bytes, 0x18, 0x18);
        w16(&mut bytes, 0x1a, 2);
        w32(&mut bytes, 0x1c, 0x70);
        descriptor(&mut bytes, 0x20, SYID, 7, 9, 0x100, 16);
        descriptor(&mut bytes, 0x38, STRS, 1, 0, 0x120, 20);
        w16(&mut bytes, 0x70, 0x18);
        w16(&mut bytes, 0x72, 1);
        w32(&mut bytes, 0x74, QUILL_DESCRIPTOR_LIST_END);
        descriptor(&mut bytes, 0x78, TEXT, 1, 0, 0x150, 6);

        w32(&mut bytes, 0x100, 0xaabb_ccdd);
        w32(&mut bytes, 0x104, 2);
        w32(&mut bytes, 0x108, 11);
        w32(&mut bytes, 0x10c, 22);
        w32(&mut bytes, 0x120, 2);
        w32(&mut bytes, 0x124, 8);
        bytes[0x128..0x12c].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        w32(&mut bytes, 0x12c, 2);
        w32(&mut bytes, 0x130, 1);
        bytes[0x150..0x156].copy_from_slice(&[b'A', 0, b'B', 0, b'C', 0]);
        (stream, bytes)
    }

    fn tcd_fixture() -> (StreamPath, Vec<u8>) {
        let (stream, mut bytes) = fixture();
        w16(&mut bytes, 0x72, 2);
        descriptor(&mut bytes, 0x90, TCD, 1, 0, 0x160, 20);
        w16(&mut bytes, 0x96, 0);

        w32(&mut bytes, 0x160, 0);
        w32(&mut bytes, 0x164, 0x1111_2222);
        w32(&mut bytes, 0x168, 0x3333_4444);
        w32(&mut bytes, 0x16c, 2);
        bytes[0x170..0x174].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        (stream, bytes)
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
    fn linked_overflow_and_raw_story_slices_are_preserved() {
        let (stream, bytes) = fixture();
        let parsed = parse_confirmed_story_catalog(stream.clone(), &bytes).unwrap();
        assert_eq!(parsed.descriptor_nodes.len(), 2);
        assert_eq!(parsed.descriptor_nodes[0].next.value, 0x70);
        assert_eq!(parsed.descriptor_nodes[0].descriptors[0].option_b.value, 7);
        assert_eq!(parsed.descriptor_nodes[0].descriptors[0].option_c.value, 9);
        assert_eq!(
            parsed.descriptor_nodes[0].descriptors[0].option_b.raw,
            vec![7, 0]
        );
        assert_eq!(parsed.syid.ids[0].value, QuillSyid(11));
        assert_eq!(parsed.strs.service_span.value, 8);
        assert_eq!(parsed.stories[0].utf16le, vec![b'A', 0, b'B', 0]);
        assert_eq!(parsed.stories[1].utf16le, vec![b'C', 0]);
        assert_eq!(
            parsed.stories[0].text_source,
            RawSpan {
                stream,
                offset: 0x150,
                len: 4
            }
        );
    }

    #[test]
    fn tcd_preserves_raw_boundaries_and_story_relation() {
        let (stream, bytes) = tcd_fixture();
        let parsed = parse_confirmed_story_catalog(stream.clone(), &bytes).unwrap();

        assert_eq!(parsed.tcd.len(), 1);
        let tcd = &parsed.tcd[0];
        assert_eq!(tcd.story_ordinal.value, 0);
        assert_eq!(tcd.story_syid.value, QuillSyid(11));
        assert_eq!(tcd.story_utf16_code_units.value, 2);
        assert_eq!(tcd.stored_cell_count_minus_one.value, 0);
        assert_eq!(tcd.header_word_1.value, 0x1111_2222);
        assert_eq!(tcd.header_word_2.value, 0x3333_4444);
        assert_eq!(
            tcd.cell_end_offsets_utf16
                .iter()
                .map(|value| value.value)
                .collect::<Vec<_>>(),
            vec![2]
        );
        assert_eq!(
            tcd.cell_end_offsets_utf16[0].source,
            RawSpan {
                stream: stream.clone(),
                offset: 0x16c,
                len: 4,
            }
        );
        assert_eq!(tcd.opaque_tail, vec![0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(
            tcd.opaque_tail_source,
            Some(RawSpan {
                stream,
                offset: 0x170,
                len: 4,
            })
        );
    }

    #[test]
    fn tcd_story_ordinal_must_index_syid_and_strs() {
        let (stream, mut bytes) = tcd_fixture();
        w16(&mut bytes, 0x96, 2);

        assert_eq!(
            parse_confirmed_story_catalog(stream, &bytes),
            Err(QuillStoryReadError::TcdStoryOrdinalOutOfBounds {
                story_ordinal: 2,
                story_count: 2,
            })
        );
    }

    #[test]
    fn tcd_cell_count_overflow_fails_closed() {
        let (stream, mut bytes) = tcd_fixture();
        w32(&mut bytes, 0x160, u32::MAX);

        assert_eq!(
            parse_confirmed_story_catalog(stream, &bytes),
            Err(QuillStoryReadError::TcdCellCountOverflow {
                stored_cell_count_minus_one: u32::MAX,
            })
        );
    }

    #[test]
    fn apache_sample_tcd_matches_table_story_and_raw_boundaries() {
        let pub_bytes = decode_base64(include_str!(
            "../../pub-reader/tests/fixtures/Sample.pub.b64"
        ));
        let quill = pub_cfb::read_stream_reader(
            std::io::Cursor::new(pub_bytes.as_slice()),
            "/Quill/QuillSub/CONTENTS",
        )
        .expect("read pinned Apache Sample.pub Quill stream");
        let parsed =
            parse_confirmed_story_catalog(StreamPath("/Quill/QuillSub/CONTENTS".into()), &quill)
                .expect("parse pinned Apache Sample.pub Quill");

        let tcd = parsed
            .tcd
            .iter()
            .find(|tcd| tcd.story_ordinal.value == 3)
            .expect("Sample.pub table TCD story ordinal 3");

        assert_eq!(tcd.story_syid.value, QuillSyid(6));
        assert_eq!(tcd.story_utf16_code_units.value, 80);
        assert_eq!(
            tcd.cell_end_offsets_utf16
                .iter()
                .map(|value| value.value)
                .collect::<Vec<_>>(),
            vec![15, 25, 39, 54, 66, 80]
        );
        assert!(tcd.opaque_tail.is_empty());
    }

    #[test]
    fn descriptor_cycle_fails_closed() {
        let stream = StreamPath("/Quill/QuillSub/CONTENTS".into());
        let mut bytes = vec![0; 0x40];
        w16(&mut bytes, 0x18, 0x18);
        w16(&mut bytes, 0x1a, 0);
        w32(&mut bytes, 0x1c, QUILL_DESCRIPTOR_LIST_ROOT_OFFSET);
        assert_eq!(
            parse_confirmed_story_catalog(stream, &bytes),
            Err(QuillStoryReadError::DescriptorListCycle {
                offset: QUILL_DESCRIPTOR_LIST_ROOT_OFFSET
            })
        );
    }

    #[test]
    fn story_count_mismatch_fails_closed() {
        let (stream, mut bytes) = fixture();
        w32(&mut bytes, 0x120, 1);
        assert_eq!(
            parse_confirmed_story_catalog(stream, &bytes),
            Err(QuillStoryReadError::StoryCountMismatch {
                syid_count: 2,
                strs_count: 1
            })
        );
    }

    #[test]
    fn text_length_mismatch_fails_closed() {
        let (stream, mut bytes) = fixture();
        w32(&mut bytes, 0x130, 2);
        assert_eq!(
            parse_confirmed_story_catalog(stream, &bytes),
            Err(QuillStoryReadError::TextLengthMismatch {
                expected_bytes: 8,
                actual_bytes: 6
            })
        );
    }

    #[test]
    fn next_descriptor_node_must_be_in_bounds() {
        let stream = StreamPath("/Quill/QuillSub/CONTENTS".into());
        let mut bytes = vec![0; 0x40];
        w16(&mut bytes, 0x18, 0x18);
        w16(&mut bytes, 0x1a, 0);
        w32(&mut bytes, 0x1c, 0x40);
        assert_eq!(
            parse_confirmed_story_catalog(stream, &bytes),
            Err(QuillStoryReadError::DescriptorListPointerOutOfBounds {
                offset: 0x40,
                stream_len: 0x40
            })
        );
    }
}
