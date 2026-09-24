//! Minimal public-safe port of the canonical CDM surface required by
//! CDM-SHAPE-PAINT-01.
//!
//! This crate intentionally contains only the Shape paint contract needed by
//! the active task. It is a dependency slice, not a mirror of the frozen
//! historical repositories.

mod create_shape;
mod shape_paint;
mod shape_paint_op;

pub use shape_paint::{
    AuthorityClassV1, ReadConfidenceV1, ShapePaintProvenanceV1, ShapePaintV1,
    ShapePaintValidationError, SolidFillV1, SolidStrokeV1, SourceRefV1, SourceRoleV1, Srgb8,
    author_created_shape_paint_v1, canonical_shape_paint_hash_v1, validate_shape_paint_v1,
};

pub use shape_paint_op::{
    SetFillV1, SetStrokeV1, ShapePaintOperationError, ShapePaintOperationV1,
    apply_shape_paint_operation_v1, inverse_shape_paint_operation_v1,
};

pub use create_shape::{
    AuthoredShapeV1, CreateShapeError, CreateShapeV1, CreateShapeV2, EntityProvenanceV1, RectEmuV1,
    ShapeDestinationV2, ShapeKindV1, ShapeTransformV1, create_shape_entity_v1,
    create_shape_entity_v2, validate_rect_emu_v1, validate_uuid_v7_v1,
};
