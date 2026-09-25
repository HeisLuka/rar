use crate::{
    BLOCK_TYPE_CONTAINER_88, BLOCK_TYPE_U32, BlockReadError, Contents0x2cChunk, ContentsCursor,
    ContentsReadError, RawContentsBlock, RawContentsBlockBody, parse_confirmed_block,
};
use pub_core::RawSpan;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const MARGINS_PAGE_EXTENT_ID: u16 = 0x03;
pub const MARGINS_PAGE_WIDTH_ID: u16 = 0x01;
pub const MARGINS_PAGE_HEIGHT_ID: u16 = 0x02;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarginsPageExtent {
    pub width_emu: u32,
    pub width_source: RawSpan,
    pub height_emu: u32,
    pub height_source: RawSpan,
    pub block: RawContentsBlock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarginsPageExtentReadError {
    Contents(ContentsReadError),
    Block(BlockReadError),
    MissingExtentBlock,
    DuplicateExtentBlock,
    UnexpectedExtentType { offset: u64, block_type: u8 },
    InconsistentExtentBody { offset: u64 },
    SpanTooLarge { source: RawSpan },
    MissingDimension { id: u16 },
    DuplicateDimension { id: u16 },
    UnexpectedDimensionType { id: u16, offset: u64, block_type: u8 },
    InconsistentDimensionBody { id: u16, offset: u64 },
}

impl fmt::Display for MarginsPageExtentReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contents(error) => error.fmt(f),
            Self::Block(error) => error.fmt(f),
            Self::MissingExtentBlock => {
                write!(f, "Margins/OplMg не содержит field0x03 page extent")
            }
            Self::DuplicateExtentBlock => {
                write!(f, "Margins/OplMg содержит несколько field0x03 page extent")
            }
            Self::UnexpectedExtentType { offset, block_type } => write!(
                f,
                "Margins/OplMg field0x03 по смещению {offset} имеет неподтверждённый wire type 0x{block_type:02X}"
            ),
            Self::InconsistentExtentBody { offset } => write!(
                f,
                "Margins/OplMg field0x03 по смещению {offset} не содержит container body"
            ),
            Self::SpanTooLarge { source } => write!(
                f,
                "Margins/OplMg nested span не помещается в адресное пространство: offset={}, len={}",
                source.offset, source.len
            ),
            Self::MissingDimension { id } => {
                write!(f, "Margins/OplMg page extent не содержит field0x{id:02X}")
            }
            Self::DuplicateDimension { id } => {
                write!(
                    f,
                    "Margins/OplMg page extent содержит несколько field0x{id:02X}"
                )
            }
            Self::UnexpectedDimensionType {
                id,
                offset,
                block_type,
            } => write!(
                f,
                "Margins/OplMg page extent field0x{id:02X} по смещению {offset} имеет wire type 0x{block_type:02X}, ожидался 0x20"
            ),
            Self::InconsistentDimensionBody { id, offset } => write!(
                f,
                "Margins/OplMg page extent field0x{id:02X} по смещению {offset} не содержит u32 body"
            ),
        }
    }
}

impl std::error::Error for MarginsPageExtentReadError {}

impl From<ContentsReadError> for MarginsPageExtentReadError {
    fn from(value: ContentsReadError) -> Self {
        Self::Contents(value)
    }
}

impl From<BlockReadError> for MarginsPageExtentReadError {
    fn from(value: BlockReadError) -> Self {
        Self::Block(value)
    }
}

/// Reads only the confirmed pageWidth/pageHeight carrier inside raw0x4C
/// Margins/OplMg.
///
/// The outer raw type is resolved by the caller from the directory. This
/// function deliberately does not assign semantics to the other layout-guide
/// fields.
pub fn parse_confirmed_margins_page_extent(
    bytes: &[u8],
    chunk: &Contents0x2cChunk,
) -> Result<MarginsPageExtent, MarginsPageExtentReadError> {
    let mut extent_blocks = chunk
        .fields
        .iter()
        .filter(|block| block.id == MARGINS_PAGE_EXTENT_ID);

    let block = extent_blocks
        .next()
        .ok_or(MarginsPageExtentReadError::MissingExtentBlock)?;
    if extent_blocks.next().is_some() {
        return Err(MarginsPageExtentReadError::DuplicateExtentBlock);
    }
    if block.block_type != BLOCK_TYPE_CONTAINER_88 {
        return Err(MarginsPageExtentReadError::UnexpectedExtentType {
            offset: block.source.offset,
            block_type: block.block_type,
        });
    }

    let content_source = match &block.body {
        RawContentsBlockBody::Container { content_source, .. } => content_source.clone(),
        _ => {
            return Err(MarginsPageExtentReadError::InconsistentExtentBody {
                offset: block.source.offset,
            });
        }
    };

    let start = usize::try_from(content_source.offset).map_err(|_| {
        MarginsPageExtentReadError::SpanTooLarge {
            source: content_source.clone(),
        }
    })?;
    let len = usize::try_from(content_source.len).map_err(|_| {
        MarginsPageExtentReadError::SpanTooLarge {
            source: content_source.clone(),
        }
    })?;
    let mut cursor = ContentsCursor::bounded(content_source.stream.clone(), bytes, start, len)?;

    let mut width = None;
    let mut height = None;

    while cursor.remaining() > 0 {
        let nested = parse_confirmed_block(&mut cursor)?;
        match nested.id {
            MARGINS_PAGE_WIDTH_ID => {
                if width.is_some() {
                    return Err(MarginsPageExtentReadError::DuplicateDimension {
                        id: MARGINS_PAGE_WIDTH_ID,
                    });
                }
                width = Some(read_dimension(&nested, MARGINS_PAGE_WIDTH_ID)?);
            }
            MARGINS_PAGE_HEIGHT_ID => {
                if height.is_some() {
                    return Err(MarginsPageExtentReadError::DuplicateDimension {
                        id: MARGINS_PAGE_HEIGHT_ID,
                    });
                }
                height = Some(read_dimension(&nested, MARGINS_PAGE_HEIGHT_ID)?);
            }
            _ => {}
        }
    }

    let (width_emu, width_source) = width.ok_or(MarginsPageExtentReadError::MissingDimension {
        id: MARGINS_PAGE_WIDTH_ID,
    })?;
    let (height_emu, height_source) =
        height.ok_or(MarginsPageExtentReadError::MissingDimension {
            id: MARGINS_PAGE_HEIGHT_ID,
        })?;

    Ok(MarginsPageExtent {
        width_emu,
        width_source,
        height_emu,
        height_source,
        block: block.clone(),
    })
}

fn read_dimension(
    block: &RawContentsBlock,
    id: u16,
) -> Result<(u32, RawSpan), MarginsPageExtentReadError> {
    if block.block_type != BLOCK_TYPE_U32 {
        return Err(MarginsPageExtentReadError::UnexpectedDimensionType {
            id,
            offset: block.source.offset,
            block_type: block.block_type,
        });
    }

    match &block.body {
        RawContentsBlockBody::U32 {
            value,
            value_source,
        } => Ok((*value, value_source.clone())),
        _ => Err(MarginsPageExtentReadError::InconsistentDimensionBody {
            id,
            offset: block.source.offset,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_confirmed_0x2c_chunk;
    use pub_core::StreamPath;

    #[test]
    fn parses_exact_confirmed_extent_container() {
        let bytes = [
            0x16, 0x00, 0x00, 0x00, // chunk length 22
            0x03, 0x88, 0x10, 0x00, 0x00, 0x00, // field03 container, declared=16
            0x01, 0x20, 0x40, 0x5B, 0x73, 0x00, // 7,560,000
            0x02, 0x20, 0xA0, 0x25, 0xA3, 0x00, // 10,692,000
        ];
        let chunk = parse_confirmed_0x2c_chunk(StreamPath("/Contents".into()), &bytes, 0).unwrap();
        let extent = parse_confirmed_margins_page_extent(&bytes, &chunk).unwrap();

        assert_eq!(extent.width_emu, 7_560_000);
        assert_eq!(extent.height_emu, 10_692_000);
        assert_eq!(extent.width_source.offset, 12);
        assert_eq!(extent.height_source.offset, 18);
    }
}
