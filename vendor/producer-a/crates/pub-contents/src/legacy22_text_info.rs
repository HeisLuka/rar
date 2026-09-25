use crate::{ContentsFamily, ContentsReadError, detect_family};
use pub_core::{RawSpan, StreamPath};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const LEGACY_0X22_TEXT_INFO_CHUNK_TYPE: u16 = 0x0016;

const LEGACY_0X22_TRAILER_POINTER_OFFSET: usize = 0x16;
const LEGACY_DIRECTORY_ENTRY_SIZE: usize = 10;
const LEGACY_TEXT_INFO_MIN_HEADER_SIZE: usize = 10;
const LEGACY_TEXT_LIST_HEADER_SIZE: usize = 10;
const LEGACY_TEXT_LIST_EXTRA_HEADER_SIZE: usize = 12;
const LEGACY_TEXT_INFO_RECORD_SIZE: u16 = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22TextInfoOwnerEnd {
    /// Contents owner/object id stored by the low-family owner/frame index.
    /// Historical Publisher consumers reuse this id as the effective text key
    /// when this explicit boundary is present.
    pub owner_id: u16,
    /// Inclusive owner/frame boundary relative to the beginning of the
    /// legacy text block, matching the historical `offset - 1` representation.
    pub relative_end_offset: u32,
    pub owner_id_source: RawSpan,
    pub end_offset_source: RawSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22TextInfoMap {
    pub text_info_chunk_id: u16,
    pub text_info_chunk_source: RawSpan,
    pub target_chunk_id: u16,
    pub target_chunk_id_source: RawSpan,
    pub list_count: u16,
    pub list_count_source: RawSpan,
    pub record_size: u16,
    pub record_size_source: RawSpan,
    pub ends: Vec<Legacy0x22TextInfoOwnerEnd>,
}

impl Legacy0x22TextInfoMap {
    pub fn end_for_owner(&self, owner_id: u16) -> Option<&Legacy0x22TextInfoOwnerEnd> {
        self.ends.iter().find(|end| end.owner_id == owner_id)
    }

    pub fn owner_boundary_at(
        &self,
        relative_end_offset: u32,
    ) -> Option<&Legacy0x22TextInfoOwnerEnd> {
        self.ends
            .iter()
            .find(|end| end.relative_end_offset == relative_end_offset)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Legacy0x22TextInfoReadError {
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
    MultipleTextInfoChunks {
        chunk_ids: Vec<u16>,
    },
    TextInfoHeaderTooShort {
        chunk_id: u16,
        chunk_offset: u32,
        data_offset: u32,
    },
    MissingTargetChunk {
        text_info_chunk_id: u16,
        target_chunk_id: u16,
    },
    TargetHeaderTooShort {
        chunk_id: u16,
        chunk_offset: u32,
    },
    ListHeaderOutOfBounds {
        chunk_id: u16,
        list_offset: u32,
        chunk_end: u32,
    },
    UnsupportedExtendedListHeader {
        chunk_id: u16,
        count: u16,
        max_count: u16,
    },
    UnexpectedRecordSize {
        chunk_id: u16,
        found: u16,
    },
    ListPayloadOutOfBounds {
        chunk_id: u16,
        count: u16,
        record_size: u16,
        chunk_end: u32,
    },
    PositionDecreased {
        chunk_id: u16,
        index: u16,
        previous: u32,
        current: u32,
    },
    ZeroOwnerIdWithoutPredecessor {
        chunk_id: u16,
        index: u16,
        end_offset: u32,
    },
    DuplicateEndOffset {
        first_owner_id: u16,
        second_owner_id: u16,
        end_offset: u32,
    },
}

impl fmt::Display for Legacy0x22TextInfoReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contents(error) => error.fmt(f),
            Self::UnexpectedFamily(found) => {
                write!(f, "expected legacy Contents family 0x22, found {found:?}")
            }
            Self::TrailerPointerOutOfBounds { offset, stream_len } => write!(
                f,
                "legacy 0x22 text-info trailer pointer {offset:#x} is outside Contents length {stream_len}"
            ),
            Self::DirectoryOutOfBounds {
                trailer_offset,
                count,
                stream_len,
            } => write!(
                f,
                "legacy 0x22 text-info directory at {trailer_offset:#x} with {count} entries exceeds Contents length {stream_len}"
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
            Self::MultipleTextInfoChunks { chunk_ids } => write!(
                f,
                "legacy 0x22 Contents contains multiple text-info chunks: {chunk_ids:?}"
            ),
            Self::TextInfoHeaderTooShort {
                chunk_id,
                chunk_offset,
                data_offset,
            } => write!(
                f,
                "legacy 0x22 text-info chunk {chunk_id} header {chunk_offset:#x}..{data_offset:#x} is too short"
            ),
            Self::MissingTargetChunk {
                text_info_chunk_id,
                target_chunk_id,
            } => write!(
                f,
                "legacy 0x22 text-info chunk {text_info_chunk_id} references missing target chunk {target_chunk_id}"
            ),
            Self::TargetHeaderTooShort {
                chunk_id,
                chunk_offset,
            } => write!(
                f,
                "legacy 0x22 text-info target chunk {chunk_id} at {chunk_offset:#x} has no complete chunk header"
            ),
            Self::ListHeaderOutOfBounds {
                chunk_id,
                list_offset,
                chunk_end,
            } => write!(
                f,
                "legacy 0x22 text-info target chunk {chunk_id} list header at {list_offset:#x} exceeds chunk end {chunk_end:#x}"
            ),
            Self::UnsupportedExtendedListHeader {
                chunk_id,
                count,
                max_count,
            } => write!(
                f,
                "legacy 0x22 text-info target chunk {chunk_id} uses unsupported extended list header count={count} max={max_count}"
            ),
            Self::UnexpectedRecordSize { chunk_id, found } => write!(
                f,
                "legacy 0x22 text-info target chunk {chunk_id} record size is {found}, expected 10"
            ),
            Self::ListPayloadOutOfBounds {
                chunk_id,
                count,
                record_size,
                chunk_end,
            } => write!(
                f,
                "legacy 0x22 text-info target chunk {chunk_id} payload count={count} record_size={record_size} exceeds chunk end {chunk_end:#x}"
            ),
            Self::PositionDecreased {
                chunk_id,
                index,
                previous,
                current,
            } => write!(
                f,
                "legacy 0x22 text-info target chunk {chunk_id} position {index} decreases from {previous} to {current}"
            ),
            Self::ZeroOwnerIdWithoutPredecessor {
                chunk_id,
                index,
                end_offset,
            } => write!(
                f,
                "legacy 0x22 text-info target chunk {chunk_id} record {index} extends end {end_offset} before any nonzero owner id"
            ),
            Self::DuplicateEndOffset {
                first_owner_id,
                second_owner_id,
                end_offset,
            } => write!(
                f,
                "legacy 0x22 text-info owner records {first_owner_id} and {second_owner_id} both end at relative offset {end_offset}"
            ),
        }
    }
}

impl std::error::Error for Legacy0x22TextInfoReadError {}

impl From<ContentsReadError> for Legacy0x22TextInfoReadError {
    fn from(value: ContentsReadError) -> Self {
        Self::Contents(value)
    }
}

#[derive(Debug, Clone)]
struct DirectoryEntry {
    directory_entry_source: RawSpan,
    chunk_id: u16,
    chunk_offset: usize,
    chunk_end: usize,
    chunk_type: u16,
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

fn directory(
    stream: &StreamPath,
    bytes: &[u8],
) -> Result<Vec<DirectoryEntry>, Legacy0x22TextInfoReadError> {
    let trailer_offset = read_u32(bytes, LEGACY_0X22_TRAILER_POINTER_OFFSET).ok_or(
        Legacy0x22TextInfoReadError::TrailerPointerOutOfBounds {
            offset: u32::MAX,
            stream_len: bytes.len(),
        },
    )?;
    let trailer = trailer_offset as usize;
    if trailer.checked_add(2).is_none_or(|end| end > bytes.len()) {
        return Err(Legacy0x22TextInfoReadError::TrailerPointerOutOfBounds {
            offset: trailer_offset,
            stream_len: bytes.len(),
        });
    }

    let count = read_u16(bytes, trailer).expect("two trailer bytes checked above");
    let directory_end = trailer
        .checked_add(2)
        .and_then(|value| value.checked_add(usize::from(count) * LEGACY_DIRECTORY_ENTRY_SIZE))
        .ok_or(Legacy0x22TextInfoReadError::DirectoryOutOfBounds {
            trailer_offset,
            count,
            stream_len: bytes.len(),
        })?;
    if directory_end > bytes.len() {
        return Err(Legacy0x22TextInfoReadError::DirectoryOutOfBounds {
            trailer_offset,
            count,
            stream_len: bytes.len(),
        });
    }

    let mut raw_entries = Vec::with_capacity(usize::from(count));
    for index in 0..usize::from(count) {
        let entry_offset = trailer + 2 + index * LEGACY_DIRECTORY_ENTRY_SIZE;
        let chunk_id = read_u16(bytes, entry_offset + 2).expect("directory bounds checked");
        let chunk_offset = read_u32(bytes, entry_offset + 6).expect("directory bounds checked");
        let chunk = chunk_offset as usize;
        if chunk.checked_add(2).is_none_or(|end| end > bytes.len()) {
            return Err(Legacy0x22TextInfoReadError::ChunkOffsetOutOfBounds {
                directory_index: index as u16,
                chunk_id,
                chunk_offset,
                stream_len: bytes.len(),
            });
        }
        raw_entries.push((
            index as u16,
            span(stream, entry_offset, LEGACY_DIRECTORY_ENTRY_SIZE),
            chunk_id,
            chunk,
            read_u16(bytes, chunk).expect("chunk type bounds checked"),
        ));
    }

    let mut offsets = raw_entries.iter().map(|entry| entry.3).collect::<Vec<_>>();
    offsets.sort_unstable();
    offsets.dedup();

    Ok(raw_entries
        .into_iter()
        .map(
            |(_directory_index, directory_entry_source, chunk_id, chunk_offset, chunk_type)| {
                let chunk_end = offsets
                    .iter()
                    .copied()
                    .find(|offset| *offset > chunk_offset)
                    .unwrap_or(bytes.len());
                DirectoryEntry {
                    directory_entry_source,
                    chunk_id,
                    chunk_offset,
                    chunk_end,
                    chunk_type,
                }
            },
        )
        .collect())
}

/// Decode the Publisher 2 TEXT_INFO owner-boundary map used by the historical
/// text consumer to replace the synthetic `65536 + shape_ordinal` text key
/// with an explicit Contents owner/object id at matching boundaries.
///
/// This mirrors the verified low-family algorithm only:
/// - one type-0x0016 text-info chunk;
/// - a 16-bit list header in its referenced target chunk;
/// - 10-byte records;
/// - u32 end positions stored separately from those records;
/// - a zero owner id extends the immediately preceding nonzero owner id.
pub fn parse_legacy_0x22_text_info_map(
    stream: StreamPath,
    bytes: &[u8],
) -> Result<Option<Legacy0x22TextInfoMap>, Legacy0x22TextInfoReadError> {
    let family = detect_family(bytes)?;
    if family != ContentsFamily::Family0x22 {
        return Err(Legacy0x22TextInfoReadError::UnexpectedFamily(family));
    }

    let entries = directory(&stream, bytes)?;
    let text_info_entries = entries
        .iter()
        .filter(|entry| entry.chunk_type == LEGACY_0X22_TEXT_INFO_CHUNK_TYPE)
        .collect::<Vec<_>>();
    if text_info_entries.is_empty() {
        return Ok(None);
    }
    if text_info_entries.len() > 1 {
        return Err(Legacy0x22TextInfoReadError::MultipleTextInfoChunks {
            chunk_ids: text_info_entries
                .iter()
                .map(|entry| entry.chunk_id)
                .collect(),
        });
    }

    let text_info = text_info_entries[0];
    let chunk = text_info.chunk_offset;
    let data_offset_delta =
        *bytes
            .get(chunk + 3)
            .ok_or(Legacy0x22TextInfoReadError::TextInfoHeaderTooShort {
                chunk_id: text_info.chunk_id,
                chunk_offset: chunk as u32,
                data_offset: chunk as u32,
            })?;
    let data_offset = chunk.saturating_add(usize::from(data_offset_delta));
    if data_offset < chunk + LEGACY_TEXT_INFO_MIN_HEADER_SIZE {
        return Err(Legacy0x22TextInfoReadError::TextInfoHeaderTooShort {
            chunk_id: text_info.chunk_id,
            chunk_offset: chunk as u32,
            data_offset: data_offset as u32,
        });
    }
    let target_chunk_id =
        read_u16(bytes, chunk + 8).ok_or(Legacy0x22TextInfoReadError::TextInfoHeaderTooShort {
            chunk_id: text_info.chunk_id,
            chunk_offset: chunk as u32,
            data_offset: data_offset as u32,
        })?;
    let target = entries
        .iter()
        .find(|entry| entry.chunk_id == target_chunk_id)
        .ok_or(Legacy0x22TextInfoReadError::MissingTargetChunk {
            text_info_chunk_id: text_info.chunk_id,
            target_chunk_id,
        })?;

    let target_delta = *bytes.get(target.chunk_offset + 3).ok_or(
        Legacy0x22TextInfoReadError::TargetHeaderTooShort {
            chunk_id: target.chunk_id,
            chunk_offset: target.chunk_offset as u32,
        },
    )?;
    let list_offset = target
        .chunk_offset
        .checked_add(usize::from(target_delta))
        .ok_or(Legacy0x22TextInfoReadError::TargetHeaderTooShort {
            chunk_id: target.chunk_id,
            chunk_offset: target.chunk_offset as u32,
        })?;
    let list_end = list_offset.saturating_add(LEGACY_TEXT_LIST_HEADER_SIZE);
    if list_end > target.chunk_end || list_end > bytes.len() {
        return Err(Legacy0x22TextInfoReadError::ListHeaderOutOfBounds {
            chunk_id: target.chunk_id,
            list_offset: list_offset as u32,
            chunk_end: target.chunk_end as u32,
        });
    }

    let count = read_u16(bytes, list_offset).expect("list header bounds checked");
    let max_count = read_u16(bytes, list_offset + 2).expect("list header bounds checked");
    if max_count < count {
        return Err(Legacy0x22TextInfoReadError::UnsupportedExtendedListHeader {
            chunk_id: target.chunk_id,
            count,
            max_count,
        });
    }
    let record_size = read_u16(bytes, list_offset + 4).expect("list header bounds checked");
    if record_size != LEGACY_TEXT_INFO_RECORD_SIZE {
        return Err(Legacy0x22TextInfoReadError::UnexpectedRecordSize {
            chunk_id: target.chunk_id,
            found: record_size,
        });
    }

    let positions_start = list_offset
        .saturating_add(LEGACY_TEXT_LIST_HEADER_SIZE + LEGACY_TEXT_LIST_EXTRA_HEADER_SIZE);
    let positions_len = usize::from(count).saturating_mul(4);
    let records_start = positions_start.saturating_add(positions_len);
    let records_len = usize::from(count).saturating_mul(usize::from(record_size));
    let payload_end = records_start.saturating_add(records_len);
    if payload_end > target.chunk_end || payload_end > bytes.len() {
        return Err(Legacy0x22TextInfoReadError::ListPayloadOutOfBounds {
            chunk_id: target.chunk_id,
            count,
            record_size,
            chunk_end: target.chunk_end as u32,
        });
    }

    let mut by_owner = BTreeMap::<u16, Legacy0x22TextInfoOwnerEnd>::new();
    let mut active_offset = 0u32;
    let mut old_owner_id = None::<u16>;

    for index in 0..usize::from(count) {
        let position_offset = positions_start + index * 4;
        let current = read_u32(bytes, position_offset).expect("positions bounds checked");
        if current < active_offset {
            return Err(Legacy0x22TextInfoReadError::PositionDecreased {
                chunk_id: target.chunk_id,
                index: index as u16,
                previous: active_offset,
                current,
            });
        }
        if current == active_offset {
            continue;
        }

        let record_offset = records_start + index * usize::from(record_size);
        let owner_id = read_u16(bytes, record_offset).expect("record bounds checked");
        let relative_end_offset = current - 1;
        let end_offset_source = span(&stream, position_offset, 4);

        if owner_id != 0 {
            by_owner
                .entry(owner_id)
                .and_modify(|end| {
                    end.relative_end_offset = relative_end_offset;
                    end.end_offset_source = end_offset_source.clone();
                })
                .or_insert_with(|| Legacy0x22TextInfoOwnerEnd {
                    owner_id,
                    relative_end_offset,
                    owner_id_source: span(&stream, record_offset, 2),
                    end_offset_source,
                });
            old_owner_id = Some(owner_id);
            active_offset = current;
            continue;
        }

        let previous =
            old_owner_id.ok_or(Legacy0x22TextInfoReadError::ZeroOwnerIdWithoutPredecessor {
                chunk_id: target.chunk_id,
                index: index as u16,
                end_offset: relative_end_offset,
            })?;
        let end = by_owner
            .get_mut(&previous)
            .expect("previous nonzero owner id was inserted");
        end.relative_end_offset = relative_end_offset;
        end.end_offset_source = end_offset_source;
        active_offset = current;
    }

    let mut ends = by_owner.into_values().collect::<Vec<_>>();
    ends.sort_by_key(|end| end.relative_end_offset);
    let mut seen_offsets = BTreeSet::new();
    for end in &ends {
        if !seen_offsets.insert(end.relative_end_offset) {
            let first_owner_id = ends
                .iter()
                .find(|candidate| {
                    candidate.relative_end_offset == end.relative_end_offset
                        && candidate.owner_id != end.owner_id
                })
                .map(|candidate| candidate.owner_id)
                .unwrap_or(end.owner_id);
            return Err(Legacy0x22TextInfoReadError::DuplicateEndOffset {
                first_owner_id,
                second_owner_id: end.owner_id,
                end_offset: end.relative_end_offset,
            });
        }
    }

    Ok(Some(Legacy0x22TextInfoMap {
        text_info_chunk_id: text_info.chunk_id,
        text_info_chunk_source: text_info.directory_entry_source.clone(),
        target_chunk_id,
        target_chunk_id_source: span(&stream, chunk + 8, 2),
        list_count: count,
        list_count_source: span(&stream, list_offset, 2),
        record_size,
        record_size_source: span(&stream, list_offset + 4, 2),
        ends,
    }))
}
