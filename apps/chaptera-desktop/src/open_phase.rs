use std::{collections::BTreeSet, fs, path::Path, time::Instant};

use pub_viewer::{
    ViewerGeometryDocument, open_mature_0x2c_geometry, open_mature_0x2c_geometry_with_timing,
    viewer_geometry_environment_v0_1,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub const RUN_SCHEMA_V1: &str = "chaptera.open-first-useful-page-run.v1";

#[derive(Debug, Clone, Serialize)]
pub struct PhaseObservation {
    pub phase_id: &'static str,
    pub start_ms: f64,
    pub end_ms: f64,
    pub cpu_ms: Option<f64>,
    pub scope: &'static str,
    pub bytes_read: u64,
    pub bytes_materialized: u64,
    pub pages_touched: u64,
    pub stories_touched: u64,
    pub resources_touched: u64,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FirstUsefulDefinition {
    pub page_geometry_present: bool,
    pub fidelity_diagnostics_present: bool,
    pub visible_resources_ready: bool,
    pub current_text_layout_present: bool,
    pub non_empty_visible_content: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EquivalenceObservation {
    pub canonical_document_equal: bool,
    pub final_scene_equal: bool,
    pub final_search_projection_equal: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeIdentity {
    pub runtime: &'static str,
    pub platform: &'static str,
    pub arch: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenPhaseRun {
    pub schema_version: &'static str,
    pub cache_state: String,
    pub first_useful_page_ms: f64,
    pub fully_ready_ms: f64,
    pub first_useful_page_definition: FirstUsefulDefinition,
    pub phases: Vec<PhaseObservation>,
    pub document_global_before_first_useful_page: Vec<&'static str>,
    pub final_equivalence: EquivalenceObservation,
    pub source_sha256: String,
    pub source_bytes: u64,
    pub page_count: u64,
    pub runtime_identity: RuntimeIdentity,
    pub instrumented_output_sha256: String,
    pub instrumentation_overhead_ms: f64,
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn ns_ms(value: u64) -> f64 {
    value as f64 / 1_000_000.0
}

fn hash_json<T: Serialize>(value: &T) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn decode_visible_resources(visual: &ViewerGeometryDocument) -> Result<(), String> {
    for embedded in &visual.images {
        let format = match embedded.mime.as_str() {
            "image/png" => image::ImageFormat::Png,
            "image/jpeg" => image::ImageFormat::Jpeg,
            _ => continue,
        };
        image::load_from_memory_with_format(&embedded.bytes, format)
            .map_err(|error| format!("decode visible resource: {error}"))?
            .to_rgba8();
    }
    Ok(())
}

fn search_probe(document: &pub_viewer::ViewerDocument) -> Option<String> {
    document.stories.iter().find_map(|story| {
        story.text.split_whitespace().find_map(|raw| {
            let word = raw
                .trim_matches(|ch: char| !ch.is_alphanumeric())
                .to_owned();
            (word.chars().count() >= 4).then_some(word)
        })
    })
}

fn phase(
    phase_id: &'static str,
    cursor: &mut f64,
    duration_ms: f64,
    scope: &'static str,
    counts: (u64, u64, u64, u64, u64),
    note: impl Into<String>,
) -> PhaseObservation {
    let start_ms = *cursor;
    let end_ms = start_ms + duration_ms.max(0.0);
    *cursor = end_ms;
    PhaseObservation {
        phase_id,
        start_ms,
        end_ms,
        cpu_ms: None,
        scope,
        bytes_read: counts.0,
        bytes_materialized: counts.1,
        pages_touched: counts.2,
        stories_touched: counts.3,
        resources_touched: counts.4,
        note: note.into(),
    }
}

pub fn run_one(path: &Path, cache_state: &str) -> Result<OpenPhaseRun, String> {
    let total_started = Instant::now();

    let file_started = Instant::now();
    let bytes = fs::read(path).map_err(|error| format!("read fixture: {error}"))?;
    let source_open_ms = elapsed_ms(file_started);
    let source_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let source_sha256 = format!("{:x}", Sha256::digest(&bytes));

    let (visual, timing) = open_mature_0x2c_geometry_with_timing(
        &bytes,
        viewer_geometry_environment_v0_1(),
    )
    .map_err(|error| format!("instrumented Viewer open: {error:#}"))?;

    let decode_started = Instant::now();
    decode_visible_resources(&visual)?;
    let decode_ms = elapsed_ms(decode_started);

    let paint_started = Instant::now();
    let first_page = visual
        .document
        .pages
        .first()
        .ok_or_else(|| "document has no Viewer pages".to_owned())?;
    let first_surface = visual
        .scene
        .surfaces
        .iter()
        .find(|surface| surface.origin == first_page.id);
    let page_origin = first_page.id.into_canonical();
    let page_nodes = visual
        .scene
        .nodes
        .iter()
        .filter(|node| node.parent_origin == page_origin)
        .map(|node| node.origin)
        .collect::<BTreeSet<_>>();
    let has_text_layout = visual
        .story_frames
        .iter()
        .any(|frame| page_nodes.contains(&frame.frame_id));
    let non_empty_visible_content = !page_nodes.is_empty();
    let paint_ready = first_surface.is_some() && non_empty_visible_content;
    let paint_ms = elapsed_ms(paint_started);

    let search_started = Instant::now();
    let probe = search_probe(&visual.document);
    let instrumented_search = probe
        .as_deref()
        .map(|query| visual.document.search_text(query))
        .unwrap_or_default();
    let search_ms = elapsed_ms(search_started);

    let measured_total_ms = elapsed_ms(total_started);

    // Equivalence is checked after the measured interval so the baseline open
    // cannot pollute the timing being reported.
    let plain = open_mature_0x2c_geometry(&bytes, viewer_geometry_environment_v0_1())
        .map_err(|error| format!("plain Viewer equivalence open: {error:#}"))?;
    let plain_search = probe
        .as_deref()
        .map(|query| plain.document.search_text(query))
        .unwrap_or_default();

    let page_count = u64::try_from(visual.document.pages.len()).unwrap_or(u64::MAX);
    let story_count = u64::try_from(visual.document.stories.len()).unwrap_or(u64::MAX);
    let resource_count = u64::try_from(visual.images.len()).unwrap_or(u64::MAX);

    let mut cursor = 0.0;
    let mut phases = Vec::new();
    phases.push(phase(
        "source_container_open",
        &mut cursor,
        source_open_ms,
        "bounded_dependencies",
        (source_bytes, source_bytes, 0, 0, 0),
        "real filesystem read of the supplied PUB fixture",
    ));
    phases.push(phase(
        "profile_classification",
        &mut cursor,
        0.0,
        "bounded_dependencies",
        (0, 0, 0, 0, 0),
        "current smoke path calls the mature-0x2c entrypoint explicitly; no separate classifier runs",
    ));
    phases.push(phase(
        "logical_stream_read",
        &mut cursor,
        ns_ms(
            timing
                .reader
                .input_materialization_ns
                .saturating_add(timing.reader.logical_stream_read_ns),
        ),
        "document_global",
        (
            timing.reader.source_bytes,
            timing.reader.logical_stream_bytes,
            0,
            0,
            0,
        ),
        "current Reader materializes the full input and exact Contents/Quill/Escher streams",
    ));
    phases.push(phase(
        "parse_model_projection",
        &mut cursor,
        ns_ms(timing.reader.parse_source_graph_ns)
            + ns_ms(timing.resolve_model_ns)
            + ns_ms(timing.viewer_document_projection_ns)
            + ns_ms(timing.orchestration_ns),
        "document_global",
        (0, 0, page_count, story_count, 0),
        "current source graph, resolved graph, Viewer document projection and inter-stage Viewer orchestration are eager/document-global",
    ));
    phases.push(phase(
        "first_page_dependency_resolution",
        &mut cursor,
        ns_ms(timing.dependency_resolution_ns),
        "document_global",
        (0, 0, page_count, story_count, 0),
        "current bounded authoring slice is built from the resolved document graph, not lazily per page",
    ));
    phases.push(phase(
        "first_page_layout_scene",
        &mut cursor,
        ns_ms(timing.layout_projection_ns) + ns_ms(timing.scene_resolution_ns),
        "document_global",
        (0, 0, page_count, story_count, 0),
        "current bounded layout/scene projection resolves the document-wide geometry slice",
    ));
    phases.push(phase(
        "visible_resource_decode",
        &mut cursor,
        ns_ms(timing.visible_resource_materialization_ns) + decode_ms,
        "document_global",
        (0, 0, page_count, 0, resource_count),
        "exact embedded PNG/JPEG bytes are materialized by Viewer and decoded through the same image crate used by desktop textures",
    ));
    phases.push(phase(
        "first_paint",
        &mut cursor,
        paint_ms,
        "first_page_only",
        (0, 0, 1, if has_text_layout { 1 } else { 0 }, resource_count),
        "hosted headless paint-readiness boundary; separate reader-only WGPU acceptance proves an actual rendered first frame",
    ));
    let first_useful_page_ms = cursor;
    phases.push(phase(
        "background_remaining_document",
        &mut cursor,
        0.0,
        "background",
        (0, 0, 0, 0, 0),
        "current open path is eager; no deferred remaining-document work is scheduled",
    ));
    phases.push(phase(
        "search_index_ready",
        &mut cursor,
        search_ms,
        "background",
        (0, 0, 0, story_count, 0),
        "current Viewer search scans recovered Story text directly; no separate index build exists",
    ));
    let fully_ready_ms = cursor;

    let document_global_before_first_useful_page = phases
        .iter()
        .filter(|phase| {
            phase.scope == "document_global" && phase.start_ms < first_useful_page_ms
        })
        .map(|phase| phase.phase_id)
        .collect::<Vec<_>>();

    let accounted_ms = phases
        .iter()
        .map(|phase| phase.end_ms - phase.start_ms)
        .sum::<f64>();
    let instrumentation_overhead_ms = (measured_total_ms - accounted_ms).max(0.0);

    Ok(OpenPhaseRun {
        schema_version: RUN_SCHEMA_V1,
        cache_state: cache_state.to_owned(),
        first_useful_page_ms,
        fully_ready_ms,
        first_useful_page_definition: FirstUsefulDefinition {
            page_geometry_present: first_surface.is_some(),
            fidelity_diagnostics_present: true,
            visible_resources_ready: true,
            current_text_layout_present: has_text_layout,
            non_empty_visible_content,
        },
        phases,
        document_global_before_first_useful_page,
        final_equivalence: EquivalenceObservation {
            canonical_document_equal: visual.document == plain.document,
            final_scene_equal: visual.scene == plain.scene,
            final_search_projection_equal: instrumented_search == plain_search,
        },
        source_sha256,
        source_bytes,
        page_count,
        runtime_identity: RuntimeIdentity {
            runtime: "chaptera-reader",
            platform: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        },
        instrumented_output_sha256: hash_json(&visual)?,
        instrumentation_overhead_ms,
    })
}

pub fn run_series(
    path: &Path,
    cache_state: &str,
    count: usize,
) -> Result<Vec<OpenPhaseRun>, String> {
    if count == 0 || count > 16 {
        return Err("run count must be 1..=16".to_owned());
    }
    (0..count)
        .map(|_| run_one(path, cache_state))
        .collect::<Result<Vec<_>, _>>()
}
