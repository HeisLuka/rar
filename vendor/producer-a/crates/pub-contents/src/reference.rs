use crate::{
    BlockReadError, Contents0x2cDirectory, Contents0x2cDirectorySlot, ContentsCursor,
    ContentsReadError, RawContentsBlock, RawContentsBlockBody, parse_confirmed_block,
};
use pub_core::RawSpan;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const CHUNK_REFERENCE_RAW_TYPE_ID: u8 = 0x02;
pub const CHUNK_REFERENCE_OFFSET_ID: u8 = 0x04;
pub const CHUNK_REFERENCE_PARENT_SEQ_NUM_ID: u8 = 0x05;

pub const CHUNK_REFERENCE_WIRE_EMPTY: u8 = 0x08;
pub const CHUNK_REFERENCE_WIRE_U16_SERVICE: u8 = 0x10;
pub const CHUNK_REFERENCE_WIRE_U16: u8 = 0x18;
pub const CHUNK_REFERENCE_WIRE_PARENT_SEQ_NUM: u8 = 0x68;
pub const CHUNK_REFERENCE_WIRE_OFFSET: u8 = 0xB8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedU16Field {
    pub value: u16,
    pub source: RawSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedU32Field {
    pub value: u32,
    pub source: RawSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contents0x2cChunkReference {
    /// Позиционный seqNum: ordinal слота directory, а не отдельное wire-поле.
    pub seq_num: usize,
    pub source: RawSpan,
    /// Все физически разобранные поля occupied slot сохраняются без фильтрации.
    pub fields: Vec<RawContentsBlock>,
    /// Наблюдения поля 0x02 из подтверждённой пары id0x02/type0x18.
    pub raw_types: Vec<ObservedU16Field>,
    /// Наблюдения поля 0x04 из подтверждённой пары id0x04/type0xB8.
    pub chunk_offsets: Vec<ObservedU32Field>,
    /// Наблюдения поля 0x05 из подтверждённой пары id0x05/type0x68.
    /// Поле не считается обязательным.
    pub parent_seq_nums: Vec<ObservedU32Field>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkReferenceReadError {
    Contents(ContentsReadError),
    Block(BlockReadError),
    SpanTooLarge { source: RawSpan },
    SlotOutOfRange { seq_num: usize, slot_count: usize },
    InconsistentOccupiedSlot { seq_num: usize },
    UnsupportedReferenceWireType { block_type: u8, offset: usize },
}

impl fmt::Display for ChunkReferenceReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contents(error) => error.fmt(f),
            Self::Block(error) => error.fmt(f),
            Self::SpanTooLarge { source } => write!(
                f,
                "диапазон occupied slot Contents не помещается в адресное пространство: offset={}, len={}",
                source.offset, source.len
            ),
            Self::SlotOutOfRange {
                seq_num,
                slot_count,
            } => write!(
                f,
                "seqNum {seq_num} выходит за границы directory из {slot_count} слотов"
            ),
            Self::InconsistentOccupiedSlot { seq_num } => write!(
                f,
                "occupied slot seqNum {seq_num} не содержит подтверждённый container body"
            ),
            Self::UnsupportedReferenceWireType { block_type, offset } => write!(
                f,
                "неподтверждённый wire-type 0x{block_type:02X} внутри chunk reference по смещению {offset}"
            ),
        }
    }
}

impl std::error::Error for ChunkReferenceReadError {}

impl From<ContentsReadError> for ChunkReferenceReadError {
    fn from(value: ContentsReadError) -> Self {
        Self::Contents(value)
    }
}

impl From<BlockReadError> for ChunkReferenceReadError {
    fn from(value: BlockReadError) -> Self {
        Self::Block(value)
    }
}

/// Разбирает содержимое occupied directory slot по PUB-C-123 и PUB-C-125.
///
/// Сначала сохраняется физическая запись каждого field. Семантическое значение
/// поднимается только при совпадении и field id, и подтверждённого wire-type.
/// Дубликаты не схлопываются, optional parent не синтезируется.
pub fn parse_confirmed_chunk_reference(
    bytes: &[u8],
    directory: &Contents0x2cDirectory,
    seq_num: usize,
) -> Result<Option<Contents0x2cChunkReference>, ChunkReferenceReadError> {
    let slot = directory
        .slot(seq_num)
        .ok_or(ChunkReferenceReadError::SlotOutOfRange {
            seq_num,
            slot_count: directory.slots.len(),
        })?;

    let block = match slot {
        Contents0x2cDirectorySlot::Empty { .. } => return Ok(None),
        Contents0x2cDirectorySlot::Occupied { block } => block,
    };

    let content_source = match &block.body {
        RawContentsBlockBody::Container { content_source, .. } => content_source,
        _ => {
            return Err(ChunkReferenceReadError::InconsistentOccupiedSlot { seq_num });
        }
    };

    let start = usize::try_from(content_source.offset).map_err(|_| {
        ChunkReferenceReadError::SpanTooLarge {
            source: content_source.clone(),
        }
    })?;
    let len =
        usize::try_from(content_source.len).map_err(|_| ChunkReferenceReadError::SpanTooLarge {
            source: content_source.clone(),
        })?;

    let mut cursor = ContentsCursor::bounded(content_source.stream.clone(), bytes, start, len)?;
    let mut fields = Vec::new();
    let mut raw_types = Vec::new();
    let mut chunk_offsets = Vec::new();
    let mut parent_seq_nums = Vec::new();

    while cursor.remaining() > 0 {
        let field = parse_confirmed_reference_field(&mut cursor)?;

        match (&field.body, field.id, field.block_type) {
            (
                RawContentsBlockBody::U16 {
                    value,
                    value_source,
                },
                CHUNK_REFERENCE_RAW_TYPE_ID,
                CHUNK_REFERENCE_WIRE_U16,
            ) => raw_types.push(ObservedU16Field {
                value: *value,
                source: value_source.clone(),
            }),
            (
                RawContentsBlockBody::U32 {
                    value,
                    value_source,
                },
                CHUNK_REFERENCE_OFFSET_ID,
                CHUNK_REFERENCE_WIRE_OFFSET,
            ) => chunk_offsets.push(ObservedU32Field {
                value: *value,
                source: value_source.clone(),
            }),
            (
                RawContentsBlockBody::U32 {
                    value,
                    value_source,
                },
                CHUNK_REFERENCE_PARENT_SEQ_NUM_ID,
                CHUNK_REFERENCE_WIRE_PARENT_SEQ_NUM,
            ) => parent_seq_nums.push(ObservedU32Field {
                value: *value,
                source: value_source.clone(),
            }),
            _ => {}
        }

        fields.push(field);
    }

    Ok(Some(Contents0x2cChunkReference {
        seq_num,
        source: block.source.clone(),
        fields,
        raw_types,
        chunk_offsets,
        parent_seq_nums,
    }))
}

/// Читает field внутри occupied chunk reference.
///
/// Сначала переиспользуется общий parser для уже глобально подтверждённых
/// wire-types. Если тип не входит в него, применяются только контекстные
/// формы, подтверждённые PUB-C-125 на 234/234 occupied references.
fn parse_confirmed_reference_field(
    cursor: &mut ContentsCursor<'_>,
) -> Result<RawContentsBlock, ChunkReferenceReadError> {
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
        CHUNK_REFERENCE_WIRE_EMPTY => RawContentsBlockBody::Empty,
        CHUNK_REFERENCE_WIRE_U16_SERVICE | CHUNK_REFERENCE_WIRE_U16 => {
            let (value, value_source) = probe.read_u16_le()?;
            RawContentsBlockBody::U16 {
                value,
                value_source,
            }
        }
        CHUNK_REFERENCE_WIRE_PARENT_SEQ_NUM | CHUNK_REFERENCE_WIRE_OFFSET => {
            let (value, value_source) = probe.read_u32_le()?;
            RawContentsBlockBody::U32 {
                value,
                value_source,
            }
        }
        _ => {
            return Err(ChunkReferenceReadError::UnsupportedReferenceWireType {
                block_type,
                offset: start,
            });
        }
    };

    let end = probe.position();
    let source = RawSpan {
        stream: id_source.stream,
        offset: id_source.offset,
        len: (end - start) as u64,
    };

    let block = RawContentsBlock {
        id,
        block_type,
        source,
        body,
    };

    *cursor = probe;
    Ok(block)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_confirmed_0x2c_directory;
    use pub_core::StreamPath;

    fn parse_directory(bytes: &[u8]) -> Contents0x2cDirectory {
        parse_confirmed_0x2c_directory(
            bytes,
            RawSpan {
                stream: StreamPath("/Contents".into()),
                offset: 0,
                len: bytes.len() as u64,
            },
        )
        .expect("directory должен читаться")
    }

    #[test]
    fn maps_real_confirmed_wire_types() {
        let bytes = [
            0x00, 0x88, 0x14, 0x00, 0x00, 0x00, // occupied, 16 bytes content
            0x02, 0x18, 0x44, 0x00, // raw type = 0x44
            0x04, 0xB8, 0x34, 0x12, 0x00, 0x00, // chunk offset = 0x1234
            0x05, 0x68, 0x00, 0x01, 0x00, 0x00, // parent seqNum = 256
        ];
        let directory = parse_directory(&bytes);

        let reference = parse_confirmed_chunk_reference(&bytes, &directory, 0)
            .expect("reference должен читаться")
            .expect("slot 0 должен быть occupied");

        assert_eq!(reference.seq_num, 0);
        assert_eq!(reference.fields.len(), 3);
        assert_eq!(reference.raw_types[0].value, 0x44);
        assert_eq!(reference.chunk_offsets[0].value, 0x1234);
        assert_eq!(reference.parent_seq_nums[0].value, 256);
        assert_eq!(reference.raw_types[0].source.offset, 8);
        assert_eq!(reference.raw_types[0].source.len, 2);
    }

    #[test]
    fn parses_observed_root_reference_without_parent() {
        let bytes = [
            0x00, 0x88, 0x1C, 0x00, 0x00, 0x00, // occupied, 24 bytes content
            0x02, 0x18, 0x44, 0x00, // raw type
            0x04, 0xB8, 0x0A, 0x0A, 0x00, 0x00, // chunk offset
            0x06, 0x10, 0x02, 0x0A, // service field
            0x08, 0x08, // presence field
            0x09, 0x08, // presence field
            0x0A, 0x08, // presence field
            0x0B, 0x18, 0x01, 0x00, // optional u16 field
        ];
        let directory = parse_directory(&bytes);

        let reference = parse_confirmed_chunk_reference(&bytes, &directory, 0)
            .expect("reference должен читаться")
            .expect("slot 0 должен быть occupied");

        assert_eq!(reference.fields.len(), 7);
        assert_eq!(reference.raw_types[0].value, 0x44);
        assert_eq!(reference.chunk_offsets[0].value, 0x0A0A);
        assert!(reference.parent_seq_nums.is_empty());
    }

    #[test]
    fn parent_field_is_optional() {
        let bytes = [
            0x00, 0x88, 0x0E, 0x00, 0x00, 0x00, // occupied, 10 bytes content
            0x02, 0x18, 0x44, 0x00, 0x04, 0xB8, 0x20, 0x00, 0x00, 0x00,
        ];
        let directory = parse_directory(&bytes);

        let reference = parse_confirmed_chunk_reference(&bytes, &directory, 0)
            .expect("reference должен читаться")
            .expect("slot 0 должен быть occupied");

        assert!(reference.parent_seq_nums.is_empty());
    }

    #[test]
    fn duplicate_semantic_fields_are_preserved_as_multiple_observations() {
        let bytes = [
            0x00, 0x88, 0x0C, 0x00, 0x00, 0x00, // occupied, 8 bytes content
            0x02, 0x18, 0x44, 0x00, 0x02, 0x18, 0x43, 0x00,
        ];
        let directory = parse_directory(&bytes);

        let reference = parse_confirmed_chunk_reference(&bytes, &directory, 0)
            .expect("reference должен читаться")
            .expect("slot 0 должен быть occupied");

        assert_eq!(
            reference
                .raw_types
                .iter()
                .map(|field| field.value)
                .collect::<Vec<_>>(),
            vec![0x44, 0x43]
        );
    }

    #[test]
    fn same_id_with_other_confirmed_wire_type_is_not_semantically_promoted() {
        let bytes = [
            0x00, 0x88, 0x08, 0x00, 0x00, 0x00, // occupied, 4 bytes content
            0x02, 0x10, 0x44, 0x00,
        ];
        let directory = parse_directory(&bytes);

        let reference = parse_confirmed_chunk_reference(&bytes, &directory, 0)
            .expect("reference должен читаться")
            .expect("slot 0 должен быть occupied");

        assert_eq!(reference.fields.len(), 1);
        assert!(reference.raw_types.is_empty());
    }

    #[test]
    fn unsupported_reference_wire_type_is_local_error() {
        let bytes = [
            0x00, 0x88, 0x08, 0x00, 0x00, 0x00, // occupied, 4 bytes content
            0x02, 0x19, 0x44, 0x00,
        ];
        let directory = parse_directory(&bytes);

        assert_eq!(
            parse_confirmed_chunk_reference(&bytes, &directory, 0)
                .expect_err("неподтверждённый wire-type должен быть ошибкой"),
            ChunkReferenceReadError::UnsupportedReferenceWireType {
                block_type: 0x19,
                offset: 6,
            }
        );
    }

    #[test]
    fn empty_slot_has_no_chunk_reference() {
        let bytes = [0x00, 0x78];
        let directory = parse_directory(&bytes);

        assert_eq!(
            parse_confirmed_chunk_reference(&bytes, &directory, 0)
                .expect("empty slot должен корректно обрабатываться"),
            None
        );
    }

    #[test]
    fn out_of_range_seq_num_is_explicit_error() {
        let bytes = [0x00, 0x78];
        let directory = parse_directory(&bytes);

        assert_eq!(
            parse_confirmed_chunk_reference(&bytes, &directory, 1)
                .expect_err("несуществующий seqNum должен быть ошибкой"),
            ChunkReferenceReadError::SlotOutOfRange {
                seq_num: 1,
                slot_count: 1,
            }
        );
    }
}
