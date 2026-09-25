use crate::{
    BLOCK_TYPE_CONTAINER_90, BLOCK_TYPE_U32, BlockReadError, Contents0x2cDirectory,
    Contents0x2cHeader, ContentsCursor, ContentsReadError, DirectoryReadError, RawContentsBlock,
    RawContentsBlockBody, parse_confirmed_0x2c_directory, parse_confirmed_block,
};
use pub_core::RawSpan;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const TRAILER_SLOT_COUNT_ID: u16 = 0x01;
pub const TRAILER_MAX_ORDINAL_ID: u16 = 0x02;
pub const TRAILER_DIRECTORY_ID: u16 = 0x03;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contents0x2cTrailerRoot {
    pub source: RawSpan,
    pub declared_length: u32,
    pub declared_length_source: RawSpan,
    pub slot_count: u32,
    pub slot_count_source: RawSpan,
    pub max_ordinal: u32,
    pub max_ordinal_source: RawSpan,
    pub slot_count_block: RawContentsBlock,
    pub max_ordinal_block: RawContentsBlock,
    pub directory_block: RawContentsBlock,
    pub directory: Contents0x2cDirectory,
    /// Непроинтерпретированный хвост после трёх подтверждённых root-блоков.
    ///
    /// В пяти напрямую проверенных PUB такого хвоста нет. Если новый вариант
    /// его содержит, parser сохраняет диапазон как raw evidence и не угадывает
    /// его грамматику.
    pub trailing_source: Option<RawSpan>,
}

impl Contents0x2cTrailerRoot {
    pub fn observed_slot_count_matches_directory(&self) -> bool {
        usize::try_from(self.slot_count)
            .map(|count| count == self.directory.slots.len())
            .unwrap_or(false)
    }

    pub fn observed_max_ordinal_matches_directory(&self) -> bool {
        self.directory
            .slots
            .len()
            .checked_sub(1)
            .and_then(|value| u32::try_from(value).ok())
            .map(|value| value == self.max_ordinal)
            .unwrap_or(false)
    }

    pub fn roots_fill_declared_trailer(&self) -> bool {
        self.trailing_source.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrailerRootReadError {
    Contents(ContentsReadError),
    Block(BlockReadError),
    Directory(DirectoryReadError),
    OffsetTooLarge {
        offset: u32,
    },
    DeclaredLengthTooSmall {
        offset: u32,
        declared_length: u32,
    },
    DeclaredRangeOutOfBounds {
        offset: u32,
        declared_length: u32,
        stream_len: usize,
    },
    UnexpectedRootId {
        index: usize,
        offset: u64,
        id: u16,
        expected: u16,
    },
    UnexpectedRootType {
        index: usize,
        offset: u64,
        block_type: u8,
        expected: u8,
    },
    InconsistentRootBody {
        index: usize,
        offset: u64,
    },
}

impl fmt::Display for TrailerRootReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contents(error) => error.fmt(f),
            Self::Block(error) => error.fmt(f),
            Self::Directory(error) => error.fmt(f),
            Self::OffsetTooLarge { offset } => write!(
                f,
                "смещение trailer Contents не помещается в адресное пространство: {offset}"
            ),
            Self::DeclaredLengthTooSmall {
                offset,
                declared_length,
            } => write!(
                f,
                "declared length trailer по смещению {offset} меньше 4: {declared_length}"
            ),
            Self::DeclaredRangeOutOfBounds {
                offset,
                declared_length,
                stream_len,
            } => write!(
                f,
                "declared trailer выходит за границы Contents: offset={offset}, len={declared_length}, stream_len={stream_len}"
            ),
            Self::UnexpectedRootId {
                index,
                offset,
                id,
                expected,
            } => write!(
                f,
                "неожиданный id trailer root[{index}] по смещению {offset}: 0x{id:02X}, ожидался 0x{expected:02X}"
            ),
            Self::UnexpectedRootType {
                index,
                offset,
                block_type,
                expected,
            } => write!(
                f,
                "неожиданный type trailer root[{index}] по смещению {offset}: 0x{block_type:02X}, ожидался 0x{expected:02X}"
            ),
            Self::InconsistentRootBody { index, offset } => write!(
                f,
                "trailer root[{index}] по смещению {offset} не содержит ожидаемый body"
            ),
        }
    }
}

impl std::error::Error for TrailerRootReadError {}

impl From<ContentsReadError> for TrailerRootReadError {
    fn from(value: ContentsReadError) -> Self {
        Self::Contents(value)
    }
}

impl From<BlockReadError> for TrailerRootReadError {
    fn from(value: BlockReadError) -> Self {
        Self::Block(value)
    }
}

impl From<DirectoryReadError> for TrailerRootReadError {
    fn from(value: DirectoryReadError) -> Self {
        Self::Directory(value)
    }
}

/// Разбирает подтверждённый корень trailer семейства 0x2C по PUB-C-124.
///
/// Подтверждённая физическая форма:
///
/// ```text
/// u32 declared_trailer_length
/// 01:20 <u32>
/// 02:20 <u32>
/// 03:90 <directory>
/// [неизвестный хвост, если встретится в новом варианте]
/// ```
///
/// Функция проверяет границы и exact id/type трёх известных roots, но не
/// превращает наблюдённые отношения slot_count/max_ordinal в универсальные
/// инварианты. Эти отношения доступны через методы `observed_*`.
pub fn parse_confirmed_0x2c_trailer_root(
    bytes: &[u8],
    header: &Contents0x2cHeader,
) -> Result<Contents0x2cTrailerRoot, TrailerRootReadError> {
    let offset = header.trailer_offset;
    let start =
        usize::try_from(offset).map_err(|_| TrailerRootReadError::OffsetTooLarge { offset })?;
    if start > bytes.len() {
        return Err(TrailerRootReadError::DeclaredRangeOutOfBounds {
            offset,
            declared_length: 0,
            stream_len: bytes.len(),
        });
    }

    let stream = header.trailer_offset_source.stream.clone();
    let available = bytes.len().saturating_sub(start);
    let mut leading = ContentsCursor::bounded(stream.clone(), bytes, start, available)?;
    let (declared_length, declared_length_source) = leading.read_u32_le()?;

    if declared_length < 4 {
        return Err(TrailerRootReadError::DeclaredLengthTooSmall {
            offset,
            declared_length,
        });
    }

    let declared_usize = usize::try_from(declared_length).map_err(|_| {
        TrailerRootReadError::DeclaredRangeOutOfBounds {
            offset,
            declared_length,
            stream_len: bytes.len(),
        }
    })?;
    let end = start
        .checked_add(declared_usize)
        .filter(|end| *end <= bytes.len())
        .ok_or(TrailerRootReadError::DeclaredRangeOutOfBounds {
            offset,
            declared_length,
            stream_len: bytes.len(),
        })?;

    let roots_start = start + 4;
    let roots_len = end - roots_start;
    let mut cursor = ContentsCursor::bounded(stream.clone(), bytes, roots_start, roots_len)?;

    let slot_count_block = parse_confirmed_block(&mut cursor)?;
    validate_root(&slot_count_block, 0, TRAILER_SLOT_COUNT_ID, BLOCK_TYPE_U32)?;
    let (slot_count, slot_count_source) = read_u32_root(&slot_count_block, 0)?;

    let max_ordinal_block = parse_confirmed_block(&mut cursor)?;
    validate_root(
        &max_ordinal_block,
        1,
        TRAILER_MAX_ORDINAL_ID,
        BLOCK_TYPE_U32,
    )?;
    let (max_ordinal, max_ordinal_source) = read_u32_root(&max_ordinal_block, 1)?;

    let directory_block = parse_confirmed_block(&mut cursor)?;
    validate_root(
        &directory_block,
        2,
        TRAILER_DIRECTORY_ID,
        BLOCK_TYPE_CONTAINER_90,
    )?;
    let directory_source = match &directory_block.body {
        RawContentsBlockBody::Container { content_source, .. } => content_source.clone(),
        _ => {
            return Err(TrailerRootReadError::InconsistentRootBody {
                index: 2,
                offset: directory_block.source.offset,
            });
        }
    };
    let directory = parse_confirmed_0x2c_directory(bytes, directory_source)?;

    let trailing_source = if cursor.remaining() == 0 {
        None
    } else {
        Some(RawSpan {
            stream,
            offset: cursor.position() as u64,
            len: cursor.remaining() as u64,
        })
    };

    Ok(Contents0x2cTrailerRoot {
        source: RawSpan {
            stream: header.trailer_offset_source.stream.clone(),
            offset: start as u64,
            len: declared_usize as u64,
        },
        declared_length,
        declared_length_source,
        slot_count,
        slot_count_source,
        max_ordinal,
        max_ordinal_source,
        slot_count_block,
        max_ordinal_block,
        directory_block,
        directory,
        trailing_source,
    })
}

fn validate_root(
    block: &RawContentsBlock,
    index: usize,
    expected_id: u16,
    expected_type: u8,
) -> Result<(), TrailerRootReadError> {
    if block.id != expected_id {
        return Err(TrailerRootReadError::UnexpectedRootId {
            index,
            offset: block.source.offset,
            id: block.id,
            expected: expected_id,
        });
    }
    if block.block_type != expected_type {
        return Err(TrailerRootReadError::UnexpectedRootType {
            index,
            offset: block.source.offset,
            block_type: block.block_type,
            expected: expected_type,
        });
    }
    Ok(())
}

fn read_u32_root(
    block: &RawContentsBlock,
    index: usize,
) -> Result<(u32, RawSpan), TrailerRootReadError> {
    match &block.body {
        RawContentsBlockBody::U32 {
            value,
            value_source,
        } => Ok((*value, value_source.clone())),
        _ => Err(TrailerRootReadError::InconsistentRootBody {
            index,
            offset: block.source.offset,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContentsFamily, ContentsPreamble};
    use pub_core::StreamPath;

    fn span(offset: u64, len: u64) -> RawSpan {
        RawSpan {
            stream: StreamPath("/Contents".into()),
            offset,
            len,
        }
    }

    fn header(offset: u32) -> Contents0x2cHeader {
        Contents0x2cHeader {
            preamble: ContentsPreamble {
                family: ContentsFamily::Family0x2c,
                family_source: span(0, 4),
                serialization_revision: 0x0015,
                serialization_revision_source: span(12, 2),
            },
            trailer_offset: offset,
            trailer_offset_source: span(0x1A, 4),
        }
    }

    fn exact_trailer() -> Vec<u8> {
        vec![
            0x20, 0x00, 0x00, 0x00, // declared trailer length = 32
            0x01, 0x20, 0x03, 0x00, 0x00, 0x00, // slot count = 3
            0x02, 0x20, 0x02, 0x00, 0x00, 0x00, // max ordinal = 2
            0x03, 0x90, 0x0E, 0x00, 0x00, 0x00, // directory: 10 bytes content
            0x00, 0x78, // slot 0
            0x00, 0x88, 0x04, 0x00, 0x00, 0x00, // slot 1, empty occupied payload
            0x00, 0x78, // slot 2
        ]
    }

    #[test]
    fn parses_confirmed_three_root_trailer_and_directory() {
        let bytes = exact_trailer();
        let trailer = parse_confirmed_0x2c_trailer_root(&bytes, &header(0))
            .expect("подтверждённый trailer должен читаться");

        assert_eq!(trailer.declared_length, 32);
        assert_eq!(trailer.slot_count, 3);
        assert_eq!(trailer.max_ordinal, 2);
        assert_eq!(trailer.directory.slots.len(), 3);
        assert!(trailer.observed_slot_count_matches_directory());
        assert!(trailer.observed_max_ordinal_matches_directory());
        assert!(trailer.roots_fill_declared_trailer());
        assert_eq!(trailer.source, span(0, 32));
    }

    #[test]
    fn preserves_uninterpreted_trailing_bytes() {
        let mut bytes = exact_trailer();
        bytes[0..4].copy_from_slice(&34_u32.to_le_bytes());
        bytes.extend_from_slice(&[0xAA, 0xBB]);

        let trailer = parse_confirmed_0x2c_trailer_root(&bytes, &header(0))
            .expect("неизвестный хвост должен сохраняться, а не угадываться");

        assert_eq!(trailer.trailing_source, Some(span(32, 2)));
        assert!(!trailer.roots_fill_declared_trailer());
    }

    #[test]
    fn observed_count_relation_is_reported_but_not_forced() {
        let mut bytes = exact_trailer();
        bytes[6..10].copy_from_slice(&4_u32.to_le_bytes());

        let trailer = parse_confirmed_0x2c_trailer_root(&bytes, &header(0))
            .expect("несовпадение count relation не должно менять physical grammar");

        assert_eq!(trailer.slot_count, 4);
        assert!(!trailer.observed_slot_count_matches_directory());
        assert!(trailer.observed_max_ordinal_matches_directory());
    }

    #[test]
    fn rejects_declared_range_past_stream_end() {
        let mut bytes = exact_trailer();
        bytes[0..4].copy_from_slice(&64_u32.to_le_bytes());

        assert_eq!(
            parse_confirmed_0x2c_trailer_root(&bytes, &header(0))
                .expect_err("declared range за Contents должен быть ошибкой"),
            TrailerRootReadError::DeclaredRangeOutOfBounds {
                offset: 0,
                declared_length: 64,
                stream_len: 32,
            }
        );
    }

    #[test]
    fn rejects_unexpected_root_identity() {
        let mut bytes = exact_trailer();
        bytes[4] = 0x09;

        assert_eq!(
            parse_confirmed_0x2c_trailer_root(&bytes, &header(0))
                .expect_err("другой root id не должен повышаться до PUB-C-124"),
            TrailerRootReadError::UnexpectedRootId {
                index: 0,
                offset: 4,
                id: 0x09,
                expected: TRAILER_SLOT_COUNT_ID,
            }
        );
    }
}
