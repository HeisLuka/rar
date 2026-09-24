use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Srgb8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl From<[u8; 3]> for Srgb8 {
    fn from(value: [u8; 3]) -> Self {
        Self {
            r: value[0],
            g: value[1],
            b: value[2],
        }
    }
}

impl From<Srgb8> for [u8; 3] {
    fn from(value: Srgb8) -> Self {
        [value.r, value.g, value.b]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolidFillV1 {
    pub visible: bool,
    pub color: Srgb8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolidStrokeV1 {
    pub visible: bool,
    pub color: Srgb8,
    pub width_emu: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceRoleV1 {
    Semantic,
    Projection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityClassV1 {
    Authoritative,
    Derived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadConfidenceV1 {
    Exact,
    Structural,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRefV1 {
    pub format: String,
    pub adapter_version: String,
    pub source_hash_hex: String,
    pub carrier: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub role: SourceRoleV1,
    pub authority: AuthorityClassV1,
    pub confidence: ReadConfidenceV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShapePaintProvenanceV1 {
    SourceBacked { source_ref: SourceRefV1 },
    AuthorCreated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapePaintV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<SolidFillV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<SolidStrokeV1>,
    pub provenance: ShapePaintProvenanceV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShapePaintValidationError {
    EmptyPaint,
    NonPositiveStrokeWidth { width_emu: i64 },
    EmptySourceField { field: &'static str },
    InvalidSourceHash,
    NonAuthoritativeSource,
    NonExactSourceConfidence,
}

pub fn validate_shape_paint_v1(
    paint: &ShapePaintV1,
) -> Result<(), ShapePaintValidationError> {
    if paint.fill.is_none() && paint.stroke.is_none() {
        return Err(ShapePaintValidationError::EmptyPaint);
    }

    if let Some(stroke) = &paint.stroke
        && stroke.width_emu <= 0
    {
        return Err(ShapePaintValidationError::NonPositiveStrokeWidth {
            width_emu: stroke.width_emu,
        });
    }

    if let ShapePaintProvenanceV1::SourceBacked { source_ref } = &paint.provenance {
        validate_source_ref(source_ref)?;
    }

    Ok(())
}

pub fn author_created_shape_paint_v1(
    fill: Option<SolidFillV1>,
    stroke: Option<SolidStrokeV1>,
) -> Result<ShapePaintV1, ShapePaintValidationError> {
    let paint = ShapePaintV1 {
        fill,
        stroke,
        provenance: ShapePaintProvenanceV1::AuthorCreated,
    };
    validate_shape_paint_v1(&paint)?;
    Ok(paint)
}

pub fn canonical_shape_paint_hash_v1(
    paint: &ShapePaintV1,
) -> Result<String, serde_json::Error> {
    let bytes = serde_json::to_vec(paint)?;
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut hex, "{byte:02x}").expect("write to String cannot fail");
    }
    Ok(format!("sha256:{hex}"))
}

fn validate_source_ref(source_ref: &SourceRefV1) -> Result<(), ShapePaintValidationError> {
    for (field, value) in [
        ("format", source_ref.format.as_str()),
        ("adapter_version", source_ref.adapter_version.as_str()),
        ("carrier", source_ref.carrier.as_str()),
    ] {
        if value.is_empty() {
            return Err(ShapePaintValidationError::EmptySourceField { field });
        }
    }

    if source_ref
        .object_key
        .as_ref()
        .is_some_and(|value| value.is_empty())
    {
        return Err(ShapePaintValidationError::EmptySourceField {
            field: "object_key",
        });
    }
    if source_ref.path.as_ref().is_some_and(|value| value.is_empty()) {
        return Err(ShapePaintValidationError::EmptySourceField { field: "path" });
    }

    if source_ref.source_hash_hex.len() != 64
        || !source_ref
            .source_hash_hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ShapePaintValidationError::InvalidSourceHash);
    }

    if source_ref.authority != AuthorityClassV1::Authoritative {
        return Err(ShapePaintValidationError::NonAuthoritativeSource);
    }
    if source_ref.confidence != ReadConfidenceV1::Exact {
        return Err(ShapePaintValidationError::NonExactSourceConfidence);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_ref() -> SourceRefV1 {
        SourceRefV1 {
            format: "pub".to_owned(),
            adapter_version: "pub-rs/0.1".to_owned(),
            source_hash_hex: "11".repeat(32),
            carrier: "/Escher/EscherStm".to_owned(),
            object_key: Some("escher/client-data-shape-id/42".to_owned()),
            path: Some("SpContainer/FOPT".to_owned()),
            role: SourceRoleV1::Semantic,
            authority: AuthorityClassV1::Authoritative,
            confidence: ReadConfidenceV1::Exact,
        }
    }

    #[test]
    fn author_created_paint_has_no_source_ref_by_construction() {
        let paint = author_created_shape_paint_v1(
            Some(SolidFillV1 {
                visible: true,
                color: Srgb8 { r: 1, g: 2, b: 3 },
            }),
            None,
        )
        .expect("valid author-created paint");

        assert_eq!(paint.provenance, ShapePaintProvenanceV1::AuthorCreated);
        let json = serde_json::to_string(&paint).expect("serialize");
        assert!(!json.contains("source_ref"));
    }

    #[test]
    fn source_backed_paint_requires_authoritative_exact_source_ref() {
        let paint = ShapePaintV1 {
            fill: Some(SolidFillV1 {
                visible: true,
                color: Srgb8 { r: 4, g: 5, b: 6 },
            }),
            stroke: None,
            provenance: ShapePaintProvenanceV1::SourceBacked {
                source_ref: source_ref(),
            },
        };
        assert_eq!(validate_shape_paint_v1(&paint), Ok(()));

        let mut bad = paint.clone();
        if let ShapePaintProvenanceV1::SourceBacked { source_ref } = &mut bad.provenance {
            source_ref.authority = AuthorityClassV1::Derived;
        }
        assert_eq!(
            validate_shape_paint_v1(&bad),
            Err(ShapePaintValidationError::NonAuthoritativeSource)
        );
    }

    #[test]
    fn stroke_width_must_be_positive() {
        let paint = author_created_shape_paint_v1(
            None,
            Some(SolidStrokeV1 {
                visible: true,
                color: Srgb8 { r: 0, g: 0, b: 0 },
                width_emu: 0,
            }),
        );
        assert_eq!(
            paint,
            Err(ShapePaintValidationError::NonPositiveStrokeWidth { width_emu: 0 })
        );
    }

    #[test]
    fn canonical_hash_is_stable_and_value_sensitive() {
        let first = author_created_shape_paint_v1(
            Some(SolidFillV1 {
                visible: true,
                color: Srgb8 {
                    r: 0x11,
                    g: 0x22,
                    b: 0x33,
                },
            }),
            Some(SolidStrokeV1 {
                visible: true,
                color: Srgb8 {
                    r: 0x44,
                    g: 0x55,
                    b: 0x66,
                },
                width_emu: 12_700,
            }),
        )
        .expect("paint");
        let round_trip: ShapePaintV1 = serde_json::from_slice(
            &serde_json::to_vec(&first).expect("serialize"),
        )
        .expect("deserialize");

        assert_eq!(first, round_trip);
        assert_eq!(
            canonical_shape_paint_hash_v1(&first).expect("hash"),
            canonical_shape_paint_hash_v1(&round_trip).expect("hash")
        );

        let mut changed = first.clone();
        changed.fill.as_mut().expect("fill").color.r = 0x12;
        assert_ne!(
            canonical_shape_paint_hash_v1(&first).expect("hash"),
            canonical_shape_paint_hash_v1(&changed).expect("hash")
        );
    }
}
