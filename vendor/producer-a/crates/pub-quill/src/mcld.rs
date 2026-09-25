use crate::QuillDescriptorListNode;
use pub_core::{Decoded, RawSpan, StreamPath};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

const MCLD: [u8; 4] = *b"MCLD";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillMcldChunk {
    pub source: RawSpan,
    pub record_count: Decoded<u32>,
    pub record_id_count: Decoded<u32>,
    pub record_ids: Vec<Decoded<u32>>,
    pub records: Vec<QuillMcldRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillMcldRecord {
    pub record_id: u32,
    pub source: RawSpan,
    pub header_source: RawSpan,
    pub child_count: Decoded<u32>,
    pub children: Vec<QuillMcldChild>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillMcldChild {
    pub source: RawSpan,
    pub fields: Vec<QuillMcldField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillMcldField {
    pub id: u8,
    pub wire_type: u8,
    pub source: RawSpan,
    pub value: QuillMcldFieldValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum QuillMcldFieldValue {
    U32(u32),
    U16(u16),
    True,
    False,
    OpaqueNested(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillMcldConsensusU32 {
    pub value: u32,
    pub sources: Vec<RawSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuillMcldTableMetrics {
    pub record_id: u32,
    pub child_count: u32,
    pub cell_width_emu: QuillMcldConsensusU32,
    pub row_pitch_emu: QuillMcldConsensusU32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuillMcldReadError {
    MissingMcldDescriptor,
    DuplicateMcldDescriptor,
    ChunkOutOfBounds {
        offset: u32,
        length: u32,
        stream_len: usize,
    },
    TooShort {
        offset: usize,
        requested: usize,
        available: usize,
    },
    RecordCountMismatch {
        record_count: u32,
        record_id_count: u32,
    },
    DuplicateRecordId {
        record_id: u32,
    },
    InvalidSizedBlock {
        offset: u64,
        declared_size: u32,
        minimum_size: u32,
    },
    SizedBlockOutOfBounds {
        offset: u64,
        declared_size: u32,
        remaining: usize,
    },
    UnsupportedFieldType {
        record_id: u32,
        child_index: u32,
        field_id: u8,
        wire_type: u8,
    },
    InvalidNestedSize {
        record_id: u32,
        child_index: u32,
        field_id: u8,
        declared_size: u32,
    },
    TrailingChunkBytes {
        remaining: usize,
    },
    RecordIdNotFound {
        record_id: u32,
    },
    MissingRequiredField {
        record_id: u32,
        child_index: u32,
        field_id: u8,
    },
    DuplicateRequiredField {
        record_id: u32,
        child_index: u32,
        field_id: u8,
    },
    RequiredFieldWrongType {
        record_id: u32,
        child_index: u32,
        field_id: u8,
        wire_type: u8,
    },
    NonUniformRequiredField {
        record_id: u32,
        field_id: u8,
        expected: u32,
        found: u32,
        child_index: u32,
    },
}

impl fmt::Display for QuillMcldReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for QuillMcldReadError {}

/// Parses the bounded modern MCLD record/child grammar observed in grounded
/// Publisher fixtures.
///
/// The outer layout is:
/// - u32 record_count
/// - u32 record_id_count
/// - u32 record_ids[record_id_count]
/// - record bodies in ordinal order, keyed by the parallel record_id table
///
/// Each record begins with a sized header block, followed by u32 child_count.
/// Each child is another sized block. Sizes include their own u32 size field.
/// Child property types supported here are only the grounded wire forms needed
/// to walk the complete modern Sample payload without byte-pattern scanning.
pub fn parse_bounded_mcld(
    stream: StreamPath,
    quill_bytes: &[u8],
    descriptor_nodes: &[QuillDescriptorListNode],
) -> Result<QuillMcldChunk, QuillMcldReadError> {
    let descriptor = unique_mcld_descriptor(descriptor_nodes)?;
    let start = usize::try_from(descriptor.data_offset.value).map_err(|_| {
        QuillMcldReadError::ChunkOutOfBounds {
            offset: descriptor.data_offset.value,
            length: descriptor.data_length.value,
            stream_len: quill_bytes.len(),
        }
    })?;
    let len = usize::try_from(descriptor.data_length.value).map_err(|_| {
        QuillMcldReadError::ChunkOutOfBounds {
            offset: descriptor.data_offset.value,
            length: descriptor.data_length.value,
            stream_len: quill_bytes.len(),
        }
    })?;
    let end = start
        .checked_add(len)
        .ok_or(QuillMcldReadError::ChunkOutOfBounds {
            offset: descriptor.data_offset.value,
            length: descriptor.data_length.value,
            stream_len: quill_bytes.len(),
        })?;
    let bytes = quill_bytes
        .get(start..end)
        .ok_or(QuillMcldReadError::ChunkOutOfBounds {
            offset: descriptor.data_offset.value,
            length: descriptor.data_length.value,
            stream_len: quill_bytes.len(),
        })?;

    let mut cursor = Cursor::new(stream.clone(), bytes, start);
    let record_count = cursor.read_u32()?;
    let record_id_count = cursor.read_u32()?;
    if record_count.value != record_id_count.value {
        return Err(QuillMcldReadError::RecordCountMismatch {
            record_count: record_count.value,
            record_id_count: record_id_count.value,
        });
    }

    let mut record_ids = Vec::with_capacity(record_id_count.value as usize);
    let mut seen = BTreeSet::new();
    for _ in 0..record_id_count.value {
        let record_id = cursor.read_u32()?;
        if !seen.insert(record_id.value) {
            return Err(QuillMcldReadError::DuplicateRecordId {
                record_id: record_id.value,
            });
        }
        record_ids.push(record_id);
    }

    let mut records = Vec::with_capacity(record_count.value as usize);
    for record_id in &record_ids {
        records.push(parse_record(&mut cursor, record_id.value)?);
    }

    if cursor.remaining() != 0 {
        return Err(QuillMcldReadError::TrailingChunkBytes {
            remaining: cursor.remaining(),
        });
    }

    Ok(QuillMcldChunk {
        source: span(stream, start, len),
        record_count,
        record_id_count,
        record_ids,
        records,
    })
}

/// Promotes only the two grounded uniform table metrics from one keyed MCLD
/// record. Every child must contain exactly one u32 field 0x04 and 0x05 and all
/// children must agree; disagreement is an error, never a coercion.
pub fn bounded_mcld_table_metrics(
    mcld: &QuillMcldChunk,
    record_id: u32,
) -> Result<QuillMcldTableMetrics, QuillMcldReadError> {
    let record = mcld
        .records
        .iter()
        .find(|record| record.record_id == record_id)
        .ok_or(QuillMcldReadError::RecordIdNotFound { record_id })?;

    let mut widths = Vec::with_capacity(record.children.len());
    let mut pitches = Vec::with_capacity(record.children.len());

    for (child_index, child) in record.children.iter().enumerate() {
        let child_index = u32::try_from(child_index).unwrap_or(u32::MAX);
        widths.push(required_u32_field(record_id, child_index, child, 0x04)?);
        pitches.push(required_u32_field(record_id, child_index, child, 0x05)?);
    }

    let width = consensus(record_id, 0x04, &widths)?;
    let pitch = consensus(record_id, 0x05, &pitches)?;

    Ok(QuillMcldTableMetrics {
        record_id,
        child_count: record.child_count.value,
        cell_width_emu: width,
        row_pitch_emu: pitch,
    })
}

fn unique_mcld_descriptor(
    nodes: &[QuillDescriptorListNode],
) -> Result<&crate::QuillChunkDescriptor, QuillMcldReadError> {
    let mut found = nodes
        .iter()
        .flat_map(|node| node.descriptors.iter())
        .filter(|descriptor| descriptor.name.value == MCLD);
    let first = found
        .next()
        .ok_or(QuillMcldReadError::MissingMcldDescriptor)?;
    if found.next().is_some() {
        return Err(QuillMcldReadError::DuplicateMcldDescriptor);
    }
    Ok(first)
}

fn parse_record(
    cursor: &mut Cursor<'_>,
    record_id: u32,
) -> Result<QuillMcldRecord, QuillMcldReadError> {
    let record_start = cursor.position();
    let header_source = cursor.skip_sized_block(4)?;
    let child_count = cursor.read_u32()?;
    let mut children = Vec::with_capacity(child_count.value as usize);

    for child_index in 0..child_count.value {
        children.push(parse_child(cursor, record_id, child_index)?);
    }

    let record_end = cursor.position();
    Ok(QuillMcldRecord {
        record_id,
        source: span(
            cursor.stream.clone(),
            record_start,
            record_end - record_start,
        ),
        header_source,
        child_count,
        children,
    })
}

fn parse_child(
    cursor: &mut Cursor<'_>,
    record_id: u32,
    child_index: u32,
) -> Result<QuillMcldChild, QuillMcldReadError> {
    let child_start = cursor.position();
    let declared_size = cursor.peek_u32()?;
    if declared_size < 4 {
        return Err(QuillMcldReadError::InvalidSizedBlock {
            offset: child_start as u64,
            declared_size,
            minimum_size: 4,
        });
    }
    let declared_size =
        usize::try_from(declared_size).map_err(|_| QuillMcldReadError::SizedBlockOutOfBounds {
            offset: child_start as u64,
            declared_size,
            remaining: cursor.remaining(),
        })?;
    if declared_size > cursor.remaining() {
        return Err(QuillMcldReadError::SizedBlockOutOfBounds {
            offset: child_start as u64,
            declared_size: u32::try_from(declared_size).unwrap_or(u32::MAX),
            remaining: cursor.remaining(),
        });
    }

    let child_end = child_start + declared_size;
    cursor.read_u32()?;
    let mut fields = Vec::new();
    while cursor.position() < child_end {
        fields.push(parse_field(cursor, record_id, child_index, child_end)?);
    }
    if cursor.position() != child_end {
        return Err(QuillMcldReadError::SizedBlockOutOfBounds {
            offset: child_start as u64,
            declared_size: u32::try_from(declared_size).unwrap_or(u32::MAX),
            remaining: cursor.remaining(),
        });
    }

    Ok(QuillMcldChild {
        source: span(cursor.stream.clone(), child_start, declared_size),
        fields,
    })
}

fn parse_field(
    cursor: &mut Cursor<'_>,
    record_id: u32,
    child_index: u32,
    child_end: usize,
) -> Result<QuillMcldField, QuillMcldReadError> {
    let field_start = cursor.position();
    let id = cursor.read_u8_bounded(child_end)?;
    let wire_type = cursor.read_u8_bounded(child_end)?;

    let value = match wire_type {
        0x22 => QuillMcldFieldValue::U32(cursor.read_u32_bounded(child_end)?),
        0x12 | 0x1a => QuillMcldFieldValue::U16(cursor.read_u16_bounded(child_end)?),
        0x0a => QuillMcldFieldValue::True,
        0x02 => QuillMcldFieldValue::False,
        0x8a => {
            let nested_start = cursor.position();
            let declared_size = cursor.read_u32_bounded(child_end)?;
            if declared_size < 4 {
                return Err(QuillMcldReadError::InvalidNestedSize {
                    record_id,
                    child_index,
                    field_id: id,
                    declared_size,
                });
            }
            let payload_len = usize::try_from(declared_size - 4).map_err(|_| {
                QuillMcldReadError::InvalidNestedSize {
                    record_id,
                    child_index,
                    field_id: id,
                    declared_size,
                }
            })?;
            let payload = cursor.read_bytes_bounded(child_end, payload_len)?.to_vec();
            let _nested_source = span(
                cursor.stream.clone(),
                nested_start,
                usize::try_from(declared_size).unwrap_or(usize::MAX),
            );
            QuillMcldFieldValue::OpaqueNested(payload)
        }
        _ => {
            return Err(QuillMcldReadError::UnsupportedFieldType {
                record_id,
                child_index,
                field_id: id,
                wire_type,
            });
        }
    };

    let field_end = cursor.position();
    Ok(QuillMcldField {
        id,
        wire_type,
        source: span(cursor.stream.clone(), field_start, field_end - field_start),
        value,
    })
}

fn required_u32_field(
    record_id: u32,
    child_index: u32,
    child: &QuillMcldChild,
    field_id: u8,
) -> Result<(u32, RawSpan), QuillMcldReadError> {
    let mut found = child.fields.iter().filter(|field| field.id == field_id);
    let field = found
        .next()
        .ok_or(QuillMcldReadError::MissingRequiredField {
            record_id,
            child_index,
            field_id,
        })?;
    if found.next().is_some() {
        return Err(QuillMcldReadError::DuplicateRequiredField {
            record_id,
            child_index,
            field_id,
        });
    }
    match field.value {
        QuillMcldFieldValue::U32(value) if field.wire_type == 0x22 => {
            Ok((value, field.source.clone()))
        }
        _ => Err(QuillMcldReadError::RequiredFieldWrongType {
            record_id,
            child_index,
            field_id,
            wire_type: field.wire_type,
        }),
    }
}

fn consensus(
    record_id: u32,
    field_id: u8,
    values: &[(u32, RawSpan)],
) -> Result<QuillMcldConsensusU32, QuillMcldReadError> {
    let Some((expected, _)) = values.first() else {
        return Err(QuillMcldReadError::MissingRequiredField {
            record_id,
            child_index: 0,
            field_id,
        });
    };
    for (child_index, (found, _)) in values.iter().enumerate().skip(1) {
        if found != expected {
            return Err(QuillMcldReadError::NonUniformRequiredField {
                record_id,
                field_id,
                expected: *expected,
                found: *found,
                child_index: u32::try_from(child_index).unwrap_or(u32::MAX),
            });
        }
    }

    Ok(QuillMcldConsensusU32 {
        value: *expected,
        sources: values.iter().map(|(_, source)| source.clone()).collect(),
    })
}

struct Cursor<'a> {
    stream: StreamPath,
    bytes: &'a [u8],
    base: usize,
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(stream: StreamPath, bytes: &'a [u8], base: usize) -> Self {
        Self {
            stream,
            bytes,
            base,
            position: 0,
        }
    }

    fn position(&self) -> usize {
        self.base + self.position
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    fn peek_u32(&self) -> Result<u32, QuillMcldReadError> {
        let bytes = self.bytes.get(self.position..self.position + 4).ok_or(
            QuillMcldReadError::TooShort {
                offset: self.position(),
                requested: 4,
                available: self.remaining(),
            },
        )?;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn read_u32(&mut self) -> Result<Decoded<u32>, QuillMcldReadError> {
        let start = self.position();
        let bytes = self.read_bytes(4)?;
        Ok(Decoded {
            value: u32::from_le_bytes(bytes.try_into().unwrap()),
            source: span(self.stream.clone(), start, 4),
            raw: bytes.to_vec(),
        })
    }

    fn skip_sized_block(&mut self, minimum_size: u32) -> Result<RawSpan, QuillMcldReadError> {
        let start = self.position();
        let declared_size = self.peek_u32()?;
        if declared_size < minimum_size {
            return Err(QuillMcldReadError::InvalidSizedBlock {
                offset: start as u64,
                declared_size,
                minimum_size,
            });
        }
        let size = usize::try_from(declared_size).map_err(|_| {
            QuillMcldReadError::SizedBlockOutOfBounds {
                offset: start as u64,
                declared_size,
                remaining: self.remaining(),
            }
        })?;
        if size > self.remaining() {
            return Err(QuillMcldReadError::SizedBlockOutOfBounds {
                offset: start as u64,
                declared_size,
                remaining: self.remaining(),
            });
        }
        self.read_bytes(size)?;
        Ok(span(self.stream.clone(), start, size))
    }

    fn read_u8_bounded(&mut self, end: usize) -> Result<u8, QuillMcldReadError> {
        Ok(self.read_bytes_bounded(end, 1)?[0])
    }

    fn read_u16_bounded(&mut self, end: usize) -> Result<u16, QuillMcldReadError> {
        let bytes = self.read_bytes_bounded(end, 2)?;
        Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn read_u32_bounded(&mut self, end: usize) -> Result<u32, QuillMcldReadError> {
        let bytes = self.read_bytes_bounded(end, 4)?;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn read_bytes_bounded(
        &mut self,
        end: usize,
        len: usize,
    ) -> Result<&'a [u8], QuillMcldReadError> {
        let start = self.position();
        let absolute_end = start.checked_add(len).ok_or(QuillMcldReadError::TooShort {
            offset: start,
            requested: len,
            available: end.saturating_sub(start),
        })?;
        if absolute_end > end {
            return Err(QuillMcldReadError::TooShort {
                offset: start,
                requested: len,
                available: end.saturating_sub(start),
            });
        }
        self.read_bytes(len)
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], QuillMcldReadError> {
        let start = self.position;
        let end = start.checked_add(len).ok_or(QuillMcldReadError::TooShort {
            offset: self.position(),
            requested: len,
            available: self.remaining(),
        })?;
        let bytes = self
            .bytes
            .get(start..end)
            .ok_or(QuillMcldReadError::TooShort {
                offset: self.position(),
                requested: len,
                available: self.remaining(),
            })?;
        self.position = end;
        Ok(bytes)
    }
}

fn span(stream: StreamPath, offset: usize, len: usize) -> RawSpan {
    RawSpan {
        stream,
        offset: offset as u64,
        len: len as u64,
    }
}
