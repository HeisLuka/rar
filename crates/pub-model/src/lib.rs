//! Minimal public-safe port of the canonical CDM surface required by
//! CDM-SHAPE-PAINT-01.
//!
//! This crate intentionally contains only the Shape paint contract needed by
//! the active task. It is a dependency slice, not a mirror of the frozen
//! historical repositories.

mod create_shape;
mod create_textbox;
mod fragment;
mod projection_context;
mod rotate_quarter;
mod shape_paint;
mod shape_paint_op;
mod source_identity;
mod text_preset;

pub use shape_paint::{
    AuthorityClassV1, ReadConfidenceV1, ShapePaintProvenanceV1, ShapePaintV1,
    ShapePaintValidationError, SolidFillV1, SolidStrokeV1, SourceRefV1, SourceRoleV1, Srgb8,
    author_created_shape_paint_v1, canonical_shape_paint_hash_v1, validate_shape_paint_v1,
};

pub use shape_paint_op::{
    SetFillV1, SetStrokeV1, ShapePaintOperationError, ShapePaintOperationV1,
    apply_shape_paint_operation_v1, inverse_shape_paint_operation_v1,
};

pub use create_textbox::{
    AuthoredTextFrameV1, AuthoringStorySeedV1, CreateTextBoxError, CreateTextBoxPlanV1,
    CreateTextBoxV1, TextBoxProvenanceV1, create_textbox_plan_v1,
};

pub use create_shape::{
    AuthoredShapeV1, CreateShapeError, CreateShapeV1, CreateShapeV2, EntityProvenanceV1, RectEmuV1,
    ShapeDestinationV2, ShapeKindV1, ShapeTransformV1, create_shape_entity_v1,
    create_shape_entity_v2, validate_rect_emu_v1, validate_uuid_v7_v1,
};

pub use fragment::{
    AUTHORING_FRAGMENT_SCHEMA_V1, AuthoringFragmentError, AuthoringFragmentV1,
    FragmentSourceProvenanceV1, PasteFragmentResultV1, PasteFragmentV1, PasteIdentityRemapV1,
    RectangleFragmentEntityV1, SINGLE_RECTANGLE_ENTITY_ID_V1, TranslationEmuV1,
    capture_rectangle_fragment_v1, materialize_paste_fragment_v1,
};

pub use projection_context::{
    CmoProjectionRelationV1, MasterProjectionRelationV1, PUB_PROJECTION_CONTEXT_SCHEMA_V1,
    PubProjectionContextV1,
};

pub use source_identity::{
    PUB_SOURCE_ADAPTER_ID_V1, SOURCE_DERIVED_NAMESPACE_V1, SourceIdentityError,
    derive_pub_node_id_v1, derive_pub_page_id_v1, derive_pub_story_id_v1, derive_source_uuid_v5_v1,
    pub_contents_object_key_v1, pub_quill_story_object_key_v1,
};

pub use rotate_quarter::{
    ExactAffineV1, RotateQuarterError, RotateQuarterResultV1, apply_authored_shape_quarter_turn_v1,
    canonical_shape_affine_v1, validate_exact_affine_v1,
};

pub use text_preset::{
    AUTHORING_TEXT_PRESET_VERSION_V1, AuthoringCharacterDefaultsV1, AuthoringParagraphAlignmentV1,
    AuthoringParagraphDefaultsV1, AuthoringTextPresetError, AuthoringTextPresetRecordV1,
    AuthoringTextPresetV1, AuthoringTextShapingInputV1, authoring_text_preset_id_v1,
    authoring_text_preset_record_v1, authoring_text_shaping_input_v1, font_fingerprint_v1,
    validate_font_resource_v1,
};
