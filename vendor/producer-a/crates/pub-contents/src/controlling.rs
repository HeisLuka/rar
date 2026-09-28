use crate::{
    BLOCK_TYPE_CONTAINER_88, BLOCK_TYPE_CONTAINER_90, BLOCK_TYPE_FIXED_8, BlockReadError,
    ContentsCursor, ContentsReadError, OidIdentityPayload, RawContentsBlock,
    RawContentsBlockBody, parse_confirmed_block, parse_confirmed_oid_identity_payload,
};
use pub_core::RawSpan;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const CONTROLLING_PAGE_LIST_ID: u16 = 0x06;
pub const CONTROLLING_PAGE_LIST_ENTRY_ID: u16 = 0x00;
pub const CONTROLLING_PAGE_LIST_PGID_ID: u16 = 0x01;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControllingPageList {
    pub block: RawContentsBlock,
    pub entries: Vec<ControllingPageListEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControllingPageListEntry {
    pub source: RawSpan,
    pub fields: Vec<RawContentsBlock>,
    pub pgid: OidIdentityPayload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControllingPageListReadError {
    Contents(ContentsReadError),
    Block(BlockReadError),
    SpanTooLarge { source: RawSpan },
    UnexpectedOuterId { offset: u64, id: u16 },
    UnexpectedOuterType { offset: u64, block_type: u8 },
    InconsistentOuterBody,
    UnexpectedEntryId { offset: u64, id: u16 },
    UnexpectedEntryType { offset: u64, block_type: u8 },
    InconsistentEntryBody { offset: u64 },
    MissingPgid { entry_index: usize },
    DuplicatePgid { entry_index: usize },
    UnexpectedPgidType { entry_index: usize, block_type: u8 },
    InvalidPgid { entry_index: usize, reason: String },
}

impl fmt::Display for ControllingPageListReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ControllingPageListReadError {}

impl From<ContentsReadError> for ControllingPageListReadError {
    fn from(value: ContentsReadError) -> Self {
        Self::Contents(value)
    }
}

impl From<BlockReadError> for ControllingPageListReadError {
    fn from(value: BlockReadError) -> Self {
        Self::Block(value)
    }
}

/// Parses only the Publisher-named OplControlling.PageList / OplPageListInfo.Pgid
/// subset grounded by Publisher-generated hidden XML.
///
/// Confirmed bounded shape:
///
/// field0x06 / container 0x90
///   repeated field0x00 / container 0x88
///     unique field0x01 / fixed8 Pgid
///
/// Pgid is retained as the same 8-byte identity representation used by Page.Oid.
/// This reader does not claim that PageList exists on every OplControlling and
/// does not classify any page as customer-visible on its own.
pub fn parse_confirmed_controlling_page_list(
    bytes: &[u8],
    block: RawContentsBlock,
) -> Result<ControllingPageList, ControllingPageListReadError> {
    if block.id != CONTROLLING_PAGE_LIST_ID {
        return Err(ControllingPageListReadError::UnexpectedOuterId {
            offset: block.source.offset,
            id: block.id,
        });
    }
    if block.block_type != BLOCK_TYPE_CONTAINER_90 {
        return Err(ControllingPageListReadError::UnexpectedOuterType {
            offset: block.source.offset,
            block_type: block.block_type,
        });
    }

    let content_source = match &block.body {
        RawContentsBlockBody::Container { content_source, .. } => content_source.clone(),
        _ => return Err(ControllingPageListReadError::InconsistentOuterBody),
    };
    let start = usize::try_from(content_source.offset)
        .map_err(|_| ControllingPageListReadError::SpanTooLarge {
            source: content_source.clone(),
        })?;
    let len = usize::try_from(content_source.len)
        .map_err(|_| ControllingPageListReadError::SpanTooLarge {
            source: content_source.clone(),
        })?;

    let mut cursor = ContentsCursor::bounded(content_source.stream.clone(), bytes, start, len)?;
    let mut entries = Vec::new();

    while cursor.remaining() > 0 {
        let entry_index = entries.len();
        let entry = parse_confirmed_block(&mut cursor)?;
        if entry.id != CONTROLLING_PAGE_LIST_ENTRY_ID {
            return Err(ControllingPageListReadError::UnexpectedEntryId {
                offset: entry.source.offset,
                id: entry.id,
            });
        }
        if entry.block_type != BLOCK_TYPE_CONTAINER_88 {
            return Err(ControllingPageListReadError::UnexpectedEntryType {
                offset: entry.source.offset,
                block_type: entry.block_type,
            });
        }
        let entry_source = match &entry.body {
            RawContentsBlockBody::Container { content_source, .. } => content_source.clone(),
            _ => {
                return Err(ControllingPageListReadError::InconsistentEntryBody {
                    offset: entry.source.offset,
                });
            }
        };

        let entry_start = usize::try_from(entry_source.offset)
            .map_err(|_| ControllingPageListReadError::SpanTooLarge {
                source: entry_source.clone(),
            })?;
        let entry_len = usize::try_from(entry_source.len)
            .map_err(|_| ControllingPageListReadError::SpanTooLarge {
                source: entry_source.clone(),
            })?;
        let mut entry_cursor =
            ContentsCursor::bounded(entry_source.stream.clone(), bytes, entry_start, entry_len)?;
        let mut fields = Vec::new();
        while entry_cursor.remaining() > 0 {
            fields.push(parse_confirmed_block(&mut entry_cursor)?);
        }

        let mut pgids = fields
            .iter()
            .filter(|field| field.id == CONTROLLING_PAGE_LIST_PGID_ID);
        let pgid_block = pgids
            .next()
            .ok_or(ControllingPageListReadError::MissingPgid { entry_index })?;
        if pgids.next().is_some() {
            return Err(ControllingPageListReadError::DuplicatePgid { entry_index });
        }
        if pgid_block.block_type != BLOCK_TYPE_FIXED_8 {
            return Err(ControllingPageListReadError::UnexpectedPgidType {
                entry_index,
                block_type: pgid_block.block_type,
            });
        }
        let pgid = parse_confirmed_oid_identity_payload(pgid_block.clone()).map_err(|error| {
            ControllingPageListReadError::InvalidPgid {
                entry_index,
                reason: error.to_string(),
            }
        })?;

        entries.push(ControllingPageListEntry {
            source: entry.source,
            fields,
            pgid,
        });
    }

    Ok(ControllingPageList { block, entries })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode_packed_field_tag;
    use pub_core::StreamPath;

    fn fixed8(id: u16, bytes: [u8; 8]) -> Vec<u8> {
        let mut out = encode_packed_field_tag(id, BLOCK_TYPE_FIXED_8)
            .expect("fixed8 tag")
            .to_vec();
        out.extend_from_slice(&bytes);
        out
    }

    fn container(id: u16, wire: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = encode_packed_field_tag(id, wire)
            .expect("container tag")
            .to_vec();
        out.extend_from_slice(&u32::try_from(payload.len() + 4).unwrap().to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn parses_four_pgid_entries_from_68_byte_outer_field() {
        let mut payload = Vec::new();
        for dword1 in 0..4_u32 {
            let mut oid = Vec::new();
            oid.extend_from_slice(&1_u32.to_le_bytes());
            oid.extend_from_slice(&dword1.to_le_bytes());
            payload.extend_from_slice(&container(
                CONTROLLING_PAGE_LIST_ENTRY_ID,
                BLOCK_TYPE_CONTAINER_88,
                &fixed8(
                    CONTROLLING_PAGE_LIST_PGID_ID,
                    oid.try_into().expect("8-byte oid"),
                ),
            ));
        }
        let bytes = container(CONTROLLING_PAGE_LIST_ID, BLOCK_TYPE_CONTAINER_90, &payload);
        assert_eq!(bytes.len(), 70);
        let mut cursor = ContentsCursor::new(StreamPath("/Contents".into()), &bytes);
        let outer = parse_confirmed_block(&mut cursor).expect("outer page list");
        let parsed = parse_confirmed_controlling_page_list(&bytes, outer).expect("page list");

        assert_eq!(parsed.block.source.len, 70);
        assert_eq!(parsed.entries.len(), 4);
        assert_eq!(
            parsed
                .entries
                .iter()
                .map(|entry| (entry.pgid.dword0, entry.pgid.dword1))
                .collect::<Vec<_>>(),
            vec![(1, 0), (1, 1), (1, 2), (1, 3)]
        );
    }
}
