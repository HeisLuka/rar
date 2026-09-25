//! Bounded Microsoft Publisher 2002+ source adapter.
//!
//! This crate is the first format-aware layer above the raw CFB/Contents/Quill/
//! Escher readers. It projects only evidence-backed mature-0x2C semantics into
//! pub-model::SourceGraph and emits diagnostics instead of inventing missing
//! geometry, page roles, or relation state.

use anyhow::{Context, Result, anyhow, bail};

mod asset_export;
mod assets;
mod failure_envelope;
mod failure_intake;
mod intake_protocol;
mod resolve;
mod structural_base;
mod table_bridge;

pub use asset_export::{
    PUB_ASSET_EXPORT_SCHEMA_V0_1, PUB_ASSET_MANIFEST_FILENAME, PubAssetExportBundle,
    PubAssetExportDiagnostic, PubAssetExportFile, PubAssetExportManifest,
    PubAssetExportManifestEntry, build_mature_0x2c_asset_export_bundle_from_bytes,
    build_pub_asset_export_bundle, pub_asset_manifest_json, write_pub_asset_export_bundle,
};
pub use assets::{
    PubAssetManifest, PubAssetManifestDiagnostic, PubAssetManifestEntry, PubAssetUse,
    PubImageAlpha, PubImageBlobRef, PubImageResource, PubImageResourceCatalog,
    PubImageResourceDiagnostic, build_pub_asset_manifest, build_pub_image_resource_catalog,
};
pub use failure_envelope::{
    CHAPTERA_FAILURE_ENVELOPE_SCHEMA_V1, CHAPTERA_READER_BUILD_ID, FailureArchitecture,
    FailureCoarseLocale, FailureCode, FailureContainerFamily, FailureEnvelope,
    FailureEnvelopeBuildError, FailureEnvelopeContext, FailureOsFamily, FailureParserStage,
    FailureSizeBucket, FailureTelemetryChoice, PUB_READER_ENGINE_BUILD_ID, build_failure_envelope,
};
pub use failure_intake::{
    FailureIntakeClass, FailureIntakeClassification, FailureIntakeConfidence, FailureIntakeReason,
    classify_failure_candidate,
};
pub use intake_protocol::{
    CHAPTERA_EXACT_FILE_CONSENT_V1, CHAPTERA_INTAKE_PROTOCOL_SCHEMA_V1,
    CHAPTERA_INTAKE_RETENTION_POLICY_V1, IntakeCapabilityRequest, IntakeClusterDisposition,
    IntakeDedupeDisposition, IntakeProtocolError, IntakeReceipt, build_intake_capability_request,
    exact_file_intake_eligible, validate_intake_capability_request, validate_intake_receipt,
};
use pub_contents::{
    CONTENTS_RAW_TYPE_STORY_CATALOG, Contents0x2cChunk, Contents0x2cChunkReference,
    DOCUMENT_PAGE_LIST_ID, RawContentsBlock, RawContentsBlockBody, parse_0x2c_header,
    parse_confirmed_0x2c_chunk, parse_confirmed_0x2c_trailer_root, parse_confirmed_chunk_reference,
    parse_confirmed_document_page_list, parse_confirmed_margins_page_extent,
    parse_confirmed_mature_story_catalog,
};
use pub_core::{RawSpan, StreamPath};
use pub_escher::{
    OFFICE_ART_PROPERTY_CROP_FROM_BOTTOM, OFFICE_ART_PROPERTY_CROP_FROM_LEFT,
    OFFICE_ART_PROPERTY_CROP_FROM_RIGHT, OFFICE_ART_PROPERTY_CROP_FROM_TOP,
    OFFICE_ART_PROPERTY_PIB, PUBLISHER_FIELD_SHAPE_ID, PUBLISHER_FIELD_XE, PUBLISHER_FIELD_XS,
    PUBLISHER_FIELD_YE, PUBLISHER_FIELD_YS, PublisherField, PublisherFieldRecord,
    SpContainerInventory, inspect_sp_containers,
};
use pub_model::{
    Affine2D, AuthorityClass, ByteRange, CanonicalId, Document, DocumentId, LengthEmu, Node,
    NodeHeader, NodeId, NodeKind, Page, PageId, ReadConfidence, RectEmu, Sha256Digest, Size2D,
    SourceDerivedIdInput, SourceDescriptor, SourceGraph, SourceRef, SourceRole, Story, StoryId,
    derive_source_canonical_id,
};
use pub_quill::{QuillMcldReadError, parse_bounded_mcld, parse_confirmed_story_catalog};
pub use resolve::{
    PUB_RESOLVER_VERSION_V1, PubResolveDiagnostic, PubResolvedGraph, PubResolvedGraphBuild,
    PubResolvedNodePayload, PubResolvedStoryFrame, resolve_pub_source_graph,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Seek, SeekFrom};
pub use structural_base::{
    PUB_STRUCTURAL_BASE_SCHEMA_V1, PubStructuralBaseCandidate, PubStructuralBaseManifest,
    PubStructuralBaseStreamDigest, build_mature_0x2c_structural_base_manifest,
    structural_base_manifest_json,
};
pub use table_bridge::{
    PubMaterializedTableCell, PubTableCellCoordinates, PubTableCellSource,
    PubTableLayoutMetricsSource, PubTableSource, PubTableStoryOwnershipSource, PubTableTextError,
    RAW_TYPE_TABLE, materialize_bounded_simple_table_cells,
};

pub const PUB_ADAPTER_ID: &str = "pub-rs";
pub const PUB_FORMAT_PROFILE_ID: &str = "pub-mature-0x2c-v0.1";

pub fn format_profile()
-> Result<pub_format_registry::FormatProfileEntry, pub_format_registry::RegistryError> {
    pub_format_registry::resolve(PUB_FORMAT_PROFILE_ID)
}
pub const CONTENTS_STREAM_PATH: &str = "/Contents";
pub const QUILL_STREAM_PATH: &str = "/Quill/QuillSub/CONTENTS";
pub const ESCHER_STREAM_PATH: &str = "/Escher/EscherStm";
pub const ESCHER_DELAY_STREAM_PATH: &str = "/Escher/EscherDelayStm";

const RAW_TYPE_SHAPE: u16 = 0x01;
const RAW_TYPE_GROUP: u16 = 0x30;
const RAW_TYPE_PAGE: u16 = 0x43;
const RAW_TYPE_DOCUMENT: u16 = 0x44;
const RAW_TYPE_MARGINS: u16 = 0x4C;
const RAW_TYPE_PAGE_LIST_SPECIAL: u16 = 0x59;

const OFFICEART_PROPERTY_ROTATION: u16 = 0x0004;
const OFFICEART_FSP_FLIP_H: u32 = 1 << 6;
const OFFICEART_FSP_FLIP_V: u32 = 1 << 7;

const FIELD_STORY_ID: u16 = 0x27;
const FIELD_FRAME_ORDINAL: u16 = 0x28;
const FIELD_PREVIOUS_FRAME: u16 = 0x36;
const FIELD_NEXT_FRAME: u16 = 0x37;

const ROLE_DOCUMENT: &str = "cdm.document";
const ROLE_PAGE: &str = "cdm.page";
const ROLE_NODE: &str = "cdm.node";
const ROLE_STORY: &str = "cdm.story";

pub type PubSourceGraph = SourceGraph<PubNodePayload, (), (), (), String>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubSourceGraphBuild {
    pub graph: PubSourceGraph,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<PubBridgeDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubNodePayload {
    pub contents_seq_num: u32,
    pub officeart_shape_type: Option<u16>,
    pub officeart_spid: Option<u32>,
    /// Exact one-based OfficeArt BStore identity from non-complex fBid pib.
    pub image_slot: Option<u32>,
    /// Bounded observation of explicit OfficeArt picture-crop properties on
    /// this image-bound shape. Raw scalar values are intentionally not
    /// reinterpreted as Publisher points or normalized crop geometry here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explicit_image_crop: Option<PubExplicitImageCropSource>,
    /// Explicit shape-local OfficeArt paint state only. Inherited drawing-group
    /// defaults are deliberately not materialized by this bounded adapter.
    pub explicit_paint: PubExplicitShapePaintSource,
    pub story_frame: Option<PubStoryFrameSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table_story: Option<PubTableStoryOwnershipSource>,
    pub table: Option<PubTableSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubExplicitImageCropSource {
    pub top_raw: Option<u32>,
    pub bottom_raw: Option<u32>,
    pub left_raw: Option<u32>,
    pub right_raw: Option<u32>,
    /// True when at least one crop property cannot be represented as one
    /// unambiguous non-complex scalar value. Presence remains explicit so
    /// callers must fail closed rather than misclassify the image as crop-free.
    pub ambiguous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PubExplicitShapePaintSource {
    pub fill: PubExplicitFillSource,
    pub line: PubExplicitLineSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PubExplicitFillSource {
    /// Explicit `fillType` is solid (MS-ODRAW 0x0180 == 0).
    pub solid: bool,
    /// Direct RGB only; scheme/system/palette COLORREF forms are not promoted.
    pub color_rgb: Option<[u8; 3]>,
    /// Set only when fUsefFilled is present in explicit 0x01BF.
    pub visible: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PubExplicitLineSource {
    /// Direct RGB only; scheme/system/palette COLORREF forms are not promoted.
    pub color_rgb: Option<[u8; 3]>,
    /// Explicit shape-local line width in EMU (MS-ODRAW 0x01CB).
    pub width_emu: Option<i64>,
    /// Set only when fUsefLine is present in explicit 0x01FF.
    pub visible: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubStoryFrameSource {
    pub text_id: u32,
    pub story_id: Option<StoryId>,
    /// Exact source state. Omission is not rewritten to effective ordinal zero
    /// until the resolver layer.
    pub explicit_ordinal: Option<u32>,
    pub previous_seq_num: Option<u32>,
    pub previous_frame: Option<NodeId>,
    pub next_seq_num: Option<u32>,
    pub next_frame: Option<NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PubStoryFrameCorrelation {
    pub quill_syid_count: usize,
    pub shape_chunk_count: usize,
    pub shape_chunks_with_opaque_tail: usize,
    pub decoded_field_27_syid_matches: usize,
    pub raw_scalar_syid_matches: usize,
    pub raw_field_27_syid_matches: usize,
    pub raw_field_27_syid_matches_in_opaque_tail: usize,
    pub shape_chunks_with_one_field_27_syid_match: usize,
    pub shape_chunks_with_multiple_field_27_syid_matches: usize,
    pub distinct_field_27_matched_syids: usize,
    pub shape_chunks_with_field_27_match_on_document_pages: usize,
    pub shape_chunks_with_field_27_match_outside_document_pages: usize,
    pub distinct_field_27_matched_syids_on_document_pages: usize,
    pub distinct_field_27_matched_syids_outside_document_pages: usize,
    pub table_chunk_count: usize,
    pub table_chunks_with_field_27_syid_match: usize,
    pub distinct_table_field_27_matched_syids: usize,
    pub table_chunks_with_field_27_match_on_document_pages: usize,
    pub table_chunks_with_field_27_match_outside_document_pages: usize,
    pub distinct_table_field_27_matched_syids_on_document_pages: usize,
    pub distinct_table_field_27_matched_syids_outside_document_pages: usize,
    pub distinct_story_syids_with_shape_or_table_identity: usize,
    pub distinct_story_syids_with_both_shape_and_table_identity: usize,
    pub distinct_story_syids_with_shape_only_identity: usize,
    pub distinct_story_syids_with_table_only_identity: usize,
    pub distinct_story_syids_without_shape_or_table_identity: usize,
    pub field_wire_match_counts: BTreeMap<String, usize>,
    pub field_27_parent_class_counts: BTreeMap<String, usize>,
    pub table_field_27_parent_class_counts: BTreeMap<String, usize>,
    pub multiframe_story_syid_count: usize,
    pub multiframe_shape_count: usize,
    pub multiframe_peer_seq_reference_counts: BTreeMap<String, usize>,
    pub multiframe_peer_seq_reference_story_counts: BTreeMap<String, usize>,
    pub multiframe_zero_based_ordinal_candidate_story_counts: BTreeMap<String, usize>,
    pub multiframe_one_based_ordinal_candidate_story_counts: BTreeMap<String, usize>,
    pub multiframe_unique_scalar_candidate_story_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PubGroupedStoryGeometryCorrelation {
    pub grouped_story_shape_count: usize,
    pub grouped_story_distinct_syid_count: usize,
    pub unique_child_escher_shape_count: usize,
    pub child_anchor_count: usize,
    pub parent_group_shape_link_count: usize,
    pub parent_group_contents_identity_match_count: usize,
    pub parent_group_fspgr_count: usize,
    pub parent_group_complete_client_anchor_count: usize,
    pub one_level_page_group_count: usize,
    pub nested_group_count: usize,
    pub complete_one_level_geometry_count: usize,
    pub complete_recursive_geometry_count: usize,
    pub max_group_depth: usize,
    pub group_depth_counts: BTreeMap<String, usize>,
    pub grouped_story_child_rotation_count: usize,
    pub grouped_story_child_flip_h_count: usize,
    pub grouped_story_child_flip_v_count: usize,
    pub grouped_story_group_ancestor_rotation_count: usize,
    pub grouped_story_group_ancestor_flip_h_count: usize,
    pub grouped_story_group_ancestor_flip_v_count: usize,
    pub grouped_story_group_ancestor_nontrivial_transform_count: usize,
    pub direct_page_story_shape_count: usize,
    pub direct_page_story_incomplete_client_anchor_count: usize,
    pub direct_page_story_incomplete_client_anchor_with_child_anchor_count: usize,
    pub direct_page_story_incomplete_client_anchor_with_fspgr_count: usize,
    pub direct_page_story_incomplete_client_anchor_missing_field_counts: BTreeMap<String, usize>,
    pub top_group_incomplete_client_anchor_missing_field_counts: BTreeMap<String, usize>,
    pub grouped_table_story_count: usize,
    pub grouped_table_complete_recursive_geometry_count: usize,
    pub grouped_table_nontrivial_transform_count: usize,
    pub grouped_table_max_group_depth: usize,
    pub grouped_table_incomplete_reason_counts: BTreeMap<String, usize>,
    pub incomplete_reason_counts: BTreeMap<String, usize>,
    pub recursive_incomplete_reason_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum PubBridgeDiagnostic {
    PageListSpecialEntry {
        handle: u32,
        raw_type: u16,
    },
    PageListUnknownEntry {
        handle: u32,
        raw_type: Option<u16>,
    },
    OpaqueContentsTail {
        seq_num: u32,
        byte_range: ByteRange,
    },
    MissingEscherGeometry {
        seq_num: u32,
    },
    AmbiguousEscherGeometry {
        seq_num: u32,
        matches: usize,
    },
    AmbiguousImageSlot {
        seq_num: u32,
        slots: Vec<u32>,
    },
    IncompleteEscherAnchor {
        seq_num: u32,
    },
    InvalidEscherAnchor {
        seq_num: u32,
    },
    MissingQuillStory {
        seq_num: u32,
        text_id: u32,
    },
    McldRecordCountMismatch {
        record_count: u32,
        record_id_count: u32,
    },
    EquivalentMarginsPageExtents {
        count: usize,
        width_emu: u32,
        height_emu: u32,
    },
    LinkedFrameNotMaterialized {
        seq_num: u32,
        target_seq_num: u32,
    },
    GroupedStoryProjected {
        seq_num: u32,
        depth: usize,
    },
    GroupedStoryProjectionUnavailable {
        seq_num: u32,
        reason: String,
    },
    GroupedTableProjected {
        seq_num: u32,
        depth: usize,
    },
    GroupedTableProjectionUnavailable {
        seq_num: u32,
        reason: String,
    },
    TableMissingRequiredField {
        seq_num: u32,
        field_id: u8,
    },
    TableMissingTcd {
        seq_num: u32,
        text_id: u32,
    },
    TableAmbiguousTcd {
        seq_num: u32,
        text_id: u32,
    },
    TableMissingCellsObject {
        seq_num: u32,
        cells_seq_num: u32,
    },
    TableCellsWrongRawType {
        seq_num: u32,
        cells_seq_num: u32,
        raw_type: Option<u16>,
    },
    TableCellsWrongParent {
        seq_num: u32,
        cells_seq_num: u32,
        parent_seq_num: Option<u32>,
    },
    TableCellCountMismatch {
        seq_num: u32,
        cells_records: usize,
        tcd_boundaries: usize,
    },
    TableCellTextRangeInvalid {
        seq_num: u32,
        stored_record_index: u32,
        previous_end: u32,
        end: u32,
        story_len: u32,
    },
    TableStoryLengthMismatch {
        seq_num: u32,
        tcd_last_end: u32,
        story_len: u32,
    },
    TableCellCoordinatesAmbiguous {
        seq_num: u32,
        stored_record_index: u32,
    },
    TableLayoutMetricsUnavailable {
        seq_num: u32,
        text_id: u32,
        layout_key: Option<u32>,
        reason: String,
    },
}

/// Canonical source key for a physical mature-0x2C Contents directory slot.
pub fn contents_object_key(seq_num: u32) -> String {
    format!("contents/0x2c/seq/{seq_num}")
}

/// Canonical source key for a persistent Quill SYID story identity.
pub fn quill_story_object_key(syid: u32) -> String {
    format!("quill/syid/{syid}")
}

pub fn derive_pub_document_id(source_hash: &Sha256Digest, seq_num: u32) -> Result<DocumentId> {
    Ok(DocumentId::from_canonical(derive_pub_id(
        source_hash,
        &contents_object_key(seq_num),
        ROLE_DOCUMENT,
    )?))
}

pub fn derive_pub_page_id(source_hash: &Sha256Digest, seq_num: u32) -> Result<PageId> {
    Ok(PageId::from_canonical(derive_pub_id(
        source_hash,
        &contents_object_key(seq_num),
        ROLE_PAGE,
    )?))
}

pub fn derive_pub_node_id(source_hash: &Sha256Digest, seq_num: u32) -> Result<NodeId> {
    Ok(NodeId::from_canonical(derive_pub_id(
        source_hash,
        &contents_object_key(seq_num),
        ROLE_NODE,
    )?))
}

pub fn derive_pub_story_id(source_hash: &Sha256Digest, syid: u32) -> Result<StoryId> {
    Ok(StoryId::from_canonical(derive_pub_id(
        source_hash,
        &quill_story_object_key(syid),
        ROLE_STORY,
    )?))
}

/// Builds a bounded mature-0x2C SourceGraph from one complete CFB file.
///
/// The caller supplies a verified SHA-256 digest. This function deliberately
/// does not pretend that a representation type is a hashing implementation.
pub fn build_mature_0x2c_source_graph<R: Read + Seek>(
    mut reader: R,
    source_hash: Sha256Digest,
) -> Result<PubSourceGraphBuild> {
    reader.seek(SeekFrom::Start(0))?;
    let mut pub_bytes = Vec::new();
    reader.read_to_end(&mut pub_bytes)?;

    let contents =
        pub_cfb::read_stream_reader(Cursor::new(pub_bytes.as_slice()), CONTENTS_STREAM_PATH)
            .with_context(|| format!("read {CONTENTS_STREAM_PATH}"))?;
    let quill = pub_cfb::read_stream_reader(Cursor::new(pub_bytes.as_slice()), QUILL_STREAM_PATH)
        .with_context(|| format!("read {QUILL_STREAM_PATH}"))?;
    let escher = pub_cfb::read_stream_reader(Cursor::new(pub_bytes.as_slice()), ESCHER_STREAM_PATH)
        .with_context(|| format!("read {ESCHER_STREAM_PATH}"))?;

    build_mature_0x2c_from_streams(source_hash, &contents, &quill, &escher)
}

/// Research-only correlation scan for the Story ↔ shape ownership bridge.
///
/// This function does not promote any raw candidate to semantics. It searches
/// mature-0x2C SHAPE chunks for known scalar wire forms whose u32 payload equals
/// a Quill SYID, separately records whether the currently expected field 0x27
/// occurs after the bounded chunk parser entered its opaque suffix, and
/// classifies matched SHAPE ownership by DOCUMENT PageList membership.
///
/// The result is suitable for corpus evidence gathering. Semantic readers must
/// continue to rely on explicitly decoded fields until a wire form is proven.
pub fn analyze_mature_0x2c_story_frame_candidates<R: Read + Seek>(
    mut reader: R,
) -> Result<PubStoryFrameCorrelation> {
    reader.seek(SeekFrom::Start(0))?;
    let mut pub_bytes = Vec::new();
    reader.read_to_end(&mut pub_bytes)?;

    let contents =
        pub_cfb::read_stream_reader(Cursor::new(pub_bytes.as_slice()), CONTENTS_STREAM_PATH)
            .with_context(|| format!("read {CONTENTS_STREAM_PATH}"))?;
    let quill = pub_cfb::read_stream_reader(Cursor::new(pub_bytes.as_slice()), QUILL_STREAM_PATH)
        .with_context(|| format!("read {QUILL_STREAM_PATH}"))?;

    analyze_mature_0x2c_story_frame_candidates_from_streams(&contents, &quill)
}

pub fn analyze_mature_0x2c_story_frame_candidates_from_streams(
    contents: &[u8],
    quill: &[u8],
) -> Result<PubStoryFrameCorrelation> {
    let contents_stream = StreamPath(CONTENTS_STREAM_PATH.into());
    let header = parse_0x2c_header(contents_stream.clone(), contents)
        .context("parse mature-0x2C Contents header for Story-frame correlation")?;
    let trailer = parse_confirmed_0x2c_trailer_root(contents, &header)
        .context("parse mature-0x2C Contents trailer for Story-frame correlation")?;
    let references = build_reference_index(contents, &trailer.directory)?;

    let document_reference =
        unique_reference_by_raw_type(&references, RAW_TYPE_DOCUMENT, "DOCUMENT")?;
    let document_chunk =
        chunk_for_reference(contents_stream.clone(), contents, document_reference)?;
    let page_list_block = unique_block(&document_chunk, DOCUMENT_PAGE_LIST_ID)?.clone();
    let page_list = parse_confirmed_document_page_list(contents, page_list_block)
        .context("parse DOCUMENT PageList for Story-frame correlation")?;
    let document_page_handles = page_list
        .entries
        .iter()
        .filter_map(|entry| {
            references
                .get(&entry.handle)
                .is_some_and(|reference| single_raw_type(reference) == Some(RAW_TYPE_PAGE))
                .then_some(entry.handle)
        })
        .collect::<BTreeSet<_>>();

    let quill_catalog = parse_confirmed_story_catalog(StreamPath(QUILL_STREAM_PATH.into()), quill)
        .context("parse Quill story catalog for Story-frame correlation")?;
    let syids = quill_catalog
        .stories
        .iter()
        .map(|story| story.syid.0)
        .collect::<BTreeSet<_>>();

    let mut result = PubStoryFrameCorrelation {
        quill_syid_count: syids.len(),
        ..Default::default()
    };
    let mut field_27_matched_syids = BTreeSet::new();
    let mut field_27_matched_syids_on_document_pages = BTreeSet::new();
    let mut field_27_matched_syids_outside_document_pages = BTreeSet::new();
    let mut shape_scalars_by_story =
        BTreeMap::<u32, Vec<(u32, BTreeMap<(u8, u8), Vec<u32>>)>>::new();

    for reference in references.values() {
        if single_raw_type(reference) != Some(RAW_TYPE_SHAPE) {
            continue;
        }

        result.shape_chunk_count += 1;

        let parent_seq = single_parent_seq(reference);
        let parent_is_document_page =
            parent_seq.is_some_and(|seq_num| document_page_handles.contains(&seq_num));
        let parent_class = match parent_seq {
            Some(_) if parent_is_document_page => "document_page/0x43".to_owned(),
            Some(seq_num) => references
                .get(&seq_num)
                .and_then(single_raw_type)
                .map(|raw_type| format!("outside_document_pages/0x{raw_type:02x}"))
                .unwrap_or_else(|| "outside_document_pages/unknown".to_owned()),
            None => "missing_or_ambiguous_parent".to_owned(),
        };

        let chunk = chunk_for_reference(contents_stream.clone(), contents, reference)?;
        if chunk.unsupported_tail.is_some() {
            result.shape_chunks_with_opaque_tail += 1;
        }

        let decoded_story_id = chunk.fields.iter().find_map(|field| {
            (field.id == FIELD_STORY_ID)
                .then(|| match &field.body {
                    RawContentsBlockBody::U16 { value, .. } => Some(u32::from(*value)),
                    RawContentsBlockBody::U32 { value, .. } => Some(*value),
                    _ => None,
                })
                .flatten()
        });
        if let Some(story_id) = decoded_story_id.filter(|value| syids.contains(value)) {
            let seq_num = seq_u32(reference.seq_num)?;
            let mut scalars = BTreeMap::<(u16, u8), Vec<u32>>::new();
            for field in &chunk.fields {
                let value = match &field.body {
                    RawContentsBlockBody::U16 { value, .. } => Some(u32::from(*value)),
                    RawContentsBlockBody::U32 { value, .. } => Some(*value),
                    _ => None,
                };
                if let Some(value) = value {
                    scalars
                        .entry((field.id, field.block_type))
                        .or_default()
                        .push(value);
                }
            }
            shape_scalars_by_story
                .entry(story_id)
                .or_default()
                .push((seq_num, scalars));
        }

        for field in &chunk.fields {
            if field.id != FIELD_STORY_ID {
                continue;
            }
            if let RawContentsBlockBody::U32 { value, .. } = &field.body
                && syids.contains(value)
            {
                result.decoded_field_27_syid_matches += 1;
            }
        }

        let start = usize::try_from(chunk.source.offset)
            .context("Story-frame correlation chunk offset does not fit usize")?;
        let len = usize::try_from(chunk.source.len)
            .context("Story-frame correlation chunk length does not fit usize")?;
        let end = start
            .checked_add(len)
            .filter(|end| *end <= contents.len())
            .context("Story-frame correlation chunk range is out of bounds")?;
        let raw = &contents[start..end];

        let tail_range = chunk.unsupported_tail.as_ref().and_then(|tail| {
            let tail_start = usize::try_from(tail.offset).ok()?;
            let tail_len = usize::try_from(tail.len).ok()?;
            let tail_end = tail_start.checked_add(tail_len)?;
            Some((tail_start, tail_end))
        });

        let mut field_27_matches_in_shape = 0_usize;
        if raw.len() >= 6 {
            for relative in 4..=raw.len() - 6 {
                let id = raw[relative];
                let wire = raw[relative + 1];
                if !matches!(
                    wire,
                    pub_contents::BLOCK_TYPE_U32
                        | pub_contents::BLOCK_TYPE_REFERENCE_U32
                        | pub_contents::BLOCK_TYPE_HANDLE_U32
                ) {
                    continue;
                }

                let value = u32::from_le_bytes([
                    raw[relative + 2],
                    raw[relative + 3],
                    raw[relative + 4],
                    raw[relative + 5],
                ]);
                if !syids.contains(&value) {
                    continue;
                }

                result.raw_scalar_syid_matches += 1;
                *result
                    .field_wire_match_counts
                    .entry(format!("0x{id:02x}/0x{wire:02x}"))
                    .or_insert(0) += 1;

                if id != FIELD_STORY_ID {
                    continue;
                }

                result.raw_field_27_syid_matches += 1;
                field_27_matches_in_shape += 1;
                field_27_matched_syids.insert(value);
                if parent_is_document_page {
                    field_27_matched_syids_on_document_pages.insert(value);
                } else {
                    field_27_matched_syids_outside_document_pages.insert(value);
                }

                let absolute = start + relative;
                if tail_range.is_some_and(|(tail_start, tail_end)| {
                    absolute >= tail_start && absolute < tail_end
                }) {
                    result.raw_field_27_syid_matches_in_opaque_tail += 1;
                }
            }
        }

        match field_27_matches_in_shape {
            1 => result.shape_chunks_with_one_field_27_syid_match += 1,
            2.. => result.shape_chunks_with_multiple_field_27_syid_matches += 1,
            _ => {}
        }

        if field_27_matches_in_shape > 0 {
            if parent_is_document_page {
                result.shape_chunks_with_field_27_match_on_document_pages += 1;
            } else {
                result.shape_chunks_with_field_27_match_outside_document_pages += 1;
            }
            *result
                .field_27_parent_class_counts
                .entry(parent_class)
                .or_insert(0) += 1;
        }
    }

    result.distinct_field_27_matched_syids = field_27_matched_syids.len();
    result.distinct_field_27_matched_syids_on_document_pages =
        field_27_matched_syids_on_document_pages.len();
    result.distinct_field_27_matched_syids_outside_document_pages =
        field_27_matched_syids_outside_document_pages.len();

    for frames in shape_scalars_by_story
        .values()
        .filter(|frames| frames.len() > 1)
    {
        result.multiframe_story_syid_count += 1;
        result.multiframe_shape_count += frames.len();

        let frame_seq_nums = frames
            .iter()
            .map(|(seq_num, _)| *seq_num)
            .collect::<BTreeSet<_>>();
        let mut peer_fields_seen = BTreeSet::new();
        let mut all_keys = BTreeSet::new();
        for (_, scalars) in frames {
            all_keys.extend(scalars.keys().copied());
            for (&(id, wire), values) in scalars {
                if id == FIELD_STORY_ID {
                    continue;
                }
                for value in values {
                    if frame_seq_nums.contains(value) {
                        let key = format!("0x{id:02x}/0x{wire:02x}");
                        *result
                            .multiframe_peer_seq_reference_counts
                            .entry(key.clone())
                            .or_insert(0) += 1;
                        peer_fields_seen.insert(key);
                    }
                }
            }
        }
        for key in peer_fields_seen {
            *result
                .multiframe_peer_seq_reference_story_counts
                .entry(key)
                .or_insert(0) += 1;
        }

        let expected_zero =
            (0..u32::try_from(frames.len()).unwrap_or(u32::MAX)).collect::<BTreeSet<_>>();
        let expected_one =
            (1..=u32::try_from(frames.len()).unwrap_or(u32::MAX)).collect::<BTreeSet<_>>();

        for (id, wire) in all_keys {
            if id == FIELD_STORY_ID {
                continue;
            }
            let values = frames
                .iter()
                .filter_map(|(_, scalars)| {
                    let values = scalars.get(&(id, wire))?;
                    (values.len() == 1).then_some(values[0])
                })
                .collect::<Vec<_>>();
            if values.len() != frames.len() {
                continue;
            }

            let distinct = values.iter().copied().collect::<BTreeSet<_>>();
            if distinct.len() == frames.len() {
                let key = format!("0x{id:02x}/0x{wire:02x}");
                *result
                    .multiframe_unique_scalar_candidate_story_counts
                    .entry(key.clone())
                    .or_insert(0) += 1;
                if distinct == expected_zero {
                    *result
                        .multiframe_zero_based_ordinal_candidate_story_counts
                        .entry(key.clone())
                        .or_insert(0) += 1;
                }
                if distinct == expected_one {
                    *result
                        .multiframe_one_based_ordinal_candidate_story_counts
                        .entry(key)
                        .or_insert(0) += 1;
                }
            }
        }
    }

    let mut table_field_27_matched_syids = BTreeSet::new();
    let mut table_field_27_matched_syids_on_document_pages = BTreeSet::new();
    let mut table_field_27_matched_syids_outside_document_pages = BTreeSet::new();
    for reference in references.values() {
        if single_raw_type(reference) != Some(RAW_TYPE_TABLE) {
            continue;
        }

        result.table_chunk_count += 1;

        let parent_seq = single_parent_seq(reference);
        let parent_is_document_page =
            parent_seq.is_some_and(|seq_num| document_page_handles.contains(&seq_num));
        let parent_class = match parent_seq {
            Some(_) if parent_is_document_page => "document_page/0x43".to_owned(),
            Some(seq_num) => references
                .get(&seq_num)
                .and_then(single_raw_type)
                .map(|raw_type| format!("outside_document_pages/0x{raw_type:02x}"))
                .unwrap_or_else(|| "outside_document_pages/unknown".to_owned()),
            None => "missing_or_ambiguous_parent".to_owned(),
        };

        let chunk = chunk_for_reference(contents_stream.clone(), contents, reference)?;
        let mut matched = false;
        for field in &chunk.fields {
            if field.id != FIELD_STORY_ID {
                continue;
            }
            let value = match &field.body {
                RawContentsBlockBody::U16 { value, .. } => u32::from(*value),
                RawContentsBlockBody::U32 { value, .. } => *value,
                _ => continue,
            };
            if syids.contains(&value) {
                matched = true;
                table_field_27_matched_syids.insert(value);
                if parent_is_document_page {
                    table_field_27_matched_syids_on_document_pages.insert(value);
                } else {
                    table_field_27_matched_syids_outside_document_pages.insert(value);
                }
            }
        }
        if matched {
            result.table_chunks_with_field_27_syid_match += 1;
            if parent_is_document_page {
                result.table_chunks_with_field_27_match_on_document_pages += 1;
            } else {
                result.table_chunks_with_field_27_match_outside_document_pages += 1;
            }
            *result
                .table_field_27_parent_class_counts
                .entry(parent_class)
                .or_insert(0) += 1;
        }
    }

    result.distinct_table_field_27_matched_syids = table_field_27_matched_syids.len();
    result.distinct_table_field_27_matched_syids_on_document_pages =
        table_field_27_matched_syids_on_document_pages.len();
    result.distinct_table_field_27_matched_syids_outside_document_pages =
        table_field_27_matched_syids_outside_document_pages.len();

    let shape_or_table = field_27_matched_syids
        .union(&table_field_27_matched_syids)
        .copied()
        .collect::<BTreeSet<_>>();
    result.distinct_story_syids_with_shape_or_table_identity = shape_or_table.len();
    result.distinct_story_syids_with_both_shape_and_table_identity = field_27_matched_syids
        .intersection(&table_field_27_matched_syids)
        .count();
    result.distinct_story_syids_with_shape_only_identity = field_27_matched_syids
        .difference(&table_field_27_matched_syids)
        .count();
    result.distinct_story_syids_with_table_only_identity = table_field_27_matched_syids
        .difference(&field_27_matched_syids)
        .count();
    result.distinct_story_syids_without_shape_or_table_identity =
        syids.difference(&shape_or_table).count();

    Ok(result)
}

/// Research-only census for grouped Story geometry prerequisites.
///
/// This does not materialize Group nodes or derive absolute child bounds. It
/// measures whether exact Contents Story ownership can be joined to the
/// OfficeArt group hierarchy and to the raw coordinate records required by
/// the proven FSPGR + ChildAnchor transform model.
pub fn analyze_mature_0x2c_grouped_story_geometry<R: Read + Seek>(
    mut reader: R,
) -> Result<PubGroupedStoryGeometryCorrelation> {
    reader.seek(SeekFrom::Start(0))?;
    let mut pub_bytes = Vec::new();
    reader.read_to_end(&mut pub_bytes)?;

    let contents =
        pub_cfb::read_stream_reader(Cursor::new(pub_bytes.as_slice()), CONTENTS_STREAM_PATH)
            .with_context(|| format!("read {CONTENTS_STREAM_PATH}"))?;
    let quill = pub_cfb::read_stream_reader(Cursor::new(pub_bytes.as_slice()), QUILL_STREAM_PATH)
        .with_context(|| format!("read {QUILL_STREAM_PATH}"))?;
    let escher = pub_cfb::read_stream_reader(Cursor::new(pub_bytes.as_slice()), ESCHER_STREAM_PATH)
        .with_context(|| format!("read {ESCHER_STREAM_PATH}"))?;

    analyze_mature_0x2c_grouped_story_geometry_from_streams(&contents, &quill, &escher)
}

pub fn analyze_mature_0x2c_grouped_story_geometry_from_streams(
    contents: &[u8],
    quill: &[u8],
    escher: &[u8],
) -> Result<PubGroupedStoryGeometryCorrelation> {
    let contents_stream = StreamPath(CONTENTS_STREAM_PATH.into());
    let header = parse_0x2c_header(contents_stream.clone(), contents)
        .context("parse mature-0x2C Contents header for grouped Story census")?;
    let trailer = parse_confirmed_0x2c_trailer_root(contents, &header)
        .context("parse mature-0x2C Contents trailer for grouped Story census")?;
    let references = build_reference_index(contents, &trailer.directory)?;

    let document_reference =
        unique_reference_by_raw_type(&references, RAW_TYPE_DOCUMENT, "DOCUMENT")?;
    let document_chunk =
        chunk_for_reference(contents_stream.clone(), contents, document_reference)?;
    let page_list_block = unique_block(&document_chunk, DOCUMENT_PAGE_LIST_ID)?.clone();
    let page_list = parse_confirmed_document_page_list(contents, page_list_block)
        .context("parse DOCUMENT PageList for grouped Story census")?;
    let document_page_handles = page_list
        .entries
        .iter()
        .filter_map(|entry| {
            references
                .get(&entry.handle)
                .is_some_and(|reference| single_raw_type(reference) == Some(RAW_TYPE_PAGE))
                .then_some(entry.handle)
        })
        .collect::<BTreeSet<_>>();

    let quill_catalog = parse_confirmed_story_catalog(StreamPath(QUILL_STREAM_PATH.into()), quill)
        .context("parse Quill story catalog for grouped Story census")?;
    let syids = quill_catalog
        .stories
        .iter()
        .map(|story| story.syid.0)
        .collect::<BTreeSet<_>>();

    let escher_inventory = inspect_sp_containers(StreamPath(ESCHER_STREAM_PATH.into()), escher)
        .context("parse OfficeArt SpContainers for grouped Story census")?;
    let escher_by_contents_seq = index_escher_by_contents_seq(&escher_inventory);

    let mut result = PubGroupedStoryGeometryCorrelation::default();
    let mut grouped_syids = BTreeSet::new();

    // Measure the direct-page Story shapes that the runtime currently rejects
    // only because their Publisher ClientAnchor is incomplete. This is
    // evidence-only: alternate OfficeArt records are counted, not promoted.
    for reference in references.values() {
        if single_raw_type(reference) != Some(RAW_TYPE_SHAPE) {
            continue;
        }
        let Some(parent_seq) = single_parent_seq(reference) else {
            continue;
        };
        if !document_page_handles.contains(&parent_seq) {
            continue;
        }

        let seq_num = seq_u32(reference.seq_num)?;
        let chunk = chunk_for_reference(contents_stream.clone(), contents, reference)?;
        let Some((text_id, _)) = unique_u32_field(&chunk, FIELD_STORY_ID)? else {
            continue;
        };
        if !syids.contains(&text_id) {
            continue;
        }

        result.direct_page_story_shape_count += 1;
        let matches = escher_by_contents_seq
            .get(&seq_num)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let shape = match matches {
            [index] => &escher_inventory.shapes[*index],
            _ => continue,
        };

        let complete = shape
            .client_anchor
            .as_ref()
            .is_some_and(anchor_has_unique_geometry_fields);
        if complete {
            continue;
        }

        result.direct_page_story_incomplete_client_anchor_count += 1;
        if shape.child_anchor.is_some() {
            result.direct_page_story_incomplete_client_anchor_with_child_anchor_count += 1;
        }
        if shape.fspgr.is_some() {
            result.direct_page_story_incomplete_client_anchor_with_fspgr_count += 1;
        }
        record_missing_anchor_fields(
            shape.client_anchor.as_ref(),
            &mut result.direct_page_story_incomplete_client_anchor_missing_field_counts,
        );
    }

    for reference in references.values() {
        if single_raw_type(reference) != Some(RAW_TYPE_SHAPE) {
            continue;
        }

        let Some(parent_seq) = single_parent_seq(reference) else {
            continue;
        };
        let Some(parent_reference) = references.get(&parent_seq) else {
            continue;
        };
        if single_raw_type(parent_reference) != Some(RAW_TYPE_GROUP) {
            continue;
        }

        let seq_num = seq_u32(reference.seq_num)?;
        let chunk = chunk_for_reference(contents_stream.clone(), contents, reference)?;
        let Some((text_id, _)) = unique_u32_field(&chunk, FIELD_STORY_ID)? else {
            continue;
        };
        if !syids.contains(&text_id) {
            continue;
        }

        result.grouped_story_shape_count += 1;
        grouped_syids.insert(text_id);

        let mut complete = true;
        let matches = escher_by_contents_seq
            .get(&seq_num)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let child = match matches {
            [index] => {
                result.unique_child_escher_shape_count += 1;
                &escher_inventory.shapes[*index]
            }
            [] => {
                increment_reason(&mut result, "child_escher_missing");
                continue;
            }
            _ => {
                increment_reason(&mut result, "child_escher_ambiguous");
                continue;
            }
        };

        if child.child_anchor.is_some() {
            result.child_anchor_count += 1;
        } else {
            complete = false;
            increment_reason(&mut result, "child_anchor_missing");
        }

        let Some(parent_source) = child.parent_group_shape_source.as_ref() else {
            increment_reason(&mut result, "parent_group_shape_link_missing");
            continue;
        };
        result.parent_group_shape_link_count += 1;

        let mut parent_matches = escher_inventory
            .shapes
            .iter()
            .filter(|shape| &shape.source == parent_source);
        let Some(group_shape) = parent_matches.next() else {
            increment_reason(&mut result, "parent_group_shape_missing");
            continue;
        };
        if parent_matches.next().is_some() {
            increment_reason(&mut result, "parent_group_shape_ambiguous");
            continue;
        }

        let group_contents_id_matches = group_shape
            .client_data
            .as_ref()
            .and_then(|record| unique_escher_field(record, PUBLISHER_FIELD_SHAPE_ID))
            .is_some_and(|field| field.value == parent_seq);
        if group_contents_id_matches {
            result.parent_group_contents_identity_match_count += 1;
        } else {
            complete = false;
            increment_reason(&mut result, "parent_group_contents_identity_mismatch");
        }

        if group_shape.fspgr.is_some() {
            result.parent_group_fspgr_count += 1;
        } else {
            complete = false;
            increment_reason(&mut result, "parent_group_fspgr_missing");
        }

        let complete_group_anchor = group_shape
            .client_anchor
            .as_ref()
            .is_some_and(anchor_has_unique_geometry_fields);
        if complete_group_anchor {
            result.parent_group_complete_client_anchor_count += 1;
        } else {
            complete = false;
            increment_reason(&mut result, "parent_group_client_anchor_incomplete");
        }

        let group_parent_seq = single_parent_seq(parent_reference);
        let one_level = group_parent_seq
            .is_some_and(|group_parent| document_page_handles.contains(&group_parent));
        if one_level {
            result.one_level_page_group_count += 1;
        } else {
            complete = false;
            let nested = group_parent_seq
                .and_then(|group_parent| references.get(&group_parent))
                .and_then(single_raw_type)
                == Some(RAW_TYPE_GROUP);
            if nested {
                result.nested_group_count += 1;
                increment_reason(&mut result, "nested_group");
            } else {
                increment_reason(&mut result, "group_parent_not_document_page");
            }
        }

        if complete {
            result.complete_one_level_geometry_count += 1;
        }

        // A nested group does not have a page-level ClientAnchor. Its own
        // ChildAnchor positions it inside the next parent group's FSPGR
        // coordinate space. Walk that exact hierarchy until a DOCUMENT page
        // is reached, preserving the one-level counters above as a separate
        // measurement.
        let child_rotation = shape_has_nonzero_rotation(child);
        let child_flip_h = shape_has_fsp_flag(child, OFFICEART_FSP_FLIP_H);
        let child_flip_v = shape_has_fsp_flag(child, OFFICEART_FSP_FLIP_V);
        result.grouped_story_child_rotation_count += usize::from(child_rotation);
        result.grouped_story_child_flip_h_count += usize::from(child_flip_h);
        result.grouped_story_child_flip_v_count += usize::from(child_flip_v);

        let mut recursive_complete = child.child_anchor.is_some();
        if !recursive_complete {
            increment_recursive_reason(&mut result, "child_anchor_missing");
        }

        let mut has_group_rotation = false;
        let mut has_group_flip_h = false;
        let mut has_group_flip_v = false;
        let mut current_shape = child;
        let mut current_group_seq = parent_seq;
        let mut seen_group_seq = BTreeSet::new();
        let mut group_depth = 0_usize;

        loop {
            if !seen_group_seq.insert(current_group_seq) {
                recursive_complete = false;
                increment_recursive_reason(&mut result, "group_cycle");
                break;
            }
            group_depth += 1;

            let Some(group_reference) = references.get(&current_group_seq) else {
                recursive_complete = false;
                increment_recursive_reason(&mut result, "group_contents_reference_missing");
                break;
            };
            if single_raw_type(group_reference) != Some(RAW_TYPE_GROUP) {
                recursive_complete = false;
                increment_recursive_reason(&mut result, "group_contents_wrong_raw_type");
                break;
            }

            let group_matches = escher_by_contents_seq
                .get(&current_group_seq)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let group_shape = match group_matches {
                [index] => &escher_inventory.shapes[*index],
                [] => {
                    recursive_complete = false;
                    increment_recursive_reason(&mut result, "group_escher_missing");
                    break;
                }
                _ => {
                    recursive_complete = false;
                    increment_recursive_reason(&mut result, "group_escher_ambiguous");
                    break;
                }
            };

            if current_shape.parent_group_shape_source.as_ref() != Some(&group_shape.source) {
                recursive_complete = false;
                increment_recursive_reason(&mut result, "group_parent_link_mismatch");
            }
            if group_shape.fspgr.is_none() {
                recursive_complete = false;
                increment_recursive_reason(&mut result, "group_fspgr_missing");
            }

            has_group_rotation |= shape_has_nonzero_rotation(group_shape);
            has_group_flip_h |= shape_has_fsp_flag(group_shape, OFFICEART_FSP_FLIP_H);
            has_group_flip_v |= shape_has_fsp_flag(group_shape, OFFICEART_FSP_FLIP_V);

            let Some(next_parent_seq) = single_parent_seq(group_reference) else {
                recursive_complete = false;
                increment_recursive_reason(&mut result, "group_parent_missing_or_ambiguous");
                break;
            };

            if document_page_handles.contains(&next_parent_seq) {
                if !group_shape
                    .client_anchor
                    .as_ref()
                    .is_some_and(anchor_has_unique_geometry_fields)
                {
                    recursive_complete = false;
                    increment_recursive_reason(&mut result, "top_group_client_anchor_incomplete");
                    record_missing_anchor_fields(
                        group_shape.client_anchor.as_ref(),
                        &mut result.top_group_incomplete_client_anchor_missing_field_counts,
                    );
                }
                break;
            }

            let nested =
                references.get(&next_parent_seq).and_then(single_raw_type) == Some(RAW_TYPE_GROUP);
            if !nested {
                recursive_complete = false;
                increment_recursive_reason(&mut result, "group_parent_not_document_page_or_group");
                break;
            }

            if group_shape.child_anchor.is_none() {
                recursive_complete = false;
                increment_recursive_reason(&mut result, "nested_group_child_anchor_missing");
            }

            current_shape = group_shape;
            current_group_seq = next_parent_seq;
        }

        result.max_group_depth = result.max_group_depth.max(group_depth);
        *result
            .group_depth_counts
            .entry(group_depth.to_string())
            .or_insert(0) += 1;
        result.grouped_story_group_ancestor_rotation_count += usize::from(has_group_rotation);
        result.grouped_story_group_ancestor_flip_h_count += usize::from(has_group_flip_h);
        result.grouped_story_group_ancestor_flip_v_count += usize::from(has_group_flip_v);
        result.grouped_story_group_ancestor_nontrivial_transform_count +=
            usize::from(has_group_rotation || has_group_flip_h || has_group_flip_v);
        if recursive_complete {
            result.complete_recursive_geometry_count += 1;
        }
    }

    result.grouped_story_distinct_syid_count = grouped_syids.len();

    // TABLE ownership is distinct from ordinary StoryFrame ownership, but a
    // grouped TABLE still needs the same OfficeArt ancestry to obtain runtime
    // geometry. Measure that fence independently without changing semantics.
    for reference in references.values() {
        if single_raw_type(reference) != Some(RAW_TYPE_TABLE) {
            continue;
        }
        let Some(parent_seq) = single_parent_seq(reference) else {
            continue;
        };
        if references.get(&parent_seq).and_then(single_raw_type) != Some(RAW_TYPE_GROUP) {
            continue;
        }

        let table_seq = seq_u32(reference.seq_num)?;
        let chunk = chunk_for_reference(contents_stream.clone(), contents, reference)?;
        let Some((text_id, _)) = unique_u32_field(&chunk, FIELD_STORY_ID)? else {
            continue;
        };
        if !syids.contains(&text_id) {
            continue;
        }

        result.grouped_table_story_count += 1;
        let child_matches = escher_by_contents_seq
            .get(&table_seq)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let child = match child_matches {
            [index] => &escher_inventory.shapes[*index],
            [] => {
                *result
                    .grouped_table_incomplete_reason_counts
                    .entry("child_escher_missing".into())
                    .or_insert(0) += 1;
                continue;
            }
            _ => {
                *result
                    .grouped_table_incomplete_reason_counts
                    .entry("child_escher_ambiguous".into())
                    .or_insert(0) += 1;
                continue;
            }
        };

        match inspect_grouped_geometry_chain(
            child,
            parent_seq,
            &references,
            &document_page_handles,
            &escher_inventory,
            &escher_by_contents_seq,
        ) {
            Ok((depth, has_nontrivial_transform)) => {
                result.grouped_table_max_group_depth =
                    result.grouped_table_max_group_depth.max(depth);
                if has_nontrivial_transform {
                    result.grouped_table_nontrivial_transform_count += 1;
                } else {
                    result.grouped_table_complete_recursive_geometry_count += 1;
                }
            }
            Err(reason) => {
                *result
                    .grouped_table_incomplete_reason_counts
                    .entry(reason)
                    .or_insert(0) += 1;
            }
        }
    }

    Ok(result)
}

fn inspect_grouped_geometry_chain(
    child: &pub_escher::SpContainerObservation,
    first_group_seq: u32,
    references: &BTreeMap<u32, Contents0x2cChunkReference>,
    document_page_handles: &BTreeSet<u32>,
    escher_inventory: &SpContainerInventory,
    escher_by_contents_seq: &BTreeMap<u32, Vec<usize>>,
) -> std::result::Result<(usize, bool), String> {
    if child.child_anchor.is_none() {
        return Err("child_anchor_missing".into());
    }

    let mut current_shape = child;
    let mut current_group_seq = first_group_seq;
    let mut seen = BTreeSet::new();
    let mut depth = 0_usize;
    let mut nontrivial_transform = shape_has_nonzero_rotation(child)
        || shape_has_fsp_flag(child, OFFICEART_FSP_FLIP_H)
        || shape_has_fsp_flag(child, OFFICEART_FSP_FLIP_V);

    loop {
        if !seen.insert(current_group_seq) {
            return Err("group_cycle".into());
        }
        depth += 1;

        let group_reference = references
            .get(&current_group_seq)
            .ok_or_else(|| "group_contents_reference_missing".to_owned())?;
        if single_raw_type(group_reference) != Some(RAW_TYPE_GROUP) {
            return Err("group_contents_wrong_raw_type".into());
        }

        let group_matches = escher_by_contents_seq
            .get(&current_group_seq)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let group_shape = match group_matches {
            [index] => &escher_inventory.shapes[*index],
            [] => return Err("group_escher_missing".into()),
            _ => return Err("group_escher_ambiguous".into()),
        };

        if current_shape.parent_group_shape_source.as_ref() != Some(&group_shape.source) {
            return Err("group_parent_link_mismatch".into());
        }
        if group_shape.fspgr.is_none() {
            return Err("group_fspgr_missing".into());
        }
        nontrivial_transform |= shape_has_nonzero_rotation(group_shape)
            || shape_has_fsp_flag(group_shape, OFFICEART_FSP_FLIP_H)
            || shape_has_fsp_flag(group_shape, OFFICEART_FSP_FLIP_V);

        let parent_seq = single_parent_seq(group_reference)
            .ok_or_else(|| "group_parent_missing_or_ambiguous".to_owned())?;
        if document_page_handles.contains(&parent_seq) {
            if !group_shape
                .client_anchor
                .as_ref()
                .is_some_and(anchor_has_unique_geometry_fields)
            {
                return Err("top_group_client_anchor_incomplete".into());
            }
            return Ok((depth, nontrivial_transform));
        }

        if references.get(&parent_seq).and_then(single_raw_type) != Some(RAW_TYPE_GROUP) {
            return Err("group_parent_not_document_page_or_group".into());
        }
        if group_shape.child_anchor.is_none() {
            return Err("nested_group_child_anchor_missing".into());
        }

        current_shape = group_shape;
        current_group_seq = parent_seq;
    }
}

fn shape_has_nonzero_rotation(shape: &pub_escher::SpContainerObservation) -> bool {
    shape.fopts.iter().any(|record| {
        record.properties.iter().any(|property| {
            property.property_id() == OFFICEART_PROPERTY_ROTATION && property.op != 0
        })
    })
}

fn shape_has_fsp_flag(shape: &pub_escher::SpContainerObservation, flag: u32) -> bool {
    shape.fsp.as_ref().is_some_and(|fsp| fsp.flags & flag != 0)
}

fn increment_reason(result: &mut PubGroupedStoryGeometryCorrelation, reason: &str) {
    *result
        .incomplete_reason_counts
        .entry(reason.to_owned())
        .or_insert(0) += 1;
}

fn increment_recursive_reason(result: &mut PubGroupedStoryGeometryCorrelation, reason: &str) {
    *result
        .recursive_incomplete_reason_counts
        .entry(reason.to_owned())
        .or_insert(0) += 1;
}

pub fn build_mature_0x2c_from_streams(
    source_hash: Sha256Digest,
    contents: &[u8],
    quill: &[u8],
    escher: &[u8],
) -> Result<PubSourceGraphBuild> {
    let adapter_version = format!("pub-rs/{}", env!("CARGO_PKG_VERSION"));
    let source = SourceDescriptor {
        format: "pub".into(),
        format_version: Some("0x2c".into()),
        adapter_version,
        source_hash,
    };

    let contents_stream = StreamPath(CONTENTS_STREAM_PATH.into());
    let header = parse_0x2c_header(contents_stream.clone(), contents)
        .context("parse mature-0x2C Contents header")?;
    let trailer = parse_confirmed_0x2c_trailer_root(contents, &header)
        .context("parse mature-0x2C Contents trailer")?;
    let references = build_reference_index(contents, &trailer.directory)?;

    let story_catalog_reference = unique_reference_by_raw_type(
        &references,
        CONTENTS_RAW_TYPE_STORY_CATALOG,
        "Story catalog 0x65",
    )?;
    let story_catalog_chunk =
        chunk_for_reference(contents_stream.clone(), contents, story_catalog_reference)?;
    let story_catalog = parse_confirmed_mature_story_catalog(contents, &story_catalog_chunk)
        .context("parse mature Story catalog 0x65")?;
    let story_layout_keys: BTreeMap<_, _> = story_catalog
        .entries
        .iter()
        .filter_map(|entry| {
            Some((
                entry.text_id,
                (entry.layout_key?, entry.layout_key_source.as_ref()?.clone()),
            ))
        })
        .collect();

    let document_reference =
        unique_reference_by_raw_type(&references, RAW_TYPE_DOCUMENT, "DOCUMENT")?;
    let document_seq = seq_u32(document_reference.seq_num)?;
    let document_chunk =
        chunk_for_reference(contents_stream.clone(), contents, document_reference)?;
    let page_list_block = unique_block(&document_chunk, DOCUMENT_PAGE_LIST_ID)?.clone();
    let page_list = parse_confirmed_document_page_list(contents, page_list_block)
        .context("parse DOCUMENT PageList")?;

    let mut diagnostics = Vec::new();
    let (page_width_emu, page_height_emu, margins_count) =
        consensus_publication_page_extent(contents_stream.clone(), contents, &references)?;
    if margins_count > 1 {
        diagnostics.push(PubBridgeDiagnostic::EquivalentMarginsPageExtents {
            count: margins_count,
            width_emu: page_width_emu,
            height_emu: page_height_emu,
        });
    }

    let document_id = derive_pub_document_id(&source_hash, document_seq)?;
    let mut document_pages = Vec::new();
    let mut pages = BTreeMap::new();
    let mut page_seq_to_id = BTreeMap::new();
    let mut seen_page_handles = BTreeSet::new();

    for entry in &page_list.entries {
        let Some(reference) = references.get(&entry.handle) else {
            diagnostics.push(PubBridgeDiagnostic::PageListUnknownEntry {
                handle: entry.handle,
                raw_type: None,
            });
            continue;
        };

        match single_raw_type(reference) {
            Some(RAW_TYPE_PAGE) => {
                if !seen_page_handles.insert(entry.handle) {
                    bail!("DOCUMENT PageList repeats PAGE handle {}", entry.handle);
                }
                let page_id = derive_pub_page_id(&source_hash, entry.handle)?;
                document_pages.push(page_id);
                page_seq_to_id.insert(entry.handle, page_id);
                pages.insert(
                    page_id,
                    Page {
                        id: page_id,
                        size: Size2D::new(
                            LengthEmu::new(i64::from(page_width_emu)),
                            LengthEmu::new(i64::from(page_height_emu)),
                        ),
                        bleed: None,
                        margins: None,
                        // Membership is preserved by NodeHeader.parent_id. We do
                        // not guess a z-order from directory seqNum.
                        children: Vec::new(),
                        extensions: Vec::new(),
                    },
                );
            }
            Some(RAW_TYPE_PAGE_LIST_SPECIAL) => {
                diagnostics.push(PubBridgeDiagnostic::PageListSpecialEntry {
                    handle: entry.handle,
                    raw_type: RAW_TYPE_PAGE_LIST_SPECIAL,
                });
            }
            other => {
                diagnostics.push(PubBridgeDiagnostic::PageListUnknownEntry {
                    handle: entry.handle,
                    raw_type: other,
                });
            }
        }
    }

    if pages.is_empty() {
        bail!("DOCUMENT PageList exposes no confirmed PAGE 0x43 entries");
    }

    let document = Document {
        id: document_id,
        format_origin: "pub".into(),
        source_hash,
        pages: document_pages,
        resources: Vec::new(),
        styles: Vec::new(),
    };
    let mut graph = PubSourceGraph::empty(source, document);
    graph.pages = pages;

    let quill_stream = StreamPath(QUILL_STREAM_PATH.into());
    let quill_catalog = parse_confirmed_story_catalog(quill_stream.clone(), quill)
        .context("parse grounded Quill story catalog")?;
    let mcld = match parse_bounded_mcld(quill_stream, quill, &quill_catalog.descriptor_nodes) {
        Ok(mcld) => Some(mcld),
        Err(QuillMcldReadError::MissingMcldDescriptor) => None,
        Err(QuillMcldReadError::RecordCountMismatch {
            record_count,
            record_id_count,
        }) => {
            diagnostics.push(PubBridgeDiagnostic::McldRecordCountMismatch {
                record_count,
                record_id_count,
            });
            None
        }
        Err(error) => return Err(error).context("parse bounded Quill MCLD"),
    };
    let mut story_by_syid = BTreeMap::new();

    for story_slice in &quill_catalog.stories {
        let syid = story_slice.syid.0;
        let story_id = derive_pub_story_id(&source_hash, syid)?;
        let object_key = quill_story_object_key(syid);
        let text = decode_utf16le_strict(&story_slice.utf16le)
            .with_context(|| format!("decode Quill story SYID {syid} as strict UTF-16LE"))?;

        let source_refs = vec![
            source_ref(
                &graph.source,
                &story_slice.syid_source,
                Some(object_key.clone()),
                Some("SYID".into()),
                SourceRole::Relation,
                AuthorityClass::Authoritative,
                ReadConfidence::Exact,
            ),
            source_ref(
                &graph.source,
                &story_slice.text_source,
                Some(object_key),
                Some("TEXT".into()),
                SourceRole::Semantic,
                AuthorityClass::Authoritative,
                ReadConfidence::Exact,
            ),
        ];

        graph.stories.insert(
            story_id,
            Story {
                id: story_id,
                text,
                paragraphs: Vec::new(),
                runs: Vec::new(),
                fields: Vec::new(),
                hyperlinks: Vec::new(),
                source_refs,
            },
        );
        story_by_syid.insert(syid, story_id);
    }

    let escher_inventory = inspect_sp_containers(StreamPath(ESCHER_STREAM_PATH.into()), escher)
        .context("parse OfficeArt SpContainers")?;
    let escher_by_contents_seq = index_escher_by_contents_seq(&escher_inventory);

    for reference in references.values() {
        let raw_type = single_raw_type(reference);
        if raw_type != Some(RAW_TYPE_SHAPE) && raw_type != Some(RAW_TYPE_TABLE) {
            continue;
        }

        let Some(parent_seq) = single_parent_seq(reference) else {
            continue;
        };

        let seq_num = seq_u32(reference.seq_num)?;
        let chunk = chunk_for_reference(contents_stream.clone(), contents, reference)?;
        if let Some(tail) = &chunk.unsupported_tail {
            diagnostics.push(PubBridgeDiagnostic::OpaqueContentsTail {
                seq_num,
                byte_range: ByteRange::new(tail.offset, tail.len),
            });
        }

        let matches = escher_by_contents_seq
            .get(&seq_num)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let shape = match matches {
            [] => {
                diagnostics.push(PubBridgeDiagnostic::MissingEscherGeometry { seq_num });
                continue;
            }
            [index] => &escher_inventory.shapes[*index],
            many => {
                diagnostics.push(PubBridgeDiagnostic::AmbiguousEscherGeometry {
                    seq_num,
                    matches: many.len(),
                });
                continue;
            }
        };

        let direct_page = page_seq_to_id.get(&parent_seq).copied();
        let exact_story_identity = match raw_type {
            Some(RAW_TYPE_SHAPE) => unique_u32_field(&chunk, FIELD_STORY_ID)?
                .map(|(value, _)| value)
                .filter(|value| story_by_syid.contains_key(value)),
            Some(RAW_TYPE_TABLE) => {
                unique_story_id_scalar(&chunk)?.filter(|value| story_by_syid.contains_key(value))
            }
            _ => None,
        };
        let grouped_projection = if direct_page.is_none()
            && references.get(&parent_seq).and_then(single_raw_type) == Some(RAW_TYPE_GROUP)
            && exact_story_identity.is_some()
        {
            match project_grouped_object_shape(
                parent_seq,
                shape,
                &references,
                &page_seq_to_id,
                &graph.pages,
                &escher_inventory,
                &escher_by_contents_seq,
            ) {
                Ok(Some(projection)) => {
                    diagnostics.push(if raw_type == Some(RAW_TYPE_TABLE) {
                        PubBridgeDiagnostic::GroupedTableProjected {
                            seq_num,
                            depth: projection.depth,
                        }
                    } else {
                        PubBridgeDiagnostic::GroupedStoryProjected {
                            seq_num,
                            depth: projection.depth,
                        }
                    });
                    Some(projection)
                }
                Ok(None) => None,
                Err(error) => {
                    diagnostics.push(if raw_type == Some(RAW_TYPE_TABLE) {
                        PubBridgeDiagnostic::GroupedTableProjectionUnavailable {
                            seq_num,
                            reason: error.to_string(),
                        }
                    } else {
                        PubBridgeDiagnostic::GroupedStoryProjectionUnavailable {
                            seq_num,
                            reason: error.to_string(),
                        }
                    });
                    None
                }
            }
        } else {
            None
        };

        let (page_id, bounds, grouped_sources) = if let Some(page_id) = direct_page {
            let Some(anchor) = shape.client_anchor.as_ref() else {
                diagnostics.push(PubBridgeDiagnostic::IncompleteEscherAnchor { seq_num });
                continue;
            };
            let Some(bounds) = page_relative_bounds(
                graph
                    .pages
                    .get(&page_id)
                    .expect("page id came from graph registry"),
                anchor,
            ) else {
                let complete = anchor_has_unique_geometry_fields(anchor);
                diagnostics.push(if complete {
                    PubBridgeDiagnostic::InvalidEscherAnchor { seq_num }
                } else {
                    PubBridgeDiagnostic::IncompleteEscherAnchor { seq_num }
                });
                continue;
            };
            (page_id, bounds, Vec::new())
        } else if let Some(projection) = grouped_projection {
            (
                projection.page_id,
                projection.bounds,
                projection.group_sources,
            )
        } else {
            continue;
        };

        let node_id = derive_pub_node_id(&source_hash, seq_num)?;
        let image_slot = exact_image_slot(shape, seq_num, &mut diagnostics);
        let explicit_image_crop = image_slot
            .is_some()
            .then(|| bounded_officeart_image_crop(shape))
            .flatten();
        let story_frame = if raw_type == Some(RAW_TYPE_SHAPE) {
            build_story_frame(
                source_hash,
                seq_num,
                &chunk,
                &story_by_syid,
                &mut diagnostics,
            )?
        } else {
            None
        };
        let (table_story, table) = if raw_type == Some(RAW_TYPE_TABLE) {
            let context = table_bridge::TableBridgeContext {
                source: &graph.source,
                contents_stream: &contents_stream,
                contents,
                references: &references,
                quill_catalog: &quill_catalog,
                story_by_syid: &story_by_syid,
                story_layout_keys: &story_layout_keys,
                mcld: mcld.as_ref(),
            };
            (
                table_bridge::build_table_story_ownership_source(&context, seq_num, &chunk)?,
                table_bridge::build_table_source(&context, seq_num, &chunk, &mut diagnostics)?,
            )
        } else {
            (None, None)
        };

        let object_key = contents_object_key(seq_num);
        let mut source_refs = vec![source_ref(
            &graph.source,
            &chunk.source,
            Some(object_key.clone()),
            Some("chunk".into()),
            SourceRole::Semantic,
            AuthorityClass::Authoritative,
            ReadConfidence::Exact,
        )];
        source_refs.push(source_ref(
            &graph.source,
            &shape.source,
            Some(format!("escher/client-data-shape-id/{seq_num}")),
            Some(if grouped_sources.is_empty() {
                "SpContainer/ClientAnchor".into()
            } else {
                "SpContainer/ChildAnchor".into()
            }),
            SourceRole::Projection,
            AuthorityClass::Authoritative,
            ReadConfidence::Exact,
        ));
        for (depth, span) in grouped_sources.iter().enumerate() {
            source_refs.push(source_ref(
                &graph.source,
                span,
                Some(format!("escher/group-ancestor/{seq_num}/{depth}")),
                Some("SpgrContainer/SpContainer".into()),
                SourceRole::Projection,
                AuthorityClass::Authoritative,
                ReadConfidence::Exact,
            ));
        }
        if let Some(table) = &table {
            source_refs.extend(table.source_refs.clone());
        } else if let Some(table_story) = &table_story {
            source_refs.extend(table_story.source_refs.clone());
        }

        graph.nodes.insert(
            node_id,
            Node {
                // Grouped Story shapes and TABLEs are projected to page-relative
                // geometry while exact group ancestry remains in provenance.
                // The current resolver does not yet compose Group transforms.
                kind: if raw_type == Some(RAW_TYPE_TABLE) {
                    NodeKind::Table
                } else {
                    NodeKind::Shape
                },
                header: NodeHeader {
                    id: node_id,
                    parent_id: page_id.into_canonical(),
                    bounds,
                    transform: Affine2D::identity(),
                    source_refs,
                    extensions: Vec::new(),
                },
                payload: PubNodePayload {
                    contents_seq_num: seq_num,
                    officeart_shape_type: shape.fsp.as_ref().map(|fsp| fsp.shape_type),
                    officeart_spid: shape.fsp.as_ref().map(|fsp| fsp.spid),
                    image_slot,
                    explicit_image_crop,
                    explicit_paint: explicit_officeart_paint(shape),
                    story_frame,
                    table_story,
                    table,
                },
            },
        );
    }

    add_missing_link_target_diagnostics(&graph, &mut diagnostics);

    Ok(PubSourceGraphBuild { graph, diagnostics })
}

fn build_reference_index(
    contents: &[u8],
    directory: &pub_contents::Contents0x2cDirectory,
) -> Result<BTreeMap<u32, Contents0x2cChunkReference>> {
    let mut references = BTreeMap::new();

    for seq_num in 0..directory.slots.len() {
        let Some(reference) = parse_confirmed_chunk_reference(contents, directory, seq_num)
            .with_context(|| format!("parse Contents directory reference seq {seq_num}"))?
        else {
            continue;
        };
        let key = seq_u32(reference.seq_num)?;
        if references.insert(key, reference).is_some() {
            bail!("duplicate Contents directory seq {key}");
        }
    }

    Ok(references)
}

fn consensus_publication_page_extent(
    stream: StreamPath,
    contents: &[u8],
    references: &BTreeMap<u32, Contents0x2cChunkReference>,
) -> Result<(u32, u32, usize)> {
    let margins = references
        .values()
        .filter(|reference| single_raw_type(reference) == Some(RAW_TYPE_MARGINS))
        .collect::<Vec<_>>();

    if margins.is_empty() {
        bail!("missing Margins/OplMg raw type 0x{RAW_TYPE_MARGINS:02X}");
    }

    let mut dimensions = Vec::with_capacity(margins.len());
    for reference in margins {
        let chunk = chunk_for_reference(stream.clone(), contents, reference)?;
        let extent = parse_confirmed_margins_page_extent(contents, &chunk).with_context(|| {
            format!("parse Margins/OplMg page extent seq {}", reference.seq_num)
        })?;
        dimensions.push((extent.width_emu, extent.height_emu));
    }

    let (width_emu, height_emu) = require_consensus_page_extent(&dimensions)?;
    Ok((width_emu, height_emu, dimensions.len()))
}

fn require_consensus_page_extent(extents: &[(u32, u32)]) -> Result<(u32, u32)> {
    let first = extents
        .first()
        .copied()
        .context("publication has no confirmed Margins/OplMg page extent")?;
    if first.0 == 0 || first.1 == 0 {
        bail!("publication page extent must be positive");
    }

    for &(width_emu, height_emu) in &extents[1..] {
        if width_emu == 0 || height_emu == 0 {
            bail!("publication page extent must be positive");
        }
        if (width_emu, height_emu) != first {
            bail!(
                "conflicting Margins/OplMg page extents: expected {}x{} EMU, found {}x{} EMU",
                first.0,
                first.1,
                width_emu,
                height_emu
            );
        }
    }

    Ok(first)
}

fn unique_reference_by_raw_type<'a>(
    references: &'a BTreeMap<u32, Contents0x2cChunkReference>,
    raw_type: u16,
    label: &str,
) -> Result<&'a Contents0x2cChunkReference> {
    let mut matches = references
        .values()
        .filter(|reference| single_raw_type(reference) == Some(raw_type));
    let first = matches
        .next()
        .with_context(|| format!("missing {label} raw type 0x{raw_type:02X}"))?;
    if matches.next().is_some() {
        bail!("multiple {label} raw type 0x{raw_type:02X} objects");
    }
    Ok(first)
}

fn chunk_for_reference(
    stream: StreamPath,
    contents: &[u8],
    reference: &Contents0x2cChunkReference,
) -> Result<Contents0x2cChunk> {
    if reference.chunk_offsets.len() != 1 {
        bail!(
            "Contents seq {} has {} chunk offsets, expected exactly one",
            reference.seq_num,
            reference.chunk_offsets.len()
        );
    }

    parse_confirmed_0x2c_chunk(stream, contents, reference.chunk_offsets[0].value)
        .with_context(|| format!("parse Contents chunk seq {}", reference.seq_num))
}

fn unique_block(chunk: &Contents0x2cChunk, id: u16) -> Result<&RawContentsBlock> {
    let mut matches = chunk.fields.iter().filter(|field| field.id == id);
    let first = matches
        .next()
        .with_context(|| format!("missing Contents field 0x{id:02X}"))?;
    if matches.next().is_some() {
        bail!("duplicate Contents field 0x{id:02X}");
    }
    Ok(first)
}

fn single_raw_type(reference: &Contents0x2cChunkReference) -> Option<u16> {
    match reference.raw_types.as_slice() {
        [field] => Some(field.value),
        _ => None,
    }
}

fn single_parent_seq(reference: &Contents0x2cChunkReference) -> Option<u32> {
    match reference.parent_seq_nums.as_slice() {
        [field] => Some(field.value),
        _ => None,
    }
}

fn seq_u32(seq_num: usize) -> Result<u32> {
    u32::try_from(seq_num).map_err(|_| anyhow!("Contents seqNum does not fit u32: {seq_num}"))
}

fn derive_pub_id(
    source_hash: &Sha256Digest,
    object_key: &str,
    semantic_role: &str,
) -> Result<CanonicalId> {
    derive_source_canonical_id(SourceDerivedIdInput {
        source_hash,
        adapter_id: PUB_ADAPTER_ID,
        source_object_key: object_key,
        semantic_role,
    })
    .map_err(|error| anyhow!("source-derived identity error: {error:?}"))
}

fn decode_utf16le_strict(bytes: &[u8]) -> Result<String> {
    if bytes.len() % 2 != 0 {
        bail!("UTF-16LE byte length is odd: {}", bytes.len());
    }
    let units = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    String::from_utf16(&units).map_err(|error| anyhow!("invalid UTF-16LE: {error}"))
}

fn source_ref(
    source: &SourceDescriptor,
    span: &RawSpan,
    object_key: Option<String>,
    path: Option<String>,
    role: SourceRole,
    authority: AuthorityClass,
    confidence: ReadConfidence,
) -> SourceRef {
    SourceRef {
        format: source.format.clone(),
        adapter_version: source.adapter_version.clone(),
        source_hash: source.source_hash,
        carrier: span.stream.0.clone(),
        object_key,
        path,
        byte_range: Some(ByteRange::new(span.offset, span.len)),
        role,
        authority,
        confidence: Some(confidence),
    }
}

fn index_escher_by_contents_seq(inventory: &SpContainerInventory) -> BTreeMap<u32, Vec<usize>> {
    let mut index = BTreeMap::<u32, Vec<usize>>::new();

    for (shape_index, shape) in inventory.shapes.iter().enumerate() {
        let Some(client_data) = shape.client_data.as_ref() else {
            continue;
        };
        for field in client_data
            .fields
            .iter()
            .filter(|field| field.id == PUBLISHER_FIELD_SHAPE_ID)
        {
            let entry = index.entry(field.value).or_default();
            if entry.last().copied() != Some(shape_index) {
                entry.push(shape_index);
            }
        }
    }

    index
}

const OFFICE_ART_FILL_TYPE: u16 = 0x0180;
const OFFICE_ART_FILL_COLOR: u16 = 0x0181;
const OFFICE_ART_FILL_BOOLEANS: u16 = 0x01BF;
const OFFICE_ART_LINE_COLOR: u16 = 0x01C0;
const OFFICE_ART_LINE_WIDTH: u16 = 0x01CB;
const OFFICE_ART_LINE_BOOLEANS: u16 = 0x01FF;

const FILL_USE_FILLED_BIT: u32 = 1 << 11;
const FILL_FILLED_BIT: u32 = 1 << 27;
const LINE_USE_LINE_BIT: u32 = 1 << 12;
const LINE_LINE_BIT: u32 = 1 << 28;

fn explicit_officeart_paint(
    shape: &pub_escher::SpContainerObservation,
) -> PubExplicitShapePaintSource {
    let fill_type = unique_explicit_officeart_scalar(shape, OFFICE_ART_FILL_TYPE);
    let fill_color = unique_explicit_officeart_scalar(shape, OFFICE_ART_FILL_COLOR)
        .and_then(direct_officeart_rgb);
    let fill_visible =
        unique_explicit_officeart_scalar(shape, OFFICE_ART_FILL_BOOLEANS).and_then(|value| {
            (value & FILL_USE_FILLED_BIT != 0).then_some(value & FILL_FILLED_BIT != 0)
        });

    let line_color = unique_explicit_officeart_scalar(shape, OFFICE_ART_LINE_COLOR)
        .and_then(direct_officeart_rgb);
    let line_width = unique_explicit_officeart_scalar(shape, OFFICE_ART_LINE_WIDTH)
        .and_then(|value| (value <= 0x0132_F540).then_some(i64::from(value)));
    let line_visible = unique_explicit_officeart_scalar(shape, OFFICE_ART_LINE_BOOLEANS)
        .and_then(|value| (value & LINE_USE_LINE_BIT != 0).then_some(value & LINE_LINE_BIT != 0));

    PubExplicitShapePaintSource {
        fill: PubExplicitFillSource {
            solid: fill_type == Some(0),
            color_rgb: fill_color,
            visible: fill_visible,
        },
        line: PubExplicitLineSource {
            color_rgb: line_color,
            width_emu: line_width,
            visible: line_visible,
        },
    }
}

fn unique_explicit_officeart_scalar(
    shape: &pub_escher::SpContainerObservation,
    property_id: u16,
) -> Option<u32> {
    let values = shape
        .fopts
        .iter()
        .flat_map(|record| record.properties.iter())
        .filter(|property| {
            property.property_id() == property_id && !property.f_bid() && !property.f_complex()
        })
        .map(|property| property.op)
        .collect::<BTreeSet<_>>();

    if values.len() == 1 {
        values.iter().next().copied()
    } else {
        None
    }
}

fn bounded_officeart_image_crop(
    shape: &pub_escher::SpContainerObservation,
) -> Option<PubExplicitImageCropSource> {
    fn arm(
        shape: &pub_escher::SpContainerObservation,
        property_id: u16,
    ) -> (bool, Option<u32>, bool) {
        let properties = shape
            .fopts
            .iter()
            .flat_map(|record| record.properties.iter())
            .filter(|property| property.property_id() == property_id)
            .collect::<Vec<_>>();

        if properties.is_empty() {
            return (false, None, false);
        }

        if properties
            .iter()
            .any(|property| property.f_bid() || property.f_complex())
        {
            return (true, None, true);
        }

        let values = properties
            .iter()
            .map(|property| property.op)
            .collect::<BTreeSet<_>>();
        match values.len() {
            1 => (true, values.iter().next().copied(), false),
            _ => (true, None, true),
        }
    }

    let (top_present, top_raw, top_ambiguous) = arm(shape, OFFICE_ART_PROPERTY_CROP_FROM_TOP);
    let (bottom_present, bottom_raw, bottom_ambiguous) =
        arm(shape, OFFICE_ART_PROPERTY_CROP_FROM_BOTTOM);
    let (left_present, left_raw, left_ambiguous) = arm(shape, OFFICE_ART_PROPERTY_CROP_FROM_LEFT);
    let (right_present, right_raw, right_ambiguous) =
        arm(shape, OFFICE_ART_PROPERTY_CROP_FROM_RIGHT);

    if !(top_present || bottom_present || left_present || right_present) {
        return None;
    }

    Some(PubExplicitImageCropSource {
        top_raw,
        bottom_raw,
        left_raw,
        right_raw,
        ambiguous: top_ambiguous || bottom_ambiguous || left_ambiguous || right_ambiguous,
    })
}

fn direct_officeart_rgb(value: u32) -> Option<[u8; 3]> {
    // OfficeArtCOLORREF uses upper-byte flags for non-direct color forms.
    // This bounded path accepts only the unflagged direct RGB form.
    if value & 0xFF00_0000 != 0 {
        return None;
    }
    let bytes = value.to_le_bytes();
    Some([bytes[0], bytes[1], bytes[2]])
}

fn exact_image_slot(
    shape: &pub_escher::SpContainerObservation,
    seq_num: u32,
    diagnostics: &mut Vec<PubBridgeDiagnostic>,
) -> Option<u32> {
    let mut slots = shape
        .fopts
        .iter()
        .flat_map(|record| record.properties.iter())
        .filter(|property| {
            property.property_id() == OFFICE_ART_PROPERTY_PIB && property.op_is_blip_id()
        })
        .map(|property| property.op)
        .collect::<BTreeSet<_>>();

    match slots.len() {
        0 => None,
        1 => slots.pop_first(),
        _ => {
            diagnostics.push(PubBridgeDiagnostic::AmbiguousImageSlot {
                seq_num,
                slots: slots.into_iter().collect(),
            });
            None
        }
    }
}

#[derive(Debug)]
struct GroupedObjectProjection {
    page_id: PageId,
    bounds: RectEmu,
    depth: usize,
    group_sources: Vec<RawSpan>,
}

fn project_grouped_object_shape(
    first_group_seq: u32,
    child_shape: &pub_escher::SpContainerObservation,
    references: &BTreeMap<u32, Contents0x2cChunkReference>,
    page_seq_to_id: &BTreeMap<u32, PageId>,
    pages: &BTreeMap<PageId, Page>,
    escher_inventory: &SpContainerInventory,
    escher_by_contents_seq: &BTreeMap<u32, Vec<usize>>,
) -> Result<Option<GroupedObjectProjection>> {
    if shape_has_nonzero_rotation(child_shape)
        || shape_has_fsp_flag(child_shape, OFFICEART_FSP_FLIP_H)
        || shape_has_fsp_flag(child_shape, OFFICEART_FSP_FLIP_V)
    {
        bail!("grouped child has rotation or flip");
    }

    let child_anchor = child_shape
        .child_anchor
        .as_ref()
        .context("grouped child is missing ChildAnchor")?;
    let mut rect = coordinate_rect_i128(child_anchor)?;
    let mut current_shape = child_shape;
    let mut current_group_seq = first_group_seq;
    let mut seen = BTreeSet::new();
    let mut group_sources = Vec::new();

    for depth in 1..=2 {
        if !seen.insert(current_group_seq) {
            bail!("group ancestry cycle");
        }
        let group_reference = references
            .get(&current_group_seq)
            .context("group Contents reference missing")?;
        if single_raw_type(group_reference) != Some(RAW_TYPE_GROUP) {
            bail!("group ancestry raw type is not 0x30");
        }

        let group_matches = escher_by_contents_seq
            .get(&current_group_seq)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let group_shape = match group_matches {
            [index] => &escher_inventory.shapes[*index],
            [] => bail!("group Escher shape missing"),
            _ => bail!("group Escher shape ambiguous"),
        };
        if current_shape.parent_group_shape_source.as_ref() != Some(&group_shape.source) {
            bail!("OfficeArt parent-group link does not match Contents ancestry");
        }
        if shape_has_nonzero_rotation(group_shape)
            || shape_has_fsp_flag(group_shape, OFFICEART_FSP_FLIP_H)
            || shape_has_fsp_flag(group_shape, OFFICEART_FSP_FLIP_V)
        {
            bail!("group ancestor has rotation or flip");
        }

        let fspgr = group_shape
            .fspgr
            .as_ref()
            .context("group is missing FSPGR")?;
        let group_coords = coordinate_rect_i128(fspgr)?;
        group_sources.push(group_shape.source.clone());

        let parent_seq =
            single_parent_seq(group_reference).context("group parent is missing or ambiguous")?;
        if let Some(&page_id) = page_seq_to_id.get(&parent_seq) {
            let anchor = group_shape
                .client_anchor
                .as_ref()
                .context("top group is missing ClientAnchor")?;
            let absolute = publisher_anchor_rect_i128(anchor)?;
            rect = project_rect_trunc(rect, group_coords, absolute)?;
            let page = pages
                .get(&page_id)
                .context("group page id is missing from graph")?;
            let bounds = center_origin_rect_to_page_bounds(page, rect)?;
            return Ok(Some(GroupedObjectProjection {
                page_id,
                bounds,
                depth,
                group_sources,
            }));
        }

        if depth == 2 {
            bail!("group ancestry exceeds bounded depth 2");
        }
        if references.get(&parent_seq).and_then(single_raw_type) != Some(RAW_TYPE_GROUP) {
            bail!("group parent is neither DOCUMENT page nor group");
        }

        let placement = group_shape
            .child_anchor
            .as_ref()
            .context("nested group is missing ChildAnchor")?;
        rect = project_rect_trunc(rect, group_coords, coordinate_rect_i128(placement)?)?;
        current_shape = group_shape;
        current_group_seq = parent_seq;
    }

    Ok(None)
}

fn coordinate_rect_i128(rect: &pub_escher::OfficeArtCoordinateRect) -> Result<[i128; 4]> {
    let out = [
        i128::from(rect.x_left),
        i128::from(rect.y_top),
        i128::from(rect.x_right),
        i128::from(rect.y_bottom),
    ];
    if out[2] <= out[0] || out[3] <= out[1] {
        bail!("coordinate rectangle is non-positive");
    }
    Ok(out)
}

fn publisher_anchor_rect_i128(anchor: &PublisherFieldRecord) -> Result<[i128; 4]> {
    let xs = signed_field(anchor, PUBLISHER_FIELD_XS).context("group ClientAnchor missing XS")?;
    let ys = signed_field(anchor, PUBLISHER_FIELD_YS).context("group ClientAnchor missing YS")?;
    let xe = signed_field(anchor, PUBLISHER_FIELD_XE).context("group ClientAnchor missing XE")?;
    let ye = signed_field(anchor, PUBLISHER_FIELD_YE).context("group ClientAnchor missing YE")?;
    let out = [
        i128::from(xs),
        i128::from(ys),
        i128::from(xe),
        i128::from(ye),
    ];
    if out[2] <= out[0] || out[3] <= out[1] {
        bail!("group ClientAnchor rectangle is non-positive");
    }
    Ok(out)
}

/// Deterministic grouped-geometry projection.
///
/// Integer division in Rust truncates toward zero. Raw FSPGR/ChildAnchor
/// records remain authoritative provenance; these rounded EMU coordinates are
/// a derived runtime projection only.
fn project_rect_trunc(
    rect: [i128; 4],
    source_space: [i128; 4],
    target_space: [i128; 4],
) -> Result<[i128; 4]> {
    let x0 = project_axis_trunc(
        rect[0],
        source_space[0],
        source_space[2],
        target_space[0],
        target_space[2],
    )?;
    let y0 = project_axis_trunc(
        rect[1],
        source_space[1],
        source_space[3],
        target_space[1],
        target_space[3],
    )?;
    let x1 = project_axis_trunc(
        rect[2],
        source_space[0],
        source_space[2],
        target_space[0],
        target_space[2],
    )?;
    let y1 = project_axis_trunc(
        rect[3],
        source_space[1],
        source_space[3],
        target_space[1],
        target_space[3],
    )?;
    if x1 <= x0 || y1 <= y0 {
        bail!("projected grouped rectangle is non-positive");
    }
    Ok([x0, y0, x1, y1])
}

fn project_axis_trunc(
    value: i128,
    source_start: i128,
    source_end: i128,
    target_start: i128,
    target_end: i128,
) -> Result<i128> {
    let source_len = source_end - source_start;
    let target_len = target_end - target_start;
    if source_len <= 0 || target_len <= 0 {
        bail!("group projection has non-positive coordinate extent");
    }
    Ok(target_start + (value - source_start) * target_len / source_len)
}

fn center_origin_rect_to_page_bounds(page: &Page, rect: [i128; 4]) -> Result<RectEmu> {
    let half_width = i128::from(page.size.width.get()) / 2;
    let half_height = i128::from(page.size.height.get()) / 2;
    let x = half_width + rect[0];
    let y = half_height + rect[1];
    let width = rect[2] - rect[0];
    let height = rect[3] - rect[1];

    let to_i64 = |value: i128, label: &str| {
        i64::try_from(value).with_context(|| format!("{label} does not fit i64"))
    };
    Ok(RectEmu::new(
        LengthEmu::new(to_i64(x, "grouped x")?),
        LengthEmu::new(to_i64(y, "grouped y")?),
        LengthEmu::new(to_i64(width, "grouped width")?),
        LengthEmu::new(to_i64(height, "grouped height")?),
    ))
}

fn page_relative_bounds(page: &Page, anchor: &PublisherFieldRecord) -> Option<RectEmu> {
    let xs = signed_field(anchor, PUBLISHER_FIELD_XS)?;
    let ys = signed_field(anchor, PUBLISHER_FIELD_YS)?;
    let xe = signed_field(anchor, PUBLISHER_FIELD_XE)?;
    let ye = signed_field(anchor, PUBLISHER_FIELD_YE)?;

    let width = xe.checked_sub(xs)?;
    let height = ye.checked_sub(ys)?;
    if width <= 0 || height <= 0 {
        return None;
    }

    let x = page.size.width.get().checked_div(2)?.checked_add(xs)?;
    let y = page.size.height.get().checked_div(2)?.checked_add(ys)?;

    Some(RectEmu::new(
        LengthEmu::new(x),
        LengthEmu::new(y),
        LengthEmu::new(width),
        LengthEmu::new(height),
    ))
}

fn signed_field(record: &PublisherFieldRecord, id: u16) -> Option<i64> {
    let field = unique_escher_field(record, id)?;
    Some(i64::from(i32::from_le_bytes(field.value.to_le_bytes())))
}

fn unique_escher_field(record: &PublisherFieldRecord, id: u16) -> Option<&PublisherField> {
    let mut matches = record.fields.iter().filter(|field| field.id == id);
    let first = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(first)
}

fn anchor_has_unique_geometry_fields(anchor: &PublisherFieldRecord) -> bool {
    [
        PUBLISHER_FIELD_XS,
        PUBLISHER_FIELD_YS,
        PUBLISHER_FIELD_XE,
        PUBLISHER_FIELD_YE,
    ]
    .into_iter()
    .all(|id| unique_escher_field(anchor, id).is_some())
}

fn record_missing_anchor_fields(
    anchor: Option<&PublisherFieldRecord>,
    counts: &mut BTreeMap<String, usize>,
) {
    for (id, label) in [
        (PUBLISHER_FIELD_XS, "xs"),
        (PUBLISHER_FIELD_YS, "ys"),
        (PUBLISHER_FIELD_XE, "xe"),
        (PUBLISHER_FIELD_YE, "ye"),
    ] {
        if anchor
            .and_then(|record| unique_escher_field(record, id))
            .is_none()
        {
            *counts.entry(label.to_owned()).or_insert(0) += 1;
        }
    }
}

fn build_story_frame(
    source_hash: Sha256Digest,
    seq_num: u32,
    chunk: &Contents0x2cChunk,
    story_by_syid: &BTreeMap<u32, StoryId>,
    diagnostics: &mut Vec<PubBridgeDiagnostic>,
) -> Result<Option<PubStoryFrameSource>> {
    let Some((text_id, _)) = unique_u32_field(chunk, FIELD_STORY_ID)? else {
        return Ok(None);
    };

    let story_id = story_by_syid.get(&text_id).copied();
    if story_id.is_none() {
        diagnostics.push(PubBridgeDiagnostic::MissingQuillStory { seq_num, text_id });
    }

    let explicit_ordinal = unique_u32_field(chunk, FIELD_FRAME_ORDINAL)?.map(|(value, _)| value);
    let previous_seq = unique_u32_field(chunk, FIELD_PREVIOUS_FRAME)?.map(|(value, _)| value);
    let next_seq = unique_u32_field(chunk, FIELD_NEXT_FRAME)?.map(|(value, _)| value);

    let previous_frame = previous_seq
        .map(|target| derive_pub_node_id(&source_hash, target))
        .transpose()?;
    let next_frame = next_seq
        .map(|target| derive_pub_node_id(&source_hash, target))
        .transpose()?;

    Ok(Some(PubStoryFrameSource {
        text_id,
        story_id,
        explicit_ordinal,
        previous_seq_num: previous_seq,
        previous_frame,
        next_seq_num: next_seq,
        next_frame,
    }))
}

fn unique_story_id_scalar(chunk: &Contents0x2cChunk) -> Result<Option<u32>> {
    let mut matches = chunk
        .fields
        .iter()
        .filter(|field| field.id == FIELD_STORY_ID);
    let Some(field) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        bail!("duplicate Contents field 0x{FIELD_STORY_ID:02X} in one chunk");
    }

    match &field.body {
        RawContentsBlockBody::U16 { value, .. } => Ok(Some(u32::from(*value))),
        RawContentsBlockBody::U32 { value, .. } => Ok(Some(*value)),
        _ => bail!(
            "Contents Story field 0x{FIELD_STORY_ID:02X} at {} is not a confirmed u16/u32 scalar body",
            field.source.offset
        ),
    }
}

fn unique_u32_field(chunk: &Contents0x2cChunk, id: u16) -> Result<Option<(u32, RawSpan)>> {
    let mut matches = chunk.fields.iter().filter(|field| field.id == id);
    let Some(field) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        bail!("duplicate Contents field 0x{id:02X} in one chunk");
    }

    match &field.body {
        RawContentsBlockBody::U32 {
            value,
            value_source,
        } => Ok(Some((*value, value_source.clone()))),
        _ => bail!(
            "Contents field 0x{id:02X} at {} is not a confirmed u32/reference body",
            field.source.offset
        ),
    }
}

fn add_missing_link_target_diagnostics(
    graph: &PubSourceGraph,
    diagnostics: &mut Vec<PubBridgeDiagnostic>,
) {
    let node_ids = graph.nodes.keys().copied().collect::<BTreeSet<_>>();

    for node in graph.nodes.values() {
        let Some(frame) = node.payload.story_frame.as_ref() else {
            continue;
        };
        for (target_seq_num, target) in [
            (frame.previous_seq_num, frame.previous_frame),
            (frame.next_seq_num, frame.next_frame),
        ] {
            let (Some(target_seq_num), Some(target)) = (target_seq_num, target) else {
                continue;
            };
            if node_ids.contains(&target) {
                continue;
            }

            diagnostics.push(PubBridgeDiagnostic::LinkedFrameNotMaterialized {
                seq_num: node.payload.contents_seq_num,
                target_seq_num,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_hash() -> Sha256Digest {
        "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf"
            .parse()
            .expect("known SampleNewsletter SHA-256")
    }

    fn crop_test_span(offset: u64, len: u64) -> RawSpan {
        RawSpan {
            stream: StreamPath("Escher/EscherStm".to_owned()),
            offset,
            len,
        }
    }

    fn crop_test_property(property_id: u16, op: u32) -> pub_escher::Fopte {
        pub_escher::Fopte {
            opid: property_id,
            op,
            source: crop_test_span(0, 6),
            complex_source: None,
            complex_data: None,
        }
    }

    fn crop_test_shape(properties: Vec<pub_escher::Fopte>) -> pub_escher::SpContainerObservation {
        pub_escher::SpContainerObservation {
            source: crop_test_span(0, 0),
            parent_group_shape_source: None,
            fspgr: None,
            fsp: None,
            fopts: vec![pub_escher::FoptObservation {
                rec_type: 0xF00B,
                source: crop_test_span(0, 0),
                properties,
            }],
            client_anchor: None,
            client_data: None,
            client_textbox: None,
            child_anchor: None,
            unknown_children: Vec::new(),
        }
    }

    #[test]
    fn bounded_image_crop_preserves_unique_raw_scalars_and_marks_ambiguity() {
        let shape = crop_test_shape(vec![
            crop_test_property(OFFICE_ART_PROPERTY_CROP_FROM_TOP, 28_954),
            crop_test_property(OFFICE_ART_PROPERTY_CROP_FROM_BOTTOM, 21_446),
        ]);
        let crop = bounded_officeart_image_crop(&shape).expect("explicit crop");
        assert_eq!(crop.top_raw, Some(28_954));
        assert_eq!(crop.bottom_raw, Some(21_446));
        assert_eq!(crop.left_raw, None);
        assert_eq!(crop.right_raw, None);
        assert!(!crop.ambiguous);

        let conflicting = crop_test_shape(vec![
            crop_test_property(OFFICE_ART_PROPERTY_CROP_FROM_LEFT, 1),
            crop_test_property(OFFICE_ART_PROPERTY_CROP_FROM_LEFT, 2),
        ]);
        let crop = bounded_officeart_image_crop(&conflicting).expect("conflicting crop");
        assert_eq!(crop.left_raw, None);
        assert!(crop.ambiguous);

        let mut bid = crop_test_property(OFFICE_ART_PROPERTY_CROP_FROM_RIGHT, 7);
        bid.opid |= 0x4000;
        let crop = bounded_officeart_image_crop(&crop_test_shape(vec![bid]))
            .expect("fBid crop property remains visible but ambiguous");
        assert_eq!(crop.right_raw, None);
        assert!(crop.ambiguous);
    }

    #[test]
    fn grouped_projection_maps_full_fspgr_extent_exactly() {
        let source = [109_743_916, 106_908_792, 113_353_061, 109_780_257];
        let target = [-837_598, -3_276_408, 3_167_861, -89_633];

        assert_eq!(project_rect_trunc(source, source, target).unwrap(), target);
    }

    #[test]
    fn grouped_projection_matches_obs_017_child_298_with_truncation_toward_zero() {
        let group_coords = [109_743_916, 106_908_792, 113_353_061, 109_780_257];
        let group_absolute = [-837_598, -3_276_408, 3_167_861, -89_633];
        let child_anchor = [111_348_265, 107_124_104, 112_981_404, 108_383_122];

        assert_eq!(
            project_rect_trunc(child_anchor, group_coords, group_absolute).unwrap(),
            [942_921, -3_037_454, 2_755_392, -1_640_185]
        );
    }

    #[test]
    fn page_extent_consensus_accepts_one_extent() {
        assert_eq!(
            require_consensus_page_extent(&[(7_560_000, 10_692_000)]).unwrap(),
            (7_560_000, 10_692_000)
        );
    }

    #[test]
    fn page_extent_consensus_accepts_equivalent_duplicates() {
        assert_eq!(
            require_consensus_page_extent(&[
                (7_772_400, 10_058_400),
                (7_772_400, 10_058_400),
                (7_772_400, 10_058_400),
            ])
            .unwrap(),
            (7_772_400, 10_058_400)
        );
    }

    #[test]
    fn page_extent_consensus_rejects_conflicts() {
        let error =
            require_consensus_page_extent(&[(7_772_400, 10_058_400), (7_560_000, 10_692_000)])
                .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("conflicting Margins/OplMg page extents")
        );
    }

    #[test]
    fn page_extent_consensus_rejects_zero_dimension() {
        let error = require_consensus_page_extent(&[(7_772_400, 0)]).unwrap_err();
        assert!(error.to_string().contains("must be positive"));
    }

    #[test]
    fn direct_officeart_rgb_accepts_only_unflagged_colorref() {
        assert_eq!(direct_officeart_rgb(0x0000_00FF), Some([0xFF, 0x00, 0x00]));
        assert_eq!(direct_officeart_rgb(0x0000_FF00), Some([0x00, 0xFF, 0x00]));
        assert_eq!(direct_officeart_rgb(0x00FF_0000), Some([0x00, 0x00, 0xFF]));
        assert_eq!(direct_officeart_rgb(0x0800_0007), None);
    }

    #[test]
    fn officeart_visibility_masks_require_use_bits() {
        let fill_without_use = FILL_FILLED_BIT;
        let fill_with_use = FILL_USE_FILLED_BIT | FILL_FILLED_BIT;
        let line_without_use = LINE_LINE_BIT;
        let line_with_use = LINE_USE_LINE_BIT | LINE_LINE_BIT;

        assert_eq!(
            (fill_without_use & FILL_USE_FILLED_BIT != 0)
                .then_some(fill_without_use & FILL_FILLED_BIT != 0),
            None
        );
        assert_eq!(
            (fill_with_use & FILL_USE_FILLED_BIT != 0)
                .then_some(fill_with_use & FILL_FILLED_BIT != 0),
            Some(true)
        );
        assert_eq!(
            (line_without_use & LINE_USE_LINE_BIT != 0)
                .then_some(line_without_use & LINE_LINE_BIT != 0),
            None
        );
        assert_eq!(
            (line_with_use & LINE_USE_LINE_BIT != 0).then_some(line_with_use & LINE_LINE_BIT != 0),
            Some(true)
        );
    }

    #[test]
    fn source_object_key_vocabularies_are_explicit_and_disjoint() {
        assert_eq!(contents_object_key(330), "contents/0x2c/seq/330");
        assert_eq!(quill_story_object_key(22), "quill/syid/22");
    }

    #[test]
    fn node_and_story_identity_do_not_collapse_numeric_namespaces() {
        let hash = source_hash();
        let node = derive_pub_node_id(&hash, 22).unwrap();
        let story = derive_pub_story_id(&hash, 22).unwrap();

        assert_ne!(node.into_canonical(), story.into_canonical());
    }
}
