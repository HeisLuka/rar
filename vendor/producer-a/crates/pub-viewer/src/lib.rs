//! Read-only product boundary for PUB Viewer.
//!
//! This crate deliberately sits above the format-aware `pub-reader` adapter.
//! Its public document DTO is source-neutral: callers receive canonical model
//! identities, page sizes, semantic story text, stable viewer diagnostics, and
//! (for the visual path) an existing source-free `pub-layout` resolved scene.
//! CFB, Contents, Quill, Escher, byte offsets, and writer mutation state are not
//! part of the Viewer contract.

use anyhow::{Context, Result, anyhow};
use pub_layout::{
    BoundedAuthoringSlice, BoundedNodeGeometryInput, BoundedTextFlowEnvironment,
    BoundedTextMetrics, ProjectionDiagnostic, ResolveDiagnostic, ResolvedPhysicalNode,
    project_bounded, resolve_bounded_geometry, resolve_bounded_text_flow,
};
#[cfg(feature = "cmo-projection")]
use chaptera_layout_projection::{
    CarrierExtentV1, CmoStorySlotFlowInputV1, resolve_cmo_slot_flow_v1,
};
pub use pub_layout::{BoundedLayoutEnvironment, BoundedResolvedScene};
use pub_model::{
    Affine2D, CanonicalId, LengthEmu, NodeId, NodeKind, PageId, RectEmu, ResourceId, Sha256Digest,
    StoryFrame, StoryId,
};
use pub_presentation_profile::{
    CARLTON_PRESENTATION_INPUT_SCHEMA_V1, CarltonPageEvidenceV1, CarltonPresentationProfileInputV1,
    carlton_admitted_carrier_page_seq_nums_v1, select_carlton_customer_page_seq_nums_v1,
};
pub use pub_reader::{
    CHAPTERA_EXACT_FILE_CONSENT_V1, CHAPTERA_INTAKE_RETENTION_POLICY_V1, FailureIntakeClass,
    FailureIntakeClassification, FailureIntakeConfidence, FailureIntakeReason,
    classify_failure_candidate, exact_file_intake_eligible,
};
use pub_reader::{
    FailureCode, FailureEnvelope, FailureEnvelopeContext, FailureParserStage,
    FailureTelemetryChoice, PubAssetExportDiagnostic, PubBridgeDiagnostic, PubResolveDiagnostic,
    PubResolvedGraph, PubResolvedGraphBuild, PubSourceGraphBuild, analyze_mature_0x2c_page_roles,
    build_failure_envelope, build_mature_0x2c_asset_export_bundle_from_bytes,
    build_mature_0x2c_source_graph, derive_pub_page_id, resolve_pub_source_graph,
};
#[cfg(feature = "cmo-projection")]
use pub_reader::build_mature_0x2c_cmo_projection_bridge_v1;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

pub const VIEWER_DOCUMENT_SCHEMA_V0_1: &str = "0.1";
pub const VIEWER_GEOMETRY_SCHEMA_V0_1: &str = "0.1";

pub const VIEWER_FAILURE_REPORT_SCHEMA_V0_1: &str = "chaptera-viewer-failure-report/v0.1";
pub const VIEWER_FALLBACK_TEXT_METRICS_REVISION_V0_1: &str = "viewer-fallback-text-metrics-v0.1";
const VIEWER_FALLBACK_SCALAR_ADVANCE_EMU_V0_1: i64 = 57_150;
const VIEWER_FALLBACK_LINE_HEIGHT_EMU_V0_1: i64 = 142_875;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerFailureDiagnosticReport {
    pub schema_version: String,
    pub envelope: FailureEnvelope,
}

/// Builds an inspectable, local-only diagnostic report for a failed Reader open.
///
/// The report deliberately reuses the allowlisted structural failure envelope:
/// it contains no filesystem path, filename, exact file hash, document text,
/// images, raw streams, or document bytes. This function performs no network
/// I/O and does not imply that the file is eligible for corpus submission.
pub fn build_local_failure_diagnostic_report(
    bytes: &[u8],
) -> Result<ViewerFailureDiagnosticReport> {
    let classification = classify_failure_candidate(bytes);
    let envelope = build_failure_envelope(
        FailureTelemetryChoice::MinimalStructural,
        &classification,
        FailureEnvelopeContext {
            parser_stage: FailureParserStage::PubReaderOpen,
            failure_code: FailureCode::OpenFailed,
            timeout: false,
            resource_limit: false,
            byte_len: bytes.len(),
            os_family: None,
            architecture: None,
            coarse_locale: None,
        },
    )
    .expect("minimal structural failure envelope must be enabled");

    Ok(ViewerFailureDiagnosticReport {
        schema_version: VIEWER_FAILURE_REPORT_SCHEMA_V0_1.to_owned(),
        envelope,
    })
}

/// Serializes the local diagnostic report as human-inspectable JSON.
///
/// The caller owns choosing a destination path and writing the returned bytes.
pub fn local_failure_diagnostic_json(bytes: &[u8]) -> Result<String> {
    let report = build_local_failure_diagnostic_report(bytes)?;
    serde_json::to_string_pretty(&report).context("serialize local Chaptera failure diagnostics")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerDocument {
    pub schema_version: String,
    pub source: ViewerSource,
    pub pages: Vec<ViewerPage>,
    pub stories: Vec<ViewerStory>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<ViewerDiagnostic>,
}

impl ViewerDocument {
    /// Product-facing visual fidelity status for a successfully opened document.
    ///
    /// `Unsupported` is reserved for the application open boundary: once a
    /// `ViewerDocument` exists, the document is either supported within the
    /// current scope or partial because one or more known fidelity warnings
    /// remain.
    pub fn fidelity_status(&self) -> ViewerFidelityStatus {
        if self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == ViewerDiagnosticSeverity::FidelityWarning)
        {
            ViewerFidelityStatus::Partial
        } else {
            ViewerFidelityStatus::Supported
        }
    }

    /// Exact search over recovered semantic story text.
    ///
    /// Results intentionally carry a stable StoryId but no page association:
    /// the current ViewerDocument contract does not expose a proven
    /// story/frame-to-page relation, and the Viewer must not invent one.
    pub fn search_text(&self, query: &str) -> Vec<ViewerTextMatch> {
        if query.is_empty() {
            return Vec::new();
        }

        let mut matches = Vec::new();
        for story in &self.stories {
            for (start, matched) in story.text.match_indices(query) {
                let end = start + matched.len();
                matches.push(ViewerTextMatch {
                    story_id: story.id,
                    start_byte: u64::try_from(start)
                        .expect("Viewer story byte offset must fit into u64"),
                    end_byte: u64::try_from(end)
                        .expect("Viewer story byte offset must fit into u64"),
                    text: matched.to_owned(),
                });
            }
        }
        matches
    }
}

/// First real visual Viewer handoff.
///
/// `document` owns application order/text/diagnostics. `scene` is the
/// source-free physical geometry produced by the existing layout boundary. It
/// intentionally does not claim that text, images, fill/line, or effects have
/// been painted yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerGeometryDocument {
    pub schema_version: String,
    pub document: ViewerDocument,
    pub scene: BoundedResolvedScene,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paints: Vec<ViewerNodePaint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub story_frames: Vec<ViewerStoryFrame>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_fragments: Vec<ViewerTextFragment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub typography_runs: Vec<ViewerTypographyRun>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ViewerEmbeddedImage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projected_instances: Vec<ViewerProjectedInstance>,
}

impl ViewerGeometryDocument {
    /// Reprojects Viewer Story text and bounded frame fragments from the current
    /// resolved graph without reparsing the immutable source PUB.
    ///
    /// This is the authoring-to-Viewer synchronization seam: EditorSession owns
    /// canonical mutations, while the Viewer continues to consume the same
    /// source-neutral pub-layout text-flow authority used during initial open.
    /// The update is transactional: on projection failure the existing Viewer
    /// text state is left untouched.
    pub fn refresh_text_projection_from_resolved(
        &mut self,
        graph: &PubResolvedGraph,
    ) -> Result<()> {
        if graph.source.source_hash != self.document.source.source_hash
            || graph.document.source_hash != self.document.source.source_hash
        {
            return Err(anyhow!(
                "Viewer text refresh rejected a resolved graph with different source identity"
            ));
        }

        let effective_page_ids = self
            .document
            .pages
            .iter()
            .map(|page| page.id)
            .collect::<Vec<_>>();
        let authoring = bounded_authoring_slice_from_resolved_pages(graph, &effective_page_ids)?;
        let projection = project_bounded(authoring);
        let (mut text_fragments, text_flow_diagnostics) = resolve_viewer_text_fragments(&projection)?;

        let stories = graph
            .stories
            .values()
            .map(|story| ViewerStory {
                id: story.id,
                text: story.text.clone(),
            })
            .collect::<Vec<_>>();
        let mut story_frames = projection
            .story_frames
            .iter()
            .map(|frame| ViewerStoryFrame {
                story_id: frame.story_origin,
                frame_id: frame.frame_origin,
                ordinal: frame.ordinal,
            })
            .collect::<Vec<_>>();

        let mut diagnostics = self.document.diagnostics.clone();
        diagnostics
            .retain(|diagnostic| !is_refreshable_text_flow_diagnostic(diagnostic.code.as_str()));
        diagnostics.extend(text_flow_diagnostics.iter().map(map_scene_diagnostic));
        if !text_fragments.is_empty() {
            diagnostics.push(viewer_fallback_flow_metrics_diagnostic());
        }
        normalize_diagnostics(&mut diagnostics);

        self.document.stories = stories;
        self.story_frames = story_frames;
        self.text_fragments = text_fragments;
        self.document.diagnostics = diagnostics;
        Ok(())
    }
    /// Synchronizes the bounded author-created direct page-local TextFrame class
    /// into the current Viewer scene without rebuilding or deleting unrelated
    /// projected/inherited scene instances.
    ///
    /// The active IDs must come from a higher-layer durable creation proof
    /// (currently applied CreateTextBox operations). The previous IDs are
    /// transient product state used only to remove this same managed class on
    /// Undo/reopen; they are never treated as authoring truth.
    pub fn sync_editor_created_text_box_scene_nodes(
        &mut self,
        graph: &PubResolvedGraph,
        active_node_ids: &[NodeId],
        previously_synced_node_ids: &BTreeSet<NodeId>,
    ) -> Result<BTreeSet<NodeId>> {
        if graph.source.source_hash != self.document.source.source_hash
            || graph.document.source_hash != self.document.source.source_hash
        {
            return Err(anyhow!(
                "Viewer created-node scene sync rejected a resolved graph with different source identity"
            ));
        }

        let mut seen = BTreeSet::new();
        for node_id in active_node_ids {
            if !seen.insert(*node_id) {
                return Err(anyhow!(
                    "Viewer created-node scene sync received duplicate NodeId {}",
                    node_id.as_canonical()
                ));
            }
        }

        let surface_ids = self
            .scene
            .surfaces
            .iter()
            .map(|surface| surface.origin.into_canonical())
            .collect::<BTreeSet<_>>();

        let mut next_nodes = self
            .scene
            .nodes
            .iter()
            .filter(|node| !previously_synced_node_ids.contains(&node.origin))
            .cloned()
            .collect::<Vec<_>>();

        for node_id in active_node_ids {
            let node = graph.nodes.get(node_id).ok_or_else(|| {
                anyhow!(
                    "Viewer created-node scene sync missing current graph node {}",
                    node_id.as_canonical()
                )
            })?;
            if node.kind != NodeKind::TextFrame
                || !node.header.source_refs.is_empty()
                || node.header.transform != Affine2D::identity()
                || node.header.bounds.width.get() <= 0
                || node.header.bounds.height.get() <= 0
                || node.header.bounds.right().is_none()
                || node.header.bounds.bottom().is_none()
            {
                return Err(anyhow!(
                    "Viewer created-node scene sync rejected unsupported TextFrame {}",
                    node_id.as_canonical()
                ));
            }
            if !surface_ids.contains(&node.header.parent_id) {
                return Err(anyhow!(
                    "Viewer created-node scene sync rejected non-page parent for {}",
                    node_id.as_canonical()
                ));
            }
            let parent_page = graph
                .pages
                .values()
                .find(|page| page.id.as_canonical() == &node.header.parent_id)
                .ok_or_else(|| {
                    anyhow!(
                        "Viewer created-node scene sync could not resolve parent page for {}",
                        node_id.as_canonical()
                    )
                })?;
            if !parent_page.children.contains(node_id) {
                return Err(anyhow!(
                    "Viewer created-node scene sync rejected missing page-child membership for {}",
                    node_id.as_canonical()
                ));
            }
            let story_id = node
                .payload
                .story_frame
                .as_ref()
                .and_then(|frame| frame.story_id)
                .ok_or_else(|| {
                    anyhow!(
                        "Viewer created-node scene sync requires one StoryFrame owner for {}",
                        node_id.as_canonical()
                    )
                })?;
            if !graph.stories.contains_key(&story_id) {
                return Err(anyhow!(
                    "Viewer created-node scene sync missing Story {} for TextFrame {}",
                    story_id.as_canonical(),
                    node_id.as_canonical()
                ));
            }
            if next_nodes
                .iter()
                .any(|scene_node| scene_node.origin == *node_id)
            {
                return Err(anyhow!(
                    "Viewer created-node scene sync collided with an unrelated scene node {}",
                    node_id.as_canonical()
                ));
            }

            next_nodes.push(ResolvedPhysicalNode {
                origin: *node_id,
                parent_origin: node.header.parent_id,
                bounds: node.header.bounds,
                transform: node.header.transform.clone(),
            });
        }

        self.scene.nodes = next_nodes;
        Ok(seen)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerNodePaint {
    pub node_id: NodeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solid_fill_rgb: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solid_line: Option<ViewerSolidLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerSolidLine {
    pub rgb: [u8; 3],
    pub width_emu: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerStoryFrame {
    pub story_id: StoryId,
    pub frame_id: NodeId,
    pub ordinal: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerTextFragment {
    pub story_id: StoryId,
    pub frame_id: NodeId,
    pub scalar_start: u32,
    pub scalar_end: u32,
    pub text: String,
    pub line_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerProjectedInstance {
    pub instance_id: String,
    pub projection_kind: String,
    pub origin_node_id: NodeId,
    pub target_page_id: PageId,
    pub bounds: RectEmu,
    pub transform: Affine2D,
    pub read_only: bool,
    pub target_story_id: StoryId,
    pub target_frame_node_id: NodeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carrier_story_id: Option<StoryId>,
    pub slot_index: usize,
    pub scalar_index: u32,
    pub source_order: usize,
    pub cmo_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerTypographyRun {
    pub story_id: StoryId,
    pub scalar_start: u32,
    pub scalar_end: u32,
    pub source_font_name: String,
    pub text_size_emu: u32,
    pub font_inherited: bool,
    pub size_inherited: bool,
    pub source_story_text_sha256: Sha256Digest,
}

impl ViewerTypographyRun {
    pub fn applies_to_story_text(&self, text: &str) -> bool {
        self.source_story_text_sha256 == viewer_story_text_sha256(text)
    }
}

pub fn viewer_story_text_sha256(text: &str) -> Sha256Digest {
    let digest = Sha256::digest(text.as_bytes());
    let mut bytes = [0_u8; 32];
    bytes.copy_from_slice(&digest);
    Sha256Digest::from_bytes(bytes)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerEmbeddedImage {
    pub resource_id: ResourceId,
    pub mime: String,
    pub node_ids: Vec<NodeId>,
    #[serde(skip)]
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerSource {
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_version: Option<String>,
    pub source_hash: Sha256Digest,
    pub byte_len: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerPage {
    /// One-based document order, independent from source directory numbering.
    pub index: u32,
    pub id: PageId,
    pub width_emu: i64,
    pub height_emu: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerStory {
    pub id: StoryId,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerTextMatch {
    pub story_id: StoryId,
    pub start_byte: u64,
    pub end_byte: u64,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewerFidelityStatus {
    Supported,
    Partial,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewerDiagnosticSeverity {
    Info,
    FidelityWarning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewerDiagnostic {
    /// Stable product-facing code. It must not encode source byte offsets or
    /// parser-private carrier names.
    pub code: String,
    pub severity: ViewerDiagnosticSeverity,
    pub message: String,
}

struct Mature0x2cPipeline {
    source_hash: Sha256Digest,
    source: PubSourceGraphBuild,
    resolved: PubResolvedGraphBuild,
    page_selection: ViewerPageSelection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ViewerPageSelection {
    page_ids: Vec<PageId>,
    disposition: ViewerPageSelectionDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ViewerPageSelectionDisposition {
    GenericNoLoss,
    FamilyProfileApplied {
        profile_id: String,
        raw_page_count: usize,
        customer_page_count: usize,
    },
    FamilyProfileUnavailable {
        reason: String,
    },
}

/// Opens one mature 0x2C Publisher file into the read-only Viewer manifest.
///
/// This function never writes to the supplied bytes and does not construct a
/// writer/mutation plan. Unsupported or ambiguous source observations that do
/// not make the bounded adapter fail are translated to stable Viewer
/// diagnostics instead of being silently discarded.
pub fn open_mature_0x2c(bytes: &[u8]) -> Result<ViewerDocument> {
    let pipeline = build_mature_0x2c_pipeline(bytes)?;
    viewer_document_from_pipeline(bytes.len(), &pipeline)
}

/// Opens one mature 0x2C Publisher file through the real layout/scene boundary.
///
/// Current scope is deliberately geometry-only. The scene contains page
/// surfaces and physical node rectangles/transforms. Recovered text remains in
/// `ViewerDocument` for search/copy and the scene emits an explicit fidelity
/// warning that text layout is not yet painted.
pub fn open_mature_0x2c_geometry(
    bytes: &[u8],
    environment: BoundedLayoutEnvironment,
) -> Result<ViewerGeometryDocument> {
    let pipeline = build_mature_0x2c_pipeline(bytes)?;
    let mut document = viewer_document_from_pipeline(bytes.len(), &pipeline)?;
    let effective_page_ids = document
        .pages
        .iter()
        .map(|page| page.id)
        .collect::<Vec<_>>();
    let authoring =
        bounded_authoring_slice_from_resolved_pages(&pipeline.resolved.graph, &effective_page_ids)?;
    let projection = project_bounded(authoring);

    document
        .diagnostics
        .extend(projection.diagnostics.iter().map(map_projection_diagnostic));

    let paints = pipeline
        .resolved
        .graph
        .nodes
        .values()
        .filter_map(|node| {
            let paint = &node.payload.explicit_paint;
            let solid_fill_rgb = (paint.fill.solid && paint.fill.visible == Some(true))
                .then_some(paint.fill.color_rgb)
                .flatten();
            let solid_line = match (
                paint.line.visible,
                paint.line.color_rgb,
                paint.line.width_emu,
            ) {
                (Some(true), Some(rgb), Some(width_emu)) if width_emu > 0 => {
                    Some(ViewerSolidLine { rgb, width_emu })
                }
                _ => None,
            };

            if solid_fill_rgb.is_none() && solid_line.is_none() {
                None
            } else {
                Some(ViewerNodePaint {
                    node_id: node.header.id,
                    solid_fill_rgb,
                    solid_line,
                })
            }
        })
        .collect::<Vec<_>>();

    let story_frames = projection
        .story_frames
        .iter()
        .map(|frame| ViewerStoryFrame {
            story_id: frame.story_origin,
            frame_id: frame.frame_origin,
            ordinal: frame.ordinal,
        })
        .collect::<Vec<_>>();

    let (text_fragments, text_flow_diagnostics) = resolve_viewer_text_fragments(&projection)?;
    document
        .diagnostics
        .extend(text_flow_diagnostics.iter().map(map_scene_diagnostic));
    if !text_fragments.is_empty() {
        document
            .diagnostics
            .push(viewer_fallback_flow_metrics_diagnostic());
    }

    let typography_runs = pipeline
        .source
        .typography_runs
        .iter()
        .filter_map(|run| {
            let story = pipeline.resolved.graph.stories.get(&run.story_id)?;
            Some(ViewerTypographyRun {
                story_id: run.story_id,
                scalar_start: run.story_scalar_start,
                scalar_end: run.story_scalar_end,
                source_font_name: run.source_font_name.clone(),
                text_size_emu: run.text_size_emu,
                font_inherited: run.font_inherited,
                size_inherited: run.size_inherited,
                source_story_text_sha256: viewer_story_text_sha256(&story.text),
            })
        })
        .collect::<Vec<_>>();
    if !typography_runs.is_empty() {
        let inherited = typography_runs
            .iter()
            .filter(|run| run.font_inherited || run.size_inherited)
            .count();
        document.diagnostics.push(ViewerDiagnostic {
            code: "viewer.text.source_typography_partial".to_owned(),
            severity: ViewerDiagnosticSeverity::FidelityWarning,
            message: format!(
                "{} source typography range(s) are available for preview sizing; {} use bounded inherited font/size authority. The renderer still uses the pinned fallback font face and does not claim Publisher-exact reflow.",
                typography_runs.len(),
                inherited
            ),
        });
    }

    let images = match build_mature_0x2c_asset_export_bundle_from_bytes(
        bytes,
        &pipeline.source.graph,
    ) {
        Ok(bundle) => {
            for diagnostic in &bundle.manifest.diagnostics {
                match diagnostic {
                    PubAssetExportDiagnostic::AssetNotPromoted { .. } => {
                        document.diagnostics.push(ViewerDiagnostic {
                            code: "viewer.image.asset_not_exact".to_owned(),
                            severity: ViewerDiagnosticSeverity::FidelityWarning,
                            message: "An image placement exists, but exact embedded image bytes are not available for the Viewer overlay.".to_owned(),
                        });
                    }
                }
            }

            bundle
                .files
                .into_iter()
                .filter_map(|file| {
                    let entry = bundle
                        .manifest
                        .assets
                        .iter()
                        .find(|entry| entry.resource_id == file.resource_id)?;
                    Some(ViewerEmbeddedImage {
                        resource_id: file.resource_id,
                        mime: entry.mime.clone(),
                        node_ids: entry.uses.iter().map(|usage| usage.node_id).collect(),
                        bytes: file.bytes,
                    })
                })
                .collect::<Vec<_>>()
        }
        Err(_) => {
            document.diagnostics.push(ViewerDiagnostic {
                code: "viewer.image.asset_pipeline_unavailable".to_owned(),
                severity: ViewerDiagnosticSeverity::FidelityWarning,
                message: "The Viewer could not materialize the exact embedded-image resource bundle for this document.".to_owned(),
            });
            Vec::new()
        }
    };

    let scene = resolve_bounded_geometry(&projection, environment).map_err(|blocked| {
        let codes = blocked
            .projection_errors
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        anyhow!("Viewer geometry resolution blocked by layout projection errors: {codes}")
    })?;

    let mut projected_instances = Vec::new();
    #[cfg(feature = "cmo-projection")]
    if matches!(
        pipeline.page_selection.disposition,
        ViewerPageSelectionDisposition::FamilyProfileApplied { .. }
    ) {
        match materialize_viewer_cmo_projection_v1(bytes, &pipeline) {
            Ok(materialized) => {
                projected_instances = materialized.instances;
                story_frames.extend(materialized.story_frames);
                text_fragments.extend(materialized.text_fragments);
                document.diagnostics.extend(materialized.diagnostics);
            }
            Err(error) => document.diagnostics.push(ViewerDiagnostic {
                code: "viewer.cmo_projection.unavailable".to_owned(),
                severity: ViewerDiagnosticSeverity::FidelityWarning,
                message: format!(
                    "Admitted family Cmo authority could not be materialized safely: {error}"
                ),
            }),
        }
    }

    document.diagnostics.extend(
        scene
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code != "story_text_layout_not_implemented")
            .map(map_scene_diagnostic),
    );

    if !scene.nodes.is_empty() {
        document.diagnostics.push(ViewerDiagnostic {
            code: "viewer.visual.geometry_only".to_owned(),
            severity: ViewerDiagnosticSeverity::FidelityWarning,
            message: "Object positions and sizes are resolved. The desktop Viewer may paint bounded semantic text, including explicit linked-frame chains and admitted source font sizes (including bounded inheritance) through Viewer fallback font metrics, plus exact embedded PNG/JPEG bytes and complete explicit shape-local solid fill/line state when available. Other inherited/default styling beyond admitted font/size, Publisher-exact typography/reflow, image crop/fit, gradients/patterns, effects, and transforms are not faithfully painted yet."
                .to_owned(),
        });
    }
    normalize_diagnostics(&mut document.diagnostics);

    Ok(ViewerGeometryDocument {
        schema_version: VIEWER_GEOMETRY_SCHEMA_V0_1.to_owned(),
        document,
        scene,
        paints,
        story_frames,
        text_fragments,
        typography_runs,
        images,
        projected_instances,
    })
}

#[cfg(feature = "cmo-projection")]
struct ViewerCmoMaterializationV1 {
    instances: Vec<ViewerProjectedInstance>,
    story_frames: Vec<ViewerStoryFrame>,
    text_fragments: Vec<ViewerTextFragment>,
    diagnostics: Vec<ViewerDiagnostic>,
}

#[cfg(feature = "cmo-projection")]
fn canonical_from_bridge_id(value: &str, label: &str) -> Result<CanonicalId> {
    value
        .parse()
        .map_err(|error| anyhow!("{label} is not a canonical id: {error:?}"))
}

#[cfg(feature = "cmo-projection")]
fn materialize_viewer_cmo_projection_v1(
    bytes: &[u8],
    pipeline: &Mature0x2cPipeline,
) -> Result<ViewerCmoMaterializationV1> {
    let bridge = build_mature_0x2c_cmo_projection_bridge_v1(
        bytes,
        pipeline.source_hash,
        &pipeline.source.graph,
        &pipeline.resolved.graph,
    )
    .context("build active Reader Cmo authority bridge")?;
    if !bridge.active_graph_identity_parity {
        return Err(anyhow!("Cmo authority bridge did not prove active graph identity parity"));
    }

    let graph = &pipeline.resolved.graph;
    let context = &bridge.output.context;
    let target_story_ids = context
        .cmo_relations
        .iter()
        .map(|relation| relation.target_story_id.clone())
        .collect::<BTreeSet<_>>();
    let page_by_canonical = graph
        .pages
        .values()
        .map(|page| (page.id.into_canonical(), page.id))
        .collect::<BTreeMap<_, _>>();
    let effective_pages = pipeline
        .page_selection
        .page_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    let mut diagnostics = Vec::new();
    let mut grouped = BTreeMap::<(u32, String, String), Vec<_>>::new();
    let mut unresolved_frame_relations = 0usize;
    for relation in context.cmo_relations.iter().cloned() {
        let Some(frame_id) = relation.target_frame_node_id.clone() else {
            unresolved_frame_relations += 1;
            continue;
        };
        grouped
            .entry((
                relation.target_qsid,
                relation.target_story_id.clone(),
                frame_id,
            ))
            .or_default()
            .push(relation);
    }

    let mut instances = Vec::new();
    let mut instance_ids = BTreeSet::new();
    let mut visible_carrier_nodes = BTreeSet::new();
    let mut lower_bound_targets = 0usize;
    let mut withheld_nonzero_slots = 0usize;
    let mut overset_targets = 0usize;

    for ((target_qsid, target_story_text, target_frame_text), relations) in grouped {
        let target_story_id = StoryId::from_canonical(canonical_from_bridge_id(
            &target_story_text,
            "Cmo target StoryId",
        )?);
        let target_frame_node_id = NodeId::from_canonical(canonical_from_bridge_id(
            &target_frame_text,
            "Cmo target frame NodeId",
        )?);
        let story = graph
            .stories
            .get(&target_story_id)
            .ok_or_else(|| anyhow!("Cmo target Story missing from active graph"))?;
        let target_frame = graph
            .nodes
            .get(&target_frame_node_id)
            .ok_or_else(|| anyhow!("Cmo target frame missing from active graph"))?;
        if target_frame
            .payload
            .story_frame
            .as_ref()
            .and_then(|frame| frame.story_id)
            != Some(target_story_id)
        {
            return Err(anyhow!("Cmo target frame/Story ownership mismatch"));
        }
        let target_page_id = *page_by_canonical
            .get(&target_frame.header.parent_id)
            .ok_or_else(|| anyhow!("Cmo target frame parent is not a canonical page"))?;
        if !effective_pages.contains(&target_page_id) {
            continue;
        }

        let frame_count = graph
            .nodes
            .values()
            .filter(|node| {
                node.payload
                    .story_frame
                    .as_ref()
                    .and_then(|frame| frame.story_id)
                    == Some(target_story_id)
            })
            .count();
        let frame_count = u32::try_from(frame_count).context("Cmo target frame count exceeds u32")?;

        let object_marker_scalars = story
            .text
            .chars()
            .enumerate()
            .filter_map(|(index, ch)| {
                (ch == '￼')
                    .then(|| u32::try_from(index).context("Cmo marker scalar exceeds u32"))
            })
            .collect::<Result<Vec<_>>>()?;

        let has_non_marker_text = story
            .text
            .chars()
            .any(|ch| ch != '￼' && !ch.is_whitespace());
        if has_non_marker_text {
            lower_bound_targets += 1;
        }

        let mut carrier_extents = Vec::with_capacity(relations.len());
        for relation in &relations {
            let carrier_node_id = NodeId::from_canonical(canonical_from_bridge_id(
                &relation.carrier_node_id,
                "Cmo carrier NodeId",
            )?);
            let carrier = graph
                .nodes
                .get(&carrier_node_id)
                .ok_or_else(|| anyhow!("Cmo carrier missing from active graph"))?;
            carrier_extents.push(CarrierExtentV1 {
                carrier_node_id: relation.carrier_node_id.clone(),
                width_emu: carrier.header.bounds.width.get(),
                height_emu: carrier.header.bounds.height.get(),
                nested_cmo: relation
                    .carrier_story_id
                    .as_ref()
                    .is_some_and(|story_id| target_story_ids.contains(story_id)),
            });
        }

        let output = resolve_cmo_slot_flow_v1(
            context,
            &CmoStorySlotFlowInputV1 {
                target_qsid,
                target_page_id: target_page_id.as_canonical().to_string(),
                target_story_id: target_story_text.clone(),
                target_frame_node_id: target_frame_text.clone(),
                frame_count,
                host_width_emu: target_frame.header.bounds.width.get(),
                host_height_emu: target_frame.header.bounds.height.get(),
                object_marker_scalars,
                // The active Viewer does not yet expose shaped per-line scalar
                // coverage to this seam. Empty lines therefore mean lower-bound
                // slot flow. Only scalar-0 visible slots are admitted below.
                text_lines: Vec::new(),
                carrier_extents,
            },
        )
        .map_err(|error| anyhow!("canonical Cmo slot-flow rejected target {target_qsid}: {error}"))?;

        if output.overset.story_overset {
            overset_targets += 1;
        }

        for slot in output.visible_slots {
            // Without shaped interleave, a slot after any Story scalar would need
            // preceding line metrics. Scalar zero has no preceding text by
            // construction, so it is the only exact placement admitted here.
            if slot.scalar_index != 0 {
                withheld_nonzero_slots += 1;
                continue;
            }

            let carrier_node_id = NodeId::from_canonical(canonical_from_bridge_id(
                &slot.carrier_node_id,
                "visible Cmo carrier NodeId",
            )?);
            let carrier_story_id = slot
                .carrier_story_id
                .as_deref()
                .map(|value| {
                    canonical_from_bridge_id(value, "visible Cmo carrier StoryId")
                        .map(StoryId::from_canonical)
                })
                .transpose()?;
            let x = target_frame
                .header
                .bounds
                .x
                .get()
                .checked_add(slot.resolved_x_emu)
                .ok_or_else(|| anyhow!("projected Cmo x overflow"))?;
            let y = target_frame
                .header
                .bounds
                .y
                .get()
                .checked_add(slot.resolved_y_emu)
                .ok_or_else(|| anyhow!("projected Cmo y overflow"))?;
            if !instance_ids.insert(slot.instance_id.clone()) {
                return Err(anyhow!("duplicate projected Cmo instance identity"));
            }

            visible_carrier_nodes.insert(carrier_node_id);
            instances.push(ViewerProjectedInstance {
                instance_id: slot.instance_id,
                projection_kind: "cmo_story_slot".to_owned(),
                origin_node_id: carrier_node_id,
                target_page_id,
                bounds: RectEmu::new(
                    LengthEmu::new(x),
                    LengthEmu::new(y),
                    LengthEmu::new(slot.resolved_width_emu),
                    LengthEmu::new(slot.resolved_height_emu),
                ),
                transform: Affine2D::identity(),
                read_only: true,
                target_story_id,
                target_frame_node_id,
                carrier_story_id,
                slot_index: slot.slot_index,
                scalar_index: slot.scalar_index,
                source_order: slot.source_order,
                cmo_id: slot.cmo_id,
            });
        }
    }

    // Resolve carrier text through the existing Viewer fallback-flow authority
    // on the carriers' original pages, then attach only fragments belonging to
    // actually visible projected carrier origins. Raw carrier PAGE surfaces are
    // never added to the customer scene.
    let mut carrier_pages = BTreeSet::new();
    for node_id in &visible_carrier_nodes {
        let node = graph
            .nodes
            .get(node_id)
            .ok_or_else(|| anyhow!("visible Cmo carrier disappeared from active graph"))?;
        let page_id = *page_by_canonical
            .get(&node.header.parent_id)
            .ok_or_else(|| anyhow!("visible Cmo carrier parent is not a canonical page"))?;
        carrier_pages.insert(page_id);
    }

    let mut story_frames = Vec::new();
    let mut text_fragments = Vec::new();
    for page_id in carrier_pages {
        let carrier_authoring =
            bounded_authoring_slice_from_resolved_pages(graph, &[page_id])?;
        let carrier_projection = project_bounded(carrier_authoring);
        let (carrier_fragments, _) = resolve_viewer_text_fragments(&carrier_projection)?;
        text_fragments.extend(
            carrier_fragments
                .into_iter()
                .filter(|fragment| visible_carrier_nodes.contains(&fragment.frame_id)),
        );
        story_frames.extend(
            carrier_projection
                .story_frames
                .iter()
                .filter(|frame| visible_carrier_nodes.contains(&frame.frame_origin))
                .map(|frame| ViewerStoryFrame {
                    story_id: frame.story_origin,
                    frame_id: frame.frame_origin,
                    ordinal: frame.ordinal,
                }),
        );
    }

    text_fragments.sort_by_key(|fragment| {
        (fragment.story_id, fragment.scalar_start, fragment.frame_id)
    });
    text_fragments.dedup_by(|left, right| {
        left.story_id == right.story_id
            && left.frame_id == right.frame_id
            && left.scalar_start == right.scalar_start
            && left.scalar_end == right.scalar_end
    });
    story_frames.sort_by_key(|frame| (frame.story_id, frame.ordinal, frame.frame_id));
    story_frames.dedup();

    let carrier_text_frames = text_fragments
        .iter()
        .map(|fragment| fragment.frame_id)
        .collect::<BTreeSet<_>>();
    let missing_carrier_text = instances
        .iter()
        .filter(|instance| instance.carrier_story_id.is_some())
        .filter(|instance| !carrier_text_frames.contains(&instance.origin_node_id))
        .count();

    instances.sort_by(|left, right| {
        (
            left.target_page_id,
            left.target_story_id,
            left.scalar_index,
            left.instance_id.as_str(),
        )
            .cmp(&(
                right.target_page_id,
                right.target_story_id,
                right.scalar_index,
                right.instance_id.as_str(),
            ))
    });

    diagnostics.push(ViewerDiagnostic {
        code: "viewer.cmo_projection.materialized".to_owned(),
        severity: ViewerDiagnosticSeverity::Info,
        message: format!(
            "{} read-only Cmo Story-slot instance(s) were materialized through the canonical slot-flow authority.",
            instances.len()
        ),
    });
    if unresolved_frame_relations > 0 {
        diagnostics.push(ViewerDiagnostic {
            code: "viewer.cmo_projection.target_frame_unresolved".to_owned(),
            severity: ViewerDiagnosticSeverity::FidelityWarning,
            message: format!(
                "{unresolved_frame_relations} admitted Cmo relation(s) have no unique target frame and remain withheld."
            ),
        });
    }
    if lower_bound_targets > 0 || withheld_nonzero_slots > 0 {
        diagnostics.push(ViewerDiagnostic {
            code: "viewer.cmo_projection.lower_bound_flow".to_owned(),
            severity: ViewerDiagnosticSeverity::FidelityWarning,
            message: format!(
                "Cmo placement is currently a scalar-0 lower bound for {lower_bound_targets} target(s); {withheld_nonzero_slots} later visible slot(s) were withheld until shaped scalar-line interleave reaches the Viewer seam."
            ),
        });
    }
    if overset_targets > 0 {
        diagnostics.push(ViewerDiagnostic {
            code: "viewer.cmo_projection.overset".to_owned(),
            severity: ViewerDiagnosticSeverity::FidelityWarning,
            message: format!(
                "{overset_targets} Cmo target(s) hit canonical first-nonfit overset; later slots were not skipped into view."
            ),
        });
    }
    if missing_carrier_text > 0 {
        diagnostics.push(ViewerDiagnostic {
            code: "viewer.cmo_projection.carrier_text_unavailable".to_owned(),
            severity: ViewerDiagnosticSeverity::FidelityWarning,
            message: format!(
                "{missing_carrier_text} visible Cmo carrier instance(s) have no bounded Viewer text fragment; geometry/paint remain available without invented text."
            ),
        });
    }

    Ok(ViewerCmoMaterializationV1 {
        instances,
        story_frames,
        text_fragments,
        diagnostics,
    })
}

fn viewer_fallback_text_flow_environment_v0_1() -> BoundedTextFlowEnvironment {
    BoundedTextFlowEnvironment {
        layout: BoundedLayoutEnvironment {
            engine_revision: VIEWER_FALLBACK_TEXT_METRICS_REVISION_V0_1.to_owned(),
            font_set_fingerprint: VIEWER_FALLBACK_TEXT_METRICS_REVISION_V0_1.to_owned(),
            resource_fingerprint: "resources:not-consumed:text-flow-v0.1".to_owned(),
        },
        text_metrics: Some(BoundedTextMetrics {
            font_fingerprint: VIEWER_FALLBACK_TEXT_METRICS_REVISION_V0_1.to_owned(),
            scalar_advance: LengthEmu::new(VIEWER_FALLBACK_SCALAR_ADVANCE_EMU_V0_1),
            line_height: LengthEmu::new(VIEWER_FALLBACK_LINE_HEIGHT_EMU_V0_1),
        }),
    }
}

fn viewer_fallback_flow_metrics_diagnostic() -> ViewerDiagnostic {
    ViewerDiagnostic {
        code: "viewer.text.fallback_flow_metrics".to_owned(),
        severity: ViewerDiagnosticSeverity::FidelityWarning,
        message: "Visible text fragments use Viewer fallback font metrics for bounded frame flow while admitted source font sizes may affect sizing. Their frame ownership is grounded, but line breaks and fragment boundaries are not claimed to match Publisher typography.".to_owned(),
    }
}

fn is_refreshable_text_flow_diagnostic(code: &str) -> bool {
    matches!(
        code,
        "viewer.text.flow_not_explicit"
            | "viewer.text.flow_partial"
            | "viewer.text.fallback_overset"
            | "viewer.text.frame_capacity_partial"
            | "viewer.text.fallback_metrics_unavailable"
            | "viewer.text.fallback_flow_metrics"
    )
}

fn resolve_viewer_text_fragments(
    projection: &pub_layout::BoundedLayoutProjection,
) -> Result<(Vec<ViewerTextFragment>, Vec<ResolveDiagnostic>)> {
    let flow = resolve_bounded_text_flow(projection, viewer_fallback_text_flow_environment_v0_1())
        .map_err(|blocked| {
            let codes = blocked
                .projection_errors
                .iter()
                .map(|diagnostic| diagnostic.code.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            anyhow!("Viewer text-flow resolution blocked by layout projection errors: {codes}")
        })?;

    let fragments = flow
        .text_fragments
        .into_iter()
        .map(|fragment| ViewerTextFragment {
            story_id: fragment.story_origin,
            frame_id: fragment.frame_origin,
            scalar_start: fragment.scalar_start,
            scalar_end: fragment.scalar_end,
            text: fragment.text,
            line_count: fragment.line_count,
        })
        .collect::<Vec<_>>();

    Ok((fragments, flow.diagnostics))
}

/// Explicit deterministic environment profile for the geometry-only Viewer
/// scene. Fonts and resources are fenced as not consumed by this resolver.
pub fn viewer_geometry_environment_v0_1() -> BoundedLayoutEnvironment {
    BoundedLayoutEnvironment {
        engine_revision: "viewer-geometry-v0.1".to_owned(),
        font_set_fingerprint: "fonts:not-consumed:geometry-only".to_owned(),
        resource_fingerprint: "resources:not-consumed:geometry-only".to_owned(),
    }
}

fn build_mature_0x2c_pipeline(bytes: &[u8]) -> Result<Mature0x2cPipeline> {
    let source_hash = sha256_digest(bytes)?;
    let source = build_mature_0x2c_source_graph(Cursor::new(bytes), source_hash)
        .context("build mature-0x2C PUB source graph for Viewer")?;
    let resolved =
        resolve_pub_source_graph(&source.graph).context("resolve PUB source graph for Viewer")?;
    let page_selection = select_viewer_pages(bytes, source_hash, &source, &resolved);

    Ok(Mature0x2cPipeline {
        source_hash,
        source,
        resolved,
        page_selection,
    })
}

fn select_viewer_pages(
    bytes: &[u8],
    source_hash: Sha256Digest,
    source: &PubSourceGraphBuild,
    resolved: &PubResolvedGraphBuild,
) -> ViewerPageSelection {
    let generic = || ViewerPageSelection {
        page_ids: source.effective_pages.page_ids.clone(),
        disposition: ViewerPageSelectionDisposition::GenericNoLoss,
    };

    let source_sha256 = source_hash.to_string();
    let Some(carrier_page_seq_nums) = carlton_admitted_carrier_page_seq_nums_v1(&source_sha256)
    else {
        return generic();
    };

    let page_roles = match analyze_mature_0x2c_page_roles(Cursor::new(bytes)) {
        Ok(receipt) => receipt,
        Err(error) => {
            return ViewerPageSelection {
                page_ids: source.effective_pages.page_ids.clone(),
                disposition: ViewerPageSelectionDisposition::FamilyProfileUnavailable {
                    reason: format!("page_role_evidence_unavailable:{error}"),
                },
            };
        }
    };

    let input = CarltonPresentationProfileInputV1 {
        schema_version: CARLTON_PRESENTATION_INPUT_SCHEMA_V1.to_owned(),
        source_sha256,
        pages: page_roles
            .pages
            .into_iter()
            .map(|page| CarltonPageEvidenceV1 {
                document_ordinal: page.document_ordinal,
                contents_seq_num: page.contents_seq_num,
                oid_dword0: page.oid_dword0,
                oid_dword1: page.oid_dword1,
                applied_master_seq_num: page.applied_master_seq_num,
                shape_child_count: page.shape_child_count,
            })
            .collect(),
        // The exact SHA admission binds this to the PlcCmob carrier evidence
        // proven by CARLTON-PAGE-PROJECTION-01. Unknown hashes never reach here.
        carrier_page_seq_nums: carrier_page_seq_nums.to_vec(),
    };

    let selection = match select_carlton_customer_page_seq_nums_v1(input) {
        Ok(selection) => selection,
        Err(error) => {
            return ViewerPageSelection {
                page_ids: source.effective_pages.page_ids.clone(),
                disposition: ViewerPageSelectionDisposition::FamilyProfileUnavailable {
                    reason: format!("family_profile_rejected:{error}"),
                },
            };
        }
    };

    let mut page_ids = Vec::with_capacity(selection.customer_page_seq_nums.len());
    for seq_num in &selection.customer_page_seq_nums {
        let page_id = match derive_pub_page_id(&source_hash, *seq_num) {
            Ok(page_id) => page_id,
            Err(error) => {
                return ViewerPageSelection {
                    page_ids: source.effective_pages.page_ids.clone(),
                    disposition: ViewerPageSelectionDisposition::FamilyProfileUnavailable {
                        reason: format!("customer_page_identity_unavailable:{seq_num}:{error}"),
                    },
                };
            }
        };
        if !resolved.graph.pages.contains_key(&page_id) {
            return ViewerPageSelection {
                page_ids: source.effective_pages.page_ids.clone(),
                disposition: ViewerPageSelectionDisposition::FamilyProfileUnavailable {
                    reason: format!("customer_page_missing_from_resolved_graph:{seq_num}"),
                },
            };
        }
        page_ids.push(page_id);
    }

    ViewerPageSelection {
        page_ids,
        disposition: ViewerPageSelectionDisposition::FamilyProfileApplied {
            profile_id: selection.profile_id,
            raw_page_count: selection.raw_page_count,
            customer_page_count: selection.customer_page_seq_nums.len(),
        },
    }
}

fn viewer_document_from_pipeline(
    byte_len: usize,
    pipeline: &Mature0x2cPipeline,
) -> Result<ViewerDocument> {
    let graph = &pipeline.resolved.graph;

    let effective_page_ids = &pipeline.page_selection.page_ids;
    let mut pages = Vec::with_capacity(effective_page_ids.len());
    for (zero_based, page_id) in effective_page_ids.iter().enumerate() {
        let page = graph
            .pages
            .get(page_id)
            .with_context(|| format!("document references missing canonical page {page_id:?}"))?;
        let index = u32::try_from(zero_based + 1).context("Viewer page index exceeds u32")?;
        pages.push(ViewerPage {
            index,
            id: *page_id,
            width_emu: page.size.width.get(),
            height_emu: page.size.height.get(),
        });
    }

    let stories = graph
        .stories
        .values()
        .map(|story| ViewerStory {
            id: story.id,
            text: story.text.clone(),
        })
        .collect::<Vec<_>>();

    let mut diagnostics = pipeline
        .source
        .diagnostics
        .iter()
        .map(map_bridge_diagnostic)
        .chain(
            pipeline
                .resolved
                .diagnostics
                .iter()
                .map(map_resolve_diagnostic),
        )
        .collect::<Vec<_>>();

    match &pipeline.page_selection.disposition {
        ViewerPageSelectionDisposition::GenericNoLoss => {}
        ViewerPageSelectionDisposition::FamilyProfileApplied {
            profile_id,
            raw_page_count,
            customer_page_count,
        } => {
            diagnostics
                .retain(|diagnostic| diagnostic.code != "viewer.page_projection.roles_unresolved");
            diagnostics.push(ViewerDiagnostic {
                code: "viewer.page_projection.family_profile_applied".to_owned(),
                severity: ViewerDiagnosticSeverity::Info,
                message: format!(
                    "Admitted family presentation profile {profile_id} selects {customer_page_count} customer pages from {raw_page_count} preserved raw PAGE records."
                ),
            });
        }
        ViewerPageSelectionDisposition::FamilyProfileUnavailable { reason } => {
            diagnostics.push(ViewerDiagnostic {
                code: "viewer.page_projection.family_profile_unavailable".to_owned(),
                severity: ViewerDiagnosticSeverity::FidelityWarning,
                message: format!(
                    "An exact family presentation profile was recognized but could not be applied safely ({reason}); the Viewer preserves every recovered raw PAGE."
                ),
            });
        }
    }
    normalize_diagnostics(&mut diagnostics);

    Ok(ViewerDocument {
        schema_version: VIEWER_DOCUMENT_SCHEMA_V0_1.to_owned(),
        source: ViewerSource {
            format: graph.source.format.clone(),
            format_version: graph.source.format_version.clone(),
            source_hash: pipeline.source_hash,
            byte_len: u64::try_from(byte_len).context("PUB byte length exceeds u64")?,
        },
        pages,
        stories,
        diagnostics,
    })
}

/// Creates only the grounded semantic subset already accepted by pub-layout.
///
/// Viewer remains the semantic owner of this resolved-graph -> bounded-authoring
/// bridge. Desktop shaped-flow is the second concrete consumer, so the mapping
/// is public instead of being copied into another engine adapter.
pub fn bounded_authoring_slice_from_resolved(
    graph: &PubResolvedGraph,
) -> Result<BoundedAuthoringSlice> {
    bounded_authoring_slice_from_resolved_pages(graph, &graph.document.pages)
}

fn bounded_authoring_slice_from_resolved_pages(
    graph: &PubResolvedGraph,
    page_ids: &[PageId],
) -> Result<BoundedAuthoringSlice> {
    let pages = page_ids
        .iter()
        .map(|page_id| {
            graph
                .pages
                .get(page_id)
                .cloned()
                .with_context(|| format!("layout projection missing document page {page_id:?}"))
        })
        .collect::<Result<Vec<_>>>()?;

    let page_origins = page_ids
        .iter()
        .map(|page_id| page_id.into_canonical())
        .collect::<BTreeSet<_>>();

    let node_geometry = graph
        .nodes
        .values()
        .filter(|node| page_origins.contains(&node.header.parent_id))
        .map(|node| BoundedNodeGeometryInput {
            node_id: node.header.id,
            parent_origin: node.header.parent_id,
            bounds: node.header.bounds,
            transform: node.header.transform.clone(),
        })
        .collect();

    let stories = graph.stories.values().cloned().collect();

    let story_frames = graph
        .nodes
        .values()
        .filter(|node| page_origins.contains(&node.header.parent_id))
        .filter_map(|node| {
            let frame = node.payload.story_frame.as_ref()?;
            let story_id = frame.story_id?;
            Some(StoryFrame {
                story_id,
                frame_id: node.header.id,
                ordinal: frame.ordinal,
                previous: frame.previous_frame,
                next: frame.next_frame,
            })
        })
        .collect();

    Ok(BoundedAuthoringSlice {
        pages,
        node_geometry,
        stories,
        story_frames,
        tables: Vec::new(),
        guides: Vec::new(),
        unknown_layout_state: Vec::new(),
    })
}

fn sha256_digest(bytes: &[u8]) -> Result<Sha256Digest> {
    let digest = Sha256::digest(bytes);
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    hex.parse()
        .map_err(|error| anyhow!("invalid internally computed SHA-256: {error:?}"))
}

fn map_bridge_diagnostic(diagnostic: &PubBridgeDiagnostic) -> ViewerDiagnostic {
    use PubBridgeDiagnostic::*;

    let (code, severity, message) = match diagnostic {
        PageListSpecialEntry { .. } => (
            "viewer.page_list.special_entry",
            ViewerDiagnosticSeverity::Info,
            "The document page list contains a special non-page entry.",
        ),
        PageListUnknownEntry { .. } => (
            "viewer.page_list.unknown_entry",
            ViewerDiagnosticSeverity::FidelityWarning,
            "The document page list contains an entry the Viewer cannot classify.",
        ),
        OpaqueContentsTail { .. } => (
            "viewer.object.opaque_data",
            ViewerDiagnosticSeverity::FidelityWarning,
            "Some object data is not understood by the current Viewer model.",
        ),
        MissingEscherGeometry { .. } => (
            "viewer.geometry.missing",
            ViewerDiagnosticSeverity::FidelityWarning,
            "A placed object has no confirmed display geometry.",
        ),
        AmbiguousEscherGeometry { .. } => (
            "viewer.geometry.ambiguous",
            ViewerDiagnosticSeverity::FidelityWarning,
            "A placed object has more than one possible display geometry.",
        ),
        AmbiguousImageSlot { .. } => (
            "viewer.image.identity_ambiguous",
            ViewerDiagnosticSeverity::FidelityWarning,
            "An image reference cannot be resolved to one confirmed embedded image.",
        ),
        IncompleteEscherAnchor { .. } => (
            "viewer.geometry.incomplete",
            ViewerDiagnosticSeverity::FidelityWarning,
            "A placed object has incomplete position or size information.",
        ),
        InvalidEscherAnchor { .. } => (
            "viewer.geometry.invalid",
            ViewerDiagnosticSeverity::FidelityWarning,
            "A placed object has invalid position or size information.",
        ),
        MissingQuillStory { .. } => (
            "viewer.text.story_missing",
            ViewerDiagnosticSeverity::FidelityWarning,
            "A text frame refers to text that the Viewer could not recover.",
        ),
        McldRecordCountMismatch { .. } => (
            "viewer.table.mcld_layout_metrics_unavailable",
            ViewerDiagnosticSeverity::FidelityWarning,
            "A Quill layout-metrics table uses a structure outside the bounded MCLD profile; core document content remains available.",
        ),
        EquivalentMarginsPageExtents { .. } => (
            "viewer.page_extent.equivalent_source_records",
            ViewerDiagnosticSeverity::Info,
            "Multiple source page-extent records agree exactly; the Viewer uses their shared page size.",
        ),
        ScenarioPageOrderObserved { .. } => (
            "viewer.page_projection.scenario_order_observed",
            ViewerDiagnosticSeverity::Info,
            "A persisted scenario/design page-identity order was recovered. It is retained as evidence only and is not used to suppress physical pages.",
        ),
        ScenarioPageOrderUnavailable { .. } => (
            "viewer.page_projection.scenario_order_unavailable",
            ViewerDiagnosticSeverity::Info,
            "Scenario/design page-order metadata could not be resolved safely; it is not used for physical page filtering.",
        ),
        PageRoleClassificationUnresolved { .. } => (
            "viewer.page_projection.roles_unresolved",
            ViewerDiagnosticSeverity::FidelityWarning,
            "Generic customer/master/service page-role filtering is not proven for this file family, so the Viewer preserves all recovered physical PAGE records.",
        ),
        LinkedFrameNotMaterialized { .. } => (
            "viewer.text.link_target_missing",
            ViewerDiagnosticSeverity::FidelityWarning,
            "A linked text frame refers to another frame that is not materialized.",
        ),
        GroupedStoryProjected { .. } => (
            "viewer.geometry.grouped_story_projected",
            ViewerDiagnosticSeverity::Info,
            "A grouped text shape was projected through its exact bounded group geometry chain.",
        ),
        GroupedStoryProjectionUnavailable { .. } => (
            "viewer.geometry.grouped_story_projection_unavailable",
            ViewerDiagnosticSeverity::FidelityWarning,
            "A grouped text shape falls outside the bounded group geometry profile.",
        ),
        GroupedTableProjected { .. } => (
            "viewer.geometry.grouped_table_projected",
            ViewerDiagnosticSeverity::Info,
            "A grouped table was projected through its exact bounded group geometry chain.",
        ),
        GroupedTableProjectionUnavailable { .. } => (
            "viewer.geometry.grouped_table_projection_unavailable",
            ViewerDiagnosticSeverity::FidelityWarning,
            "A grouped table falls outside the bounded group geometry profile.",
        ),
        TableMissingRequiredField { .. } => (
            "viewer.table.required_data_missing",
            ViewerDiagnosticSeverity::FidelityWarning,
            "A table is missing data required for the bounded table model.",
        ),
        TableMissingTcd { .. } => (
            "viewer.table.text_map_missing",
            ViewerDiagnosticSeverity::FidelityWarning,
            "A table's text mapping could not be recovered.",
        ),
        TableAmbiguousTcd { .. } => (
            "viewer.table.text_map_ambiguous",
            ViewerDiagnosticSeverity::FidelityWarning,
            "A table has more than one possible text mapping.",
        ),
        TableMissingCellsObject { .. } => (
            "viewer.table.cells_missing",
            ViewerDiagnosticSeverity::FidelityWarning,
            "A table refers to a cell collection that is not available.",
        ),
        TableCellsWrongRawType { .. } => (
            "viewer.table.cells_unexpected_kind",
            ViewerDiagnosticSeverity::FidelityWarning,
            "A table's cell collection has an unexpected object kind.",
        ),
        TableCellsWrongParent { .. } => (
            "viewer.table.cells_parent_mismatch",
            ViewerDiagnosticSeverity::FidelityWarning,
            "A table's cell collection has an unexpected ownership relation.",
        ),
        TableCellCountMismatch { .. } => (
            "viewer.table.cell_count_mismatch",
            ViewerDiagnosticSeverity::FidelityWarning,
            "A table's recovered cell counts disagree.",
        ),
        TableCellTextRangeInvalid { .. } => (
            "viewer.table.cell_text_range_invalid",
            ViewerDiagnosticSeverity::FidelityWarning,
            "A table cell points outside the recovered text range.",
        ),
        TableStoryLengthMismatch { .. } => (
            "viewer.table.story_length_mismatch",
            ViewerDiagnosticSeverity::FidelityWarning,
            "A table's cell text boundaries disagree with the recovered story length.",
        ),
        TableCellCoordinatesAmbiguous { .. } => (
            "viewer.table.cell_coordinates_ambiguous",
            ViewerDiagnosticSeverity::FidelityWarning,
            "A table cell does not have one confirmed row and column position.",
        ),
        TableLayoutMetricsUnavailable { .. } => (
            "viewer.table.layout_metrics_unavailable",
            ViewerDiagnosticSeverity::FidelityWarning,
            "Exact table layout metrics are not available.",
        ),
        TypographyProjectionUnavailable { .. } => (
            "viewer.text.typography_projection_unavailable",
            ViewerDiagnosticSeverity::FidelityWarning,
            "Some source typography could not be projected safely; pinned fallback text rendering remains in use.",
        ),
        TypographyUnknownFixedBlockTypes { .. } => (
            "viewer.text.typography_unknown_block_type",
            ViewerDiagnosticSeverity::FidelityWarning,
            "The typography stream contains unproven fixed block widths; affected typography promotion fails closed.",
        ),
    };

    ViewerDiagnostic {
        code: code.to_owned(),
        severity,
        message: message.to_owned(),
    }
}

fn map_resolve_diagnostic(diagnostic: &PubResolveDiagnostic) -> ViewerDiagnostic {
    match diagnostic {
        PubResolveDiagnostic::MissingStoryIdentity { .. } => ViewerDiagnostic {
            code: "viewer.text.story_identity_missing".to_owned(),
            severity: ViewerDiagnosticSeverity::FidelityWarning,
            message: "A text frame could not be joined to one recovered story.".to_owned(),
        },
    }
}

fn map_projection_diagnostic(diagnostic: &ProjectionDiagnostic) -> ViewerDiagnostic {
    let (code, message) = match diagnostic.code.as_str() {
        "invalid_page_semantics" => (
            "viewer.layout.invalid_page",
            "A page cannot be projected into the current visual layout model.",
        ),
        "missing_story_content" => (
            "viewer.layout.story_content_missing",
            "A text frame refers to story content absent from the visual projection.",
        ),
        "missing_frame_geometry" => (
            "viewer.layout.frame_geometry_missing",
            "A text frame has no confirmed geometry in the visual projection.",
        ),
        "unknown_layout_affecting_state" => (
            "viewer.layout.unknown_affecting_state",
            "Some layout-affecting state is not understood by the current Viewer.",
        ),
        _ => (
            "viewer.layout.projection_partial",
            "The current Viewer cannot fully project some authoring state.",
        ),
    };

    ViewerDiagnostic {
        code: code.to_owned(),
        severity: ViewerDiagnosticSeverity::FidelityWarning,
        message: message.to_owned(),
    }
}

fn map_scene_diagnostic(diagnostic: &ResolveDiagnostic) -> ViewerDiagnostic {
    let (code, message) = match diagnostic.code.as_str() {
        "story_text_layout_not_implemented" => (
            "viewer.layout.text_not_rendered",
            "Text is recovered for search and copy, but is not yet visually laid out in this Viewer slice.",
        ),
        "shared_story_without_explicit_flow" => (
            "viewer.text.flow_not_explicit",
            "Several frames share one recovered Story, but no explicit reciprocal flow chain is proven, so the Viewer does not invent one.",
        ),
        "ambiguous_story_flow"
        | "cyclic_story_flow"
        | "broken_story_flow"
        | "non_reciprocal_story_flow"
        | "disconnected_story_flow" => (
            "viewer.text.flow_partial",
            "A recovered multi-frame Story does not form one bounded explicit flow chain, so its preview text placement remains partial.",
        ),
        "story_overset" => (
            "viewer.text.fallback_overset",
            "The explicit frame chain cannot place all Story text under the Viewer fallback metrics. This is not Publisher-native overset evidence.",
        ),
        "text_frame_geometry_missing" | "text_frame_has_no_capacity" => (
            "viewer.text.frame_capacity_partial",
            "A recovered text frame cannot accept bounded fallback text placement with the current resolved geometry.",
        ),
        "text_metrics_missing" | "text_metrics_font_mismatch" | "invalid_text_metrics" => (
            "viewer.text.fallback_metrics_unavailable",
            "The Viewer fallback text environment is unavailable or inconsistent, so text flow is not materialized.",
        ),
        _ => (
            "viewer.layout.scene_partial",
            "Some visual content is only partially resolved by the current Viewer.",
        ),
    };

    ViewerDiagnostic {
        code: code.to_owned(),
        severity: ViewerDiagnosticSeverity::FidelityWarning,
        message: message.to_owned(),
    }
}

fn normalize_diagnostics(diagnostics: &mut Vec<ViewerDiagnostic>) {
    diagnostics.sort_by(|left, right| {
        (&left.code, &left.message, severity_order(left.severity)).cmp(&(
            &right.code,
            &right.message,
            severity_order(right.severity),
        ))
    });
    diagnostics.dedup();
}

const fn severity_order(severity: ViewerDiagnosticSeverity) -> u8 {
    match severity {
        ViewerDiagnosticSeverity::Info => 0,
        ViewerDiagnosticSeverity::FidelityWarning => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pub_model::{
        Affine2D, CanonicalId, Document, DocumentId, LengthEmu, Node, NodeHeader, NodeId, NodeKind,
        Page, RectEmu, Size2D, SourceDescriptor, Story,
    };
    use pub_reader::{PubResolvedNodePayload, PubResolvedStoryFrame};
    use std::collections::BTreeMap;

    #[test]
    fn local_failure_report_exposes_structural_class_without_source_content() {
        let bytes = b"<!DOCTYPE html><html><body>private-customer-text-8271</body></html>";
        let json = local_failure_diagnostic_json(bytes).expect("local failure report");

        assert!(json.contains(VIEWER_FAILURE_REPORT_SCHEMA_V0_1));
        assert!(json.contains("\"intake_class\": \"not_pub\""));
        assert!(json.contains("\"container_family\": \"foreign\""));
        assert!(!json.contains("private-customer-text-8271"));
        assert!(!json.contains("<!DOCTYPE"));
        assert!(!json.contains("\"path\""));
        assert!(!json.contains("\"filename\""));
        assert!(!json.contains("\"source_hash\""));
        assert!(!json.contains("\"sha256\""));
    }

    #[test]
    fn local_failure_report_uses_size_bucket_instead_of_exact_length() {
        let bytes = vec![0_u8; 5_123];
        let report = build_local_failure_diagnostic_report(&bytes).expect("local failure report");

        assert_eq!(
            report.envelope.size_bucket,
            pub_reader::FailureSizeBucket::Under64KiB
        );

        let json = serde_json::to_string(&report).expect("serialize report");
        assert!(!json.contains("\"byte_len\""));
        assert!(!json.contains("5123"));
    }

    fn id(byte: u8) -> CanonicalId {
        CanonicalId::from_bytes([byte; 16])
    }

    fn resolved_graph_fixture() -> PubResolvedGraph {
        let source_hash = Sha256Digest::from_bytes([0xAB; 32]);
        let page_id = PageId::from_canonical(id(2));
        let node_id = NodeId::from_canonical(id(3));
        let story_id = StoryId::from_canonical(id(4));

        PubResolvedGraph {
            cdm_version: "0.1".to_owned(),
            resolver_version: "test".to_owned(),
            source: SourceDescriptor {
                format: "pub".to_owned(),
                format_version: Some("0x2c".to_owned()),
                adapter_version: "test".to_owned(),
                source_hash,
            },
            document: Document {
                id: DocumentId::from_canonical(id(1)),
                format_origin: "pub".to_owned(),
                source_hash,
                pages: vec![page_id],
                resources: Vec::new(),
                styles: Vec::new(),
            },
            pages: BTreeMap::from([(
                page_id,
                Page {
                    id: page_id,
                    size: Size2D::new(LengthEmu::new(1_000), LengthEmu::new(2_000)),
                    bleed: None,
                    margins: None,
                    children: Vec::new(),
                    extensions: Vec::new(),
                },
            )]),
            nodes: BTreeMap::from([(
                node_id,
                Node {
                    kind: NodeKind::Shape,
                    header: NodeHeader {
                        id: node_id,
                        parent_id: page_id.into_canonical(),
                        bounds: RectEmu::new(
                            LengthEmu::new(10),
                            LengthEmu::new(20),
                            LengthEmu::new(300),
                            LengthEmu::new(400),
                        ),
                        transform: Affine2D::identity(),
                        source_refs: Vec::new(),
                        extensions: Vec::new(),
                    },
                    payload: PubResolvedNodePayload {
                        contents_seq_num: 7,
                        officeart_shape_type: None,
                        officeart_spid: None,
                        image_slot: None,
                        explicit_image_crop: None,
                        explicit_paint: pub_reader::PubExplicitShapePaintSource::default(),
                        story_frame: Some(PubResolvedStoryFrame {
                            story_id: Some(story_id),
                            ordinal: 0,
                            previous_frame: None,
                            next_frame: None,
                        }),
                        table_story: None,
                        table: None,
                    },
                },
            )]),
            stories: BTreeMap::from([(
                story_id,
                Story {
                    id: story_id,
                    text: "Hello Viewer".to_owned(),
                    paragraphs: Vec::new(),
                    runs: Vec::new(),
                    fields: Vec::new(),
                    hyperlinks: Vec::new(),
                    source_refs: Vec::new(),
                },
            )]),
            paragraphs: BTreeMap::new(),
            text_runs: BTreeMap::new(),
            resources: BTreeMap::new(),
            styles: BTreeMap::new(),
            extensions: BTreeMap::new(),
        }
    }

    fn linked_resolved_graph_fixture(text: &str) -> PubResolvedGraph {
        let mut graph = resolved_graph_fixture();
        let page_id = graph.document.pages[0];
        let first_frame = *graph.nodes.keys().next().expect("fixture frame");
        let second_frame = NodeId::from_canonical(id(5));
        let story_id = *graph.stories.keys().next().expect("fixture story");
        let scalar_advance = VIEWER_FALLBACK_SCALAR_ADVANCE_EMU_V0_1;
        let line_height = VIEWER_FALLBACK_LINE_HEIGHT_EMU_V0_1;

        {
            let page = graph.pages.get_mut(&page_id).expect("fixture page");
            page.size = Size2D::new(
                LengthEmu::new(scalar_advance * 12),
                LengthEmu::new(line_height * 4),
            );
            page.children = vec![first_frame, second_frame];
        }

        let mut second_node = graph.nodes.get(&first_frame).expect("first frame").clone();
        second_node.header.id = second_frame;
        second_node.header.bounds = RectEmu::new(
            LengthEmu::ZERO,
            LengthEmu::new(line_height * 2),
            LengthEmu::new(scalar_advance * 4),
            LengthEmu::new(line_height),
        );
        second_node.payload.story_frame = Some(PubResolvedStoryFrame {
            story_id: Some(story_id),
            ordinal: 1,
            previous_frame: Some(first_frame),
            next_frame: None,
        });

        {
            let first = graph.nodes.get_mut(&first_frame).expect("first frame");
            first.header.bounds = RectEmu::new(
                LengthEmu::ZERO,
                LengthEmu::ZERO,
                LengthEmu::new(scalar_advance * 4),
                LengthEmu::new(line_height),
            );
            first.payload.story_frame = Some(PubResolvedStoryFrame {
                story_id: Some(story_id),
                ordinal: 0,
                previous_frame: None,
                next_frame: Some(second_frame),
            });
        }
        graph.nodes.insert(second_frame, second_node);
        graph
            .stories
            .get_mut(&story_id)
            .expect("fixture story")
            .text = text.to_owned();
        graph
    }

    #[test]
    fn created_text_box_scene_sync_tracks_create_undo_redo_without_touching_baseline_nodes() {
        let mut graph = resolved_graph_fixture();
        let projection =
            project_bounded(bounded_authoring_slice_from_resolved(&graph).expect("projection"));
        let baseline_scene =
            resolve_bounded_geometry(&projection, viewer_geometry_environment_v0_1())
                .expect("baseline scene");
        let baseline_nodes = baseline_scene.nodes.clone();
        let source_hash = graph.source.source_hash;
        let mut visual = ViewerGeometryDocument {
            schema_version: VIEWER_GEOMETRY_SCHEMA_V0_1.to_owned(),
            document: ViewerDocument {
                schema_version: VIEWER_DOCUMENT_SCHEMA_V0_1.to_owned(),
                source: ViewerSource {
                    format: "pub".to_owned(),
                    format_version: Some("0x2c".to_owned()),
                    source_hash,
                    byte_len: 1,
                },
                pages: Vec::new(),
                stories: Vec::new(),
                diagnostics: Vec::new(),
            },
            scene: baseline_scene,
            paints: Vec::new(),
            story_frames: Vec::new(),
            text_fragments: Vec::new(),
            typography_runs: Vec::new(),
            images: Vec::new(),
            projected_instances: Vec::new(),
        };

        let page_id = graph.document.pages[0];
        let node_id = NodeId::from_canonical(id(9));
        let story_id = StoryId::from_canonical(id(10));
        let bounds = RectEmu::new(
            LengthEmu::new(100),
            LengthEmu::new(200),
            LengthEmu::new(300),
            LengthEmu::new(400),
        );
        let story = Story {
            id: story_id,
            text: String::new(),
            paragraphs: Vec::new(),
            runs: Vec::new(),
            fields: Vec::new(),
            hyperlinks: Vec::new(),
            source_refs: Vec::new(),
        };
        let node = Node {
            kind: NodeKind::TextFrame,
            header: NodeHeader {
                id: node_id,
                parent_id: page_id.into_canonical(),
                bounds,
                transform: Affine2D::identity(),
                source_refs: Vec::new(),
                extensions: Vec::new(),
            },
            payload: PubResolvedNodePayload {
                contents_seq_num: 0,
                officeart_shape_type: None,
                officeart_spid: None,
                image_slot: None,
                explicit_image_crop: None,
                explicit_paint: pub_reader::PubExplicitShapePaintSource::default(),
                story_frame: Some(PubResolvedStoryFrame {
                    story_id: Some(story_id),
                    ordinal: 0,
                    previous_frame: None,
                    next_frame: None,
                }),
                table_story: None,
                table: None,
            },
        };

        graph.stories.insert(story_id, story.clone());
        graph.nodes.insert(node_id, node.clone());
        graph
            .pages
            .get_mut(&page_id)
            .expect("page")
            .children
            .push(node_id);

        let synced = visual
            .sync_editor_created_text_box_scene_nodes(&graph, &[node_id], &BTreeSet::new())
            .expect("create sync");
        let created = visual
            .scene
            .nodes
            .iter()
            .find(|scene_node| scene_node.origin == node_id)
            .expect("created TextBox scene node");
        assert_eq!(created.parent_origin, page_id.into_canonical());
        assert_eq!(created.bounds, bounds);
        assert_eq!(
            visual
                .scene
                .nodes
                .iter()
                .filter(|scene_node| !synced.contains(&scene_node.origin))
                .cloned()
                .collect::<Vec<_>>(),
            baseline_nodes
        );

        graph.nodes.remove(&node_id);
        graph.stories.remove(&story_id);
        graph
            .pages
            .get_mut(&page_id)
            .expect("page")
            .children
            .retain(|child| *child != node_id);
        let after_undo = visual
            .sync_editor_created_text_box_scene_nodes(&graph, &[], &synced)
            .expect("undo sync");
        assert!(after_undo.is_empty());
        assert_eq!(visual.scene.nodes, baseline_nodes);

        graph.stories.insert(story_id, story);
        graph.nodes.insert(node_id, node);
        graph
            .pages
            .get_mut(&page_id)
            .expect("page")
            .children
            .push(node_id);
        let after_redo = visual
            .sync_editor_created_text_box_scene_nodes(&graph, &[node_id], &after_undo)
            .expect("redo sync");
        assert_eq!(after_redo, BTreeSet::from([node_id]));
        assert_eq!(
            visual
                .scene
                .nodes
                .iter()
                .find(|scene_node| scene_node.origin == node_id)
                .expect("redo node")
                .bounds,
            bounds
        );
        assert_eq!(visual.document.source.source_hash, source_hash);
    }

    #[test]
    fn created_text_box_scene_sync_fails_closed_on_unproven_source_backed_node() {
        let mut graph = resolved_graph_fixture();
        let projection =
            project_bounded(bounded_authoring_slice_from_resolved(&graph).expect("projection"));
        let scene = resolve_bounded_geometry(&projection, viewer_geometry_environment_v0_1())
            .expect("scene");
        let source_hash = graph.source.source_hash;
        let existing = *graph.nodes.keys().next().expect("fixture node");
        let mut visual = ViewerGeometryDocument {
            schema_version: VIEWER_GEOMETRY_SCHEMA_V0_1.to_owned(),
            document: ViewerDocument {
                schema_version: VIEWER_DOCUMENT_SCHEMA_V0_1.to_owned(),
                source: ViewerSource {
                    format: "pub".to_owned(),
                    format_version: Some("0x2c".to_owned()),
                    source_hash,
                    byte_len: 1,
                },
                pages: Vec::new(),
                stories: Vec::new(),
                diagnostics: Vec::new(),
            },
            scene,
            paints: Vec::new(),
            story_frames: Vec::new(),
            text_fragments: Vec::new(),
            typography_runs: Vec::new(),
            images: Vec::new(),
            projected_instances: Vec::new(),
        };
        let before = visual.scene.nodes.clone();

        graph.nodes.get_mut(&existing).expect("fixture node").kind = NodeKind::TextFrame;
        assert!(
            visual
                .sync_editor_created_text_box_scene_nodes(&graph, &[existing], &BTreeSet::new(),)
                .is_err()
        );
        assert_eq!(visual.scene.nodes, before);
    }

    #[test]
    fn source_hash_matches_sha256_known_answer() {
        let expected: Sha256Digest =
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
                .parse()
                .expect("known SHA-256 should parse");

        assert_eq!(sha256_digest(b"abc").unwrap(), expected);
    }

    #[test]
    fn bridge_diagnostic_is_mapped_to_stable_product_vocabulary() {
        let mapped =
            map_bridge_diagnostic(&PubBridgeDiagnostic::MissingEscherGeometry { seq_num: 42 });

        assert_eq!(mapped.code, "viewer.geometry.missing");
        assert_eq!(mapped.severity, ViewerDiagnosticSeverity::FidelityWarning);
        assert!(!mapped.message.contains("Escher"));
        assert!(!mapped.message.contains("seq_num"));
    }

    #[test]
    fn equivalent_margins_are_reported_without_fidelity_loss() {
        let mapped = map_bridge_diagnostic(&PubBridgeDiagnostic::EquivalentMarginsPageExtents {
            count: 8,
            width_emu: 7_772_400,
            height_emu: 10_058_400,
        });

        assert_eq!(mapped.code, "viewer.page_extent.equivalent_source_records");
        assert_eq!(mapped.severity, ViewerDiagnosticSeverity::Info);
        assert!(mapped.message.contains("agree exactly"));
    }

    #[test]
    fn mismatched_mcld_is_reported_as_bounded_fidelity_loss() {
        let mapped = map_bridge_diagnostic(&PubBridgeDiagnostic::McldRecordCountMismatch {
            record_count: 44,
            record_id_count: 4,
        });

        assert_eq!(mapped.code, "viewer.table.mcld_layout_metrics_unavailable");
        assert_eq!(mapped.severity, ViewerDiagnosticSeverity::FidelityWarning);
        assert!(
            mapped
                .message
                .contains("core document content remains available")
        );
    }

    #[test]
    fn viewer_diagnostic_json_does_not_expose_source_carrier_fields() {
        let mapped = map_resolve_diagnostic(&PubResolveDiagnostic::MissingStoryIdentity {
            node_id: pub_model::NodeId::from_canonical(pub_model::CanonicalId::from_bytes(
                [0x11; 16],
            )),
            text_id: 9,
        });
        let json = serde_json::to_string(&mapped).unwrap();

        assert!(json.contains("viewer.text.story_identity_missing"));
        assert!(!json.contains("text_id"));
        assert!(!json.contains("node_id"));
    }

    #[test]
    fn resolved_graph_bridges_to_existing_layout_scene() {
        let graph = resolved_graph_fixture();
        let slice = bounded_authoring_slice_from_resolved(&graph).unwrap();
        let projection = project_bounded(slice);
        let scene =
            resolve_bounded_geometry(&projection, viewer_geometry_environment_v0_1()).unwrap();

        assert_eq!(scene.surfaces.len(), 1);
        assert_eq!(scene.nodes.len(), 1);
        assert_eq!(scene.surfaces[0].origin, graph.document.pages[0]);
        assert_eq!(
            scene.nodes[0].parent_origin,
            graph.document.pages[0].into_canonical()
        );
        assert_eq!(scene.diagnostics.len(), 1);
        assert_eq!(
            scene.diagnostics[0].code,
            "story_text_layout_not_implemented"
        );
    }

    #[test]
    fn fidelity_status_is_supported_without_fidelity_warnings() {
        let mut document = ViewerDocument {
            schema_version: VIEWER_DOCUMENT_SCHEMA_V0_1.to_owned(),
            source: ViewerSource {
                format: "pub".to_owned(),
                format_version: Some("0x2c".to_owned()),
                source_hash: Sha256Digest::from_bytes([0x22; 32]),
                byte_len: 123,
            },
            pages: Vec::new(),
            stories: Vec::new(),
            diagnostics: vec![ViewerDiagnostic {
                code: "viewer.page_list.special_entry".to_owned(),
                severity: ViewerDiagnosticSeverity::Info,
                message: "Informational diagnostic.".to_owned(),
            }],
        };

        assert_eq!(document.fidelity_status(), ViewerFidelityStatus::Supported);

        document.diagnostics.push(ViewerDiagnostic {
            code: "viewer.geometry.missing".to_owned(),
            severity: ViewerDiagnosticSeverity::FidelityWarning,
            message: "Known visual limitation.".to_owned(),
        });

        assert_eq!(document.fidelity_status(), ViewerFidelityStatus::Partial);
    }

    #[test]
    fn unsupported_status_is_not_derived_from_an_open_document() {
        let document = ViewerDocument {
            schema_version: VIEWER_DOCUMENT_SCHEMA_V0_1.to_owned(),
            source: ViewerSource {
                format: "pub".to_owned(),
                format_version: Some("0x2c".to_owned()),
                source_hash: Sha256Digest::from_bytes([0x33; 32]),
                byte_len: 0,
            },
            pages: Vec::new(),
            stories: Vec::new(),
            diagnostics: Vec::new(),
        };

        assert_ne!(
            document.fidelity_status(),
            ViewerFidelityStatus::Unsupported
        );
    }

    #[test]
    fn semantic_search_returns_stable_story_ranges_in_document_order() {
        let source_hash = Sha256Digest::from_bytes([0x44; 32]);
        let first_story = StoryId::from_canonical(id(4));
        let second_story = StoryId::from_canonical(id(5));
        let document = ViewerDocument {
            schema_version: VIEWER_DOCUMENT_SCHEMA_V0_1.to_owned(),
            source: ViewerSource {
                format: "pub".to_owned(),
                format_version: Some("0x2c".to_owned()),
                source_hash,
                byte_len: 10,
            },
            pages: Vec::new(),
            stories: vec![
                ViewerStory {
                    id: first_story,
                    text: "alpha beta alpha".to_owned(),
                },
                ViewerStory {
                    id: second_story,
                    text: "alpha".to_owned(),
                },
            ],
            diagnostics: Vec::new(),
        };

        let matches = document.search_text("alpha");

        assert_eq!(matches.len(), 3);
        assert_eq!(matches[0].story_id, first_story);
        assert_eq!((matches[0].start_byte, matches[0].end_byte), (0, 5));
        assert_eq!(matches[0].text, "alpha");
        assert_eq!((matches[1].start_byte, matches[1].end_byte), (11, 16));
        assert_eq!(matches[2].story_id, second_story);
        assert_eq!((matches[2].start_byte, matches[2].end_byte), (0, 5));
    }

    #[test]
    fn semantic_search_empty_query_returns_no_matches() {
        let document = ViewerDocument {
            schema_version: VIEWER_DOCUMENT_SCHEMA_V0_1.to_owned(),
            source: ViewerSource {
                format: "pub".to_owned(),
                format_version: Some("0x2c".to_owned()),
                source_hash: Sha256Digest::from_bytes([0x55; 32]),
                byte_len: 5,
            },
            pages: Vec::new(),
            stories: vec![ViewerStory {
                id: StoryId::from_canonical(id(6)),
                text: "hello".to_owned(),
            }],
            diagnostics: Vec::new(),
        };

        assert!(document.search_text("").is_empty());
    }

    #[test]
    fn search_match_contract_does_not_claim_page_ownership() {
        let json = serde_json::to_value(ViewerTextMatch {
            story_id: StoryId::from_canonical(id(7)),
            start_byte: 1,
            end_byte: 3,
            text: "bc".to_owned(),
        })
        .unwrap();

        let object = json
            .as_object()
            .expect("ViewerTextMatch must serialize as object");
        assert!(object.contains_key("story_id"));
        assert!(!object.contains_key("page_id"));
        assert!(!object.contains_key("page"));
    }

    fn linked_text_projection(explicit_links: bool) -> pub_layout::BoundedLayoutProjection {
        let page_id = PageId::from_canonical(id(40));
        let first_frame = NodeId::from_canonical(id(41));
        let second_frame = NodeId::from_canonical(id(42));
        let story_id = StoryId::from_canonical(id(43));
        let scalar_advance = VIEWER_FALLBACK_SCALAR_ADVANCE_EMU_V0_1;
        let line_height = VIEWER_FALLBACK_LINE_HEIGHT_EMU_V0_1;

        project_bounded(BoundedAuthoringSlice {
            pages: vec![Page {
                id: page_id,
                size: Size2D::new(
                    LengthEmu::new(scalar_advance * 12),
                    LengthEmu::new(line_height * 4),
                ),
                bleed: None,
                margins: None,
                children: vec![first_frame, second_frame],
                extensions: Vec::new(),
            }],
            node_geometry: vec![
                BoundedNodeGeometryInput {
                    node_id: first_frame,
                    parent_origin: page_id.into_canonical(),
                    bounds: RectEmu::new(
                        LengthEmu::ZERO,
                        LengthEmu::ZERO,
                        LengthEmu::new(scalar_advance * 4),
                        LengthEmu::new(line_height),
                    ),
                    transform: Affine2D::identity(),
                },
                BoundedNodeGeometryInput {
                    node_id: second_frame,
                    parent_origin: page_id.into_canonical(),
                    bounds: RectEmu::new(
                        LengthEmu::ZERO,
                        LengthEmu::new(line_height * 2),
                        LengthEmu::new(scalar_advance * 4),
                        LengthEmu::new(line_height),
                    ),
                    transform: Affine2D::identity(),
                },
            ],
            stories: vec![Story {
                id: story_id,
                text: "ABCDEFG".to_owned(),
                paragraphs: Vec::new(),
                runs: Vec::new(),
                fields: Vec::new(),
                hyperlinks: Vec::new(),
                source_refs: Vec::new(),
            }],
            story_frames: vec![
                StoryFrame {
                    story_id,
                    frame_id: first_frame,
                    ordinal: 0,
                    previous: None,
                    next: explicit_links.then_some(second_frame),
                },
                StoryFrame {
                    story_id,
                    frame_id: second_frame,
                    ordinal: 1,
                    previous: explicit_links.then_some(first_frame),
                    next: None,
                },
            ],
            tables: Vec::new(),
            guides: Vec::new(),
            unknown_layout_state: Vec::new(),
        })
    }

    #[test]
    fn viewer_fallback_flow_materializes_explicit_linked_story_frames() {
        let projection = linked_text_projection(true);
        let (fragments, diagnostics) =
            resolve_viewer_text_fragments(&projection).expect("fallback flow should resolve");

        assert_eq!(fragments.len(), 2);
        assert_eq!(fragments[0].text, "ABCD");
        assert_eq!(fragments[0].scalar_start, 0);
        assert_eq!(fragments[0].scalar_end, 4);
        assert_eq!(fragments[1].text, "EFG");
        assert_eq!(fragments[1].scalar_start, 4);
        assert_eq!(fragments[1].scalar_end, 7);
        assert!(
            !diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "story_overset")
        );
    }

    #[test]
    fn viewer_fallback_flow_does_not_invent_ordinal_only_chain() {
        let projection = linked_text_projection(false);
        let (fragments, diagnostics) =
            resolve_viewer_text_fragments(&projection).expect("fallback flow should resolve");

        assert!(fragments.is_empty());
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == "shared_story_without_explicit_flow" })
        );
    }

    #[test]
    fn viewer_text_fragment_contract_is_source_neutral() {
        let fragment = ViewerTextFragment {
            story_id: StoryId::from_canonical(id(50)),
            frame_id: NodeId::from_canonical(id(51)),
            scalar_start: 2,
            scalar_end: 5,
            text: "abc".to_owned(),
            line_count: 1,
        };
        let json = serde_json::to_string(&fragment).expect("serialize Viewer text fragment");

        assert!(json.contains("story_id"));
        assert!(json.contains("frame_id"));
        assert!(json.contains("scalar_start"));
        for forbidden in ["Quill", "FDPC", "BTEC", "Contents", "Escher", "offset"] {
            assert!(
                !json.contains(forbidden),
                "Viewer text fragment must not expose parser-private {forbidden}"
            );
        }
    }

    #[test]
    fn viewer_text_projection_refresh_uses_current_resolved_story_state() {
        let mut graph = resolved_graph_fixture();
        let page_id = graph.document.pages[0];
        let node_id = *graph.nodes.keys().next().expect("fixture node");
        let story_id = *graph.stories.keys().next().expect("fixture story");

        graph.pages.get_mut(&page_id).expect("fixture page").size =
            Size2D::new(LengthEmu::new(2_000_000), LengthEmu::new(2_000_000));
        graph
            .pages
            .get_mut(&page_id)
            .expect("fixture page")
            .children = vec![node_id];
        graph
            .nodes
            .get_mut(&node_id)
            .expect("fixture node")
            .header
            .bounds = RectEmu::new(
            LengthEmu::ZERO,
            LengthEmu::ZERO,
            LengthEmu::new(1_000_000),
            LengthEmu::new(1_000_000),
        );

        let projection =
            project_bounded(bounded_authoring_slice_from_resolved(&graph).expect("projection"));
        let scene = resolve_bounded_geometry(&projection, viewer_geometry_environment_v0_1())
            .expect("scene");
        let (initial_fragments, _) =
            resolve_viewer_text_fragments(&projection).expect("initial text flow");
        assert!(!initial_fragments.is_empty());

        let source_hash = graph.source.source_hash;
        let mut visual = ViewerGeometryDocument {
            schema_version: VIEWER_GEOMETRY_SCHEMA_V0_1.to_owned(),
            document: ViewerDocument {
                schema_version: VIEWER_DOCUMENT_SCHEMA_V0_1.to_owned(),
                source: ViewerSource {
                    format: "pub".to_owned(),
                    format_version: Some("0x2c".to_owned()),
                    source_hash,
                    byte_len: 1,
                },
                pages: vec![ViewerPage {
                    index: 1,
                    id: page_id,
                    width_emu: graph.pages[&page_id].size.width.get(),
                    height_emu: graph.pages[&page_id].size.height.get(),
                }],
                stories: vec![ViewerStory {
                    id: story_id,
                    text: graph.stories[&story_id].text.clone(),
                }],
                diagnostics: vec![viewer_fallback_flow_metrics_diagnostic()],
            },
            scene,
            paints: Vec::new(),
            story_frames: Vec::new(),
            text_fragments: initial_fragments,
            typography_runs: Vec::new(),
            images: Vec::new(),
            projected_instances: Vec::new(),
        };

        graph
            .stories
            .get_mut(&story_id)
            .expect("fixture story")
            .text = "Changed after edit".to_owned();

        visual
            .refresh_text_projection_from_resolved(&graph)
            .expect("refresh current editor graph");

        assert_eq!(visual.document.stories[0].text, "Changed after edit");
        assert_eq!(
            visual
                .text_fragments
                .iter()
                .map(|fragment| fragment.text.as_str())
                .collect::<String>(),
            "Changed after edit"
        );
        assert_eq!(visual.story_frames.len(), 1);
        assert_eq!(
            visual
                .document
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "viewer.text.fallback_flow_metrics")
                .count(),
            1
        );
    }

    #[test]
    fn viewer_text_projection_refresh_reflows_same_explicit_linked_chain() {
        let mut graph = linked_resolved_graph_fixture("ABCDEFGHI");
        let page_id = graph.document.pages[0];
        let story_id = *graph.stories.keys().next().expect("fixture story");
        let projection =
            project_bounded(bounded_authoring_slice_from_resolved(&graph).expect("projection"));
        let scene = resolve_bounded_geometry(&projection, viewer_geometry_environment_v0_1())
            .expect("scene");
        let (initial_fragments, initial_flow_diagnostics) =
            resolve_viewer_text_fragments(&projection).expect("initial linked flow");
        assert_eq!(
            initial_fragments
                .iter()
                .map(|fragment| fragment.text.as_str())
                .collect::<String>(),
            "ABCDEFGH"
        );
        assert!(
            initial_flow_diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "story_overset")
        );

        let initial_frames = projection
            .story_frames
            .iter()
            .map(|frame| ViewerStoryFrame {
                story_id: frame.story_origin,
                frame_id: frame.frame_origin,
                ordinal: frame.ordinal,
            })
            .collect::<Vec<_>>();
        let mut diagnostics = initial_flow_diagnostics
            .iter()
            .map(map_scene_diagnostic)
            .collect::<Vec<_>>();
        diagnostics.push(viewer_fallback_flow_metrics_diagnostic());
        normalize_diagnostics(&mut diagnostics);

        let source_hash = graph.source.source_hash;
        let mut visual = ViewerGeometryDocument {
            schema_version: VIEWER_GEOMETRY_SCHEMA_V0_1.to_owned(),
            document: ViewerDocument {
                schema_version: VIEWER_DOCUMENT_SCHEMA_V0_1.to_owned(),
                source: ViewerSource {
                    format: "pub".to_owned(),
                    format_version: Some("0x2c".to_owned()),
                    source_hash,
                    byte_len: 1,
                },
                pages: vec![ViewerPage {
                    index: 1,
                    id: page_id,
                    width_emu: graph.pages[&page_id].size.width.get(),
                    height_emu: graph.pages[&page_id].size.height.get(),
                }],
                stories: vec![ViewerStory {
                    id: story_id,
                    text: "ABCDEFGHI".to_owned(),
                }],
                diagnostics,
            },
            scene,
            paints: Vec::new(),
            story_frames: initial_frames.clone(),
            text_fragments: initial_fragments,
            typography_runs: Vec::new(),
            images: Vec::new(),
            projected_instances: Vec::new(),
        };

        graph
            .stories
            .get_mut(&story_id)
            .expect("fixture story")
            .text = "XYZ1234".to_owned();

        visual
            .refresh_text_projection_from_resolved(&graph)
            .expect("refresh linked Story");

        assert_eq!(
            visual
                .text_fragments
                .iter()
                .map(|fragment| fragment.text.as_str())
                .collect::<String>(),
            "XYZ1234"
        );
        assert_eq!(visual.story_frames, initial_frames);
        assert!(
            !visual
                .document
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "viewer.text.fallback_overset")
        );
    }

    #[test]
    fn viewer_text_projection_refresh_rejects_source_identity_change_transactionally() {
        let graph = resolved_graph_fixture();
        let source_hash = graph.source.source_hash;
        let mut visual = ViewerGeometryDocument {
            schema_version: VIEWER_GEOMETRY_SCHEMA_V0_1.to_owned(),
            document: ViewerDocument {
                schema_version: VIEWER_DOCUMENT_SCHEMA_V0_1.to_owned(),
                source: ViewerSource {
                    format: "pub".to_owned(),
                    format_version: Some("0x2c".to_owned()),
                    source_hash,
                    byte_len: 1,
                },
                pages: Vec::new(),
                stories: vec![ViewerStory {
                    id: StoryId::from_canonical(id(99)),
                    text: "keep me".to_owned(),
                }],
                diagnostics: Vec::new(),
            },
            scene: resolve_bounded_geometry(
                &project_bounded(
                    bounded_authoring_slice_from_resolved(&graph).expect("projection"),
                ),
                viewer_geometry_environment_v0_1(),
            )
            .expect("scene"),
            paints: Vec::new(),
            story_frames: Vec::new(),
            text_fragments: Vec::new(),
            typography_runs: Vec::new(),
            images: Vec::new(),
            projected_instances: Vec::new(),
        };
        let before = visual.clone();
        visual.document.source.source_hash = Sha256Digest::from_bytes([0xCD; 32]);
        let mismatched_before = visual.clone();

        assert!(
            visual
                .refresh_text_projection_from_resolved(&graph)
                .is_err()
        );
        assert_eq!(visual, mismatched_before);
        assert_ne!(visual, before);
    }

    #[test]
    fn viewer_geometry_environment_is_explicit_and_stable() {
        assert_eq!(
            viewer_geometry_environment_v0_1(),
            viewer_geometry_environment_v0_1()
        );
    }
}
