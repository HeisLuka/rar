//! Bounded authoring session for Publisher migration workflows.
//!
//! This crate does not make a general "editable PUB" claim. Every edit
//! operation is capability-gated and preserves the immutable source identity.
//! The first operation is a bounded ordinary-story text replacement over the
//! resolved authoring graph. Native PUB materialization remains a separate
//! writer gate.

mod writer_assessment;

pub use writer_assessment::{
    EDITOR_PUB_WRITER_ASSESSMENT_SCHEMA_V0_1, EditorPubPersistenceAssessment,
    EditorPubWriterAssessment, EditorPubWriterAssessmentError, EditorStoryWriterProbeResult,
    EditorStoryWriterProbeState, EffectiveStoryTextMutation,
};

use pub_export::{
    CapabilityLevel, ExportPlan, ExportReport, ExportReportSource, FormatCompatibilityManifest,
    FormatRepresentability, LossItem, LossKind, LossSeverity, PersistenceCompatibilityAssessment,
    PersistenceCompatibilityError, PersistenceRequirement, PersistenceRequirements,
    PersistenceTargetProfile, SemanticFeatureRequest, TargetCapabilityManifest, TargetProfile,
    WriterCapabilityManifest, assess_persistence_compatibility, build_export_report, plan_export,
    render_human_summary,
};
use pub_idml::{
    IDML_ADAPTER_VERSION_V0_1, IDML_SCHEMA_FENCE_LEGACY_DOM_7, IMAGE_BYTES_FEATURE,
    IMAGE_CONTENT_TRANSFORM_FEATURE, IMAGE_FRAME_GEOMETRY_FEATURE, IdmlEmbeddedImagePlacement,
    IdmlWireProfile, add_embedded_images_to_idml, project_resolved_graph_to_idml, write_idml_ucf,
};
pub use pub_model::{LengthEmu, NodeId, RectEmu, Sha256Digest, StoryId, TableCellId};
use pub_model::{
    ResourceId, SourceDerivedIdInput, StoryFrame, derive_source_canonical_id, validate_story_frames,
};
use pub_odg::{
    ODG_ADAPTER_VERSION_V0_1, ODG_SCHEMA_FENCE_ODF_1_4, project_resolved_graph_to_odg, write_odg,
};
use pub_reader::{
    PubResolvedGraph, PubResolvedNodePayload, build_mature_0x2c_source_graph,
    materialize_bounded_simple_table_cells, resolve_pub_source_graph,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::Cursor;

pub const EDITOR_PROJECT_VERSION_V0_1: &str = "pub-editor-v0.1";
pub const EDITOR_PROJECT_VERSION_V0_2: &str = "pub-editor-v0.2";
pub const EDITOR_PROJECT_VERSION_V0_3: &str = "pub-editor-v0.3";
pub const EDITOR_PROJECT_VERSION_V0_4: &str = "pub-editor-v0.4";
pub const EDITOR_PROJECT_VERSION_V0_5: &str = "pub-editor-v0.5";
pub const EDITOR_PROJECT_VERSION_CURRENT: &str = EDITOR_PROJECT_VERSION_V0_5;
pub const PUB_MATURE_0X2C_PERSISTENCE_PROFILE: &str = "mature-0x2c";
pub const PUB_MATURE_0X2C_SCHEMA_FENCE: &str = "pub-family-0x2c";

/// Canonical Story-state identity shared with services/editor-api/story_range_v1.py.
pub fn story_state_id_v1(story_id: StoryId, text: &str) -> String {
    let payload = serde_json::json!({
        "protocol_version": "chaptera.story-state.v1",
        "story_id": story_id.as_canonical().to_string(),
        "text": text,
    });
    let bytes =
        serde_json::to_vec(&payload).expect("canonical Story state JSON serialization cannot fail");
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing lowercase hex into String cannot fail");
    }
    format!("sha256:{encoded}")
}

fn scalar_byte_offset(text: &str, scalar_index: u32) -> Option<usize> {
    let target = usize::try_from(scalar_index).ok()?;
    if target == text.chars().count() {
        return Some(text.len());
    }
    text.char_indices().nth(target).map(|(offset, _)| offset)
}

fn replace_scalar_range_text(
    text: &str,
    start_scalar: u32,
    end_scalar: u32,
    expected_before: &str,
    replacement_text: &str,
) -> Option<String> {
    if end_scalar < start_scalar {
        return None;
    }
    let start = scalar_byte_offset(text, start_scalar)?;
    let end = scalar_byte_offset(text, end_scalar)?;
    if text.get(start..end)? != expected_before {
        return None;
    }
    let mut after = String::with_capacity(
        text.len()
            .saturating_sub(end.saturating_sub(start))
            .saturating_add(replacement_text.len()),
    );
    after.push_str(&text[..start]);
    after.push_str(replacement_text);
    after.push_str(&text[end..]);
    Some(after)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EditOperation {
    ReplaceStoryRange {
        story_id: StoryId,
        start_scalar: u32,
        end_scalar: u32,
        expected_before: String,
        replacement_text: String,
        before_story_state_id: String,
        after_story_state_id: String,
    },
    ReplaceStoryText {
        story_id: StoryId,
        before: String,
        after: String,
    },
    ReplaceTableCellText {
        node_id: NodeId,
        story_id: StoryId,
        cell_id: TableCellId,
        before_story: String,
        after_story: String,
        before_ranges: Vec<TableCellRangeSnapshot>,
        after_ranges: Vec<TableCellRangeSnapshot>,
    },
    ReplaceImage {
        node_id: NodeId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before_asset: Option<Sha256Digest>,
        after_asset: Sha256Digest,
    },
    MoveNode {
        node_id: NodeId,
        before: RectEmu,
        after: RectEmu,
    },
    ResizeNode {
        node_id: NodeId,
        before: RectEmu,
        after: RectEmu,
    },
}

impl PersistenceRequirements for EditOperation {
    fn persistence_requirements(&self) -> Vec<PersistenceRequirement> {
        match self {
            Self::ReplaceStoryRange { story_id, .. } | Self::ReplaceStoryText { story_id, .. } => {
                vec![PersistenceRequirement {
                    feature: "story.text".into(),
                    origin: Some(story_id.into_canonical()),
                    property_path: Some("story.text".into()),
                }]
            }
            Self::ReplaceTableCellText {
                story_id, cell_id, ..
            } => vec![
                PersistenceRequirement {
                    feature: "story.text".into(),
                    origin: Some(story_id.into_canonical()),
                    property_path: Some("story.text".into()),
                },
                PersistenceRequirement {
                    feature: "table.cell_text".into(),
                    origin: Some(cell_id.into_canonical()),
                    property_path: Some("table.cell.text".into()),
                },
            ],
            Self::ReplaceImage { node_id, .. } => vec![PersistenceRequirement {
                feature: "image.replacement".into(),
                origin: Some(node_id.into_canonical()),
                property_path: Some("node.image.resource".into()),
            }],
            Self::MoveNode { node_id, .. } => vec![PersistenceRequirement {
                feature: "node.geometry.position".into(),
                origin: Some(node_id.into_canonical()),
                property_path: Some("node.bounds.position".into()),
            }],
            Self::ResizeNode { node_id, .. } => vec![PersistenceRequirement {
                feature: "node.geometry.bounds".into(),
                origin: Some(node_id.into_canonical()),
                property_path: Some("node.bounds".into()),
            }],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableCellRangeSnapshot {
    pub cell_id: TableCellId,
    pub utf16_start: u32,
    pub utf16_end: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorProjectAsset {
    pub sha256: Sha256Digest,
    pub mime: String,
    pub byte_len: u64,
}

impl EditorProjectAsset {
    pub fn file_name(&self) -> Result<String, EditorAssetError> {
        editor_asset_file_name(self.sha256, &self.mime)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorReplacementAsset {
    pub sha256: Sha256Digest,
    pub mime: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorProject {
    pub schema_version: String,
    pub source_hash: Sha256Digest,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<EditorProjectAsset>,
    pub operations: Vec<EditOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorAssetError {
    EmptyBytes,
    UnsupportedMime {
        mime: String,
    },
    SignatureMismatch {
        mime: String,
    },
    ByteLengthOverflow,
    MimeConflict {
        sha256: Sha256Digest,
        existing: String,
        requested: String,
    },
}

impl fmt::Display for EditorAssetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyBytes => formatter.write_str("replacement asset is empty"),
            Self::UnsupportedMime { mime } => {
                write!(formatter, "replacement asset MIME {mime:?} is unsupported")
            }
            Self::SignatureMismatch { mime } => write!(
                formatter,
                "replacement asset bytes do not match declared MIME {mime:?}"
            ),
            Self::ByteLengthOverflow => {
                formatter.write_str("replacement asset length does not fit u64")
            }
            Self::MimeConflict {
                sha256,
                existing,
                requested,
            } => write!(
                formatter,
                "replacement asset {sha256} is already registered as {existing:?}, not {requested:?}"
            ),
        }
    }
}

impl std::error::Error for EditorAssetError {}

impl PersistenceRequirements for EditorProject {
    fn persistence_requirements(&self) -> Vec<PersistenceRequirement> {
        let mut requirements = self
            .operations
            .iter()
            .flat_map(PersistenceRequirements::persistence_requirements)
            .collect::<Vec<_>>();
        requirements.sort();
        requirements.dedup();
        requirements
    }
}

pub fn mature_0x2c_pub_persistence_target() -> PersistenceTargetProfile {
    PersistenceTargetProfile {
        format: "pub".into(),
        profile: PUB_MATURE_0X2C_PERSISTENCE_PROFILE.into(),
        schema_fence: Some(PUB_MATURE_0X2C_SCHEMA_FENCE.into()),
    }
}

/// Ограниченный профиль представимости semantic-классов, уже материализованных
/// mature-0x2C моделью Publisher.
///
/// Это не утверждает, что текущий Rust native writer способен записать каждый
/// конкретный instance. Writer readiness намеренно передаётся отдельным manifest.
pub fn mature_0x2c_pub_format_manifest() -> FormatCompatibilityManifest {
    let mut features = BTreeMap::new();
    features.insert("story.text".into(), FormatRepresentability::Lossless);
    features.insert("table.cell_text".into(), FormatRepresentability::Lossless);
    features.insert("image.replacement".into(), FormatRepresentability::Lossless);
    features.insert(
        "node.geometry.position".into(),
        FormatRepresentability::Lossless,
    );
    features.insert(
        "node.geometry.bounds".into(),
        FormatRepresentability::Lossless,
    );

    FormatCompatibilityManifest {
        target: mature_0x2c_pub_persistence_target(),
        features,
        scoped: Vec::new(),
    }
}

pub fn assess_mature_0x2c_pub_project_persistence(
    project: &EditorProject,
    writer: &WriterCapabilityManifest,
) -> Result<PersistenceCompatibilityAssessment, PersistenceCompatibilityError> {
    assess_persistence_compatibility(
        &mature_0x2c_pub_format_manifest(),
        writer,
        project.persistence_requirements(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorError {
    SourceIdentityChanged,
    MissingStory {
        story_id: StoryId,
    },
    RichStoryUnsupported {
        story_id: StoryId,
    },
    TableStoryUnsupported {
        story_id: StoryId,
    },
    FrameCountUnsupported {
        story_id: StoryId,
        found: usize,
    },
    FrameTopologyUnsupported {
        story_id: StoryId,
        errors: usize,
    },
    TableEditUnsupported {
        node_id: NodeId,
    },
    MissingTableCell {
        node_id: NodeId,
        cell_id: TableCellId,
    },
    TableCellNoChange {
        node_id: NodeId,
        cell_id: TableCellId,
    },
    ImageReplaceUnsupported {
        node_id: NodeId,
    },
    MissingReplacementAsset {
        sha256: Sha256Digest,
    },
    ImageReplacementNoChange {
        node_id: NodeId,
        sha256: Sha256Digest,
    },
    StaleImageOperation {
        node_id: NodeId,
    },
    NodeMoveUnsupported {
        node_id: NodeId,
    },
    NodeMoveNoChange {
        node_id: NodeId,
    },
    NodeMoveOverflow {
        node_id: NodeId,
    },
    StaleNodeMove {
        node_id: NodeId,
    },
    NodeResizeUnsupported {
        node_id: NodeId,
    },
    NodeResizeNoChange {
        node_id: NodeId,
    },
    NodeResizeNoSizeChange {
        node_id: NodeId,
    },
    NodeResizeNonPositive {
        node_id: NodeId,
    },
    NodeResizeOverflow {
        node_id: NodeId,
    },
    StaleNodeResize {
        node_id: NodeId,
    },
    NoChange {
        story_id: StoryId,
    },
    StaleOperation {
        story_id: StoryId,
    },
    NothingToUndo,
    NothingToRedo,
}

impl fmt::Display for EditorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceIdentityChanged => {
                formatter.write_str("editor source identity changed after session creation")
            }
            Self::MissingStory { story_id } => write!(
                formatter,
                "story {} is not present in the resolved authoring graph",
                story_id.as_canonical()
            ),
            Self::RichStoryUnsupported { story_id } => write!(
                formatter,
                "story {} carries paragraph/run/field/hyperlink semantics that ReplaceStoryText does not remap",
                story_id.as_canonical()
            ),
            Self::TableStoryUnsupported { story_id } => write!(
                formatter,
                "story {} is owned by a table and is outside the first editor slice",
                story_id.as_canonical()
            ),
            Self::FrameCountUnsupported { story_id, found } => write!(
                formatter,
                "story {} is referenced by {found} ordinary frames; ReplaceStoryText requires at least one materialized frame",
                story_id.as_canonical()
            ),
            Self::FrameTopologyUnsupported { story_id, errors } => write!(
                formatter,
                "story {} has unsupported or inconsistent multi-frame topology ({errors} validation errors)",
                story_id.as_canonical()
            ),
            Self::TableEditUnsupported { node_id } => write!(
                formatter,
                "table node {} is outside the bounded simple-cell text edit slice",
                node_id.as_canonical()
            ),
            Self::MissingTableCell { node_id, cell_id } => write!(
                formatter,
                "table node {} does not contain cell {}",
                node_id.as_canonical(),
                cell_id.as_canonical()
            ),
            Self::TableCellNoChange { node_id, cell_id } => write!(
                formatter,
                "replacement text for table node {} cell {} is identical to current text",
                node_id.as_canonical(),
                cell_id.as_canonical()
            ),
            Self::ImageReplaceUnsupported { node_id } => write!(
                formatter,
                "image node {} is outside the bounded crop-free replacement slice",
                node_id.as_canonical()
            ),
            Self::MissingReplacementAsset { sha256 } => write!(
                formatter,
                "replacement image asset {sha256} is not registered in this editor session"
            ),
            Self::ImageReplacementNoChange { node_id, sha256 } => write!(
                formatter,
                "image node {} already uses replacement asset {sha256}",
                node_id.as_canonical()
            ),
            Self::StaleImageOperation { node_id } => write!(
                formatter,
                "image node {} no longer matches the replacement operation precondition",
                node_id.as_canonical()
            ),
            Self::NodeMoveUnsupported { node_id } => write!(
                formatter,
                "node {} is outside the bounded directly-page-owned move slice",
                node_id.as_canonical()
            ),
            Self::NodeMoveNoChange { node_id } => write!(
                formatter,
                "node {} already has the requested authored position",
                node_id.as_canonical()
            ),
            Self::NodeMoveOverflow { node_id } => write!(
                formatter,
                "node {} move would overflow canonical EMU bounds",
                node_id.as_canonical()
            ),
            Self::StaleNodeMove { node_id } => write!(
                formatter,
                "node {} no longer matches the move operation precondition",
                node_id.as_canonical()
            ),
            Self::NodeResizeUnsupported { node_id } => write!(
                formatter,
                "node {} is outside the bounded directly-page-owned resize slice",
                node_id.as_canonical()
            ),
            Self::NodeResizeNoChange { node_id } => write!(
                formatter,
                "node {} already has the requested authored bounds",
                node_id.as_canonical()
            ),
            Self::NodeResizeNoSizeChange { node_id } => write!(
                formatter,
                "node {} ResizeNode request does not change width or height",
                node_id.as_canonical()
            ),
            Self::NodeResizeNonPositive { node_id } => write!(
                formatter,
                "node {} resize requires strictly positive width and height",
                node_id.as_canonical()
            ),
            Self::NodeResizeOverflow { node_id } => write!(
                formatter,
                "node {} resize would overflow canonical EMU bounds",
                node_id.as_canonical()
            ),
            Self::StaleNodeResize { node_id } => write!(
                formatter,
                "node {} no longer matches the resize operation precondition",
                node_id.as_canonical()
            ),
            Self::NoChange { story_id } => write!(
                formatter,
                "replacement text for story {} is identical to the current text",
                story_id.as_canonical()
            ),
            Self::StaleOperation { story_id } => write!(
                formatter,
                "story {} no longer matches the edit operation precondition",
                story_id.as_canonical()
            ),
            Self::NothingToUndo => formatter.write_str("editor session has nothing to undo"),
            Self::NothingToRedo => formatter.write_str("editor session has nothing to redo"),
        }
    }
}

impl EditorError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::SourceIdentityChanged => "source_identity_changed",
            Self::MissingStory { .. } => "missing_story",
            Self::RichStoryUnsupported { .. } => "rich_story_unsupported",
            Self::TableStoryUnsupported { .. } => "table_story_unsupported",
            Self::FrameCountUnsupported { .. } => "frame_count_unsupported",
            Self::FrameTopologyUnsupported { .. } => "frame_topology_unsupported",
            Self::TableEditUnsupported { .. } => "table_edit_unsupported",
            Self::MissingTableCell { .. } => "missing_table_cell",
            Self::TableCellNoChange { .. } => "table_cell_no_change",
            Self::ImageReplaceUnsupported { .. } => "image_replace_unsupported",
            Self::MissingReplacementAsset { .. } => "missing_replacement_asset",
            Self::ImageReplacementNoChange { .. } => "image_replacement_no_change",
            Self::StaleImageOperation { .. } => "stale_image_operation",
            Self::NodeMoveUnsupported { .. } => "node_move_unsupported",
            Self::NodeMoveNoChange { .. } => "node_move_no_change",
            Self::NodeMoveOverflow { .. } => "node_move_overflow",
            Self::StaleNodeMove { .. } => "stale_node_move",
            Self::NodeResizeUnsupported { .. } => "node_resize_unsupported",
            Self::NodeResizeNoChange { .. } => "node_resize_no_change",
            Self::NodeResizeNoSizeChange { .. } => "node_resize_no_size_change",
            Self::NodeResizeNonPositive { .. } => "node_resize_non_positive",
            Self::NodeResizeOverflow { .. } => "node_resize_overflow",
            Self::StaleNodeResize { .. } => "stale_node_resize",
            Self::NoChange { .. } => "no_change",
            Self::StaleOperation { .. } => "stale_operation",
            Self::NothingToUndo => "nothing_to_undo",
            Self::NothingToRedo => "nothing_to_redo",
        }
    }
}

impl std::error::Error for EditorError {}

#[derive(Debug)]
pub enum EditorOpenError {
    SourceGraph(String),
    Resolve(String),
    Session(EditorError),
}

impl fmt::Display for EditorOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceGraph(message) => write!(
                formatter,
                "could not build the mature-0x2C source graph for the editor: {message}"
            ),
            Self::Resolve(message) => write!(
                formatter,
                "could not resolve the mature-0x2C authoring graph for the editor: {message}"
            ),
            Self::Session(error) => write!(formatter, "could not create editor session: {error}"),
        }
    }
}

impl std::error::Error for EditorOpenError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorProjectError {
    UnsupportedSchema {
        found: String,
    },
    SourceHashMismatch {
        expected: Sha256Digest,
        found: Sha256Digest,
    },
    SessionNotEmpty,
    LegacyProjectCarriesAssets,
    LegacyProjectCarriesImageOperation {
        index: usize,
    },
    LegacyProjectCarriesGeometryOperation {
        index: usize,
    },
    LegacyProjectCarriesResizeOperation {
        index: usize,
    },
    MissingAssetBytes {
        sha256: Sha256Digest,
    },
    AssetLengthMismatch {
        sha256: Sha256Digest,
        expected: u64,
        found: u64,
    },
    AssetHashMismatch {
        expected: Sha256Digest,
        found: Sha256Digest,
    },
    AssetMetadataNonCanonical,
    Asset {
        index: usize,
        error: EditorAssetError,
    },
    Session(EditorError),
    Operation {
        index: usize,
        error: EditorError,
    },
    OperationMismatch {
        index: usize,
    },
    InvalidTableAfterState {
        index: usize,
        node_id: NodeId,
        cell_id: TableCellId,
    },
}

impl fmt::Display for EditorProjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema { found } => write!(
                formatter,
                "editor project schema {found:?} is unsupported; expected {EDITOR_PROJECT_VERSION_V0_1:?}, {EDITOR_PROJECT_VERSION_V0_2:?}, {EDITOR_PROJECT_VERSION_V0_3:?}, {EDITOR_PROJECT_VERSION_V0_4:?}, or {EDITOR_PROJECT_VERSION_V0_5:?}"
            ),
            Self::SourceHashMismatch { expected, found } => write!(
                formatter,
                "editor project source hash {found} does not match opened PUB {expected}"
            ),
            Self::SessionNotEmpty => formatter.write_str(
                "editor project replay requires a fresh session with no existing edits or replacement assets",
            ),
            Self::LegacyProjectCarriesAssets => {
                formatter.write_str("pub-editor-v0.1 projects cannot carry replacement assets")
            }
            Self::LegacyProjectCarriesImageOperation { index } => write!(
                formatter,
                "editor project operation {index} uses ReplaceImage but the project schema predates pub-editor-v0.3"
            ),
            Self::LegacyProjectCarriesGeometryOperation { index } => write!(
                formatter,
                "editor project operation {index} uses MoveNode but the project schema predates pub-editor-v0.4"
            ),
            Self::LegacyProjectCarriesResizeOperation { index } => write!(
                formatter,
                "editor project operation {index} uses ResizeNode but the project schema predates pub-editor-v0.5"
            ),
            Self::MissingAssetBytes { sha256 } => {
                write!(
                    formatter,
                    "editor project replacement asset {sha256} is missing"
                )
            }
            Self::AssetLengthMismatch {
                sha256,
                expected,
                found,
            } => write!(
                formatter,
                "editor project replacement asset {sha256} has {found} bytes; expected {expected}"
            ),
            Self::AssetHashMismatch { expected, found } => write!(
                formatter,
                "editor project replacement asset hash {found} does not match metadata {expected}"
            ),
            Self::AssetMetadataNonCanonical => formatter.write_str(
                "editor project replacement asset metadata is not in canonical content-addressed order",
            ),
            Self::Asset { index, error } => {
                write!(
                    formatter,
                    "editor project asset {index} was rejected: {error}"
                )
            }
            Self::Session(error) => write!(formatter, "editor session is invalid: {error}"),
            Self::Operation { index, error } => {
                write!(
                    formatter,
                    "editor project operation {index} was rejected: {error}"
                )
            }
            Self::OperationMismatch { index } => write!(
                formatter,
                "editor project operation {index} does not match the canonical operation generated by the current editor"
            ),
            Self::InvalidTableAfterState {
                index,
                node_id,
                cell_id,
            } => write!(
                formatter,
                "editor project operation {index} cannot derive a bounded replacement for table node {} cell {}",
                node_id.as_canonical(),
                cell_id.as_canonical()
            ),
        }
    }
}

impl std::error::Error for EditorProjectError {}

pub fn open_mature_0x2c_editor(
    bytes: &[u8],
    source_hash: Sha256Digest,
) -> Result<EditorSession, EditorOpenError> {
    let source = build_mature_0x2c_source_graph(Cursor::new(bytes), source_hash)
        .map_err(|error| EditorOpenError::SourceGraph(error.to_string()))?;
    let resolved = resolve_pub_source_graph(&source.graph)
        .map_err(|error| EditorOpenError::Resolve(error.to_string()))?;
    EditorSession::new(resolved.graph).map_err(EditorOpenError::Session)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditorEditableTarget {
    Idml,
    Odg,
}

impl EditorEditableTarget {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Idml => "idml",
            Self::Odg => "odg",
        }
    }

    const fn format(self) -> &'static str {
        match self {
            Self::Idml => "idml",
            Self::Odg => "odg",
        }
    }

    const fn adapter_version(self) -> &'static str {
        match self {
            Self::Idml => IDML_ADAPTER_VERSION_V0_1,
            Self::Odg => ODG_ADAPTER_VERSION_V0_1,
        }
    }

    const fn schema_fence(self) -> &'static str {
        match self {
            Self::Idml => IDML_SCHEMA_FENCE_LEGACY_DOM_7,
            Self::Odg => ODG_SCHEMA_FENCE_ODF_1_4,
        }
    }
}

impl fmt::Display for EditorEditableTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Idml => "IDML",
            Self::Odg => "ODG",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorEditableExportPreview {
    pub target: EditorEditableTarget,
    pub report: ExportReport,
    pub human_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorEditableTableCell {
    pub node_id: NodeId,
    pub story_id: StoryId,
    pub cell_id: TableCellId,
    pub row: u32,
    pub column: u32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorEditableExport {
    pub target: EditorEditableTarget,
    pub report: ExportReport,
    pub human_summary: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorExportError {
    Session(EditorError),
    Report(String),
    Blocked {
        target: EditorEditableTarget,
        blockers: u64,
    },
    Projection {
        target: EditorEditableTarget,
        message: String,
    },
    Write {
        target: EditorEditableTarget,
        message: String,
    },
}

impl fmt::Display for EditorExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session(error) => write!(formatter, "{error}"),
            Self::Report(message) => write!(formatter, "could not build export report: {message}"),
            Self::Blocked { target, blockers } => write!(
                formatter,
                "{target} export is blocked by {blockers} required semantic losses"
            ),
            Self::Projection { target, message } => {
                write!(formatter, "{target} semantic projection failed: {message}")
            }
            Self::Write { target, message } => {
                write!(
                    formatter,
                    "{target} package serialization failed: {message}"
                )
            }
        }
    }
}

impl std::error::Error for EditorExportError {}

#[derive(Debug, Clone)]
pub struct EditorSession {
    source_hash: Sha256Digest,
    graph: PubResolvedGraph,
    replacement_assets: BTreeMap<Sha256Digest, EditorReplacementAsset>,
    image_replacements: BTreeMap<NodeId, Sha256Digest>,
    undo: Vec<EditOperation>,
    redo: Vec<EditOperation>,
}

impl EditorSession {
    pub fn new(graph: PubResolvedGraph) -> Result<Self, EditorError> {
        let source_hash = graph.source.source_hash;
        if graph.document.source_hash != source_hash {
            return Err(EditorError::SourceIdentityChanged);
        }

        Ok(Self {
            source_hash,
            graph,
            replacement_assets: BTreeMap::new(),
            image_replacements: BTreeMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
        })
    }

    pub fn source_hash(&self) -> Sha256Digest {
        self.source_hash
    }

    pub fn graph(&self) -> &PubResolvedGraph {
        &self.graph
    }

    pub fn operations(&self) -> &[EditOperation] {
        &self.undo
    }

    pub fn replacement_assets(
        &self,
    ) -> impl ExactSizeIterator<Item = &EditorReplacementAsset> + DoubleEndedIterator {
        self.replacement_assets.values()
    }

    pub fn image_replacement_for(&self, node_id: NodeId) -> Option<Sha256Digest> {
        self.image_replacements.get(&node_id).copied()
    }

    pub fn import_replacement_asset(
        &mut self,
        mime: impl Into<String>,
        bytes: Vec<u8>,
    ) -> Result<Sha256Digest, EditorAssetError> {
        let mime = mime.into();
        let asset = validated_editor_asset(mime, bytes)?;

        if let Some(existing) = self.replacement_assets.get(&asset.sha256) {
            if existing.mime != asset.mime {
                return Err(EditorAssetError::MimeConflict {
                    sha256: asset.sha256,
                    existing: existing.mime.clone(),
                    requested: asset.mime,
                });
            }
            return Ok(asset.sha256);
        }

        let sha256 = asset.sha256;
        self.replacement_assets.insert(sha256, asset);
        Ok(sha256)
    }

    pub fn project(&self) -> EditorProject {
        let schema_version = if self
            .undo
            .iter()
            .any(|operation| matches!(operation, EditOperation::ResizeNode { .. }))
        {
            EDITOR_PROJECT_VERSION_V0_5
        } else if self
            .undo
            .iter()
            .any(|operation| matches!(operation, EditOperation::MoveNode { .. }))
        {
            EDITOR_PROJECT_VERSION_V0_4
        } else if self
            .undo
            .iter()
            .any(|operation| matches!(operation, EditOperation::ReplaceImage { .. }))
        {
            EDITOR_PROJECT_VERSION_V0_3
        } else {
            EDITOR_PROJECT_VERSION_V0_2
        };

        EditorProject {
            schema_version: schema_version.into(),
            source_hash: self.source_hash,
            assets: self.project_asset_metadata(),
            operations: self.undo.clone(),
        }
    }

    pub fn persistence_requirements(&self) -> Vec<PersistenceRequirement> {
        self.project().persistence_requirements()
    }

    pub fn assess_mature_0x2c_pub_persistence(
        &self,
        writer: &WriterCapabilityManifest,
    ) -> Result<PersistenceCompatibilityAssessment, PersistenceCompatibilityError> {
        assess_mature_0x2c_pub_project_persistence(&self.project(), writer)
    }

    fn project_asset_metadata(&self) -> Vec<EditorProjectAsset> {
        self.replacement_assets
            .values()
            .map(|asset| EditorProjectAsset {
                sha256: asset.sha256,
                mime: asset.mime.clone(),
                byte_len: u64::try_from(asset.bytes.len())
                    .expect("validated editor asset length must fit u64"),
            })
            .collect()
    }

    /// Replays a persisted editor project through the current capability-gated
    /// mutation APIs.
    ///
    /// Replay is transactional: this session is changed only if every
    /// operation is still accepted and regenerates the exact canonical
    /// operation recorded in the project.
    pub fn apply_project(&mut self, project: &EditorProject) -> Result<(), EditorProjectError> {
        self.apply_project_with_assets(project, &BTreeMap::new())
    }

    pub fn apply_project_with_assets(
        &mut self,
        project: &EditorProject,
        asset_bytes: &BTreeMap<Sha256Digest, Vec<u8>>,
    ) -> Result<(), EditorProjectError> {
        self.validate_source_identity()
            .map_err(EditorProjectError::Session)?;

        if project.schema_version != EDITOR_PROJECT_VERSION_V0_1
            && project.schema_version != EDITOR_PROJECT_VERSION_V0_2
            && project.schema_version != EDITOR_PROJECT_VERSION_V0_3
            && project.schema_version != EDITOR_PROJECT_VERSION_V0_4
            && project.schema_version != EDITOR_PROJECT_VERSION_V0_5
        {
            return Err(EditorProjectError::UnsupportedSchema {
                found: project.schema_version.clone(),
            });
        }
        if project.schema_version == EDITOR_PROJECT_VERSION_V0_1 && !project.assets.is_empty() {
            return Err(EditorProjectError::LegacyProjectCarriesAssets);
        }
        if project.schema_version == EDITOR_PROJECT_VERSION_V0_1
            || project.schema_version == EDITOR_PROJECT_VERSION_V0_2
        {
            if let Some(index) = project
                .operations
                .iter()
                .position(|operation| matches!(operation, EditOperation::ReplaceImage { .. }))
            {
                return Err(EditorProjectError::LegacyProjectCarriesImageOperation { index });
            }
        }
        if project.schema_version != EDITOR_PROJECT_VERSION_V0_4
            && project.schema_version != EDITOR_PROJECT_VERSION_V0_5
        {
            if let Some(index) = project
                .operations
                .iter()
                .position(|operation| matches!(operation, EditOperation::MoveNode { .. }))
            {
                return Err(EditorProjectError::LegacyProjectCarriesGeometryOperation { index });
            }
        }
        if project.schema_version != EDITOR_PROJECT_VERSION_V0_5 {
            if let Some(index) = project
                .operations
                .iter()
                .position(|operation| matches!(operation, EditOperation::ResizeNode { .. }))
            {
                return Err(EditorProjectError::LegacyProjectCarriesResizeOperation { index });
            }
        }
        if project.source_hash != self.source_hash {
            return Err(EditorProjectError::SourceHashMismatch {
                expected: self.source_hash,
                found: project.source_hash,
            });
        }
        if !self.undo.is_empty()
            || !self.redo.is_empty()
            || !self.replacement_assets.is_empty()
            || !self.image_replacements.is_empty()
        {
            return Err(EditorProjectError::SessionNotEmpty);
        }

        let mut candidate = self.clone();
        for (index, metadata) in project.assets.iter().enumerate() {
            let bytes = asset_bytes
                .get(&metadata.sha256)
                .ok_or(EditorProjectError::MissingAssetBytes {
                    sha256: metadata.sha256,
                })?
                .clone();
            let found_len = u64::try_from(bytes.len()).map_err(|_| EditorProjectError::Asset {
                index,
                error: EditorAssetError::ByteLengthOverflow,
            })?;
            if found_len != metadata.byte_len {
                return Err(EditorProjectError::AssetLengthMismatch {
                    sha256: metadata.sha256,
                    expected: metadata.byte_len,
                    found: found_len,
                });
            }

            let asset = validated_editor_asset(metadata.mime.clone(), bytes)
                .map_err(|error| EditorProjectError::Asset { index, error })?;
            if asset.sha256 != metadata.sha256 {
                return Err(EditorProjectError::AssetHashMismatch {
                    expected: metadata.sha256,
                    found: asset.sha256,
                });
            }
            candidate.replacement_assets.insert(metadata.sha256, asset);
        }

        if candidate.project_asset_metadata() != project.assets {
            return Err(EditorProjectError::AssetMetadataNonCanonical);
        }

        for (index, expected) in project.operations.iter().enumerate() {
            let actual = replay_canonical_operation(&mut candidate, expected, index)?;
            if &actual != expected {
                return Err(EditorProjectError::OperationMismatch { index });
            }
        }

        *self = candidate;
        Ok(())
    }

    pub fn preview_editable_export(
        &self,
        target: EditorEditableTarget,
        source_label: impl Into<String>,
    ) -> Result<EditorEditableExportPreview, EditorExportError> {
        let (report, human_summary, _) =
            self.build_editable_export_plan(target, source_label.into())?;
        Ok(EditorEditableExportPreview {
            target,
            report,
            human_summary,
        })
    }

    pub fn export_editable(
        &self,
        target: EditorEditableTarget,
        source_label: impl Into<String>,
    ) -> Result<EditorEditableExport, EditorExportError> {
        let (report, human_summary, plan) =
            self.build_editable_export_plan(target, source_label.into())?;
        if !report.can_serialize {
            return Err(EditorExportError::Blocked {
                target,
                blockers: report.counts.blocking,
            });
        }

        let bytes = match target {
            EditorEditableTarget::Idml => {
                let projection_plan =
                    idml_base_projection_plan(&plan, self.image_replacements.keys().copied());
                let mut package = project_resolved_graph_to_idml(
                    &projection_plan,
                    &self.graph,
                    &IdmlWireProfile::legacy_dom_7(),
                    frame_from_payload,
                )
                .map_err(|error| EditorExportError::Projection {
                    target,
                    message: error.to_string(),
                })?;
                let placements = self.idml_replacement_placements()?;
                add_embedded_images_to_idml(&plan, &mut package, &placements).map_err(|error| {
                    EditorExportError::Projection {
                        target,
                        message: error.to_string(),
                    }
                })?;
                write_idml_ucf(&package).map_err(|error| EditorExportError::Write {
                    target,
                    message: error.to_string(),
                })?
            }
            EditorEditableTarget::Odg => {
                let package = project_resolved_graph_to_odg(&plan, &self.graph, frame_from_payload)
                    .map_err(|error| EditorExportError::Projection {
                        target,
                        message: error.to_string(),
                    })?;
                write_odg(&package).map_err(|error| EditorExportError::Write {
                    target,
                    message: error.to_string(),
                })?
            }
        };

        Ok(EditorEditableExport {
            target,
            report,
            human_summary,
            bytes,
        })
    }

    fn build_editable_export_plan(
        &self,
        target: EditorEditableTarget,
        source_label: String,
    ) -> Result<(ExportReport, String, ExportPlan), EditorExportError> {
        self.validate_source_identity()
            .map_err(EditorExportError::Session)?;
        let plan = editable_export_plan(target, &self.graph, &self.image_replacements);
        let report = build_export_report(
            &plan,
            ExportReportSource {
                label: source_label,
                source_hash: Some(self.source_hash),
            },
        )
        .map_err(|error| EditorExportError::Report(error.to_string()))?;
        let human_summary = render_human_summary(&report);
        Ok((report, human_summary, plan))
    }

    fn idml_replacement_placements(
        &self,
    ) -> Result<Vec<IdmlEmbeddedImagePlacement>, EditorExportError> {
        let target = EditorEditableTarget::Idml;
        let mut placements = Vec::with_capacity(self.image_replacements.len());

        for (node_id, asset_sha) in &self.image_replacements {
            let node =
                self.graph
                    .nodes
                    .get(node_id)
                    .ok_or_else(|| EditorExportError::Projection {
                        target,
                        message: format!(
                            "replacement image node {} is missing from the resolved graph",
                            node_id.as_canonical()
                        ),
                    })?;
            let asset = self.replacement_assets.get(asset_sha).ok_or_else(|| {
                EditorExportError::Projection {
                    target,
                    message: format!(
                        "replacement image asset {asset_sha} is missing from the editor session"
                    ),
                }
            })?;
            let (page_id, page) = self
                .graph
                .pages
                .iter()
                .find(|(page_id, _)| page_id.into_canonical() == node.header.parent_id)
                .ok_or_else(|| EditorExportError::Projection {
                    target,
                    message: format!(
                        "replacement image node {} is not directly authored on a page",
                        node_id.as_canonical()
                    ),
                })?;

            placements.push(IdmlEmbeddedImagePlacement {
                node_id: *node_id,
                page_id: *page_id,
                page_size: page.size,
                resource_id: replacement_asset_resource_id(*asset_sha),
                frame_bounds: node.header.bounds,
                mime: asset.mime.clone(),
                bytes: asset.bytes.clone(),
            });
        }

        Ok(placements)
    }

    pub fn can_replace_story_text(&self, story_id: StoryId) -> Result<(), EditorError> {
        self.validate_source_identity()?;

        let story = self
            .graph
            .stories
            .get(&story_id)
            .ok_or(EditorError::MissingStory { story_id })?;

        if !story.paragraphs.is_empty()
            || !story.runs.is_empty()
            || !story.fields.is_empty()
            || !story.hyperlinks.is_empty()
        {
            return Err(EditorError::RichStoryUnsupported { story_id });
        }

        if self.graph.nodes.values().any(|node| {
            node.payload
                .table_story
                .as_ref()
                .is_some_and(|owner| owner.story_id == Some(story_id))
                || node
                    .payload
                    .table
                    .as_ref()
                    .is_some_and(|table| table.story_id == Some(story_id))
        }) {
            return Err(EditorError::TableStoryUnsupported { story_id });
        }

        let frames = self
            .graph
            .nodes
            .iter()
            .filter_map(|(node_id, node)| {
                let frame = node.payload.story_frame.as_ref()?;
                (frame.story_id == Some(story_id)).then_some(StoryFrame {
                    story_id,
                    frame_id: *node_id,
                    ordinal: frame.ordinal,
                    previous: frame.previous_frame,
                    next: frame.next_frame,
                })
            })
            .collect::<Vec<StoryFrame<StoryId, NodeId>>>();

        if frames.is_empty() {
            return Err(EditorError::FrameCountUnsupported { story_id, found: 0 });
        }

        let topology_errors = validate_story_frames(&frames);
        if !topology_errors.is_empty() {
            return Err(EditorError::FrameTopologyUnsupported {
                story_id,
                errors: topology_errors.len(),
            });
        }

        Ok(())
    }

    pub fn editable_table_cells_for_story(
        &self,
        story_id: StoryId,
    ) -> Vec<EditorEditableTableCell> {
        let mut result = Vec::new();

        for (node_id, node) in &self.graph.nodes {
            let Some(table) = node.payload.table.as_ref() else {
                continue;
            };
            if table.story_id != Some(story_id) {
                continue;
            }
            let Some(story) = self.graph.stories.get(&story_id) else {
                continue;
            };
            let Ok(cells) = materialize_bounded_simple_table_cells(table, story) else {
                continue;
            };

            for cell in cells {
                if self.can_replace_table_cell_text(*node_id, cell.id).is_err() {
                    continue;
                }
                result.push(EditorEditableTableCell {
                    node_id: *node_id,
                    story_id,
                    cell_id: cell.id,
                    row: cell.address.row,
                    column: cell.address.column,
                    text: cell.text,
                });
            }
        }

        result.sort_by_key(|cell| (cell.row, cell.column, cell.cell_id));
        result
    }

    pub fn can_replace_table_cell_text(
        &self,
        node_id: NodeId,
        cell_id: TableCellId,
    ) -> Result<(), EditorError> {
        self.validate_source_identity()?;

        let node = self
            .graph
            .nodes
            .get(&node_id)
            .ok_or(EditorError::TableEditUnsupported { node_id })?;
        let table = node
            .payload
            .table
            .as_ref()
            .ok_or(EditorError::TableEditUnsupported { node_id })?;
        let story_id = table
            .story_id
            .ok_or(EditorError::TableEditUnsupported { node_id })?;
        let story = self
            .graph
            .stories
            .get(&story_id)
            .ok_or(EditorError::TableEditUnsupported { node_id })?;

        let simple = table
            .simple_table
            .as_ref()
            .ok_or(EditorError::TableEditUnsupported { node_id })?;
        if !simple.cells_are_row_major()
            || !story.paragraphs.is_empty()
            || !story.runs.is_empty()
            || !story.fields.is_empty()
            || !story.hyperlinks.is_empty()
        {
            return Err(EditorError::TableEditUnsupported { node_id });
        }
        let table_owner_count = self
            .graph
            .nodes
            .values()
            .filter(|candidate| {
                candidate
                    .payload
                    .table_story
                    .as_ref()
                    .is_some_and(|owner| owner.story_id == Some(story_id))
                    || candidate
                        .payload
                        .table
                        .as_ref()
                        .is_some_and(|candidate_table| candidate_table.story_id == Some(story_id))
            })
            .count();
        if table_owner_count != 1 {
            return Err(EditorError::TableEditUnsupported { node_id });
        }

        let cells = materialize_bounded_simple_table_cells(table, story)
            .map_err(|_| EditorError::TableEditUnsupported { node_id })?;
        if !cells.iter().any(|cell| cell.id == cell_id) {
            return Err(EditorError::MissingTableCell { node_id, cell_id });
        }

        let (roundtrip_story, roundtrip_ranges) = rebuild_simple_table_story(table, &cells)
            .map_err(|_| EditorError::TableEditUnsupported { node_id })?;
        if roundtrip_story != story.text || roundtrip_ranges != snapshot_table_ranges(table) {
            return Err(EditorError::TableEditUnsupported { node_id });
        }

        Ok(())
    }

    pub fn replace_table_cell_text(
        &mut self,
        node_id: NodeId,
        cell_id: TableCellId,
        replacement: impl Into<String>,
    ) -> Result<EditOperation, EditorError> {
        self.can_replace_table_cell_text(node_id, cell_id)?;

        let replacement = replacement.into();
        let node = self
            .graph
            .nodes
            .get(&node_id)
            .expect("capability check verified table node");
        let table = node
            .payload
            .table
            .as_ref()
            .expect("capability check verified table payload");
        let story_id = table
            .story_id
            .expect("capability check verified table Story");
        let story = self
            .graph
            .stories
            .get(&story_id)
            .expect("capability check verified table Story presence");
        let mut cells = materialize_bounded_simple_table_cells(table, story)
            .expect("capability check verified table cell materialization");

        let target = cells
            .iter_mut()
            .find(|cell| cell.id == cell_id)
            .expect("capability check verified target cell");
        if target.text == replacement {
            return Err(EditorError::TableCellNoChange { node_id, cell_id });
        }
        target.text = replacement;

        let before_story = story.text.clone();
        let before_ranges = snapshot_table_ranges(table);
        let (after_story, after_ranges) = rebuild_simple_table_story(table, &cells)
            .map_err(|_| EditorError::TableEditUnsupported { node_id })?;

        let operation = EditOperation::ReplaceTableCellText {
            node_id,
            story_id,
            cell_id,
            before_story,
            after_story,
            before_ranges,
            after_ranges,
        };

        apply_forward(&mut self.graph, &operation)?;
        self.undo.push(operation.clone());
        self.redo.clear();
        self.validate_source_identity()?;
        Ok(operation)
    }

    pub fn can_replace_image(
        &self,
        node_id: NodeId,
        replacement_asset: Sha256Digest,
    ) -> Result<(), EditorError> {
        self.validate_source_identity()?;

        if !self.replacement_assets.contains_key(&replacement_asset) {
            return Err(EditorError::MissingReplacementAsset {
                sha256: replacement_asset,
            });
        }

        let node = self
            .graph
            .nodes
            .get(&node_id)
            .ok_or(EditorError::ImageReplaceUnsupported { node_id })?;
        if node.payload.image_slot.is_none() || node.payload.explicit_image_crop.is_some() {
            return Err(EditorError::ImageReplaceUnsupported { node_id });
        }
        if node.header.bounds.width.get() <= 0
            || node.header.bounds.height.get() <= 0
            || node.header.bounds.right().is_none()
            || node.header.bounds.bottom().is_none()
        {
            return Err(EditorError::ImageReplaceUnsupported { node_id });
        }
        if !self
            .graph
            .pages
            .keys()
            .any(|page_id| page_id.into_canonical() == node.header.parent_id)
        {
            return Err(EditorError::ImageReplaceUnsupported { node_id });
        }

        Ok(())
    }

    pub fn replace_image(
        &mut self,
        node_id: NodeId,
        replacement_asset: Sha256Digest,
    ) -> Result<EditOperation, EditorError> {
        self.can_replace_image(node_id, replacement_asset)?;

        let before_asset = self.image_replacement_for(node_id);
        if before_asset == Some(replacement_asset) {
            return Err(EditorError::ImageReplacementNoChange {
                node_id,
                sha256: replacement_asset,
            });
        }

        let operation = EditOperation::ReplaceImage {
            node_id,
            before_asset,
            after_asset: replacement_asset,
        };
        apply_image_forward(&mut self.image_replacements, &operation)?;
        self.undo.push(operation.clone());
        self.redo.clear();
        self.validate_source_identity()?;
        Ok(operation)
    }

    pub fn can_move_node_to(
        &self,
        node_id: NodeId,
        x: LengthEmu,
        y: LengthEmu,
    ) -> Result<(), EditorError> {
        self.validate_source_identity()?;

        let node = self
            .graph
            .nodes
            .get(&node_id)
            .ok_or(EditorError::NodeMoveUnsupported { node_id })?;
        if node.header.bounds.width.get() <= 0
            || node.header.bounds.height.get() <= 0
            || node.header.bounds.right().is_none()
            || node.header.bounds.bottom().is_none()
            || node.header.transform != pub_model::Affine2D::identity()
        {
            return Err(EditorError::NodeMoveUnsupported { node_id });
        }
        if !self
            .graph
            .pages
            .keys()
            .any(|page_id| page_id.into_canonical() == node.header.parent_id)
        {
            return Err(EditorError::NodeMoveUnsupported { node_id });
        }

        let candidate = RectEmu::new(x, y, node.header.bounds.width, node.header.bounds.height);
        if candidate.right().is_none() || candidate.bottom().is_none() {
            return Err(EditorError::NodeMoveOverflow { node_id });
        }

        Ok(())
    }

    pub fn move_node_to(
        &mut self,
        node_id: NodeId,
        x: LengthEmu,
        y: LengthEmu,
    ) -> Result<EditOperation, EditorError> {
        self.can_move_node_to(node_id, x, y)?;

        let before = self
            .graph
            .nodes
            .get(&node_id)
            .expect("capability check verified move node")
            .header
            .bounds;
        let after = RectEmu::new(x, y, before.width, before.height);
        if before == after {
            return Err(EditorError::NodeMoveNoChange { node_id });
        }

        let operation = EditOperation::MoveNode {
            node_id,
            before,
            after,
        };
        apply_forward(&mut self.graph, &operation)?;
        self.undo.push(operation.clone());
        self.redo.clear();
        self.validate_source_identity()?;
        Ok(operation)
    }

    pub fn can_resize_node(&self, node_id: NodeId) -> Result<(), EditorError> {
        self.validate_source_identity()?;

        let node = self
            .graph
            .nodes
            .get(&node_id)
            .ok_or(EditorError::NodeResizeUnsupported { node_id })?;
        let before = node.header.bounds;
        if before.width.get() <= 0
            || before.height.get() <= 0
            || before.right().is_none()
            || before.bottom().is_none()
            || node.header.transform != pub_model::Affine2D::identity()
        {
            return Err(EditorError::NodeResizeUnsupported { node_id });
        }
        if !self
            .graph
            .pages
            .keys()
            .any(|page_id| page_id.into_canonical() == node.header.parent_id)
        {
            return Err(EditorError::NodeResizeUnsupported { node_id });
        }

        Ok(())
    }

    pub fn can_resize_node_to(
        &self,
        node_id: NodeId,
        bounds: RectEmu,
    ) -> Result<(), EditorError> {
        self.can_resize_node(node_id)?;

        let before = self
            .graph
            .nodes
            .get(&node_id)
            .expect("target capability check verified resize node")
            .header
            .bounds;
        if bounds.width.get() <= 0 || bounds.height.get() <= 0 {
            return Err(EditorError::NodeResizeNonPositive { node_id });
        }
        if bounds.right().is_none() || bounds.bottom().is_none() {
            return Err(EditorError::NodeResizeOverflow { node_id });
        }
        if before == bounds {
            return Err(EditorError::NodeResizeNoChange { node_id });
        }
        if before.width == bounds.width && before.height == bounds.height {
            return Err(EditorError::NodeResizeNoSizeChange { node_id });
        }

        Ok(())
    }

    pub fn resize_node_to(
        &mut self,
        node_id: NodeId,
        bounds: RectEmu,
    ) -> Result<EditOperation, EditorError> {
        self.can_resize_node_to(node_id, bounds)?;

        let before = self
            .graph
            .nodes
            .get(&node_id)
            .expect("capability check verified resize node")
            .header
            .bounds;
        let operation = EditOperation::ResizeNode {
            node_id,
            before,
            after: bounds,
        };
        apply_forward(&mut self.graph, &operation)?;
        self.undo.push(operation.clone());
        self.redo.clear();
        self.validate_source_identity()?;
        Ok(operation)
    }

    pub fn replace_story_range(
        &mut self,
        story_id: StoryId,
        start_scalar: u32,
        end_scalar: u32,
        expected_before: impl Into<String>,
        replacement_text: impl Into<String>,
    ) -> Result<EditOperation, EditorError> {
        self.can_replace_story_text(story_id)?;

        let expected_before = expected_before.into();
        let replacement_text = replacement_text.into();
        let before = self
            .graph
            .stories
            .get(&story_id)
            .expect("capability check verified story presence")
            .text
            .clone();

        let after = replace_scalar_range_text(
            &before,
            start_scalar,
            end_scalar,
            &expected_before,
            &replacement_text,
        )
        .ok_or(EditorError::StaleOperation { story_id })?;

        if before == after {
            return Err(EditorError::NoChange { story_id });
        }

        let operation = EditOperation::ReplaceStoryRange {
            story_id,
            start_scalar,
            end_scalar,
            expected_before,
            replacement_text,
            before_story_state_id: story_state_id_v1(story_id, &before),
            after_story_state_id: story_state_id_v1(story_id, &after),
        };

        apply_forward(&mut self.graph, &operation)?;
        self.undo.push(operation.clone());
        self.redo.clear();
        self.validate_source_identity()?;
        Ok(operation)
    }

    pub fn replace_story_text(
        &mut self,
        story_id: StoryId,
        replacement: impl Into<String>,
    ) -> Result<EditOperation, EditorError> {
        self.can_replace_story_text(story_id)?;
        let before = self
            .graph
            .stories
            .get(&story_id)
            .expect("capability check verified story presence")
            .text
            .clone();
        let scalar_len = u32::try_from(before.chars().count())
            .map_err(|_| EditorError::StaleOperation { story_id })?;
        self.replace_story_range(story_id, 0, scalar_len, before, replacement)
    }

    pub fn undo(&mut self) -> Result<&EditOperation, EditorError> {
        let operation = self.undo.pop().ok_or(EditorError::NothingToUndo)?;
        if matches!(operation, EditOperation::ReplaceImage { .. }) {
            apply_image_inverse(&mut self.image_replacements, &operation)?;
        } else {
            apply_inverse(&mut self.graph, &operation)?;
        }
        self.redo.push(operation);
        self.validate_source_identity()?;
        Ok(self.redo.last().expect("just pushed undo operation"))
    }

    pub fn redo(&mut self) -> Result<&EditOperation, EditorError> {
        let operation = self.redo.pop().ok_or(EditorError::NothingToRedo)?;
        if matches!(operation, EditOperation::ReplaceImage { .. }) {
            apply_image_forward(&mut self.image_replacements, &operation)?;
        } else {
            apply_forward(&mut self.graph, &operation)?;
        }
        self.undo.push(operation);
        self.validate_source_identity()?;
        Ok(self.undo.last().expect("just pushed redo operation"))
    }

    fn validate_source_identity(&self) -> Result<(), EditorError> {
        if self.graph.source.source_hash != self.source_hash
            || self.graph.document.source_hash != self.source_hash
        {
            return Err(EditorError::SourceIdentityChanged);
        }
        Ok(())
    }
}

pub fn editor_asset_file_name(
    sha256: Sha256Digest,
    mime: &str,
) -> Result<String, EditorAssetError> {
    let extension = match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        other => {
            return Err(EditorAssetError::UnsupportedMime {
                mime: other.to_owned(),
            });
        }
    };
    Ok(format!("asset-{sha256}.{extension}"))
}

fn validated_editor_asset(
    mime: String,
    bytes: Vec<u8>,
) -> Result<EditorReplacementAsset, EditorAssetError> {
    if bytes.is_empty() {
        return Err(EditorAssetError::EmptyBytes);
    }
    u64::try_from(bytes.len()).map_err(|_| EditorAssetError::ByteLengthOverflow)?;

    let signature_matches = match mime.as_str() {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        other => {
            return Err(EditorAssetError::UnsupportedMime {
                mime: other.to_owned(),
            });
        }
    };
    if !signature_matches {
        return Err(EditorAssetError::SignatureMismatch { mime });
    }

    let digest = Sha256::digest(&bytes);
    let mut digest_bytes = [0_u8; 32];
    digest_bytes.copy_from_slice(&digest);
    Ok(EditorReplacementAsset {
        sha256: Sha256Digest::from_bytes(digest_bytes),
        mime,
        bytes,
    })
}

fn replay_canonical_operation(
    session: &mut EditorSession,
    expected: &EditOperation,
    index: usize,
) -> Result<EditOperation, EditorProjectError> {
    match expected {
        EditOperation::ReplaceStoryRange {
            story_id,
            start_scalar,
            end_scalar,
            expected_before,
            replacement_text,
            ..
        } => session
            .replace_story_range(
                *story_id,
                *start_scalar,
                *end_scalar,
                expected_before.clone(),
                replacement_text.clone(),
            )
            .map_err(|error| EditorProjectError::Operation { index, error }),
        EditOperation::ReplaceStoryText {
            story_id, after, ..
        } => session
            .replace_story_text(*story_id, after.clone())
            .map_err(|error| EditorProjectError::Operation { index, error }),
        EditOperation::ReplaceTableCellText {
            node_id,
            story_id,
            cell_id,
            ..
        } => {
            // Materialize only a temporary expected-after graph to recover the
            // target cell's intended text. The real candidate session is then
            // mutated through the public capability-gated table API, and the
            // generated canonical operation must exactly match the persisted one.
            let mut expected_after_graph = session.graph.clone();
            apply_forward(&mut expected_after_graph, expected)
                .map_err(|error| EditorProjectError::Operation { index, error })?;

            let replacement = expected_after_graph
                .nodes
                .get(node_id)
                .and_then(|node| node.payload.table.as_ref())
                .filter(|table| table.story_id == Some(*story_id))
                .and_then(|table| {
                    let story = expected_after_graph.stories.get(story_id)?;
                    materialize_bounded_simple_table_cells(table, story)
                        .ok()?
                        .into_iter()
                        .find(|cell| cell.id == *cell_id)
                        .map(|cell| cell.text)
                })
                .ok_or(EditorProjectError::InvalidTableAfterState {
                    index,
                    node_id: *node_id,
                    cell_id: *cell_id,
                })?;

            session
                .replace_table_cell_text(*node_id, *cell_id, replacement)
                .map_err(|error| EditorProjectError::Operation { index, error })
        }
        EditOperation::ReplaceImage {
            node_id,
            after_asset,
            ..
        } => session
            .replace_image(*node_id, *after_asset)
            .map_err(|error| EditorProjectError::Operation { index, error }),
        EditOperation::MoveNode { node_id, after, .. } => session
            .move_node_to(*node_id, after.x, after.y)
            .map_err(|error| EditorProjectError::Operation { index, error }),
        EditOperation::ResizeNode { node_id, after, .. } => session
            .resize_node_to(*node_id, *after)
            .map_err(|error| EditorProjectError::Operation { index, error }),
    }
}

fn frame_from_payload(
    node_id: NodeId,
    payload: &PubResolvedNodePayload,
) -> Option<StoryFrame<StoryId, NodeId>> {
    let frame = payload.story_frame.as_ref()?;
    let story_id = frame.story_id?;

    Some(StoryFrame {
        story_id,
        frame_id: node_id,
        ordinal: frame.ordinal,
        previous: frame.previous_frame,
        next: frame.next_frame,
    })
}

fn editable_export_plan(
    target: EditorEditableTarget,
    graph: &PubResolvedGraph,
    image_replacements: &BTreeMap<NodeId, Sha256Digest>,
) -> ExportPlan {
    let mut features = BTreeMap::new();
    features.insert("page.geometry".into(), CapabilityLevel::Preserved);
    features.insert("story.text".into(), CapabilityLevel::Preserved);
    features.insert("story.linked_frames".into(), CapabilityLevel::Preserved);
    if target == EditorEditableTarget::Idml {
        features.insert(IMAGE_BYTES_FEATURE.into(), CapabilityLevel::Preserved);
        features.insert(
            IMAGE_FRAME_GEOMETRY_FEATURE.into(),
            CapabilityLevel::Preserved,
        );
        features.insert(
            IMAGE_CONTENT_TRANSFORM_FEATURE.into(),
            CapabilityLevel::Approximated,
        );
    }

    let manifest = TargetCapabilityManifest {
        target: TargetProfile {
            format: target.format().into(),
            adapter_version: target.adapter_version().into(),
            profile: "bounded-editable".into(),
            schema_fence: Some(target.schema_fence().into()),
        },
        features,
    };

    let mut requests = Vec::new();
    for page_id in &graph.document.pages {
        requests.push(SemanticFeatureRequest {
            feature: "page.geometry".into(),
            origin: Some(page_id.into_canonical()),
            property_path: Some("page.size".into()),
            require_preserved: true,
        });

        let page = graph
            .pages
            .get(page_id)
            .expect("resolved graph document page must be present");
        let authored = graph
            .nodes
            .iter()
            .filter_map(|(node_id, node)| {
                (node.header.parent_id == page_id.into_canonical()).then_some(*node_id)
            })
            .collect::<Vec<_>>();
        if page.children != authored {
            requests.push(SemanticFeatureRequest {
                feature: "page.object_order".into(),
                origin: Some(page_id.into_canonical()),
                property_path: Some("page.children".into()),
                require_preserved: false,
            });
        }
    }

    for story_id in graph.stories.keys() {
        requests.push(SemanticFeatureRequest {
            feature: "story.text".into(),
            origin: Some(story_id.into_canonical()),
            property_path: Some("story.text".into()),
            require_preserved: true,
        });
    }

    let page_ids = graph
        .document
        .pages
        .iter()
        .map(|page_id| page_id.into_canonical())
        .collect::<BTreeSet<_>>();
    let mut roots_per_story = BTreeMap::<StoryId, usize>::new();

    for (node_id, node) in &graph.nodes {
        if !page_ids.contains(&node.header.parent_id) {
            continue;
        }

        if let Some(frame) = frame_from_payload(*node_id, &node.payload) {
            if frame.previous.is_none() {
                *roots_per_story.entry(frame.story_id).or_default() += 1;
            }
            requests.push(SemanticFeatureRequest {
                feature: "story.linked_frames".into(),
                origin: Some(node_id.into_canonical()),
                property_path: Some("node.story_frame".into()),
                require_preserved: true,
            });
        } else if let Some(asset_sha) = image_replacements.get(node_id) {
            let resource_id = replacement_asset_resource_id(*asset_sha);
            requests.push(SemanticFeatureRequest {
                feature: IMAGE_BYTES_FEATURE.into(),
                origin: Some(resource_id.into_canonical()),
                property_path: Some("replacement_asset.bytes".into()),
                require_preserved: true,
            });
            requests.push(SemanticFeatureRequest {
                feature: IMAGE_FRAME_GEOMETRY_FEATURE.into(),
                origin: Some(node_id.into_canonical()),
                property_path: Some("node.bounds".into()),
                require_preserved: true,
            });
            requests.push(SemanticFeatureRequest {
                feature: IMAGE_CONTENT_TRANSFORM_FEATURE.into(),
                origin: Some(node_id.into_canonical()),
                property_path: Some("image.content_transform".into()),
                require_preserved: false,
            });
        } else {
            requests.push(SemanticFeatureRequest {
                feature: "node.unsupported".into(),
                origin: Some(node_id.into_canonical()),
                property_path: Some("node".into()),
                require_preserved: false,
            });
        }
    }

    for (story_id, roots) in roots_per_story {
        if roots > 1 {
            requests.push(SemanticFeatureRequest {
                feature: "story.shared_identity".into(),
                origin: Some(story_id.into_canonical()),
                property_path: Some("story.placements".into()),
                require_preserved: false,
            });
        }
    }

    plan_export(&manifest, requests)
}

fn replacement_asset_resource_id(sha256: Sha256Digest) -> ResourceId {
    ResourceId::from_canonical(
        derive_source_canonical_id(SourceDerivedIdInput {
            source_hash: &sha256,
            adapter_id: "pub-editor",
            source_object_key: "replacement-image",
            semantic_role: "cdm.resource.image",
        })
        .expect("editor replacement asset identity contract uses fixed valid identifiers"),
    )
}

fn idml_base_projection_plan(
    plan: &ExportPlan,
    externally_projected_nodes: impl IntoIterator<Item = NodeId>,
) -> ExportPlan {
    let mut projection = plan.clone();
    for node_id in externally_projected_nodes {
        projection.losses.push(LossItem {
            feature: "node.external_image_projection".into(),
            origin: Some(node_id.into_canonical()),
            property_path: Some("node".into()),
            kind: LossKind::Unsupported,
            severity: LossSeverity::Semantic,
            target: projection.target.clone(),
            reversible: true,
            code: "export.internal.external_image_projection".into(),
        });
    }
    projection
}

fn apply_forward(
    graph: &mut PubResolvedGraph,
    operation: &EditOperation,
) -> Result<(), EditorError> {
    match operation {
        EditOperation::ReplaceStoryRange {
            story_id,
            start_scalar,
            end_scalar,
            expected_before,
            replacement_text,
            before_story_state_id,
            after_story_state_id,
        } => {
            let story = graph
                .stories
                .get_mut(story_id)
                .ok_or(EditorError::MissingStory {
                    story_id: *story_id,
                })?;
            if story_state_id_v1(*story_id, &story.text) != *before_story_state_id {
                return Err(EditorError::StaleOperation {
                    story_id: *story_id,
                });
            }
            let after = replace_scalar_range_text(
                &story.text,
                *start_scalar,
                *end_scalar,
                expected_before,
                replacement_text,
            )
            .ok_or(EditorError::StaleOperation {
                story_id: *story_id,
            })?;
            if story_state_id_v1(*story_id, &after) != *after_story_state_id {
                return Err(EditorError::StaleOperation {
                    story_id: *story_id,
                });
            }
            story.text = after;
        }
        EditOperation::ReplaceStoryText {
            story_id,
            before,
            after,
        } => {
            let story = graph
                .stories
                .get_mut(story_id)
                .ok_or(EditorError::MissingStory {
                    story_id: *story_id,
                })?;
            if story.text != *before {
                return Err(EditorError::StaleOperation {
                    story_id: *story_id,
                });
            }
            story.text.clone_from(after);
        }
        EditOperation::ReplaceTableCellText {
            node_id,
            story_id,
            before_story,
            after_story,
            before_ranges,
            after_ranges,
            ..
        } => {
            apply_table_cell_state(
                graph,
                *node_id,
                *story_id,
                before_story,
                after_story,
                before_ranges,
                after_ranges,
            )?;
        }
        EditOperation::MoveNode {
            node_id,
            before,
            after,
        } => {
            let node = graph
                .nodes
                .get_mut(node_id)
                .ok_or(EditorError::NodeMoveUnsupported { node_id: *node_id })?;
            if node.header.bounds != *before {
                return Err(EditorError::StaleNodeMove { node_id: *node_id });
            }
            node.header.bounds = *after;
        }
        EditOperation::ResizeNode {
            node_id,
            before,
            after,
        } => {
            let node = graph
                .nodes
                .get_mut(node_id)
                .ok_or(EditorError::NodeResizeUnsupported { node_id: *node_id })?;
            if node.header.bounds != *before {
                return Err(EditorError::StaleNodeResize { node_id: *node_id });
            }
            node.header.bounds = *after;
        }
        EditOperation::ReplaceImage { .. } => {
            unreachable!("image replacements are applied to editor overlay state")
        }
    }
    Ok(())
}

fn apply_inverse(
    graph: &mut PubResolvedGraph,
    operation: &EditOperation,
) -> Result<(), EditorError> {
    match operation {
        EditOperation::ReplaceStoryRange {
            story_id,
            start_scalar,
            expected_before,
            replacement_text,
            before_story_state_id,
            after_story_state_id,
            ..
        } => {
            let story = graph
                .stories
                .get_mut(story_id)
                .ok_or(EditorError::MissingStory {
                    story_id: *story_id,
                })?;
            if story_state_id_v1(*story_id, &story.text) != *after_story_state_id {
                return Err(EditorError::StaleOperation {
                    story_id: *story_id,
                });
            }
            let replacement_end = start_scalar
                .checked_add(
                    u32::try_from(replacement_text.chars().count()).map_err(|_| {
                        EditorError::StaleOperation {
                            story_id: *story_id,
                        }
                    })?,
                )
                .ok_or(EditorError::StaleOperation {
                    story_id: *story_id,
                })?;
            let before = replace_scalar_range_text(
                &story.text,
                *start_scalar,
                replacement_end,
                replacement_text,
                expected_before,
            )
            .ok_or(EditorError::StaleOperation {
                story_id: *story_id,
            })?;
            if story_state_id_v1(*story_id, &before) != *before_story_state_id {
                return Err(EditorError::StaleOperation {
                    story_id: *story_id,
                });
            }
            story.text = before;
        }
        EditOperation::ReplaceStoryText {
            story_id,
            before,
            after,
        } => {
            let story = graph
                .stories
                .get_mut(story_id)
                .ok_or(EditorError::MissingStory {
                    story_id: *story_id,
                })?;
            if story.text != *after {
                return Err(EditorError::StaleOperation {
                    story_id: *story_id,
                });
            }
            story.text.clone_from(before);
        }
        EditOperation::ReplaceTableCellText {
            node_id,
            story_id,
            before_story,
            after_story,
            before_ranges,
            after_ranges,
            ..
        } => {
            apply_table_cell_state(
                graph,
                *node_id,
                *story_id,
                after_story,
                before_story,
                after_ranges,
                before_ranges,
            )?;
        }
        EditOperation::MoveNode {
            node_id,
            before,
            after,
        } => {
            let node = graph
                .nodes
                .get_mut(node_id)
                .ok_or(EditorError::NodeMoveUnsupported { node_id: *node_id })?;
            if node.header.bounds != *after {
                return Err(EditorError::StaleNodeMove { node_id: *node_id });
            }
            node.header.bounds = *before;
        }
        EditOperation::ResizeNode {
            node_id,
            before,
            after,
        } => {
            let node = graph
                .nodes
                .get_mut(node_id)
                .ok_or(EditorError::NodeResizeUnsupported { node_id: *node_id })?;
            if node.header.bounds != *after {
                return Err(EditorError::StaleNodeResize { node_id: *node_id });
            }
            node.header.bounds = *before;
        }
        EditOperation::ReplaceImage { .. } => {
            unreachable!("image replacements are applied to editor overlay state")
        }
    }
    Ok(())
}

fn apply_image_forward(
    replacements: &mut BTreeMap<NodeId, Sha256Digest>,
    operation: &EditOperation,
) -> Result<(), EditorError> {
    let EditOperation::ReplaceImage {
        node_id,
        before_asset,
        after_asset,
    } = operation
    else {
        unreachable!("only ReplaceImage reaches image overlay apply")
    };

    if replacements.get(node_id).copied() != *before_asset {
        return Err(EditorError::StaleImageOperation { node_id: *node_id });
    }
    replacements.insert(*node_id, *after_asset);
    Ok(())
}

fn apply_image_inverse(
    replacements: &mut BTreeMap<NodeId, Sha256Digest>,
    operation: &EditOperation,
) -> Result<(), EditorError> {
    let EditOperation::ReplaceImage {
        node_id,
        before_asset,
        after_asset,
    } = operation
    else {
        unreachable!("only ReplaceImage reaches image overlay inverse")
    };

    if replacements.get(node_id).copied() != Some(*after_asset) {
        return Err(EditorError::StaleImageOperation { node_id: *node_id });
    }
    if let Some(before_asset) = before_asset {
        replacements.insert(*node_id, *before_asset);
    } else {
        replacements.remove(node_id);
    }
    Ok(())
}

fn snapshot_table_ranges(table: &pub_reader::PubTableSource) -> Vec<TableCellRangeSnapshot> {
    table
        .cells
        .iter()
        .map(|cell| TableCellRangeSnapshot {
            cell_id: cell.id,
            utf16_start: cell.utf16_start,
            utf16_end: cell.utf16_end,
        })
        .collect()
}

fn rebuild_simple_table_story(
    table: &pub_reader::PubTableSource,
    cells: &[pub_reader::PubMaterializedTableCell],
) -> Result<(String, Vec<TableCellRangeSnapshot>), ()> {
    let simple = table.simple_table.as_ref().ok_or(())?;
    let mut ordered = simple.cells.clone();
    ordered.sort_by_key(|cell| (cell.address.row, cell.address.column, cell.id));

    let mut utf16 = Vec::<u16>::new();
    let mut ranges = Vec::with_capacity(ordered.len());

    for (index, semantic) in ordered.iter().enumerate() {
        let materialized = cells.iter().find(|cell| cell.id == semantic.id).ok_or(())?;
        let start = u32::try_from(utf16.len()).map_err(|_| ())?;
        if index > 0 {
            utf16.push(0x000D);
        }
        utf16.extend(materialized.text.encode_utf16());
        if index + 1 == ordered.len() {
            utf16.push(0x000D);
        }
        let end = u32::try_from(utf16.len()).map_err(|_| ())?;
        ranges.push(TableCellRangeSnapshot {
            cell_id: semantic.id,
            utf16_start: start,
            utf16_end: end,
        });
    }

    let text = String::from_utf16(&utf16).map_err(|_| ())?;
    Ok((text, ranges))
}

fn apply_table_cell_state(
    graph: &mut PubResolvedGraph,
    node_id: NodeId,
    story_id: StoryId,
    expected_story: &str,
    replacement_story: &str,
    expected_ranges: &[TableCellRangeSnapshot],
    replacement_ranges: &[TableCellRangeSnapshot],
) -> Result<(), EditorError> {
    let story = graph
        .stories
        .get_mut(&story_id)
        .ok_or(EditorError::MissingStory { story_id })?;
    if story.text != expected_story {
        return Err(EditorError::StaleOperation { story_id });
    }

    let node = graph
        .nodes
        .get_mut(&node_id)
        .ok_or(EditorError::TableEditUnsupported { node_id })?;
    let table = node
        .payload
        .table
        .as_mut()
        .ok_or(EditorError::TableEditUnsupported { node_id })?;
    if table.story_id != Some(story_id) {
        return Err(EditorError::StaleOperation { story_id });
    }

    if snapshot_table_ranges(table) != expected_ranges {
        return Err(EditorError::StaleOperation { story_id });
    }

    for range in replacement_ranges {
        let cell = table
            .cells
            .iter_mut()
            .find(|cell| cell.id == range.cell_id)
            .ok_or(EditorError::TableEditUnsupported { node_id })?;
        cell.utf16_start = range.utf16_start;
        cell.utf16_end = range.utf16_end;
    }
    story.text.clear();
    story.text.push_str(replacement_story);
    Ok(())
}
