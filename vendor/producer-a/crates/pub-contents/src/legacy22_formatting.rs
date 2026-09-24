use crate::{
    FKP_PAGE_SIZE, FkpPropertyKind, FkpReadError, Legacy0x22CellStyle, Legacy0x22CellStyleError,
    Legacy0x22CharacterStyle, Legacy0x22CharacterStyleError, Legacy0x22FormattingDescriptor,
    Legacy0x22ParagraphStyle, Legacy0x22ParagraphStyleError, Legacy0x22ReadError,
    decode_legacy_0x22_cell_style, decode_legacy_0x22_character_style,
    decode_legacy_0x22_paragraph_style, parse_fkp_page, parse_legacy_0x22_formatting_descriptor,
};
use pub_core::{RawSpan, StreamPath};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22CharacterRun {
    pub fc_first: u32,
    pub fc_lim: u32,
    pub fc_first_source: RawSpan,
    pub fc_lim_source: RawSpan,
    pub property_pointer: u8,
    pub property_pointer_source: RawSpan,
    pub property_source: Option<RawSpan>,
    pub style: Option<Legacy0x22CharacterStyle>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22ParagraphRun {
    pub fc_first: u32,
    pub fc_lim: u32,
    pub fc_first_source: RawSpan,
    pub fc_lim_source: RawSpan,
    pub property_pointer: u8,
    pub property_pointer_source: RawSpan,
    pub property_source: Option<RawSpan>,
    pub style: Option<Legacy0x22ParagraphStyle>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22CellStyleBoundary {
    /// Application position used by the Publisher 2 cell-style parser.
    /// The wire value stored in the FKP is two bytes larger.
    pub position: u32,
    pub stored_position: u32,
    pub stored_position_source: RawSpan,
    pub property_pointer: u8,
    pub property_pointer_source: RawSpan,
    pub property_source: Option<RawSpan>,
    pub style: Option<Legacy0x22CellStyle>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22FormattingRuns {
    pub descriptor: Legacy0x22FormattingDescriptor,
    pub character_runs: Vec<Legacy0x22CharacterRun>,
    pub paragraph_runs: Vec<Legacy0x22ParagraphRun>,
    pub cell_style_boundaries: Vec<Legacy0x22CellStyleBoundary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Legacy0x22FormattingRunsError {
    Descriptor(Legacy0x22ReadError),
    CharacterFkp(FkpReadError),
    ParagraphFkp(FkpReadError),
    CellFkp(FkpReadError),
    CharacterStyle {
        run_index: usize,
        source: Legacy0x22CharacterStyleError,
    },
    ParagraphStyle {
        run_index: usize,
        source: Legacy0x22ParagraphStyleError,
    },
    CellBoundaryUnderflow {
        boundary_index: usize,
        stored_position: u32,
    },
    CellStyle {
        boundary_index: usize,
        source: Legacy0x22CellStyleError,
    },
}

impl fmt::Display for Legacy0x22FormattingRunsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Descriptor(error) => write!(f, "legacy 0x22 descriptor: {error}"),
            Self::CharacterFkp(error) => write!(f, "legacy 0x22 character FKP: {error}"),
            Self::ParagraphFkp(error) => write!(f, "legacy 0x22 paragraph FKP: {error}"),
            Self::CellFkp(error) => write!(f, "legacy 0x22 cell-style FKP: {error}"),
            Self::CharacterStyle { run_index, source } => {
                write!(f, "legacy 0x22 character run {run_index}: {source}")
            }
            Self::ParagraphStyle { run_index, source } => {
                write!(f, "legacy 0x22 paragraph run {run_index}: {source}")
            }
            Self::CellBoundaryUnderflow {
                boundary_index,
                stored_position,
            } => write!(
                f,
                "legacy 0x22 cell-style boundary {boundary_index} stored position {stored_position} cannot apply the observed -2 adjustment"
            ),
            Self::CellStyle {
                boundary_index,
                source,
            } => write!(
                f,
                "legacy 0x22 cell-style boundary {boundary_index}: {source}"
            ),
        }
    }
}

impl std::error::Error for Legacy0x22FormattingRunsError {}

impl From<Legacy0x22ReadError> for Legacy0x22FormattingRunsError {
    fn from(value: Legacy0x22ReadError) -> Self {
        Self::Descriptor(value)
    }
}

/// Parse the oracle-grounded legacy 0x22 formatting lane end-to-end.
///
/// This composes the physical descriptor, its referenced character/paragraph/
/// cell-style FKP pages, and only the bounded fields grounded by the Microsoft
/// Publisher 2 oracle, native corpus, and independent historical parser
/// evidence.
///
/// The result remains a physical/provenance view. It does not claim that the
/// legacy 0x22 grammar applies to mature 0x2C Contents.
pub fn parse_legacy_0x22_formatting_runs(
    stream: StreamPath,
    bytes: &[u8],
) -> Result<Legacy0x22FormattingRuns, Legacy0x22FormattingRunsError> {
    let descriptor = parse_legacy_0x22_formatting_descriptor(stream.clone(), bytes)?;

    // Native Publisher 2 files with no text story can carry an empty text
    // range together with zero character/paragraph FKP page indices. Page 0
    // is the Contents header, not an FKP page, so do not reinterpret it as
    // formatting data. This shape is independently accepted by MSWPUB2.CNV
    // and reads back as an empty RTF story.
    if descriptor.text_start == descriptor.text_end
        && descriptor.character_fkp_page == 0
        && descriptor.paragraph_fkp_page == 0
        && descriptor.observed_page_3 == 0
        && descriptor.observed_page_3_end == 0
    {
        return Ok(Legacy0x22FormattingRuns {
            descriptor,
            character_runs: Vec::new(),
            paragraph_runs: Vec::new(),
            cell_style_boundaries: Vec::new(),
        });
    }

    let mut character_runs = Vec::new();
    for page_index in descriptor.character_fkp_page_range() {
        let page_offset = usize::from(page_index) * FKP_PAGE_SIZE;
        let page = &bytes[page_offset..page_offset + FKP_PAGE_SIZE];
        let fkp = parse_fkp_page(
            stream.clone(),
            page_offset as u64,
            page,
            FkpPropertyKind::Character,
        )
        .map_err(Legacy0x22FormattingRunsError::CharacterFkp)?;

        for run in fkp.runs {
            let run_index = character_runs.len();
            let property_source = run
                .property
                .as_ref()
                .map(|property| property.source.clone());
            let style = match run.property {
                Some(property) => Some(
                    decode_legacy_0x22_character_style(&property.payload).map_err(|source| {
                        Legacy0x22FormattingRunsError::CharacterStyle { run_index, source }
                    })?,
                ),
                None => None,
            };

            character_runs.push(Legacy0x22CharacterRun {
                fc_first: run.fc_first,
                fc_lim: run.fc_lim,
                fc_first_source: run.fc_first_source,
                fc_lim_source: run.fc_lim_source,
                property_pointer: run.property_pointer,
                property_pointer_source: run.property_pointer_source,
                property_source,
                style,
            });
        }
    }

    let mut paragraph_runs = Vec::new();
    for page_index in descriptor.paragraph_fkp_page_range() {
        let page_offset = usize::from(page_index) * FKP_PAGE_SIZE;
        let page = &bytes[page_offset..page_offset + FKP_PAGE_SIZE];
        let fkp = parse_fkp_page(
            stream.clone(),
            page_offset as u64,
            page,
            FkpPropertyKind::Paragraph,
        )
        .map_err(Legacy0x22FormattingRunsError::ParagraphFkp)?;

        for run in fkp.runs {
            let run_index = paragraph_runs.len();
            let property_source = run
                .property
                .as_ref()
                .map(|property| property.source.clone());
            let style = match run.property {
                Some(property) => Some(
                    decode_legacy_0x22_paragraph_style(&property.payload).map_err(|source| {
                        Legacy0x22FormattingRunsError::ParagraphStyle { run_index, source }
                    })?,
                ),
                None => None,
            };

            paragraph_runs.push(Legacy0x22ParagraphRun {
                fc_first: run.fc_first,
                fc_lim: run.fc_lim,
                fc_first_source: run.fc_first_source,
                fc_lim_source: run.fc_lim_source,
                property_pointer: run.property_pointer,
                property_pointer_source: run.property_pointer_source,
                property_source,
                style,
            });
        }
    }

    let mut cell_style_boundaries = Vec::new();
    for page_index in descriptor.cell_style_fkp_page_range() {
        let page_offset = usize::from(page_index) * FKP_PAGE_SIZE;
        let page = &bytes[page_offset..page_offset + FKP_PAGE_SIZE];
        let fkp = parse_fkp_page(
            stream.clone(),
            page_offset as u64,
            page,
            FkpPropertyKind::Cell,
        )
        .map_err(Legacy0x22FormattingRunsError::CellFkp)?;

        for run in fkp.runs {
            let boundary_index = cell_style_boundaries.len();
            let stored_position = run.fc_lim;
            let position = stored_position.checked_sub(2).ok_or(
                Legacy0x22FormattingRunsError::CellBoundaryUnderflow {
                    boundary_index,
                    stored_position,
                },
            )?;
            let property_source = run
                .property
                .as_ref()
                .map(|property| property.source.clone());
            let style = match run.property {
                Some(property) => Some(decode_legacy_0x22_cell_style(&property.payload).map_err(
                    |source| Legacy0x22FormattingRunsError::CellStyle {
                        boundary_index,
                        source,
                    },
                )?),
                None => None,
            };

            cell_style_boundaries.push(Legacy0x22CellStyleBoundary {
                position,
                stored_position,
                stored_position_source: run.fc_lim_source,
                property_pointer: run.property_pointer,
                property_pointer_source: run.property_pointer_source,
                property_source,
                style,
            });
        }
    }

    Ok(Legacy0x22FormattingRuns {
        descriptor,
        character_runs,
        paragraph_runs,
        cell_style_boundaries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CONTENTS_0X22_MAGIC, Legacy0x22ParagraphAlignment, Legacy0x22ParagraphStyle,
        Legacy0x22Underline,
    };

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0u8; 0x800];
        bytes[0..4].copy_from_slice(&CONTENTS_0X22_MAGIC);
        bytes[0x12..0x16].copy_from_slice(&0x22u32.to_le_bytes());

        let descriptor = 0x22 + 14;
        bytes[descriptor..descriptor + 4].copy_from_slice(&0x50u32.to_le_bytes());
        bytes[descriptor + 4..descriptor + 8].copy_from_slice(&0x57u32.to_le_bytes());
        bytes[descriptor + 8..descriptor + 10].copy_from_slice(&1u16.to_le_bytes());
        bytes[descriptor + 10..descriptor + 12].copy_from_slice(&2u16.to_le_bytes());
        bytes[descriptor + 12..descriptor + 14].copy_from_slice(&3u16.to_le_bytes());
        bytes[descriptor + 14..descriptor + 16].copy_from_slice(&4u16.to_le_bytes());

        let ch = 0x200;
        bytes[ch..ch + 4].copy_from_slice(&0x50u32.to_le_bytes());
        bytes[ch + 4..ch + 8].copy_from_slice(&0x51u32.to_le_bytes());
        bytes[ch + 8..ch + 12].copy_from_slice(&0x57u32.to_le_bytes());
        bytes[ch + 12] = 0xFC;
        bytes[ch + 13] = 0xF6;
        bytes[ch + 0x1F8..ch + 0x1FE].copy_from_slice(&[0x05, 0x01, 0x00, 0x00, 0x00, 0x04]);
        bytes[ch + 0x1EC..ch + 0x1F6]
            .copy_from_slice(&[0x09, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x01]);
        bytes[ch + 0x1FF] = 2;

        let pap = 0x400;
        bytes[pap..pap + 4].copy_from_slice(&0x50u32.to_le_bytes());
        bytes[pap + 4..pap + 8].copy_from_slice(&0x57u32.to_le_bytes());
        bytes[pap + 8] = 0xFD;
        bytes[pap + 0x1FA..pap + 0x1FF].copy_from_slice(&[0x02, 0x04, 0x00, 0x01, 0x00]);
        bytes[pap + 0x1FF] = 1;

        bytes
    }

    fn oracle_range_fixture(
        relative_ends: &[u32],
        pointers: &[u8],
        records: &[(usize, &[u8])],
    ) -> Vec<u8> {
        assert_eq!(relative_ends.len(), pointers.len());
        let text_start = 0x50u32;
        let text_end = text_start + relative_ends.last().copied().unwrap_or(1);
        let mut bytes = vec![0u8; 0x800];
        bytes[0..4].copy_from_slice(&CONTENTS_0X22_MAGIC);
        bytes[0x12..0x16].copy_from_slice(&0x22u32.to_le_bytes());

        let descriptor = 0x22 + 14;
        bytes[descriptor..descriptor + 4].copy_from_slice(&text_start.to_le_bytes());
        bytes[descriptor + 4..descriptor + 8].copy_from_slice(&text_end.to_le_bytes());
        bytes[descriptor + 8..descriptor + 10].copy_from_slice(&1u16.to_le_bytes());
        bytes[descriptor + 10..descriptor + 12].copy_from_slice(&2u16.to_le_bytes());
        bytes[descriptor + 12..descriptor + 14].copy_from_slice(&3u16.to_le_bytes());
        bytes[descriptor + 14..descriptor + 16].copy_from_slice(&4u16.to_le_bytes());

        let ch = 0x200;
        let mut fcs = Vec::with_capacity(relative_ends.len() + 1);
        fcs.push(text_start);
        fcs.extend(relative_ends.iter().map(|end| text_start + end));
        for (index, fc) in fcs.iter().enumerate() {
            bytes[ch + index * 4..ch + index * 4 + 4].copy_from_slice(&fc.to_le_bytes());
        }
        let pointer_start = ch + fcs.len() * 4;
        bytes[pointer_start..pointer_start + pointers.len()].copy_from_slice(pointers);
        for (offset, record) in records {
            bytes[ch + *offset..ch + *offset + record.len()].copy_from_slice(record);
        }
        bytes[ch + 0x1FF] = pointers.len() as u8;

        let pap = 0x400;
        bytes[pap..pap + 4].copy_from_slice(&text_start.to_le_bytes());
        bytes[pap + 4..pap + 8].copy_from_slice(&text_end.to_le_bytes());
        bytes[pap + 8] = 0;
        bytes[pap + 0x1FF] = 1;

        bytes
    }

    fn oracle_paragraph_fixture(pointers: &[u8]) -> Vec<u8> {
        assert_eq!(pointers.len(), 3);
        let text_start = 0x50u32;
        let absolute_ends = [0x57u32, 0x5Fu32, 0x61u32];
        let mut bytes = vec![0u8; 0x800];
        bytes[0..4].copy_from_slice(&CONTENTS_0X22_MAGIC);
        bytes[0x12..0x16].copy_from_slice(&0x22u32.to_le_bytes());

        let descriptor = 0x22 + 14;
        bytes[descriptor..descriptor + 4].copy_from_slice(&text_start.to_le_bytes());
        bytes[descriptor + 4..descriptor + 8].copy_from_slice(&0x61u32.to_le_bytes());
        bytes[descriptor + 8..descriptor + 10].copy_from_slice(&1u16.to_le_bytes());
        bytes[descriptor + 10..descriptor + 12].copy_from_slice(&2u16.to_le_bytes());
        bytes[descriptor + 12..descriptor + 14].copy_from_slice(&3u16.to_le_bytes());
        bytes[descriptor + 14..descriptor + 16].copy_from_slice(&4u16.to_le_bytes());

        let ch = 0x200;
        bytes[ch..ch + 4].copy_from_slice(&text_start.to_le_bytes());
        bytes[ch + 4..ch + 8].copy_from_slice(&0x61u32.to_le_bytes());
        bytes[ch + 8] = 0;
        bytes[ch + 0x1FF] = 1;

        let pap = 0x400;
        bytes[pap..pap + 4].copy_from_slice(&text_start.to_le_bytes());
        for (index, fc) in absolute_ends.iter().enumerate() {
            let offset = pap + (index + 1) * 4;
            bytes[offset..offset + 4].copy_from_slice(&fc.to_le_bytes());
        }
        let pointer_start = pap + 16;
        bytes[pointer_start..pointer_start + pointers.len()].copy_from_slice(pointers);
        if pointers.contains(&0xFD) {
            bytes[pap + 0x1FA..pap + 0x1FF].copy_from_slice(&[0x02, 0x04, 0x00, 0x01, 0x00]);
        }
        bytes[pap + 0x1FF] = 3;

        bytes
    }

    #[test]
    fn empty_native_shape_does_not_parse_page_zero_as_fkp() {
        let mut bytes = vec![0u8; 0x800];
        bytes[0..4].copy_from_slice(&CONTENTS_0X22_MAGIC);
        bytes[0x12..0x16].copy_from_slice(&0x22u32.to_le_bytes());

        let descriptor = 0x22 + 14;
        bytes[descriptor..descriptor + 4].copy_from_slice(&0x50u32.to_le_bytes());
        bytes[descriptor + 4..descriptor + 8].copy_from_slice(&0x50u32.to_le_bytes());

        let parsed =
            parse_legacy_0x22_formatting_runs(StreamPath("/Contents".into()), &bytes).unwrap();

        assert_eq!(parsed.descriptor.text_start, parsed.descriptor.text_end);
        assert_eq!(parsed.descriptor.character_fkp_page, 0);
        assert_eq!(parsed.descriptor.paragraph_fkp_page, 0);
        assert!(parsed.character_runs.is_empty());
        assert!(parsed.paragraph_runs.is_empty());
    }

    #[test]
    fn walks_all_paragraph_fkp_pages_until_observed_third_boundary() {
        let mut bytes = vec![0u8; 0xA00];
        bytes[0..4].copy_from_slice(&CONTENTS_0X22_MAGIC);
        bytes[0x12..0x16].copy_from_slice(&0x22u32.to_le_bytes());

        let descriptor = 0x22 + 14;
        bytes[descriptor..descriptor + 4].copy_from_slice(&0x50u32.to_le_bytes());
        bytes[descriptor + 4..descriptor + 8].copy_from_slice(&0x60u32.to_le_bytes());
        bytes[descriptor + 8..descriptor + 10].copy_from_slice(&1u16.to_le_bytes());
        bytes[descriptor + 10..descriptor + 12].copy_from_slice(&2u16.to_le_bytes());
        bytes[descriptor + 12..descriptor + 14].copy_from_slice(&4u16.to_le_bytes());
        bytes[descriptor + 14..descriptor + 16].copy_from_slice(&5u16.to_le_bytes());

        let ch = 0x200;
        bytes[ch..ch + 4].copy_from_slice(&0x50u32.to_le_bytes());
        bytes[ch + 4..ch + 8].copy_from_slice(&0x60u32.to_le_bytes());
        bytes[ch + 8] = 0;
        bytes[ch + 0x1FF] = 1;

        let pap1 = 0x400;
        bytes[pap1..pap1 + 4].copy_from_slice(&0x50u32.to_le_bytes());
        bytes[pap1 + 4..pap1 + 8].copy_from_slice(&0x58u32.to_le_bytes());
        bytes[pap1 + 8] = 0;
        bytes[pap1 + 0x1FF] = 1;

        let pap2 = 0x600;
        bytes[pap2..pap2 + 4].copy_from_slice(&0x58u32.to_le_bytes());
        bytes[pap2 + 4..pap2 + 8].copy_from_slice(&0x60u32.to_le_bytes());
        bytes[pap2 + 8] = 0;
        bytes[pap2 + 0x1FF] = 1;

        let parsed =
            parse_legacy_0x22_formatting_runs(StreamPath("/Contents".into()), &bytes).unwrap();

        assert_eq!(parsed.character_runs.len(), 1);
        assert_eq!(parsed.paragraph_runs.len(), 2);
        assert_eq!(parsed.paragraph_runs[0].fc_first, 0x50);
        assert_eq!(parsed.paragraph_runs[0].fc_lim, 0x58);
        assert_eq!(parsed.paragraph_runs[1].fc_first, 0x58);
        assert_eq!(parsed.paragraph_runs[1].fc_lim, 0x60);
        assert_eq!(parsed.paragraph_runs[1].fc_first_source.offset, 0x600);
    }

    #[test]
    fn composes_cell_style_boundary_with_observed_minus_two_adjustment() {
        let mut bytes = fixture();
        let cell = 0x600;
        bytes[cell..cell + 4].copy_from_slice(&0x50u32.to_le_bytes());
        bytes[cell + 4..cell + 8].copy_from_slice(&0x57u32.to_le_bytes());
        bytes[cell + 8] = 0xF2;
        let payload = [
            0x00, 0x00, 0x00, 0x01, 0x01, 0x5a, 0x00, 0x5a, 0x00, 0x5a, 0x00, 0x5a, 0x00, 0x00,
            0x81, 0x40, 0x02, 0x00, 0x81, 0x00,
        ];
        bytes[cell + 0x1E4] = payload.len() as u8;
        bytes[cell + 0x1E5..cell + 0x1E5 + payload.len()].copy_from_slice(&payload);
        bytes[cell + 0x1FF] = 1;

        let parsed =
            parse_legacy_0x22_formatting_runs(StreamPath("/Contents".into()), &bytes).unwrap();

        assert_eq!(parsed.cell_style_boundaries.len(), 1);
        let boundary = &parsed.cell_style_boundaries[0];
        assert_eq!(boundary.stored_position, 0x57);
        assert_eq!(boundary.position, 0x55);
        assert_eq!(boundary.stored_position_source.offset, 0x604);
        let style = boundary.style.as_ref().unwrap();
        assert_eq!(style.fill_pattern_id, Some(1));
        assert_eq!(style.borders[0].width_quarter_points, 1);
        assert_eq!(style.borders[0].legacy_color_index, Some(0x40));
        assert_eq!(style.borders[1].width_quarter_points, 8);
    }

    #[test]
    fn composes_descriptor_fkp_and_typed_styles() {
        let parsed =
            parse_legacy_0x22_formatting_runs(StreamPath("/Contents".into()), &fixture()).unwrap();

        assert_eq!(parsed.descriptor.character_fkp_offset(), 0x200);
        assert_eq!(parsed.descriptor.paragraph_fkp_offset(), 0x400);

        assert_eq!(parsed.character_runs.len(), 2);
        let first = parsed.character_runs[0].style.as_ref().unwrap();
        assert!(first.bold);
        assert_eq!(first.size_half_points(), Some(24));

        let second = parsed.character_runs[1].style.as_ref().unwrap();
        assert_eq!(second.underline, Some(Legacy0x22Underline::Single));

        assert_eq!(parsed.paragraph_runs.len(), 1);
        match parsed.paragraph_runs[0].style.as_ref().unwrap() {
            Legacy0x22ParagraphStyle::Scalar(style) => {
                assert_eq!(style.alignment, Legacy0x22ParagraphAlignment::Center);
            }
            Legacy0x22ParagraphStyle::Special(_) => panic!("expected scalar paragraph style"),
        }
    }

    #[test]
    fn matches_oracle_first_middle_last_and_disjoint_character_ownership() {
        type RangeCase<'a> = (&'a str, &'a [u32], &'a [u8], &'a [bool]);

        const PLAIN: &[u8] = &[0x05, 0x00, 0x00, 0x00, 0x00, 0x04];
        const BOLD: &[u8] = &[0x05, 0x01, 0x00, 0x00, 0x00, 0x04];

        let cases: &[RangeCase<'_>] = &[
            ("first", &[1, 10], &[0xFC, 0xF9], &[true, false]),
            (
                "middle",
                &[1, 2, 10],
                &[0xFC, 0xF9, 0xF6],
                &[false, true, false],
            ),
            (
                "last",
                &[4, 5, 10],
                &[0xFC, 0xF9, 0xF6],
                &[false, true, false],
            ),
            (
                "edges",
                &[1, 4, 5, 10],
                &[0xFC, 0xF9, 0xF6, 0xF3],
                &[true, false, true, false],
            ),
        ];

        for (name, ends, pointers, expected_bold) in cases {
            let records: Vec<(usize, &[u8])> = pointers
                .iter()
                .enumerate()
                .map(|(index, pointer)| {
                    let record = if expected_bold[index] { BOLD } else { PLAIN };
                    (usize::from(*pointer) * 2, record)
                })
                .collect();
            let bytes = oracle_range_fixture(ends, pointers, &records);
            let parsed = parse_legacy_0x22_formatting_runs(StreamPath("/Contents".into()), &bytes)
                .unwrap_or_else(|error| panic!("{name}: {error}"));

            assert_eq!(parsed.character_runs.len(), expected_bold.len(), "{name}");
            let actual_ends: Vec<u32> = parsed
                .character_runs
                .iter()
                .map(|run| run.fc_lim - parsed.descriptor.text_start)
                .collect();
            assert_eq!(actual_ends, *ends, "{name}");

            let actual_bold: Vec<bool> = parsed
                .character_runs
                .iter()
                .map(|run| run.style.as_ref().map(|style| style.bold).unwrap_or(false))
                .collect();
            assert_eq!(actual_bold, *expected_bold, "{name}");
        }
    }

    #[test]
    fn keeps_duplicate_identical_chpx_records_physically_distinct() {
        const PLAIN: &[u8] = &[0x05, 0x00, 0x00, 0x00, 0x00, 0x04];
        const BOLD: &[u8] = &[0x05, 0x01, 0x00, 0x00, 0x00, 0x04];
        let bytes = oracle_range_fixture(
            &[1, 4, 5, 10],
            &[0xFC, 0xF9, 0xF6, 0xF3],
            &[(0x1F8, BOLD), (0x1F2, PLAIN), (0x1EC, BOLD), (0x1E6, PLAIN)],
        );

        let parsed =
            parse_legacy_0x22_formatting_runs(StreamPath("/Contents".into()), &bytes).unwrap();
        let first_bold = &parsed.character_runs[0];
        let second_bold = &parsed.character_runs[2];

        assert_eq!(
            first_bold.style.as_ref().unwrap().raw_payload,
            second_bold.style.as_ref().unwrap().raw_payload
        );
        assert_ne!(first_bold.property_pointer, second_bold.property_pointer);
        assert_ne!(first_bold.property_source, second_bold.property_source);
        assert_eq!(
            first_bold.property_source,
            Some(RawSpan {
                stream: StreamPath("/Contents".into()),
                offset: 0x3F8,
                len: 6,
            })
        );
        assert_eq!(
            second_bold.property_source,
            Some(RawSpan {
                stream: StreamPath("/Contents".into()),
                offset: 0x3EC,
                len: 6,
            })
        );
        assert_eq!(
            first_bold.property_pointer_source,
            RawSpan {
                stream: StreamPath("/Contents".into()),
                offset: 0x214,
                len: 1,
            }
        );
        assert_eq!(
            second_bold.property_pointer_source,
            RawSpan {
                stream: StreamPath("/Contents".into()),
                offset: 0x216,
                len: 1,
            }
        );
    }

    #[test]
    fn matches_oracle_red_middle_character_range() {
        const PLAIN: &[u8] = &[0x05, 0x00, 0x00, 0x00, 0x00, 0x04];
        const RED: &[u8] = &[0x08, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x02];
        let bytes = oracle_range_fixture(
            &[1, 2, 10],
            &[0xFC, 0xF7, 0xF4],
            &[(0x1F8, PLAIN), (0x1EE, RED), (0x1E8, PLAIN)],
        );

        let parsed =
            parse_legacy_0x22_formatting_runs(StreamPath("/Contents".into()), &bytes).unwrap();
        assert_eq!(
            parsed
                .character_runs
                .iter()
                .map(|run| run.fc_lim - parsed.descriptor.text_start)
                .collect::<Vec<_>>(),
            vec![1, 2, 10]
        );
        assert_eq!(
            parsed.character_runs[1]
                .style
                .as_ref()
                .unwrap()
                .legacy_color_index,
            Some(2)
        );
    }

    #[test]
    fn matches_oracle_multi_paragraph_pointer_ownership() {
        let cases: &[(&str, [u8; 3], [bool; 3])] = &[
            ("base", [0, 0, 0], [false, false, false]),
            ("center_first", [0xFD, 0, 0], [true, false, false]),
            ("center_second", [0, 0xFD, 0xFD], [false, true, true]),
            ("center_both", [0xFD, 0xFD, 0xFD], [true, true, true]),
        ];

        for (name, pointers, centered) in cases {
            let bytes = oracle_paragraph_fixture(pointers);
            let parsed = parse_legacy_0x22_formatting_runs(StreamPath("/Contents".into()), &bytes)
                .unwrap_or_else(|error| panic!("{name}: {error}"));

            assert_eq!(
                parsed
                    .paragraph_runs
                    .iter()
                    .map(|run| run.fc_lim - parsed.descriptor.text_start)
                    .collect::<Vec<_>>(),
                vec![7, 15, 17],
                "{name}"
            );

            for (index, expected_centered) in centered.iter().enumerate() {
                let run = &parsed.paragraph_runs[index];
                if *expected_centered {
                    match run.style.as_ref().expect("centered run must carry PAPX") {
                        Legacy0x22ParagraphStyle::Scalar(style) => {
                            assert_eq!(
                                style.alignment,
                                Legacy0x22ParagraphAlignment::Center,
                                "{name} run {index}"
                            );
                        }
                        Legacy0x22ParagraphStyle::Special(_) => {
                            panic!("{name} run {index}: expected scalar PAPX")
                        }
                    }
                } else {
                    assert!(run.style.is_none(), "{name} run {index}");
                }
            }
        }

        let second = parse_legacy_0x22_formatting_runs(
            StreamPath("/Contents".into()),
            &oracle_paragraph_fixture(&[0, 0xFD, 0xFD]),
        )
        .unwrap();
        assert_eq!(
            second.paragraph_runs[1].property_source,
            second.paragraph_runs[2].property_source
        );
    }

    #[test]
    fn preserves_property_provenance() {
        let parsed =
            parse_legacy_0x22_formatting_runs(StreamPath("/Contents".into()), &fixture()).unwrap();

        assert_eq!(
            parsed.character_runs[0].property_source,
            Some(RawSpan {
                stream: StreamPath("/Contents".into()),
                offset: 0x3F8,
                len: 6,
            })
        );
        assert_eq!(
            parsed.paragraph_runs[0].property_source,
            Some(RawSpan {
                stream: StreamPath("/Contents".into()),
                offset: 0x5FA,
                len: 5,
            })
        );
        assert_eq!(
            parsed.character_runs[0].fc_first_source,
            RawSpan {
                stream: StreamPath("/Contents".into()),
                offset: 0x200,
                len: 4,
            }
        );
        assert_eq!(
            parsed.character_runs[0].property_pointer_source,
            RawSpan {
                stream: StreamPath("/Contents".into()),
                offset: 0x20C,
                len: 1,
            }
        );
    }
}
