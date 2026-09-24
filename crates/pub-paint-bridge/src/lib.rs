//! Public-safe bounded bridge from the already-proven VIEWER-FILL-LINE-01
//! observation shape into canonical CDM Shape paint.
//!
//! Source-specific observations stop here. Downstream Viewer/layout/export
//! consumers receive canonical pub-model paint, never a competing paint truth.

use pub_model::{
    ShapePaintProvenanceV1, ShapePaintV1, ShapePaintValidationError, SolidFillV1, SolidStrokeV1,
    SourceRefV1, Srgb8, validate_shape_paint_v1,
};
use serde::{Deserialize, Serialize};

pub const MAX_BOUNDED_SOURCE_LINE_WIDTH_EMU_V1: i64 = 0x0132_F540;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PubExplicitShapePaintSourceV1 {
    pub fill: PubExplicitFillSourceV1,
    pub line: PubExplicitLineSourceV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PubExplicitFillSourceV1 {
    pub solid: bool,
    pub color_rgb: Option<[u8; 3]>,
    pub visible: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PubExplicitLineSourceV1 {
    pub color_rgb: Option<[u8; 3]>,
    pub width_emu: Option<i64>,
    pub visible: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerNodePaintV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solid_fill_rgb: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solid_line: Option<ViewerSolidLineV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerSolidLineV1 {
    pub rgb: [u8; 3],
    pub width_emu: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutShapePaintV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<SolidFillV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<SolidStrokeV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditableExportShapePaintV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<SolidFillV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<SolidStrokeV1>,
}

pub fn promote_explicit_source_paint_v1(
    source: &PubExplicitShapePaintSourceV1,
    source_ref: SourceRefV1,
) -> Result<Option<ShapePaintV1>, ShapePaintValidationError> {
    let fill = match (
        source.fill.solid,
        source.fill.color_rgb,
        source.fill.visible,
    ) {
        (true, Some(rgb), Some(visible)) => Some(SolidFillV1 {
            visible,
            color: Srgb8::from(rgb),
        }),
        _ => None,
    };

    let stroke = match (
        source.line.color_rgb,
        source.line.width_emu,
        source.line.visible,
    ) {
        (Some(rgb), Some(width_emu), Some(visible))
            if width_emu > 0 && width_emu <= MAX_BOUNDED_SOURCE_LINE_WIDTH_EMU_V1 =>
        {
            Some(SolidStrokeV1 {
                visible,
                color: Srgb8::from(rgb),
                width_emu,
            })
        }
        _ => None,
    };

    if fill.is_none() && stroke.is_none() {
        return Ok(None);
    }

    let paint = ShapePaintV1 {
        fill,
        stroke,
        provenance: ShapePaintProvenanceV1::SourceBacked { source_ref },
    };
    validate_shape_paint_v1(&paint)?;
    Ok(Some(paint))
}

pub fn project_viewer_node_paint_v1(paint: &ShapePaintV1) -> Option<ViewerNodePaintV1> {
    let solid_fill_rgb = paint
        .fill
        .as_ref()
        .filter(|fill| fill.visible)
        .map(|fill| fill.color.into());

    let solid_line = paint
        .stroke
        .as_ref()
        .filter(|stroke| stroke.visible)
        .map(|stroke| ViewerSolidLineV1 {
            rgb: stroke.color.into(),
            width_emu: stroke.width_emu,
        });

    if solid_fill_rgb.is_none() && solid_line.is_none() {
        None
    } else {
        Some(ViewerNodePaintV1 {
            solid_fill_rgb,
            solid_line,
        })
    }
}

pub fn project_layout_shape_paint_v1(paint: &ShapePaintV1) -> LayoutShapePaintV1 {
    LayoutShapePaintV1 {
        fill: paint.fill.clone(),
        stroke: paint.stroke.clone(),
    }
}

pub fn project_editable_export_shape_paint_v1(paint: &ShapePaintV1) -> EditableExportShapePaintV1 {
    EditableExportShapePaintV1 {
        fill: paint.fill.clone(),
        stroke: paint.stroke.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pub_model::{
        AuthorityClassV1, ReadConfidenceV1, ShapePaintProvenanceV1, SourceRoleV1,
        author_created_shape_paint_v1,
    };

    fn source_ref() -> SourceRefV1 {
        SourceRefV1 {
            format: "pub".to_owned(),
            adapter_version: "pub-rs/0.1".to_owned(),
            source_hash_hex: "ab".repeat(32),
            carrier: "/Escher/EscherStm".to_owned(),
            object_key: Some("escher/client-data-shape-id/7".to_owned()),
            path: Some("SpContainer/FOPT".to_owned()),
            role: SourceRoleV1::Projection,
            authority: AuthorityClassV1::Authoritative,
            confidence: ReadConfidenceV1::Exact,
        }
    }

    fn legacy_viewer_projection(
        source: &PubExplicitShapePaintSourceV1,
    ) -> Option<ViewerNodePaintV1> {
        let solid_fill_rgb = (source.fill.solid && source.fill.visible == Some(true))
            .then_some(source.fill.color_rgb)
            .flatten();

        let solid_line = match (
            source.line.visible,
            source.line.color_rgb,
            source.line.width_emu,
        ) {
            (Some(true), Some(rgb), Some(width_emu)) if width_emu > 0 => {
                Some(ViewerSolidLineV1 { rgb, width_emu })
            }
            _ => None,
        };

        if solid_fill_rgb.is_none() && solid_line.is_none() {
            None
        } else {
            Some(ViewerNodePaintV1 {
                solid_fill_rgb,
                solid_line,
            })
        }
    }

    #[test]
    fn complete_bounded_source_paint_promotes_and_preserves_viewer_parity() {
        let source = PubExplicitShapePaintSourceV1 {
            fill: PubExplicitFillSourceV1 {
                solid: true,
                color_rgb: Some([0x11, 0x22, 0x33]),
                visible: Some(true),
            },
            line: PubExplicitLineSourceV1 {
                color_rgb: Some([0x44, 0x55, 0x66]),
                width_emu: Some(12_700),
                visible: Some(true),
            },
        };

        let canonical = promote_explicit_source_paint_v1(&source, source_ref())
            .expect("promotion")
            .expect("bounded paint");

        assert!(matches!(
            canonical.provenance,
            ShapePaintProvenanceV1::SourceBacked { .. }
        ));
        assert_eq!(
            project_viewer_node_paint_v1(&canonical),
            legacy_viewer_projection(&source)
        );
    }

    #[test]
    fn unresolved_or_omitted_defaults_are_not_materialized() {
        let source = PubExplicitShapePaintSourceV1 {
            fill: PubExplicitFillSourceV1 {
                solid: true,
                color_rgb: Some([1, 2, 3]),
                visible: None,
            },
            line: PubExplicitLineSourceV1 {
                color_rgb: Some([4, 5, 6]),
                width_emu: Some(12_700),
                visible: None,
            },
        };
        assert_eq!(
            promote_explicit_source_paint_v1(&source, source_ref()).expect("promotion"),
            None
        );

        let non_solid = PubExplicitShapePaintSourceV1 {
            fill: PubExplicitFillSourceV1 {
                solid: false,
                color_rgb: Some([1, 2, 3]),
                visible: Some(true),
            },
            ..Default::default()
        };
        assert_eq!(
            promote_explicit_source_paint_v1(&non_solid, source_ref()).expect("promotion"),
            None
        );
    }

    #[test]
    fn partial_complete_components_promote_independently_without_defaults() {
        let source = PubExplicitShapePaintSourceV1 {
            fill: PubExplicitFillSourceV1 {
                solid: true,
                color_rgb: Some([1, 2, 3]),
                visible: Some(false),
            },
            line: PubExplicitLineSourceV1 {
                color_rgb: Some([4, 5, 6]),
                width_emu: None,
                visible: Some(true),
            },
        };
        let paint = promote_explicit_source_paint_v1(&source, source_ref())
            .expect("promotion")
            .expect("fill");

        assert_eq!(paint.fill.as_ref().expect("fill").visible, false);
        assert!(paint.stroke.is_none());
        assert_eq!(project_viewer_node_paint_v1(&paint), None);
        assert_eq!(
            project_viewer_node_paint_v1(&paint),
            legacy_viewer_projection(&source)
        );
    }

    #[test]
    fn source_stroke_keeps_existing_bounded_width_fence() {
        let source = PubExplicitShapePaintSourceV1 {
            fill: PubExplicitFillSourceV1::default(),
            line: PubExplicitLineSourceV1 {
                color_rgb: Some([4, 5, 6]),
                width_emu: Some(MAX_BOUNDED_SOURCE_LINE_WIDTH_EMU_V1 + 1),
                visible: Some(true),
            },
        };
        assert_eq!(
            promote_explicit_source_paint_v1(&source, source_ref()).expect("promotion"),
            None
        );
    }

    #[test]
    fn author_created_and_source_backed_values_share_one_canonical_type() {
        let authored = author_created_shape_paint_v1(
            Some(SolidFillV1 {
                visible: true,
                color: Srgb8 { r: 7, g: 8, b: 9 },
            }),
            Some(SolidStrokeV1 {
                visible: true,
                color: Srgb8 {
                    r: 10,
                    g: 11,
                    b: 12,
                },
                width_emu: 25_400,
            }),
        )
        .expect("authored paint");

        assert_eq!(
            project_layout_shape_paint_v1(&authored),
            LayoutShapePaintV1 {
                fill: authored.fill.clone(),
                stroke: authored.stroke.clone(),
            }
        );
        assert_eq!(
            project_editable_export_shape_paint_v1(&authored),
            EditableExportShapePaintV1 {
                fill: authored.fill.clone(),
                stroke: authored.stroke.clone(),
            }
        );
        assert_eq!(authored.provenance, ShapePaintProvenanceV1::AuthorCreated);
    }

    #[test]
    fn hidden_canonical_values_remain_explicit_for_layout_and_export_but_not_viewer_paint() {
        let paint = author_created_shape_paint_v1(
            Some(SolidFillV1 {
                visible: false,
                color: Srgb8 { r: 1, g: 2, b: 3 },
            }),
            Some(SolidStrokeV1 {
                visible: false,
                color: Srgb8 { r: 4, g: 5, b: 6 },
                width_emu: 9_525,
            }),
        )
        .expect("authored paint");

        assert!(project_viewer_node_paint_v1(&paint).is_none());
        assert_eq!(project_layout_shape_paint_v1(&paint).fill, paint.fill);
        assert_eq!(
            project_editable_export_shape_paint_v1(&paint).stroke,
            paint.stroke
        );
    }
}
