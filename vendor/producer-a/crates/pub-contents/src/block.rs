use crate::{ContentsCursor, ContentsReadError};
use pub_core::RawSpan;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const BLOCK_TYPE_EMPTY: u8 = 0x08;
/// Service-flavored 2-byte scalar wire observed in mature Publisher chunks.
///
/// The physical width is independently corroborated by the Story Catalog
/// reader, the bounded TABLE tail scanner, and wild SHAPE corpus evidence.
pub const BLOCK_TYPE_U16_SERVICE: u8 = 0x10;
pub const BLOCK_TYPE_U16: u8 = 0x18;
pub const BLOCK_TYPE_U32: u8 = 0x20;
pub const BLOCK_TYPE_FIXED_8: u8 = 0x28;
pub const BLOCK_TYPE_FIXED_16: u8 = 0x38;
pub const BLOCK_TYPE_REFERENCE_U32: u8 = 0x68;
pub const BLOCK_TYPE_HANDLE_U32: u8 = 0x70;
pub const BLOCK_TYPE_DUMMY: u8 = 0x78;
pub const BLOCK_TYPE_CONTAINER_88: u8 = 0x88;
pub const BLOCK_TYPE_CONTAINER_90: u8 = 0x90;
pub const BLOCK_TYPE_CONTAINER_A0: u8 = 0xA0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawContentsBlock {
    pub id: u8,
    pub block_type: u8,
    pub source: RawSpan,
    pub body: RawContentsBlockBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RawContentsBlockBody {
    Empty,
    U16 {
        value: u16,
        value_source: RawSpan,
    },
    U32 {
        value: u32,
        value_source: RawSpan,
    },
    Fixed8 {
        bytes: [u8; 8],
        value_source: RawSpan,
    },
    Fixed16 {
        bytes: [u8; 16],
        value_source: RawSpan,
    },
    Container {
        declared_length: u32,
        length_source: RawSpan,
        content_source: RawSpan,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockReadError {
    Contents(ContentsReadError),
    UnsupportedType {
        block_type: u8,
        offset: usize,
    },
    InvalidDeclaredLength {
        block_type: u8,
        offset: usize,
        declared_length: u32,
    },
    LengthTooLarge {
        block_type: u8,
        declared_length: u32,
    },
}

impl fmt::Display for BlockReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contents(error) => error.fmt(f),
            Self::UnsupportedType { block_type, offset } => write!(
                f,
                "неподтверждённый физический тип блока Contents 0x{block_type:02X} по смещению {offset}"
            ),
            Self::InvalidDeclaredLength {
                block_type,
                offset,
                declared_length,
            } => write!(
                f,
                "некорректная длина блока Contents типа 0x{block_type:02X} по смещению {offset}: {declared_length}"
            ),
            Self::LengthTooLarge {
                block_type,
                declared_length,
            } => write!(
                f,
                "длина блока Contents типа 0x{block_type:02X} не помещается в адресное пространство: {declared_length}"
            ),
        }
    }
}

impl std::error::Error for BlockReadError {}

impl From<ContentsReadError> for BlockReadError {
    fn from(value: ContentsReadError) -> Self {
        Self::Contents(value)
    }
}

/// Читает только физически подтверждённые типы блоков текущего foundation.
///
/// Ошибка не сдвигает исходный курсор. Это позволяет локализовать malformed
/// или пока неподдерживаемый блок, не теряя позицию начала.
pub fn parse_confirmed_block(
    cursor: &mut ContentsCursor<'_>,
) -> Result<RawContentsBlock, BlockReadError> {
    let mut probe = cursor.clone();
    let block = parse_confirmed_block_inner(&mut probe)?;
    *cursor = probe;
    Ok(block)
}

fn parse_confirmed_block_inner(
    cursor: &mut ContentsCursor<'_>,
) -> Result<RawContentsBlock, BlockReadError> {
    let start = cursor.position();
    let (id, id_source) = cursor.read_u8()?;
    let (block_type, _) = cursor.read_u8()?;

    let body = match block_type {
        BLOCK_TYPE_EMPTY => RawContentsBlockBody::Empty,
        BLOCK_TYPE_U16_SERVICE | BLOCK_TYPE_U16 => {
            let (value, value_source) = cursor.read_u16_le()?;
            RawContentsBlockBody::U16 {
                value,
                value_source,
            }
        }
        BLOCK_TYPE_U32 | BLOCK_TYPE_REFERENCE_U32 | BLOCK_TYPE_HANDLE_U32 => {
            let (value, value_source) = cursor.read_u32_le()?;
            RawContentsBlockBody::U32 {
                value,
                value_source,
            }
        }
        BLOCK_TYPE_FIXED_8 => {
            let (raw, value_source) = cursor.take(8)?;
            let mut bytes = [0_u8; 8];
            bytes.copy_from_slice(raw);
            RawContentsBlockBody::Fixed8 {
                bytes,
                value_source,
            }
        }
        BLOCK_TYPE_FIXED_16 => {
            let (raw, value_source) = cursor.take(16)?;
            let mut bytes = [0_u8; 16];
            bytes.copy_from_slice(raw);
            RawContentsBlockBody::Fixed16 {
                bytes,
                value_source,
            }
        }
        BLOCK_TYPE_DUMMY => RawContentsBlockBody::Empty,
        BLOCK_TYPE_CONTAINER_88 | BLOCK_TYPE_CONTAINER_90 | BLOCK_TYPE_CONTAINER_A0 => {
            let (declared_length, length_source) = cursor.read_u32_le()?;
            if declared_length < 4 {
                return Err(BlockReadError::InvalidDeclaredLength {
                    block_type,
                    offset: start,
                    declared_length,
                });
            }

            let content_len_u32 = declared_length - 4;
            let content_len =
                usize::try_from(content_len_u32).map_err(|_| BlockReadError::LengthTooLarge {
                    block_type,
                    declared_length,
                })?;
            let (_, content_source) = cursor.take(content_len)?;

            RawContentsBlockBody::Container {
                declared_length,
                length_source,
                content_source,
            }
        }
        _ => {
            return Err(BlockReadError::UnsupportedType {
                block_type,
                offset: start,
            });
        }
    };

    let end = cursor.position();
    let source = RawSpan {
        stream: id_source.stream,
        offset: id_source.offset,
        len: (end - start) as u64,
    };

    Ok(RawContentsBlock {
        id,
        block_type,
        source,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pub_core::StreamPath;

    #[test]
    fn parses_service_u16_block() {
        let mut cursor = ContentsCursor::new(
            StreamPath("/Contents".into()),
            &[0x04, BLOCK_TYPE_U16_SERVICE, 0x03, 0x2D],
        );

        let block = parse_confirmed_block(&mut cursor).expect("service u16 block must parse");

        assert_eq!(block.id, 0x04);
        assert_eq!(block.block_type, BLOCK_TYPE_U16_SERVICE);
        assert_eq!(
            block.body,
            RawContentsBlockBody::U16 {
                value: 0x2D03,
                value_source: RawSpan {
                    stream: StreamPath("/Contents".into()),
                    offset: 2,
                    len: 2,
                },
            }
        );
        assert_eq!(cursor.position(), 4);
    }

    #[test]
    fn parses_u32_block() {
        let mut cursor = ContentsCursor::new(
            StreamPath("/Contents".into()),
            &[0x01, 0x20, 0x1B, 0x01, 0x00, 0x00],
        );

        let block = parse_confirmed_block(&mut cursor).expect("u32-блок должен читаться");

        assert_eq!(block.id, 0x01);
        assert_eq!(block.block_type, BLOCK_TYPE_U32);
        assert_eq!(block.source.offset, 0);
        assert_eq!(block.source.len, 6);
        assert_eq!(
            block.body,
            RawContentsBlockBody::U32 {
                value: 283,
                value_source: RawSpan {
                    stream: StreamPath("/Contents".into()),
                    offset: 2,
                    len: 4,
                },
            }
        );
        assert_eq!(cursor.position(), 6);
    }

    #[test]
    fn parses_handle_u32_block() {
        let mut cursor = ContentsCursor::new(
            StreamPath("/Contents".into()),
            &[0x00, 0x70, 0x27, 0x01, 0x00, 0x00],
        );

        let block = parse_confirmed_block(&mut cursor).expect("handle-u32 блок должен читаться");

        assert_eq!(block.id, 0x00);
        assert_eq!(block.block_type, BLOCK_TYPE_HANDLE_U32);
        assert_eq!(
            block.body,
            RawContentsBlockBody::U32 {
                value: 295,
                value_source: RawSpan {
                    stream: StreamPath("/Contents".into()),
                    offset: 2,
                    len: 4,
                },
            }
        );
        assert_eq!(cursor.position(), 6);
    }

    #[test]
    fn parses_confirmed_fixed_8_identity_payload() {
        let mut cursor = ContentsCursor::new(
            StreamPath("/Contents".into()),
            &[0x0D, 0x28, 0x02, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00],
        );

        let block =
            parse_confirmed_block(&mut cursor).expect("8-byte identity block должен читаться");

        assert_eq!(block.id, 0x0D);
        assert_eq!(block.block_type, BLOCK_TYPE_FIXED_8);
        assert_eq!(block.source.len, 10);
        assert_eq!(
            block.body,
            RawContentsBlockBody::Fixed8 {
                bytes: [0x02, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00],
                value_source: RawSpan {
                    stream: StreamPath("/Contents".into()),
                    offset: 2,
                    len: 8,
                },
            }
        );
        assert_eq!(cursor.position(), 10);
    }

    #[test]
    fn parses_dummy_as_two_byte_block() {
        let mut cursor = ContentsCursor::new(StreamPath("/Contents".into()), &[0x00, 0x78]);

        let block = parse_confirmed_block(&mut cursor).expect("DUMMY должен читаться");

        assert_eq!(block.body, RawContentsBlockBody::Empty);
        assert_eq!(block.source.len, 2);
        assert_eq!(cursor.position(), 2);
    }

    #[test]
    fn parses_container_length_as_length_including_its_own_dword() {
        let bytes = [0x03, 0x90, 0x08, 0x00, 0x00, 0x00, 0xAA, 0xBB, 0xCC, 0xDD];
        let mut cursor = ContentsCursor::new(StreamPath("/Contents".into()), &bytes);

        let block = parse_confirmed_block(&mut cursor).expect("контейнер должен читаться");

        assert_eq!(block.source.len, 10);
        assert_eq!(
            block.body,
            RawContentsBlockBody::Container {
                declared_length: 8,
                length_source: RawSpan {
                    stream: StreamPath("/Contents".into()),
                    offset: 2,
                    len: 4,
                },
                content_source: RawSpan {
                    stream: StreamPath("/Contents".into()),
                    offset: 6,
                    len: 4,
                },
            }
        );
        assert_eq!(cursor.position(), 10);
    }

    #[test]
    fn parses_a0_container_with_generic_length_rule() {
        let bytes = [
            0x02, 0xA0, 0x0A, 0x00, 0x00, 0x00, 0x00, 0x70, 0x07, 0x01, 0x00, 0x00,
        ];
        let mut cursor = ContentsCursor::new(StreamPath("/Contents".into()), &bytes);

        let block = parse_confirmed_block(&mut cursor)
            .expect("A0-контейнер должен использовать подтверждённое variable-length framing");

        assert_eq!(block.id, 0x02);
        assert_eq!(block.block_type, BLOCK_TYPE_CONTAINER_A0);
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
    fn malformed_container_does_not_advance_cursor() {
        let bytes = [0x00, 0x88, 0x03, 0x00, 0x00, 0x00];
        let mut cursor = ContentsCursor::new(StreamPath("/Contents".into()), &bytes);

        let error = parse_confirmed_block(&mut cursor)
            .expect_err("declared length меньше 4 должен быть ошибкой");

        assert_eq!(
            error,
            BlockReadError::InvalidDeclaredLength {
                block_type: 0x88,
                offset: 0,
                declared_length: 3,
            }
        );
        assert_eq!(cursor.position(), 0);
    }

    #[test]
    fn unsupported_type_does_not_advance_cursor() {
        let mut cursor =
            ContentsCursor::new(StreamPath("/Contents".into()), &[0x01, 0x21, 0xAA, 0xBB]);

        let error = parse_confirmed_block(&mut cursor)
            .expect_err("неподтверждённый тип должен отклоняться");

        assert_eq!(
            error,
            BlockReadError::UnsupportedType {
                block_type: 0x21,
                offset: 0,
            }
        );
        assert_eq!(cursor.position(), 0);
    }

    #[test]
    fn bounded_cursor_prevents_container_from_crossing_parent_boundary() {
        let bytes = [
            0xFF, 0xFF, 0x00, 0x88, 0x08, 0x00, 0x00, 0x00, 0xAA, 0xBB, 0xCC, 0xDD,
        ];
        let mut cursor = ContentsCursor::bounded(StreamPath("/Contents".into()), &bytes, 2, 8)
            .expect("bounded cursor должен создаваться");

        let error = parse_confirmed_block(&mut cursor)
            .expect_err("контейнер не должен выходить за границы parent range");

        assert_eq!(
            error,
            BlockReadError::Contents(ContentsReadError::TooShort {
                offset: 8,
                requested: 4,
                available: 2,
            })
        );
        assert_eq!(cursor.position(), 2);
    }
}
