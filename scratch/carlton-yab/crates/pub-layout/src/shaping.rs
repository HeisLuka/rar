use crate::BoundedLayoutEnvironment;
use harfrust::{Direction, FontRef, ShapeOptions, ShaperData, UnicodeBuffer};
use pub_model::LengthEmu;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

pub const BOUNDED_SHAPER_REVISION: &str = "harfrust-0.13.3";

#[derive(Debug, Clone)]
pub struct BoundedShapingRuntime<'a> {
    pub layout: BoundedLayoutEnvironment,
    pub face_index: u32,
    pub font_size_emu: LengthEmu,
    pub font_bytes: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedShapingDescriptor {
    pub layout: BoundedLayoutEnvironment,
    pub face_index: u32,
    pub font_size_emu: LengthEmu,
    pub shaper_revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedShapedText {
    pub environment: BoundedShapingDescriptor,
    pub units_per_em: u32,
    pub glyphs: Vec<BoundedShapedGlyph>,
    pub total_x_advance: LengthEmu,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedShapedGlyph {
    pub glyph_id: u32,
    /// Zero-based Unicode scalar index in the original logical string.
    pub cluster: u32,
    pub x_advance: LengthEmu,
    pub y_advance: LengthEmu,
    pub x_offset: LengthEmu,
    pub y_offset: LengthEmu,
    pub unsafe_to_break: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundedShapeError {
    NonPositiveFontSize { font_size_emu: i64 },
    FontFingerprintMismatch { expected: String, actual: String },
    InvalidFont { face_index: u32 },
    ScalarIndexOverflow,
    MetricScaleOverflow,
}

impl fmt::Display for BoundedShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonPositiveFontSize { font_size_emu } => {
                write!(f, "font size must be positive, got {font_size_emu} EMU")
            }
            Self::FontFingerprintMismatch { expected, actual } => {
                write!(
                    f,
                    "font fingerprint mismatch: expected {expected}, actual {actual}"
                )
            }
            Self::InvalidFont { face_index } => {
                write!(f, "invalid OpenType font or face index {face_index}")
            }
            Self::ScalarIndexOverflow => {
                write!(f, "text contains more Unicode scalars than u32 can index")
            }
            Self::MetricScaleOverflow => {
                write!(f, "font-unit to EMU scaling overflowed")
            }
        }
    }
}

impl std::error::Error for BoundedShapeError {}

pub fn font_fingerprint_sha256(font_bytes: &[u8]) -> String {
    let digest = Sha256::digest(font_bytes);
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(HEX[usize::from(byte >> 4)] as char);
        out.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    out
}

/// Shapes one LTR logical text run using explicit, fingerprint-fenced font bytes.
///
/// This is LAYOUT-RESOLVE-01B1. It deliberately does not perform paragraph
/// bidi resolution, line breaking, fallback, hyphenation or frame continuation.
/// Cluster IDs are assigned explicitly as Unicode scalar indices before shaping,
/// so the resolved glyph stream can be mapped back to authoring text without
/// depending on UTF-8 byte offsets.
pub fn shape_bounded_ltr(
    text: &str,
    runtime: &BoundedShapingRuntime<'_>,
) -> Result<BoundedShapedText, BoundedShapeError> {
    shape_bounded_ltr_with_cluster_base(text, 0, runtime)
}

/// Shapes an independently composed LTR segment while preserving Story-global
/// Unicode-scalar cluster identities.
///
/// B2C uses this when a chosen line boundary cannot safely split the original
/// full-run shape. The caller supplies the scalar index of the segment start;
/// local HarfRust cluster IDs are rebased with checked arithmetic.
pub fn shape_bounded_ltr_segment(
    text: &str,
    scalar_base: u32,
    runtime: &BoundedShapingRuntime<'_>,
) -> Result<BoundedShapedText, BoundedShapeError> {
    shape_bounded_ltr_with_cluster_base(text, scalar_base, runtime)
}

fn shape_bounded_ltr_with_cluster_base(
    text: &str,
    scalar_base: u32,
    runtime: &BoundedShapingRuntime<'_>,
) -> Result<BoundedShapedText, BoundedShapeError> {
    if runtime.font_size_emu.get() <= 0 {
        return Err(BoundedShapeError::NonPositiveFontSize {
            font_size_emu: runtime.font_size_emu.get(),
        });
    }

    let actual_fingerprint = font_fingerprint_sha256(runtime.font_bytes);
    if actual_fingerprint != runtime.layout.font_set_fingerprint {
        return Err(BoundedShapeError::FontFingerprintMismatch {
            expected: runtime.layout.font_set_fingerprint.clone(),
            actual: actual_fingerprint,
        });
    }

    let font = FontRef::from_index(runtime.font_bytes, runtime.face_index).map_err(|_| {
        BoundedShapeError::InvalidFont {
            face_index: runtime.face_index,
        }
    })?;
    let data = ShaperData::new(&font);
    let shaper = data.shaper(&font).build();
    let units_per_em =
        u32::try_from(shaper.units_per_em()).map_err(|_| BoundedShapeError::MetricScaleOverflow)?;

    let mut buffer = UnicodeBuffer::new();
    buffer.set_direction(Direction::LeftToRight);
    for (index, ch) in text.chars().enumerate() {
        let local_cluster =
            u32::try_from(index).map_err(|_| BoundedShapeError::ScalarIndexOverflow)?;
        let cluster = scalar_base
            .checked_add(local_cluster)
            .ok_or(BoundedShapeError::ScalarIndexOverflow)?;
        buffer.add(ch, cluster);
    }
    buffer.guess_segment_properties();

    let glyph_buffer = shaper.shape(buffer, ShapeOptions::new());
    let infos = glyph_buffer.glyph_infos();
    let positions = glyph_buffer.glyph_positions();

    let mut glyphs = Vec::with_capacity(infos.len());
    let mut total_x_advance = LengthEmu::ZERO;

    for (info, position) in infos.iter().zip(positions) {
        let x_advance = scale_font_units(position.x_advance, runtime.font_size_emu, units_per_em)?;
        let y_advance = scale_font_units(position.y_advance, runtime.font_size_emu, units_per_em)?;
        let x_offset = scale_font_units(position.x_offset, runtime.font_size_emu, units_per_em)?;
        let y_offset = scale_font_units(position.y_offset, runtime.font_size_emu, units_per_em)?;

        total_x_advance = total_x_advance
            .checked_add(x_advance)
            .ok_or(BoundedShapeError::MetricScaleOverflow)?;

        glyphs.push(BoundedShapedGlyph {
            glyph_id: info.glyph_id,
            cluster: info.cluster,
            x_advance,
            y_advance,
            x_offset,
            y_offset,
            unsafe_to_break: info.unsafe_to_break(),
        });
    }

    Ok(BoundedShapedText {
        environment: BoundedShapingDescriptor {
            layout: runtime.layout.clone(),
            face_index: runtime.face_index,
            font_size_emu: runtime.font_size_emu,
            shaper_revision: BOUNDED_SHAPER_REVISION.into(),
        },
        units_per_em,
        glyphs,
        total_x_advance,
    })
}

fn scale_font_units(
    value: i32,
    font_size_emu: LengthEmu,
    units_per_em: u32,
) -> Result<LengthEmu, BoundedShapeError> {
    if units_per_em == 0 {
        return Err(BoundedShapeError::MetricScaleOverflow);
    }

    let numerator = i128::from(value) * i128::from(font_size_emu.get());
    let denominator = i128::from(units_per_em);
    let rounded = if numerator >= 0 {
        (numerator + denominator / 2) / denominator
    } else {
        -((-numerator + denominator / 2) / denominator)
    };

    let emu = i64::try_from(rounded).map_err(|_| BoundedShapeError::MetricScaleOverflow)?;
    Ok(LengthEmu::new(emu))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pub_model::EMU_PER_POINT;

    fn runtime(font_bytes: &[u8]) -> BoundedShapingRuntime<'_> {
        BoundedShapingRuntime {
            layout: BoundedLayoutEnvironment {
                engine_revision: "layout-resolve-01b1".into(),
                font_set_fingerprint: font_fingerprint_sha256(font_bytes),
                resource_fingerprint: "resources:none".into(),
            },
            face_index: 0,
            font_size_emu: LengthEmu::new(12 * EMU_PER_POINT),
            font_bytes,
        }
    }

    #[test]
    fn real_opentype_shaping_is_deterministic() {
        let font = font_test_data::NOTOSERIF_AUTOHINT_SHAPING;
        let runtime = runtime(font);

        let left = shape_bounded_ltr("Hfix", &runtime).expect("valid pinned font");
        let right = shape_bounded_ltr("Hfix", &runtime).expect("valid pinned font");

        assert_eq!(left, right);
        assert_eq!(left.environment.shaper_revision, BOUNDED_SHAPER_REVISION);
        assert!(left.total_x_advance.get() > 0);
        assert!(left.glyphs.iter().all(|glyph| glyph.x_advance.get() >= 0));
    }

    #[test]
    fn fi_ligature_preserves_source_cluster_mapping() {
        let font = font_test_data::NOTOSERIF_AUTOHINT_SHAPING;
        let shaped = shape_bounded_ltr("fix", &runtime(font)).expect("valid pinned font");

        assert!(
            shaped.glyphs.len() < "fix".chars().count(),
            "fixture must exercise a real substitution rather than 1:1 cmap lookup"
        );
        assert_eq!(shaped.glyphs[0].cluster, 0);
        assert!(
            shaped
                .glyphs
                .iter()
                .all(|glyph| usize::try_from(glyph.cluster).unwrap() < "fix".chars().count())
        );
    }

    #[test]
    fn segment_reshaping_rebases_clusters_to_story_space() {
        let font = font_test_data::NOTOSERIF_AUTOHINT_SHAPING;
        let shaped =
            shape_bounded_ltr_segment("fix", 9, &runtime(font)).expect("valid pinned font");

        assert!(shaped.glyphs.len() < "fix".chars().count());
        assert_eq!(shaped.glyphs[0].cluster, 9);
        assert!(shaped.glyphs.iter().all(|glyph| glyph.cluster >= 9));
    }

    #[test]
    fn fingerprint_mismatch_fails_closed() {
        let font = font_test_data::NOTOSERIF_AUTOHINT_SHAPING;
        let mut runtime = runtime(font);
        runtime.layout.font_set_fingerprint = "00".repeat(32);

        let error = shape_bounded_ltr("text", &runtime).expect_err("mismatch must fail");

        assert!(matches!(
            error,
            BoundedShapeError::FontFingerprintMismatch { .. }
        ));
    }

    #[test]
    fn invalid_font_fails_closed() {
        let bytes = b"not a font";
        let runtime = runtime(bytes);

        assert_eq!(
            shape_bounded_ltr("text", &runtime),
            Err(BoundedShapeError::InvalidFont { face_index: 0 })
        );
    }

    #[test]
    fn font_unit_scaling_is_symmetric_and_checked() {
        let size = LengthEmu::new(1_000);

        assert_eq!(
            scale_font_units(500, size, 1_000).unwrap(),
            LengthEmu::new(500)
        );
        assert_eq!(
            scale_font_units(-500, size, 1_000).unwrap(),
            LengthEmu::new(-500)
        );
        assert_eq!(scale_font_units(1, size, 3).unwrap(), LengthEmu::new(333));
        assert_eq!(scale_font_units(-1, size, 3).unwrap(), LengthEmu::new(-333));
    }
}
