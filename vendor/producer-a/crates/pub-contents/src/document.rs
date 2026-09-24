use crate::{
    BLOCK_TYPE_CONTAINER_A0, BLOCK_TYPE_HANDLE_U32, BLOCK_TYPE_U32, BlockReadError, ContentsCursor,
    ContentsReadError, RawContentsBlock, RawContentsBlockBody, parse_confirmed_block,
};
use pub_core::RawSpan;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const DOCUMENT_PAGE_LIST_ID: u8 = 0x02;
pub const DOCUMENT_DW_NEXT_UNIQUE_OID_ID: u8 = 0x23;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentPageListEntry {
    /// Физический handle из wire-записи 00:70 + u32.
    ///
    /// Это не PageID и не гарантированно handle на chunk типа PAGE:
    /// в подтверждённых Publisher 2002/2003 fixtures список содержит raw0x59.
    pub handle: u32,
    pub handle_source: RawSpan,
    pub block: RawContentsBlock,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentPageList {
    /// Исходный 02:A0 block целиком.
    pub block: RawContentsBlock,
    /// Упорядоченные физические handles без semantic filtering.
    pub entries: Vec<DocumentPageListEntry>,
}

impl DocumentPageList {
    pub fn handles(&self) -> impl Iterator<Item = u32> + '_ {
        self.entries.iter().map(|entry| entry.handle)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentDwNextUniqueOid {
    /// Значение Microsoft-origin поля DOCUMENT.DwNextUniqueOid.
    ///
    /// Тип намеренно не называет это значение allocator-правилом: связь с
    /// конкретным компонентом Oid остаётся отдельной исследовательской задачей.
    pub value: u32,
    pub value_source: RawSpan,
    pub block: RawContentsBlock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentDwNextUniqueOidReadError {
    UnexpectedId { offset: u64, id: u8 },
    UnexpectedType { offset: u64, block_type: u8 },
    InconsistentBody { offset: u64 },
}

impl fmt::Display for DocumentDwNextUniqueOidReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedId { offset, id } => write!(
                f,
                "неподтверждённый id DOCUMENT.DwNextUniqueOid по смещению {offset}: 0x{id:02X}"
            ),
            Self::UnexpectedType { offset, block_type } => write!(
                f,
                "неподтверждённый wire type DOCUMENT.DwNextUniqueOid по смещению {offset}: 0x{block_type:02X}"
            ),
            Self::InconsistentBody { offset } => write!(
                f,
                "DOCUMENT.DwNextUniqueOid по смещению {offset} не содержит u32 body"
            ),
        }
    }
}

impl std::error::Error for DocumentDwNextUniqueOidReadError {}

/// Поднимает только подтверждённую физическую форму DOCUMENT field0x23.
///
/// Microsoft-generated Publisher metadata называет поле DwNextUniqueOid, а
/// закрытый priv/wire Rosetta относит его к fixed-u32 классу. Функция не
/// выводит из значения политику выделения Oid и не изменяет его.
pub fn parse_confirmed_document_dw_next_unique_oid(
    block: RawContentsBlock,
) -> Result<DocumentDwNextUniqueOid, DocumentDwNextUniqueOidReadError> {
    if block.id != DOCUMENT_DW_NEXT_UNIQUE_OID_ID {
        return Err(DocumentDwNextUniqueOidReadError::UnexpectedId {
            offset: block.source.offset,
            id: block.id,
        });
    }
    if block.block_type != BLOCK_TYPE_U32 {
        return Err(DocumentDwNextUniqueOidReadError::UnexpectedType {
            offset: block.source.offset,
            block_type: block.block_type,
        });
    }

    let (value, value_source) = match &block.body {
        RawContentsBlockBody::U32 {
            value,
            value_source,
        } => (*value, value_source.clone()),
        _ => {
            return Err(DocumentDwNextUniqueOidReadError::InconsistentBody {
                offset: block.source.offset,
            });
        }
    };

    Ok(DocumentDwNextUniqueOid {
        value,
        value_source,
        block,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentPageListReadError {
    Contents(ContentsReadError),
    Block(BlockReadError),
    SpanTooLarge { source: RawSpan },
    UnexpectedOuterId { offset: u64, id: u8 },
    UnexpectedOuterType { offset: u64, block_type: u8 },
    InconsistentOuterBody,
    UnexpectedEntryId { offset: u64, id: u8 },
    UnexpectedEntryType { offset: u64, block_type: u8 },
    InconsistentEntryBody { offset: u64 },
}

impl fmt::Display for DocumentPageListReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contents(error) => error.fmt(f),
            Self::Block(error) => error.fmt(f),
            Self::SpanTooLarge { source } => write!(
                f,
                "диапазон DOCUMENT PageList не помещается в адресное пространство: offset={}, len={}",
                source.offset, source.len
            ),
            Self::UnexpectedOuterId { offset, id } => write!(
                f,
                "неподтверждённый id DOCUMENT PageList по смещению {offset}: 0x{id:02X}"
            ),
            Self::UnexpectedOuterType { offset, block_type } => write!(
                f,
                "неподтверждённый wire type DOCUMENT PageList по смещению {offset}: 0x{block_type:02X}"
            ),
            Self::InconsistentOuterBody => {
                write!(f, "DOCUMENT PageList 02:A0 не содержит container body")
            }
            Self::UnexpectedEntryId { offset, id } => write!(
                f,
                "неподтверждённый id элемента DOCUMENT PageList по смещению {offset}: 0x{id:02X}"
            ),
            Self::UnexpectedEntryType { offset, block_type } => write!(
                f,
                "неподтверждённый wire type элемента DOCUMENT PageList по смещению {offset}: 0x{block_type:02X}"
            ),
            Self::InconsistentEntryBody { offset } => write!(
                f,
                "элемент DOCUMENT PageList по смещению {offset} не содержит u32 handle"
            ),
        }
    }
}

impl std::error::Error for DocumentPageListReadError {}

impl From<ContentsReadError> for DocumentPageListReadError {
    fn from(value: ContentsReadError) -> Self {
        Self::Contents(value)
    }
}

impl From<BlockReadError> for DocumentPageListReadError {
    fn from(value: BlockReadError) -> Self {
        Self::Block(value)
    }
}

/// Разбирает уже найденный DOCUMENT field0x02 только по подтверждённой
/// физической грамматике OBS-RS-002.
///
/// Поддержанный wire:
///
/// 02 A0 <u32 declared = 4 + 6*N>
///   00 70 <u32 handle>
///   ...
///
/// Функция намеренно не разрешает handle в PAGE/0x59/другой chunk type и
/// не называет последовательность видимым порядком страниц.
pub fn parse_confirmed_document_page_list(
    bytes: &[u8],
    block: RawContentsBlock,
) -> Result<DocumentPageList, DocumentPageListReadError> {
    if block.id != DOCUMENT_PAGE_LIST_ID {
        return Err(DocumentPageListReadError::UnexpectedOuterId {
            offset: block.source.offset,
            id: block.id,
        });
    }
    if block.block_type != BLOCK_TYPE_CONTAINER_A0 {
        return Err(DocumentPageListReadError::UnexpectedOuterType {
            offset: block.source.offset,
            block_type: block.block_type,
        });
    }

    let content_source = match &block.body {
        RawContentsBlockBody::Container { content_source, .. } => content_source.clone(),
        _ => return Err(DocumentPageListReadError::InconsistentOuterBody),
    };

    let start = usize::try_from(content_source.offset).map_err(|_| {
        DocumentPageListReadError::SpanTooLarge {
            source: content_source.clone(),
        }
    })?;
    let len = usize::try_from(content_source.len).map_err(|_| {
        DocumentPageListReadError::SpanTooLarge {
            source: content_source.clone(),
        }
    })?;

    let mut cursor = ContentsCursor::bounded(content_source.stream.clone(), bytes, start, len)?;
    let mut entries = Vec::new();

    while cursor.remaining() > 0 {
        let entry_block = parse_confirmed_block(&mut cursor)?;

        if entry_block.id != 0 {
            return Err(DocumentPageListReadError::UnexpectedEntryId {
                offset: entry_block.source.offset,
                id: entry_block.id,
            });
        }
        if entry_block.block_type != BLOCK_TYPE_HANDLE_U32 {
            return Err(DocumentPageListReadError::UnexpectedEntryType {
                offset: entry_block.source.offset,
                block_type: entry_block.block_type,
            });
        }

        let (handle, handle_source) = match &entry_block.body {
            RawContentsBlockBody::U32 {
                value,
                value_source,
            } => (*value, value_source.clone()),
            _ => {
                return Err(DocumentPageListReadError::InconsistentEntryBody {
                    offset: entry_block.source.offset,
                });
            }
        };

        entries.push(DocumentPageListEntry {
            handle,
            handle_source,
            block: entry_block,
        });
    }

    Ok(DocumentPageList { block, entries })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pub_core::StreamPath;

    fn parse_outer(bytes: &[u8]) -> RawContentsBlock {
        let mut cursor = ContentsCursor::new(StreamPath("/Contents".into()), bytes);
        parse_confirmed_block(&mut cursor).expect("outer PageList должен читаться")
    }

    #[test]
    fn parses_confirmed_dw_next_unique_oid_as_u32_with_provenance() {
        let bytes = [0x23, 0x20, 0x02, 0x00, 0x00, 0x00];
        let mut cursor = ContentsCursor::new(StreamPath("/Contents".into()), &bytes);
        let block = parse_confirmed_block(&mut cursor)
            .expect("DwNextUniqueOid block должен физически читаться");

        let field = parse_confirmed_document_dw_next_unique_oid(block)
            .expect("field0x23/type0x20 должен приниматься как DwNextUniqueOid");

        assert_eq!(field.value, 2);
        assert_eq!(field.value_source.offset, 2);
        assert_eq!(field.value_source.len, 4);
        assert_eq!(field.block.id, DOCUMENT_DW_NEXT_UNIQUE_OID_ID);
    }

    #[test]
    fn dw_next_unique_oid_rejects_handle_u32_wire() {
        let bytes = [0x23, 0x70, 0x02, 0x00, 0x00, 0x00];
        let mut cursor = ContentsCursor::new(StreamPath("/Contents".into()), &bytes);
        let block = parse_confirmed_block(&mut cursor)
            .expect("handle-u32 block должен оставаться физически читаемым");

        assert_eq!(
            parse_confirmed_document_dw_next_unique_oid(block)
                .expect_err("handle wire нельзя повышать до DwNextUniqueOid"),
            DocumentDwNextUniqueOidReadError::UnexpectedType {
                offset: 0,
                block_type: BLOCK_TYPE_HANDLE_U32,
            }
        );
    }

    #[test]
    fn parses_exact_observed_wire_and_preserves_entry_provenance() {
        let bytes = [
            0x02, 0xA0, 0x10, 0x00, 0x00, 0x00, 0x00, 0x70, 0x07, 0x01, 0x00, 0x00, 0x00, 0x70,
            0x0A, 0x01, 0x00, 0x00,
        ];
        let page_list = parse_confirmed_document_page_list(&bytes, parse_outer(&bytes))
            .expect("PageList должен читаться");

        assert_eq!(page_list.handles().collect::<Vec<_>>(), vec![263, 266]);
        assert_eq!(page_list.entries[0].block.source.offset, 6);
        assert_eq!(page_list.entries[0].block.source.len, 6);
        assert_eq!(page_list.entries[0].handle_source.offset, 8);
        assert_eq!(page_list.entries[0].handle_source.len, 4);
        assert_eq!(page_list.entries[1].block.source.offset, 12);
    }

    #[test]
    fn does_not_filter_non_page_targets() {
        let bytes = [
            0x02, 0xA0, 0x10, 0x00, 0x00, 0x00, 0x00, 0x70, 0x0A, 0x01, 0x00, 0x00, 0x00, 0x70,
            0x10, 0x01, 0x00, 0x00,
        ];
        let page_list = parse_confirmed_document_page_list(&bytes, parse_outer(&bytes))
            .expect("физический reader не должен фильтровать handles по target type");

        assert_eq!(page_list.handles().collect::<Vec<_>>(), vec![266, 272]);
    }

    #[test]
    fn rejects_other_outer_id() {
        let bytes = [0x03, 0xA0, 0x04, 0x00, 0x00, 0x00];
        let error = parse_confirmed_document_page_list(&bytes, parse_outer(&bytes))
            .expect_err("другой outer id не должен приниматься как PageList");

        assert_eq!(
            error,
            DocumentPageListReadError::UnexpectedOuterId { offset: 0, id: 3 }
        );
    }

    #[test]
    fn rejects_other_child_wire_without_losing_raw_boundary() {
        let bytes = [
            0x02, 0xA0, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x20, 0x07, 0x01, 0x00, 0x00,
        ];
        let error = parse_confirmed_document_page_list(&bytes, parse_outer(&bytes))
            .expect_err("обычный 0x20 u32 не является подтверждённым PageList child wire");

        assert_eq!(
            error,
            DocumentPageListReadError::UnexpectedEntryType {
                offset: 6,
                block_type: 0x20,
            }
        );
    }

    #[test]
    fn malformed_child_is_bounded_by_outer_container() {
        let bytes = [
            0x02, 0xA0, 0x09, 0x00, 0x00, 0x00, 0x00, 0x70, 0x07, 0x01, 0x00, 0xFF, 0xFF,
        ];
        let outer = parse_outer(&bytes);
        let error = parse_confirmed_document_page_list(&bytes, outer)
            .expect_err("child не должен читать за content boundary");

        assert!(matches!(
            error,
            DocumentPageListReadError::Block(BlockReadError::Contents(
                ContentsReadError::TooShort { .. }
            ))
        ));
    }
}
