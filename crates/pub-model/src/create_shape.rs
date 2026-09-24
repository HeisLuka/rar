use serde::{Deserialize, Serialize};

use crate::{
    ShapePaintProvenanceV1, ShapePaintV1, ShapePaintValidationError, validate_shape_paint_v1,
};

pub const MAX_SAFE_EMU_V1: i64 = 9_007_199_254_740_991;
pub const MIN_SAFE_EMU_V1: i64 = -MAX_SAFE_EMU_V1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RectEmuV1 {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateShapeV1 {
    pub node_id: String,
    pub page_id: String,
    pub bounds: RectEmuV1,
    pub paint: ShapePaintV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShapeDestinationV2 {
    Page { id: String },
    Group { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateShapeV2 {
    pub node_id: String,
    pub page_id: String,
    pub destination: ShapeDestinationV2,
    pub bounds: RectEmuV1,
    pub paint: ShapePaintV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShapeKindV1 {
    Rectangle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShapeTransformV1 {
    Identity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntityProvenanceV1 {
    AuthorCreated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoredShapeV1 {
    pub node_id: String,
    pub page_id: String,
    pub parent_id: String,
    pub shape_kind: ShapeKindV1,
    pub bounds: RectEmuV1,
    pub transform: ShapeTransformV1,
    pub paint: ShapePaintV1,
    pub provenance: EntityProvenanceV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateShapeError {
    InvalidUuidV7,
    EmptyPageId,
    InvalidBounds,
    UnsafeBoundsEdge,
    InvalidPaint(ShapePaintValidationError),
    ExplicitFillAndStrokeRequired,
    PaintMustBeAuthorCreated,
}

pub fn create_shape_entity_v2(
    operation: &CreateShapeV2,
) -> Result<AuthoredShapeV1, CreateShapeError> {
    validate_uuid_v7_v1(&operation.node_id)?;
    if operation.page_id.is_empty() {
        return Err(CreateShapeError::EmptyPageId);
    }
    validate_rect_emu_v1(operation.bounds)?;
    validate_shape_paint_v1(&operation.paint).map_err(CreateShapeError::InvalidPaint)?;
    if operation.paint.fill.is_none() || operation.paint.stroke.is_none() {
        return Err(CreateShapeError::ExplicitFillAndStrokeRequired);
    }
    if !matches!(
        operation.paint.provenance,
        ShapePaintProvenanceV1::AuthorCreated
    ) {
        return Err(CreateShapeError::PaintMustBeAuthorCreated);
    }

    let parent_id = match &operation.destination {
        ShapeDestinationV2::Page { id } => {
            if id.is_empty() || id != &operation.page_id {
                return Err(CreateShapeError::EmptyPageId);
            }
            id.clone()
        }
        ShapeDestinationV2::Group { id } => {
            if id.is_empty() {
                return Err(CreateShapeError::EmptyPageId);
            }
            id.clone()
        }
    };

    Ok(AuthoredShapeV1 {
        node_id: operation.node_id.clone(),
        page_id: operation.page_id.clone(),
        parent_id,
        shape_kind: ShapeKindV1::Rectangle,
        bounds: operation.bounds,
        transform: ShapeTransformV1::Identity,
        paint: operation.paint.clone(),
        provenance: EntityProvenanceV1::AuthorCreated,
    })
}

pub fn create_shape_entity_v1(
    operation: &CreateShapeV1,
) -> Result<AuthoredShapeV1, CreateShapeError> {
    validate_uuid_v7_v1(&operation.node_id)?;
    if operation.page_id.is_empty() {
        return Err(CreateShapeError::EmptyPageId);
    }
    validate_rect_emu_v1(operation.bounds)?;
    validate_shape_paint_v1(&operation.paint).map_err(CreateShapeError::InvalidPaint)?;
    if operation.paint.fill.is_none() || operation.paint.stroke.is_none() {
        return Err(CreateShapeError::ExplicitFillAndStrokeRequired);
    }
    if !matches!(
        operation.paint.provenance,
        ShapePaintProvenanceV1::AuthorCreated
    ) {
        return Err(CreateShapeError::PaintMustBeAuthorCreated);
    }

    Ok(AuthoredShapeV1 {
        node_id: operation.node_id.clone(),
        page_id: operation.page_id.clone(),
        parent_id: operation.page_id.clone(),
        shape_kind: ShapeKindV1::Rectangle,
        bounds: operation.bounds,
        transform: ShapeTransformV1::Identity,
        paint: operation.paint.clone(),
        provenance: EntityProvenanceV1::AuthorCreated,
    })
}

pub fn validate_uuid_v7_v1(value: &str) -> Result<(), CreateShapeError> {
    let bytes = value.as_bytes();
    if bytes.len() != 36
        || bytes[8] != b'-'
        || bytes[13] != b'-'
        || bytes[18] != b'-'
        || bytes[23] != b'-'
    {
        return Err(CreateShapeError::InvalidUuidV7);
    }

    for (index, byte) in bytes.iter().copied().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            continue;
        }
        if !(byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
            return Err(CreateShapeError::InvalidUuidV7);
        }
    }

    if bytes[14] != b'7' || !matches!(bytes[19], b'8' | b'9' | b'a' | b'b') {
        return Err(CreateShapeError::InvalidUuidV7);
    }

    Ok(())
}

pub fn validate_rect_emu_v1(rect: RectEmuV1) -> Result<(), CreateShapeError> {
    if rect.width <= 0
        || rect.height <= 0
        || rect.x < MIN_SAFE_EMU_V1
        || rect.x > MAX_SAFE_EMU_V1
        || rect.y < MIN_SAFE_EMU_V1
        || rect.y > MAX_SAFE_EMU_V1
        || rect.width > MAX_SAFE_EMU_V1
        || rect.height > MAX_SAFE_EMU_V1
    {
        return Err(CreateShapeError::InvalidBounds);
    }

    let right = rect
        .x
        .checked_add(rect.width)
        .ok_or(CreateShapeError::UnsafeBoundsEdge)?;
    let bottom = rect
        .y
        .checked_add(rect.height)
        .ok_or(CreateShapeError::UnsafeBoundsEdge)?;
    if !(MIN_SAFE_EMU_V1..=MAX_SAFE_EMU_V1).contains(&right)
        || !(MIN_SAFE_EMU_V1..=MAX_SAFE_EMU_V1).contains(&bottom)
    {
        return Err(CreateShapeError::UnsafeBoundsEdge);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SolidFillV1, SolidStrokeV1, Srgb8, author_created_shape_paint_v1};

    fn paint() -> ShapePaintV1 {
        author_created_shape_paint_v1(
            Some(SolidFillV1 {
                visible: true,
                color: Srgb8 { r: 1, g: 2, b: 3 },
            }),
            Some(SolidStrokeV1 {
                visible: true,
                color: Srgb8 { r: 4, g: 5, b: 6 },
                width_emu: 12_700,
            }),
        )
        .expect("paint")
    }

    fn operation() -> CreateShapeV1 {
        CreateShapeV1 {
            node_id: "01890f47-0c00-7abc-8def-0123456789ab".to_owned(),
            page_id: "page:1".to_owned(),
            bounds: RectEmuV1 {
                x: -100,
                y: 200,
                width: 300,
                height: 400,
            },
            paint: paint(),
        }
    }

    #[test]
    fn create_shape_v2_preserves_explicit_page_or_group_parent() {
        let page = CreateShapeV2 {
            node_id: operation().node_id,
            page_id: "page:1".to_owned(),
            destination: ShapeDestinationV2::Page {
                id: "page:1".to_owned(),
            },
            bounds: operation().bounds,
            paint: paint(),
        };
        let page_entity = create_shape_entity_v2(&page).expect("page create");
        assert_eq!(page_entity.parent_id, "page:1");

        let group = CreateShapeV2 {
            node_id: "01890f47-0c01-7abc-8def-0123456789ab".to_owned(),
            page_id: "page:1".to_owned(),
            destination: ShapeDestinationV2::Group {
                id: "group:1".to_owned(),
            },
            bounds: RectEmuV1 {
                x: 10,
                y: 20,
                width: 30,
                height: 40,
            },
            paint: paint(),
        };
        let group_entity = create_shape_entity_v2(&group).expect("group create");
        assert_eq!(group_entity.parent_id, "group:1");
        assert_eq!(group_entity.page_id, "page:1");
    }

    #[test]
    fn create_shape_materializes_one_direct_page_owned_identity_rectangle() {
        let entity = create_shape_entity_v1(&operation()).expect("create");
        assert_eq!(entity.node_id, operation().node_id);
        assert_eq!(entity.page_id, "page:1");
        assert_eq!(entity.parent_id, "page:1");
        assert_eq!(entity.shape_kind, ShapeKindV1::Rectangle);
        assert_eq!(entity.transform, ShapeTransformV1::Identity);
        assert_eq!(entity.provenance, EntityProvenanceV1::AuthorCreated);
        assert_eq!(entity.paint, paint());
    }

    #[test]
    fn replay_and_save_reopen_preserve_same_id_and_state() {
        let first = create_shape_entity_v1(&operation()).expect("create");
        let replay = create_shape_entity_v1(&operation()).expect("replay");
        assert_eq!(first, replay);

        let bytes = serde_json::to_vec(&first).expect("serialize");
        let reopened: AuthoredShapeV1 = serde_json::from_slice(&bytes).expect("deserialize");
        assert_eq!(first, reopened);
    }

    #[test]
    fn uuid_must_be_canonical_lowercase_v7_with_rfc_variant() {
        assert_eq!(
            validate_uuid_v7_v1("01890f47-0c00-7abc-8def-0123456789ab"),
            Ok(())
        );
        for bad in [
            "01890f47-0c00-4abc-8def-0123456789ab",
            "01890f47-0c00-7abc-7def-0123456789ab",
            "01890F47-0C00-7ABC-8DEF-0123456789AB",
            "not-a-uuid",
        ] {
            assert_eq!(
                validate_uuid_v7_v1(bad),
                Err(CreateShapeError::InvalidUuidV7)
            );
        }
    }

    #[test]
    fn bounds_must_be_positive_and_js_safe_at_edges() {
        assert_eq!(
            validate_rect_emu_v1(RectEmuV1 {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            }),
            Ok(())
        );
        assert_eq!(
            validate_rect_emu_v1(RectEmuV1 {
                x: 0,
                y: 0,
                width: 0,
                height: 1,
            }),
            Err(CreateShapeError::InvalidBounds)
        );
        assert_eq!(
            validate_rect_emu_v1(RectEmuV1 {
                x: MAX_SAFE_EMU_V1,
                y: 0,
                width: 1,
                height: 1,
            }),
            Err(CreateShapeError::UnsafeBoundsEdge)
        );
    }

    #[test]
    fn source_backed_paint_cannot_be_used_for_created_shape() {
        let mut operation = operation();
        operation.paint.provenance = ShapePaintProvenanceV1::SourceBacked {
            source_ref: crate::SourceRefV1 {
                format: "pub".to_owned(),
                adapter_version: "pub-rs/0.1".to_owned(),
                source_hash_hex: "11".repeat(32),
                carrier: "/Escher/EscherStm".to_owned(),
                object_key: None,
                path: None,
                role: crate::SourceRoleV1::Semantic,
                authority: crate::AuthorityClassV1::Authoritative,
                confidence: crate::ReadConfidenceV1::Exact,
            },
        };
        assert_eq!(
            create_shape_entity_v1(&operation),
            Err(CreateShapeError::PaintMustBeAuthorCreated)
        );
    }
}
