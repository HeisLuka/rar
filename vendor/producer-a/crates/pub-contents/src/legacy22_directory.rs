use crate::{ContentsFamily, ContentsReadError, detect_family};
use pub_core::{RawSpan, StreamPath};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const LEGACY_0X22_TRAILER_POINTER_OFFSET: usize = 0x16;
pub const LEGACY_0X22_DIRECTORY_ENTRY_SIZE: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22DirectoryEntry {
    pub directory_index: u16,
    pub entry_source: RawSpan,
    /// Uninterpreted/service word at directory-entry +0.
    pub service_word: u16,
    pub service_word_source: RawSpan,
    /// Persistent old-family object identity at directory-entry +2.
    pub object_id: u16,
    pub object_id_source: RawSpan,
    /// Persistent old-family parent identity at directory-entry +4.
    pub parent_id: u16,
    pub parent_id_source: RawSpan,
    /// Absolute Contents stream offset at directory-entry +6.
    pub chunk_offset: u32,
    pub chunk_offset_source: RawSpan,
    /// Physical old-family marker stored at chunk +0.
    pub chunk_type: u16,
    pub chunk_type_source: RawSpan,
    /// Bounded physical extent from this chunk start to the next distinct
    /// chunk start, or to the trailer when this is the final pre-trailer chunk.
    /// No semantics are inferred from the extent.
    pub chunk_source: RawSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22Directory {
    pub trailer_offset: u32,
    pub trailer_offset_source: RawSpan,
    pub entry_count: u16,
    pub entry_count_source: RawSpan,
    /// Complete directory in persisted order. Unknown/service objects are
    /// retained exactly instead of being filtered by semantic type.
    pub entries: Vec<Legacy0x22DirectoryEntry>,
}

impl Legacy0x22Directory {
    pub fn entry_by_object_id(&self, object_id: u16) -> Option<&Legacy0x22DirectoryEntry> {
        self.entries
            .iter()
            .find(|entry| entry.object_id == object_id)
    }

    pub fn entries_by_parent_id(
        &self,
        parent_id: u16,
    ) -> impl Iterator<Item = &Legacy0x22DirectoryEntry> {
        self.entries
            .iter()
            .filter(move |entry| entry.parent_id == parent_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Legacy0x22DirectoryReadError {
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
        object_id: u16,
        chunk_offset: u32,
        stream_len: usize,
    },
}

impl fmt::Display for Legacy0x22DirectoryReadError {
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
                object_id,
                chunk_offset,
                stream_len,
            } => write!(
                f,
                "legacy 0x22 directory entry {directory_index} object {object_id} points to {chunk_offset:#x} outside Contents length {stream_len}"
            ),
        }
    }
}

impl std::error::Error for Legacy0x22DirectoryReadError {}

impl From<ContentsReadError> for Legacy0x22DirectoryReadError {
    fn from(value: ContentsReadError) -> Self {
        Self::Contents(value)
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

/// Losslessly inventories the old-family 0x22 trailer directory.
///
/// This parser intentionally stops before object semantics. It preserves every
/// 10-byte directory entry, persistent object/parent identity, physical chunk
/// marker, and bounded source span so higher layers can classify only the
/// object types for which format evidence exists.
pub fn parse_legacy_0x22_directory(
    stream: StreamPath,
    bytes: &[u8],
) -> Result<Legacy0x22Directory, Legacy0x22DirectoryReadError> {
    let family = detect_family(bytes)?;
    if family != ContentsFamily::Family0x22 {
        return Err(Legacy0x22DirectoryReadError::UnexpectedFamily(family));
    }

    let trailer_offset = read_u32(bytes, LEGACY_0X22_TRAILER_POINTER_OFFSET).ok_or(
        Legacy0x22DirectoryReadError::TrailerPointerOutOfBounds {
            offset: u32::MAX,
            stream_len: bytes.len(),
        },
    )?;
    let trailer = trailer_offset as usize;
    if trailer.checked_add(2).is_none_or(|end| end > bytes.len()) {
        return Err(Legacy0x22DirectoryReadError::TrailerPointerOutOfBounds {
            offset: trailer_offset,
            stream_len: bytes.len(),
        });
    }

    let entry_count = read_u16(bytes, trailer).expect("trailer count bounds checked");
    let directory_end = trailer
        .checked_add(2)
        .and_then(|value| {
            value.checked_add(
                usize::from(entry_count).saturating_mul(LEGACY_0X22_DIRECTORY_ENTRY_SIZE),
            )
        })
        .ok_or(Legacy0x22DirectoryReadError::DirectoryOutOfBounds {
            trailer_offset,
            count: entry_count,
            stream_len: bytes.len(),
        })?;
    if directory_end > bytes.len() {
        return Err(Legacy0x22DirectoryReadError::DirectoryOutOfBounds {
            trailer_offset,
            count: entry_count,
            stream_len: bytes.len(),
        });
    }

    #[derive(Debug, Clone)]
    struct RawEntry {
        directory_index: u16,
        entry_offset: usize,
        service_word: u16,
        object_id: u16,
        parent_id: u16,
        chunk_offset: usize,
        chunk_type: u16,
    }

    let mut raw_entries = Vec::with_capacity(usize::from(entry_count));
    for index in 0..usize::from(entry_count) {
        let entry_offset = trailer + 2 + index * LEGACY_0X22_DIRECTORY_ENTRY_SIZE;
        let service_word = read_u16(bytes, entry_offset).expect("directory bounds checked");
        let object_id = read_u16(bytes, entry_offset + 2).expect("directory bounds checked");
        let parent_id = read_u16(bytes, entry_offset + 4).expect("directory bounds checked");
        let chunk_offset = read_u32(bytes, entry_offset + 6).expect("directory bounds checked");
        let chunk = chunk_offset as usize;
        if chunk.checked_add(2).is_none_or(|end| end > bytes.len()) {
            return Err(Legacy0x22DirectoryReadError::ChunkOffsetOutOfBounds {
                directory_index: index as u16,
                object_id,
                chunk_offset,
                stream_len: bytes.len(),
            });
        }

        raw_entries.push(RawEntry {
            directory_index: index as u16,
            entry_offset,
            service_word,
            object_id,
            parent_id,
            chunk_offset: chunk,
            chunk_type: read_u16(bytes, chunk).expect("chunk marker bounds checked"),
        });
    }

    let mut starts = raw_entries
        .iter()
        .map(|entry| entry.chunk_offset)
        .collect::<Vec<_>>();
    starts.sort_unstable();
    starts.dedup();

    let entries = raw_entries
        .into_iter()
        .map(|entry| {
            let next_chunk = starts
                .iter()
                .copied()
                .find(|offset| *offset > entry.chunk_offset)
                .unwrap_or(bytes.len());
            let chunk_end = if trailer > entry.chunk_offset {
                next_chunk.min(trailer)
            } else {
                next_chunk
            };
            let chunk_len = chunk_end.saturating_sub(entry.chunk_offset);

            Legacy0x22DirectoryEntry {
                directory_index: entry.directory_index,
                entry_source: span(
                    &stream,
                    entry.entry_offset,
                    LEGACY_0X22_DIRECTORY_ENTRY_SIZE,
                ),
                service_word: entry.service_word,
                service_word_source: span(&stream, entry.entry_offset, 2),
                object_id: entry.object_id,
                object_id_source: span(&stream, entry.entry_offset + 2, 2),
                parent_id: entry.parent_id,
                parent_id_source: span(&stream, entry.entry_offset + 4, 2),
                chunk_offset: entry.chunk_offset as u32,
                chunk_offset_source: span(&stream, entry.entry_offset + 6, 4),
                chunk_type: entry.chunk_type,
                chunk_type_source: span(&stream, entry.chunk_offset, 2),
                chunk_source: span(&stream, entry.chunk_offset, chunk_len),
            }
        })
        .collect();

    Ok(Legacy0x22Directory {
        trailer_offset,
        trailer_offset_source: span(&stream, LEGACY_0X22_TRAILER_POINTER_OFFSET, 4),
        entry_count,
        entry_count_source: span(&stream, trailer, 2),
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CONTENTS_0X2C_MAGIC, CONTENTS_0X22_MAGIC};

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0_u8; 128];
        bytes[..4].copy_from_slice(&CONTENTS_0X22_MAGIC);
        bytes[LEGACY_0X22_TRAILER_POINTER_OFFSET..LEGACY_0X22_TRAILER_POINTER_OFFSET + 4]
            .copy_from_slice(&80_u32.to_le_bytes());
        bytes[80..82].copy_from_slice(&2_u16.to_le_bytes());

        bytes[82..84].copy_from_slice(&0xaaaa_u16.to_le_bytes());
        bytes[84..86].copy_from_slice(&10_u16.to_le_bytes());
        bytes[86..88].copy_from_slice(&1_u16.to_le_bytes());
        bytes[88..92].copy_from_slice(&32_u32.to_le_bytes());

        bytes[92..94].copy_from_slice(&0xbbbb_u16.to_le_bytes());
        bytes[94..96].copy_from_slice(&11_u16.to_le_bytes());
        bytes[96..98].copy_from_slice(&10_u16.to_le_bytes());
        bytes[98..102].copy_from_slice(&48_u32.to_le_bytes());

        bytes[32..34].copy_from_slice(&0x0014_u16.to_le_bytes());
        bytes[48..50].copy_from_slice(&0x0008_u16.to_le_bytes());
        bytes
    }

    #[test]
    fn inventories_every_directory_entry_in_persisted_order() {
        let stream = StreamPath("/Contents".into());
        let parsed = parse_legacy_0x22_directory(stream.clone(), &fixture()).unwrap();

        assert_eq!(parsed.trailer_offset, 80);
        assert_eq!(parsed.entry_count, 2);
        assert_eq!(parsed.entries.len(), 2);

        let first = &parsed.entries[0];
        assert_eq!(first.directory_index, 0);
        assert_eq!(first.service_word, 0xaaaa);
        assert_eq!(first.object_id, 10);
        assert_eq!(first.parent_id, 1);
        assert_eq!(first.chunk_offset, 32);
        assert_eq!(first.chunk_type, 0x0014);
        assert_eq!(
            first.chunk_source,
            RawSpan {
                stream: stream.clone(),
                offset: 32,
                len: 16,
            }
        );

        let second = &parsed.entries[1];
        assert_eq!(second.object_id, 11);
        assert_eq!(second.parent_id, 10);
        assert_eq!(second.chunk_type, 0x0008);
        assert_eq!(
            second.chunk_source,
            RawSpan {
                stream,
                offset: 48,
                len: 32,
            }
        );
    }

    #[test]
    fn rejects_non_legacy_family_without_guessing() {
        let mut bytes = fixture();
        bytes[..4].copy_from_slice(&CONTENTS_0X2C_MAGIC);
        assert_eq!(
            parse_legacy_0x22_directory(StreamPath("/Contents".into()), &bytes),
            Err(Legacy0x22DirectoryReadError::UnexpectedFamily(
                ContentsFamily::Family0x2c
            ))
        );
    }

    #[test]
    fn rejects_directory_entry_pointing_outside_contents() {
        let mut bytes = fixture();
        bytes[88..92].copy_from_slice(&127_u32.to_le_bytes());
        assert!(matches!(
            parse_legacy_0x22_directory(StreamPath("/Contents".into()), &bytes),
            Err(Legacy0x22DirectoryReadError::ChunkOffsetOutOfBounds {
                directory_index: 0,
                object_id: 10,
                chunk_offset: 127,
                ..
            })
        ));
    }
}
