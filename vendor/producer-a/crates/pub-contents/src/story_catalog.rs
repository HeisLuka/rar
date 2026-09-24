use crate::{
    BlockReadError, Contents0x2cChunk, ContentsCursor, ContentsReadError, RawContentsBlock,
    RawContentsBlockBody, parse_confirmed_block,
};
use pub_core::RawSpan;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

pub const CONTENTS_RAW_TYPE_STORY_CATALOG: u16 = 0x65;
pub const STORY_CATALOG_DECLARED_COUNT_ID: u8 = 0x01;
pub const STORY_CATALOG_ENTRY_ARRAY_ID: u8 = 0x02;
pub const STORY_CATALOG_ENTRY_TEXT_ID: u8 = 0x01;
pub const STORY_CATALOG_ENTRY_LAYOUT_KEY_ID: u8 = 0x07;
pub const STORY_CATALOG_WIRE_U16_SERVICE: u8 = 0x10;
pub const STORY_CATALOG_WIRE_U32_SERVICE: u8 = 0x58;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatureStoryCatalog {
    pub source: RawSpan,
    pub fields: Vec<RawContentsBlock>,
    pub declared_count: u32,
    pub declared_count_source: RawSpan,
    pub entry_array_source: RawSpan,
    pub entries: Vec<MatureStoryCatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatureStoryCatalogEntry {
    pub source: RawSpan,
    pub fields: Vec<RawContentsBlock>,
    pub text_id: u32,
    pub text_id_source: RawSpan,
    /// Persistent story-layout/layout-record key. When MCLD exists this key
    /// selects the stored MCLD recordId. The key also exists in fixtures where
    /// MCLD is absent, so it must not be globally named an MCLD id.
    pub layout_key: Option<u32>,
    pub layout_key_source: Option<RawSpan>,
    /// Exact suffix beginning at the first entry-local wire form that this
    /// bounded reader does not yet understand.
    pub unsupported_tail: Option<RawSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoryCatalogReadError {
    Contents(ContentsReadError),
    Block(BlockReadError),
    SpanTooLarge { source: RawSpan },
    MissingDeclaredCount,
    DuplicateDeclaredCount,
    InvalidDeclaredCount,
    MissingEntryArray,
    DuplicateEntryArray,
    InvalidEntryArray,
    UnexpectedEntryId { offset: u64, id: u8 },
    InvalidEntryContainer { offset: u64 },
    MissingTextId { entry_index: usize },
    DuplicateTextId { entry_index: usize },
    InvalidTextId { entry_index: usize },
    MissingLayoutKey { entry_index: usize },
    DuplicateLayoutKey { entry_index: usize },
    InvalidLayoutKey { entry_index: usize },
    DuplicateTextIdentity { text_id: u32 },
    EntryCountMismatch { declared: u32, actual: usize },
}

impl fmt::Display for StoryCatalogReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for StoryCatalogReadError {}

impl From<ContentsReadError> for StoryCatalogReadError {
    fn from(value: ContentsReadError) -> Self {
        Self::Contents(value)
    }
}

impl From<BlockReadError> for StoryCatalogReadError {
    fn from(value: BlockReadError) -> Self {
        Self::Block(value)
    }
}

/// Parses the mature 0x65 Story catalog from an already bounded Contents chunk.
///
/// This reader raises only the grounded identity bridge:
/// - field 0x01: declared Story count;
/// - field 0x02: array of 0x00 container entries;
/// - entry field 0x01: textId;
/// - entry field 0x07: persistent story-layout key.
///
/// The caller is responsible for proving that the enclosing directory object
/// has raw type 0x65. Unknown entry fields remain raw in the fields vector.
pub fn parse_confirmed_mature_story_catalog(
    bytes: &[u8],
    chunk: &Contents0x2cChunk,
) -> Result<MatureStoryCatalog, StoryCatalogReadError> {
    let mut count_blocks = chunk
        .fields
        .iter()
        .filter(|field| field.id == STORY_CATALOG_DECLARED_COUNT_ID);
    let count_block = count_blocks
        .next()
        .ok_or(StoryCatalogReadError::MissingDeclaredCount)?;
    if count_blocks.next().is_some() {
        return Err(StoryCatalogReadError::DuplicateDeclaredCount);
    }
    let (declared_count, declared_count_source) =
        scalar_u32(count_block).ok_or(StoryCatalogReadError::InvalidDeclaredCount)?;

    let mut array_blocks = chunk
        .fields
        .iter()
        .filter(|field| field.id == STORY_CATALOG_ENTRY_ARRAY_ID);
    let array_block = array_blocks
        .next()
        .ok_or(StoryCatalogReadError::MissingEntryArray)?;
    if array_blocks.next().is_some() {
        return Err(StoryCatalogReadError::DuplicateEntryArray);
    }
    let RawContentsBlockBody::Container {
        content_source: entry_array_source,
        ..
    } = &array_block.body
    else {
        return Err(StoryCatalogReadError::InvalidEntryArray);
    };

    let mut entries = Vec::new();
    let mut seen_text_ids = BTreeSet::new();

    for (entry_index, item) in parse_blocks_in_span(bytes, entry_array_source)?
        .into_iter()
        .enumerate()
    {
        if item.id != 0 {
            return Err(StoryCatalogReadError::UnexpectedEntryId {
                offset: item.source.offset,
                id: item.id,
            });
        }
        let RawContentsBlockBody::Container {
            content_source: entry_source,
            ..
        } = &item.body
        else {
            return Err(StoryCatalogReadError::InvalidEntryContainer {
                offset: item.source.offset,
            });
        };

        let (fields, unsupported_tail) = parse_entry_fields_in_span(bytes, entry_source)?;
        let (text_id, text_id_source) = unique_entry_scalar(
            &fields,
            STORY_CATALOG_ENTRY_TEXT_ID,
            StoryCatalogReadError::MissingTextId { entry_index },
            StoryCatalogReadError::DuplicateTextId { entry_index },
            StoryCatalogReadError::InvalidTextId { entry_index },
        )?;
        let (layout_key, layout_key_source) = optional_unique_entry_scalar(
            &fields,
            STORY_CATALOG_ENTRY_LAYOUT_KEY_ID,
            StoryCatalogReadError::DuplicateLayoutKey { entry_index },
            StoryCatalogReadError::InvalidLayoutKey { entry_index },
        )?;

        if !seen_text_ids.insert(text_id) {
            return Err(StoryCatalogReadError::DuplicateTextIdentity { text_id });
        }

        entries.push(MatureStoryCatalogEntry {
            source: item.source,
            fields,
            text_id,
            text_id_source,
            layout_key,
            layout_key_source,
            unsupported_tail,
        });
    }

    if usize::try_from(declared_count).ok() != Some(entries.len()) {
        return Err(StoryCatalogReadError::EntryCountMismatch {
            declared: declared_count,
            actual: entries.len(),
        });
    }

    Ok(MatureStoryCatalog {
        source: chunk.source.clone(),
        fields: chunk.fields.clone(),
        declared_count,
        declared_count_source,
        entry_array_source: entry_array_source.clone(),
        entries,
    })
}

fn optional_unique_entry_scalar(
    fields: &[RawContentsBlock],
    id: u8,
    duplicate: StoryCatalogReadError,
    invalid: StoryCatalogReadError,
) -> Result<(Option<u32>, Option<RawSpan>), StoryCatalogReadError> {
    let mut matches = fields.iter().filter(|field| field.id == id);
    let Some(field) = matches.next() else {
        return Ok((None, None));
    };
    if matches.next().is_some() {
        return Err(duplicate);
    }
    let (value, source) = scalar_u32(field).ok_or(invalid)?;
    Ok((Some(value), Some(source)))
}

fn unique_entry_scalar(
    fields: &[RawContentsBlock],
    id: u8,
    missing: StoryCatalogReadError,
    duplicate: StoryCatalogReadError,
    invalid: StoryCatalogReadError,
) -> Result<(u32, RawSpan), StoryCatalogReadError> {
    let mut matches = fields.iter().filter(|field| field.id == id);
    let field = matches.next().ok_or(missing)?;
    if matches.next().is_some() {
        return Err(duplicate);
    }
    scalar_u32(field).ok_or(invalid)
}

fn scalar_u32(field: &RawContentsBlock) -> Option<(u32, RawSpan)> {
    match &field.body {
        RawContentsBlockBody::U16 {
            value,
            value_source,
        } => Some((u32::from(*value), value_source.clone())),
        RawContentsBlockBody::U32 {
            value,
            value_source,
        } => Some((*value, value_source.clone())),
        _ => None,
    }
}

fn parse_blocks_in_span(
    bytes: &[u8],
    source: &RawSpan,
) -> Result<Vec<RawContentsBlock>, StoryCatalogReadError> {
    let start =
        usize::try_from(source.offset).map_err(|_| StoryCatalogReadError::SpanTooLarge {
            source: source.clone(),
        })?;
    let len = usize::try_from(source.len).map_err(|_| StoryCatalogReadError::SpanTooLarge {
        source: source.clone(),
    })?;
    let mut cursor = ContentsCursor::bounded(source.stream.clone(), bytes, start, len)?;
    let mut fields = Vec::new();

    while cursor.remaining() > 0 {
        fields.push(parse_story_catalog_block(&mut cursor)?);
    }

    Ok(fields)
}

fn parse_entry_fields_in_span(
    bytes: &[u8],
    source: &RawSpan,
) -> Result<(Vec<RawContentsBlock>, Option<RawSpan>), StoryCatalogReadError> {
    let start =
        usize::try_from(source.offset).map_err(|_| StoryCatalogReadError::SpanTooLarge {
            source: source.clone(),
        })?;
    let len = usize::try_from(source.len).map_err(|_| StoryCatalogReadError::SpanTooLarge {
        source: source.clone(),
    })?;
    let mut cursor = ContentsCursor::bounded(source.stream.clone(), bytes, start, len)?;
    let mut fields = Vec::new();

    while cursor.remaining() > 0 {
        match parse_story_catalog_block(&mut cursor) {
            Ok(field) => fields.push(field),
            Err(StoryCatalogReadError::Block(BlockReadError::UnsupportedType { .. })) => {
                return Ok((
                    fields,
                    Some(RawSpan {
                        stream: source.stream.clone(),
                        offset: cursor.position() as u64,
                        len: cursor.remaining() as u64,
                    }),
                ));
            }
            Err(error) => return Err(error),
        }
    }

    Ok((fields, None))
}

fn parse_story_catalog_block(
    cursor: &mut ContentsCursor<'_>,
) -> Result<RawContentsBlock, StoryCatalogReadError> {
    let original = cursor.clone();
    match parse_confirmed_block(cursor) {
        Ok(block) => return Ok(block),
        Err(BlockReadError::UnsupportedType { .. }) => {
            *cursor = original;
        }
        Err(error) => return Err(error.into()),
    }

    let mut probe = cursor.clone();
    let start = probe.position();
    let (id, id_source) = probe.read_u8()?;
    let (block_type, _) = probe.read_u8()?;

    let body = match block_type {
        STORY_CATALOG_WIRE_U16_SERVICE => {
            let (value, value_source) = probe.read_u16_le()?;
            RawContentsBlockBody::U16 {
                value,
                value_source,
            }
        }
        STORY_CATALOG_WIRE_U32_SERVICE => {
            let (value, value_source) = probe.read_u32_le()?;
            RawContentsBlockBody::U32 {
                value,
                value_source,
            }
        }
        _ => {
            return Err(StoryCatalogReadError::Block(
                BlockReadError::UnsupportedType {
                    block_type,
                    offset: start,
                },
            ));
        }
    };

    let end = probe.position();
    let block = RawContentsBlock {
        id,
        block_type,
        source: RawSpan {
            stream: id_source.stream,
            offset: id_source.offset,
            len: (end - start) as u64,
        },
        body,
    };
    *cursor = probe;
    Ok(block)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BLOCK_TYPE_CONTAINER_88, BLOCK_TYPE_CONTAINER_A0, BLOCK_TYPE_U16, BLOCK_TYPE_U32,
        parse_0x2c_header, parse_confirmed_0x2c_chunk, parse_confirmed_0x2c_trailer_root,
        parse_confirmed_chunk_reference,
    };
    use pub_core::StreamPath;

    fn push_u32(out: &mut Vec<u8>, id: u8, value: u32) {
        out.extend_from_slice(&[id, BLOCK_TYPE_U32]);
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn container(id: u8, wire: u8, content: &[u8]) -> Vec<u8> {
        let mut out = vec![id, wire];
        out.extend_from_slice(&u32::try_from(content.len() + 4).unwrap().to_le_bytes());
        out.extend_from_slice(content);
        out
    }

    fn synthetic_chunk() -> Vec<u8> {
        let mut e1 = Vec::new();
        push_u32(&mut e1, STORY_CATALOG_ENTRY_TEXT_ID, 6);
        push_u32(&mut e1, STORY_CATALOG_ENTRY_LAYOUT_KEY_ID, 4);

        let mut e2 = Vec::new();
        push_u32(&mut e2, STORY_CATALOG_ENTRY_TEXT_ID, 9);
        push_u32(&mut e2, STORY_CATALOG_ENTRY_LAYOUT_KEY_ID, 12);

        let mut array = Vec::new();
        array.extend_from_slice(&container(0, BLOCK_TYPE_CONTAINER_88, &e1));
        array.extend_from_slice(&container(0, BLOCK_TYPE_CONTAINER_88, &e2));

        let mut fields = vec![STORY_CATALOG_DECLARED_COUNT_ID, BLOCK_TYPE_U16, 2, 0];
        fields.extend_from_slice(&container(
            STORY_CATALOG_ENTRY_ARRAY_ID,
            BLOCK_TYPE_CONTAINER_A0,
            &array,
        ));

        let mut out = u32::try_from(fields.len() + 4)
            .unwrap()
            .to_le_bytes()
            .to_vec();
        out.extend_from_slice(&fields);
        out
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
    fn synthetic_catalog_preserves_text_id_to_layout_key_mapping() {
        let bytes = synthetic_chunk();
        let chunk = parse_confirmed_0x2c_chunk(StreamPath("/Contents".into()), &bytes, 0).unwrap();
        let catalog = parse_confirmed_mature_story_catalog(&bytes, &chunk).unwrap();

        assert_eq!(catalog.declared_count, 2);
        assert_eq!(
            catalog
                .entries
                .iter()
                .map(|entry| (entry.text_id, entry.layout_key))
                .collect::<Vec<_>>(),
            vec![(6, Some(4)), (9, Some(12))]
        );
    }

    #[test]
    fn apache_sample_table_story_text6_selects_layout_key4() {
        let pub_bytes = decode_base64(include_str!(
            "../../pub-reader/tests/fixtures/Sample.pub.b64"
        ));
        let contents =
            pub_cfb::read_stream_reader(std::io::Cursor::new(pub_bytes.as_slice()), "/Contents")
                .expect("read pinned Apache Sample.pub Contents");
        let stream = StreamPath("/Contents".into());
        let header = parse_0x2c_header(stream.clone(), &contents).expect("0x2c header");
        let trailer = parse_confirmed_0x2c_trailer_root(&contents, &header).expect("0x2c trailer");

        let mut catalogs = Vec::new();
        for seq_num in 0..trailer.directory.slots.len() {
            let Some(reference) =
                parse_confirmed_chunk_reference(&contents, &trailer.directory, seq_num)
                    .expect("chunk reference")
            else {
                continue;
            };
            if !reference
                .raw_types
                .iter()
                .any(|field| field.value == CONTENTS_RAW_TYPE_STORY_CATALOG)
            {
                continue;
            }

            for offset in &reference.chunk_offsets {
                let chunk =
                    parse_confirmed_0x2c_chunk(stream.clone(), &contents, offset.value).unwrap();
                catalogs.push(parse_confirmed_mature_story_catalog(&contents, &chunk).unwrap());
            }
        }

        assert_eq!(catalogs.len(), 1);
        let entry = catalogs[0]
            .entries
            .iter()
            .find(|entry| entry.text_id == 6)
            .expect("Sample table story textId 6");
        assert_eq!(entry.layout_key, Some(4));
        assert_eq!(catalogs[0].entries.len(), 6);
    }
}
