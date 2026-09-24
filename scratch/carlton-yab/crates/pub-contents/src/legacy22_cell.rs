use serde::{Deserialize, Serialize};
use std::fmt;

const CELL_STYLE_BORDER_OFFSET: usize = 14;
const CELL_STYLE_MAX_BORDERS: usize = 4;

pub const LEGACY_0X22_CELL_HORIZONTAL_MERGE_START_FLAG: u16 = 0x0001;
pub const LEGACY_0X22_CELL_HORIZONTAL_MERGE_CONTINUATION_FLAG: u16 = 0x0004;
/// Gates the explicit right-border slot in the tested Publisher 2 cell-style record.
///
/// The Microsoft reader ignores a populated raw right-border slot when this bit
/// is cleared. Publisher's writer can also emit the bit on terminal cells whose
/// left border is mirrored to the table edge, so the bit alone does not prove
/// that an explicit raw right-border slot is present.
pub const LEGACY_0X22_CELL_RIGHT_BORDER_GATE_FLAG: u16 = 0x0008;
/// Gates the explicit bottom-border slot in the tested Publisher 2 cell-style record.
///
/// The Microsoft reader ignores a populated raw bottom-border slot when this bit
/// is cleared. Publisher's writer can also emit the bit on terminal cells whose
/// top border is mirrored to the table edge, so the bit alone does not prove
/// that an explicit raw bottom-border slot is present.
pub const LEGACY_0X22_CELL_BOTTOM_BORDER_GATE_FLAG: u16 = 0x0010;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22CellBorder {
    pub width_raw: u8,
    /// Exact width in quarter-points.
    ///
    /// Publisher 2 encodes values with bit 7 set as low-seven-bits / 4 pt;
    /// values without bit 7 are whole points.
    pub width_quarter_points: u16,
    /// Legacy Publisher palette slot. None means the truncated record did not
    /// carry the color byte; the raw payload still preserves that distinction.
    pub legacy_color_index: Option<u8>,
}

impl Legacy0x22CellBorder {
    pub fn line_exists(&self) -> bool {
        self.width_raw != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Legacy0x22CellStyle {
    /// Raw flags. Bits 0x0001/0x0004 carry independently corroborated
    /// horizontal-merge semantics. Bits 0x0008/0x0010 gate the explicit
    /// right/bottom border slots in Microsoft Publisher 2 readback. All
    /// remaining bits stay preservation-only.
    pub flags_raw: u16,
    /// Two legacy palette slots consumed by the historical Publisher parser
    /// when the record is long enough.
    pub legacy_fill_color_indices: Option<[u8; 2]>,
    /// Raw fill/pattern selector. Pattern semantics beyond the observed
    /// simple fills remain preservation-only.
    pub fill_pattern_id: Option<u8>,
    /// Bytes 5..14 in the observed record family. The historical parser skips
    /// this area as margins/unknown state; pub-rs deliberately keeps it raw.
    pub observed_middle: Vec<u8>,
    /// Up to four raw left/top/right/bottom border records beginning at byte 14.
    ///
    /// The right and bottom slots remain raw unless their corresponding gate
    /// bit is set. Terminal-edge mirroring of a left/top border is a separate
    /// table-level behavior and is not synthesized by this decoder.
    pub borders: Vec<Legacy0x22CellBorder>,
    pub raw_payload: Vec<u8>,
}

impl Legacy0x22CellStyle {
    /// Native Publisher 2 + independent regression output corroborate bit 0
    /// as the start marker for a horizontal merged-cell run.
    pub fn starts_horizontal_merge(&self) -> bool {
        (self.flags_raw & LEGACY_0X22_CELL_HORIZONTAL_MERGE_START_FLAG) != 0
    }

    /// Native Publisher 2 + independent regression output corroborate bit 2
    /// as a continuation marker when it follows a merge-start cell in the
    /// same row.
    pub fn continues_horizontal_merge(&self) -> bool {
        (self.flags_raw & LEGACY_0X22_CELL_HORIZONTAL_MERGE_CONTINUATION_FLAG) != 0
    }

    /// Microsoft Publisher 2 uses bit 3 to gate the explicit raw right-border
    /// slot. The writer may also leave this bit set for terminal-edge
    /// normalization even when the raw right slot is absent.
    pub fn right_border_slot_enabled(&self) -> bool {
        (self.flags_raw & LEGACY_0X22_CELL_RIGHT_BORDER_GATE_FLAG) != 0
    }

    /// Microsoft Publisher 2 uses bit 4 to gate the explicit raw bottom-border
    /// slot. The writer may also leave this bit set for terminal-edge
    /// normalization even when the raw bottom slot is absent.
    pub fn bottom_border_slot_enabled(&self) -> bool {
        (self.flags_raw & LEGACY_0X22_CELL_BOTTOM_BORDER_GATE_FLAG) != 0
    }

    /// Return the explicit right-border record only when Microsoft would treat
    /// that raw slot as active in the tested Publisher 2 format.
    pub fn active_explicit_right_border(&self) -> Option<&Legacy0x22CellBorder> {
        self.right_border_slot_enabled()
            .then(|| self.borders.get(2))
            .flatten()
            .filter(|border| border.line_exists())
    }

    /// Return the explicit bottom-border record only when Microsoft would treat
    /// that raw slot as active in the tested Publisher 2 format.
    pub fn active_explicit_bottom_border(&self) -> Option<&Legacy0x22CellBorder> {
        self.bottom_border_slot_enabled()
            .then(|| self.borders.get(3))
            .flatten()
            .filter(|border| border.line_exists())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Legacy0x22CellStyleError {
    TooShort { len: usize },
}

impl fmt::Display for Legacy0x22CellStyleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { len } => write!(
                f,
                "legacy 0x22 cell-style payload needs at least 2 bytes for flags, found {len}"
            ),
        }
    }
}

impl std::error::Error for Legacy0x22CellStyleError {}

fn width_quarter_points(raw: u8) -> u16 {
    if (raw & 0x80) != 0 {
        u16::from(raw & 0x7f)
    } else {
        u16::from(raw) * 4
    }
}

/// Decode the bounded Publisher 2 cell-style record carried by the third
/// legacy 0x22 FKP lane.
///
/// This lane is independently distinguished from CHPX/PAPX by:
/// - native Publisher 2 descriptor/page framing;
/// - the forgotten fosnola/libmspub low-family parser, which maps descriptor
///   index[2]..index[3] to cell styles;
/// - byte-pinned Microsoft-writer controls where the lane stays invariant
///   while ordinary character/paragraph properties vary.
///
/// The decoder promotes only fields used by that historical Publisher 2
/// branch and directly corroborated by native table output. Unknown middle
/// bytes and the complete payload remain preserved.
pub fn decode_legacy_0x22_cell_style(
    payload: &[u8],
) -> Result<Legacy0x22CellStyle, Legacy0x22CellStyleError> {
    if payload.len() < 2 {
        return Err(Legacy0x22CellStyleError::TooShort { len: payload.len() });
    }

    let flags_raw = u16::from_le_bytes([payload[0], payload[1]]);
    let legacy_fill_color_indices = payload.get(2..4).map(|raw| [raw[0], raw[1]]);
    let fill_pattern_id = payload.get(4).copied();

    let middle_start = 5.min(payload.len());
    let middle_end = CELL_STYLE_BORDER_OFFSET.min(payload.len());
    let observed_middle = payload[middle_start..middle_end].to_vec();

    let mut borders = Vec::new();
    let mut offset = CELL_STYLE_BORDER_OFFSET;
    while offset < payload.len() && borders.len() < CELL_STYLE_MAX_BORDERS {
        let width_raw = payload[offset];
        let legacy_color_index = payload.get(offset + 1).copied();
        borders.push(Legacy0x22CellBorder {
            width_raw,
            width_quarter_points: width_quarter_points(width_raw),
            legacy_color_index,
        });
        offset += if legacy_color_index.is_some() { 2 } else { 1 };
    }

    Ok(Legacy0x22CellStyle {
        flags_raw,
        legacy_fill_color_indices,
        fill_pattern_id,
        observed_middle,
        borders,
        raw_payload: payload.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_publisher2_default_cell_record() {
        let style = decode_legacy_0x22_cell_style(&[
            0x00, 0x00, 0x00, 0x01, 0x01, 0x5a, 0x00, 0x5a, 0x00, 0x5a, 0x00, 0x5a,
        ])
        .unwrap();

        assert_eq!(style.flags_raw, 0);
        assert_eq!(style.legacy_fill_color_indices, Some([0, 1]));
        assert_eq!(style.fill_pattern_id, Some(1));
        assert_eq!(style.observed_middle, [0x5a, 0, 0x5a, 0, 0x5a, 0, 0x5a]);
        assert!(style.borders.is_empty());
    }

    #[test]
    fn exposes_grounded_merge_and_border_gate_flag_bits() {
        let start = decode_legacy_0x22_cell_style(&[0x01, 0x00]).unwrap();
        assert!(start.starts_horizontal_merge());
        assert!(!start.continues_horizontal_merge());

        let continuation = decode_legacy_0x22_cell_style(&[0x04, 0x00]).unwrap();
        assert!(!continuation.starts_horizontal_merge());
        assert!(continuation.continues_horizontal_merge());

        let right = decode_legacy_0x22_cell_style(&[0x08, 0x00]).unwrap();
        assert!(right.right_border_slot_enabled());
        assert!(!right.bottom_border_slot_enabled());

        let bottom = decode_legacy_0x22_cell_style(&[0x10, 0x00]).unwrap();
        assert!(!bottom.right_border_slot_enabled());
        assert!(bottom.bottom_border_slot_enabled());
        assert!(!bottom.starts_horizontal_merge());
        assert!(!bottom.continues_horizontal_merge());
    }

    #[test]
    fn border_gate_bits_control_explicit_trailing_slots() {
        let right_payload = [
            0x08, 0x00, 0x00, 0x01, 0x01, 0x5a, 0x00, 0x5a, 0x00, 0x5a, 0x00, 0x5a, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x81, 0x00,
        ];
        let right = decode_legacy_0x22_cell_style(&right_payload).unwrap();
        assert_eq!(
            right
                .active_explicit_right_border()
                .map(|border| border.width_quarter_points),
            Some(1)
        );

        let mut ungated_right_payload = right_payload;
        ungated_right_payload[0] = 0;
        let ungated_right = decode_legacy_0x22_cell_style(&ungated_right_payload).unwrap();
        assert_eq!(ungated_right.borders[2].width_quarter_points, 1);
        assert!(ungated_right.active_explicit_right_border().is_none());

        let bottom_payload = [
            0x10, 0x00, 0x00, 0x01, 0x01, 0x5a, 0x00, 0x5a, 0x00, 0x5a, 0x00, 0x5a, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x81, 0x00,
        ];
        let bottom = decode_legacy_0x22_cell_style(&bottom_payload).unwrap();
        assert_eq!(
            bottom
                .active_explicit_bottom_border()
                .map(|border| border.width_quarter_points),
            Some(1)
        );

        let mut ungated_bottom_payload = bottom_payload;
        ungated_bottom_payload[0] = 0;
        let ungated_bottom = decode_legacy_0x22_cell_style(&ungated_bottom_payload).unwrap();
        assert_eq!(ungated_bottom.borders[3].width_quarter_points, 1);
        assert!(ungated_bottom.active_explicit_bottom_border().is_none());
    }

    #[test]
    fn decodes_publisher2_border_widths_and_palette_slots() {
        let style = decode_legacy_0x22_cell_style(&[
            0x00, 0x00, 0x00, 0x01, 0x01, 0x5a, 0x00, 0x5a, 0x00, 0x5a, 0x00, 0x5a, 0x00, 0x00,
            0x81, 0x40, 0x02, 0x00, 0x81, 0x00,
        ])
        .unwrap();

        assert_eq!(style.borders.len(), 3);
        assert_eq!(style.borders[0].width_quarter_points, 1);
        assert_eq!(style.borders[0].legacy_color_index, Some(0x40));
        assert_eq!(style.borders[1].width_quarter_points, 8);
        assert_eq!(style.borders[1].legacy_color_index, Some(0));
        assert_eq!(style.borders[2].width_quarter_points, 1);
        assert_eq!(style.borders[2].legacy_color_index, Some(0));
    }

    #[test]
    fn keeps_short_native_cell_records_bounded() {
        let style = decode_legacy_0x22_cell_style(&[0x00, 0x00, 0x01]).unwrap();

        assert_eq!(style.flags_raw, 0);
        assert_eq!(style.legacy_fill_color_indices, None);
        assert_eq!(style.fill_pattern_id, None);
        assert!(style.observed_middle.is_empty());
        assert!(style.borders.is_empty());
    }

    #[test]
    fn rejects_payload_without_complete_flags() {
        assert_eq!(
            decode_legacy_0x22_cell_style(&[0]),
            Err(Legacy0x22CellStyleError::TooShort { len: 1 })
        );
    }
}
