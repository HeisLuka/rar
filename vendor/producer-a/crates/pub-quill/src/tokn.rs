use crate::story::{QuillChunkDescriptor, QuillStoryReadError};
use pub_core::{Decoded, QuillSyid, RawSpan, StreamPath};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const TOKN_PLC_TYPE: u32 = 12;
pub const TOKN_PROPERTY_STATE: u16 = 0x2200;
pub const TOKN_PROPERTY_TEXT_LENGTH: u16 = 0x2201;
pub const TOKN_PROPERTY_KIND: u16 = 0x2202;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillToknProperty {
    pub tag: Decoded<u16>,
    pub value: Decoded<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "form", rename_all = "snake_case")]
pub enum QuillToknPropertyBlock {
    Properties {
        source: RawSpan,
        length: Decoded<u32>,
        properties: Vec<QuillToknProperty>,
    },
    Opaque {
        source: RawSpan,
        length: Decoded<u32>,
        payload_source: RawSpan,
        payload: Vec<u8>,
    },
}

impl QuillToknPropertyBlock {
    pub fn source(&self) -> &RawSpan {
        match self {
            Self::Properties { source, .. } | Self::Opaque { source, .. } => source,
        }
    }

    pub fn property(&self, tag: u16) -> Option<&Decoded<u32>> {
        let Self::Properties { properties, .. } = self else {
            return None;
        };
        let mut matches = properties
            .iter()
            .filter(|property| property.tag.value == tag);
        let first = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(&first.value)
    }

    fn has_duplicate_property(&self, tag: u16) -> bool {
        let Self::Properties { properties, .. } = self else {
            return false;
        };
        properties
            .iter()
            .filter(|property| property.tag.value == tag)
            .nth(1)
            .is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillToknEffectiveToken {
    pub index: u32,
    pub start_utf16: Decoded<u32>,
    pub end_utf16: Decoded<u32>,
    pub state_raw: Option<u32>,
    pub text_length_utf16: Option<u32>,
    pub kind_raw: Option<u32>,
    pub attached_target_index: Option<i32>,
}

impl QuillToknEffectiveToken {
    pub fn kind_i32(&self) -> Option<i32> {
        self.kind_raw.map(|value| value as i32)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillToknTargetSectionHeader {
    pub source: RawSpan,
    pub remaining_payload_size: Decoded<u32>,
    pub count: Decoded<u32>,
    pub service_like: Decoded<u32>,
    pub reserved_a: Decoded<u32>,
    pub reserved_b: Decoded<u32>,
    pub offsets: Vec<Decoded<u32>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "form", rename_all = "snake_case")]
pub enum QuillToknTargetRecord {
    Utf16String {
        source: RawSpan,
        declared_units: Decoded<u16>,
        bytes_source: RawSpan,
        bytes: Vec<u8>,
        text: String,
    },
    CompactPayload {
        source: RawSpan,
        payload_units: Decoded<u16>,
        payload_source: RawSpan,
        payload: Vec<u8>,
        physical_target_value: Option<u32>,
    },
    Unknown {
        source: RawSpan,
        bytes: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillToknTargetSection {
    pub header: QuillToknTargetSectionHeader,
    pub records: Vec<QuillToknTargetRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillToknChunk {
    pub source: RawSpan,
    pub descriptor_source: RawSpan,
    pub story_ordinal: Decoded<u16>,
    pub story_syid: Decoded<QuillSyid>,
    pub plc_count: Decoded<u32>,
    pub plc_type: Decoded<u32>,
    /// Raw first DWORD after the PLC header. Historical consumers gave this
    /// hyperlink-specific meaning; the bounded reader deliberately does not.
    pub predata_service: Decoded<u32>,
    /// N token starts followed by one final story-local boundary.
    pub boundaries_utf16: Vec<Decoded<u32>>,
    pub first_phase: Vec<QuillToknPropertyBlock>,
    pub second_phase: Vec<QuillToknPropertyBlock>,
    pub effective_tokens: Vec<QuillToknEffectiveToken>,
    pub target_section: Option<QuillToknTargetSection>,
    pub opaque_tail_source: Option<RawSpan>,
    pub opaque_tail: Vec<u8>,
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
        Ok(Self { stream, bytes, position: start, limit: end })
    }

    fn position(&self) -> usize { self.position }
    fn remaining(&self) -> usize { self.limit.saturating_sub(self.position) }

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
        Ok((&self.bytes[start..end], span(self.stream.clone(), start, len)))
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
}

pub(crate) fn parse_tokn_chunks(
    stream: StreamPath,
    bytes: &[u8],
    descriptors: &[&QuillChunkDescriptor],
    syid_ids: &[Decoded<QuillSyid>],
) -> Result<Vec<QuillToknChunk>, QuillStoryReadError> {
    let mut chunks = Vec::new();
    for descriptor in descriptors
        .iter()
        .copied()
        .filter(|descriptor| descriptor.name.value == *b"TOKN")
    {
        chunks.push(parse_tokn_chunk(
            stream.clone(),
            bytes,
            descriptor,
            syid_ids,
        )?);
    }
    Ok(chunks)
}

fn parse_tokn_chunk(
    stream: StreamPath,
    bytes: &[u8],
    descriptor: &QuillChunkDescriptor,
    syid_ids: &[Decoded<QuillSyid>],
) -> Result<QuillToknChunk, QuillStoryReadError> {
    let ordinal = usize::from(descriptor.option_a.value);
    let story_syid = syid_ids
        .get(ordinal)
        .cloned()
        .ok_or(QuillStoryReadError::ToknStoryOrdinalOutOfBounds {
            story_ordinal: descriptor.option_a.value,
            story_count: u32::try_from(syid_ids.len()).unwrap_or(u32::MAX),
        })?;

    let (start, len) = chunk_range(bytes, descriptor)?;
    let mut cursor = Cursor::bounded(stream.clone(), bytes, start, len)?;
    let plc_count = cursor.u32()?;
    let plc_type = cursor.u32()?;
    if plc_type.value != TOKN_PLC_TYPE {
        return Err(QuillStoryReadError::ToknUnexpectedPlcType {
            found: plc_type.value,
        });
    }
    let count = usize::try_from(plc_count.value)
        .map_err(|_| QuillStoryReadError::ToknCountOverflow {
            count: plc_count.value,
        })?;
    if count > 100_000 {
        return Err(QuillStoryReadError::ToknCountOverflow {
            count: plc_count.value,
        });
    }

    let predata_service = cursor.u32()?;
    let mut boundaries_utf16 = Vec::with_capacity(count.saturating_add(1));
    for _ in 0..=count {
        boundaries_utf16.push(cursor.u32()?);
    }
    for pair in boundaries_utf16.windows(2) {
        if pair[1].value < pair[0].value {
            return Err(QuillStoryReadError::ToknNonMonotonicBoundary {
                previous: pair[0].value,
                next: pair[1].value,
            });
        }
    }

    let mut first_phase = Vec::with_capacity(count);
    for _ in 0..count {
        first_phase.push(parse_property_block(&mut cursor)?);
    }
    let mut second_phase = Vec::with_capacity(count);
    for _ in 0..count {
        second_phase.push(parse_property_block(&mut cursor)?);
    }

    let effective_tokens = effective_tokens(
        &first_phase,
        &second_phase,
        &boundaries_utf16,
    )?;

    let has_target_refs = effective_tokens
        .iter()
        .any(|token| token.attached_target_index.is_some_and(|value| value >= 0));

    let target_section = if has_target_refs && cursor.remaining() >= 20 {
        parse_target_section(&mut cursor, &effective_tokens)?
    } else {
        None
    };

    let (opaque_tail_source, opaque_tail) = if cursor.remaining() == 0 {
        (None, Vec::new())
    } else {
        let remaining = cursor.remaining();
        let (tail, source) = cursor.take(remaining)?;
        (Some(source), tail.to_vec())
    };

    Ok(QuillToknChunk {
        source: span(stream, start, len),
        descriptor_source: descriptor.source.clone(),
        story_ordinal: descriptor.option_a.clone(),
        story_syid,
        plc_count,
        plc_type,
        predata_service,
        boundaries_utf16,
        first_phase,
        second_phase,
        effective_tokens,
        target_section,
        opaque_tail_source,
        opaque_tail,
    })
}

fn effective_tokens(
    first_phase: &[QuillToknPropertyBlock],
    second_phase: &[QuillToknPropertyBlock],
    boundaries: &[Decoded<u32>],
) -> Result<Vec<QuillToknEffectiveToken>, QuillStoryReadError> {
    let mut state_raw = None;
    let mut text_length = None;
    let mut kind_raw = None;
    let mut result = Vec::with_capacity(first_phase.len());

    for (index, block) in first_phase.iter().enumerate() {
        if block.has_duplicate_property(TOKN_PROPERTY_STATE) {
            state_raw = None;
        } else if let Some(value) = block.property(TOKN_PROPERTY_STATE) {
            state_raw = Some(value.value);
        }
        if block.has_duplicate_property(TOKN_PROPERTY_TEXT_LENGTH) {
            text_length = None;
        } else if let Some(value) = block.property(TOKN_PROPERTY_TEXT_LENGTH) {
            text_length = Some(value.value);
        }
        if block.has_duplicate_property(TOKN_PROPERTY_KIND) {
            kind_raw = None;
        } else if let Some(value) = block.property(TOKN_PROPERTY_KIND) {
            kind_raw = Some(value.value);
        }

        let start = boundaries[index].clone();
        let end = boundaries[index + 1].clone();
        if let Some(length) = text_length {
            let expected_end = start
                .value
                .checked_add(length)
                .ok_or(QuillStoryReadError::ToknTokenSpanOverflow {
                    start: start.value,
                    length,
                })?;
            if expected_end > end.value {
                return Err(QuillStoryReadError::ToknTokenLengthExceedsBoundary {
                    start: start.value,
                    length,
                    boundary: end.value,
                });
            }
        }

        let attached_target_index = second_phase[index]
            .property(TOKN_PROPERTY_STATE)
            .map(|value| value.value as i32);

        result.push(QuillToknEffectiveToken {
            index: u32::try_from(index).map_err(|_| QuillStoryReadError::ToknCountOverflow {
                count: u32::MAX,
            })?,
            start_utf16: start,
            end_utf16: end,
            state_raw,
            text_length_utf16: text_length,
            kind_raw,
            attached_target_index,
        });
    }
    Ok(result)
}

fn parse_property_block(
    cursor: &mut Cursor<'_>,
) -> Result<QuillToknPropertyBlock, QuillStoryReadError> {
    let start = cursor.position();
    let length = cursor.u32()?;
    if length.value < 4 {
        return Err(QuillStoryReadError::ToknInvalidBlockLength {
            offset: length.source.offset,
            length: length.value,
        });
    }
    let payload_len = usize::try_from(length.value - 4)
        .map_err(|_| QuillStoryReadError::ToknInvalidBlockLength {
            offset: length.source.offset,
            length: length.value,
        })?;
    if payload_len > cursor.remaining() {
        return Err(QuillStoryReadError::TooShort {
            offset: cursor.position(),
            requested: payload_len,
            available: cursor.remaining(),
        });
    }

    if payload_len % 6 != 0 {
        let (payload, payload_source) = cursor.take(payload_len)?;
        return Ok(QuillToknPropertyBlock::Opaque {
            source: span(cursor.stream.clone(), start, usize::try_from(length.value).unwrap_or(0)),
            length,
            payload_source,
            payload: payload.to_vec(),
        });
    }

    let count = payload_len / 6;
    let mut properties = Vec::with_capacity(count);
    for _ in 0..count {
        properties.push(QuillToknProperty {
            tag: cursor.u16()?,
            value: cursor.u32()?,
        });
    }
    Ok(QuillToknPropertyBlock::Properties {
        source: span(cursor.stream.clone(), start, usize::try_from(length.value).unwrap_or(0)),
        length,
        properties,
    })
}

fn parse_target_section(
    cursor: &mut Cursor<'_>,
    tokens: &[QuillToknEffectiveToken],
) -> Result<Option<QuillToknTargetSection>, QuillStoryReadError> {
    let start = cursor.position();
    let remaining_payload_size = cursor.u32()?;
    let count = cursor.u32()?;
    let service_like = cursor.u32()?;
    let reserved_a = cursor.u32()?;
    let reserved_b = cursor.u32()?;

    let section_end = start
        .checked_add(20)
        .and_then(|value| value.checked_add(usize::try_from(remaining_payload_size.value).ok()?))
        .ok_or(QuillStoryReadError::ToknTargetSectionOverflow)?;
    if section_end != cursor.limit {
        // The target header equation is part of the grounded grammar. If it
        // does not hold, do not reinterpret the tail as a target section.
        cursor.position = start;
        return Ok(None);
    }

    let count_usize = usize::try_from(count.value)
        .map_err(|_| QuillStoryReadError::ToknTargetSectionOverflow)?;
    let nonnegative_refs = tokens
        .iter()
        .filter_map(|token| token.attached_target_index)
        .filter(|value| *value >= 0)
        .count();
    if nonnegative_refs != count_usize {
        cursor.position = start;
        return Ok(None);
    }

    let offsets_start = cursor.position();
    let mut offsets = Vec::with_capacity(count_usize);
    for _ in 0..count_usize {
        offsets.push(cursor.u32()?);
    }
    let minimum_offset = count_usize
        .checked_mul(4)
        .ok_or(QuillStoryReadError::ToknTargetSectionOverflow)?;
    if offsets
        .iter()
        .any(|offset| usize::try_from(offset.value).ok().is_none_or(|value| value < minimum_offset))
    {
        cursor.position = start;
        return Ok(None);
    }

    let mut token_kinds_by_target = BTreeMap::<usize, Vec<i32>>::new();
    for token in tokens {
        let Some(target) = token.attached_target_index else {
            continue;
        };
        let Ok(target) = usize::try_from(target) else {
            continue;
        };
        if let Some(kind) = token.kind_i32() {
            token_kinds_by_target.entry(target).or_default().push(kind);
        }
    }

    let base = offsets_start;
    let mut records = Vec::with_capacity(count_usize);
    for index in 0..count_usize {
        let record_start = base
            .checked_add(usize::try_from(offsets[index].value)
                .map_err(|_| QuillStoryReadError::ToknTargetSectionOverflow)?)
            .ok_or(QuillStoryReadError::ToknTargetSectionOverflow)?;
        let record_end = if index + 1 < count_usize {
            base.checked_add(
                usize::try_from(offsets[index + 1].value)
                    .map_err(|_| QuillStoryReadError::ToknTargetSectionOverflow)?,
            )
            .ok_or(QuillStoryReadError::ToknTargetSectionOverflow)?
        } else {
            section_end
        };
        if record_start > record_end || record_end > section_end {
            cursor.position = start;
            return Ok(None);
        }
        records.push(decode_target_record(
            cursor.stream.clone(),
            cursor.bytes,
            record_start,
            record_end,
            token_kinds_by_target.get(&index).map(Vec::as_slice).unwrap_or(&[]),
        ));
    }

    cursor.position = section_end;
    Ok(Some(QuillToknTargetSection {
        header: QuillToknTargetSectionHeader {
            source: span(cursor.stream.clone(), start, 20 + minimum_offset),
            remaining_payload_size,
            count,
            service_like,
            reserved_a,
            reserved_b,
            offsets,
        },
        records,
    }))
}

fn decode_target_record(
    stream: StreamPath,
    bytes: &[u8],
    start: usize,
    end: usize,
    referring_kinds: &[i32],
) -> QuillToknTargetRecord {
    let raw = &bytes[start..end];
    let source = span(stream.clone(), start, raw.len());
    if raw.len() < 2 {
        return QuillToknTargetRecord::Unknown {
            source,
            bytes: raw.to_vec(),
        };
    }

    let units = u16::from_le_bytes([raw[0], raw[1]]);
    let expected = 2usize.checked_add(usize::from(units).saturating_mul(2));
    if expected != Some(raw.len()) {
        return QuillToknTargetRecord::Unknown {
            source,
            bytes: raw.to_vec(),
        };
    }
    let declared_units = Decoded {
        value: units,
        source: span(stream.clone(), start, 2),
        raw: raw[..2].to_vec(),
    };
    let payload = raw[2..].to_vec();
    let payload_source = span(stream.clone(), start + 2, payload.len());

    if !referring_kinds.is_empty() && referring_kinds.iter().all(|kind| *kind == 3) {
        let physical_target_value = match (units, payload.as_slice()) {
            (1, [lo, hi]) => Some(u32::from(u16::from_le_bytes([*lo, *hi]))),
            (2, [b0, b1, b2, b3]) => Some(u32::from_le_bytes([*b0, *b1, *b2, *b3])),
            _ => None,
        };
        return QuillToknTargetRecord::CompactPayload {
            source,
            payload_units: declared_units,
            payload_source,
            payload,
            physical_target_value,
        };
    }

    let string_class = !referring_kinds.is_empty()
        && referring_kinds
            .iter()
            .all(|kind| matches!(*kind, 1 | 4));
    if string_class {
        let utf16 = payload
            .chunks_exact(2)
            .map(|word| u16::from_le_bytes([word[0], word[1]]))
            .collect::<Vec<_>>();
        if let Ok(text) = String::from_utf16(&utf16) {
            return QuillToknTargetRecord::Utf16String {
                source,
                declared_units,
                bytes_source: payload_source,
                bytes: payload,
                text,
            };
        }
    }

    QuillToknTargetRecord::Unknown {
        source,
        bytes: raw.to_vec(),
    }
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

fn span(stream: StreamPath, start: usize, len: usize) -> RawSpan {
    RawSpan {
        stream,
        offset: start as u64,
        len: len as u64,
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn w16(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn w32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn property_block(properties: &[(u16, u32)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        w32(
            &mut bytes,
            u32::try_from(4 + properties.len() * 6).unwrap(),
        );
        for (tag, value) in properties {
            w16(&mut bytes, *tag);
            w32(&mut bytes, *value);
        }
        bytes
    }

    fn type12_prefix(
        count: u32,
        service: u32,
        boundaries: &[u32],
    ) -> Vec<u8> {
        assert_eq!(boundaries.len(), usize::try_from(count).unwrap() + 1);
        let mut bytes = Vec::new();
        w32(&mut bytes, count);
        w32(&mut bytes, TOKN_PLC_TYPE);
        w32(&mut bytes, service);
        for boundary in boundaries {
            w32(&mut bytes, *boundary);
        }
        bytes
    }

    fn descriptor(len: usize, ordinal: u16) -> QuillChunkDescriptor {
        fn decoded_u16(value: u16, offset: u64) -> Decoded<u16> {
            Decoded {
                value,
                source: RawSpan {
                    stream: StreamPath("/Quill/QuillSub/CONTENTS".into()),
                    offset,
                    len: 2,
                },
                raw: value.to_le_bytes().to_vec(),
            }
        }
        fn decoded_u32(value: u32, offset: u64) -> Decoded<u32> {
            Decoded {
                value,
                source: RawSpan {
                    stream: StreamPath("/Quill/QuillSub/CONTENTS".into()),
                    offset,
                    len: 4,
                },
                raw: value.to_le_bytes().to_vec(),
            }
        }
        fn decoded_name(value: [u8; 4], offset: u64) -> Decoded<[u8; 4]> {
            Decoded {
                value,
                source: RawSpan {
                    stream: StreamPath("/Quill/QuillSub/CONTENTS".into()),
                    offset,
                    len: 4,
                },
                raw: value.to_vec(),
            }
        }

        QuillChunkDescriptor {
            source: RawSpan {
                stream: StreamPath("/Quill/QuillSub/CONTENTS".into()),
                offset: 0x40,
                len: 24,
            },
            presence_marker: decoded_u16(0x18, 0x40),
            name: decoded_name(*b"TOKN", 0x42),
            option_a: decoded_u16(ordinal, 0x46),
            option_b: decoded_u16(0, 0x48),
            option_c: decoded_u16(0, 0x4a),
            bit_type: decoded_name(*b"PLC ", 0x4c),
            data_offset: decoded_u32(0, 0x50),
            data_length: decoded_u32(u32::try_from(len).unwrap(), 0x54),
        }
    }

    fn syid(value: u32) -> Decoded<QuillSyid> {
        Decoded {
            value: QuillSyid(value),
            source: RawSpan {
                stream: StreamPath("/Quill/QuillSub/CONTENTS".into()),
                offset: 0x100,
                len: 4,
            },
            raw: value.to_le_bytes().to_vec(),
        }
    }

    #[test]
    fn compact_n3_60685_shape_parses_as_generic_two_phase_tokn() {
        let mut bytes = type12_prefix(3, 0x0001_ffff, &[262, 273, 1870, 1882]);
        bytes.extend(property_block(&[
            (TOKN_PROPERTY_STATE, 0x0600),
            (TOKN_PROPERTY_TEXT_LENGTH, 9),
            (TOKN_PROPERTY_KIND, 7),
        ]));
        bytes.extend(property_block(&[(TOKN_PROPERTY_TEXT_LENGTH, 15)]));
        bytes.extend(property_block(&[(TOKN_PROPERTY_TEXT_LENGTH, 10)]));
        bytes.extend(property_block(&[(TOKN_PROPERTY_STATE, u32::MAX)]));
        bytes.extend(property_block(&[]));
        bytes.extend(property_block(&[]));
        assert_eq!(bytes.len(), 88);

        let parsed = parse_tokn_chunks(
            StreamPath("/Quill/QuillSub/CONTENTS".into()),
            &bytes,
            &[&descriptor(bytes.len(), 0)],
            &[syid(11)],
        )
        .unwrap();

        assert_eq!(parsed.len(), 1);
        let tokn = &parsed[0];
        assert_eq!(tokn.story_ordinal.value, 0);
        assert_eq!(tokn.story_syid.value, QuillSyid(11));
        assert_eq!(tokn.first_phase.len(), 3);
        assert_eq!(tokn.second_phase.len(), 3);
        assert!(tokn.target_section.is_none());
        assert!(tokn.opaque_tail.is_empty());
        assert_eq!(
            tokn.effective_tokens
                .iter()
                .map(|token| (
                    token.start_utf16.value,
                    token.text_length_utf16,
                    token.kind_i32(),
                    token.attached_target_index,
                ))
                .collect::<Vec<_>>(),
            vec![
                (262, Some(9), Some(7), Some(-1)),
                (273, Some(15), Some(7), None),
                (1870, Some(10), Some(7), None),
            ]
        );
    }

    #[test]
    fn controlled_url_shape_uses_same_tokn_grammar_and_utf16_target_record() {
        let url = "http://poi.apache.org/";
        let url_utf16 = url.encode_utf16().collect::<Vec<_>>();

        let mut bytes = type12_prefix(1, 0x0001_ffff, &[10, 14]);
        bytes.extend(property_block(&[
            (TOKN_PROPERTY_STATE, 0x08c0),
            (TOKN_PROPERTY_TEXT_LENGTH, 4),
            (TOKN_PROPERTY_KIND, 1),
        ]));
        bytes.extend(property_block(&[(TOKN_PROPERTY_STATE, 0)]));

        let record_len = 2 + url_utf16.len() * 2;
        let remaining = 4 + record_len;
        w32(&mut bytes, u32::try_from(remaining).unwrap());
        w32(&mut bytes, 1);
        w32(&mut bytes, 0x1234_5678);
        w32(&mut bytes, 0);
        w32(&mut bytes, 0);
        w32(&mut bytes, 4);
        w16(&mut bytes, u16::try_from(url_utf16.len()).unwrap());
        for word in &url_utf16 {
            w16(&mut bytes, *word);
        }

        let parsed = parse_tokn_chunks(
            StreamPath("/Quill/QuillSub/CONTENTS".into()),
            &bytes,
            &[&descriptor(bytes.len(), 0)],
            &[syid(22)],
        )
        .unwrap();
        let tokn = &parsed[0];
        let target = tokn.target_section.as_ref().expect("target section");
        assert_eq!(target.header.count.value, 1);
        assert_eq!(target.header.service_like.value, 0x1234_5678);
        assert_eq!(tokn.effective_tokens[0].kind_i32(), Some(1));
        assert_eq!(tokn.effective_tokens[0].attached_target_index, Some(0));
        assert!(matches!(
            &target.records[0],
            QuillToknTargetRecord::Utf16String { text, .. } if text == url
        ));
        assert!(tokn.opaque_tail.is_empty());
    }

    fn parse_internal_page_target(units: u16, payload: &[u8]) -> QuillToknTargetRecord {
        let mut bytes = type12_prefix(1, 0x0001_ffff, &[20, 21]);
        bytes.extend(property_block(&[
            (TOKN_PROPERTY_STATE, 0x08c0),
            (TOKN_PROPERTY_TEXT_LENGTH, 1),
            (TOKN_PROPERTY_KIND, 3),
        ]));
        bytes.extend(property_block(&[(TOKN_PROPERTY_STATE, 0)]));

        let record_len = 2 + payload.len();
        w32(&mut bytes, u32::try_from(4 + record_len).unwrap());
        w32(&mut bytes, 1);
        w32(&mut bytes, 0);
        w32(&mut bytes, 0);
        w32(&mut bytes, 0);
        w32(&mut bytes, 4);
        w16(&mut bytes, units);
        bytes.extend_from_slice(payload);

        let parsed = parse_tokn_chunks(
            StreamPath("/Quill/QuillSub/CONTENTS".into()),
            &bytes,
            &[&descriptor(bytes.len(), 0)],
            &[syid(33)],
        )
        .unwrap();
        parsed[0].target_section.as_ref().unwrap().records[0].clone()
    }

    #[test]
    fn kind3_units1_internal_page_target_preserves_u16_physical_seqnum() {
        let target = parse_internal_page_target(1, &266_u16.to_le_bytes());
        assert!(matches!(
            target,
            QuillToknTargetRecord::CompactPayload {
                payload_units,
                payload,
                physical_target_value: Some(266),
                ..
            } if payload_units.value == 1 && payload == 266_u16.to_le_bytes().to_vec()
        ));
    }

    #[test]
    fn kind3_units2_internal_page_target_preserves_u32_physical_seqnum() {
        let target = parse_internal_page_target(2, &266_u32.to_le_bytes());
        assert!(matches!(
            target,
            QuillToknTargetRecord::CompactPayload {
                payload_units,
                payload,
                physical_target_value: Some(266),
                ..
            } if payload_units.value == 2 && payload == 266_u32.to_le_bytes().to_vec()
        ));
    }

    #[test]
    fn malformed_property_block_body_is_preserved_opaque_not_guessed() {
        let mut bytes = type12_prefix(1, 0, &[0, 1]);
        w32(&mut bytes, 5);
        bytes.push(0xaa);
        bytes.extend(property_block(&[]));

        let parsed = parse_tokn_chunks(
            StreamPath("/Quill/QuillSub/CONTENTS".into()),
            &bytes,
            &[&descriptor(bytes.len(), 0)],
            &[syid(44)],
        )
        .unwrap();
        assert!(matches!(
            parsed[0].first_phase[0],
            QuillToknPropertyBlock::Opaque { .. }
        ));
    }


    #[test]
    fn duplicate_known_property_clears_effective_value_instead_of_inheriting() {
        let mut bytes = type12_prefix(2, 0, &[0, 5, 10]);
        bytes.extend(property_block(&[
            (TOKN_PROPERTY_TEXT_LENGTH, 5),
            (TOKN_PROPERTY_KIND, 7),
        ]));
        bytes.extend(property_block(&[
            (TOKN_PROPERTY_TEXT_LENGTH, 4),
            (TOKN_PROPERTY_TEXT_LENGTH, 5),
        ]));
        bytes.extend(property_block(&[]));
        bytes.extend(property_block(&[]));

        let parsed = parse_tokn_chunks(
            StreamPath("/Quill/QuillSub/CONTENTS".into()),
            &bytes,
            &[&descriptor(bytes.len(), 0)],
            &[syid(66)],
        )
        .unwrap();

        assert_eq!(parsed[0].effective_tokens[0].text_length_utf16, Some(5));
        assert_eq!(parsed[0].effective_tokens[1].text_length_utf16, None);
        assert_eq!(parsed[0].effective_tokens[1].kind_i32(), Some(7));
    }

    #[test]
    fn unknown_kind_target_is_not_guessed_as_string_even_when_utf16_is_valid() {
        let mut bytes = type12_prefix(1, 0, &[0, 1]);
        bytes.extend(property_block(&[
            (TOKN_PROPERTY_TEXT_LENGTH, 1),
            (TOKN_PROPERTY_KIND, 99),
        ]));
        bytes.extend(property_block(&[(TOKN_PROPERTY_STATE, 0)]));

        w32(&mut bytes, 8);
        w32(&mut bytes, 1);
        w32(&mut bytes, 0);
        w32(&mut bytes, 0);
        w32(&mut bytes, 0);
        w32(&mut bytes, 4);
        w16(&mut bytes, 1);
        w16(&mut bytes, b'X' as u16);

        let parsed = parse_tokn_chunks(
            StreamPath("/Quill/QuillSub/CONTENTS".into()),
            &bytes,
            &[&descriptor(bytes.len(), 0)],
            &[syid(77)],
        )
        .unwrap();

        let target = parsed[0].target_section.as_ref().unwrap();
        assert!(matches!(
            target.records[0],
            QuillToknTargetRecord::Unknown { .. }
        ));
    }

    #[test]
    fn malformed_target_section_is_preserved_as_exact_opaque_tail() {
        let mut bytes = type12_prefix(1, 0, &[0, 1]);
        bytes.extend(property_block(&[
            (TOKN_PROPERTY_TEXT_LENGTH, 1),
            (TOKN_PROPERTY_KIND, 1),
        ]));
        bytes.extend(property_block(&[(TOKN_PROPERTY_STATE, 0)]));

        let tail_start = bytes.len();
        // Deliberately false size equation. The reader must not guess a target
        // section from these bytes.
        w32(&mut bytes, 999);
        w32(&mut bytes, 1);
        w32(&mut bytes, 0xdead_beef);
        w32(&mut bytes, 0);
        w32(&mut bytes, 0);
        w32(&mut bytes, 4);
        w16(&mut bytes, 1);
        w16(&mut bytes, b'X' as u16);

        let parsed = parse_tokn_chunks(
            StreamPath("/Quill/QuillSub/CONTENTS".into()),
            &bytes,
            &[&descriptor(bytes.len(), 0)],
            &[syid(55)],
        )
        .unwrap();

        let tokn = &parsed[0];
        assert!(tokn.target_section.is_none());
        assert_eq!(
            tokn.opaque_tail_source,
            Some(RawSpan {
                stream: StreamPath("/Quill/QuillSub/CONTENTS".into()),
                offset: tail_start as u64,
                len: (bytes.len() - tail_start) as u64,
            })
        );
        assert_eq!(tokn.opaque_tail, bytes[tail_start..].to_vec());
    }


    #[test]
    fn descriptor_ordinal_joins_exact_parallel_syid() {
        let mut bytes = type12_prefix(1, 0, &[0, 1]);
        bytes.extend(property_block(&[
            (TOKN_PROPERTY_TEXT_LENGTH, 1),
            (TOKN_PROPERTY_KIND, 7),
        ]));
        bytes.extend(property_block(&[]));

        let descriptor = descriptor(bytes.len(), 1);
        let parsed = parse_tokn_chunks(
            StreamPath("/Quill/QuillSub/CONTENTS".into()),
            &bytes,
            &[&descriptor],
            &[syid(11), syid(22)],
        )
        .unwrap();

        assert_eq!(parsed[0].story_ordinal.value, 1);
        assert_eq!(parsed[0].story_syid.value, QuillSyid(22));
    }
}
