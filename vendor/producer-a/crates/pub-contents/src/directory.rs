use crate::{
    BLOCK_TYPE_CONTAINER_88, BLOCK_TYPE_DUMMY, BlockReadError, ContentsCursor, ContentsReadError,
    RawContentsBlock, parse_confirmed_block,
};
use pub_core::RawSpan;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contents0x2cDirectory {
    pub source: RawSpan,
    pub slots: Vec<Contents0x2cDirectorySlot>,
}

impl Contents0x2cDirectory {
    /// Возвращает слот по его позиционному seqNum.
    ///
    /// В подтверждённом 0x2C-каталоге seqNum — это именно ordinal позиции,
    /// а не отдельное сериализованное поле внутри occupied slot.
    pub fn slot(&self, seq_num: usize) -> Option<&Contents0x2cDirectorySlot> {
        self.slots.get(seq_num)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Contents0x2cDirectorySlot {
    Empty { block: RawContentsBlock },
    Occupied { block: RawContentsBlock },
}

impl Contents0x2cDirectorySlot {
    pub fn source(&self) -> &RawSpan {
        match self {
            Self::Empty { block } | Self::Occupied { block } => &block.source,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectoryReadError {
    Contents(ContentsReadError),
    Block(BlockReadError),
    SpanTooLarge { source: RawSpan },
    UnexpectedSlotId { offset: u64, id: u16 },
    UnexpectedSlotType { offset: u64, block_type: u8 },
}

impl fmt::Display for DirectoryReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contents(error) => error.fmt(f),
            Self::Block(error) => error.fmt(f),
            Self::SpanTooLarge { source } => write!(
                f,
                "диапазон directory Contents не помещается в адресное пространство: offset={}, len={}",
                source.offset, source.len
            ),
            Self::UnexpectedSlotId { offset, id } => write!(
                f,
                "неподтверждённый id слота directory Contents по смещению {offset}: 0x{id:02X}"
            ),
            Self::UnexpectedSlotType { offset, block_type } => write!(
                f,
                "неподтверждённый тип слота directory Contents по смещению {offset}: 0x{block_type:02X}"
            ),
        }
    }
}

impl std::error::Error for DirectoryReadError {}

impl From<ContentsReadError> for DirectoryReadError {
    fn from(value: ContentsReadError) -> Self {
        Self::Contents(value)
    }
}

impl From<BlockReadError> for DirectoryReadError {
    fn from(value: BlockReadError) -> Self {
        Self::Block(value)
    }
}

/// Разбирает только подтверждённую позиционную форму directory семейства 0x2C.
///
/// В текущем foundation подтверждены два вида слотов:
/// - `00 78` — пустая позиция;
/// - `00 88 ...` — занятая позиция.
///
/// Порядок элементов в `slots` является seqNum. Никакое отдельное поле
/// идентичности из occupied slot не синтезируется.
pub fn parse_confirmed_0x2c_directory(
    bytes: &[u8],
    source: RawSpan,
) -> Result<Contents0x2cDirectory, DirectoryReadError> {
    let start = usize::try_from(source.offset).map_err(|_| DirectoryReadError::SpanTooLarge {
        source: source.clone(),
    })?;
    let len = usize::try_from(source.len).map_err(|_| DirectoryReadError::SpanTooLarge {
        source: source.clone(),
    })?;

    let mut cursor = ContentsCursor::bounded(source.stream.clone(), bytes, start, len)?;
    let mut slots = Vec::new();

    while cursor.remaining() > 0 {
        let block = parse_confirmed_block(&mut cursor)?;

        if block.id != 0 {
            return Err(DirectoryReadError::UnexpectedSlotId {
                offset: block.source.offset,
                id: block.id,
            });
        }

        let slot = match block.block_type {
            BLOCK_TYPE_DUMMY => Contents0x2cDirectorySlot::Empty { block },
            BLOCK_TYPE_CONTAINER_88 => Contents0x2cDirectorySlot::Occupied { block },
            block_type => {
                return Err(DirectoryReadError::UnexpectedSlotType {
                    offset: block.source.offset,
                    block_type,
                });
            }
        };

        slots.push(slot);
    }

    Ok(Contents0x2cDirectory { source, slots })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RawContentsBlockBody;
    use pub_core::StreamPath;

    fn directory_source(len: u64) -> RawSpan {
        RawSpan {
            stream: StreamPath("/Contents".into()),
            offset: 0,
            len,
        }
    }

    #[test]
    fn parses_positional_empty_and_occupied_slots() {
        let bytes = [
            0x00, 0x78, // seqNum 0: DUMMY
            0x00, 0x88, 0x0A, 0x00, 0x00, 0x00, // seqNum 1: container
            0x02, 0x20, 0x44, 0x00, 0x00, 0x00, // opaque container content
            0x00, 0x78, // seqNum 2: DUMMY
        ];

        let directory =
            parse_confirmed_0x2c_directory(&bytes, directory_source(bytes.len() as u64))
                .expect("позиционный directory должен читаться");

        assert_eq!(directory.slots.len(), 3);
        assert!(matches!(
            directory.slot(0),
            Some(Contents0x2cDirectorySlot::Empty { .. })
        ));
        assert!(matches!(
            directory.slot(1),
            Some(Contents0x2cDirectorySlot::Occupied { .. })
        ));
        assert!(matches!(
            directory.slot(2),
            Some(Contents0x2cDirectorySlot::Empty { .. })
        ));
        assert!(directory.slot(3).is_none());

        let Contents0x2cDirectorySlot::Occupied { block } = &directory.slots[1] else {
            panic!("seqNum 1 должен быть occupied");
        };
        assert_eq!(block.source.offset, 2);
        assert_eq!(block.source.len, 12);
        assert!(matches!(
            block.body,
            RawContentsBlockBody::Container {
                declared_length: 10,
                ..
            }
        ));
    }

    #[test]
    fn slot_ordinal_is_not_read_from_occupied_payload() {
        let bytes = [
            0x00, 0x78, // ordinal 0
            0x00, 0x78, // ordinal 1
            0x00, 0x88, 0x04, 0x00, 0x00, 0x00, // ordinal 2
        ];

        let directory =
            parse_confirmed_0x2c_directory(&bytes, directory_source(bytes.len() as u64))
                .expect("directory должен читаться");

        assert!(matches!(
            directory.slot(2),
            Some(Contents0x2cDirectorySlot::Occupied { .. })
        ));
    }

    #[test]
    fn rejects_nonzero_slot_id() {
        let bytes = [0x01, 0x78];

        let error = parse_confirmed_0x2c_directory(&bytes, directory_source(2))
            .expect_err("id слота должен оставаться подтверждённым нулём");

        assert_eq!(
            error,
            DirectoryReadError::UnexpectedSlotId { offset: 0, id: 1 }
        );
    }

    #[test]
    fn rejects_other_confirmed_block_type_as_directory_slot() {
        let bytes = [0x00, 0x20, 0x01, 0x00, 0x00, 0x00];

        let error = parse_confirmed_0x2c_directory(&bytes, directory_source(6))
            .expect_err("обычный u32 block не является подтверждённой формой slot");

        assert_eq!(
            error,
            DirectoryReadError::UnexpectedSlotType {
                offset: 0,
                block_type: 0x20,
            }
        );
    }

    #[test]
    fn malformed_occupied_slot_stays_local_to_directory_range() {
        let bytes = [
            0xFF, 0xFF, // bytes before directory
            0x00, 0x88, 0x08, 0x00, 0x00, 0x00, 0xAA, 0xBB, // truncated slot
            0xCC, 0xDD, // bytes after directory
        ];
        let source = RawSpan {
            stream: StreamPath("/Contents".into()),
            offset: 2,
            len: 8,
        };

        let error = parse_confirmed_0x2c_directory(&bytes, source)
            .expect_err("slot не должен выходить за bounded directory");

        assert_eq!(
            error,
            DirectoryReadError::Block(BlockReadError::Contents(ContentsReadError::TooShort {
                offset: 8,
                requested: 4,
                available: 2,
            }))
        );
    }
}
