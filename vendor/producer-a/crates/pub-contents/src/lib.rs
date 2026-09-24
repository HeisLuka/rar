use pub_core::{RawSpan, StreamPath};
use serde::{Deserialize, Serialize};
use std::fmt;

mod block;
mod cells;
mod chunk;
mod directory;
mod document;
mod document_write;
mod fkp;
mod identity;
mod legacy22;
mod legacy22_cell;
mod legacy22_chpx;
mod legacy22_formatting;
mod legacy22_papx;
mod legacy22_table;
mod legacy22_table_object;
mod legacy22_text_info;
mod margins;
mod reference;
mod story_catalog;
mod trailer;

pub use block::{
    BLOCK_TYPE_CONTAINER_88, BLOCK_TYPE_CONTAINER_90, BLOCK_TYPE_CONTAINER_A0, BLOCK_TYPE_DUMMY,
    BLOCK_TYPE_EMPTY, BLOCK_TYPE_FIXED_8, BLOCK_TYPE_FIXED_16, BLOCK_TYPE_HANDLE_U32,
    BLOCK_TYPE_REFERENCE_U32, BLOCK_TYPE_U16, BLOCK_TYPE_U16_SERVICE, BLOCK_TYPE_U32,
    BlockReadError, RawContentsBlock, RawContentsBlockBody, parse_confirmed_block,
};
pub use cells::{
    CELL_END_COLUMN_ID, CELL_END_ROW_ID, CELL_START_COLUMN_ID, CELL_START_ROW_ID,
    CELLS_DECLARED_COUNT_ID, CELLS_RECORD_ARRAY_ID, CONTENTS_RAW_TYPE_CELLS, CellsReadError,
    MatureCellCoordinates, MatureCellRecord, MatureCellsChunk, ObservedCellCount,
    ObservedCellScalar, parse_confirmed_mature_cells,
};
pub use chunk::{ChunkReadError, Contents0x2cChunk, parse_confirmed_0x2c_chunk};
pub use directory::{
    Contents0x2cDirectory, Contents0x2cDirectorySlot, DirectoryReadError,
    parse_confirmed_0x2c_directory,
};
pub use document::{
    DOCUMENT_DW_NEXT_UNIQUE_OID_ID, DOCUMENT_PAGE_LIST_ID, DocumentDwNextUniqueOid,
    DocumentDwNextUniqueOidReadError, DocumentPageList, DocumentPageListEntry,
    DocumentPageListReadError, parse_confirmed_document_dw_next_unique_oid,
    parse_confirmed_document_page_list,
};
pub use document_write::{
    DocumentSequenceHandlePatch, DocumentSequencePermutationError, DocumentSequencePermutationPlan,
    apply_confirmed_document_sequence_permutation, plan_confirmed_document_sequence_permutation,
};
pub use fkp::{
    FKP_PAGE_SIZE, FkpPage, FkpProperty, FkpPropertyKind, FkpReadError, FkpRun, parse_fkp_page,
};
pub use identity::{
    OidIdentityPayload, OidIdentityReadError, parse_confirmed_oid_identity_payload,
};
pub use legacy22::{
    LEGACY_0X22_DESCRIPTOR_DELTA, LEGACY_0X22_HEADER_POINTER_OFFSET, LEGACY_FORMATTING_PAGE_SIZE,
    Legacy0x22FormattingDescriptor, Legacy0x22ReadError, parse_legacy_0x22_formatting_descriptor,
};
pub use legacy22_cell::{
    LEGACY_0X22_CELL_HORIZONTAL_MERGE_CONTINUATION_FLAG,
    LEGACY_0X22_CELL_HORIZONTAL_MERGE_START_FLAG, Legacy0x22CellBorder, Legacy0x22CellStyle,
    Legacy0x22CellStyleError, decode_legacy_0x22_cell_style,
};
pub use legacy22_chpx::{
    Legacy0x22CharacterStyle, Legacy0x22CharacterStyleError, Legacy0x22Underline,
    decode_legacy_0x22_character_style,
};
pub use legacy22_formatting::{
    Legacy0x22CellStyleBoundary, Legacy0x22CharacterRun, Legacy0x22FormattingRuns,
    Legacy0x22FormattingRunsError, Legacy0x22ParagraphRun, parse_legacy_0x22_formatting_runs,
};
pub use legacy22_papx::{
    Legacy0x22LineSpacing, Legacy0x22ParagraphAlignment, Legacy0x22ParagraphScalarStyle,
    Legacy0x22ParagraphSpecialStyle, Legacy0x22ParagraphStyle, Legacy0x22ParagraphStyleError,
    decode_legacy_0x22_paragraph_style,
};
pub use legacy22_table::{
    Legacy0x22TableCellSlice, Legacy0x22TableSlice, Legacy0x22TableTextMap,
    Legacy0x22TableTextReadError, parse_legacy_0x22_table_text_map,
};
pub use legacy22_table_object::{
    LEGACY_0X22_TABLE_CHUNK_TYPE, LEGACY_0X22_TRAILER_POINTER_OFFSET, Legacy0x22HorizontalMerge,
    Legacy0x22ResolvedTable, Legacy0x22ResolvedTableCatalog, Legacy0x22TableAxisSegment,
    Legacy0x22TableCatalog, Legacy0x22TableCatalogReadError, Legacy0x22TableChunk,
    Legacy0x22TableListHeader, parse_legacy_0x22_resolved_tables, parse_legacy_0x22_table_catalog,
};
pub use legacy22_text_info::{
    LEGACY_0X22_TEXT_INFO_CHUNK_TYPE, Legacy0x22TextInfoMap, Legacy0x22TextInfoOwnerEnd,
    Legacy0x22TextInfoReadError, parse_legacy_0x22_text_info_map,
};
pub use margins::{
    MARGINS_PAGE_EXTENT_ID, MARGINS_PAGE_HEIGHT_ID, MARGINS_PAGE_WIDTH_ID, MarginsPageExtent,
    MarginsPageExtentReadError, parse_confirmed_margins_page_extent,
};
pub use reference::{
    CHUNK_REFERENCE_OFFSET_ID, CHUNK_REFERENCE_PARENT_SEQ_NUM_ID, CHUNK_REFERENCE_RAW_TYPE_ID,
    CHUNK_REFERENCE_WIRE_EMPTY, CHUNK_REFERENCE_WIRE_OFFSET, CHUNK_REFERENCE_WIRE_PARENT_SEQ_NUM,
    CHUNK_REFERENCE_WIRE_U16, CHUNK_REFERENCE_WIRE_U16_SERVICE, ChunkReferenceReadError,
    Contents0x2cChunkReference, ObservedU16Field, ObservedU32Field,
    parse_confirmed_chunk_reference,
};
pub use story_catalog::{
    CONTENTS_RAW_TYPE_STORY_CATALOG, MatureStoryCatalog, MatureStoryCatalogEntry,
    STORY_CATALOG_DECLARED_COUNT_ID, STORY_CATALOG_ENTRY_ARRAY_ID,
    STORY_CATALOG_ENTRY_LAYOUT_KEY_ID, STORY_CATALOG_ENTRY_TEXT_ID, StoryCatalogReadError,
    parse_confirmed_mature_story_catalog,
};
pub use trailer::{
    Contents0x2cTrailerRoot, TRAILER_DIRECTORY_ID, TRAILER_MAX_ORDINAL_ID, TRAILER_SLOT_COUNT_ID,
    TrailerRootReadError, parse_confirmed_0x2c_trailer_root,
};

pub const CONTENTS_0X22_MAGIC: [u8; 4] = [0xE8, 0xAC, 0x22, 0x00];
pub const CONTENTS_0X2C_MAGIC: [u8; 4] = [0xE8, 0xAC, 0x2C, 0x00];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentsFamily {
    Family0x22,
    Family0x2c,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentsPreamble {
    pub family: ContentsFamily,
    pub family_source: RawSpan,
    pub serialization_revision: u16,
    pub serialization_revision_source: RawSpan,
}

/// Подтверждённые физические поля заголовка семейства 0x2C.
///
/// Этот тип намеренно отдельный от общего `ContentsPreamble`: у семейства
/// 0x22 указатель на trailer находится по другому смещению.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contents0x2cHeader {
    pub preamble: ContentsPreamble,
    pub trailer_offset: u32,
    pub trailer_offset_source: RawSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentsReadError {
    TooShort {
        offset: usize,
        requested: usize,
        available: usize,
    },
    UnsupportedMagic([u8; 4]),
    UnexpectedFamily {
        expected: ContentsFamily,
        found: ContentsFamily,
    },
    TrailerOffsetOutOfBounds {
        offset: u32,
        stream_len: usize,
    },
}

impl fmt::Display for ContentsReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort {
                offset,
                requested,
                available,
            } => write!(
                f,
                "недостаточно байтов Contents: смещение {offset}, запрошено {requested}, доступно {available}"
            ),
            Self::UnsupportedMagic(found) => write!(
                f,
                "неподдерживаемый маркер Contents: {:02X} {:02X} {:02X} {:02X}",
                found[0], found[1], found[2], found[3]
            ),
            Self::UnexpectedFamily { expected, found } => write!(
                f,
                "неожиданное семейство Contents: ожидалось {expected:?}, найдено {found:?}"
            ),
            Self::TrailerOffsetOutOfBounds { offset, stream_len } => write!(
                f,
                "указатель trailer Contents выходит за границы потока: смещение {offset}, длина потока {stream_len}"
            ),
        }
    }
}

impl std::error::Error for ContentsReadError {}

/// Определяет только бинарное семейство Contents.
///
/// Возвращаемое значение нельзя трактовать как точную маркетинговую версию
/// Microsoft Publisher.
pub fn detect_family(bytes: &[u8]) -> Result<ContentsFamily, ContentsReadError> {
    let found = read_magic(bytes)?;

    match found {
        CONTENTS_0X22_MAGIC => Ok(ContentsFamily::Family0x22),
        CONTENTS_0X2C_MAGIC => Ok(ContentsFamily::Family0x2c),
        other => Err(ContentsReadError::UnsupportedMagic(other)),
    }
}

/// Читает только подтверждённые поля физического префикса Contents.
///
/// `serialization_revision` описывает ревизию сериализации и не является
/// точной маркетинговой версией Publisher.
pub fn parse_preamble(
    stream: StreamPath,
    bytes: &[u8],
) -> Result<ContentsPreamble, ContentsReadError> {
    let mut cursor = ContentsCursor::new(stream, bytes);

    let (magic, family_source) = cursor.take(4)?;
    let family = detect_family(magic)?;

    cursor.take(8)?;
    let (serialization_revision, serialization_revision_source) = cursor.read_u16_le()?;

    Ok(ContentsPreamble {
        family,
        family_source,
        serialization_revision,
        serialization_revision_source,
    })
}

/// Читает подтверждённую физическую часть заголовка семейства 0x2C.
///
/// В проверенном 0x2C-корпусе little-endian u32 по `Contents+0x1A` указывает
/// на начало trailer. Здесь значение только извлекается и проверяется на
/// попадание внутрь исходного потока; внутренняя грамматика trailer этим
/// вызовом не интерпретируется.
pub fn parse_0x2c_header(
    stream: StreamPath,
    bytes: &[u8],
) -> Result<Contents0x2cHeader, ContentsReadError> {
    let preamble = parse_preamble(stream.clone(), bytes)?;
    if preamble.family != ContentsFamily::Family0x2c {
        return Err(ContentsReadError::UnexpectedFamily {
            expected: ContentsFamily::Family0x2c,
            found: preamble.family,
        });
    }

    let mut cursor = ContentsCursor::new(stream, bytes);
    cursor.take(0x1A)?;
    let (trailer_offset, trailer_offset_source) = cursor.read_u32_le()?;

    if u64::from(trailer_offset) >= bytes.len() as u64 {
        return Err(ContentsReadError::TrailerOffsetOutOfBounds {
            offset: trailer_offset,
            stream_len: bytes.len(),
        });
    }

    Ok(Contents0x2cHeader {
        preamble,
        trailer_offset,
        trailer_offset_source,
    })
}

fn read_magic(bytes: &[u8]) -> Result<[u8; 4], ContentsReadError> {
    if bytes.len() < 4 {
        return Err(ContentsReadError::TooShort {
            offset: 0,
            requested: 4,
            available: bytes.len(),
        });
    }

    Ok([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Проверяемый курсор по одному потоку Contents.
///
/// Курсор ничего не знает о семантике записей. Его задача — не позволять
/// декодерам читать за границы входа и для каждого чтения возвращать точный
/// диапазон исходных байтов.
#[derive(Debug, Clone)]
pub struct ContentsCursor<'a> {
    stream: StreamPath,
    bytes: &'a [u8],
    position: usize,
    limit: usize,
}

impl<'a> ContentsCursor<'a> {
    pub fn new(stream: StreamPath, bytes: &'a [u8]) -> Self {
        Self {
            stream,
            bytes,
            position: 0,
            limit: bytes.len(),
        }
    }

    /// Создаёт курсор, жёстко ограниченный заданным диапазоном исходного потока.
    ///
    /// Позиции и RawSpan остаются абсолютными относительно исходного Contents.
    pub fn bounded(
        stream: StreamPath,
        bytes: &'a [u8],
        start: usize,
        len: usize,
    ) -> Result<Self, ContentsReadError> {
        let end = start
            .checked_add(len)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| ContentsReadError::TooShort {
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

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn remaining(&self) -> usize {
        self.limit.saturating_sub(self.position)
    }

    pub fn read_u8(&mut self) -> Result<(u8, RawSpan), ContentsReadError> {
        let (bytes, source) = self.take(1)?;
        Ok((bytes[0], source))
    }

    pub fn read_u16_le(&mut self) -> Result<(u16, RawSpan), ContentsReadError> {
        let (bytes, source) = self.take(2)?;
        Ok((u16::from_le_bytes([bytes[0], bytes[1]]), source))
    }

    pub fn read_u32_le(&mut self) -> Result<(u32, RawSpan), ContentsReadError> {
        let (bytes, source) = self.take(4)?;
        Ok((
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            source,
        ))
    }

    pub fn take(&mut self, len: usize) -> Result<(&'a [u8], RawSpan), ContentsReadError> {
        let start = self.position;
        let end = start
            .checked_add(len)
            .filter(|end| *end <= self.limit)
            .ok_or_else(|| ContentsReadError::TooShort {
                offset: start,
                requested: len,
                available: self.remaining(),
            })?;

        let source = RawSpan {
            stream: self.stream.clone(),
            offset: start as u64,
            len: len as u64,
        };

        self.position = end;
        Ok((&self.bytes[start..end], source))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_family_markers_are_recognized() {
        assert_eq!(
            detect_family(&CONTENTS_0X22_MAGIC),
            Ok(ContentsFamily::Family0x22)
        );
        assert_eq!(
            detect_family(&CONTENTS_0X2C_MAGIC),
            Ok(ContentsFamily::Family0x2c)
        );
    }

    #[test]
    fn family_marker_is_not_a_partial_match() {
        assert_eq!(
            detect_family(&[0xE8, 0xAC, 0x2C, 0x01]),
            Err(ContentsReadError::UnsupportedMagic([
                0xE8, 0xAC, 0x2C, 0x01
            ]))
        );
        assert_eq!(
            detect_family(&[0xE8, 0xAD, 0x2C, 0x00]),
            Err(ContentsReadError::UnsupportedMagic([
                0xE8, 0xAD, 0x2C, 0x00
            ]))
        );
    }

    #[test]
    fn short_input_is_rejected_without_guessing() {
        assert_eq!(
            detect_family(&[0xE8, 0xAC, 0x2C]),
            Err(ContentsReadError::TooShort {
                offset: 0,
                requested: 4,
                available: 3,
            })
        );
    }

    #[test]
    fn preamble_reads_family_and_serialization_revision_with_provenance() {
        let stream = StreamPath("/Contents".into());
        let mut bytes = vec![0; 14];
        bytes[0..4].copy_from_slice(&CONTENTS_0X2C_MAGIC);
        bytes[12..14].copy_from_slice(&0x001Au16.to_le_bytes());

        let preamble =
            parse_preamble(stream.clone(), &bytes).expect("префикс Contents должен читаться");

        assert_eq!(preamble.family, ContentsFamily::Family0x2c);
        assert_eq!(preamble.serialization_revision, 0x001A);
        assert_eq!(
            preamble.family_source,
            RawSpan {
                stream: stream.clone(),
                offset: 0,
                len: 4,
            }
        );
        assert_eq!(
            preamble.serialization_revision_source,
            RawSpan {
                stream,
                offset: 12,
                len: 2,
            }
        );
    }

    #[test]
    fn preamble_keeps_old_family_revision_family_scoped() {
        let mut bytes = vec![0; 14];
        bytes[0..4].copy_from_slice(&CONTENTS_0X22_MAGIC);
        bytes[12..14].copy_from_slice(&0x02CDu16.to_le_bytes());

        let preamble = parse_preamble(StreamPath("/Contents".into()), &bytes)
            .expect("префикс старого Contents должен читаться");

        assert_eq!(preamble.family, ContentsFamily::Family0x22);
        assert_eq!(preamble.serialization_revision, 0x02CD);
    }

    #[test]
    fn short_preamble_reports_revision_boundary() {
        let mut bytes = vec![0; 13];
        bytes[0..4].copy_from_slice(&CONTENTS_0X2C_MAGIC);

        assert_eq!(
            parse_preamble(StreamPath("/Contents".into()), &bytes),
            Err(ContentsReadError::TooShort {
                offset: 12,
                requested: 2,
                available: 1,
            })
        );
    }

    #[test]
    fn header_0x2c_reads_trailer_pointer_with_provenance() {
        let stream = StreamPath("/Contents".into());
        let mut bytes = vec![0; 64];
        bytes[0..4].copy_from_slice(&CONTENTS_0X2C_MAGIC);
        bytes[12..14].copy_from_slice(&0x0018u16.to_le_bytes());
        bytes[0x1A..0x1E].copy_from_slice(&40u32.to_le_bytes());

        let header =
            parse_0x2c_header(stream.clone(), &bytes).expect("заголовок 0x2C должен читаться");

        assert_eq!(header.preamble.family, ContentsFamily::Family0x2c);
        assert_eq!(header.preamble.serialization_revision, 0x0018);
        assert_eq!(header.trailer_offset, 40);
        assert_eq!(
            header.trailer_offset_source,
            RawSpan {
                stream,
                offset: 0x1A,
                len: 4,
            }
        );
    }

    #[test]
    fn header_0x2c_rejects_old_family() {
        let mut bytes = vec![0; 64];
        bytes[0..4].copy_from_slice(&CONTENTS_0X22_MAGIC);
        bytes[12..14].copy_from_slice(&0x02CDu16.to_le_bytes());

        assert_eq!(
            parse_0x2c_header(StreamPath("/Contents".into()), &bytes),
            Err(ContentsReadError::UnexpectedFamily {
                expected: ContentsFamily::Family0x2c,
                found: ContentsFamily::Family0x22,
            })
        );
    }

    #[test]
    fn header_0x2c_rejects_trailer_pointer_outside_stream() {
        let mut bytes = vec![0; 64];
        bytes[0..4].copy_from_slice(&CONTENTS_0X2C_MAGIC);
        bytes[12..14].copy_from_slice(&0x0018u16.to_le_bytes());
        bytes[0x1A..0x1E].copy_from_slice(&64u32.to_le_bytes());

        assert_eq!(
            parse_0x2c_header(StreamPath("/Contents".into()), &bytes),
            Err(ContentsReadError::TrailerOffsetOutOfBounds {
                offset: 64,
                stream_len: 64,
            })
        );
    }

    #[test]
    fn cursor_tracks_exact_source_ranges() {
        let stream = StreamPath("/Contents".into());
        let mut cursor = ContentsCursor::new(stream.clone(), &[0x11, 0x22, 0x33, 0x44]);

        let (first, first_span) = cursor.read_u16_le().expect("u16 должен читаться");
        assert_eq!(first, 0x2211);
        assert_eq!(
            first_span,
            RawSpan {
                stream: stream.clone(),
                offset: 0,
                len: 2,
            }
        );

        let (second, second_span) = cursor.read_u8().expect("u8 должен читаться");
        assert_eq!(second, 0x33);
        assert_eq!(
            second_span,
            RawSpan {
                stream,
                offset: 2,
                len: 1,
            }
        );
        assert_eq!(cursor.position(), 3);
        assert_eq!(cursor.remaining(), 1);
    }

    #[test]
    fn cursor_never_advances_after_out_of_bounds_read() {
        let mut cursor = ContentsCursor::new(StreamPath("/Contents".into()), &[1, 2, 3]);

        let error = cursor
            .read_u32_le()
            .expect_err("чтение за границей должно завершаться ошибкой");

        assert_eq!(
            error,
            ContentsReadError::TooShort {
                offset: 0,
                requested: 4,
                available: 3,
            }
        );
        assert_eq!(cursor.position(), 0);
    }
}
