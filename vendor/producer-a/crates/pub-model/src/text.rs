use serde::{Deserialize, Serialize};
use std::fmt;

/// Half-open range [start, end) в Unicode scalar indices.
///
/// Source-format UTF-16 indices, byte offsets и glyph indices должны храниться
/// отдельно в provenance/mapping и не подменяют canonical scalar index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TextRange {
    pub start: u64,
    pub end: u64,
}

impl TextRange {
    pub fn new(start: u64, end: u64) -> Result<Self, TextRangeError> {
        if start > end {
            return Err(TextRangeError::StartAfterEnd { start, end });
        }

        Ok(Self { start, end })
    }

    pub const fn len(self) -> u64 {
        self.end - self.start
    }

    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub const fn contains_scalar_index(self, index: u64) -> bool {
        self.start <= index && index < self.end
    }

    pub fn fits_text(self, text: &str) -> bool {
        let Ok(scalar_len) = u64::try_from(text.chars().count()) else {
            return false;
        };
        self.end <= scalar_len
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRangeError {
    StartAfterEnd { start: u64, end: u64 },
}

impl fmt::Display for TextRangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StartAfterEnd { start, end } => {
                write!(
                    formatter,
                    "TextRange start {start} не может быть больше end {end}"
                )
            }
        }
    }
}

impl std::error::Error for TextRangeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_range_is_half_open() {
        let range = TextRange::new(2, 5).expect("валидный range");

        assert_eq!(range.len(), 3);
        assert!(range.contains_scalar_index(2));
        assert!(range.contains_scalar_index(4));
        assert!(!range.contains_scalar_index(5));
    }

    #[test]
    fn text_range_counts_unicode_scalars_not_utf16_units_or_bytes() {
        let text = "A😀Б";
        let whole = TextRange::new(0, 3).expect("три Unicode scalar");
        let too_long = TextRange::new(0, 4).expect("структурно валидный range");

        assert!(whole.fits_text(text));
        assert!(!too_long.fits_text(text));
    }

    #[test]
    fn text_range_rejects_reversed_bounds() {
        assert_eq!(
            TextRange::new(5, 4),
            Err(TextRangeError::StartAfterEnd { start: 5, end: 4 })
        );
    }
}
