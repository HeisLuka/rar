use crate::{ContentsFamily, ContentsReadError, detect_family};
use pub_core::{RawSpan, StreamPath};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const LEGACY_0X22_HEADER_POINTER_OFFSET: usize = 0x12;
pub const LEGACY_0X22_DESCRIPTOR_DELTA: usize = 14;
pub const LEGACY_FORMATTING_PAGE_SIZE: usize = 0x200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22FormattingDescriptor {
    pub header_pointer: u32,
    pub header_pointer_source: RawSpan,
    pub descriptor_offset: u32,
    pub text_start: u32,
    pub text_start_source: RawSpan,
    pub text_end: u32,
    pub text_end_source: RawSpan,
    pub character_fkp_page: u16,
    pub character_fkp_page_source: RawSpan,
    pub paragraph_fkp_page: u16,
    pub paragraph_fkp_page_source: RawSpan,
    pub observed_page_3: u16,
    pub observed_page_3_source: RawSpan,
    pub observed_page_3_end: u16,
    pub observed_page_3_end_source: RawSpan,
}

impl Legacy0x22FormattingDescriptor {
    pub fn character_fkp_offset(&self) -> u64 {
        u64::from(self.character_fkp_page) * LEGACY_FORMATTING_PAGE_SIZE as u64
    }

    pub fn paragraph_fkp_offset(&self) -> u64 {
        u64::from(self.paragraph_fkp_page) * LEGACY_FORMATTING_PAGE_SIZE as u64
    }

    /// Character FKP pages in the bounded legacy 0x22 layout.
    ///
    /// Native Publisher 2 corpus files and the historical libmspub reader
    /// independently agree that the paragraph-page index is the exclusive
    /// boundary of the character FKP sequence.
    pub fn character_fkp_page_range(&self) -> std::ops::Range<u16> {
        self.character_fkp_page..self.paragraph_fkp_page
    }

    /// Paragraph FKP pages in the currently verified legacy 0x22 layout.
    ///
    /// Cross-file native Publisher 2 evidence identifies `observed_page_3`
    /// as the exclusive paragraph-FKP boundary in the tested files. The
    /// subsequent range remains observation-labeled and is not interpreted
    /// by the formatting reader.
    pub fn paragraph_fkp_page_range(&self) -> std::ops::Range<u16> {
        self.paragraph_fkp_page..self.observed_page_3
    }

    /// Cell-style FKP pages in the verified no-Quill 0x22 layout.
    ///
    /// The stored field names remain observation-labeled for wire/API
    /// stability, but native Publisher 2 files plus the forgotten
    /// fosnola/libmspub low-family parser identify this exact half-open range
    /// as the cell-style property lane.
    pub fn cell_style_fkp_page_range(&self) -> std::ops::Range<u16> {
        self.observed_page_3..self.observed_page_3_end
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Legacy0x22ReadError {
    Contents(ContentsReadError),
    UnexpectedFamily(ContentsFamily),
    DescriptorOffsetOverflow {
        header_pointer: u32,
    },
    TextRangeInvalid {
        text_start: u32,
        text_end: u32,
        stream_len: usize,
    },
    FormattingPageOrderInvalid {
        character_fkp_page: u16,
        paragraph_fkp_page: u16,
        paragraph_fkp_end: u16,
    },
    CellStylePageOrderInvalid {
        cell_style_fkp_page: u16,
        cell_style_fkp_end: u16,
    },
    FormattingPageOutOfBounds {
        kind: &'static str,
        page_index: u16,
        page_offset: u64,
        stream_len: usize,
    },
}

impl fmt::Display for Legacy0x22ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contents(error) => error.fmt(f),
            Self::UnexpectedFamily(found) => {
                write!(f, "expected legacy Contents family 0x22, found {found:?}")
            }
            Self::DescriptorOffsetOverflow { header_pointer } => write!(
                f,
                "legacy 0x22 descriptor offset overflows: header pointer {header_pointer:#x}"
            ),
            Self::TextRangeInvalid {
                text_start,
                text_end,
                stream_len,
            } => write!(
                f,
                "legacy 0x22 text range {text_start:#x}..{text_end:#x} is invalid for Contents length {stream_len}"
            ),
            Self::FormattingPageOrderInvalid {
                character_fkp_page,
                paragraph_fkp_page,
                paragraph_fkp_end,
            } => write!(
                f,
                "legacy 0x22 formatting page order is invalid: character start {character_fkp_page}, paragraph start {paragraph_fkp_page}, paragraph end {paragraph_fkp_end}"
            ),
            Self::CellStylePageOrderInvalid {
                cell_style_fkp_page,
                cell_style_fkp_end,
            } => write!(
                f,
                "legacy 0x22 cell-style page order is invalid: start {cell_style_fkp_page}, end {cell_style_fkp_end}"
            ),
            Self::FormattingPageOutOfBounds {
                kind,
                page_index,
                page_offset,
                stream_len,
            } => write!(
                f,
                "legacy 0x22 {kind} FKP page {page_index} at {page_offset:#x} does not fit in Contents length {stream_len}"
            ),
        }
    }
}

impl std::error::Error for Legacy0x22ReadError {}

impl From<ContentsReadError> for Legacy0x22ReadError {
    fn from(value: ContentsReadError) -> Self {
        Self::Contents(value)
    }
}

fn span(stream: &StreamPath, offset: usize, len: usize) -> RawSpan {
    RawSpan {
        stream: stream.clone(),
        offset: offset as u64,
        len: len as u64,
    }
}

fn read_u16_at(
    stream: &StreamPath,
    bytes: &[u8],
    offset: usize,
) -> Result<(u16, RawSpan), Legacy0x22ReadError> {
    let end = offset.checked_add(2).ok_or(ContentsReadError::TooShort {
        offset,
        requested: 2,
        available: bytes.len().saturating_sub(offset),
    })?;
    let raw = bytes.get(offset..end).ok_or(ContentsReadError::TooShort {
        offset,
        requested: 2,
        available: bytes.len().saturating_sub(offset),
    })?;
    Ok((
        u16::from_le_bytes([raw[0], raw[1]]),
        span(stream, offset, 2),
    ))
}

fn read_u32_at(
    stream: &StreamPath,
    bytes: &[u8],
    offset: usize,
) -> Result<(u32, RawSpan), Legacy0x22ReadError> {
    let end = offset.checked_add(4).ok_or(ContentsReadError::TooShort {
        offset,
        requested: 4,
        available: bytes.len().saturating_sub(offset),
    })?;
    let raw = bytes.get(offset..end).ok_or(ContentsReadError::TooShort {
        offset,
        requested: 4,
        available: bytes.len().saturating_sub(offset),
    })?;
    Ok((
        u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]),
        span(stream, offset, 4),
    ))
}

/// Parse the observed legacy-0x22 formatting descriptor.
///
/// Oracle-generated Publisher 2 files place a u32 pointer at Contents+0x12.
/// The formatting descriptor begins 14 bytes after that pointer and contains
/// the text byte range followed by 0x200-byte formatting-page indices.
///
/// The four page indices delimit three verified low-family property lanes:
/// character FKP, paragraph FKP, and cell-style FKP. The final two stored
/// field names remain observation-labeled for wire/API stability, but native
/// Publisher 2 corpus evidence and the forgotten fosnola/libmspub parser
/// independently identify index[2]..index[3] as the cell-style range.
pub fn parse_legacy_0x22_formatting_descriptor(
    stream: StreamPath,
    bytes: &[u8],
) -> Result<Legacy0x22FormattingDescriptor, Legacy0x22ReadError> {
    let family = detect_family(bytes)?;
    if family != ContentsFamily::Family0x22 {
        return Err(Legacy0x22ReadError::UnexpectedFamily(family));
    }

    let (header_pointer, header_pointer_source) =
        read_u32_at(&stream, bytes, LEGACY_0X22_HEADER_POINTER_OFFSET)?;
    let descriptor_offset = header_pointer
        .checked_add(LEGACY_0X22_DESCRIPTOR_DELTA as u32)
        .ok_or(Legacy0x22ReadError::DescriptorOffsetOverflow { header_pointer })?;
    let descriptor = usize::try_from(descriptor_offset)
        .map_err(|_| Legacy0x22ReadError::DescriptorOffsetOverflow { header_pointer })?;

    let (text_start, text_start_source) = read_u32_at(&stream, bytes, descriptor)?;
    let (text_end, text_end_source) = read_u32_at(&stream, bytes, descriptor + 4)?;
    let (character_fkp_page, character_fkp_page_source) =
        read_u16_at(&stream, bytes, descriptor + 8)?;
    let (paragraph_fkp_page, paragraph_fkp_page_source) =
        read_u16_at(&stream, bytes, descriptor + 10)?;
    let (observed_page_3, observed_page_3_source) = read_u16_at(&stream, bytes, descriptor + 12)?;
    let (observed_page_3_end, observed_page_3_end_source) =
        read_u16_at(&stream, bytes, descriptor + 14)?;

    if text_start > text_end || u64::from(text_end) > bytes.len() as u64 {
        return Err(Legacy0x22ReadError::TextRangeInvalid {
            text_start,
            text_end,
            stream_len: bytes.len(),
        });
    }

    let empty_zero_page_shape = text_start == text_end
        && character_fkp_page == 0
        && paragraph_fkp_page == 0
        && observed_page_3 == 0
        && observed_page_3_end == 0;

    if !empty_zero_page_shape {
        if character_fkp_page >= paragraph_fkp_page || paragraph_fkp_page >= observed_page_3 {
            return Err(Legacy0x22ReadError::FormattingPageOrderInvalid {
                character_fkp_page,
                paragraph_fkp_page,
                paragraph_fkp_end: observed_page_3,
            });
        }
        if observed_page_3 > observed_page_3_end {
            return Err(Legacy0x22ReadError::CellStylePageOrderInvalid {
                cell_style_fkp_page: observed_page_3,
                cell_style_fkp_end: observed_page_3_end,
            });
        }

        for page_index in character_fkp_page..observed_page_3_end {
            let kind = if page_index < paragraph_fkp_page {
                "character"
            } else if page_index < observed_page_3 {
                "paragraph"
            } else {
                "cell-style"
            };
            let page_offset = u64::from(page_index) * LEGACY_FORMATTING_PAGE_SIZE as u64;
            let page_end = page_offset + LEGACY_FORMATTING_PAGE_SIZE as u64;
            if page_end > bytes.len() as u64 {
                return Err(Legacy0x22ReadError::FormattingPageOutOfBounds {
                    kind,
                    page_index,
                    page_offset,
                    stream_len: bytes.len(),
                });
            }
        }
    }

    Ok(Legacy0x22FormattingDescriptor {
        header_pointer,
        header_pointer_source,
        descriptor_offset,
        text_start,
        text_start_source,
        text_end,
        text_end_source,
        character_fkp_page,
        character_fkp_page_source,
        paragraph_fkp_page,
        paragraph_fkp_page_source,
        observed_page_3,
        observed_page_3_source,
        observed_page_3_end,
        observed_page_3_end_source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CONTENTS_0X22_MAGIC;

    fn oracle_shape(text_end: u32) -> Vec<u8> {
        let mut bytes = vec![0u8; 0x800];
        bytes[0..4].copy_from_slice(&CONTENTS_0X22_MAGIC);
        bytes[0x12..0x16].copy_from_slice(&0x22u32.to_le_bytes());

        let descriptor = 0x22 + 14;
        bytes[descriptor..descriptor + 4].copy_from_slice(&0x50u32.to_le_bytes());
        bytes[descriptor + 4..descriptor + 8].copy_from_slice(&text_end.to_le_bytes());
        bytes[descriptor + 8..descriptor + 10].copy_from_slice(&1u16.to_le_bytes());
        bytes[descriptor + 10..descriptor + 12].copy_from_slice(&2u16.to_le_bytes());
        bytes[descriptor + 12..descriptor + 14].copy_from_slice(&3u16.to_le_bytes());
        bytes[descriptor + 14..descriptor + 16].copy_from_slice(&4u16.to_le_bytes());
        bytes
    }

    #[test]
    fn parses_oracle_observed_descriptor_and_page_offsets() {
        let stream = StreamPath("/Contents".into());
        let bytes = oracle_shape(0x57);

        let parsed = parse_legacy_0x22_formatting_descriptor(stream.clone(), &bytes).unwrap();

        assert_eq!(parsed.header_pointer, 0x22);
        assert_eq!(parsed.descriptor_offset, 0x30);
        assert_eq!((parsed.text_start, parsed.text_end), (0x50, 0x57));
        assert_eq!(parsed.character_fkp_page, 1);
        assert_eq!(parsed.paragraph_fkp_page, 2);
        assert_eq!(parsed.character_fkp_offset(), 0x200);
        assert_eq!(parsed.paragraph_fkp_offset(), 0x400);
        assert_eq!(parsed.character_fkp_page_range(), 1..2);
        assert_eq!(parsed.paragraph_fkp_page_range(), 2..3);
        assert_eq!(parsed.cell_style_fkp_page_range(), 3..4);
        assert_eq!(
            parsed.character_fkp_page_source,
            RawSpan {
                stream,
                offset: 0x38,
                len: 2,
            }
        );
    }

    #[test]
    fn rejects_non_legacy_family() {
        let mut bytes = oracle_shape(0x57);
        bytes[0..4].copy_from_slice(&crate::CONTENTS_0X2C_MAGIC);

        assert!(matches!(
            parse_legacy_0x22_formatting_descriptor(StreamPath("/Contents".into()), &bytes),
            Err(Legacy0x22ReadError::UnexpectedFamily(
                ContentsFamily::Family0x2c
            ))
        ));
    }

    #[test]
    fn rejects_overlapping_formatting_page_ranges() {
        let mut bytes = oracle_shape(0x57);
        let descriptor = 0x22 + 14;
        bytes[descriptor + 12..descriptor + 14].copy_from_slice(&2u16.to_le_bytes());

        assert!(matches!(
            parse_legacy_0x22_formatting_descriptor(StreamPath("/Contents".into()), &bytes),
            Err(Legacy0x22ReadError::FormattingPageOrderInvalid {
                character_fkp_page: 1,
                paragraph_fkp_page: 2,
                paragraph_fkp_end: 2,
            })
        ));
    }

    #[test]
    fn rejects_formatting_page_beyond_stream() {
        let mut bytes = oracle_shape(0x57);
        let descriptor = 0x22 + 14;
        bytes[descriptor + 14..descriptor + 16].copy_from_slice(&9u16.to_le_bytes());

        assert!(matches!(
            parse_legacy_0x22_formatting_descriptor(StreamPath("/Contents".into()), &bytes),
            Err(Legacy0x22ReadError::FormattingPageOutOfBounds {
                kind: "cell-style",
                page_index: 4,
                ..
            })
        ));
    }
}
