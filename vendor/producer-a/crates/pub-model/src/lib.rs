//! Граница семантической модели PUB.
//!
//! Этот crate намеренно не вводит универсальный `native_id`, единого владельца
//! текста или другие предполагаемые связи между проекциями. Текущие доказательства
//! показывают наличие нескольких собственных пространств идентичности в Contents,
//! Quill и OfficeArt/Escher. Отдельные типы идентификаторов и связи между ними
//! должны добавляться только тогда, когда явно зафиксированы область действия и
//! доказательство.
//!
//! Слой исходных байтов находится в `pub-core`; низкоуровневые парсеры проекций —
//! в соответствующих crate. Семантическая модель должна подниматься поверх этих
//! проекций, а не подменять их как источник истины.
//!
//! Код ниже начинает только с grounded semantic primitives. Он не является
//! универсальной publishing ontology и не определяет Layout Contract. Layout-facing
//! projection появляется отдельной boundary после effective authoring state.

mod core;
mod decimal;
mod entity_id;
mod geometry;
mod guides;
mod id_derivation;
mod identity;
mod provenance;
mod resolved_graph;
mod snapshot;
mod source_graph;
mod story;
mod table;
mod text;
mod validation;

pub use core::{
    BoxEdges, Document, ImageFrame, ImageResource, Node, NodeHeader, NodeKind, Page,
    PageValidationError, Paragraph, PixelSize, Size2D, Story, StoryStructureError, Style,
    TextFrame, TextRun, validate_story_structure,
};
pub use decimal::{Decimal, DecimalParseError};
pub use entity_id::{
    DocumentId, ExtensionId, NodeId, PageId, ParagraphId, ResourceId, StoryId, StyleId,
    TableCellId, TextRunId,
};
pub use geometry::{
    Affine2D, EMU_PER_CSS_PIXEL_96_DPI, EMU_PER_INCH, EMU_PER_MILLIMETER, EMU_PER_POINT, LengthEmu,
    RectEmu,
};
pub use guides::{GroundedRulerGuide, PublisherGuideRole, RulerGuide, RulerGuideAxis};
pub use id_derivation::{
    SOURCE_DERIVED_NAMESPACE_V1, SourceDerivedIdError, SourceDerivedIdInput, SourceIdComponent,
    derive_source_canonical_id, new_editor_canonical_id,
};
pub use identity::{CanonicalId, CanonicalIdParseError};
pub use provenance::{
    AuthorityClass, ByteRange, OpaqueExtension, PreservationPolicy, ReadConfidence, Sha256Digest,
    Sha256DigestParseError, SourceCapsuleRef, SourceDescriptor, SourceIdentityField, SourceRef,
    SourceRefValidationError, SourceRole,
};
pub use resolved_graph::ResolvedGraph;
pub use snapshot::{CDM_DEBUG_JSON_PROFILE_V0_1, to_cdm_debug_json_v0_1};
pub use source_graph::{
    CDM_VERSION_V0_1, SourceGraph, SourceGraphRegistryError, validate_source_graph_registries,
};
pub use story::{FlowDirection, StoryFlowError, StoryFrame, validate_story_frames};
pub use table::{SimpleRectangularTable, SimpleTableCell, SimpleTableError, TableCellAddress};
pub use text::{TextRange, TextRangeError};

pub use validation::{SemanticGraphError, validate_source_graph_semantics};
