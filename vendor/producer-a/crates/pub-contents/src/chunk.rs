use crate::{
    BlockReadError, ContentsCursor, ContentsReadError, RawContentsBlock, parse_confirmed_block,
};
use pub_core::{RawSpan, StreamPath};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contents0x2cChunk {
    pub source: RawSpan,
    pub declared_length: u32,
    pub declared_length_source: RawSpan,
    pub fields: Vec<RawContentsBlock>,
    /// Exact undecoded suffix beginning at the first unsupported field.
    ///
    /// This is intentionally not skipped. A higher layer can attach the range
    /// to SourceCapsule/OpaqueExtension or report an explicit capability loss.
    pub unsupported_tail: Option<RawSpan>,
}

impl Contents0x2cChunk {
    pub const fn is_fully_decoded(&self) -> bool {
        self.unsupported_tail.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkReadError {
    Contents(ContentsReadError),
    Block(BlockReadError),
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
}

impl fmt::Display for ChunkReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contents(error) => error.fmt(f),
            Self::Block(error) => error.fmt(f),
            Self::OffsetTooLarge { offset } => {
                write!(
                    f,
                    "смещение 0x2C chunk не помещается в адресное пространство: {offset}"
                )
            }
            Self::DeclaredLengthTooSmall {
                offset,
                declared_length,
            } => write!(
                f,
                "declared length 0x2C chunk по смещению {offset} меньше 4: {declared_length}"
            ),
            Self::DeclaredRangeOutOfBounds {
                offset,
                declared_length,
                stream_len,
            } => write!(
                f,
                "declared 0x2C chunk выходит за границы Contents: offset={offset}, len={declared_length}, stream_len={stream_len}"
            ),
        }
    }
}

impl std::error::Error for ChunkReadError {}

impl From<ContentsReadError> for ChunkReadError {
    fn from(value: ContentsReadError) -> Self {
        Self::Contents(value)
    }
}

impl From<BlockReadError> for ChunkReadError {
    fn from(value: BlockReadError) -> Self {
        Self::Block(value)
    }
}

/// Parses the confirmed mature-0x2C chunk envelope.
///
/// Corpus evidence establishes that the first little-endian u32 is the full
/// chunk length including the length word. Known field wire forms are decoded
/// by the shared block reader. On the first unsupported wire form, the parser
/// preserves the exact remaining suffix instead of guessing a length or
/// silently discarding bytes.
pub fn parse_confirmed_0x2c_chunk(
    stream: StreamPath,
    bytes: &[u8],
    offset: u32,
) -> Result<Contents0x2cChunk, ChunkReadError> {
    let start = usize::try_from(offset).map_err(|_| ChunkReadError::OffsetTooLarge { offset })?;
    if start >= bytes.len() {
        return Err(ChunkReadError::DeclaredRangeOutOfBounds {
            offset,
            declared_length: 0,
            stream_len: bytes.len(),
        });
    }

    let available = bytes.len() - start;
    let mut leading = ContentsCursor::bounded(stream.clone(), bytes, start, available)?;
    let (declared_length, declared_length_source) = leading.read_u32_le()?;

    if declared_length < 4 {
        return Err(ChunkReadError::DeclaredLengthTooSmall {
            offset,
            declared_length,
        });
    }

    let declared_usize =
        usize::try_from(declared_length).map_err(|_| ChunkReadError::DeclaredRangeOutOfBounds {
            offset,
            declared_length,
            stream_len: bytes.len(),
        })?;
    let end = start
        .checked_add(declared_usize)
        .filter(|end| *end <= bytes.len())
        .ok_or(ChunkReadError::DeclaredRangeOutOfBounds {
            offset,
            declared_length,
            stream_len: bytes.len(),
        })?;

    let fields_start = start + 4;
    let mut cursor =
        ContentsCursor::bounded(stream.clone(), bytes, fields_start, end - fields_start)?;
    let mut fields = Vec::new();
    let mut unsupported_tail = None;

    while cursor.remaining() > 0 {
        match parse_confirmed_block(&mut cursor) {
            Ok(field) => fields.push(field),
            Err(BlockReadError::UnsupportedType { .. }) => {
                unsupported_tail = Some(RawSpan {
                    stream: stream.clone(),
                    offset: cursor.position() as u64,
                    len: cursor.remaining() as u64,
                });
                break;
            }
            Err(error) => return Err(error.into()),
        }
    }

    Ok(Contents0x2cChunk {
        source: RawSpan {
            stream,
            offset: start as u64,
            len: declared_usize as u64,
        },
        declared_length,
        declared_length_source,
        fields,
        unsupported_tail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BLOCK_TYPE_REFERENCE_U32, RawContentsBlockBody};

    #[test]
    fn parses_declared_chunk_and_known_fields() {
        let bytes = [
            0x10, 0x00, 0x00, 0x00, // full chunk length = 16
            0x27, 0x20, 0x16, 0x00, 0x00, 0x00, // textId = 22
            0x37, 0x68, 0x49, 0x01, 0x00, 0x00, // next frame = 329
        ];
        let chunk = parse_confirmed_0x2c_chunk(StreamPath("/Contents".into()), &bytes, 0)
            .expect("known shape subset must parse");

        assert!(chunk.is_fully_decoded());
        assert_eq!(chunk.declared_length, 16);
        assert_eq!(chunk.fields.len(), 2);
        assert_eq!(chunk.fields[1].block_type, BLOCK_TYPE_REFERENCE_U32);
        assert!(matches!(
            chunk.fields[1].body,
            RawContentsBlockBody::U32 { value: 329, .. }
        ));
    }

    #[test]
    fn service_u16_before_story_id_keeps_shape_tail_decodable() {
        let bytes = [
            0x0E, 0x00, 0x00, 0x00, // full chunk length = 14
            0x04, 0x10, 0x03, 0x2D, // mature service-u16 field
            0x27, 0x20, 0x28, 0x00, 0x00, 0x00, // textId = 40
        ];
        let chunk = parse_confirmed_0x2c_chunk(StreamPath("/Contents".into()), &bytes, 0)
            .expect("service-u16 must not hide following story id");

        assert!(chunk.is_fully_decoded());
        assert_eq!(chunk.fields.len(), 2);
        assert_eq!(chunk.fields[1].id, 0x27);
        assert!(matches!(
            chunk.fields[1].body,
            RawContentsBlockBody::U32 { value: 40, .. }
        ));
    }

    #[test]
    fn unsupported_field_preserves_exact_suffix() {
        let bytes = [
            0x0C, 0x00, 0x00, 0x00, // full chunk length = 12
            0x27, 0x20, 0x16, 0x00, 0x00, 0x00, // known
            0x01, 0xC0, // unsupported wire form
        ];
        let stream = StreamPath("/Contents".into());
        let chunk = parse_confirmed_0x2c_chunk(stream.clone(), &bytes, 0)
            .expect("unsupported suffix must remain opaque");

        assert_eq!(chunk.fields.len(), 1);
        assert_eq!(
            chunk.unsupported_tail,
            Some(RawSpan {
                stream,
                offset: 10,
                len: 2,
            })
        );
    }

    #[test]
    fn declared_range_must_fit_stream() {
        let bytes = [0x20, 0x00, 0x00, 0x00, 0, 0, 0, 0];
        assert_eq!(
            parse_confirmed_0x2c_chunk(StreamPath("/Contents".into()), &bytes, 0),
            Err(ChunkReadError::DeclaredRangeOutOfBounds {
                offset: 0,
                declared_length: 32,
                stream_len: 8,
            })
        );
    }
}
