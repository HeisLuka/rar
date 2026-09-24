use serde::{Deserialize, Serialize};

use crate::{
    AuthoredShapeV1, CreateShapeError, CreateShapeV1, RectEmuV1, ShapeKindV1, ShapeTransformV1,
    SolidFillV1, SolidStrokeV1, author_created_shape_paint_v1, create_shape_entity_v1,
    validate_rect_emu_v1, validate_uuid_v7_v1,
};

pub const AUTHORING_FRAGMENT_SCHEMA_V1: &str = "chaptera.authoring-fragment.v1";
pub const SINGLE_RECTANGLE_ENTITY_ID_V1: &str = "entity:0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentSourceProvenanceV1 {
    pub source_node_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RectangleFragmentEntityV1 {
    pub fragment_entity_id: String,
    pub bounds: RectEmuV1,
    pub fill: SolidFillV1,
    pub stroke: SolidStrokeV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_provenance: Option<FragmentSourceProvenanceV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoringFragmentV1 {
    pub schema_version: String,
    pub rectangle: RectangleFragmentEntityV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasteIdentityRemapV1 {
    pub fragment_entity_id: String,
    pub destination_node_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranslationEmuV1 {
    pub dx_emu: i64,
    pub dy_emu: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasteFragmentV1 {
    pub fragment: AuthoringFragmentV1,
    pub identity_map: PasteIdentityRemapV1,
    pub destination_page_id: String,
    pub placement: TranslationEmuV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasteFragmentResultV1 {
    pub identity_map: PasteIdentityRemapV1,
    pub entity: AuthoredShapeV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthoringFragmentError {
    UnsupportedShapeKind,
    UnsupportedTransform,
    MissingExplicitFill,
    MissingExplicitStroke,
    PaintMustBeAuthorCreated,
    InvalidFragmentSchema,
    InvalidFragmentEntityId,
    IdentityMapMismatch,
    EmptyDestinationPageId,
    UnsafeTranslation,
    InvalidDestinationNodeId,
    DestinationReusesSourceIdentity,
    InvalidMaterializedShape(CreateShapeError),
}

pub fn capture_rectangle_fragment_v1(
    shape: &AuthoredShapeV1,
) -> Result<AuthoringFragmentV1, AuthoringFragmentError> {
    if shape.shape_kind != ShapeKindV1::Rectangle {
        return Err(AuthoringFragmentError::UnsupportedShapeKind);
    }
    if shape.transform != ShapeTransformV1::Identity {
        return Err(AuthoringFragmentError::UnsupportedTransform);
    }
    validate_rect_emu_v1(shape.bounds).map_err(AuthoringFragmentError::InvalidMaterializedShape)?;

    if !matches!(
        shape.paint.provenance,
        crate::ShapePaintProvenanceV1::AuthorCreated
    ) {
        return Err(AuthoringFragmentError::PaintMustBeAuthorCreated);
    }

    let fill = shape
        .paint
        .fill
        .clone()
        .ok_or(AuthoringFragmentError::MissingExplicitFill)?;
    let stroke = shape
        .paint
        .stroke
        .clone()
        .ok_or(AuthoringFragmentError::MissingExplicitStroke)?;

    Ok(AuthoringFragmentV1 {
        schema_version: AUTHORING_FRAGMENT_SCHEMA_V1.to_owned(),
        rectangle: RectangleFragmentEntityV1 {
            fragment_entity_id: SINGLE_RECTANGLE_ENTITY_ID_V1.to_owned(),
            bounds: shape.bounds,
            fill,
            stroke,
            source_provenance: Some(FragmentSourceProvenanceV1 {
                source_node_id: shape.node_id.clone(),
            }),
        },
    })
}

pub fn materialize_paste_fragment_v1(
    paste: &PasteFragmentV1,
) -> Result<PasteFragmentResultV1, AuthoringFragmentError> {
    if paste.fragment.schema_version != AUTHORING_FRAGMENT_SCHEMA_V1 {
        return Err(AuthoringFragmentError::InvalidFragmentSchema);
    }
    if paste.fragment.rectangle.fragment_entity_id != SINGLE_RECTANGLE_ENTITY_ID_V1 {
        return Err(AuthoringFragmentError::InvalidFragmentEntityId);
    }
    if paste.identity_map.fragment_entity_id != paste.fragment.rectangle.fragment_entity_id {
        return Err(AuthoringFragmentError::IdentityMapMismatch);
    }
    if paste.destination_page_id.is_empty() {
        return Err(AuthoringFragmentError::EmptyDestinationPageId);
    }
    validate_uuid_v7_v1(&paste.identity_map.destination_node_id)
        .map_err(|_| AuthoringFragmentError::InvalidDestinationNodeId)?;
    if paste
        .fragment
        .rectangle
        .source_provenance
        .as_ref()
        .is_some_and(|source| source.source_node_id == paste.identity_map.destination_node_id)
    {
        return Err(AuthoringFragmentError::DestinationReusesSourceIdentity);
    }

    let bounds = translated_bounds_v1(paste.fragment.rectangle.bounds, paste.placement)?;
    let paint = author_created_shape_paint_v1(
        Some(paste.fragment.rectangle.fill.clone()),
        Some(paste.fragment.rectangle.stroke.clone()),
    )
    .map_err(|error| {
        AuthoringFragmentError::InvalidMaterializedShape(CreateShapeError::InvalidPaint(error))
    })?;

    let entity = create_shape_entity_v1(&CreateShapeV1 {
        node_id: paste.identity_map.destination_node_id.clone(),
        page_id: paste.destination_page_id.clone(),
        bounds,
        paint,
    })
    .map_err(AuthoringFragmentError::InvalidMaterializedShape)?;

    Ok(PasteFragmentResultV1 {
        identity_map: paste.identity_map.clone(),
        entity,
    })
}

fn translated_bounds_v1(
    bounds: RectEmuV1,
    placement: TranslationEmuV1,
) -> Result<RectEmuV1, AuthoringFragmentError> {
    let x = bounds
        .x
        .checked_add(placement.dx_emu)
        .ok_or(AuthoringFragmentError::UnsafeTranslation)?;
    let y = bounds
        .y
        .checked_add(placement.dy_emu)
        .ok_or(AuthoringFragmentError::UnsafeTranslation)?;
    let translated = RectEmuV1 {
        x,
        y,
        width: bounds.width,
        height: bounds.height,
    };
    validate_rect_emu_v1(translated).map_err(|_| AuthoringFragmentError::UnsafeTranslation)?;
    Ok(translated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EntityProvenanceV1, ShapePaintProvenanceV1, ShapePaintV1, SolidFillV1, SolidStrokeV1, Srgb8,
    };

    const SOURCE_ID: &str = "01890f47-0c00-7abc-8def-0123456789ab";
    const PASTE_ID: &str = "01890f47-0c01-7abc-8def-0123456789ab";
    const SECOND_PASTE_ID: &str = "01890f47-0c02-7abc-8def-0123456789ab";

    fn source_shape() -> AuthoredShapeV1 {
        AuthoredShapeV1 {
            node_id: SOURCE_ID.to_owned(),
            page_id: "page:source".to_owned(),
            parent_id: "page:source".to_owned(),
            shape_kind: ShapeKindV1::Rectangle,
            bounds: RectEmuV1 {
                x: -100,
                y: 200,
                width: 300,
                height: 400,
            },
            transform: ShapeTransformV1::Identity,
            paint: ShapePaintV1 {
                fill: Some(SolidFillV1 {
                    visible: true,
                    color: Srgb8 { r: 1, g: 2, b: 3 },
                }),
                stroke: Some(SolidStrokeV1 {
                    visible: true,
                    color: Srgb8 { r: 4, g: 5, b: 6 },
                    width_emu: 12_700,
                }),
                provenance: ShapePaintProvenanceV1::AuthorCreated,
            },
            provenance: EntityProvenanceV1::AuthorCreated,
        }
    }

    fn paste(destination_node_id: &str) -> PasteFragmentV1 {
        PasteFragmentV1 {
            fragment: capture_rectangle_fragment_v1(&source_shape()).expect("capture"),
            identity_map: PasteIdentityRemapV1 {
                fragment_entity_id: SINGLE_RECTANGLE_ENTITY_ID_V1.to_owned(),
                destination_node_id: destination_node_id.to_owned(),
            },
            destination_page_id: "page:destination".to_owned(),
            placement: TranslationEmuV1 {
                dx_emu: 1_000,
                dy_emu: -50,
            },
        }
    }

    #[test]
    fn capture_uses_payload_local_identity_and_keeps_source_only_as_provenance() {
        let fragment = capture_rectangle_fragment_v1(&source_shape()).expect("capture");
        assert_eq!(fragment.schema_version, AUTHORING_FRAGMENT_SCHEMA_V1);
        assert_eq!(
            fragment.rectangle.fragment_entity_id,
            SINGLE_RECTANGLE_ENTITY_ID_V1
        );
        assert_eq!(
            fragment
                .rectangle
                .source_provenance
                .as_ref()
                .expect("source provenance")
                .source_node_id,
            SOURCE_ID
        );
        assert_eq!(fragment.rectangle.bounds, source_shape().bounds);
        assert_eq!(
            fragment.rectangle.fill,
            source_shape().paint.fill.expect("fill")
        );
        assert_eq!(
            fragment.rectangle.stroke,
            source_shape().paint.stroke.expect("stroke")
        );
    }

    #[test]
    fn capture_rejects_source_backed_paint_provenance() {
        let mut shape = source_shape();
        shape.paint.provenance = ShapePaintProvenanceV1::SourceBacked {
            source_ref: crate::SourceRefV1 {
                format: "pub".to_owned(),
                adapter_version: "pub-rs/0.1".to_owned(),
                source_hash_hex: "11".repeat(32),
                carrier: "/Escher/EscherStm".to_owned(),
                object_key: Some("shape/1".to_owned()),
                path: Some("SpContainer/FOPT".to_owned()),
                role: crate::SourceRoleV1::Semantic,
                authority: crate::AuthorityClassV1::Authoritative,
                confidence: crate::ReadConfidenceV1::Exact,
            },
        };
        assert_eq!(
            capture_rectangle_fragment_v1(&shape),
            Err(AuthoringFragmentError::PaintMustBeAuthorCreated)
        );
    }

    #[test]
    fn persisted_paste_replays_same_destination_identity_and_state() {
        let operation = paste(PASTE_ID);
        let first = materialize_paste_fragment_v1(&operation).expect("paste");
        let replay = materialize_paste_fragment_v1(&operation).expect("replay");
        assert_eq!(first, replay);
        assert_eq!(first.entity.node_id, PASTE_ID);
        assert_eq!(first.entity.parent_id, "page:destination");
        assert_eq!(
            first.entity.bounds,
            RectEmuV1 {
                x: 900,
                y: 150,
                width: 300,
                height: 400,
            }
        );
        assert_eq!(
            first.entity.paint.provenance,
            ShapePaintProvenanceV1::AuthorCreated
        );
    }

    #[test]
    fn second_user_paste_gets_distinct_identity_but_same_semantic_payload() {
        let first = materialize_paste_fragment_v1(&paste(PASTE_ID)).expect("first");
        let second = materialize_paste_fragment_v1(&paste(SECOND_PASTE_ID)).expect("second");
        assert_ne!(first.entity.node_id, second.entity.node_id);
        assert_eq!(first.entity.bounds, second.entity.bounds);
        assert_eq!(first.entity.paint, second.entity.paint);
    }

    #[test]
    fn source_identity_never_becomes_destination_identity() {
        let operation = paste(SOURCE_ID);
        assert_eq!(
            materialize_paste_fragment_v1(&operation),
            Err(AuthoringFragmentError::DestinationReusesSourceIdentity)
        );
    }

    #[test]
    fn identity_map_must_target_the_fragment_local_entity() {
        let mut operation = paste(PASTE_ID);
        operation.identity_map.fragment_entity_id = "entity:forged".to_owned();
        assert_eq!(
            materialize_paste_fragment_v1(&operation),
            Err(AuthoringFragmentError::IdentityMapMismatch)
        );
    }

    #[test]
    fn translation_must_remain_javascript_safe_at_edges() {
        let mut operation = paste(PASTE_ID);
        operation.fragment.rectangle.bounds = RectEmuV1 {
            x: crate::create_shape::MAX_SAFE_EMU_V1 - 1,
            y: 0,
            width: 1,
            height: 1,
        };
        operation.placement.dx_emu = 1;
        assert_eq!(
            materialize_paste_fragment_v1(&operation),
            Err(AuthoringFragmentError::UnsafeTranslation)
        );
    }

    #[test]
    fn fragment_serialization_is_deterministic_and_roundtrips() {
        let fragment = capture_rectangle_fragment_v1(&source_shape()).expect("capture");
        let a = serde_json::to_vec(&fragment).expect("serialize");
        let b = serde_json::to_vec(&fragment).expect("serialize");
        assert_eq!(a, b);
        let reopened: AuthoringFragmentV1 = serde_json::from_slice(&a).expect("deserialize");
        assert_eq!(fragment, reopened);
    }
}
