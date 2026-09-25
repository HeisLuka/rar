use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Legacy0x22ParagraphAlignment {
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Legacy0x22LineSpacing {
    Multiple240(u16),
    ExactTwentiethPoints(u16),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22ParagraphScalarStyle {
    pub semantic_extent_or_tab_pos: u8,
    pub reserved_zero: u8,
    pub alignment_and_spacing: u8,
    pub alignment: Legacy0x22ParagraphAlignment,
    pub paragraph_char_spacing_preset: u8,
    pub right_indent_twips: Option<u16>,
    pub left_indent_twips: Option<u16>,
    pub first_line_indent_twips: Option<i16>,
    pub line_spacing_raw: Option<i16>,
    pub line_spacing: Option<Legacy0x22LineSpacing>,
    pub space_before_twips: Option<u8>,
    pub observed_after_before: Option<u8>,
    pub space_after_twips: Option<u8>,
    pub observed_after_after: Option<u8>,
    pub raw_payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22ParagraphSpecialStyle {
    pub semantic_extent_or_tab_pos: u8,
    pub discriminator: u8,
    pub raw_special_tail: Vec<u8>,
    pub raw_payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Legacy0x22ParagraphStyle {
    Scalar(Legacy0x22ParagraphScalarStyle),
    Special(Legacy0x22ParagraphSpecialStyle),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Legacy0x22ParagraphStyleError {
    EmptyPayload,
}

impl fmt::Display for Legacy0x22ParagraphStyleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPayload => write!(f, "legacy 0x22 PAPX payload is empty"),
        }
    }
}

impl std::error::Error for Legacy0x22ParagraphStyleError {}

fn read_u16(payload: &[u8], offset: usize) -> Option<u16> {
    let raw = payload.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([raw[0], raw[1]]))
}

fn read_i16(payload: &[u8], offset: usize) -> Option<i16> {
    let raw = payload.get(offset..offset + 2)?;
    Some(i16::from_le_bytes([raw[0], raw[1]]))
}

fn alignment_from_low2(value: u8) -> Legacy0x22ParagraphAlignment {
    match value & 0x03 {
        0 => Legacy0x22ParagraphAlignment::Left,
        1 => Legacy0x22ParagraphAlignment::Center,
        2 => Legacy0x22ParagraphAlignment::Right,
        _ => Legacy0x22ParagraphAlignment::Justify,
    }
}

/// Decode the bounded, independently-confirmed scalar prefix of a legacy
/// Publisher 2/95/97 no-Quill PAPX payload.
///
/// The FKP word-count length marker is not part of the supplied payload.
///
/// Native low-family corpus evidence and the byte-pinned Microsoft
/// MSWPUB2.CNV writer independently agree on these positional fields.
/// A nonzero payload byte 1 selects the observed optional tab/numbering
/// subrecord shape; that tail is deliberately preserved raw.
pub fn decode_legacy_0x22_paragraph_style(
    payload: &[u8],
) -> Result<Legacy0x22ParagraphStyle, Legacy0x22ParagraphStyleError> {
    let semantic_extent_or_tab_pos = *payload
        .first()
        .ok_or(Legacy0x22ParagraphStyleError::EmptyPayload)?;

    let second = payload.get(1).copied().unwrap_or(0);
    if second != 0 {
        return Ok(Legacy0x22ParagraphStyle::Special(
            Legacy0x22ParagraphSpecialStyle {
                semantic_extent_or_tab_pos,
                discriminator: second,
                raw_special_tail: payload[1..].to_vec(),
                raw_payload: payload.to_vec(),
            },
        ));
    }

    let alignment_and_spacing = payload.get(2).copied().unwrap_or(0);
    let line_spacing_raw = read_i16(payload, 9);
    let line_spacing = line_spacing_raw.map(|raw| {
        if raw >= 0 {
            Legacy0x22LineSpacing::Multiple240(raw as u16)
        } else {
            Legacy0x22LineSpacing::ExactTwentiethPoints(raw.unsigned_abs())
        }
    });

    Ok(Legacy0x22ParagraphStyle::Scalar(
        Legacy0x22ParagraphScalarStyle {
            semantic_extent_or_tab_pos,
            reserved_zero: second,
            alignment_and_spacing,
            alignment: alignment_from_low2(alignment_and_spacing),
            paragraph_char_spacing_preset: (alignment_and_spacing >> 3) & 0x0F,
            right_indent_twips: read_u16(payload, 3),
            left_indent_twips: read_u16(payload, 5),
            first_line_indent_twips: read_i16(payload, 7),
            line_spacing_raw,
            line_spacing,
            space_before_twips: payload.get(11).copied(),
            observed_after_before: payload.get(12).copied(),
            space_after_twips: payload.get(13).copied(),
            observed_after_after: payload.get(14).copied(),
            raw_payload: payload.to_vec(),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar(payload: &[u8]) -> Legacy0x22ParagraphScalarStyle {
        match decode_legacy_0x22_paragraph_style(payload).unwrap() {
            Legacy0x22ParagraphStyle::Scalar(style) => style,
            Legacy0x22ParagraphStyle::Special(_) => panic!("expected scalar PAPX"),
        }
    }

    #[test]
    fn decodes_oracle_alignment_values() {
        let center = scalar(&[0x04, 0x00, 0x01, 0x00]);
        let right = scalar(&[0x04, 0x00, 0x02, 0x00]);
        let justify = scalar(&[0x04, 0x00, 0x03, 0x00]);

        assert_eq!(center.alignment, Legacy0x22ParagraphAlignment::Center);
        assert_eq!(right.alignment, Legacy0x22ParagraphAlignment::Right);
        assert_eq!(justify.alignment, Legacy0x22ParagraphAlignment::Justify);
    }

    #[test]
    fn decodes_oracle_indents_in_twips() {
        let right = scalar(&[0x06, 0x00, 0x00, 0xD0, 0x02, 0x00]);
        assert_eq!(right.right_indent_twips, Some(720));
        assert_eq!(right.left_indent_twips, None);

        let left = scalar(&[0x08, 0x00, 0x00, 0x00, 0x00, 0xD0, 0x02, 0x00]);
        assert_eq!(left.right_indent_twips, Some(0));
        assert_eq!(left.left_indent_twips, Some(720));

        let first = scalar(&[0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x68, 0x01, 0x00]);
        assert_eq!(first.first_line_indent_twips, Some(360));
    }

    #[test]
    fn decodes_oracle_line_spacing_without_float_loss() {
        let multiple = scalar(&[
            0x0C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x68, 0x01, 0x00,
        ]);
        assert_eq!(multiple.line_spacing_raw, Some(360));
        assert_eq!(
            multiple.line_spacing,
            Some(Legacy0x22LineSpacing::Multiple240(360))
        );

        let exact = scalar(&[
            0x0C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x98, 0xFE, 0x00,
        ]);
        assert_eq!(exact.line_spacing_raw, Some(-360));
        assert_eq!(
            exact.line_spacing,
            Some(Legacy0x22LineSpacing::ExactTwentiethPoints(360))
        );
    }

    #[test]
    fn decodes_oracle_before_and_after_spacing() {
        let before = scalar(&[
            0x0D, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF0,
        ]);
        assert_eq!(before.space_before_twips, Some(240));

        let after = scalar(&[
            0x0F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF0,
        ]);
        assert_eq!(after.space_after_twips, Some(240));
    }

    #[test]
    fn preserves_tab_subrecord_raw() {
        let payload = [0x02, 0x0A, 0xFE, 0x00, 0x01, 0xD0, 0x02, 0x00];
        let parsed = decode_legacy_0x22_paragraph_style(&payload).unwrap();

        match parsed {
            Legacy0x22ParagraphStyle::Special(style) => {
                assert_eq!(style.semantic_extent_or_tab_pos, 2);
                assert_eq!(style.discriminator, 0x0A);
                assert_eq!(style.raw_special_tail, payload[1..]);
                assert_eq!(style.raw_payload, payload);
            }
            Legacy0x22ParagraphStyle::Scalar(_) => panic!("expected special PAPX"),
        }
    }

    #[test]
    fn rejects_empty_payload() {
        assert_eq!(
            decode_legacy_0x22_paragraph_style(&[]),
            Err(Legacy0x22ParagraphStyleError::EmptyPayload)
        );
    }
}
