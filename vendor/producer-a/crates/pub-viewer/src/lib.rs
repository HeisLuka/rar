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
    BoundedAuthoringSlice, BoundedNodeGeometryInput, ProjectionDiagnostic, ResolveDiagnostic,
    project_bounded, resolve_bounded_geometry,
};
pub use pub_layout::{BoundedLayoutEnvironment, BoundedResolvedScene};
use pub_model::{NodeId, PageId, ResourceId, Sha256Digest, StoryFrame, StoryId};
pub use pub_reader::{
    CHAPTERA_EXACT_FILE_CONSENT_V1, CHAPTERA_INTAKE_RETENTION_POLICY_V1, FailureIntakeClass,
    FailureIntakeClassification, FailureIntakeConfidence, FailureIntakeReason,
    classify_failure_candidate, exact_file_intake_eligible,
};
use pub_reader::{
    FailureCode, FailureEnvelope, FailureEnvelopeContext, FailureParserStage,
    FailureTelemetryChoice, PubAssetExportDiagnostic, PubBridgeDiagnostic, PubResolveDiagnostic,
    PubReaderOpenTiming, PubResolvedGraph, PubResolvedGraphBuild, PubSourceGraphBuild,
    build_failure_envelope, build_mature_0x2c_asset_export_bundle_from_bytes,
    build_mature_0x2c_source_graph, build_mature_0x2c_source_graph_with_timing,
    resolve_pub_source_graph,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::time::{Duration, Instant};

pub const VIEWER_DOCUMENT_SCHEMA_V0_1: &str = "0.1";
pub const VIEWER_GEOMETRY_SCHEMA_V0_1: &str = "0.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ViewerOpenTiming {
    pub reader: PubReaderOpenTiming,
    pub resolve_model_ns: u64,
    pub viewer_document_projection_ns: u64,
    pub dependency_resolution_ns: u64,
    pub layout_projection_ns: u64,
    pub visible_resource_materialization_ns: u64,
    pub scene_resolution_ns: u64,
    pub page_count: u64,
    pub story_count: u64,
    pub resource_count: u64,
}

fn duration_ns_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

pub const VIEWER_FAILURE_REPORT_SCHEMA_V0_1: &str = "chaptera-viewer-failure-report/v0.1";

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
    pub images: Vec<ViewerEmbeddedImage>,
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
    Ok(open_mature_0x2c_geometry_with_timing(bytes, environment)?.0)
}

pub fn open_mature_0x2c_geometry_with_timing(
    bytes: &[u8],
    environment: BoundedLayoutEnvironment,
) -> Result<(ViewerGeometryDocument, ViewerOpenTiming)> {
    let (pipeline, reader_timing, resolve_model_ns) =
        build_mature_0x2c_pipeline_with_timing(bytes)?;

    let document_started = Instant::now();
    let mut document = viewer_document_from_pipeline(bytes.len(), &pipeline)?;
    let viewer_document_projection_ns = duration_ns_u64(document_started.elapsed());

    let dependency_started = Instant::now();
    let authoring = bounded_authoring_slice_from_resolved(&pipeline.resolved.graph)?;
    let dependency_resolution_ns = duration_ns_u64(dependency_started.elapsed());

    let layout_started = Instant::now();
    let projection = project_bounded(authoring);
    let layout_projection_ns = duration_ns_u64(layout_started.elapsed());

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

    let resources_started = Instant::now();
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

    let visible_resource_materialization_ns = duration_ns_u64(resources_started.elapsed());

    let scene_started = Instant::now();
    let scene = resolve_bounded_geometry(&projection, environment).map_err(|blocked| {
        let codes = blocked
            .projection_errors
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        anyhow!("Viewer geometry resolution blocked by layout projection errors: {codes}")
    })?;

    let scene_resolution_ns = duration_ns_u64(scene_started.elapsed());

    document
        .diagnostics
        .extend(scene.diagnostics.iter().map(map_scene_diagnostic));

    if !scene.nodes.is_empty() {
        document.diagnostics.push(ViewerDiagnostic {
            code: "viewer.visual.geometry_only".to_owned(),
            severity: ViewerDiagnosticSeverity::FidelityWarning,
            message: "Object positions and sizes are resolved. The desktop Viewer may paint bounded single-frame semantic text, exact embedded PNG/JPEG bytes, and complete explicit shape-local solid fill/line state when available. Inherited/default paint, linked text flow, typography, image crop/fit, gradients/patterns, effects, and transforms are not faithfully painted yet."
                .to_owned(),
        });
    }
    normalize_diagnostics(&mut document.diagnostics);

    let page_count = u64::try_from(document.pages.len()).unwrap_or(u64::MAX);
    let story_count = u64::try_from(document.stories.len()).unwrap_or(u64::MAX);
    let resource_count = u64::try_from(images.len()).unwrap_or(u64::MAX);

    Ok((
        ViewerGeometryDocument {
            schema_version: VIEWER_GEOMETRY_SCHEMA_V0_1.to_owned(),
            document,
            scene,
            paints,
            story_frames,
            images,
        },
        ViewerOpenTiming {
            reader: reader_timing,
            resolve_model_ns,
            viewer_document_projection_ns,
            dependency_resolution_ns,
            layout_projection_ns,
            visible_resource_materialization_ns,
            scene_resolution_ns,
            page_count,
            story_count,
            resource_count,
        },
    ))
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
    Ok(build_mature_0x2c_pipeline_with_timing(bytes)?.0)
}

fn build_mature_0x2c_pipeline_with_timing(
    bytes: &[u8],
) -> Result<(Mature0x2cPipeline, PubReaderOpenTiming, u64)> {
    let source_hash = sha256_digest(bytes)?;
    let (source, reader_timing) =
        build_mature_0x2c_source_graph_with_timing(Cursor::new(bytes), source_hash)
            .context("build mature-0x2C PUB source graph for Viewer")?;
    let resolve_started = Instant::now();
    let resolved =
        resolve_pub_source_graph(&source.graph).context("resolve PUB source graph for Viewer")?;
    let resolve_model_ns = duration_ns_u64(resolve_started.elapsed());

    Ok((
        Mature0x2cPipeline {
            source_hash,
            source,
            resolved,
        },
        reader_timing,
        resolve_model_ns,
    ))
}

fn viewer_document_from_pipeline(
    byte_len: usize,
    pipeline: &Mature0x2cPipeline,
) -> Result<ViewerDocument> {
    let graph = &pipeline.resolved.graph;

    let mut pages = Vec::with_capacity(graph.document.pages.len());
    for (zero_based, page_id) in graph.document.pages.iter().enumerate() {
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
/// This bridge intentionally lives in the Viewer orchestration layer for now:
/// there is only one concrete consumer. It should be factored into a reusable
/// engine adapter only after a second real consumer requires the same mapping.
fn bounded_authoring_slice_from_resolved(
    graph: &PubResolvedGraph,
) -> Result<BoundedAuthoringSlice> {
    let pages = graph
        .document
        .pages
        .iter()
        .map(|page_id| {
            graph
                .pages
                .get(page_id)
                .cloned()
                .with_context(|| format!("layout projection missing document page {page_id:?}"))
        })
        .collect::<Result<Vec<_>>>()?;

    let node_geometry = graph
        .nodes
        .values()
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

    #[test]
    fn viewer_geometry_environment_is_explicit_and_stable() {
        assert_eq!(
            viewer_geometry_environment_v0_1(),
            viewer_geometry_environment_v0_1()
        );
    }
}
