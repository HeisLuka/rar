//! Production Rust state for canonical Chaptera CreateShapeV1.
//!
//! This module deliberately models Chaptera-authored rectangles as a separate
//! authoring overlay. It never invents Publisher Contents/Oid/SPID/seqNum
//! identities and never infers authored provenance from an empty SourceRef set.

use pub_model::{NodeId, PageId, RectEmu};
use serde::{Deserialize, Serialize};

pub const MAX_SAFE_EMU_V1: i64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Srgb8V1 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoredSolidFillV1 {
    pub visible: bool,
    pub color: Srgb8V1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoredSolidStrokeV1 {
    pub visible: bool,
    pub color: Srgb8V1,
    pub width_emu: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthoredEntityProvenanceV1 {
    AuthorCreated,
    SourceBacked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthoredShapeKindV1 {
    Rectangle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthoredShapeTransformV1 {
    Identity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoredShapePaintV1 {
    pub fill: AuthoredSolidFillV1,
    pub stroke: AuthoredSolidStrokeV1,
    pub provenance: AuthoredEntityProvenanceV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoredShapeRuntimeV1 {
    pub node_id: NodeId,
    pub page_id: PageId,
    pub parent_id: PageId,
    pub shape_kind: AuthoredShapeKindV1,
    pub bounds: RectEmu,
    pub transform: AuthoredShapeTransformV1,
    pub paint: AuthoredShapePaintV1,
    pub provenance: AuthoredEntityProvenanceV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateShapeRuntimeValidationError {
    NodeIdNotUuidV7,
    ParentPageMismatch,
    UnsupportedShapeKind,
    UnsupportedTransform,
    NonAuthorCreatedProvenance,
    NonAuthorCreatedPaintProvenance,
    InvalidBounds,
    InvalidPaint,
}

pub fn is_editor_created_uuid_v7_node_id(node_id: NodeId) -> bool {
    let bytes = node_id.as_canonical().as_bytes();
    (bytes[6] & 0xf0) == 0x70 && (bytes[8] & 0xc0) == 0x80
}

pub fn validate_authored_shape_runtime_v1(
    shape: &AuthoredShapeRuntimeV1,
) -> Result<(), CreateShapeRuntimeValidationError> {
    if !is_editor_created_uuid_v7_node_id(shape.node_id) {
        return Err(CreateShapeRuntimeValidationError::NodeIdNotUuidV7);
    }
    if shape.parent_id != shape.page_id {
        return Err(CreateShapeRuntimeValidationError::ParentPageMismatch);
    }
    if shape.shape_kind != AuthoredShapeKindV1::Rectangle {
        return Err(CreateShapeRuntimeValidationError::UnsupportedShapeKind);
    }
    if shape.transform != AuthoredShapeTransformV1::Identity {
        return Err(CreateShapeRuntimeValidationError::UnsupportedTransform);
    }
    if shape.provenance != AuthoredEntityProvenanceV1::AuthorCreated {
        return Err(CreateShapeRuntimeValidationError::NonAuthorCreatedProvenance);
    }
    if shape.paint.provenance != AuthoredEntityProvenanceV1::AuthorCreated {
        return Err(CreateShapeRuntimeValidationError::NonAuthorCreatedPaintProvenance);
    }

    let bounds = shape.bounds;
    for value in [bounds.x.get(), bounds.y.get()] {
        if !(-MAX_SAFE_EMU_V1..=MAX_SAFE_EMU_V1).contains(&value) {
            return Err(CreateShapeRuntimeValidationError::InvalidBounds);
        }
    }
    for value in [bounds.width.get(), bounds.height.get()] {
        if value <= 0 || value > MAX_SAFE_EMU_V1 {
            return Err(CreateShapeRuntimeValidationError::InvalidBounds);
        }
    }
    let Some(right) = bounds.right() else {
        return Err(CreateShapeRuntimeValidationError::InvalidBounds);
    };
    let Some(bottom) = bounds.bottom() else {
        return Err(CreateShapeRuntimeValidationError::InvalidBounds);
    };
    if !(-MAX_SAFE_EMU_V1..=MAX_SAFE_EMU_V1).contains(&right.get())
        || !(-MAX_SAFE_EMU_V1..=MAX_SAFE_EMU_V1).contains(&bottom.get())
    {
        return Err(CreateShapeRuntimeValidationError::InvalidBounds);
    }

    if shape.paint.stroke.width_emu <= 0 || shape.paint.stroke.width_emu > MAX_SAFE_EMU_V1 {
        return Err(CreateShapeRuntimeValidationError::InvalidPaint);
    }

    Ok(())
}
