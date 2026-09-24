use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Legacy0x22Underline {
    None,
    Single,
    WordsOnly,
    Double,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22CharacterStyle {
    pub flags: u8,
    pub bold: bool,
    pub italic: bool,
    pub small_caps: bool,
    pub caps: bool,
    /// Table-relative font slot in the verified legacy 0x22 lane.
    pub font_index: Option<u8>,
    /// Raw byte/word beginning at payload byte 4. Its semantic role depends
    /// on the compact CHPX shape and must not be interpreted uniformly.
    pub raw_word_at_4: Option<u16>,
    /// Signed half-point delta from the legacy 10pt baseline when the
    /// observed compact CHPX shape independently grounds that interpretation.
    pub size_variation_half_points: Option<i16>,
    /// Explicit baseline shift in half-points for the verified 7-byte shape.
    pub baseline_shift_half_points: Option<i8>,
    /// Legacy Publisher palette slot, not the source RTF color-table ordinal.
    pub legacy_color_index: Option<u8>,
    /// Raw packed underline/letter-spacing field when present.
    pub underline_spacing_packed: Option<u16>,
    pub underline: Option<Legacy0x22Underline>,
    pub letter_spacing_eighth_points: Option<i16>,
    pub raw_payload: Vec<u8>,
}

impl Legacy0x22CharacterStyle {
    /// Exact point size represented as half-points, avoiding floating point.
    pub fn size_half_points(&self) -> Option<i16> {
        self.size_variation_half_points
            .and_then(|delta| 20i16.checked_add(delta))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Legacy0x22CharacterStyleError {
    EmptyPayload,
}

impl fmt::Display for Legacy0x22CharacterStyleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPayload => write!(f, "legacy 0x22 CHPX payload is empty"),
        }
    }
}

impl std::error::Error for Legacy0x22CharacterStyleError {}

fn read_i16_prefix(payload: &[u8], offset: usize) -> Option<i16> {
    match payload.get(offset..) {
        Some(bytes) if bytes.len() >= 2 => Some(i16::from_le_bytes([bytes[0], bytes[1]])),
        Some(bytes) if bytes.len() == 1 => Some(i16::from(i8::from_le_bytes([bytes[0]]))),
        _ => None,
    }
}

fn decode_spacing(packed: u16) -> i16 {
    let mut spacing = ((packed >> 2) & 0x1fff) as i16;
    if (spacing & 0x1000) != 0 {
        spacing -= 0x2000;
    }
    spacing
}

/// Decode the bounded, oracle-confirmed positional prefix of a legacy
/// Publisher 2/95/97 no-Quill CHPX payload.
///
/// The FKP byte-count length marker is not part of the supplied payload.
///
/// Evidence currently grounds:
/// - flags bits 0..3: bold / italic / small-caps / caps;
/// - font slot at byte 2;
/// - signed size variation at byte 4 for the ordinary 5-byte shape and the
///   currently observed color/underline extension shapes;
/// - explicit baseline shift at byte 6 for the verified 7-byte shape;
/// - legacy palette index at byte 7 for the verified 8-byte color shape;
/// - underline + letter-spacing packed field beginning at byte 8 for the
///   verified underline/spacing shapes.
///
/// The compact record is variable-shape. In particular, Microsoft writer
/// output for `\\up6` / `\\dn6` contains `14 00` at bytes 4..5 while
/// readback keeps the original 12 pt size, so that word is preserved but is
/// deliberately not decoded as a size delta in the 7-byte form.
///
/// Short or unrecognized payload shapes are valid: ungrounded fields remain
/// None and the complete raw payload is always preserved.
pub fn decode_legacy_0x22_character_style(
    payload: &[u8],
) -> Result<Legacy0x22CharacterStyle, Legacy0x22CharacterStyleError> {
    let flags = *payload
        .first()
        .ok_or(Legacy0x22CharacterStyleError::EmptyPayload)?;

    let raw_word_at_4 = match payload.get(4..) {
        Some(bytes) if bytes.len() >= 2 => Some(u16::from_le_bytes([bytes[0], bytes[1]])),
        Some(bytes) if bytes.len() == 1 => Some(u16::from(bytes[0])),
        _ => None,
    };

    let size_variation_half_points = if payload.len() == 5 {
        payload
            .get(4)
            .copied()
            .map(|byte| i16::from(i8::from_le_bytes([byte])))
    } else if payload.len() >= 8 {
        read_i16_prefix(payload, 4)
    } else {
        None
    };

    let baseline_shift_half_points = if payload.len() == 7 {
        payload.get(6).copied().map(|byte| byte as i8)
    } else {
        None
    };

    let legacy_color_index = if payload.len() == 8 {
        payload.get(7).copied()
    } else {
        None
    };

    let underline_spacing_packed = if payload.len() >= 9 {
        match payload.get(8..) {
            Some(bytes) if bytes.len() >= 2 => Some(u16::from_le_bytes([bytes[0], bytes[1]])),
            Some(bytes) if bytes.len() == 1 => Some(u16::from(bytes[0])),
            _ => None,
        }
    } else {
        None
    };

    let underline = underline_spacing_packed.map(|packed| match packed & 0x03 {
        0 => Legacy0x22Underline::None,
        1 => Legacy0x22Underline::Single,
        2 => Legacy0x22Underline::WordsOnly,
        _ => Legacy0x22Underline::Double,
    });

    Ok(Legacy0x22CharacterStyle {
        flags,
        bold: (flags & 0x01) != 0,
        italic: (flags & 0x02) != 0,
        small_caps: (flags & 0x04) != 0,
        caps: (flags & 0x08) != 0,
        font_index: payload.get(2).copied(),
        raw_word_at_4,
        size_variation_half_points,
        baseline_shift_half_points,
        legacy_color_index,
        underline_spacing_packed,
        underline,
        letter_spacing_eighth_points: underline_spacing_packed.map(decode_spacing),
        raw_payload: payload.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_oracle_character_flags() {
        let plain = decode_legacy_0x22_character_style(&[0x00]).unwrap();
        let bold = decode_legacy_0x22_character_style(&[0x01]).unwrap();
        let italic = decode_legacy_0x22_character_style(&[0x02]).unwrap();
        let both = decode_legacy_0x22_character_style(&[0x03]).unwrap();
        let small_caps = decode_legacy_0x22_character_style(&[0x04]).unwrap();
        let caps = decode_legacy_0x22_character_style(&[0x08]).unwrap();

        assert!(!plain.bold);
        assert!(bold.bold);
        assert!(italic.italic);
        assert!(both.bold && both.italic);
        assert!(small_caps.small_caps);
        assert!(caps.caps);
    }

    #[test]
    fn keeps_font_index_table_relative() {
        let style = decode_legacy_0x22_character_style(&[0x00, 0x00, 0x03]).unwrap();
        assert_eq!(style.font_index, Some(3));
    }

    #[test]
    fn decodes_oracle_size_delta_without_float() {
        let twelve = decode_legacy_0x22_character_style(&[0, 0, 0, 0, 0x04]).unwrap();
        let fourteen = decode_legacy_0x22_character_style(&[0, 0, 0, 0, 0x08]).unwrap();
        let twenty_four = decode_legacy_0x22_character_style(&[0, 0, 0, 0, 0x1c]).unwrap();

        assert_eq!(twelve.size_variation_half_points, Some(4));
        assert_eq!(twelve.size_half_points(), Some(24));
        assert_eq!(fourteen.size_half_points(), Some(28));
        assert_eq!(twenty_four.size_half_points(), Some(48));
    }

    #[test]
    fn decodes_signed_baseline_shift_without_false_size() {
        // Exact payload shape observed from the byte-pinned Microsoft writer
        // for \\up6 / \\dn6 in run 35514936850.
        let up = decode_legacy_0x22_character_style(&[0, 0, 0, 0, 0x14, 0, 0x06]).unwrap();
        let down = decode_legacy_0x22_character_style(&[0, 0, 0, 0, 0x14, 0, 0xfa]).unwrap();

        assert_eq!(up.raw_word_at_4, Some(0x0014));
        assert_eq!(up.size_variation_half_points, None);
        assert_eq!(up.size_half_points(), None);
        assert_eq!(up.baseline_shift_half_points, Some(6));

        assert_eq!(down.raw_word_at_4, Some(0x0014));
        assert_eq!(down.size_variation_half_points, None);
        assert_eq!(down.baseline_shift_half_points, Some(-6));
    }

    #[test]
    fn decodes_legacy_palette_slot() {
        let red = decode_legacy_0x22_character_style(&[0, 0, 0, 0, 4, 0, 0, 2]).unwrap();
        let green = decode_legacy_0x22_character_style(&[0, 0, 0, 0, 4, 0, 0, 3]).unwrap();
        let blue = decode_legacy_0x22_character_style(&[0, 0, 0, 0, 4, 0, 0, 4]).unwrap();

        assert_eq!(red.legacy_color_index, Some(2));
        assert_eq!(green.legacy_color_index, Some(3));
        assert_eq!(blue.legacy_color_index, Some(4));
        assert_eq!(red.baseline_shift_half_points, None);
    }

    #[test]
    fn decodes_underline_and_signed_letter_spacing() {
        let single =
            decode_legacy_0x22_character_style(&[0, 0, 0, 0, 4, 0, 0, 0, 0x01, 0]).unwrap();
        let words = decode_legacy_0x22_character_style(&[0, 0, 0, 0, 4, 0, 0, 0, 0x02, 0]).unwrap();
        let double =
            decode_legacy_0x22_character_style(&[0, 0, 0, 0, 4, 0, 0, 0, 0x03, 0]).unwrap();

        assert_eq!(single.underline, Some(Legacy0x22Underline::Single));
        assert_eq!(words.underline, Some(Legacy0x22Underline::WordsOnly));
        assert_eq!(double.underline, Some(Legacy0x22Underline::Double));

        let expanded =
            decode_legacy_0x22_character_style(&[0, 0, 0, 0, 4, 0, 0, 0, 0x30, 0]).unwrap();
        assert_eq!(expanded.letter_spacing_eighth_points, Some(12));

        let packed_negative = ((0x2000u16 - 12) << 2) & 0xfffc;
        let negative = decode_legacy_0x22_character_style(&[
            0,
            0,
            0,
            0,
            4,
            0,
            0,
            0,
            (packed_negative & 0xff) as u8,
            (packed_negative >> 8) as u8,
        ])
        .unwrap();
        assert_eq!(negative.letter_spacing_eighth_points, Some(-12));
    }

    #[test]
    fn rejects_empty_payload() {
        assert_eq!(
            decode_legacy_0x22_character_style(&[]),
            Err(Legacy0x22CharacterStyleError::EmptyPayload)
        );
    }
}
