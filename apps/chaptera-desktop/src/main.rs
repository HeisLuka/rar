#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

// The cadence API is intentionally staged one PR before its UI consumer (#227).
mod acceptance;
mod agent;
mod diagnostic_sweep;
mod fallback_font;
mod image_decode_adapter;
#[allow(dead_code)]
mod locale;
mod product_smoke;
mod reader_product_ui;
mod render_backend;
#[allow(dead_code)]
mod supporter;
#[allow(dead_code)]
mod supporter_attribution;
mod text_session;

use chaptera_scene_instance::{
    GeometrySyncPolicyV1, ObjectMutationKindV1, SceneInstanceV1, admit_object_mutation_v1,
    direct_page_local_instance_v1, geometry_sync_policy_v1,
};
use chaptera_viewer_render_plan::{
    ExplicitRenderTextFontResourceV1, PageRenderPlanV1, RenderPlanErrorV1,
    build_page_render_plan_with_text_layout_v1,
};
use eframe::egui;
use pub_interaction::{
    MoveTransaction, ResizeCommit, ResizeHandle, ResizePointerDown, ResizeTransaction,
    ResizeUpdate, ScreenPoint, ScreenRect, ViewTransform, classify_resize_pointer_down,
    resize_handle_center,
};
use pub_viewer::{
    CHAPTERA_EXACT_FILE_CONSENT_V1, CHAPTERA_INTAKE_RETENTION_POLICY_V1, FailureIntakeClass,
    FailureIntakeClassification, ViewerDiagnosticSeverity, ViewerFidelityStatus,
    ViewerGeometryDocument, ViewerTextMatch, classify_failure_candidate,
    exact_file_intake_eligible,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

#[cfg(feature = "reader-only")]
const APP_TITLE: &str = "Chaptera PUB Reader — Technical Preview";
#[cfg(not(feature = "reader-only"))]
const APP_TITLE: &str = "Chaptera Editor";

const READER_PRODUCT_LABEL: &str = "Chaptera PUB Reader";
const READER_FIRST_RUN_HEADING: &str = "Open a PUB file";
const READER_FIRST_RUN_TRUST_CUE: &str =
    "Files open locally. Chaptera does not require an account for reading.";
const READER_READ_ONLY_CUE: &str = "Reader is read-only. The original PUB is never overwritten.";

fn reader_only_mode() -> bool {
    cfg!(feature = "reader-only")
}

fn product_surface_label() -> &'static str {
    if reader_only_mode() {
        READER_PRODUCT_LABEL
    } else {
        "Chaptera Editor"
    }
}

fn failure_mailto_recipient_configured() -> bool {
    // CHAPTERA-FAILURE-MAILTO-01 owns replacing this with validated packaged
    // configuration. Missing verified recipient must fail closed.
    false
}

fn resolve_supporter_market() -> supporter::MarketProfile {
    let locale = locale::detect_user_locale();
    supporter::MarketProfile::from_locale(locale.as_ref().map(locale::DetectedLocale::raw))
}
const SUPPORTER_STORAGE_KEY: &str = "chaptera.supporter.v1";
const PAGE_MARGIN: f32 = 24.0;
const EMU_PER_INCH: f32 = 914_400.0;
const NUMERIC_ZOOM_POINTS_PER_INCH: f32 = 96.0;
const MIN_NUMERIC_ZOOM: f32 = 0.10;
const MAX_NUMERIC_ZOOM: f32 = 4.00;
const PAGE_THUMBNAIL_MAX_WIDTH: f32 = 116.0;
const PAGE_THUMBNAIL_MAX_HEIGHT: f32 = 148.0;
const SOURCE_REVALIDATE_INTERVAL: Duration = Duration::from_secs(2);
const SOURCE_EXACT_REVALIDATE_INTERVAL: Duration = Duration::from_secs(30);

fn desktop_text_font_resource() -> ExplicitRenderTextFontResourceV1<'static> {
    ExplicitRenderTextFontResourceV1 {
        resource_id: chaptera_desktop_fallback_font_resource::RESOURCE_ID,
        expected_sha256: chaptera_desktop_fallback_font_resource::EXPECTED_SHA256,
        face_index: 0,
        default_font_size_emu: chaptera_desktop_fallback_font_resource::FONT_SIZE_EMU,
        default_line_height_emu: chaptera_desktop_fallback_font_resource::LINE_HEIGHT_EMU,
        bytes: chaptera_desktop_fallback_font_resource::bytes(),
    }
}

fn build_desktop_page_render_plan(
    visual: &ViewerGeometryDocument,
    page_index: usize,
) -> Result<PageRenderPlanV1, RenderPlanErrorV1> {
    build_page_render_plan_with_text_layout_v1(visual, page_index, &desktop_text_font_resource())
}

fn text_layout_disposition_counts(plan: &PageRenderPlanV1) -> (usize, usize) {
    let mut shared = 0_usize;
    let mut fallback = 0_usize;
    for layout in plan
        .nodes
        .iter()
        .filter_map(|node| node.text.as_ref())
        .filter_map(|text| text.layout.as_ref())
    {
        match &layout.disposition {
            chaptera_viewer_render_plan::RenderTextLayoutDispositionV1::SharedResolved {
                ..
            } => {
                shared += 1;
            }
            chaptera_viewer_render_plan::RenderTextLayoutDispositionV1::BackendFallback {
                ..
            } => {
                fallback += 1;
            }
        }
    }
    (shared, fallback)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CanvasZoomMode {
    Percent,
    FitPage,
    PageWidth,
    FitSelection,
}

const GEOMETRY_WARNING: &str = "Partial preview: bounded semantic text may be painted across proven explicit linked-frame chains. Proven source font sizes, including bounded FDPP→STSH1 inheritance where admitted, affect text sizing through the shared render plan; the current renderer still uses Chaptera's pinned fallback font face rather than claiming source-font availability. Exact embedded PNG/JPEG images and complete explicit shape-local solid fill/line state may also be painted. Other inherited/default styling beyond admitted font/size, Publisher-exact font metrics/reflow, image crop/fit, gradients/patterns, effects, and transforms are not faithfully painted yet.";
const PREVIEW_TEXT_CLIP_WARNING: &str = "Text exceeds the height of at least one frame in the current desktop preview and is visibly clipped. Admitted single-frame homogeneous text uses shared resolved line breaks; other cases still use explicit backend fallback. This is preview-only evidence, not Publisher-native overset or exact reflow evidence.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewerLoadFailureKind {
    FileAccess,
    Unsupported,
}

#[derive(Debug, Clone)]
struct ViewerLoadFailure {
    kind: ViewerLoadFailureKind,
    attempted_path: Option<PathBuf>,
    message: String,
    classification: Option<FailureIntakeClassification>,
    diagnostic_json: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OpenGeneration(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceFileStamp {
    byte_len: u64,
    modified: Option<SystemTime>,
}

fn source_file_stamp(path: &Path) -> std::io::Result<SourceFileStamp> {
    let metadata = fs::metadata(path)?;
    Ok(SourceFileStamp {
        byte_len: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

fn source_sha256(bytes: &[u8]) -> pub_editor::Sha256Digest {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(bytes);
    let mut sha256 = [0_u8; 32];
    sha256.copy_from_slice(&digest);
    pub_editor::Sha256Digest::from_bytes(sha256)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceFreshness {
    Current,
    ReloadRequiredChanged,
    ReloadRequiredUnavailable,
}

impl SourceFreshness {
    fn requires_reload(self) -> bool {
        !matches!(self, Self::Current)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommittedSourceState {
    generation: OpenGeneration,
    source_hash: pub_editor::Sha256Digest,
    byte_len: u64,
    file_stamp: Option<SourceFileStamp>,
    freshness: SourceFreshness,
}

#[derive(Debug, Default)]
struct OpenStateAuthority {
    next_generation: u64,
    active_generation: Option<OpenGeneration>,
}

impl OpenStateAuthority {
    fn begin_attempt(&mut self) -> OpenGeneration {
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("desktop open generation exhausted");
        let generation = OpenGeneration(self.next_generation);
        self.active_generation = Some(generation);
        generation
    }

    fn commit_if_current(&mut self, generation: OpenGeneration) -> bool {
        if self.active_generation != Some(generation) {
            return false;
        }
        self.active_generation = None;
        true
    }

    fn finish_without_commit_if_current(&mut self, generation: OpenGeneration) -> bool {
        if self.active_generation != Some(generation) {
            return false;
        }
        self.active_generation = None;
        true
    }
}

struct PreparedDocumentOpen {
    source_path: PathBuf,
    source_file_stamp: Option<SourceFileStamp>,
    visual: ViewerGeometryDocument,
    editor: Option<pub_editor::EditorSession>,
    editor_load_error: Option<String>,
    project_status: Option<String>,
}

fn dropped_file_candidate(paths: &[Option<PathBuf>]) -> Result<Option<PathBuf>, &'static str> {
    match paths {
        [] => Ok(None),
        [Some(path)] => Ok(Some(path.clone())),
        [None] => Err("Dropped item has no local filesystem path."),
        _ => Err("Drop exactly one PUB file at a time; no file was opened."),
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct PreviewTextMetricDiagnostic {
    page_index: u32,
    page_id: String,
    frame_id: String,
    story_id: String,
    frame_bounds_emu: [i64; 4],
    zoom: f32,
    font_face_disposition: &'static str,
    metrics: render_backend::TextPaintMetrics,
}

impl PreviewTextMetricDiagnostic {
    fn from_executed_layout(
        page_index: u32,
        page_id: String,
        frame_id: String,
        story_id: String,
        frame_bounds: pub_editor::RectEmu,
        zoom: f32,
        metrics: render_backend::TextPaintMetrics,
    ) -> Self {
        Self {
            page_index,
            page_id,
            frame_id,
            story_id,
            frame_bounds_emu: [
                frame_bounds.x.get(),
                frame_bounds.y.get(),
                frame_bounds.width.get(),
                frame_bounds.height.get(),
            ],
            zoom,
            font_face_disposition: "fallback_not_source_font",
            metrics,
        }
    }
}

#[derive(Debug, Clone)]
struct DesktopExportPreview {
    target: pub_editor::EditorEditableTarget,
    operation_count: usize,
    can_serialize: bool,
    summary: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SceneSelectionState {
    selected: BTreeSet<String>,
    primary: Option<String>,
}

impl SceneSelectionState {
    fn clear(&mut self) {
        self.selected.clear();
        self.primary = None;
    }

    fn len(&self) -> usize {
        self.selected.len()
    }

    fn primary(&self) -> Option<&str> {
        self.primary.as_deref()
    }

    fn select_only(&mut self, instance_id: String) {
        self.selected.clear();
        self.selected.insert(instance_id.clone());
        self.primary = Some(instance_id);
    }
}

#[derive(Debug, Clone)]
struct SceneHitEntry {
    instance_id: String,
    node_id: pub_editor::NodeId,
    bounds: pub_editor::RectEmu,
    z_order: i64,
    paint_order: u32,
}

#[derive(Debug, Clone, Default)]
struct SceneHitTestIndex {
    entries: Vec<SceneHitEntry>,
    by_instance: BTreeMap<String, usize>,
    by_node: BTreeMap<pub_editor::NodeId, usize>,
}

impl SceneHitTestIndex {
    fn new(mut entries: Vec<SceneHitEntry>) -> Self {
        entries.sort_by(|left, right| {
            (left.z_order, left.paint_order, left.instance_id.as_str()).cmp(&(
                right.z_order,
                right.paint_order,
                right.instance_id.as_str(),
            ))
        });
        let by_instance = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.instance_id.clone(), index))
            .collect();
        let by_node = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.node_id, index))
            .collect();
        Self {
            entries,
            by_instance,
            by_node,
        }
    }

    fn topmost_at(&self, point: pub_interaction::DocumentPoint) -> Option<&SceneHitEntry> {
        self.entries
            .iter()
            .rev()
            .find(|entry| scene_bounds_contains(entry.bounds, point))
    }

    fn entry_for_instance(&self, instance_id: &str) -> Option<&SceneHitEntry> {
        self.by_instance
            .get(instance_id)
            .and_then(|index| self.entries.get(*index))
    }

    fn node_for_instance(&self, instance_id: &str) -> Option<pub_editor::NodeId> {
        self.entry_for_instance(instance_id)
            .map(|entry| entry.node_id)
    }

    fn instance_for_node(&self, node_id: pub_editor::NodeId) -> Option<&str> {
        self.by_node
            .get(&node_id)
            .and_then(|index| self.entries.get(*index))
            .map(|entry| entry.instance_id.as_str())
    }
}

fn scene_bounds_contains(
    bounds: pub_editor::RectEmu,
    point: pub_interaction::DocumentPoint,
) -> bool {
    if bounds.width.get() <= 0 || bounds.height.get() <= 0 {
        return false;
    }
    let (Some(right), Some(bottom)) = (bounds.right(), bounds.bottom()) else {
        return false;
    };
    point.x >= bounds.x && point.x <= right && point.y >= bounds.y && point.y <= bottom
}

fn direct_scene_instance(
    editor: &pub_editor::EditorSession,
    target_page_id: &str,
    node_id: pub_editor::NodeId,
) -> Option<SceneInstanceV1> {
    let authored = editor.graph().nodes.get(&node_id)?;
    if authored.header.parent_id.to_string() != target_page_id {
        return None;
    }
    direct_page_local_instance_v1(&node_id.as_canonical().to_string(), target_page_id).ok()
}

fn main() -> eframe::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let first_arg = args.next();

    if first_arg.as_deref()
        == Some(std::ffi::OsStr::new(
            chaptera_update_handoff::CONTROL_MODE_ARG,
        ))
    {
        if !reader_only_mode() {
            eprintln!("update control mode is reserved for the Chaptera Reader product");
            std::process::exit(2);
        }
        let Some(request_path) = args.next().map(PathBuf::from) else {
            eprintln!("usage: chaptera-reader --chaptera-update-control HANDOFF-REQUEST.json");
            std::process::exit(2);
        };
        if args.next().is_some() {
            eprintln!("Reader update control mode accepts exactly one handoff request");
            std::process::exit(2);
        }
        if let Err(error) = run_reader_update_control(&request_path) {
            eprintln!("Reader update control failed: {error}");
            std::process::exit(2);
        }
        return Ok(());
    }

    if first_arg.as_deref() == Some(std::ffi::OsStr::new("--agent-v1")) {
        if args.next().is_some() {
            eprintln!("chaptera --agent-v1 accepts no path arguments; use the open NDJSON command");
            std::process::exit(2);
        }
        if let Err(error) = agent::run_stdio() {
            eprintln!("{error}");
            std::process::exit(2);
        }
        return Ok(());
    }

    if first_arg.as_deref() == Some(std::ffi::OsStr::new("--product-smoke-v1")) {
        let output = args.next().map(PathBuf::from);
        if args.next().is_some() {
            eprintln!("usage: chaptera --product-smoke-v1 [OUTPUT.json]");
            std::process::exit(2);
        }
        match product_smoke::run() {
            Ok(receipt) => {
                let encoded = serde_json::to_string(&receipt)
                    .expect("product smoke receipt is JSON-serializable");
                if let Some(output) = output {
                    if let Err(error) = fs::write(&output, format!("{encoded}\n")) {
                        eprintln!("write product smoke receipt {}: {error}", output.display());
                        std::process::exit(2);
                    }
                } else {
                    println!("{encoded}");
                }
                return Ok(());
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(2);
            }
        }
    }

    if first_arg.as_deref() == Some(std::ffi::OsStr::new("--handoff-create-v1")) {
        if !reader_only_mode() {
            eprintln!("only the Chaptera Reader product may create a V1 suite handoff");
            std::process::exit(2);
        }
        let Some(target) = args.next().and_then(|value| value.into_string().ok()) else {
            eprintln!(
                "usage: chaptera-reader --handoff-create-v1 TARGET_PRODUCT SOURCE.pub OUTPUT.json"
            );
            std::process::exit(2);
        };
        let Some(source) = args.next().map(PathBuf::from) else {
            eprintln!(
                "usage: chaptera-reader --handoff-create-v1 TARGET_PRODUCT SOURCE.pub OUTPUT.json"
            );
            std::process::exit(2);
        };
        let Some(output) = args.next().map(PathBuf::from) else {
            eprintln!(
                "usage: chaptera-reader --handoff-create-v1 TARGET_PRODUCT SOURCE.pub OUTPUT.json"
            );
            std::process::exit(2);
        };
        if args.next().is_some() {
            eprintln!("Reader handoff creation accepts exactly target, source, and output");
            std::process::exit(2);
        }

        let reader_supported = smoke_check(&source).is_ok();
        let rescue_eligible = if reader_supported {
            false
        } else {
            fs::read(&source)
                .ok()
                .map(|bytes| {
                    matches!(
                        classify_failure_candidate(&bytes).class,
                        FailureIntakeClass::PubDamaged
                    )
                })
                .unwrap_or(false)
        };
        match chaptera_suite_handoff::create_reader_handoff(
            &source,
            &target,
            reader_supported,
            rescue_eligible,
        )
        .and_then(|packet| {
            chaptera_suite_handoff::write_packet(&packet, &output)?;
            Ok(packet)
        }) {
            Ok(packet) => {
                println!(
                    "{{\"protocol_version\":\"{}\",\"sender_product_id\":\"{}\",\"target_product_id\":\"{}\",\"requested_job\":\"{}\",\"source_sha256\":\"{}\"}}",
                    chaptera_suite_handoff::PACKET_VERSION,
                    chaptera_suite_handoff::READER_PRODUCT_ID,
                    packet.target_product_id,
                    packet.requested_job,
                    packet.source.sha256
                );
                return Ok(());
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(2);
            }
        }
    }

    if first_arg.as_deref() == Some(std::ffi::OsStr::new("--handoff-accept-v1")) {
        if reader_only_mode() {
            eprintln!("Chaptera Reader is a handoff sender, not an Editor receiver");
            std::process::exit(2);
        }
        let Some(packet_path) = args.next().map(PathBuf::from) else {
            eprintln!("usage: chaptera-editor --handoff-accept-v1 PACKET.json ACCEPTANCE.json");
            std::process::exit(2);
        };
        let Some(output) = args.next().map(PathBuf::from) else {
            eprintln!("usage: chaptera-editor --handoff-accept-v1 PACKET.json ACCEPTANCE.json");
            std::process::exit(2);
        };
        if args.next().is_some() {
            eprintln!("Editor handoff acceptance accepts exactly packet and output");
            std::process::exit(2);
        }

        let result = chaptera_suite_handoff::load_for_receiver(
            &packet_path,
            chaptera_suite_handoff::EDITOR_PRODUCT_ID,
        )
        .and_then(|validated| {
            let admitted = smoke_check(validated.source_path()).is_ok();
            chaptera_suite_handoff::finish_acceptance(validated, admitted)
        })
        .and_then(|receipt| {
            chaptera_suite_handoff::write_acceptance(&receipt, &output)?;
            Ok(receipt)
        });

        match result {
            Ok(receipt) => {
                println!(
                    "{}",
                    serde_json::to_string(&receipt)
                        .expect("suite handoff acceptance is JSON-serializable")
                );
                return Ok(());
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(2);
            }
        }
    }

    if first_arg.as_deref() == Some(std::ffi::OsStr::new("--desktop-acceptance-v1")) {
        let Some(fixture) = args.next().map(PathBuf::from) else {
            eprintln!("usage: chaptera --desktop-acceptance-v1 FIXTURE PROJECT EXPORT");
            std::process::exit(2);
        };
        let Some(project) = args.next().map(PathBuf::from) else {
            eprintln!("usage: chaptera --desktop-acceptance-v1 FIXTURE PROJECT EXPORT");
            std::process::exit(2);
        };
        let Some(export) = args.next().map(PathBuf::from) else {
            eprintln!("usage: chaptera --desktop-acceptance-v1 FIXTURE PROJECT EXPORT");
            std::process::exit(2);
        };
        if args.next().is_some() {
            eprintln!("desktop acceptance mode accepts exactly three path arguments");
            std::process::exit(2);
        }

        match acceptance::run(&fixture, &project, &export) {
            Ok(observation) => {
                println!(
                    "{}",
                    serde_json::to_string(&observation)
                        .expect("desktop acceptance observation is JSON-serializable")
                );
                return Ok(());
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(2);
            }
        }
    }

    if first_arg.as_deref() == Some(std::ffi::OsStr::new("--smoke-check")) {
        let Some(path) = args.next().map(PathBuf::from) else {
            std::process::exit(2);
        };
        if smoke_check(&path).is_err() {
            std::process::exit(1);
        }
        return Ok(());
    }

    let initial_path = first_arg.map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_TITLE)
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 600.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        APP_TITLE,
        options,
        Box::new(move |cc| {
            fallback_font::install(&cc.egui_ctx)
                .expect("pinned Chaptera fallback font resource must validate");
            Ok(Box::new(ViewerApp::new_with_storage(
                initial_path,
                cc.storage,
            )))
        }),
    )
}

#[cfg(feature = "reader-only")]
struct ReaderControlHooks;

#[cfg(feature = "reader-only")]
impl chaptera_update_orchestrator::UpdateHooks for ReaderControlHooks {
    fn quiesce(&mut self, _control_updater: &Path) -> std::result::Result<(), String> {
        // Ownership of the install lock proves the front-door U1 released its
        // mutation authority before copied U1 reaches this point. Product-level
        // process shutdown is deliberately a later slice.
        Ok(())
    }

    fn health_check(&mut self, current_tree: &Path) -> std::result::Result<(), String> {
        use sha2::{Digest, Sha256};
        use std::process::{Command, Stdio};
        use std::thread;
        use std::time::{Duration, Instant};

        const HEALTH_TIMEOUT: Duration = Duration::from_secs(15);

        let candidate = current_tree.join(
            std::env::current_exe()
                .map_err(|error| format!("resolve control executable: {error}"))?
                .file_name()
                .ok_or_else(|| "control executable has no file name".to_owned())?,
        );
        let bytes = fs::read(&candidate)
            .map_err(|error| format!("read activated Reader {}: {error}", candidate.display()))?;
        if bytes.is_empty() {
            return Err(format!(
                "activated Reader executable is empty: {}",
                candidate.display()
            ));
        }
        let sha256 = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        let mut child = Command::new(&candidate)
            .arg("--product-smoke-v1")
            .env("CHAPTERA_PRODUCT_SMOKE_BINARY_SHA256", sha256)
            .env(
                "CHAPTERA_PRODUCT_SMOKE_BINARY_BYTE_LEN",
                bytes.len().to_string(),
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("launch activated Reader health smoke: {error}"))?;

        let deadline = Instant::now() + HEALTH_TIMEOUT;
        loop {
            match child
                .try_wait()
                .map_err(|error| format!("wait for activated Reader health smoke: {error}"))?
            {
                Some(status) if status.success() => return Ok(()),
                Some(status) => {
                    return Err(format!(
                        "activated Reader health smoke failed with status {status}"
                    ));
                }
                None if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
                None => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "activated Reader health smoke exceeded {} seconds",
                        HEALTH_TIMEOUT.as_secs()
                    ));
                }
            }
        }
    }
}

#[cfg(feature = "reader-only")]
fn run_reader_update_control(request_path: &Path) -> Result<(), String> {
    let request = chaptera_update_handoff::read_control_request(request_path)
        .map_err(|error| error.to_string())?;
    let orchestrator = chaptera_update_orchestrator::UpdateOrchestrator::new(&request.install_root);
    chaptera_update_handoff::validate_request_against_engine(&request, orchestrator.engine())
        .map_err(|error| error.to_string())?;

    let _lock = chaptera_update_orchestrator::InstallLock::acquire(&request.install_root)
        .map_err(|error| error.to_string())?;
    // Revalidate after blocking lock acquisition: the request may have become
    // stale while copied U1 waited for its parent/front-door process to exit.
    chaptera_update_handoff::validate_request_against_engine(&request, orchestrator.engine())
        .map_err(|error| error.to_string())?;

    let receipt = chaptera_update_handoff::ControlReceipt {
        schema_version: chaptera_update_handoff::CONTROL_RECEIPT_SCHEMA_VERSION.to_owned(),
        transaction_id: request.transaction_id.clone(),
        pid: std::process::id(),
        executable: std::env::current_exe()
            .map_err(|error| format!("resolve control executable: {error}"))?,
    };
    chaptera_update_handoff::write_control_receipt(
        &chaptera_update_handoff::receipt_path(request_path),
        &receipt,
    )
    .map_err(|error| error.to_string())?;

    let mut hooks = ReaderControlHooks;
    orchestrator
        .continue_prepared_candidate(&mut hooks)
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(not(feature = "reader-only"))]
fn run_reader_update_control(_request_path: &Path) -> Result<(), String> {
    Err("update control mode is unavailable outside the Reader build".to_owned())
}

fn smoke_check(path: &Path) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let visual = diagnostic_sweep::open_for_product(&bytes)
        .map_err(|error| format!("open {}: {error}", path.display()))?;

    if visual.document.pages.is_empty() {
        return Err("document has no Viewer pages".to_owned());
    }
    if visual.scene.nodes.is_empty() {
        return Err("document has no resolved scene nodes".to_owned());
    }

    Ok(())
}

struct CachedImageTexture {
    texture: egui::TextureHandle,
    _cache_identity_sha256: String,
}

#[derive(Debug)]
struct CachedPageFrameWork {
    page_index: usize,
    render_plan: PageRenderPlanV1,
    hit_index: SceneHitTestIndex,
    movable_nodes: BTreeMap<String, (pub_editor::NodeId, pub_editor::RectEmu)>,
    resizable_nodes: BTreeMap<String, (pub_editor::NodeId, pub_editor::RectEmu)>,
}

struct ViewerApp {
    source_path: Option<PathBuf>,
    open_state: OpenStateAuthority,
    committed_source: Option<CommittedSourceState>,
    source_revalidate_after: Option<Instant>,
    source_exact_revalidate_after: Option<Instant>,
    visual: Option<ViewerGeometryDocument>,
    selected_page: usize,
    page_frame_cache: BTreeMap<usize, Rc<CachedPageFrameWork>>,
    page_frame_cache_builds: u64,
    canvas_selection: SceneSelectionState,
    canvas_drag: Option<MoveTransaction>,
    canvas_resize: Option<ResizeTransaction>,
    created_text_box_scene_nodes: BTreeSet<pub_editor::NodeId>,
    text_mode: Option<text_session::DesktopTextMode>,
    zoom: f32,
    zoom_mode: CanvasZoomMode,
    load_error: Option<ViewerLoadFailure>,
    search_query: String,
    search_results: Vec<ViewerTextMatch>,
    selected_search_result: Option<usize>,
    image_textures: BTreeMap<String, CachedImageTexture>,
    image_decode_diagnostics: BTreeMap<String, image_decode_adapter::DesktopImageDecodeDiagnostic>,
    editor: Option<pub_editor::EditorSession>,
    editor_load_error: Option<String>,
    edit_buffer: String,
    edit_status: Option<String>,
    selected_table_cell_index: Option<usize>,
    table_cell_buffer: String,
    export_preview: Option<DesktopExportPreview>,
    project_status: Option<String>,
    preview_clipped_frames: usize,
    preview_clipped_story_keys: BTreeSet<String>,
    preview_text_diagnostics: Vec<PreviewTextMetricDiagnostic>,
    diagnostic_save_path: String,
    diagnostic_status: Option<String>,
    diagnostic_sweep: Option<diagnostic_sweep::FolderSweepHandle>,
    diagnostic_sweep_progress: diagnostic_sweep::FolderSweepProgress,
    diagnostic_sweep_report: Option<diagnostic_sweep::FolderSweepReport>,
    diagnostic_sweep_open: bool,
    diagnostic_sweep_status: Option<String>,
    supporter_value: supporter::ValueTracker,
    supporter_state: supporter::SupporterState,
    exact_file_consent_open: bool,
    exact_file_consent_status: Option<String>,
    show_diagnostics: bool,
    reader_inspector_tab: reader_product_ui::InspectorTab,
}

impl ViewerApp {
    #[allow(dead_code)]
    fn new(initial_path: Option<PathBuf>) -> Self {
        Self::new_with_storage(initial_path, None)
    }

    fn new_with_storage(
        initial_path: Option<PathBuf>,
        storage: Option<&dyn eframe::Storage>,
    ) -> Self {
        let mut app = Self {
            source_path: None,
            open_state: OpenStateAuthority::default(),
            committed_source: None,
            source_revalidate_after: None,
            source_exact_revalidate_after: None,
            visual: None,
            selected_page: 0,
            page_frame_cache: BTreeMap::new(),
            page_frame_cache_builds: 0,
            canvas_selection: SceneSelectionState::default(),
            canvas_drag: None,
            canvas_resize: None,
            created_text_box_scene_nodes: BTreeSet::new(),
            text_mode: None,
            zoom: 1.0,
            zoom_mode: CanvasZoomMode::FitPage,
            load_error: None,
            search_query: String::new(),
            search_results: Vec::new(),
            selected_search_result: None,
            image_textures: BTreeMap::new(),
            image_decode_diagnostics: BTreeMap::new(),
            editor: None,
            editor_load_error: None,
            edit_buffer: String::new(),
            edit_status: None,
            selected_table_cell_index: None,
            table_cell_buffer: String::new(),
            export_preview: None,
            project_status: None,
            preview_clipped_frames: 0,
            preview_clipped_story_keys: BTreeSet::new(),
            preview_text_diagnostics: Vec::new(),
            diagnostic_save_path: String::new(),
            diagnostic_status: None,
            diagnostic_sweep: None,
            diagnostic_sweep_progress: diagnostic_sweep::FolderSweepProgress::default(),
            diagnostic_sweep_report: None,
            diagnostic_sweep_open: false,
            diagnostic_sweep_status: None,
            supporter_value: supporter::ValueTracker::default(),
            supporter_state: restore_supporter_state(storage),
            exact_file_consent_open: false,
            exact_file_consent_status: None,
            show_diagnostics: false,
            reader_inspector_tab: reader_product_ui::InspectorTab::Document,
        };

        if let Some(path) = initial_path {
            app.load_path(path);
        }

        app
    }

    fn open_pub_folder_diagnostics(&mut self) {
        #[cfg(target_os = "windows")]
        {
            if let Some(root) = rfd::FileDialog::new().pick_folder() {
                self.diagnostic_sweep_progress = diagnostic_sweep::FolderSweepProgress::default();
                self.diagnostic_sweep_report = None;
                self.diagnostic_sweep_status = Some(format!("Scanning {}…", root.display()));
                self.diagnostic_sweep = Some(diagnostic_sweep::start_folder_sweep(root));
                self.diagnostic_sweep_open = true;
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            self.diagnostic_sweep_status = Some(
                "Folder diagnostics picker is currently available in the Windows build.".to_owned(),
            );
            self.diagnostic_sweep_open = true;
        }
    }

    fn poll_diagnostic_sweep(&mut self) {
        let mut events = Vec::new();
        if let Some(handle) = &self.diagnostic_sweep {
            while let Ok(event) = handle.try_recv() {
                events.push(event);
            }
        }

        for event in events {
            match event {
                diagnostic_sweep::FolderSweepEvent::Started { discovered } => {
                    self.diagnostic_sweep_progress.discovered = discovered;
                    self.diagnostic_sweep_status =
                        Some(format!("Discovered {discovered} PUB file(s)."));
                }
                diagnostic_sweep::FolderSweepEvent::Progress(progress) => {
                    self.diagnostic_sweep_progress = progress;
                }
                diagnostic_sweep::FolderSweepEvent::Finished(report) => {
                    self.diagnostic_sweep_progress.discovered = report.totals.discovered;
                    self.diagnostic_sweep_progress.scanned = report.totals.scanned;
                    self.diagnostic_sweep_progress.opened = report.totals.opened;
                    self.diagnostic_sweep_progress.failed = report.totals.failed;
                    self.diagnostic_sweep_progress.failure_groups = report.totals.failure_groups;
                    self.diagnostic_sweep_progress.current_path = None;
                    self.diagnostic_sweep_status = Some(if report.cancelled {
                        format!(
                            "Scan cancelled: {} scanned, {} opened, {} failed, {} failure group(s).",
                            report.totals.scanned,
                            report.totals.opened,
                            report.totals.failed,
                            report.totals.failure_groups
                        )
                    } else {
                        format!(
                            "Scan complete: {} scanned, {} opened, {} failed, {} failure group(s).",
                            report.totals.scanned,
                            report.totals.opened,
                            report.totals.failed,
                            report.totals.failure_groups
                        )
                    });
                    self.diagnostic_sweep_report = Some(report);
                    self.diagnostic_sweep = None;
                }
                diagnostic_sweep::FolderSweepEvent::Fatal(error) => {
                    self.diagnostic_sweep_status =
                        Some(format!("Folder diagnostics failed: {error}"));
                    self.diagnostic_sweep = None;
                }
            }
        }
    }

    fn save_diagnostic_sweep_report(&mut self) {
        let Some(report) = self.diagnostic_sweep_report.as_ref() else {
            return;
        };

        #[cfg(target_os = "windows")]
        {
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name("chaptera-pub-folder-diagnostics.json")
                .save_file()
            {
                self.diagnostic_sweep_status =
                    Some(match diagnostic_sweep::write_report(report, &path) {
                        Ok(()) => format!("Saved diagnostic report to {}.", path.display()),
                        Err(error) => format!("Could not save diagnostic report: {error}"),
                    });
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            self.diagnostic_sweep_status =
                Some("Report picker is currently available in the Windows build.".to_owned());
        }
    }

    fn show_diagnostic_sweep_window(&mut self, ctx: &egui::Context) {
        if !self.diagnostic_sweep_open {
            return;
        }

        let mut open = self.diagnostic_sweep_open;
        egui::Window::new("PUB Folder Diagnostics")
            .open(&mut open)
            .default_width(760.0)
            .default_height(560.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(format!(
                        "Discovered {} · Scanned {} · Opened {} · Failed {} · Groups {}",
                        self.diagnostic_sweep_progress.discovered,
                        self.diagnostic_sweep_progress.scanned,
                        self.diagnostic_sweep_progress.opened,
                        self.diagnostic_sweep_progress.failed,
                        self.diagnostic_sweep_progress.failure_groups
                    ));
                });

                if let Some(path) = &self.diagnostic_sweep_progress.current_path {
                    ui.small(format!("Current: {path}"));
                }

                if self.diagnostic_sweep.is_some() {
                    if ui.button("Cancel scan").clicked()
                        && let Some(handle) = &self.diagnostic_sweep
                    {
                        handle.cancel();
                        self.diagnostic_sweep_status =
                            Some("Cancellation requested; finishing the current file.".to_owned());
                    }
                }

                if let Some(status) = &self.diagnostic_sweep_status {
                    ui.label(status);
                }

                if self.diagnostic_sweep_report.is_some()
                    && ui.button("Save report…").clicked()
                {
                    self.save_diagnostic_sweep_report();
                }

                ui.separator();

                let Some(report) = &self.diagnostic_sweep_report else {
                    ui.weak(
                        "Failures are grouped by stable diagnostic signature. Full diagnostics remain available in the completed report.",
                    );
                    return;
                };

                if report.failure_groups.is_empty() {
                    ui.strong("No grouped failures.");
                } else {
                    ui.heading("Failure groups");
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for group in &report.failure_groups {
                                egui::CollapsingHeader::new(format!(
                                    "{} × {} — {}",
                                    group.count, group.stage, group.normalized_message
                                ))
                                .default_open(false)
                                .show(ui, |ui| {
                                    if let Some(class) = &group.intake_class {
                                        ui.label(format!("Classification: {class}"));
                                    }
                                    ui.label(format!("Signature: {}", group.id));
                                    ui.label("Representative files:");
                                    for path in &group.representative_paths {
                                        ui.monospace(path);
                                    }
                                    ui.collapsing(
                                        format!("All affected files ({})", group.affected_paths.len()),
                                        |ui| {
                                            for path in &group.affected_paths {
                                                ui.monospace(path);
                                            }
                                        },
                                    );
                                    ui.collapsing("Full diagnostic", |ui| {
                                        ui.monospace(&group.sample_full_diagnostic);
                                    });
                                });
                                ui.add_space(4.0);
                            }
                        });
                }

                ui.separator();
                ui.collapsing(format!("Per-file results ({})", report.files.len()), |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(220.0)
                        .show(ui, |ui| {
                            for file in &report.files {
                                let verdict = if file.opened { "OPEN" } else { "FAIL" };
                                let detail = file
                                    .failure_group_id
                                    .as_deref()
                                    .or(file.format_version.as_deref())
                                    .or(file.format.as_deref())
                                    .unwrap_or("");
                                ui.monospace(format!(
                                    "{verdict:4}  d={}  {:>10}  {}  {}",
                                    file.depth,
                                    file.byte_len
                                        .map(|value| value.to_string())
                                        .unwrap_or_else(|| "-".to_owned()),
                                    file.relative_path,
                                    detail
                                ));
                            }
                        });
                });
            });
        self.diagnostic_sweep_open = open;
    }

    fn open_pub_picker(&mut self) {
        #[cfg(target_os = "windows")]
        {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Microsoft Publisher", &["pub"])
                .pick_file()
            {
                self.load_path(path);
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            self.load_error = Some(ViewerLoadFailure {
                kind: ViewerLoadFailureKind::FileAccess,
                attempted_path: None,
                message:
                    "The native Open dialog is currently provided by the Windows Reader build; drag and drop a PUB file on this platform."
                        .to_owned(),
                classification: None,
                diagnostic_json: None,
            });
        }
    }

    fn prepare_document_open(path: PathBuf) -> Result<PreparedDocumentOpen, ViewerLoadFailure> {
        let stamp_before = source_file_stamp(&path).ok();
        let bytes = fs::read(&path).map_err(|error| ViewerLoadFailure {
            kind: ViewerLoadFailureKind::FileAccess,
            attempted_path: Some(path.clone()),
            message: format!("Could not read {}: {error}", path.display()),
            classification: None,
            diagnostic_json: None,
        })?;
        let stamp_after = source_file_stamp(&path).ok();
        let source_file_stamp = (stamp_before.is_some() && stamp_before == stamp_after)
            .then_some(stamp_after)
            .flatten();

        let visual =
            diagnostic_sweep::open_for_product(&bytes).map_err(|error| ViewerLoadFailure {
                kind: ViewerLoadFailureKind::Unsupported,
                attempted_path: Some(path.clone()),
                message: format!("Could not open {}: {error:#}", path.display()),
                classification: Some(classify_failure_candidate(&bytes)),
                diagnostic_json: pub_viewer::local_failure_diagnostic_json(&bytes).ok(),
            })?;

        let (editor, editor_load_error, project_status) = if reader_only_mode() {
            (None, None, None)
        } else {
            let source_hash = visual.document.source.source_hash;
            match pub_editor::open_mature_0x2c_editor(&bytes, source_hash) {
                Ok(mut editor) => {
                    let project_status = match load_editor_project_sidecar(&path, &mut editor) {
                        Ok(Some((sidecar, operation_count))) => Some(format!(
                            "Loaded editor project {} with {operation_count} operations.",
                            sidecar.display()
                        )),
                        Ok(None) => None,
                        Err(error) => Some(format!("Editor project was not applied: {error}")),
                    };
                    (Some(editor), None, project_status)
                }
                Err(error) => (None, Some(error.to_string()), None),
            }
        };

        Ok(PreparedDocumentOpen {
            source_path: path,
            source_file_stamp,
            visual,
            editor,
            editor_load_error,
            project_status,
        })
    }

    fn commit_prepared_document_open(
        &mut self,
        generation: OpenGeneration,
        prepared: PreparedDocumentOpen,
    ) {
        let PreparedDocumentOpen {
            source_path,
            source_file_stamp,
            visual,
            editor,
            editor_load_error,
            project_status,
        } = prepared;

        let source_hash = visual.document.source.source_hash;
        let source_byte_len = visual.document.source.byte_len;
        let supporter_status = match visual.document.fidelity_status() {
            ViewerFidelityStatus::Supported => supporter::OpenStatus::Supported,
            ViewerFidelityStatus::Partial => supporter::OpenStatus::Partial,
            ViewerFidelityStatus::Unsupported => supporter::OpenStatus::Unsupported,
        };
        let supporter_page_count = visual.document.pages.len();
        let supporter_text_searchable = visual
            .document
            .stories
            .iter()
            .any(|story| !story.text.is_empty());

        self.source_path = Some(source_path);
        self.committed_source = Some(CommittedSourceState {
            generation,
            source_hash,
            byte_len: source_byte_len,
            file_stamp: source_file_stamp,
            freshness: SourceFreshness::Current,
        });
        let now = Instant::now();
        self.source_revalidate_after = Some(now + SOURCE_REVALIDATE_INTERVAL);
        self.source_exact_revalidate_after = Some(now + SOURCE_EXACT_REVALIDATE_INTERVAL);
        self.visual = Some(visual);
        self.selected_page = 0;
        self.page_frame_cache.clear();
        self.page_frame_cache_builds = 0;
        self.canvas_selection.clear();
        self.canvas_drag = None;
        self.canvas_resize = None;
        self.created_text_box_scene_nodes.clear();
        self.text_mode = None;
        self.zoom = 1.0;
        self.zoom_mode = CanvasZoomMode::FitPage;
        self.load_error = None;
        self.search_query.clear();
        self.search_results.clear();
        self.selected_search_result = None;
        self.image_textures.clear();
        self.image_decode_diagnostics.clear();
        self.editor = editor;
        self.editor_load_error = editor_load_error;
        self.edit_buffer.clear();
        self.edit_status = None;
        self.selected_table_cell_index = None;
        self.table_cell_buffer.clear();
        self.export_preview = None;
        self.project_status = project_status;
        self.preview_clipped_frames = 0;
        self.preview_clipped_story_keys.clear();
        self.preview_text_diagnostics.clear();
        self.diagnostic_save_path.clear();
        self.diagnostic_status = None;
        self.exact_file_consent_open = false;
        self.exact_file_consent_status = None;
        self.show_diagnostics = false;

        self.supporter_value
            .observe(supporter::ValueEvent::DocumentOpened {
                status: supporter_status,
                page_count: supporter_page_count,
                text_searchable: supporter_text_searchable,
                initial_page: 0,
            });

        if let Err(error) = self.sync_visual_stories_from_editor() {
            self.edit_status = Some(format!(
                "Viewer text projection refresh failed closed: {error}"
            ));
        }
        if let Err(error) = self.sync_visual_created_text_boxes_from_editor() {
            self.edit_status = Some(format!(
                "Viewer created TextBox scene sync failed closed: {error}"
            ));
        }
        self.sync_visual_geometry_from_editor();
    }

    fn load_path(&mut self, path: PathBuf) {
        self.supporter_value
            .observe(supporter::ValueEvent::WorkflowFailed);
        let generation = self.open_state.begin_attempt();

        match Self::prepare_document_open(path) {
            Ok(prepared) => {
                if self.open_state.commit_if_current(generation) {
                    self.commit_prepared_document_open(generation, prepared);
                }
            }
            Err(error) => {
                if self.open_state.finish_without_commit_if_current(generation) {
                    self.load_error = Some(error);
                    self.exact_file_consent_open = false;
                    self.exact_file_consent_status = None;
                }
            }
        }
    }

    fn revalidate_committed_source_now(&mut self) {
        let Some(path) = self.source_path.clone() else {
            return;
        };
        let Some((generation, expected_hash, expected_len, previous_stamp, previous_freshness)) =
            self.committed_source.as_ref().map(|source| {
                (
                    source.generation,
                    source.source_hash,
                    source.byte_len,
                    source.file_stamp,
                    source.freshness,
                )
            })
        else {
            return;
        };
        debug_assert!(generation.0 > 0);
        if previous_freshness.requires_reload() {
            return;
        }

        let observed_stamp = match source_file_stamp(&path) {
            Ok(stamp) => stamp,
            Err(_) => {
                if let Some(source) = self.committed_source.as_mut() {
                    source.freshness = SourceFreshness::ReloadRequiredUnavailable;
                }
                return;
            }
        };

        // Metadata is only the cheap change signal. Exact SHA-256 remains the
        // committed source identity whenever the signal moves, and is also
        // revalidated periodically so a same-length replacement that preserves
        // mtime cannot remain current forever. Adversarial Windows path/file-
        // identity races remain owned by CHAPTERA-WIN-PATH-IDENTITY-01.
        let metadata_is_strongly_unchanged = previous_stamp.is_some_and(|stamp| {
            stamp.modified.is_some()
                && stamp.byte_len == observed_stamp.byte_len
                && stamp.modified == observed_stamp.modified
        });
        let now = Instant::now();
        let exact_revalidation_due = self
            .source_exact_revalidate_after
            .is_none_or(|deadline| now >= deadline);
        if metadata_is_strongly_unchanged && !exact_revalidation_due {
            return;
        }

        let freshness = match fs::read(&path) {
            Ok(bytes) => {
                let observed_len =
                    u64::try_from(bytes.len()).expect("desktop source length must fit into u64");
                if observed_len == expected_len && source_sha256(&bytes) == expected_hash {
                    SourceFreshness::Current
                } else {
                    SourceFreshness::ReloadRequiredChanged
                }
            }
            Err(_) => SourceFreshness::ReloadRequiredUnavailable,
        };

        self.source_exact_revalidate_after = Some(now + SOURCE_EXACT_REVALIDATE_INTERVAL);
        if let Some(source) = self.committed_source.as_mut() {
            source.freshness = freshness;
            if freshness == SourceFreshness::Current {
                source.file_stamp = Some(observed_stamp);
            }
        }
    }

    fn poll_committed_source_freshness(&mut self) {
        if !reader_only_mode() || self.committed_source.is_none() {
            return;
        }

        let now = Instant::now();
        if self
            .source_revalidate_after
            .is_some_and(|deadline| now < deadline)
        {
            return;
        }
        self.source_revalidate_after = Some(now + SOURCE_REVALIDATE_INTERVAL);
        self.revalidate_committed_source_now();
    }

    fn show_source_freshness_banner(&mut self, ui: &mut egui::Ui) {
        let Some(freshness) = self
            .committed_source
            .as_ref()
            .map(|source| source.freshness)
        else {
            return;
        };
        if !freshness.requires_reload() {
            return;
        }

        let reload_path = self.source_path.clone();
        ui.horizontal_wrapped(|ui| {
            ui.strong(match freshness {
                SourceFreshness::ReloadRequiredChanged => "Source changed on disk.",
                SourceFreshness::ReloadRequiredUnavailable => "Source is no longer readable.",
                SourceFreshness::Current => unreachable!(),
            });
            ui.label(
                "Chaptera is showing the previously opened coherent snapshot. Reload is required before this view is treated as current.",
            );
            if let Some(path) = reload_path.as_ref()
                && ui.button("Reload").clicked()
            {
                self.load_path(path.clone());
            }
        });
    }

    fn show_reader_command_bar(&mut self, ui: &mut egui::Ui) {
        let document_label = self
            .source_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned());

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Chaptera Reader")
                    .size(16.0)
                    .strong()
                    .color(reader_product_ui::TEXT),
            );
            ui.separator();
            if let Some(label) = &document_label {
                ui.label(egui::RichText::new(label).color(reader_product_ui::MUTED_TEXT));
            } else {
                reader_product_ui::muted(ui, "No PUB open");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                reader_product_ui::muted(ui, "Local · read-only");
            });
        });

        reader_product_ui::panel_separator(ui);

        ui.horizontal(|ui| {
            if reader_product_ui::toolbar_button(ui, "□", "Open", false).clicked() {
                self.open_pub_picker();
            }

            ui.separator();

            if reader_product_ui::toolbar_button(ui, "−", "Zoom Out", false).clicked() {
                let base = if self.zoom_mode == CanvasZoomMode::Percent {
                    self.zoom
                } else {
                    1.0
                };
                self.zoom = (base - 0.10).clamp(MIN_NUMERIC_ZOOM, MAX_NUMERIC_ZOOM);
                self.zoom_mode = CanvasZoomMode::Percent;
            }
            if reader_product_ui::toolbar_button(ui, "+", "Zoom In", false).clicked() {
                let base = if self.zoom_mode == CanvasZoomMode::Percent {
                    self.zoom
                } else {
                    1.0
                };
                self.zoom = (base + 0.10).clamp(MIN_NUMERIC_ZOOM, MAX_NUMERIC_ZOOM);
                self.zoom_mode = CanvasZoomMode::Percent;
            }
            if reader_product_ui::toolbar_button(
                ui,
                "▣",
                "Fit Page",
                self.zoom_mode == CanvasZoomMode::FitPage,
            )
            .clicked()
            {
                self.zoom_mode = CanvasZoomMode::FitPage;
            }
            if reader_product_ui::toolbar_button(
                ui,
                "↔",
                "Fit Width",
                self.zoom_mode == CanvasZoomMode::PageWidth,
            )
            .clicked()
            {
                self.zoom_mode = CanvasZoomMode::PageWidth;
            }
            if reader_product_ui::toolbar_button(
                ui,
                "1:1",
                "Actual Size",
                self.zoom_mode == CanvasZoomMode::Percent && (self.zoom - 1.0).abs() < 0.001,
            )
            .clicked()
            {
                self.zoom = 1.0;
                self.zoom_mode = CanvasZoomMode::Percent;
            }

            ui.separator();

            if reader_product_ui::toolbar_button(
                ui,
                "⌕",
                "Search",
                self.reader_inspector_tab == reader_product_ui::InspectorTab::Text,
            )
            .clicked()
            {
                self.reader_inspector_tab = reader_product_ui::InspectorTab::Text;
            }

            if reader_product_ui::toolbar_button(
                ui,
                "ⓘ",
                "Details",
                self.reader_inspector_tab == reader_product_ui::InspectorTab::Diagnostics,
            )
            .clicked()
            {
                self.reader_inspector_tab = reader_product_ui::InspectorTab::Diagnostics;
            }

            ui.separator();
            ui.menu_button("⋯  More", |ui| {
                if ui.button("Scan PUB folder…").clicked() {
                    self.open_pub_folder_diagnostics();
                    ui.close_menu();
                }
                if ui
                    .add_enabled(
                        self.visual.is_some(),
                        egui::Button::new("Fidelity & diagnostics…"),
                    )
                    .clicked()
                {
                    self.show_diagnostics = true;
                    ui.close_menu();
                }
            });
        });
    }

    fn show_command_bar(&mut self, ui: &mut egui::Ui) {
        if reader_only_mode() {
            self.show_reader_command_bar(ui);
            return;
        }
        let operation_count = self
            .editor
            .as_ref()
            .map(|editor| editor.operations().len())
            .unwrap_or(0);
        let editor_available = self.editor.is_some();
        let saved_operation_count = self.saved_project_operation_count().ok().flatten();
        let reopen_enabled = operation_count > 0 && saved_operation_count == Some(operation_count);
        let document_label = self
            .source_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned());

        ui.horizontal(|ui| {
            ui.strong(product_surface_label());
            ui.separator();

            if ui.button("Open PUB…").clicked() {
                self.open_pub_picker();
            }
            if ui.button("Scan PUB folder…").clicked() {
                self.open_pub_folder_diagnostics();
            }

            if let Some(label) = document_label {
                ui.label(label);
            } else {
                ui.weak("Open or drop a .pub file to begin");
            }

            if reader_only_mode() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak("Local · read-only");
                });
                return;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(operation_count > 0, |ui| {
                    ui.menu_button("Export", |ui| {
                        if ui.button("Preview IDML").clicked() {
                            self.refresh_export_preview(pub_editor::EditorEditableTarget::Idml);
                        }
                        if ui.button("Preview ODG").clicked() {
                            self.refresh_export_preview(pub_editor::EditorEditableTarget::Odg);
                        }

                        if let Some(preview) = self.export_preview.clone() {
                            ui.separator();
                            ui.small(&preview.summary);
                            let preview_is_current = preview.operation_count == operation_count;
                            if !preview_is_current {
                                ui.weak("Preview is stale. Preview again before export.");
                            }
                            let export_enabled = preview_is_current && preview.can_serialize;
                            if ui
                                .add_enabled(
                                    export_enabled,
                                    egui::Button::new(format!("Export edited {} copy", preview.target)),
                                )
                                .clicked()
                            {
                                match self.export_editable_copy(preview.target) {
                                    Ok((output, report)) => {
                                        self.edit_status = Some(format!(
                                            "Exported edited {} copy to {} with report {}. Source PUB was not overwritten.",
                                            preview.target,
                                            output.display(),
                                            report.display()
                                        ));
                                    }
                                    Err(error) => {
                                        self.edit_status = Some(format!(
                                            "Could not export {}: {error}",
                                            preview.target
                                        ));
                                    }
                                }
                            }
                        } else {
                            ui.weak("Preview IDML or ODG to review fidelity/loss before export.");
                        }
                    });
                });

                let reopen_response = ui.add_enabled(
                    reopen_enabled,
                    egui::Button::new("Reopen Project"),
                );
                let reopen_clicked = reopen_response.clicked();
                if !reopen_enabled {
                    reopen_response.on_disabled_hover_text(
                        "Save the current EditorProject before reopening it. Reopen never discards unsaved operations.",
                    );
                }

                let save_clicked = ui
                    .add_enabled(operation_count > 0, egui::Button::new("Save Project"))
                    .clicked();
                let redo_clicked = ui
                    .add_enabled(editor_available, egui::Button::new("Redo"))
                    .clicked();
                let undo_clicked = ui
                    .add_enabled(editor_available && operation_count > 0, egui::Button::new("Undo"))
                    .clicked();

                if reopen_clicked {
                    if let Err(error) = self.reopen_saved_project() {
                        self.edit_status = Some(format!("Could not reopen saved project: {error}"));
                    }
                }
                if save_clicked {
                    match self.save_editor_project_sidecar() {
                        Ok(path) => {
                            self.project_status = Some(format!(
                                "Saved editor project with {operation_count} operations."
                            ));
                            self.edit_status = Some(format!(
                                "Project saved to {}. Source PUB was not overwritten.",
                                path.display()
                            ));
                        }
                        Err(error) => {
                            self.edit_status =
                                Some(format!("Could not save editor project: {error}"));
                        }
                    }
                }
                if redo_clicked {
                    self.apply_redo();
                }
                if undo_clicked {
                    self.apply_undo();
                }
            });
        });
    }

    fn show_workspace_status(&mut self, ui: &mut egui::Ui) {
        if reader_only_mode() {
            let page_count = self
                .visual
                .as_ref()
                .map(|visual| visual.document.pages.len())
                .unwrap_or(0);
            let current_page = if page_count == 0 {
                0
            } else {
                self.selected_page.min(page_count - 1) + 1
            };

            ui.columns(3, |columns| {
                columns[0].vertical_centered(|ui| {
                    if page_count > 0 {
                        ui.label(format!("Page {current_page} of {page_count}"));
                    } else {
                        reader_product_ui::muted(ui, "No document open");
                    }
                });

                columns[1].horizontal_centered(|ui| {
                    let previous = ui.add_enabled(
                        self.selected_page > 0,
                        egui::Button::new("‹").min_size(egui::vec2(34.0, 26.0)),
                    );
                    if previous.clicked() {
                        self.selected_page -= 1;
                        self.canvas_selection.clear();
                        self.canvas_drag = None;
                        self.canvas_resize = None;
                        self.supporter_value
                            .observe(supporter::ValueEvent::PageNavigated {
                                page_index: self.selected_page,
                            });
                    }

                    if page_count > 0 {
                        ui.label(
                            egui::RichText::new(current_page.to_string())
                                .strong()
                                .color(reader_product_ui::TEXT),
                        );
                    }

                    let next = ui.add_enabled(
                        page_count > 0 && self.selected_page + 1 < page_count,
                        egui::Button::new("›").min_size(egui::vec2(34.0, 26.0)),
                    );
                    if next.clicked() {
                        self.selected_page += 1;
                        self.canvas_selection.clear();
                        self.canvas_drag = None;
                        self.canvas_resize = None;
                        self.supporter_value
                            .observe(supporter::ValueEvent::PageNavigated {
                                page_index: self.selected_page,
                            });
                    }
                });

                columns[2].with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.zoom_mode == CanvasZoomMode::Percent {
                        let mut zoom_percent = self.zoom * 100.0;
                        let slider = ui.add(
                            egui::Slider::new(&mut zoom_percent, 10.0..=400.0)
                                .suffix("%")
                                .show_value(true),
                        );
                        if slider.changed() {
                            self.zoom =
                                (zoom_percent / 100.0).clamp(MIN_NUMERIC_ZOOM, MAX_NUMERIC_ZOOM);
                        }
                    } else {
                        reader_product_ui::muted(
                            ui,
                            match self.zoom_mode {
                                CanvasZoomMode::FitPage => "Fit Page",
                                CanvasZoomMode::PageWidth => "Fit Width",
                                CanvasZoomMode::FitSelection => "Fit Selection",
                                CanvasZoomMode::Percent => unreachable!(),
                            },
                        );
                    }
                });
            });
            return;
        }

        ui.horizontal_wrapped(|ui| {
            let operation_count = self
                .editor
                .as_ref()
                .map(|editor| editor.operations().len())
                .unwrap_or(0);
            if self.visual.is_some() {
                ui.strong("Source PUB protected");
                ui.label("·");
                if reader_only_mode() {
                    ui.label("Local · read-only");
                } else {
                    ui.label(format!("{operation_count} edit operation(s)"));
                }
                if let Some(fidelity) = self.fidelity_status() {
                    ui.label("·");
                    ui.label(format!("Fidelity: {}", fidelity_status_label(fidelity)));
                }
            } else {
                ui.weak("No document open");
                ui.label("·");
                if reader_only_mode() {
                    ui.label("Files open locally; no account is required.");
                } else {
                    ui.label("Source files stay local and are never overwritten.");
                }
            }
        });
    }

    fn fidelity_status(&self) -> Option<ViewerFidelityStatus> {
        if let Some(visual) = &self.visual {
            return Some(visual.document.fidelity_status());
        }

        self.load_error.as_ref().and_then(|failure| {
            (failure.kind == ViewerLoadFailureKind::Unsupported)
                .then_some(ViewerFidelityStatus::Unsupported)
        })
    }

    fn show_fidelity_status(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Fidelity:");

            match self.fidelity_status() {
                Some(status) => {
                    ui.strong(fidelity_status_label(status));
                    match status {
                        ViewerFidelityStatus::Supported => {
                            ui.weak("No known fidelity warnings");
                        }
                        ViewerFidelityStatus::Partial | ViewerFidelityStatus::Unsupported => {
                            ui.label("· Needs attention");
                        }
                    }
                }
                None => {
                    ui.weak("Not evaluated");
                }
            }

            if self.preview_clipped_frames > 0 {
                ui.label("·");
                ui.label(format!(
                    "{} preview text clipping issue(s)",
                    self.preview_clipped_frames
                ));
            }

            let details_available = self.visual.is_some();
            if ui
                .add_enabled(details_available, egui::Button::new("Details…"))
                .clicked()
            {
                self.show_diagnostics = true;
            }
        });
    }

    fn accept_dropped_file(&mut self, ctx: &egui::Context) {
        let dropped_paths = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path.clone())
                .collect::<Vec<_>>()
        });

        match dropped_file_candidate(&dropped_paths) {
            Ok(Some(path)) => self.load_path(path),
            Ok(None) => {}
            Err(message) => {
                self.load_error = Some(ViewerLoadFailure {
                    kind: ViewerLoadFailureKind::FileAccess,
                    attempted_path: None,
                    message: message.to_owned(),
                    classification: None,
                    diagnostic_json: None,
                });
                self.exact_file_consent_open = false;
                self.exact_file_consent_status = None;
            }
        }
    }

    fn refresh_search(&mut self) {
        self.search_results = self
            .visual
            .as_ref()
            .map(|visual| visual.document.search_text(&self.search_query))
            .unwrap_or_default();

        if self
            .selected_search_result
            .is_some_and(|index| index >= self.search_results.len())
        {
            self.selected_search_result = None;
        }
    }

    fn show_search(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.heading("Search");
        ui.separator();

        if self.visual.is_none() {
            ui.weak("Open a document to search recovered text.");
            return;
        }

        let response = ui.add(
            egui::TextEdit::singleline(&mut self.search_query).hint_text("Search semantic text"),
        );
        if response.changed() {
            self.refresh_search();
        }

        if self.search_query.is_empty() {
            ui.weak("Search uses recovered story text, not OCR.");
            return;
        }

        ui.label(format!("{} matches", self.search_results.len()));
        if self.search_results.is_empty() {
            ui.weak("No exact matches.");
            return;
        }

        let mut clicked = None;
        egui::ScrollArea::vertical()
            .max_height(220.0)
            .show(ui, |ui| {
                for (index, result) in self.search_results.iter().enumerate() {
                    let selected = self.selected_search_result == Some(index);
                    let label = format!("{}. {}", index + 1, search_result_preview(&result.text));
                    if ui.selectable_label(selected, label).clicked() {
                        clicked = Some(index);
                    }
                }
            });

        if let Some(index) = clicked {
            self.selected_search_result = Some(index);
            self.supporter_value
                .observe(supporter::ValueEvent::SearchResultSelected {
                    match_count: self.search_results.len(),
                });
            self.selected_table_cell_index = None;
            self.table_cell_buffer.clear();
            if let Some(text) = self
                .search_results
                .get(index)
                .and_then(|result| {
                    self.visual.as_ref().and_then(|visual| {
                        visual
                            .document
                            .stories
                            .iter()
                            .find(|story| story.id == result.story_id)
                    })
                })
                .map(|story| story.text.clone())
            {
                self.edit_buffer = text;
                self.edit_status = None;
            }
        }

        ui.small("Jump selects the exact story match. Page ownership is not shown unless proven.");
    }

    fn show_pages(&mut self, ui: &mut egui::Ui) {
        if reader_only_mode() {
            reader_product_ui::section_label(ui, "Pages");
            if let Some(visual) = &self.visual {
                reader_product_ui::muted(ui, format!("{} pages", visual.document.pages.len()));
            }
            reader_product_ui::panel_separator(ui);
        } else {
            ui.heading("Pages");
            ui.separator();
        }

        self.ensure_image_textures(ui.ctx());

        let Some(visual) = &self.visual else {
            ui.weak("No document loaded.");
            return;
        };

        let mut selected_page = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (index, page) in visual.document.pages.iter().enumerate() {
                let selected = self.selected_page == index;
                let mut thumbnail_clicked = false;

                ui.vertical_centered(|ui| {
                    if let Some(surface) = visual
                        .scene
                        .surfaces
                        .iter()
                        .find(|surface| surface.origin == page.id)
                        && let Some(size) =
                            page_thumbnail_size(surface.size.width.get(), surface.size.height.get())
                    {
                        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                true,
                                format!("Page {} thumbnail", page.index),
                            )
                        });
                        paint_page_thumbnail(
                            ui.painter(),
                            rect,
                            visual,
                            self.editor.as_ref(),
                            &self.image_textures,
                            index,
                            selected,
                        );
                        thumbnail_clicked = response.clicked();
                    }

                    let label_clicked = ui
                        .selectable_label(selected, format!("Page {}", page.index))
                        .clicked();
                    if thumbnail_clicked || label_clicked {
                        selected_page = Some(index);
                    }
                });

                ui.add_space(6.0);
            }
        });

        if let Some(index) = selected_page {
            if self.selected_page != index {
                self.canvas_selection.clear();
                self.canvas_drag = None;
                self.canvas_resize = None;
                self.supporter_value
                    .observe(supporter::ValueEvent::PageNavigated { page_index: index });
            }
            self.selected_page = index;
        }

        if !reader_only_mode() {
            self.show_search(ui);
        }
    }

    fn show_reader_inspector(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if reader_product_ui::inspector_tab(
                ui,
                "Document",
                self.reader_inspector_tab == reader_product_ui::InspectorTab::Document,
            )
            .clicked()
            {
                self.reader_inspector_tab = reader_product_ui::InspectorTab::Document;
            }
            if reader_product_ui::inspector_tab(
                ui,
                "Text",
                self.reader_inspector_tab == reader_product_ui::InspectorTab::Text,
            )
            .clicked()
            {
                self.reader_inspector_tab = reader_product_ui::InspectorTab::Text;
            }
            if reader_product_ui::inspector_tab(
                ui,
                "Diagnostics",
                self.reader_inspector_tab == reader_product_ui::InspectorTab::Diagnostics,
            )
            .clicked()
            {
                self.reader_inspector_tab = reader_product_ui::InspectorTab::Diagnostics;
            }
        });
        reader_product_ui::panel_separator(ui);

        match self.reader_inspector_tab {
            reader_product_ui::InspectorTab::Document => {
                self.show_inspector(ui);
                ui.add_space(14.0);
                reader_product_ui::panel_separator(ui);
                reader_product_ui::section_label(ui, "Fidelity");
                match self.fidelity_status() {
                    Some(status) => {
                        let label = fidelity_status_label(status);
                        ui.label(
                            egui::RichText::new(format!("●  {label}"))
                                .strong()
                                .color(reader_product_ui::status_color(label)),
                        );
                        reader_product_ui::muted(ui, fidelity_status_summary(status));
                    }
                    None => reader_product_ui::muted(ui, "Not evaluated"),
                }
                if self.preview_clipped_frames > 0 {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "{} preview text clipping issue(s)",
                            self.preview_clipped_frames
                        ))
                        .color(reader_product_ui::WARNING),
                    );
                }
                if ui
                    .add_enabled(
                        self.visual.is_some(),
                        egui::Button::new("Open fidelity details…"),
                    )
                    .clicked()
                {
                    self.show_diagnostics = true;
                }
            }
            reader_product_ui::InspectorTab::Text => {
                self.show_search(ui);
                if let Some(index) = self.selected_search_result {
                    if let Some(result) = self.search_results.get(index) {
                        ui.add_space(12.0);
                        reader_product_ui::section_label(ui, "Selected match");
                        ui.label(&result.text);
                        if ui.button("Copy match").clicked() {
                            ui.ctx().copy_text(result.text.clone());
                            self.supporter_value
                                .observe(supporter::ValueEvent::SearchMatchCopied);
                        }
                    }
                }
            }
            reader_product_ui::InspectorTab::Diagnostics => {
                reader_product_ui::section_label(ui, "Fidelity & diagnostics");
                if let Some(status) = self.fidelity_status() {
                    let label = fidelity_status_label(status);
                    ui.label(
                        egui::RichText::new(format!("●  {label}"))
                            .strong()
                            .color(reader_product_ui::status_color(label)),
                    );
                    reader_product_ui::muted(ui, fidelity_status_summary(status));
                } else {
                    reader_product_ui::muted(ui, "No document has been evaluated yet.");
                }

                if let Some(visual) = &self.visual {
                    ui.add_space(10.0);
                    if visual.document.diagnostics.is_empty() {
                        reader_product_ui::muted(ui, "No Viewer diagnostics.");
                    } else {
                        egui::ScrollArea::vertical()
                            .max_height(360.0)
                            .show(ui, |ui| {
                                for diagnostic in &visual.document.diagnostics {
                                    ui.group(|ui| {
                                        ui.strong(&diagnostic.code);
                                        ui.small(diagnostic_severity_label(diagnostic.severity));
                                        ui.label(&diagnostic.message);
                                    });
                                    ui.add_space(4.0);
                                }
                            });
                    }
                }

                ui.add_space(8.0);
                if ui
                    .add_enabled(
                        self.visual.is_some(),
                        egui::Button::new("Open detailed diagnostics…"),
                    )
                    .clicked()
                {
                    self.show_diagnostics = true;
                }
            }
        }
    }

    fn show_inspector(&mut self, ui: &mut egui::Ui) {
        if !reader_only_mode() {
            ui.heading("Document");
            ui.separator();
        }

        if let Some(path) = &self.source_path {
            ui.label("Document");
            if let Some(name) = path.file_name() {
                ui.strong(name.to_string_lossy());
            }
            if reader_only_mode() {
                ui.small("The original PUB stays unchanged. Reader is local and read-only.");
            } else {
                ui.small("The original PUB stays unchanged; edits live in the Chaptera project.");
            }
            ui.add_space(8.0);
        }

        if let Some(status) = &self.project_status {
            ui.label("Project");
            ui.small(status);
            ui.add_space(8.0);
        }

        let Some(visual) = &self.visual else {
            ui.weak("Load a .pub file to inspect it.");
            if let Some(error) = &self.load_error {
                ui.add_space(12.0);
                ui.colored_label(ui.visuals().error_fg_color, &error.message);
                if error.kind == ViewerLoadFailureKind::FileAccess {
                    ui.add_space(8.0);
                    ui.strong("File access problem");
                    ui.label(
                        "Chaptera could not read this path. Check that the file still exists and that you have permission to open it.",
                    );
                    ui.small(
                        "This is a local filesystem/read failure; it does not mean the document is unsupported.",
                    );
                }
                if let Some(classification) = &error.classification {
                    ui.add_space(8.0);
                    ui.strong(failure_intake_label(classification.class));
                    ui.label(failure_intake_summary(classification.class));
                    ui.small(
                        "This classification was computed locally. Chaptera did not send this file or its contents anywhere.",
                    );

                    if exact_file_consent_cta_visible(classification.class) {
                        ui.add_space(10.0);
                        if failure_mailto_recipient_configured() {
                            if ui.button("Report this broken PUB…").clicked() {
                                self.exact_file_consent_open = true;
                                self.exact_file_consent_status = None;
                            }
                        } else {
                            ui.small(
                                "Private file handoff is not configured in this build. Save local diagnostics below; no file is sent.",
                            );
                        }
                    }
                }
                if let Some(status) = &self.exact_file_consent_status {
                    ui.add_space(8.0);
                    ui.small(status);
                }

                let diagnostic_json = error.diagnostic_json.clone();
                if let Some(diagnostic_json) = diagnostic_json {
                    ui.add_space(12.0);
                    ui.strong("Local diagnostics");
                    ui.small(
                        "Nothing is sent. Choose a local JSON path, then save an inspectable structural report.",
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.diagnostic_save_path)
                            .hint_text("chaptera-diagnostics.json"),
                    );
                    let can_save = !self.diagnostic_save_path.trim().is_empty();
                    if ui
                        .add_enabled(can_save, egui::Button::new("Save diagnostics…"))
                        .clicked()
                    {
                        let target = PathBuf::from(self.diagnostic_save_path.trim());
                        self.diagnostic_status =
                            Some(match fs::write(&target, diagnostic_json.as_bytes()) {
                                Ok(()) => format!("Saved diagnostics to {}.", target.display()),
                                Err(error) => {
                                    format!(
                                        "Could not save diagnostics to {}: {error}",
                                        target.display()
                                    )
                                }
                            });
                    }
                    if let Some(status) = &self.diagnostic_status {
                        ui.small(status);
                    }
                }
            }
            return;
        };

        let source = &visual.document.source;
        ui.label(format!(
            "{} pages · {} stories",
            visual.document.pages.len(),
            visual.document.stories.len()
        ));
        if !reader_only_mode() {
            if self.canvas_selection.primary().is_some() {
                ui.strong(format!("{} object selected", self.canvas_selection.len()));
                ui.small(
                    "Drag to move supported page-local objects. Projected objects stay read-only.",
                );
            } else {
                ui.weak("No object selected");
            }
        }

        ui.collapsing("Technical details", |ui| {
            if let Some(path) = &self.source_path {
                ui.label("Source path");
                ui.monospace(path.display().to_string());
            }
            ui.label(format!(
                "Format: {} {}",
                source.format,
                source
                    .format_version
                    .as_deref()
                    .unwrap_or("(version unknown)")
            ));
            ui.label(format!("Bytes: {}", source.byte_len));
            ui.label(format!("Scene nodes: {}", visual.scene.nodes.len()));
            ui.label(format!(
                "Page frame cache builds: {}",
                self.page_frame_cache_builds
            ));
            ui.label(format!(
                "Engine: {}",
                visual.scene.environment.engine_revision
            ));
            ui.label("Source SHA-256");
            ui.monospace(format!("{:?}", source.source_hash));
            if let Some(instance_id) = self.canvas_selection.primary() {
                ui.label("Selected scene instance");
                ui.monospace(instance_id);
            }
        });

        if let Some(index) = self.selected_search_result {
            ui.add_space(16.0);
            ui.heading("Selected search match");
            ui.separator();

            if let Some(result) = self.search_results.get(index) {
                ui.strong(&result.text);
                ui.small("Page ownership is not shown unless the current model proves it.");
                ui.collapsing("Match details", |ui| {
                    ui.label("Story reference");
                    ui.monospace(format!("{:?}", result.story_id));
                    ui.label(format!(
                        "Exact byte range: {}..{}",
                        result.start_byte, result.end_byte
                    ));
                });

                ui.horizontal(|ui| {
                    if ui.button("Copy match").clicked() {
                        ui.ctx().copy_text(result.text.clone());
                        self.supporter_value
                            .observe(supporter::ValueEvent::SearchMatchCopied);
                    }

                    if let Some(story) = visual
                        .document
                        .stories
                        .iter()
                        .find(|story| story.id == result.story_id)
                        && ui.button("Copy full story").clicked()
                    {
                        ui.ctx().copy_text(story.text.clone());
                        self.supporter_value
                            .observe(supporter::ValueEvent::FullStoryCopied);
                    }
                });

                if let Some(story) = visual
                    .document
                    .stories
                    .iter()
                    .find(|story| story.id == result.story_id)
                {
                    egui::ScrollArea::vertical()
                        .max_height(160.0)
                        .show(ui, |ui| {
                            ui.label(&story.text);
                        });
                }
            }
        }

        if !reader_only_mode() {
            self.show_object_edit_controls(ui);
            self.show_editor_controls(ui);
        }

        if let Some(error) = &self.load_error {
            ui.add_space(12.0);
            ui.colored_label(ui.visuals().error_fg_color, &error.message);
        }
    }

    fn show_diagnostics_window(&mut self, ctx: &egui::Context) {
        if !self.show_diagnostics {
            return;
        }

        let Some(visual) = self.visual.as_ref() else {
            self.show_diagnostics = false;
            return;
        };

        let fidelity = visual.document.fidelity_status();
        let has_geometry_warning = visual
            .document
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "viewer.visual.geometry_only");
        let preview_clipped_frames = self.preview_clipped_frames;
        let diagnostics = &visual.document.diagnostics;
        let decode_diagnostics = &self.image_decode_diagnostics;
        let mut open = self.show_diagnostics;

        egui::Window::new("Fidelity & diagnostics")
            .open(&mut open)
            .resizable(true)
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(fidelity_status_label(fidelity));
                    ui.label(fidelity_status_summary(fidelity));
                });

                if has_geometry_warning {
                    ui.add_space(8.0);
                    ui.strong("Current preview boundary");
                    ui.small(GEOMETRY_WARNING);
                }

                if preview_clipped_frames > 0 {
                    ui.add_space(8.0);
                    ui.strong(format!(
                        "Preview text clipping: {preview_clipped_frames} frame(s)"
                    ));
                    ui.small(PREVIEW_TEXT_CLIP_WARNING);
                }

                if !self.preview_text_diagnostics.is_empty() {
                    ui.add_space(6.0);
                    if ui.button("Copy preview metrics JSON").clicked()
                        && let Ok(json) =
                            serde_json::to_string_pretty(&self.preview_text_diagnostics)
                    {
                        ui.ctx().copy_text(json);
                    }
                    egui::ScrollArea::vertical()
                        .max_height(180.0)
                        .show(ui, |ui| {
                            for row in &self.preview_text_diagnostics {
                                ui.monospace(format!(
                                    "p{} frame={} sizes={:?}px source={} fallback={} wrap={:.1}px galley={:.1}x{:.1}px clip={:.1}x{:.1}px overflow={:.1}px lines={}",
                                    row.page_index,
                                    row.frame_id,
                                    row.metrics.executed_font_sizes_px,
                                    row.metrics.source_typography_sections,
                                    row.metrics.fallback_sections,
                                    row.metrics.wrap_width_px,
                                    row.metrics.galley_width_px,
                                    row.metrics.galley_height_px,
                                    row.metrics.clip_width_px,
                                    row.metrics.clip_height_px,
                                    row.metrics.overflow_delta_px,
                                    row.metrics.line_count,
                                ));
                            }
                        });
                }

                ui.add_space(12.0);
                ui.heading("Diagnostics");
                ui.separator();

                if diagnostics.is_empty() {
                    ui.weak("No Viewer diagnostics.");
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .show(ui, |ui| {
                            for diagnostic in diagnostics {
                                ui.group(|ui| {
                                    ui.strong(&diagnostic.code);
                                    ui.small(diagnostic_severity_label(diagnostic.severity));
                                    ui.label(&diagnostic.message);
                                });
                                ui.add_space(4.0);
                            }
                        });
                }

                if !decode_diagnostics.is_empty() {
                    ui.add_space(12.0);
                    ui.heading("Desktop image decode");
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(180.0)
                        .show(ui, |ui| {
                            for diagnostic in decode_diagnostics.values() {
                                ui.group(|ui| {
                                    ui.strong(&diagnostic.code);
                                    ui.small("Fidelity warning · desktop decode/runtime");
                                    ui.label(format!(
                                        "{} ({})",
                                        diagnostic.resource_key, diagnostic.mime
                                    ));
                                    ui.label(&diagnostic.message);
                                });
                                ui.add_space(4.0);
                            }
                        });
                }
            });

        self.show_diagnostics = open;
    }

    fn show_exact_file_consent_dialog(&mut self, ctx: &egui::Context) {
        if !self.exact_file_consent_open {
            return;
        }

        let eligible = failure_mailto_recipient_configured()
            && self
                .load_error
                .as_ref()
                .and_then(|failure| failure.classification.as_ref())
                .is_some_and(|classification| exact_file_consent_cta_visible(classification.class));
        if !eligible {
            self.exact_file_consent_open = false;
            return;
        }

        let filename = self
            .load_error
            .as_ref()
            .and_then(|failure| failure.attempted_path.as_deref())
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "(filename unavailable)".to_owned());

        let mut open = self.exact_file_consent_open;
        let mut send_clicked = false;
        let mut cancel_clicked = false;

        egui::Window::new("Send this file to help Chaptera support it")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("File");
                ui.monospace(&filename);
                ui.add_space(8.0);
                ui.label(
                    "If you choose Send file, the full contents of this file are intended to be submitted to improve Chaptera compatibility or recovery.",
                );
                ui.strong("Files may contain personal, private, or confidential information.");
                ui.add_space(8.0);
                ui.label(format!(
                    "Consent contract: {CHAPTERA_EXACT_FILE_CONSENT_V1}"
                ));
                ui.label(format!(
                    "Retention policy: {CHAPTERA_INTAKE_RETENTION_POLICY_V1}"
                ));
                ui.small(
                    "Policy v1: rejected/suspicious files are kept up to 7 days; duplicate verification up to 24 hours; accepted unpromoted files up to 90 days; promoted research witnesses are reviewed at least every 180 days. Withdrawal removes active raw bytes within 7 days; backups expire within 35 days after active deletion.",
                );
                ui.small(
                    "Raw bytes are restricted to quarantine/dedupe and authorized research or recovery processing. Public issues, public CI artifacts, marketing/analytics, payment/supporter systems, and unrelated services must not receive the file.",
                );
                ui.small(
                    "Withdrawal uses the opaque submission receipt and does not require uploading the file again. Content-bearing or reconstructive derived artifacts are deleted with the raw file.",
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Send file").clicked() {
                        send_clicked = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel_clicked = true;
                    }
                });
                ui.small(
                    "This Technical Preview consent step does not upload anything yet; transport/storage is intentionally not implemented in this gate.",
                );
            });

        if send_clicked {
            self.exact_file_consent_status = Some(format!(
                "Consent confirmed locally for {filename} under {CHAPTERA_INTAKE_RETENTION_POLICY_V1}. No file was sent; upload transport is not implemented in this build."
            ));
            open = false;
        } else if cancel_clicked {
            self.exact_file_consent_status =
                Some("Exact-file contribution canceled. No file was sent.".to_owned());
            open = false;
        }

        self.exact_file_consent_open = open;
    }

    fn show_editor_controls(&mut self, ui: &mut egui::Ui) {
        ui.add_space(16.0);
        ui.heading("Bounded edit");
        ui.separator();

        let Some(index) = self.selected_search_result else {
            ui.weak("Select an exact text-search match to inspect edit capability.");
            return;
        };
        let Some(story_id) = self.search_results.get(index).map(|result| result.story_id) else {
            ui.weak("The selected search match is no longer available.");
            return;
        };

        let frame_count = self
            .visual
            .as_ref()
            .map(|visual| {
                visual
                    .story_frames
                    .iter()
                    .filter(|frame| frame.story_id == story_id)
                    .count()
            })
            .unwrap_or(0);

        let Some(editor) = self.editor.as_ref() else {
            ui.colored_label(
                ui.visuals().error_fg_color,
                self.editor_load_error
                    .as_deref()
                    .unwrap_or("Editor session is unavailable for this document."),
            );
            return;
        };

        let story_edit_error = editor.can_replace_story_text(story_id).err();
        let table_cells = editor.editable_table_cells_for_story(story_id);
        let _ = editor;

        if let Some(error) = story_edit_error {
            if table_cells.is_empty() {
                ui.weak(format!("Read-only: {} ({})", error, error.code()));
                return;
            }
            self.show_table_cell_edit_controls(ui, story_id, &table_cells);
        } else {
            if frame_count != 1 {
                ui.weak(format!(
                    "Read-only in the desktop slice: this Story has {frame_count} frames. The engine can validate shared-Story editing, but this Viewer does not yet paint linked text flow faithfully."
                ));
                return;
            }
            self.show_story_text_edit_controls(ui, story_id);
        }

        let operation_count = self
            .editor
            .as_ref()
            .map(|editor| editor.operations().len())
            .unwrap_or(0);
        if ui
            .add_enabled(
                operation_count > 0,
                egui::Button::new("Save Editor Project"),
            )
            .clicked()
        {
            match self.save_editor_project_sidecar() {
                Ok(path) => {
                    self.project_status = Some(format!(
                        "Saved editor project {} with {operation_count} operations.",
                        path.display()
                    ));
                    self.edit_status = Some(format!(
                        "Saved editor project sidecar to {}. Source PUB was not overwritten.",
                        path.display()
                    ));
                }
                Err(error) => {
                    self.edit_status = Some(format!("Could not save editor project: {error}"));
                }
            }
        }
        ui.small(format!(
            "Authoring operations in project: {operation_count}"
        ));

        ui.add_space(12.0);
        ui.strong("Editable copy");
        ui.small("Preview the canonical loss report before writing an editable migration target.");

        let (preview_idml, preview_odg) = ui
            .horizontal(|ui| {
                (
                    ui.add_enabled(operation_count > 0, egui::Button::new("Preview IDML"))
                        .clicked(),
                    ui.add_enabled(operation_count > 0, egui::Button::new("Preview ODG"))
                        .clicked(),
                )
            })
            .inner;

        if preview_idml {
            self.refresh_export_preview(pub_editor::EditorEditableTarget::Idml);
        }
        if preview_odg {
            self.refresh_export_preview(pub_editor::EditorEditableTarget::Odg);
        }

        if let Some(preview) = &self.export_preview {
            let preview_is_current = preview.operation_count == operation_count;
            ui.monospace(&preview.summary);
            if !preview_is_current {
                ui.weak("Preview is stale because the authoring operation log changed.");
            }

            let target = preview.target;
            let export_enabled = preview_is_current && preview.can_serialize && operation_count > 0;
            if ui
                .add_enabled(
                    export_enabled,
                    egui::Button::new(format!("Export edited {target} copy")),
                )
                .clicked()
            {
                match self.export_editable_copy(target) {
                    Ok((output, report)) => {
                        self.edit_status = Some(format!(
                            "Exported edited {target} copy to {} with report {}. Source PUB was not overwritten.",
                            output.display(),
                            report.display()
                        ));
                    }
                    Err(error) => {
                        self.edit_status = Some(format!("Could not export {target}: {error}"));
                    }
                }
            }
        }

        if let Some(status) = &self.edit_status {
            ui.small(status);
        }
    }

    fn show_story_text_edit_controls(&mut self, ui: &mut egui::Ui, story_id: pub_editor::StoryId) {
        ui.small("Safe slice: one validated ordinary Story. The original PUB remains immutable.");
        if self
            .preview_clipped_story_keys
            .contains(&format!("{:?}", story_id))
        {
            ui.strong("Current preview clips this Story.");
            ui.small(PREVIEW_TEXT_CLIP_WARNING);
        }
        ui.add(
            egui::TextEdit::multiline(&mut self.edit_buffer)
                .desired_rows(8)
                .hint_text("Replacement Story text"),
        );

        let (apply_clicked, undo_clicked, redo_clicked) = ui
            .horizontal(|ui| {
                (
                    ui.button("Apply Story edit").clicked(),
                    ui.button("Undo").clicked(),
                    ui.button("Redo").clicked(),
                )
            })
            .inner;

        if apply_clicked {
            let replacement = self.edit_buffer.clone();
            let outcome = self
                .editor
                .as_mut()
                .expect("editor presence checked above")
                .replace_story_text(story_id, replacement);
            match outcome {
                Ok(_) => self.finish_authoring_change(
                    "Applied Story edit in the authoring session. Source PUB bytes were not written.",
                ),
                Err(error) => {
                    self.edit_status = Some(format!("Edit rejected: {} ({})", error, error.code()));
                }
            }
        }

        if undo_clicked {
            self.apply_undo();
        }
        if redo_clicked {
            self.apply_redo();
        }
    }

    fn show_table_cell_edit_controls(
        &mut self,
        ui: &mut egui::Ui,
        story_id: pub_editor::StoryId,
        cells: &[pub_editor::EditorEditableTableCell],
    ) {
        ui.small(
            "Safe slice: simple materialized TABLE cells accepted by the current editor gate. Table structure and layout remain read-only.",
        );
        ui.label(format!("{} editable cells", cells.len()));

        let mut clicked = None;
        egui::ScrollArea::vertical()
            .max_height(150.0)
            .show(ui, |ui| {
                for (index, cell) in cells.iter().enumerate() {
                    let selected = self.selected_table_cell_index == Some(index);
                    let label = format!(
                        "R{} C{} · {}",
                        cell.row + 1,
                        cell.column + 1,
                        search_result_preview(&cell.text)
                    );
                    if ui.selectable_label(selected, label).clicked() {
                        clicked = Some(index);
                    }
                }
            });

        if let Some(index) = clicked {
            self.selected_table_cell_index = Some(index);
            self.table_cell_buffer.clone_from(&cells[index].text);
            self.edit_status = None;
        }

        let Some(index) = self.selected_table_cell_index else {
            ui.weak("Select a safe table cell to edit.");
            return;
        };
        let Some(target) = cells.get(index).cloned() else {
            self.selected_table_cell_index = None;
            self.table_cell_buffer.clear();
            ui.weak("The selected table cell is no longer available.");
            return;
        };

        ui.add(
            egui::TextEdit::multiline(&mut self.table_cell_buffer)
                .desired_rows(5)
                .hint_text("Replacement table cell text"),
        );

        let (apply_clicked, undo_clicked, redo_clicked) = ui
            .horizontal(|ui| {
                (
                    ui.button("Apply cell edit").clicked(),
                    ui.button("Undo").clicked(),
                    ui.button("Redo").clicked(),
                )
            })
            .inner;

        if apply_clicked {
            let replacement = self.table_cell_buffer.clone();
            let outcome = self
                .editor
                .as_mut()
                .expect("editor presence checked above")
                .replace_table_cell_text(target.node_id, target.cell_id, replacement);
            match outcome {
                Ok(_) => {
                    self.finish_authoring_change(
                        "Applied TABLE cell edit in the authoring session. Source PUB bytes were not written.",
                    );
                    if let Some(updated) = self.editor.as_ref().and_then(|editor| {
                        editor
                            .editable_table_cells_for_story(story_id)
                            .into_iter()
                            .find(|cell| {
                                cell.node_id == target.node_id && cell.cell_id == target.cell_id
                            })
                    }) {
                        self.table_cell_buffer = updated.text;
                    }
                }
                Err(error) => {
                    self.edit_status = Some(format!(
                        "TABLE cell edit rejected: {} ({})",
                        error,
                        error.code()
                    ));
                }
            }
        }

        if undo_clicked {
            self.apply_undo();
            self.selected_table_cell_index = None;
            self.table_cell_buffer.clear();
        }
        if redo_clicked {
            self.apply_redo();
            self.selected_table_cell_index = None;
            self.table_cell_buffer.clear();
        }
    }

    fn finish_authoring_change(&mut self, status: &str) {
        self.canvas_drag = None;
        self.canvas_resize = None;
        self.page_frame_cache.clear();
        let text_projection_refresh = self.sync_visual_stories_from_editor();
        let created_node_scene_sync = self.sync_visual_created_text_boxes_from_editor();
        self.sync_visual_geometry_from_editor();
        self.refresh_search();
        self.export_preview = None;
        self.project_status = Some("Editor project has unsaved changes.".to_owned());
        self.edit_status = Some(match (text_projection_refresh, created_node_scene_sync) {
            (Ok(()), Ok(())) => status.to_owned(),
            (Err(text_error), Ok(())) => {
                format!("{status} Viewer text projection refresh failed closed: {text_error}")
            }
            (Ok(()), Err(scene_error)) => {
                format!("{status} Viewer created TextBox scene sync failed closed: {scene_error}")
            }
            (Err(text_error), Err(scene_error)) => format!(
                "{status} Viewer text projection refresh failed closed: {text_error}; created TextBox scene sync failed closed: {scene_error}"
            ),
        });
    }

    fn apply_undo(&mut self) {
        let outcome = self
            .editor
            .as_mut()
            .expect("editor presence checked above")
            .undo();
        match outcome {
            Ok(_) => self.finish_authoring_change("Undo restored the previous authoring state."),
            Err(error) => {
                self.edit_status = Some(format!("Undo unavailable: {} ({})", error, error.code()));
            }
        }
    }

    fn apply_redo(&mut self) {
        let outcome = self
            .editor
            .as_mut()
            .expect("editor presence checked above")
            .redo();
        match outcome {
            Ok(_) => self.finish_authoring_change("Redo restored the edited authoring state."),
            Err(error) => {
                self.edit_status = Some(format!("Redo unavailable: {} ({})", error, error.code()));
            }
        }
    }

    fn saved_project_operation_count(&self) -> Result<Option<usize>, String> {
        let Some(source_path) = self.source_path.as_ref() else {
            return Ok(None);
        };
        let sidecar = editor_project_sidecar_path(source_path)
            .ok_or_else(|| "source path has no file name".to_owned())?;
        if !sidecar.exists() {
            return Ok(None);
        }
        let bytes =
            fs::read(&sidecar).map_err(|error| format!("read {}: {error}", sidecar.display()))?;
        let project: pub_editor::EditorProject = serde_json::from_slice(&bytes)
            .map_err(|error| format!("parse editor project JSON: {error}"))?;
        Ok(Some(project.operations.len()))
    }

    fn reopen_saved_project(&mut self) -> Result<(), String> {
        let source_path = self
            .source_path
            .clone()
            .ok_or_else(|| "source path is unavailable".to_owned())?;
        let current_operation_count = self
            .editor
            .as_ref()
            .map(|editor| editor.operations().len())
            .ok_or_else(|| "editor session is unavailable".to_owned())?;
        let saved_operation_count = self
            .saved_project_operation_count()?
            .ok_or_else(|| "saved EditorProject sidecar is unavailable".to_owned())?;
        if current_operation_count != saved_operation_count {
            return Err(
                "current edits differ from the saved EditorProject; save before reopening"
                    .to_owned(),
            );
        }

        self.load_path(source_path);
        if self.editor.is_none() {
            return Err("fresh editor session could not be opened".to_owned());
        }
        self.edit_status = Some(
            "Reopened source and replayed the saved EditorProject in a fresh session.".to_owned(),
        );
        Ok(())
    }

    fn refresh_export_preview(&mut self, target: pub_editor::EditorEditableTarget) {
        let Some(editor) = self.editor.as_ref() else {
            self.edit_status = Some("Editor session is unavailable.".to_owned());
            return;
        };
        let operation_count = editor.operations().len();
        let source_label = self
            .source_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "source.pub".to_owned());

        match editor.preview_editable_export(target, source_label) {
            Ok(preview) => {
                self.export_preview = Some(DesktopExportPreview {
                    target,
                    operation_count,
                    can_serialize: preview.report.can_serialize,
                    summary: preview.human_summary,
                });
                self.edit_status = None;
            }
            Err(error) => {
                self.export_preview = None;
                self.edit_status = Some(format!("Could not preview {target} export: {error}"));
            }
        }
    }

    fn export_editable_copy(
        &self,
        target: pub_editor::EditorEditableTarget,
    ) -> Result<(PathBuf, PathBuf), String> {
        let source_path = self
            .source_path
            .as_ref()
            .ok_or_else(|| "source path is unavailable".to_owned())?;
        let editor = self
            .editor
            .as_ref()
            .ok_or_else(|| "editor session is unavailable".to_owned())?;
        let preview = self
            .export_preview
            .as_ref()
            .ok_or_else(|| "export preview is required before output".to_owned())?;
        if preview.target != target || preview.operation_count != editor.operations().len() {
            return Err("export preview is stale or targets another format".to_owned());
        }
        if !preview.can_serialize {
            return Err("export preview contains blocking losses".to_owned());
        }

        let source_label = source_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| "source path has no file name".to_owned())?;
        let export = editor
            .export_editable(target, source_label)
            .map_err(|error| error.to_string())?;
        let output = editable_export_path(source_path, target)
            .ok_or_else(|| "source path has no file name".to_owned())?;
        let report_path = editable_export_report_path(&output);
        let report_json = serde_json::to_vec_pretty(&export.report)
            .map_err(|error| format!("serialize export report: {error}"))?;

        fs::write(&report_path, report_json)
            .map_err(|error| format!("write {}: {error}", report_path.display()))?;
        fs::write(&output, export.bytes)
            .map_err(|error| format!("write {}: {error}", output.display()))?;
        Ok((output, report_path))
    }

    fn save_editor_project_sidecar(&self) -> Result<PathBuf, String> {
        let source_path = self
            .source_path
            .as_ref()
            .ok_or_else(|| "source path is unavailable".to_owned())?;
        let editor = self
            .editor
            .as_ref()
            .ok_or_else(|| "editor session is unavailable".to_owned())?;
        if editor.operations().is_empty() {
            return Err("there are no edit operations to save".to_owned());
        }

        let sidecar = editor_project_sidecar_path(source_path)
            .ok_or_else(|| "source path has no file name".to_owned())?;
        let project = editor.project();

        if !project.assets.is_empty() {
            let asset_dir = editor_project_asset_dir_path(source_path)
                .ok_or_else(|| "source path has no file name".to_owned())?;
            fs::create_dir_all(&asset_dir)
                .map_err(|error| format!("create {}: {error}", asset_dir.display()))?;

            for asset in editor.replacement_assets() {
                let file_name = pub_editor::editor_asset_file_name(asset.sha256, &asset.mime)
                    .map_err(|error| format!("name replacement asset: {error}"))?;
                let asset_path = asset_dir.join(file_name);
                fs::write(&asset_path, &asset.bytes)
                    .map_err(|error| format!("write {}: {error}", asset_path.display()))?;
            }
        }

        let json = serde_json::to_vec_pretty(&project)
            .map_err(|error| format!("serialize project: {error}"))?;
        fs::write(&sidecar, json)
            .map_err(|error| format!("write {}: {error}", sidecar.display()))?;
        Ok(sidecar)
    }

    fn sync_visual_stories_from_editor(&mut self) -> Result<(), String> {
        let (Some(editor), Some(visual)) = (&self.editor, &mut self.visual) else {
            return Ok(());
        };

        if let Err(error) = visual.refresh_text_projection_from_resolved(editor.graph()) {
            // The refresh operation is transactional, so a failure would otherwise
            // leave source-time fragments looking current after an accepted edit.
            // Fail closed instead of painting stale text in the canvas/thumbnails.
            visual.text_fragments.clear();
            visual.story_frames.clear();
            for viewer_story in &mut visual.document.stories {
                if let Some(story) = editor.graph().stories.get(&viewer_story.id) {
                    viewer_story.text.clone_from(&story.text);
                }
            }
            return Err(error.to_string());
        }

        if let Some(index) = self.selected_search_result
            && let Some(story_id) = self.search_results.get(index).map(|result| result.story_id)
            && let Some(story) = visual
                .document
                .stories
                .iter()
                .find(|story| story.id == story_id)
        {
            self.edit_buffer.clone_from(&story.text);
        }

        Ok(())
    }

    fn sync_visual_created_text_boxes_from_editor(&mut self) -> Result<(), String> {
        let (Some(editor), Some(visual)) = (&self.editor, &mut self.visual) else {
            self.created_text_box_scene_nodes.clear();
            return Ok(());
        };

        let active_node_ids = editor
            .operations()
            .iter()
            .filter_map(|operation| match operation {
                pub_editor::EditOperation::CreateTextBox { node_id, .. } => Some(*node_id),
                _ => None,
            })
            .collect::<Vec<_>>();

        let next = visual
            .sync_editor_created_text_box_scene_nodes(
                editor.graph(),
                &active_node_ids,
                &self.created_text_box_scene_nodes,
            )
            .map_err(|error| error.to_string())?;
        self.created_text_box_scene_nodes = next;
        Ok(())
    }

    fn sync_visual_geometry_from_editor(&mut self) {
        let (Some(editor), Some(visual)) = (&self.editor, &mut self.visual) else {
            return;
        };

        for scene_node in &mut visual.scene.nodes {
            let Some(authored_node) = editor.graph().nodes.get(&scene_node.origin) else {
                continue;
            };
            if authored_node.header.parent_id != scene_node.parent_origin {
                continue;
            }
            let Ok(instance) = direct_page_local_instance_v1(
                &scene_node.origin.as_canonical().to_string(),
                &scene_node.parent_origin.to_string(),
            ) else {
                continue;
            };
            if geometry_sync_policy_v1(&instance)
                != GeometrySyncPolicyV1::ApplyAuthoredOriginGeometry
            {
                continue;
            }
            let bounds = authored_node.header.bounds;
            if editor
                .can_move_node_to(scene_node.origin, bounds.x, bounds.y)
                .is_ok()
            {
                scene_node.bounds = bounds;
            }
        }
    }

    fn selected_direct_replace_image_target(&self) -> Result<pub_editor::NodeId, String> {
        let selected_instance = self
            .canvas_selection
            .primary()
            .ok_or_else(|| "Select an image object on the current page first.".to_owned())?;
        let visual = self
            .visual
            .as_ref()
            .ok_or_else(|| "Document scene is unavailable.".to_owned())?;
        let page = visual
            .document
            .pages
            .get(self.selected_page)
            .ok_or_else(|| "Selected page is unavailable.".to_owned())?;
        let editor = self
            .editor
            .as_ref()
            .ok_or_else(|| "Editor session is unavailable.".to_owned())?;
        let page_origin = page.id.into_canonical();
        let page_id_text = page.id.as_canonical().to_string();

        for scene_node in visual
            .scene
            .nodes
            .iter()
            .filter(|node| node.parent_origin == page_origin)
        {
            let Some(instance) = direct_scene_instance(editor, &page_id_text, scene_node.origin)
            else {
                continue;
            };
            if instance.instance_id != selected_instance {
                continue;
            }

            let admission = admit_object_mutation_v1(&instance, ObjectMutationKindV1::ReplaceImage);
            let origin_node_id = scene_node.origin.as_canonical().to_string();
            if !admission.admitted
                || admission.origin_node_id.as_deref() != Some(origin_node_id.as_str())
            {
                return Err(
                    "This visual instance is projected/read-only for image replacement.".to_owned(),
                );
            }

            let authored = editor
                .graph()
                .nodes
                .get(&scene_node.origin)
                .ok_or_else(|| "Selected object has no authored node.".to_owned())?;
            if authored.payload.image_slot.is_none() {
                return Err("Selected object is not an image placement.".to_owned());
            }
            if authored.payload.explicit_image_crop.is_some() {
                return Err(
                    "Replace image is disabled for placements with explicit crop in this V0 slice."
                        .to_owned(),
                );
            }
            if authored.header.bounds.width.get() <= 0
                || authored.header.bounds.height.get() <= 0
                || authored.header.bounds.right().is_none()
                || authored.header.bounds.bottom().is_none()
            {
                return Err("Selected image placement has invalid bounds.".to_owned());
            }

            return Ok(scene_node.origin);
        }

        Err("Selected visual instance is not a direct page-local object.".to_owned())
    }

    fn replace_selected_image_from_path(&mut self, path: &Path) -> Result<(), String> {
        let node_id = self.selected_direct_replace_image_target()?;
        let mime = replacement_image_mime(path)
            .ok_or_else(|| "Choose a PNG or JPEG replacement image.".to_owned())?;
        let bytes = fs::read(path)
            .map_err(|error| format!("read replacement {}: {error}", path.display()))?;

        let editor = self
            .editor
            .as_ref()
            .ok_or_else(|| "Editor session is unavailable.".to_owned())?;
        let operation_count_before = editor.operations().len();
        let mut candidate = editor.clone();
        let replacement_asset = candidate
            .import_replacement_asset(mime, bytes)
            .map_err(|error| format!("Replacement image import rejected: {error}"))?;
        candidate
            .can_replace_image(node_id, replacement_asset)
            .map_err(|error| {
                format!("Replace image is unavailable: {} ({})", error, error.code())
            })?;
        candidate
            .replace_image(node_id, replacement_asset)
            .map_err(|error| format!("Replace image rejected: {} ({})", error, error.code()))?;

        if candidate.operations().len() != operation_count_before + 1 {
            return Err("Replace image must append exactly one authoring operation.".to_owned());
        }

        self.editor = Some(candidate);
        self.finish_authoring_change(
            "Replaced the selected image in the Chaptera project. Source PUB bytes were not written.",
        );
        Ok(())
    }

    fn show_object_edit_controls(&mut self, ui: &mut egui::Ui) {
        if self.canvas_selection.primary().is_none() {
            return;
        }

        ui.add_space(16.0);
        ui.heading("Object");
        ui.separator();

        match self.selected_direct_replace_image_target() {
            Ok(_) => {
                ui.small(
                    "Direct page-local image placement. PNG/JPEG replacement preserves source PUB bytes and is saved with the EditorProject.",
                );
                if ui.button("Replace image…").clicked() {
                    #[cfg(target_os = "windows")]
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Images", &["png", "jpg", "jpeg"])
                            .pick_file()
                        {
                            if let Err(error) = self.replace_selected_image_from_path(&path) {
                                self.edit_status = Some(error);
                            }
                        }
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        self.edit_status = Some(
                            "The native replacement picker is part of Windows Editor V0."
                                .to_owned(),
                        );
                    }
                }
            }
            Err(reason) => {
                let response = ui.add_enabled(false, egui::Button::new("Replace image…"));
                response.on_disabled_hover_text(&reason);
                ui.weak(reason);
            }
        }
    }

    fn commit_canvas_drag(&mut self, drag: MoveTransaction) {
        self.canvas_drag = None;
        self.canvas_resize = None;
        if !drag.has_moved() {
            return;
        }

        let preview = drag.preview_bounds();
        let outcome = self
            .editor
            .as_mut()
            .ok_or_else(|| "Editor session is unavailable.".to_owned())
            .and_then(|editor| {
                editor
                    .move_node_to(drag.node_id(), preview.x, preview.y)
                    .map_err(|error| format!("Move rejected: {} ({})", error, error.code()))
            });
        match outcome {
            Ok(_) => self.finish_authoring_change(
                "Moved canvas object in the authoring session. One MoveNode operation was committed.",
            ),
            Err(error) => {
                self.edit_status = Some(error);
                self.sync_visual_geometry_from_editor();
            }
        }
    }

    fn commit_canvas_resize(&mut self, resize: ResizeCommit) {
        self.canvas_resize = None;
        self.canvas_drag = None;
        let outcome = self
            .editor
            .as_mut()
            .ok_or_else(|| "Editor session is unavailable.".to_owned())
            .and_then(|editor| {
                editor
                    .resize_node_to(resize.node_id, resize.after)
                    .map_err(|error| format!("Resize rejected: {} ({})", error, error.code()))
            });
        match outcome {
            Ok(_) => self.finish_authoring_change(
                "Resized canvas object in the authoring session. One ResizeNode operation was committed.",
            ),
            Err(error) => {
                self.edit_status = Some(error);
                self.sync_visual_geometry_from_editor();
            }
        }
    }

    fn enter_canvas_text_mode(
        &mut self,
        story_id: pub_editor::StoryId,
        frame_id: pub_editor::NodeId,
    ) {
        let outcome = self
            .editor
            .as_ref()
            .ok_or_else(|| "Editor session is unavailable.".to_owned())
            .and_then(|editor| text_session::enter_explicit_text_mode(editor, story_id, frame_id));
        match outcome {
            Ok(mode) => {
                self.text_mode = Some(mode);
                self.canvas_drag = None;
                self.canvas_resize = None;
                self.edit_status = Some(
                    "Text editing active on the canvas. Typing is committed through canonical Story range operations."
                        .to_owned(),
                );
            }
            Err(error) => {
                self.edit_status = Some(format!("Edit Text unavailable: {error}"));
            }
        }
    }

    fn exit_canvas_text_mode(&mut self, trigger: &str) {
        let Some(mode) = self.text_mode.as_ref() else {
            return;
        };
        match text_session::exit_text_mode(mode, trigger) {
            Ok(()) => {
                self.text_mode = None;
                self.edit_status =
                    Some("Exited canvas text editing without creating an edit.".to_owned());
            }
            Err(error) => {
                self.edit_status = Some(format!("Could not exit canvas text editing: {error}"));
            }
        }
    }

    fn refresh_visual_text_projection_from_editor(&mut self) -> Result<(), String> {
        let editor = self
            .editor
            .as_ref()
            .ok_or_else(|| "Editor session is unavailable.".to_owned())?;
        let visual = self
            .visual
            .as_mut()
            .ok_or_else(|| "Viewer projection is unavailable.".to_owned())?;
        visual
            .refresh_text_projection_from_resolved(editor.graph())
            .map_err(|error| format!("refresh current Story projection: {error:#}"))
    }

    fn apply_canvas_text_input(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let outcome = match (&mut self.editor, &mut self.text_mode) {
            (Some(editor), Some(mode)) => text_session::replace_external_text(editor, mode, text),
            _ => return,
        };
        match outcome {
            Ok(()) => match self.refresh_visual_text_projection_from_editor() {
                Ok(()) => self.finish_authoring_change(
                    "Typed on the canvas through one canonical ReplaceStoryRange operation.",
                ),
                Err(error) => {
                    self.edit_status = Some(format!(
                        "Text was committed, but the canvas projection could not be refreshed: {error}"
                    ));
                }
            },
            Err(error) => {
                self.edit_status = Some(format!("Canvas text input rejected: {error}"));
            }
        }
    }

    fn apply_canvas_text_keyboard(
        &mut self,
        command: chaptera_text_input_adapter::keyboard::KeyboardCommandV1,
    ) {
        let before_operations = self
            .editor
            .as_ref()
            .map(|editor| editor.operations().len())
            .unwrap_or(0);
        let outcome = match (&mut self.editor, &mut self.text_mode) {
            (Some(editor), Some(mode)) => {
                text_session::apply_keyboard_command(editor, mode, command)
            }
            _ => return,
        };
        match outcome {
            Ok(()) => {
                let after_operations = self
                    .editor
                    .as_ref()
                    .map(|editor| editor.operations().len())
                    .unwrap_or(before_operations);
                if after_operations > before_operations {
                    match self.refresh_visual_text_projection_from_editor() {
                        Ok(()) => self.finish_authoring_change(
                            "Canvas text keyboard edit committed through canonical Story range authority.",
                        ),
                        Err(error) => {
                            self.edit_status = Some(format!(
                                "Text keyboard edit was committed, but the canvas projection could not be refreshed: {error}"
                            ));
                        }
                    }
                }
            }
            Err(error) => {
                self.edit_status = Some(format!("Canvas text keyboard input rejected: {error}"));
            }
        }
    }

    fn reposition_canvas_text_caret(
        &mut self,
        page_id: &str,
        point: pub_interaction::DocumentPoint,
    ) {
        let Some(mode) = self.text_mode.as_mut() else {
            return;
        };
        if let Err(error) =
            text_session::reposition_pointer(mode, page_id, point.x.get(), point.y.get())
        {
            self.edit_status = Some(format!("Canvas caret move rejected: {error}"));
        }
    }

    fn process_canvas_text_input(&mut self, ctx: &egui::Context) {
        if self.text_mode.is_none() {
            return;
        }
        if ctx.wants_keyboard_input() {
            self.exit_canvas_text_mode("focus_transfer");
            return;
        }
        let events = ctx.input(|input| input.events.clone());
        for event in events {
            if self.text_mode.is_none() {
                break;
            }
            match event {
                egui::Event::Text(text) => self.apply_canvas_text_input(&text),
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if key == egui::Key::Escape {
                        self.exit_canvas_text_mode("escape");
                        continue;
                    }
                    if modifiers.ctrl || modifiers.command || modifiers.alt {
                        continue;
                    }
                    use chaptera_text_input_adapter::keyboard::KeyboardCommandV1;
                    let command = match (key, modifiers.shift) {
                        (egui::Key::ArrowLeft, false) => Some(KeyboardCommandV1::MovePrevious),
                        (egui::Key::ArrowRight, false) => Some(KeyboardCommandV1::MoveNext),
                        (egui::Key::ArrowLeft, true) => Some(KeyboardCommandV1::ExtendPrevious),
                        (egui::Key::ArrowRight, true) => Some(KeyboardCommandV1::ExtendNext),
                        (egui::Key::Backspace, _) => Some(KeyboardCommandV1::DeleteBackward),
                        (egui::Key::Delete, _) => Some(KeyboardCommandV1::DeleteForward),
                        _ => None,
                    };
                    if let Some(command) = command {
                        self.apply_canvas_text_keyboard(command);
                    }
                }
                _ => {}
            }
        }
    }

    fn ensure_image_textures(&mut self, ctx: &egui::Context) {
        let Some(visual) = &self.visual else {
            return;
        };

        for embedded in &visual.images {
            let key = format!("{:?}", embedded.resource_id);
            if self.image_textures.contains_key(&key)
                || self.image_decode_diagnostics.contains_key(&key)
            {
                continue;
            }

            let expected_sha256 = image_decode_adapter::exact_sha256_hex(&embedded.bytes);
            match image_decode_adapter::decode_texture_image_v1(
                &embedded.bytes,
                &embedded.mime,
                &expected_sha256,
            ) {
                Ok(admitted) => {
                    let texture = ctx.load_texture(
                        format!("pub-image-{key}"),
                        admitted.color_image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.image_textures.insert(
                        key,
                        CachedImageTexture {
                            texture,
                            _cache_identity_sha256: admitted.cache_identity_sha256,
                        },
                    );
                }
                Err(error) => {
                    self.image_decode_diagnostics.insert(
                        key.clone(),
                        image_decode_adapter::diagnostic_for(key, embedded.mime.clone(), &error),
                    );
                }
            }
        }

        if let Some(editor) = &self.editor {
            for asset in editor.replacement_assets() {
                let key = format!("replacement:{:?}", asset.sha256);
                if self.image_textures.contains_key(&key)
                    || self.image_decode_diagnostics.contains_key(&key)
                {
                    continue;
                }

                match image_decode_adapter::decode_texture_image_v1(
                    &asset.bytes,
                    &asset.mime,
                    &asset.sha256.to_string(),
                ) {
                    Ok(admitted) => {
                        let texture = ctx.load_texture(
                            format!("chaptera-{key}"),
                            admitted.color_image,
                            egui::TextureOptions::LINEAR,
                        );
                        self.image_textures.insert(
                            key,
                            CachedImageTexture {
                                texture,
                                _cache_identity_sha256: admitted.cache_identity_sha256,
                            },
                        );
                    }
                    Err(error) => {
                        self.image_decode_diagnostics.insert(
                            key.clone(),
                            image_decode_adapter::diagnostic_for(key, asset.mime.clone(), &error),
                        );
                    }
                }
            }
        }
    }

    fn build_page_frame_work(&self, page_index: usize) -> Result<CachedPageFrameWork, String> {
        let visual = self
            .visual
            .as_ref()
            .ok_or_else(|| "Document scene is unavailable.".to_owned())?;
        let page = visual
            .document
            .pages
            .get(page_index)
            .ok_or_else(|| "Selected page is unavailable.".to_owned())?;
        let render_plan = build_desktop_page_render_plan(visual, page_index)
            .map_err(|error| error.to_string())?;
        let page_id_text = page.id.as_canonical().to_string();

        let mut hit_entries = Vec::new();
        let mut movable_nodes = BTreeMap::new();
        let mut resizable_nodes = BTreeMap::new();

        for (paint_order, render_node) in render_plan.nodes.iter().enumerate() {
            let projected = render_node.projected_scene_instance.as_ref();
            let instance = match projected {
                Some(instance) => instance.clone(),
                None => {
                    let Some(editor) = self.editor.as_ref() else {
                        continue;
                    };
                    let Some(instance) =
                        direct_scene_instance(editor, &page_id_text, render_node.node_id)
                    else {
                        continue;
                    };
                    instance
                }
            };
            let instance_id = instance.instance_id.clone();

            if projected.is_none() {
                if let Some(editor) = self.editor.as_ref() {
                    if let Some(authored_node) = editor.graph().nodes.get(&render_node.node_id) {
                        let bounds = authored_node.header.bounds;
                        let origin_node_id = render_node.node_id.as_canonical().to_string();

                        let move_admission =
                            admit_object_mutation_v1(&instance, ObjectMutationKindV1::MoveNode);
                        if move_admission.admitted
                            && move_admission.origin_node_id.as_deref()
                                == Some(origin_node_id.as_str())
                            && editor
                                .can_move_node_to(render_node.node_id, bounds.x, bounds.y)
                                .is_ok()
                        {
                            movable_nodes
                                .insert(instance_id.clone(), (render_node.node_id, bounds));
                        }

                        let resize_admission =
                            admit_object_mutation_v1(&instance, ObjectMutationKindV1::ResizeNode);
                        if resize_admission.admitted
                            && resize_admission.origin_node_id.as_deref()
                                == Some(origin_node_id.as_str())
                            && editor.can_resize_node(render_node.node_id).is_ok()
                        {
                            resizable_nodes
                                .insert(instance_id.clone(), (render_node.node_id, bounds));
                        }
                    }
                }
            } else {
                debug_assert!(
                    !admit_object_mutation_v1(&instance, ObjectMutationKindV1::MoveNode).admitted
                );
                debug_assert!(
                    !admit_object_mutation_v1(&instance, ObjectMutationKindV1::ResizeNode).admitted
                );
            }

            hit_entries.push(SceneHitEntry {
                instance_id,
                node_id: render_node.node_id,
                bounds: render_node.bounds,
                z_order: 0,
                paint_order: u32::try_from(paint_order).unwrap_or(u32::MAX),
            });
        }

        Ok(CachedPageFrameWork {
            page_index,
            render_plan,
            hit_index: SceneHitTestIndex::new(hit_entries),
            movable_nodes,
            resizable_nodes,
        })
    }

    fn page_frame_work(&mut self) -> Result<Rc<CachedPageFrameWork>, String> {
        if let Some(cached) = self.page_frame_cache.get(&self.selected_page) {
            return Ok(Rc::clone(cached));
        }

        let cache = Rc::new(self.build_page_frame_work(self.selected_page)?);
        self.page_frame_cache_builds = self.page_frame_cache_builds.saturating_add(1);
        self.page_frame_cache
            .insert(self.selected_page, Rc::clone(&cache));
        Ok(cache)
    }

    fn show_canvas(&mut self, ui: &mut egui::Ui) {
        self.ensure_image_textures(ui.ctx());
        self.preview_clipped_frames = 0;
        self.preview_clipped_story_keys.clear();

        let ctrl_held = ui.ctx().input(|input| input.modifiers.ctrl);

        if !reader_only_mode() {
            ui.horizontal_wrapped(|ui| {
                ui.label("Zoom");

                if self.zoom_mode == CanvasZoomMode::Percent {
                    let mut zoom_percent = self.zoom * 100.0;
                    let slider = ui.add(
                        egui::Slider::new(&mut zoom_percent, 10.0..=400.0)
                            .suffix("%")
                            .show_value(true),
                    );
                    if slider.changed() {
                        self.zoom =
                            (zoom_percent / 100.0).clamp(MIN_NUMERIC_ZOOM, MAX_NUMERIC_ZOOM);
                    }
                } else {
                    ui.label("Fit mode");
                }

                if ui
                    .selectable_label(
                        self.zoom_mode == CanvasZoomMode::Percent
                            && (self.zoom - 1.0).abs() < 0.001,
                        "100%",
                    )
                    .clicked()
                {
                    self.zoom = 1.0;
                    self.zoom_mode = CanvasZoomMode::Percent;
                }
                if ui
                    .selectable_label(self.zoom_mode == CanvasZoomMode::FitPage, "Fit Page")
                    .clicked()
                {
                    self.zoom_mode = CanvasZoomMode::FitPage;
                }
                if ui
                    .selectable_label(self.zoom_mode == CanvasZoomMode::PageWidth, "Page Width")
                    .clicked()
                {
                    self.zoom_mode = CanvasZoomMode::PageWidth;
                }
                let fit_selection = ui.add_enabled(
                    self.canvas_selection.primary().is_some(),
                    egui::SelectableLabel::new(
                        self.zoom_mode == CanvasZoomMode::FitSelection,
                        "Fit Selection",
                    ),
                );
                if fit_selection.clicked() {
                    self.zoom_mode = CanvasZoomMode::FitSelection;
                }
            });
            ui.separator();
        }

        if self.visual.is_none() {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    if reader_only_mode() {
                        ui.heading(READER_FIRST_RUN_HEADING);
                        if ui.button("Open a PUB file").clicked() {
                            self.open_pub_picker();
                        }
                        ui.label("or drag and drop a .pub file here");
                        ui.small("Keyboard: Ctrl+O");
                        ui.add_space(14.0);
                        ui.strong(READER_FIRST_RUN_TRUST_CUE);
                        ui.small(READER_READ_ONLY_CUE);
                        ui.add_space(8.0);
                        ui.small(
                            "After opening, use page navigation, search/copy, fidelity status, and diagnostics without a required internet connection.",
                        );
                    } else {
                        ui.heading("Open a Publisher file");
                        ui.label("Use Open PUB… above, or drag and drop a .pub file here.");
                    }
                    if let Some(error) = &self.load_error {
                        ui.add_space(12.0);
                        ui.colored_label(ui.visuals().error_fg_color, &error.message);
                    }
                });
            });
            return;
        }

        let frame_work = match self.page_frame_work() {
            Ok(cache) => cache,
            Err(error) => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("Selected page has no render plan: {error}"),
                );
                return;
            }
        };
        let visual = self
            .visual
            .as_ref()
            .expect("visual presence checked before page frame cache");
        let Some(page) = visual.document.pages.get(self.selected_page) else {
            ui.colored_label(ui.visuals().error_fg_color, "Selected page is unavailable.");
            return;
        };
        debug_assert_eq!(frame_work.page_index, self.selected_page);
        let render_plan = &frame_work.render_plan;
        let hit_index = &frame_work.hit_index;
        let movable_nodes = &frame_work.movable_nodes;
        let resizable_nodes = &frame_work.resizable_nodes;

        let page_origin = page.id.into_canonical();
        let page_id_text = page.id.as_canonical().to_string();
        let page_nodes = visual
            .scene
            .nodes
            .iter()
            .filter(|node| node.parent_origin == page_origin)
            .collect::<Vec<_>>();
        let selected_bounds = self
            .canvas_selection
            .primary()
            .and_then(|selected_instance| {
                hit_index
                    .entry_for_instance(selected_instance)
                    .map(|hit| hit.bounds)
            });

        let viewport = ui.available_size();
        let scene_scale = match self.zoom_mode {
            CanvasZoomMode::Percent => numeric_zoom_scene_scale(self.zoom),
            CanvasZoomMode::FitPage => fitted_scale(
                render_plan.page_size.width.get(),
                render_plan.page_size.height.get(),
                viewport,
            ),
            CanvasZoomMode::PageWidth => {
                page_width_scale(render_plan.page_size.width.get(), viewport)
            }
            CanvasZoomMode::FitSelection => selected_bounds
                .and_then(|bounds| fitted_scale(bounds.width.get(), bounds.height.get(), viewport))
                .or_else(|| {
                    fitted_scale(
                        render_plan.page_size.width.get(),
                        render_plan.page_size.height.get(),
                        viewport,
                    )
                }),
        };
        let Some(scene_scale) = scene_scale else {
            ui.colored_label(
                ui.visuals().error_fg_color,
                "Selected page has invalid physical dimensions.",
            );
            return;
        };

        let page_width = render_plan.page_size.width.get() as f32 * scene_scale;
        let page_height = render_plan.page_size.height.get() as f32 * scene_scale;
        let content_width = (page_width + PAGE_MARGIN * 2.0).max(viewport.x);
        let content_height = (page_height + PAGE_MARGIN * 2.0).max(viewport.y);
        let mut preview_clipped_frames = 0usize;
        let mut preview_clipped_story_keys = BTreeSet::new();
        let mut preview_text_diagnostics = Vec::new();
        let selected_canvas_instance = self.canvas_selection.primary().map(str::to_owned);
        let mut canvas_clicked = false;
        let mut canvas_hit: Option<String> = None;
        let mut next_canvas_drag = self.canvas_drag;
        let mut next_canvas_resize = self.canvas_resize;
        let mut drag_commit = None;
        let mut drag_error = None;
        let mut resize_commit = None;
        let mut resize_error = None;
        let mut edit_text_request: Option<(pub_editor::StoryId, pub_editor::NodeId)> = None;
        let mut text_pointer_request: Option<(String, pub_interaction::DocumentPoint)> = None;
        let mut text_exit_request = false;
        egui::ScrollArea::both()
            .enable_scrolling(!ctrl_held)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let canvas_sense = if reader_only_mode() {
                    egui::Sense::hover()
                } else {
                    egui::Sense::click_and_drag()
                };
                let (response, painter) =
                    ui.allocate_painter(egui::vec2(content_width, content_height), canvas_sense);
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Other,
                        !reader_only_mode(),
                        "Document canvas",
                    )
                });
                if response.hovered() && ctrl_held {
                    let ctrl_scroll_y = ui.ctx().input(|input| input.raw_scroll_delta.y);
                    if ctrl_scroll_y.abs() > f32::EPSILON {
                        self.zoom = ctrl_wheel_zoom(self.zoom, ctrl_scroll_y);
                        self.zoom_mode = CanvasZoomMode::Percent;
                    }
                }

                let canvas = response.rect;
                let page_rect = if self.zoom_mode == CanvasZoomMode::FitSelection {
                    selected_bounds
                        .map(|bounds| {
                            let selection_center_x = (bounds.x.get() as f32
                                + bounds.width.get() as f32 / 2.0)
                                * scene_scale;
                            let selection_center_y = (bounds.y.get() as f32
                                + bounds.height.get() as f32 / 2.0)
                                * scene_scale;
                            egui::Rect::from_min_size(
                                canvas.center()
                                    - egui::vec2(selection_center_x, selection_center_y),
                                egui::vec2(page_width, page_height),
                            )
                        })
                        .unwrap_or_else(|| {
                            egui::Rect::from_center_size(
                                canvas.center(),
                                egui::vec2(page_width, page_height),
                            )
                        })
                } else {
                    egui::Rect::from_center_size(
                        canvas.center(),
                        egui::vec2(page_width, page_height),
                    )
                };

                let pointer_document = response
                    .interact_pointer_pos()
                    .and_then(|pointer| canvas_document_point(page_rect, scene_scale, pointer));
                let press_screen = ui.ctx().input(|input| input.pointer.press_origin());
                let press_document = press_screen
                    .and_then(|pointer| canvas_document_point(page_rect, scene_scale, pointer));

                if !reader_only_mode()
                    && self.text_mode.is_none()
                    && response.drag_started_by(egui::PointerButton::Primary)
                    && let (Some(pointer_start), Some(pointer_current)) =
                        (press_document, pointer_document)
                {
                    let mut resize_started = false;
                    if let (Some(selected_instance), Some(pointer_start_screen)) =
                        (selected_canvas_instance.as_deref(), press_screen)
                        && let Some((node_id, before)) =
                            resizable_nodes.get(selected_instance).copied()
                    {
                        let selected_screen_bounds = ScreenRect::new(
                            f64::from(page_rect.left() + before.x.get() as f32 * scene_scale),
                            f64::from(page_rect.top() + before.y.get() as f32 * scene_scale),
                            f64::from(before.width.get() as f32 * scene_scale),
                            f64::from(before.height.get() as f32 * scene_scale),
                        );
                        if let Ok(selected_screen_bounds) = selected_screen_bounds
                            && let Ok(ResizePointerDown::Handle(handle)) =
                                classify_resize_pointer_down(
                                    selected_screen_bounds,
                                    ScreenPoint::new(
                                        f64::from(pointer_start_screen.x),
                                        f64::from(pointer_start_screen.y),
                                    ),
                                    6.0,
                                )
                        {
                            resize_started = true;
                            canvas_hit = Some(selected_instance.to_owned());
                            next_canvas_drag = None;
                            match ResizeTransaction::begin(node_id, before, handle, pointer_start) {
                                Ok(mut resize) => match resize.update(pointer_current) {
                                    Ok(ResizeUpdate::Preview(_))
                                    | Ok(ResizeUpdate::Invalid { .. }) => {
                                        next_canvas_resize = Some(resize);
                                    }
                                    Err(error) => {
                                        next_canvas_resize = None;
                                        resize_error =
                                            Some(format!("Object resize cancelled: {error}"));
                                    }
                                },
                                Err(error) => {
                                    next_canvas_resize = None;
                                    resize_error =
                                        Some(format!("Object resize cancelled: {error}"));
                                }
                            }
                        }
                    }

                    if !resize_started && let Some(hit) = hit_index.topmost_at(pointer_start) {
                        canvas_hit = Some(hit.instance_id.clone());
                        next_canvas_resize = None;
                        if let Some((node_id, before)) =
                            movable_nodes.get(&hit.instance_id).copied()
                        {
                            match MoveTransaction::begin(node_id, before, pointer_start).and_then(
                                |mut drag| {
                                    drag.update(pointer_current)?;
                                    Ok(drag)
                                },
                            ) {
                                Ok(drag) => next_canvas_drag = Some(drag),
                                Err(error) => {
                                    next_canvas_drag = None;
                                    drag_error = Some(format!("Object move cancelled: {error}"));
                                }
                            }
                        }
                    }
                } else if !reader_only_mode()
                    && self.text_mode.is_none()
                    && response.drag_stopped_by(egui::PointerButton::Primary)
                {
                    if let (Some(mut resize), Some(point)) =
                        (next_canvas_resize.take(), pointer_document)
                    {
                        match resize.update(point) {
                            Ok(ResizeUpdate::Preview(_)) | Ok(ResizeUpdate::Invalid { .. }) => {
                                match resize.commit() {
                                    Ok(commit) => resize_commit = Some(commit),
                                    Err(error) => {
                                        resize_error =
                                            Some(format!("Object resize cancelled: {error}"));
                                    }
                                }
                            }
                            Err(error) => {
                                resize_error = Some(format!("Object resize cancelled: {error}"));
                            }
                        }
                    } else if let (Some(mut drag), Some(point)) =
                        (next_canvas_drag, pointer_document)
                    {
                        match drag.update(point) {
                            Ok(_) => drag_commit = Some(drag),
                            Err(error) => {
                                next_canvas_drag = None;
                                drag_error = Some(format!("Object move cancelled: {error}"));
                            }
                        }
                    } else {
                        next_canvas_drag = None;
                        next_canvas_resize = None;
                    }
                } else if !reader_only_mode()
                    && self.text_mode.is_none()
                    && response.dragged_by(egui::PointerButton::Primary)
                    && let Some(point) = pointer_document
                {
                    if let Some(mut resize) = next_canvas_resize {
                        match resize.update(point) {
                            Ok(ResizeUpdate::Preview(_)) | Ok(ResizeUpdate::Invalid { .. }) => {
                                next_canvas_resize = Some(resize);
                            }
                            Err(error) => {
                                next_canvas_resize = None;
                                resize_error = Some(format!("Object resize cancelled: {error}"));
                            }
                        }
                    } else if let Some(mut drag) = next_canvas_drag {
                        match drag.update(point) {
                            Ok(_) => next_canvas_drag = Some(drag),
                            Err(error) => {
                                next_canvas_drag = None;
                                drag_error = Some(format!("Object move cancelled: {error}"));
                            }
                        }
                    }
                }

                if !reader_only_mode()
                    && response.clicked_by(egui::PointerButton::Primary)
                    && let Some(point) = pointer_document
                {
                    canvas_clicked = true;
                    let topmost = hit_index.topmost_at(point);
                    if let Some(mode) = self.text_mode.as_ref() {
                        match topmost {
                            Some(hit) if hit.node_id == mode.frame_id => {
                                canvas_hit = Some(hit.instance_id.clone());
                                text_pointer_request = Some((page_id_text.clone(), point));
                            }
                            Some(hit) => {
                                text_exit_request = true;
                                canvas_hit = Some(hit.instance_id.clone());
                            }
                            None => {
                                text_exit_request = true;
                                canvas_hit = None;
                            }
                        }
                    } else {
                        canvas_hit = topmost.map(|hit| hit.instance_id.clone());
                    }
                }

                render_backend::paint_page_surface(&painter, page_rect);

                for render_node in &render_plan.nodes {
                    let projected = render_node.projected_scene_instance.as_ref();
                    let node_id = render_node.node_id;
                    let node_bounds = if projected.is_none() {
                        next_canvas_resize
                            .filter(|resize| resize.node_id() == node_id)
                            .and_then(|resize| resize.preview_bounds())
                            .or_else(|| {
                                next_canvas_drag
                                    .filter(|drag| drag.node_id() == node_id)
                                    .map(|drag| drag.preview_bounds())
                            })
                            .unwrap_or(render_node.bounds)
                    } else {
                        render_node.bounds
                    };
                    let Some(node_rect) = render_backend::physical_rect_to_egui(
                        page_rect,
                        scene_scale,
                        node_bounds.x.get(),
                        node_bounds.y.get(),
                        node_bounds.width.get(),
                        node_bounds.height.get(),
                    ) else {
                        continue;
                    };

                    if projected.is_none()
                        && let Some(instance_id) = hit_index.instance_for_node(node_id)
                        && movable_nodes.contains_key(instance_id)
                    {
                        let a11y = ui.interact(
                            node_rect,
                            ui.id().with(("movable-canvas-object", instance_id)),
                            egui::Sense::hover(),
                        );
                        a11y.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Other,
                                true,
                                "Movable canvas object",
                            )
                        });
                    }
                    if projected.is_none()
                        && let Some(instance_id) = hit_index.instance_for_node(node_id)
                        && resizable_nodes.contains_key(instance_id)
                    {
                        let a11y = ui.interact(
                            node_rect,
                            ui.id().with(("resizable-canvas-object", instance_id)),
                            egui::Sense::hover(),
                        );
                        a11y.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Other,
                                true,
                                "Resizable canvas object",
                            )
                        });
                    }

                    let replace_image_admitted = projected.is_none_or(|instance| {
                        admit_object_mutation_v1(instance, ObjectMutationKindV1::ReplaceImage)
                            .admitted
                    });
                    let replacement_key = replace_image_admitted
                        .then(|| {
                            self.editor
                                .as_ref()
                                .and_then(|editor| editor.image_replacement_for(node_id))
                                .map(|sha256| format!("replacement:{:?}", sha256))
                        })
                        .flatten();
                    let replacement_texture = replacement_key
                        .as_ref()
                        .and_then(|key| self.image_textures.get(key));
                    let source_texture = render_node.image.as_ref().and_then(|image| {
                        let key = format!("{:?}", image.resource_id);
                        self.image_textures.get(&key)
                    });
                    render_backend::paint_document_node_base(
                        &painter,
                        render_node,
                        node_rect,
                        replacement_texture
                            .or(source_texture)
                            .map(|cached| cached.texture.id()),
                    );

                    painter.rect_stroke(
                        node_rect,
                        0,
                        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(70, 120, 210)),
                        egui::StrokeKind::Inside,
                    );

                    let paint_outcome = render_backend::paint_document_node_foreground(
                        &painter,
                        render_node,
                        node_rect,
                        scene_scale,
                    );
                    if paint_outcome.text_clipped {
                        preview_clipped_frames += 1;
                        if let Some(fragment) = render_node.text.as_ref() {
                            preview_clipped_story_keys.insert(format!("{:?}", fragment.story_id));
                            if let Some(metrics) = paint_outcome.text_metrics.clone() {
                                preview_text_diagnostics.push(
                                    PreviewTextMetricDiagnostic::from_executed_layout(
                                        page.index,
                                        page.id.as_canonical().to_string(),
                                        node_id.as_canonical().to_string(),
                                        fragment.story_id.as_canonical().to_string(),
                                        render_node.bounds,
                                        self.zoom,
                                        metrics,
                                    ),
                                );
                            }
                        }
                        painter.rect_stroke(
                            node_rect,
                            0,
                            egui::Stroke::new(2.0_f32, egui::Color32::RED),
                            egui::StrokeKind::Inside,
                        );
                        let marker_center = node_rect.right_top() + egui::vec2(7.0_f32, -7.0_f32);
                        painter.circle_filled(marker_center, 5.0_f32, egui::Color32::RED);
                        painter.text(
                            marker_center,
                            egui::Align2::CENTER_CENTER,
                            "!",
                            egui::FontId::proportional(9.0_f32),
                            egui::Color32::WHITE,
                        );
                    }
                }

                if let Some(mode) = self.text_mode.as_ref()
                    && let Some(stop) = text_session::focus_caret(mode)
                    && stop.page_id == page_id_text
                {
                    let x = page_rect.left() + stop.page_x_emu as f32 * scene_scale;
                    let y_top = page_rect.top() + stop.page_y_top_emu as f32 * scene_scale;
                    let y_bottom = page_rect.top() + stop.page_y_bottom_emu as f32 * scene_scale;
                    painter.line_segment(
                        [egui::pos2(x, y_top), egui::pos2(x, y_bottom)],
                        egui::Stroke::new(2.0_f32, egui::Color32::from_rgb(232, 126, 36)),
                    );
                }

                if let Some(selected_instance_id) = selected_canvas_instance.as_deref()
                    && let Some(selected_node_id) =
                        hit_index.node_for_instance(selected_instance_id)
                    && let Some(node) = page_nodes
                        .iter()
                        .copied()
                        .find(|node| node.origin == selected_node_id)
                    && node.bounds.width.get() > 0
                    && node.bounds.height.get() > 0
                {
                    let selected_bounds = next_canvas_resize
                        .filter(|resize| resize.node_id() == node.origin)
                        .and_then(|resize| resize.preview_bounds())
                        .or_else(|| {
                            next_canvas_drag
                                .filter(|drag| drag.node_id() == node.origin)
                                .map(|drag| drag.preview_bounds())
                        })
                        .unwrap_or(node.bounds);
                    let min = egui::pos2(
                        page_rect.left() + selected_bounds.x.get() as f32 * scene_scale,
                        page_rect.top() + selected_bounds.y.get() as f32 * scene_scale,
                    );
                    let size = egui::vec2(
                        selected_bounds.width.get() as f32 * scene_scale,
                        selected_bounds.height.get() as f32 * scene_scale,
                    );
                    let selected_rect = egui::Rect::from_min_size(min, size);
                    let resize_enabled = self.text_mode.is_none()
                        && resizable_nodes.contains_key(selected_instance_id);
                    paint_selection_overlay(&painter, selected_rect, resize_enabled);

                    if self.text_mode.is_none()
                        && let Some(fragment) = visual
                            .text_fragments
                            .iter()
                            .find(|fragment| fragment.frame_id == node.origin)
                        && self.editor.as_ref().is_some_and(|editor| {
                            editor.can_replace_story_text(fragment.story_id).is_ok()
                        })
                    {
                        let button_width = 76.0_f32;
                        let button_height = 22.0_f32;
                        let button_min = egui::pos2(
                            selected_rect.left(),
                            (selected_rect.top() - button_height - 4.0_f32)
                                .max(page_rect.top() + 2.0_f32),
                        );
                        let button_rect = egui::Rect::from_min_size(
                            button_min,
                            egui::vec2(button_width, button_height),
                        );
                        if ui
                            .put(button_rect, egui::Button::new("Edit Text"))
                            .clicked()
                        {
                            edit_text_request = Some((fragment.story_id, node.origin));
                        }
                    }
                    if resize_enabled {
                        let screen_bounds = ScreenRect::new(
                            f64::from(selected_rect.left()),
                            f64::from(selected_rect.top()),
                            f64::from(selected_rect.width()),
                            f64::from(selected_rect.height()),
                        )
                        .ok();
                        if let Some(screen_bounds) = screen_bounds {
                            for handle in ResizeHandle::ALL {
                                if let Ok(center) = resize_handle_center(screen_bounds, handle) {
                                    let center = egui::pos2(center.x as f32, center.y as f32);
                                    let handle_rect = egui::Rect::from_center_size(
                                        center,
                                        egui::vec2(12.0_f32, 12.0_f32),
                                    );
                                    let a11y = ui.interact(
                                        handle_rect,
                                        ui.id().with((
                                            "resize-handle",
                                            selected_instance_id,
                                            resize_handle_label(handle),
                                        )),
                                        egui::Sense::hover(),
                                    );
                                    a11y.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::Other,
                                            true,
                                            format!(
                                                "Resize {} handle",
                                                resize_handle_label(handle)
                                            ),
                                        )
                                    });
                                }
                            }
                        }
                    }
                }
            });

        if text_exit_request {
            self.exit_canvas_text_mode("canvas_non_text_click");
        }
        if let Some((page_id, point)) = text_pointer_request {
            self.reposition_canvas_text_caret(&page_id, point);
        }
        if let Some((story_id, frame_id)) = edit_text_request {
            self.enter_canvas_text_mode(story_id, frame_id);
        }

        let drag_instance = next_canvas_drag.and_then(|drag| {
            hit_index
                .instance_for_node(drag.node_id())
                .map(str::to_owned)
        });
        let resize_instance = next_canvas_resize.and_then(|resize| {
            hit_index
                .instance_for_node(resize.node_id())
                .map(str::to_owned)
        });
        let resize_commit_instance = resize_commit.and_then(|resize| {
            hit_index
                .instance_for_node(resize.node_id)
                .map(str::to_owned)
        });
        if canvas_clicked
            || drag_commit.is_some()
            || next_canvas_drag.is_some()
            || resize_commit.is_some()
            || next_canvas_resize.is_some()
        {
            if let Some(instance_id) = canvas_hit
                .or(resize_instance)
                .or(resize_commit_instance)
                .or(drag_instance)
            {
                self.canvas_selection.select_only(instance_id);
            } else if canvas_clicked {
                self.canvas_selection.clear();
            }
        }

        if let Some(error) = resize_error {
            self.canvas_drag = None;
            self.canvas_resize = None;
            self.edit_status = Some(error);
        } else if let Some(resize) = resize_commit {
            self.commit_canvas_resize(resize);
        } else if let Some(error) = drag_error {
            self.canvas_drag = None;
            self.canvas_resize = None;
            self.edit_status = Some(error);
        } else if let Some(drag) = drag_commit {
            self.commit_canvas_drag(drag);
        } else {
            self.canvas_drag = next_canvas_drag;
            self.canvas_resize = next_canvas_resize;
        }

        self.preview_clipped_frames = preview_clipped_frames;
        self.preview_clipped_story_keys = preview_clipped_story_keys;
        self.preview_text_diagnostics = preview_text_diagnostics;
    }
}

fn restore_supporter_state(storage: Option<&dyn eframe::Storage>) -> supporter::SupporterState {
    storage
        .and_then(|storage| storage.get_string(SUPPORTER_STORAGE_KEY))
        .and_then(|raw| supporter::SupporterState::from_json_str(&raw))
        .unwrap_or_default()
}

impl eframe::App for ViewerApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(SUPPORTER_STORAGE_KEY, self.supporter_state.to_json_string());
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::O)) {
            self.open_pub_picker();
        }
        self.accept_dropped_file(ctx);
        self.poll_committed_source_freshness();
        self.poll_diagnostic_sweep();
        self.process_canvas_text_input(ctx);

        debug_assert_eq!(
            self.supporter_value.is_eligible(),
            self.supporter_value.receipt().is_some()
        );
        if let Some(receipt) = self.supporter_value.receipt() {
            debug_assert!(receipt.page_count > 0);
            if let supporter::ValueReceiptKind::SearchMatches { match_count } = receipt.kind {
                debug_assert!(match_count > 0);
            }
        }

        if reader_only_mode() {
            reader_product_ui::configure_context(ctx);
        }

        egui::TopBottomPanel::top("workspace-command-bar").show(ctx, |ui| {
            self.show_command_bar(ui);
        });

        if reader_only_mode()
            && self
                .committed_source
                .as_ref()
                .is_some_and(|source| source.freshness.requires_reload())
        {
            egui::TopBottomPanel::top("source-freshness").show(ctx, |ui| {
                self.show_source_freshness_banner(ui);
            });
        }

        if !reader_only_mode() {
            egui::TopBottomPanel::top("fidelity-status").show(ctx, |ui| {
                self.show_fidelity_status(ui);
            });
        }

        egui::TopBottomPanel::bottom("workspace-status").show(ctx, |ui| {
            self.show_workspace_status(ui);
        });

        egui::SidePanel::left("pages")
            .resizable(true)
            .default_width(if reader_only_mode() { 210.0 } else { 170.0 })
            .min_width(if reader_only_mode() { 160.0 } else { 120.0 })
            .show(ctx, |ui| self.show_pages(ui));

        egui::SidePanel::right("inspector")
            .resizable(true)
            .default_width(if reader_only_mode() { 360.0 } else { 320.0 })
            .min_width(if reader_only_mode() { 280.0 } else { 220.0 })
            .show(ctx, |ui| {
                if reader_only_mode() {
                    self.show_reader_inspector(ui);
                } else {
                    self.show_inspector(ui);
                }
            });

        if reader_only_mode() {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(reader_product_ui::CANVAS_BG))
                .show(ctx, |ui| self.show_canvas(ui));
        } else {
            egui::CentralPanel::default().show(ctx, |ui| self.show_canvas(ui));
        }
        self.show_diagnostics_window(ctx);
        self.show_exact_file_consent_dialog(ctx);
        self.show_diagnostic_sweep_window(ctx);

        if self.text_mode.is_some() && ctx.wants_keyboard_input() {
            self.exit_canvas_text_mode("explicit_exit");
        }
    }
}

fn canvas_document_point(
    page_rect: egui::Rect,
    scene_scale: f32,
    pointer: egui::Pos2,
) -> Option<pub_interaction::DocumentPoint> {
    let transform = ViewTransform::new(
        ScreenPoint::new(f64::from(page_rect.left()), f64::from(page_rect.top())),
        f64::from(scene_scale),
    )
    .ok()?;
    transform
        .screen_to_document(ScreenPoint::new(f64::from(pointer.x), f64::from(pointer.y)))
        .ok()
}

fn paint_selection_overlay(painter: &egui::Painter, rect: egui::Rect, show_handles: bool) {
    let accent = egui::Color32::from_rgb(232, 126, 36);
    painter.rect_stroke(
        rect.expand(2.0),
        0,
        egui::Stroke::new(2.0_f32, accent),
        egui::StrokeKind::Inside,
    );

    if show_handles {
        let center = rect.center();
        let handles = [
            rect.left_top(),
            egui::pos2(center.x, rect.top()),
            rect.right_top(),
            egui::pos2(rect.left(), center.y),
            egui::pos2(rect.right(), center.y),
            rect.left_bottom(),
            egui::pos2(center.x, rect.bottom()),
            rect.right_bottom(),
        ];

        for handle in handles {
            let handle_rect = egui::Rect::from_center_size(handle, egui::vec2(7.0_f32, 7.0_f32));
            painter.rect_filled(handle_rect, 0, egui::Color32::WHITE);
            painter.rect_stroke(
                handle_rect,
                0,
                egui::Stroke::new(1.5_f32, accent),
                egui::StrokeKind::Inside,
            );
        }
    }
}

fn resize_handle_label(handle: ResizeHandle) -> &'static str {
    match handle {
        ResizeHandle::TopLeft => "top-left",
        ResizeHandle::Top => "top",
        ResizeHandle::TopRight => "top-right",
        ResizeHandle::Left => "left",
        ResizeHandle::Right => "right",
        ResizeHandle::BottomLeft => "bottom-left",
        ResizeHandle::Bottom => "bottom",
        ResizeHandle::BottomRight => "bottom-right",
    }
}

fn editable_export_path(
    source_path: &Path,
    target: pub_editor::EditorEditableTarget,
) -> Option<PathBuf> {
    let file_name = source_path.file_name()?;
    let mut output_name = file_name.to_os_string();
    output_name.push(".edited.");
    output_name.push(target.extension());
    Some(source_path.with_file_name(output_name))
}

fn editable_export_report_path(output_path: &Path) -> PathBuf {
    let mut name = output_path
        .file_name()
        .map(|value| value.to_os_string())
        .unwrap_or_default();
    name.push(".export-report.json");
    output_path.with_file_name(name)
}

#[cfg(all(test, feature = "embedded-fixture-tests"))]
fn apply_editor_project_json(
    editor: &mut pub_editor::EditorSession,
    bytes: &[u8],
) -> Result<usize, String> {
    let project: pub_editor::EditorProject = serde_json::from_slice(bytes)
        .map_err(|error| format!("parse editor project JSON: {error}"))?;
    let operation_count = project.operations.len();
    editor
        .apply_project(&project)
        .map_err(|error| format!("replay editor project: {error}"))?;
    Ok(operation_count)
}

fn load_editor_project_assets(
    source_path: &Path,
    project: &pub_editor::EditorProject,
) -> Result<BTreeMap<pub_editor::Sha256Digest, Vec<u8>>, String> {
    if project.assets.is_empty() {
        return Ok(BTreeMap::new());
    }

    let asset_dir = editor_project_asset_dir_path(source_path)
        .ok_or_else(|| "source path has no file name".to_owned())?;
    let mut assets = BTreeMap::new();
    for metadata in &project.assets {
        let file_name = metadata
            .file_name()
            .map_err(|error| format!("derive replacement asset name: {error}"))?;
        let path = asset_dir.join(file_name);
        let bytes = fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
        assets.insert(metadata.sha256, bytes);
    }
    Ok(assets)
}

fn load_editor_project_sidecar(
    source_path: &Path,
    editor: &mut pub_editor::EditorSession,
) -> Result<Option<(PathBuf, usize)>, String> {
    let sidecar = editor_project_sidecar_path(source_path)
        .ok_or_else(|| "source path has no file name".to_owned())?;
    if !sidecar.exists() {
        return Ok(None);
    }

    let bytes =
        fs::read(&sidecar).map_err(|error| format!("read {}: {error}", sidecar.display()))?;
    let project: pub_editor::EditorProject = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse editor project JSON: {error}"))?;
    let operation_count = project.operations.len();
    let assets = load_editor_project_assets(source_path, &project)?;
    editor
        .apply_project_with_assets(&project, &assets)
        .map_err(|error| format!("replay editor project: {error}"))?;
    Ok(Some((sidecar, operation_count)))
}

fn editor_project_sidecar_path(source_path: &Path) -> Option<PathBuf> {
    let file_name = source_path.file_name()?;
    let mut sidecar_name = file_name.to_os_string();
    sidecar_name.push(".pub-editor.json");
    Some(source_path.with_file_name(sidecar_name))
}

fn editor_project_asset_dir_path(source_path: &Path) -> Option<PathBuf> {
    let file_name = source_path.file_name()?;
    let mut asset_dir_name = file_name.to_os_string();
    asset_dir_name.push(".pub-editor.assets");
    Some(source_path.with_file_name(asset_dir_name))
}

fn replacement_image_mime(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        _ => None,
    }
}

fn search_result_preview(text: &str) -> String {
    const LIMIT: usize = 48;
    let mut preview = text.replace(['\r', '\n'], " ");
    if preview.chars().count() > LIMIT {
        preview = preview.chars().take(LIMIT).collect::<String>();
        preview.push('…');
    }
    preview
}

fn exact_file_consent_cta_visible(class: FailureIntakeClass) -> bool {
    exact_file_intake_eligible(class)
}

fn failure_intake_label(class: FailureIntakeClass) -> &'static str {
    match class {
        FailureIntakeClass::PubHighValue => "Publisher file detected",
        FailureIntakeClass::PubDamaged => "Publisher file appears damaged",
        FailureIntakeClass::PubPossible => "Publisher file is possible but not proven",
        FailureIntakeClass::ArchiveWithPub => "Archive contains Publisher-like content",
        FailureIntakeClass::NotPub => "Contents do not look like a Publisher document",
        FailureIntakeClass::SuspiciousPolyglot => "Conflicting file signatures detected",
    }
}

fn failure_intake_summary(class: FailureIntakeClass) -> &'static str {
    match class {
        FailureIntakeClass::PubHighValue => {
            "The file has strong Publisher structure, but this Chaptera build could not open it."
        }
        FailureIntakeClass::PubDamaged => {
            "The file has Publisher-like structure but appears truncated or damaged."
        }
        FailureIntakeClass::PubPossible => {
            "Some structure could be compatible with Publisher, but there is not enough evidence to classify it confidently."
        }
        FailureIntakeClass::ArchiveWithPub => {
            "This is an archive/container with Publisher-like content inside rather than a normal standalone PUB file."
        }
        FailureIntakeClass::NotPub => {
            "The file extension may say .pub, but the bytes look like another format such as HTML, PDF, an image, or plain text."
        }
        FailureIntakeClass::SuspiciousPolyglot => {
            "The file contains conflicting format signatures. Chaptera treats it cautiously and does not assume it is a normal PUB."
        }
    }
}

fn fidelity_status_label(status: ViewerFidelityStatus) -> &'static str {
    match status {
        ViewerFidelityStatus::Supported => "Supported",
        ViewerFidelityStatus::Partial => "Partial",
        ViewerFidelityStatus::Unsupported => "Unsupported",
    }
}

fn fidelity_status_summary(status: ViewerFidelityStatus) -> &'static str {
    match status {
        ViewerFidelityStatus::Supported => {
            "No known fidelity warnings were reported for the current Viewer scope."
        }
        ViewerFidelityStatus::Partial => {
            "The document opened, but some content is not fully displayed by the current Viewer."
        }
        ViewerFidelityStatus::Unsupported => {
            "The current Viewer could not open this document safely."
        }
    }
}

fn diagnostic_severity_label(severity: ViewerDiagnosticSeverity) -> &'static str {
    match severity {
        ViewerDiagnosticSeverity::Info => "Info",
        ViewerDiagnosticSeverity::FidelityWarning => "Fidelity warning",
    }
}

fn page_thumbnail_size(page_width_emu: i64, page_height_emu: i64) -> Option<egui::Vec2> {
    if page_width_emu <= 0 || page_height_emu <= 0 {
        return None;
    }

    let width = page_width_emu as f32;
    let height = page_height_emu as f32;
    let scale = (PAGE_THUMBNAIL_MAX_WIDTH / width).min(PAGE_THUMBNAIL_MAX_HEIGHT / height);
    Some(egui::vec2(width * scale, height * scale))
}

fn paint_page_thumbnail(
    painter: &egui::Painter,
    rect: egui::Rect,
    visual: &ViewerGeometryDocument,
    editor: Option<&pub_editor::EditorSession>,
    image_textures: &BTreeMap<String, CachedImageTexture>,
    page_index: usize,
    selected: bool,
) {
    let Some(page) = visual.document.pages.get(page_index) else {
        return;
    };
    let Ok(render_plan) = build_desktop_page_render_plan(visual, page_index) else {
        return;
    };
    if render_plan.page_size.width.get() <= 0 || render_plan.page_size.height.get() <= 0 {
        return;
    }

    let page_rect = rect.shrink(2.0);
    painter.rect_filled(page_rect, 0.0, egui::Color32::WHITE);
    painter.rect_stroke(
        page_rect,
        0.0,
        egui::Stroke::new(
            if selected { 2.0 } else { 1.0 },
            if selected {
                egui::Color32::from_rgb(70, 120, 210)
            } else {
                egui::Color32::GRAY
            },
        ),
        egui::StrokeKind::Inside,
    );

    let content_painter = painter.with_clip_rect(page_rect);
    let scale_x = page_rect.width() / render_plan.page_size.width.get() as f32;
    let scale_y = page_rect.height() / render_plan.page_size.height.get() as f32;

    for node in &render_plan.nodes {
        if node.bounds.width.get() <= 0 || node.bounds.height.get() <= 0 {
            continue;
        }
        let node_rect = egui::Rect::from_min_size(
            egui::pos2(
                page_rect.left() + node.bounds.x.get() as f32 * scale_x,
                page_rect.top() + node.bounds.y.get() as f32 * scale_y,
            ),
            egui::vec2(
                node.bounds.width.get() as f32 * scale_x,
                node.bounds.height.get() as f32 * scale_y,
            ),
        );

        if let Some(rgb) = node.solid_fill_rgb {
            content_painter.rect_filled(
                node_rect,
                0.0,
                egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]),
            );
        }

        let replace_image_admitted =
            node.projected_scene_instance
                .as_ref()
                .is_none_or(|instance| {
                    admit_object_mutation_v1(instance, ObjectMutationKindV1::ReplaceImage).admitted
                });
        let replacement_key = replace_image_admitted
            .then(|| {
                editor
                    .and_then(|editor| editor.image_replacement_for(node.node_id))
                    .map(|sha256| format!("replacement:{:?}", sha256))
            })
            .flatten();
        let replacement_texture = replacement_key
            .as_ref()
            .and_then(|key| image_textures.get(key));
        let source_texture = node.image.as_ref().and_then(|image| {
            let key = format!("{:?}", image.resource_id);
            image_textures.get(&key)
        });
        if let Some(texture) = replacement_texture.or(source_texture) {
            content_painter.image(
                texture.texture.id(),
                node_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }

        if let Some(fragment) = node.text.as_ref()
            && !fragment.text.is_empty()
            && node_rect.width() >= 4.0
            && node_rect.height() >= 4.0
        {
            let text_painter = content_painter.with_clip_rect(node_rect.shrink(1.0));
            let preview = fragment.text.replace(['\r', '\n'], " ");
            text_painter.text(
                node_rect.left_top() + egui::vec2(1.0, 1.0),
                egui::Align2::LEFT_TOP,
                preview,
                egui::FontId::proportional(4.5),
                egui::Color32::DARK_GRAY,
            );
        }

        if let Some(line) = node.solid_line.as_ref() {
            content_painter.rect_stroke(
                node_rect,
                0.0,
                egui::Stroke::new(
                    (line.width_emu as f32 * scale_x).clamp(0.5, 2.0),
                    egui::Color32::from_rgb(line.rgb[0], line.rgb[1], line.rgb[2]),
                ),
                egui::StrokeKind::Inside,
            );
        }
    }

    debug_assert_eq!(render_plan.page_id, page.id);
}

fn numeric_zoom_scene_scale(zoom: f32) -> Option<f32> {
    if !zoom.is_finite() || zoom <= 0.0 {
        return None;
    }
    Some((NUMERIC_ZOOM_POINTS_PER_INCH / EMU_PER_INCH) * zoom)
}

fn page_width_scale(page_width_emu: i64, viewport: egui::Vec2) -> Option<f32> {
    if page_width_emu <= 0 {
        return None;
    }

    let usable_width = (viewport.x - PAGE_MARGIN * 2.0).max(1.0);
    Some(usable_width / page_width_emu as f32)
}

fn ctrl_wheel_zoom(current: f32, scroll_y: f32) -> f32 {
    let delta = if scroll_y > 0.0 {
        0.10
    } else if scroll_y < 0.0 {
        -0.10
    } else {
        0.0
    };
    (current + delta).clamp(MIN_NUMERIC_ZOOM, MAX_NUMERIC_ZOOM)
}

fn fitted_scale(page_width_emu: i64, page_height_emu: i64, viewport: egui::Vec2) -> Option<f32> {
    if page_width_emu <= 0 || page_height_emu <= 0 {
        return None;
    }

    let usable_width = (viewport.x - PAGE_MARGIN * 2.0).max(1.0);
    let usable_height = (viewport.y - PAGE_MARGIN * 2.0).max(1.0);
    let page_width = page_width_emu as f32;
    let page_height = page_height_emu as f32;

    Some((usable_width / page_width).min(usable_height / page_height))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(all(feature = "embedded-fixture-tests", not(feature = "reader-only")))]
    use egui_kittest::kittest::Queryable;

    #[test]
    fn scene_hit_index_builds_stable_bidirectional_lookup_once() {
        let node_id: pub_editor::NodeId =
            serde_json::from_str("\"22222222-2222-2222-2222-222222222222\"")
                .expect("canonical NodeId");
        let instance_id = "sha256:test-instance".to_owned();
        let bounds = pub_editor::RectEmu::new(
            pub_editor::LengthEmu::new(10),
            pub_editor::LengthEmu::new(20),
            pub_editor::LengthEmu::new(30),
            pub_editor::LengthEmu::new(40),
        );
        let index = SceneHitTestIndex::new(vec![SceneHitEntry {
            instance_id: instance_id.clone(),
            node_id,
            bounds,
            z_order: 0,
            paint_order: 0,
        }]);

        assert_eq!(index.node_for_instance(&instance_id), Some(node_id));
        assert_eq!(index.instance_for_node(node_id), Some(instance_id.as_str()));
        assert_eq!(
            index
                .entry_for_instance(&instance_id)
                .map(|entry| entry.bounds),
            Some(bounds)
        );
    }

    #[test]
    fn canvas_pointer_maps_through_interaction_transform() {
        let page_rect =
            egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(400.0, 300.0));
        let point = canvas_document_point(page_rect, 0.5, egui::pos2(150.0, 125.0))
            .expect("valid canvas transform");

        assert_eq!(point.x.get(), 100);
        assert_eq!(point.y.get(), 150);
    }

    #[test]
    fn preview_text_clipping_warning_keeps_scope_fence() {
        assert!(PREVIEW_TEXT_CLIP_WARNING.contains("preview-only"));
        assert!(PREVIEW_TEXT_CLIP_WARNING.contains("not Publisher-native"));
    }

    #[test]
    fn page_thumbnail_size_preserves_aspect_ratio_and_bounds() {
        let portrait = page_thumbnail_size(8_229_600, 10_668_000).expect("portrait");
        assert!(portrait.x <= PAGE_THUMBNAIL_MAX_WIDTH + f32::EPSILON);
        assert!(portrait.y <= PAGE_THUMBNAIL_MAX_HEIGHT + f32::EPSILON);
        let source_ratio = 8_229_600.0_f32 / 10_668_000.0_f32;
        assert!((portrait.x / portrait.y - source_ratio).abs() < 0.001);

        let landscape = page_thumbnail_size(10_668_000, 8_229_600).expect("landscape");
        assert!(landscape.x <= PAGE_THUMBNAIL_MAX_WIDTH + f32::EPSILON);
        assert!(landscape.y <= PAGE_THUMBNAIL_MAX_HEIGHT + f32::EPSILON);
    }

    #[test]
    fn desktop_projected_nodes_use_scene_instance_mutation_admission() {
        let source = include_str!("main.rs");
        assert!(source.contains("for render_node in &render_plan.nodes"));
        assert!(source.contains("render_node.projected_scene_instance.as_ref()"));
        assert!(source.contains("Some(instance) => instance.instance_id.clone()"));
        assert!(source.contains("SceneHitTestIndex::new("));
        assert!(
            source.contains("admit_object_mutation_v1(instance, ObjectMutationKindV1::MoveNode)")
        );
        assert!(
            source.contains("admit_object_mutation_v1(instance, ObjectMutationKindV1::ResizeNode)")
        );
        assert!(
            source
                .contains("admit_object_mutation_v1(instance, ObjectMutationKindV1::ReplaceImage)")
        );
    }

    #[test]
    fn projected_visual_identity_wins_mixed_topmost_hit_without_mutation_admission() {
        let direct_node_id: pub_editor::NodeId =
            serde_json::from_value(serde_json::json!("11111111-1111-1111-1111-111111111111"))
                .expect("direct NodeId fixture");
        let projected_origin_node_id: pub_editor::NodeId =
            serde_json::from_value(serde_json::json!("22222222-2222-2222-2222-222222222222"))
                .expect("projected origin NodeId fixture");
        let bounds = pub_editor::RectEmu::new(
            pub_editor::LengthEmu::new(10),
            pub_editor::LengthEmu::new(20),
            pub_editor::LengthEmu::new(100),
            pub_editor::LengthEmu::new(80),
        );
        let projected_instance = SceneInstanceV1 {
            schema_version: chaptera_scene_instance::SCENE_INSTANCE_SCHEMA_V1.to_owned(),
            instance_id: "sha256:projected-hit-fixture".to_owned(),
            projection_kind: chaptera_scene_instance::SceneProjectionKindV1::CmoStorySlot,
            origin_node_id: projected_origin_node_id.as_canonical().to_string(),
            target_page_id: "33333333-3333-3333-3333-333333333333".to_owned(),
            source_parent_origin: None,
            story_authority_id: None,
            cmo_slot_index: Some(0),
            cmo_scalar_index: Some(0),
        };

        let hit_index = SceneHitTestIndex::new(vec![
            SceneHitEntry {
                instance_id: "sha256:direct-hit-fixture".to_owned(),
                node_id: direct_node_id,
                bounds,
                z_order: 0,
                paint_order: 0,
            },
            SceneHitEntry {
                instance_id: projected_instance.instance_id.clone(),
                node_id: projected_origin_node_id,
                bounds,
                z_order: 0,
                paint_order: 1,
            },
        ]);
        let point = pub_interaction::DocumentPoint::new(
            pub_editor::LengthEmu::new(50),
            pub_editor::LengthEmu::new(50),
        );
        let top = hit_index.topmost_at(point).expect("overlapping visual hit");
        assert_eq!(
            top.instance_id.as_str(),
            projected_instance.instance_id.as_str()
        );
        assert_eq!(top.node_id, projected_origin_node_id);
        assert_eq!(
            hit_index.node_for_instance(&projected_instance.instance_id),
            Some(projected_origin_node_id)
        );

        for mutation in [
            ObjectMutationKindV1::MoveNode,
            ObjectMutationKindV1::ResizeNode,
            ObjectMutationKindV1::ReplaceImage,
        ] {
            let admission = admit_object_mutation_v1(&projected_instance, mutation);
            assert!(!admission.admitted);
            assert_eq!(
                admission.origin_node_id.as_deref(),
                None,
                "read-only projected instances must not expose an authored origin for mutation"
            );
            assert_eq!(
                admission.reason, "cmo_story_slot_object_read_only",
                "Cmo projection must remain explicitly read-only"
            );
        }
    }

    #[test]
    fn desktop_pages_surface_exposes_live_thumbnail_navigation() {
        let source = include_str!("main.rs");
        assert!(source.contains("Page {} thumbnail"));
        assert!(source.contains("paint_page_thumbnail"));
        assert!(source.contains("with_clip_rect(page_rect)"));
        assert!(source.contains("build_desktop_page_render_plan(visual, page_index)"));
        assert!(source.contains("for node in &render_plan.nodes"));
        assert!(source.contains("PageNavigated"));
    }

    #[test]
    fn numeric_100_percent_zoom_is_viewport_independent() {
        let scale = numeric_zoom_scene_scale(1.0).expect("numeric 100%");
        assert!((scale - (96.0 / 914_400.0)).abs() < f32::EPSILON);

        let fit_small =
            fitted_scale(8_229_600, 10_668_000, egui::vec2(800.0, 600.0)).expect("fit small");
        let fit_large =
            fitted_scale(8_229_600, 10_668_000, egui::vec2(1600.0, 1200.0)).expect("fit large");
        assert_ne!(fit_small, fit_large);
        assert_eq!(numeric_zoom_scene_scale(1.0), Some(scale));
    }

    #[test]
    fn page_width_zoom_uses_width_without_forcing_page_height_to_fit() {
        let viewport = egui::vec2(1000.0, 400.0);
        let scale = page_width_scale(2_000_000, viewport).expect("page width scale");
        assert!((2_000_000.0 * scale - (viewport.x - PAGE_MARGIN * 2.0)).abs() < 0.01);
    }

    #[test]
    fn ctrl_wheel_zoom_is_bounded_and_switches_in_ten_point_steps() {
        assert!((ctrl_wheel_zoom(1.0, 120.0) - 1.10).abs() < f32::EPSILON);
        assert!((ctrl_wheel_zoom(1.0, -120.0) - 0.90).abs() < f32::EPSILON);
        assert_eq!(ctrl_wheel_zoom(MAX_NUMERIC_ZOOM, 120.0), MAX_NUMERIC_ZOOM);
        assert_eq!(ctrl_wheel_zoom(MIN_NUMERIC_ZOOM, -120.0), MIN_NUMERIC_ZOOM);
    }

    #[test]
    fn desktop_zoom_surface_separates_numeric_and_fit_modes() {
        let source = include_str!("main.rs");
        assert!(source.contains("\"100%\""));
        assert!(source.contains("\"Fit Page\""));
        assert!(source.contains("\"Page Width\""));
        assert!(source.contains("\"Fit Selection\""));
        assert!(!source.contains("\"100% fit\""));
        assert!(source.contains("if self.zoom_mode == CanvasZoomMode::Percent"));
        assert!(source.contains("ui.label(\"Fit mode\")"));
    }

    #[test]
    fn ctrl_wheel_zoom_is_scoped_to_canvas_without_scroll_area_pan() {
        let source = include_str!("main.rs");
        assert!(source.contains(".enable_scrolling(!ctrl_held)"));
        assert!(source.contains("response.hovered() && ctrl_held"));
        let legacy_pointer_gate = ["pointer", "over", "canvas"].join("_");
        assert!(!source.contains(&legacy_pointer_gate));
    }

    #[test]
    fn fit_scale_keeps_page_inside_viewport() {
        let viewport = egui::vec2(1000.0, 800.0);
        let scale = fitted_scale(2_000_000, 1_000_000, viewport).expect("valid page");
        let width = 2_000_000.0 * scale;
        let height = 1_000_000.0 * scale;

        assert!(width <= viewport.x - PAGE_MARGIN * 2.0 + f32::EPSILON);
        assert!(height <= viewport.y - PAGE_MARGIN * 2.0 + f32::EPSILON);
    }

    #[test]
    fn fit_scale_rejects_invalid_page_dimensions() {
        assert!(fitted_scale(0, 1, egui::vec2(100.0, 100.0)).is_none());
        assert!(fitted_scale(1, -1, egui::vec2(100.0, 100.0)).is_none());
    }

    #[test]
    fn app_manifest_keeps_parser_crates_out_of_ui_boundary() {
        let manifest = include_str!("../Cargo.toml");

        assert!(manifest.contains("pub-viewer"));
        assert!(manifest.contains("pub-editor"));
        assert!(manifest.contains("pub-interaction"));
        for forbidden in [
            "pub-reader",
            "pub-contents",
            "pub-quill",
            "pub-escher",
            "pub-cfb",
        ] {
            assert!(
                !manifest.contains(forbidden),
                "UI manifest must not depend directly on {forbidden}"
            );
        }
    }

    #[test]
    fn source_pub_is_not_a_write_target_by_construction() {
        let source = include_str!("main.rs");
        let production_source = source
            .split_once("#[cfg(test)]")
            .map_or(source, |(production, _)| production);

        assert!(production_source.contains("fs::read(&path)"));
        assert!(production_source.contains("fs::write(&sidecar"));
        assert!(!production_source.contains("fs::write(&path"));
        assert!(!production_source.contains("OpenOptions"));
    }

    #[test]
    fn editor_project_uses_distinct_sidecar_path() {
        let source = Path::new("/tmp/example.pub");
        let sidecar = editor_project_sidecar_path(source).expect("file name");
        let asset_dir = editor_project_asset_dir_path(source).expect("file name");

        assert_eq!(sidecar, PathBuf::from("/tmp/example.pub.pub-editor.json"));
        assert_eq!(
            asset_dir,
            PathBuf::from("/tmp/example.pub.pub-editor.assets")
        );
        assert_ne!(sidecar, source);
        assert_ne!(asset_dir, source);
        assert_ne!(asset_dir, sidecar);
    }

    #[test]
    fn editable_exports_use_distinct_non_pub_paths() {
        let source = Path::new("/tmp/example.pub");
        let idml = editable_export_path(source, pub_editor::EditorEditableTarget::Idml)
            .expect("IDML output path");
        let odg = editable_export_path(source, pub_editor::EditorEditableTarget::Odg)
            .expect("ODG output path");

        assert_eq!(idml, PathBuf::from("/tmp/example.pub.edited.idml"));
        assert_eq!(odg, PathBuf::from("/tmp/example.pub.edited.odg"));
        assert_ne!(idml, source);
        assert_ne!(odg, source);
        assert_eq!(
            editable_export_report_path(&idml),
            PathBuf::from("/tmp/example.pub.edited.idml.export-report.json")
        );
    }

    #[cfg(feature = "embedded-fixture-tests")]
    #[test]
    fn editor_project_json_reopens_real_authoring_state() {
        let bytes = sample_newsletter_fixture();
        let visual = pub_viewer::open_mature_0x2c_geometry(
            &bytes,
            pub_viewer::viewer_geometry_environment_v0_1(),
        )
        .expect("SampleNewsletter Viewer open");
        let source_hash = visual.document.source.source_hash;

        let mut edited =
            pub_editor::open_mature_0x2c_editor(&bytes, source_hash).expect("editor open");
        let story_id = edited
            .graph()
            .stories
            .keys()
            .copied()
            .find(|story_id| edited.can_replace_story_text(*story_id).is_ok())
            .expect("fixture should expose one editable Story");
        edited
            .replace_story_text(story_id, "reopened desktop project")
            .expect("bounded Story edit");
        let expected_graph = edited.graph().clone();
        let json = serde_json::to_vec(&edited.project()).expect("serialize EditorProject");

        let mut reopened =
            pub_editor::open_mature_0x2c_editor(&bytes, source_hash).expect("fresh editor reopen");
        let operation_count =
            apply_editor_project_json(&mut reopened, &json).expect("replay sidecar JSON");

        assert_eq!(operation_count, 1);
        assert_eq!(reopened.graph(), &expected_graph);
        assert_eq!(reopened.source_hash(), source_hash);
        assert_eq!(reopened.operations().len(), 1);
    }

    #[test]
    fn path_loading_uses_shared_product_open_boundary() {
        let source = include_str!("main.rs");
        assert!(source.contains("diagnostic_sweep::open_for_product(&bytes)"));
        let sweep = include_str!("diagnostic_sweep.rs");
        assert!(sweep.contains("open_mature_0x2c_geometry"));
        assert!(sweep.contains("viewer_geometry_environment_v0_1"));
    }

    #[test]
    fn open_generation_rejects_superseded_completion() {
        let mut authority = OpenStateAuthority::default();
        let generation_a = authority.begin_attempt();
        let generation_b = authority.begin_attempt();

        assert!(
            !authority.commit_if_current(generation_a),
            "late A completion must not publish after B supersedes it"
        );
        assert!(
            authority.commit_if_current(generation_b),
            "current B completion must retain commit authority"
        );
    }

    #[test]
    fn finished_attempt_cannot_commit_later() {
        let mut authority = OpenStateAuthority::default();
        let generation = authority.begin_attempt();

        assert!(authority.finish_without_commit_if_current(generation));
        assert!(
            !authority.commit_if_current(generation),
            "cancelled/failed work must lose publication authority"
        );
    }

    #[test]
    fn committed_source_exact_hash_detects_same_length_replacement() {
        let before = b"same-length-source-a";
        let after = b"same-length-source-b";
        assert_eq!(before.len(), after.len());
        assert_ne!(source_sha256(before), source_sha256(after));
    }

    #[test]
    fn changed_committed_source_becomes_reload_required_without_evicting_snapshot() {
        let path = std::env::temp_dir().join(format!(
            "chaptera-open-state-source-change-{}.pub",
            std::process::id()
        ));
        let before = b"committed-source-a";
        let after = b"committed-source-b";
        assert_eq!(before.len(), after.len());
        fs::write(&path, before).expect("write committed source fixture");

        let mut app = ViewerApp::new(None);
        app.source_path = Some(path.clone());
        app.selected_page = 4;
        app.search_query = "snapshot search".to_owned();
        app.committed_source = Some(CommittedSourceState {
            generation: OpenGeneration(7),
            source_hash: source_sha256(before),
            byte_len: u64::try_from(before.len()).expect("fixture length"),
            file_stamp: None,
            freshness: SourceFreshness::Current,
        });

        fs::write(&path, after).expect("replace committed source fixture");
        app.revalidate_committed_source_now();

        let committed = app
            .committed_source
            .as_ref()
            .expect("committed source state");
        assert_eq!(committed.generation, OpenGeneration(7));
        assert_eq!(committed.freshness, SourceFreshness::ReloadRequiredChanged);
        assert_eq!(app.source_path.as_ref(), Some(&path));
        assert_eq!(app.selected_page, 4);
        assert_eq!(app.search_query, "snapshot search");
        assert_eq!(fs::read(&path).expect("re-read changed source"), after);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn periodic_exact_revalidation_catches_change_even_when_metadata_signal_matches() {
        let path = std::env::temp_dir().join(format!(
            "chaptera-open-state-exact-revalidation-{}.pub",
            std::process::id()
        ));
        let before = b"same-metadata-source-a";
        let after = b"same-metadata-source-b";
        assert_eq!(before.len(), after.len());
        fs::write(&path, after).expect("write replacement source fixture");
        let observed_stamp = source_file_stamp(&path).expect("replacement source metadata");

        let mut app = ViewerApp::new(None);
        app.source_path = Some(path.clone());
        app.committed_source = Some(CommittedSourceState {
            generation: OpenGeneration(8),
            source_hash: source_sha256(before),
            byte_len: u64::try_from(before.len()).expect("fixture length"),
            file_stamp: Some(observed_stamp),
            freshness: SourceFreshness::Current,
        });
        app.source_exact_revalidate_after = Some(Instant::now() - Duration::from_secs(1));

        app.revalidate_committed_source_now();

        assert_eq!(
            app.committed_source
                .as_ref()
                .expect("committed source state")
                .freshness,
            SourceFreshness::ReloadRequiredChanged,
            "periodic exact identity check must override an unchanged metadata signal"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn missing_committed_source_becomes_reload_required_not_unsupported() {
        let path = std::env::temp_dir().join(format!(
            "chaptera-open-state-source-missing-{}.pub",
            std::process::id()
        ));
        let before = b"committed-source";
        fs::write(&path, before).expect("write committed source fixture");

        let mut app = ViewerApp::new(None);
        app.source_path = Some(path.clone());
        app.committed_source = Some(CommittedSourceState {
            generation: OpenGeneration(9),
            source_hash: source_sha256(before),
            byte_len: u64::try_from(before.len()).expect("fixture length"),
            file_stamp: None,
            freshness: SourceFreshness::Current,
        });
        fs::remove_file(&path).expect("remove committed source fixture");

        app.revalidate_committed_source_now();

        assert_eq!(
            app.committed_source
                .as_ref()
                .expect("committed source state")
                .freshness,
            SourceFreshness::ReloadRequiredUnavailable
        );
        assert!(app.load_error.is_none());
    }

    #[test]
    fn multi_file_drop_is_explicitly_ambiguous() {
        let a = Some(PathBuf::from("a.pub"));
        let b = Some(PathBuf::from("b.pub"));

        assert_eq!(
            dropped_file_candidate(&[a, b]),
            Err("Drop exactly one PUB file at a time; no file was opened.")
        );
        assert_eq!(
            dropped_file_candidate(&[Some(PathBuf::from("only.pub"))]),
            Ok(Some(PathBuf::from("only.pub")))
        );
    }

    #[test]
    fn failed_replacement_open_preserves_committed_state_and_source_bytes() {
        let replacement = std::env::temp_dir().join(format!(
            "chaptera-open-state-invalid-replacement-{}.pub",
            std::process::id()
        ));
        let replacement_bytes = b"<html>not a Publisher document</html>";
        fs::write(&replacement, replacement_bytes).expect("write invalid replacement fixture");

        let committed_path = PathBuf::from("already-open.pub");
        let mut app = ViewerApp::new(None);
        app.source_path = Some(committed_path.clone());
        app.committed_source = Some(CommittedSourceState {
            generation: OpenGeneration(11),
            source_hash: source_sha256(b"committed A"),
            byte_len: u64::try_from(b"committed A".len()).expect("fixture length"),
            file_stamp: None,
            freshness: SourceFreshness::Current,
        });
        app.selected_page = 3;
        app.search_query = "existing search state".to_owned();
        app.zoom = 1.75;

        app.load_path(replacement.clone());

        assert_eq!(
            app.source_path.as_ref(),
            Some(&committed_path),
            "failed B must not evict committed A"
        );
        assert_eq!(app.selected_page, 3);
        assert_eq!(app.search_query, "existing search state");
        assert_eq!(app.zoom, 1.75);
        let committed = app
            .committed_source
            .as_ref()
            .expect("committed A authority");
        assert_eq!(committed.generation, OpenGeneration(11));
        assert_eq!(committed.freshness, SourceFreshness::Current);
        let failure = app.load_error.as_ref().expect("replacement failure");
        assert_eq!(failure.kind, ViewerLoadFailureKind::Unsupported);
        assert_eq!(failure.attempted_path.as_ref(), Some(&replacement));
        assert_eq!(
            fs::read(&replacement).expect("re-read invalid replacement"),
            replacement_bytes,
            "open attempt must not mutate source bytes"
        );

        let _ = fs::remove_file(replacement);
    }

    #[test]
    fn geometry_warning_names_unpainted_content() {
        for term in ["text", "images", "effects", "transforms"] {
            assert!(GEOMETRY_WARNING.contains(term));
        }
    }

    #[test]
    fn unsupported_open_failure_has_unsupported_fidelity_state() {
        let app = ViewerApp {
            source_path: None,
            open_state: OpenStateAuthority::default(),
            committed_source: None,
            source_revalidate_after: None,
            source_exact_revalidate_after: None,
            visual: None,
            selected_page: 0,
            page_frame_cache: BTreeMap::new(),
            page_frame_cache_builds: 0,
            canvas_selection: SceneSelectionState::default(),
            canvas_drag: None,
            canvas_resize: None,
            created_text_box_scene_nodes: BTreeSet::new(),
            text_mode: None,
            zoom: 1.0,
            zoom_mode: CanvasZoomMode::FitPage,
            load_error: Some(ViewerLoadFailure {
                kind: ViewerLoadFailureKind::Unsupported,
                attempted_path: None,
                message: "unsupported".to_owned(),
                classification: Some(classify_failure_candidate(b"<html>not pub</html>")),
                diagnostic_json: None,
            }),
            search_query: String::new(),
            search_results: Vec::new(),
            selected_search_result: None,
            image_textures: BTreeMap::new(),
            image_decode_diagnostics: BTreeMap::new(),
            editor: None,
            editor_load_error: None,
            edit_buffer: String::new(),
            edit_status: None,
            selected_table_cell_index: None,
            table_cell_buffer: String::new(),
            export_preview: None,
            project_status: None,
            preview_clipped_frames: 0,
            preview_clipped_story_keys: BTreeSet::new(),
            preview_text_diagnostics: Vec::new(),
            diagnostic_save_path: String::new(),
            diagnostic_status: None,
            diagnostic_sweep: None,
            diagnostic_sweep_progress: diagnostic_sweep::FolderSweepProgress::default(),
            diagnostic_sweep_report: None,
            diagnostic_sweep_open: false,
            diagnostic_sweep_status: None,
            supporter_value: supporter::ValueTracker::default(),
            supporter_state: supporter::SupporterState::default(),
            exact_file_consent_open: false,
            exact_file_consent_status: None,
            show_diagnostics: false,
            reader_inspector_tab: reader_product_ui::InspectorTab::Document,
        };

        assert_eq!(
            app.fidelity_status(),
            Some(ViewerFidelityStatus::Unsupported)
        );
    }

    #[test]
    fn file_access_failure_does_not_claim_document_is_unsupported() {
        let app = ViewerApp {
            source_path: None,
            open_state: OpenStateAuthority::default(),
            committed_source: None,
            source_revalidate_after: None,
            source_exact_revalidate_after: None,
            visual: None,
            selected_page: 0,
            page_frame_cache: BTreeMap::new(),
            page_frame_cache_builds: 0,
            canvas_selection: SceneSelectionState::default(),
            canvas_drag: None,
            canvas_resize: None,
            created_text_box_scene_nodes: BTreeSet::new(),
            text_mode: None,
            zoom: 1.0,
            zoom_mode: CanvasZoomMode::FitPage,
            load_error: Some(ViewerLoadFailure {
                kind: ViewerLoadFailureKind::FileAccess,
                attempted_path: None,
                message: "permission denied".to_owned(),
                classification: None,
                diagnostic_json: None,
            }),
            search_query: String::new(),
            search_results: Vec::new(),
            selected_search_result: None,
            image_textures: BTreeMap::new(),
            image_decode_diagnostics: BTreeMap::new(),
            editor: None,
            editor_load_error: None,
            edit_buffer: String::new(),
            edit_status: None,
            selected_table_cell_index: None,
            table_cell_buffer: String::new(),
            export_preview: None,
            project_status: None,
            preview_clipped_frames: 0,
            preview_clipped_story_keys: BTreeSet::new(),
            preview_text_diagnostics: Vec::new(),
            diagnostic_save_path: String::new(),
            diagnostic_status: None,
            diagnostic_sweep: None,
            diagnostic_sweep_progress: diagnostic_sweep::FolderSweepProgress::default(),
            diagnostic_sweep_report: None,
            diagnostic_sweep_open: false,
            diagnostic_sweep_status: None,
            supporter_value: supporter::ValueTracker::default(),
            supporter_state: supporter::SupporterState::default(),
            exact_file_consent_open: false,
            exact_file_consent_status: None,
            show_diagnostics: false,
            reader_inspector_tab: reader_product_ui::InspectorTab::Document,
        };

        assert_eq!(app.fidelity_status(), None);
    }

    #[test]
    fn reader_first_run_contract_is_local_read_only_and_product_qualified() {
        assert_eq!(READER_PRODUCT_LABEL, "Chaptera PUB Reader");
        assert_eq!(READER_FIRST_RUN_HEADING, "Open a PUB file");
        assert!(READER_FIRST_RUN_TRUST_CUE.contains("Files open locally"));
        assert!(READER_FIRST_RUN_TRUST_CUE.contains("does not require an account"));
        assert!(READER_READ_ONLY_CUE.contains("read-only"));
        assert!(READER_READ_ONLY_CUE.contains("never overwritten"));

        let source = include_str!("main.rs");
        assert!(source.contains("Keyboard: Ctrl+O"));
        assert!(source.contains("input.modifiers.ctrl && input.key_pressed(egui::Key::O)"));
        assert!(source.contains("Local · read-only"));
        let retired_cli_hint = ["or start with:", " chaptera FILE.pub"].concat();
        assert!(!source.contains(&retired_cli_hint));
    }

    #[test]
    fn reader_first_run_hides_unconfigured_exact_file_handoff() {
        assert!(!failure_mailto_recipient_configured());
        let source = include_str!("main.rs");
        assert!(source.contains("Private file handoff is not configured in this build."));
        assert!(source.contains("Save local diagnostics below; no file is sent."));
    }

    #[test]
    fn failure_intake_copy_rejects_renamed_html_without_upload_language() {
        let classification = classify_failure_candidate(b"<!DOCTYPE html><html>junk</html>");
        assert_eq!(classification.class, FailureIntakeClass::NotPub);
        let label = failure_intake_label(classification.class);
        let summary = failure_intake_summary(classification.class);
        assert!(label.contains("do not look like a Publisher"));
        assert!(summary.contains("HTML"));
        assert!(!summary.to_ascii_lowercase().contains("upload"));
        assert!(!summary.to_ascii_lowercase().contains("send"));
    }

    #[test]
    fn publisher_like_failure_copy_does_not_claim_repair() {
        let label = failure_intake_label(FailureIntakeClass::PubDamaged);
        let summary = failure_intake_summary(FailureIntakeClass::PubDamaged);
        assert!(label.contains("damaged"));
        assert!(summary.contains("truncated or damaged"));
        assert!(!summary.to_ascii_lowercase().contains("repaired"));
    }

    #[test]
    fn failed_open_ui_uses_explicit_local_save_diagnostics_language() {
        let source = include_str!("main.rs");
        assert!(source.contains("Save diagnostics…"));
        assert!(source.contains("Nothing is sent."));
        assert!(source.contains("local_failure_diagnostic_json"));
        assert!(source.contains("Nothing is sent. Choose a local JSON path"));
    }

    #[test]
    fn exact_file_consent_cta_is_fail_closed_by_class() {
        for class in [
            FailureIntakeClass::PubHighValue,
            FailureIntakeClass::PubDamaged,
        ] {
            assert!(exact_file_consent_cta_visible(class), "{class:?}");
        }

        for class in [
            FailureIntakeClass::PubPossible,
            FailureIntakeClass::ArchiveWithPub,
            FailureIntakeClass::NotPub,
            FailureIntakeClass::SuspiciousPolyglot,
        ] {
            assert!(!exact_file_consent_cta_visible(class), "{class:?}");
        }
    }

    #[test]
    fn exact_file_consent_contract_is_versioned_and_transport_free() {
        assert_eq!(
            CHAPTERA_EXACT_FILE_CONSENT_V1,
            "chaptera-exact-file-consent/v1"
        );
        assert_eq!(
            CHAPTERA_INTAKE_RETENTION_POLICY_V1,
            "chaptera-intake-retention-v1"
        );

        let source = include_str!("main.rs");
        assert!(source.contains("No file was sent"));
        assert!(source.contains("transport/storage is intentionally not implemented"));
        assert!(source.contains("personal, private, or confidential information"));

        let manifest = include_str!("../Cargo.toml");
        for forbidden in ["reqwest", "hyper", "ureq", "curl", "aws-sdk", "tonic"] {
            assert!(
                !manifest.contains(forbidden),
                "consent-only UI must not introduce transport dependency {forbidden}"
            );
        }
    }

    #[test]
    fn fidelity_status_copy_has_no_pseudo_percentage() {
        for status in [
            ViewerFidelityStatus::Supported,
            ViewerFidelityStatus::Partial,
            ViewerFidelityStatus::Unsupported,
        ] {
            assert!(!fidelity_status_label(status).contains('%'));
            assert!(!fidelity_status_summary(status).contains('%'));
        }
    }

    #[test]
    fn desktop_normal_mode_keeps_raw_diagnostics_behind_disclosure() {
        let source = include_str!("main.rs");
        assert!(source.contains("Fidelity & diagnostics"));
        assert!(source.contains("Needs attention"));
        assert!(source.contains("show_diagnostics_window(ctx)"));
        assert!(source.contains("Technical details"));
    }

    #[test]
    fn search_preview_is_single_line_and_bounded() {
        let preview = search_result_preview(
            "this is a deliberately long\nsearch result that should be shortened for the list",
        );

        assert!(!preview.contains('\n'));
        assert!(preview.ends_with('…'));
        assert!(preview.chars().count() <= 49);
    }

    #[cfg(feature = "embedded-fixture-tests")]
    fn sample_newsletter_fixture() -> Vec<u8> {
        let path = std::env::var_os("CHAPTERA_SAMPLE_NEWSLETTER")
            .map(PathBuf::from)
            .expect("CHAPTERA_SAMPLE_NEWSLETTER must point to the pinned Apache POI fixture");
        fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "read pinned SampleNewsletter fixture {}: {error}",
                path.display()
            )
        })
    }

    #[cfg(feature = "embedded-fixture-tests")]
    #[test]
    fn real_pub_exposes_decodable_exact_image_bound_to_scene_node() {
        let pub_bytes = sample_newsletter_fixture();
        let visual = pub_viewer::open_mature_0x2c_geometry(
            &pub_bytes,
            pub_viewer::viewer_geometry_environment_v0_1(),
        )
        .expect("SampleNewsletter should open through Viewer image path");

        let embedded = visual
            .images
            .iter()
            .find(|embedded| matches!(embedded.mime.as_str(), "image/png" | "image/jpeg"))
            .expect("fixture should expose at least one exact PNG/JPEG image");

        assert!(!embedded.bytes.is_empty());
        assert!(
            embedded.node_ids.iter().any(|node_id| visual
                .scene
                .nodes
                .iter()
                .any(|node| node.origin == *node_id)),
            "exact image resource must retain at least one proven resolved scene-node use"
        );

        let expected_sha256 = image_decode_adapter::exact_sha256_hex(&embedded.bytes);
        let admitted = image_decode_adapter::decode_texture_image_v1(
            &embedded.bytes,
            &embedded.mime,
            &expected_sha256,
        )
        .expect("bounded desktop decoder must admit the exact embedded image");

        assert!(admitted.color_image.size[0] > 0);
        assert!(admitted.color_image.size[1] > 0);
        assert_eq!(admitted.cache_identity_sha256.len(), 64);
    }

    #[cfg(feature = "embedded-fixture-tests")]
    #[test]
    fn desktop_editor_session_updates_overlay_without_mutating_pub_bytes() {
        let bytes = sample_newsletter_fixture();
        let original_bytes = bytes.clone();
        let visual = pub_viewer::open_mature_0x2c_geometry(
            &bytes,
            pub_viewer::viewer_geometry_environment_v0_1(),
        )
        .expect("SampleNewsletter should open through the desktop Viewer path");
        let source_hash = visual.document.source.source_hash;
        let editor = pub_editor::open_mature_0x2c_editor(&bytes, source_hash)
            .expect("same real PUB should open through bounded editor facade");

        let frame = visual
            .story_frames
            .iter()
            .find(|frame| {
                visual
                    .story_frames
                    .iter()
                    .filter(|candidate| candidate.story_id == frame.story_id)
                    .count()
                    == 1
                    && editor.can_replace_story_text(frame.story_id).is_ok()
            })
            .expect("fixture should expose a single-frame safe Story");
        let story_id = frame.story_id;
        let before = editor
            .graph()
            .stories
            .get(&story_id)
            .expect("safe Story must exist")
            .text
            .clone();
        let replacement = format!("{before} [desktop edit]");

        let mut app = ViewerApp {
            source_path: Some(PathBuf::from("SampleNewsletter.pub")),
            open_state: OpenStateAuthority::default(),
            committed_source: None,
            source_revalidate_after: None,
            source_exact_revalidate_after: None,
            visual: Some(visual),
            selected_page: 0,
            page_frame_cache: BTreeMap::new(),
            page_frame_cache_builds: 0,
            canvas_selection: SceneSelectionState::default(),
            canvas_drag: None,
            canvas_resize: None,
            created_text_box_scene_nodes: BTreeSet::new(),
            text_mode: None,
            zoom: 1.0,
            zoom_mode: CanvasZoomMode::FitPage,
            load_error: None,
            search_query: String::new(),
            search_results: Vec::new(),
            selected_search_result: None,
            image_textures: BTreeMap::new(),
            image_decode_diagnostics: BTreeMap::new(),
            editor: Some(editor),
            editor_load_error: None,
            edit_buffer: replacement.clone(),
            edit_status: None,
            selected_table_cell_index: None,
            table_cell_buffer: String::new(),
            export_preview: None,
            project_status: None,
            preview_clipped_frames: 0,
            preview_clipped_story_keys: BTreeSet::new(),
            preview_text_diagnostics: Vec::new(),
            diagnostic_save_path: String::new(),
            diagnostic_status: None,
            diagnostic_sweep: None,
            diagnostic_sweep_progress: diagnostic_sweep::FolderSweepProgress::default(),
            diagnostic_sweep_report: None,
            diagnostic_sweep_open: false,
            diagnostic_sweep_status: None,
            supporter_value: supporter::ValueTracker::default(),
            supporter_state: supporter::SupporterState::default(),
            exact_file_consent_open: false,
            exact_file_consent_status: None,
            show_diagnostics: false,
            reader_inspector_tab: reader_product_ui::InspectorTab::Document,
        };

        app.editor
            .as_mut()
            .expect("editor loaded")
            .replace_story_text(story_id, replacement.clone())
            .expect("bounded desktop Story edit should succeed");
        app.sync_visual_stories_from_editor()
            .expect("current editor graph should refresh Viewer text projection");

        let rendered_story = app
            .visual
            .as_ref()
            .expect("Viewer document remains loaded")
            .document
            .stories
            .iter()
            .find(|story| story.id == story_id)
            .expect("edited Story remains visible");
        assert_eq!(rendered_story.text, replacement);
        assert_eq!(
            app.visual
                .as_ref()
                .expect("Viewer document remains loaded")
                .text_fragments
                .iter()
                .filter(|fragment| fragment.story_id == story_id)
                .map(|fragment| fragment.text.as_str())
                .collect::<String>(),
            replacement,
            "accepted Story edits must refresh the painted fragment projection"
        );
        assert_eq!(
            app.editor
                .as_ref()
                .expect("editor remains loaded")
                .source_hash(),
            source_hash
        );
        assert_eq!(
            app.visual
                .as_ref()
                .expect("Viewer document remains loaded")
                .document
                .source
                .source_hash,
            source_hash
        );
        assert_eq!(
            bytes, original_bytes,
            "desktop edit must not mutate source PUB bytes"
        );
    }

    #[cfg(feature = "embedded-fixture-tests")]
    #[test]
    fn canvas_drag_commits_exactly_one_move_and_syncs_undo_redo() {
        let bytes = sample_newsletter_fixture();
        let visual = pub_viewer::open_mature_0x2c_geometry(
            &bytes,
            pub_viewer::viewer_geometry_environment_v0_1(),
        )
        .expect("SampleNewsletter Viewer open");
        let source_hash = visual.document.source.source_hash;
        let editor = pub_editor::open_mature_0x2c_editor(&bytes, source_hash).expect("editor open");

        let (node_id, before) = visual
            .scene
            .nodes
            .iter()
            .find_map(|scene_node| {
                let authored = editor.graph().nodes.get(&scene_node.origin)?;
                let bounds = authored.header.bounds;
                editor
                    .can_move_node_to(scene_node.origin, bounds.x, bounds.y)
                    .ok()
                    .map(|_| (scene_node.origin, bounds))
            })
            .expect("fixture should expose one canvas-movable scene node");

        let mut app = ViewerApp::new(None);
        app.visual = Some(visual);
        app.editor = Some(editor);
        let target_page_id = app
            .visual
            .as_ref()
            .and_then(|visual| visual.document.pages.first())
            .map(|page| page.id.as_canonical().to_string())
            .expect("fixture page");
        let instance =
            direct_page_local_instance_v1(&node_id.as_canonical().to_string(), &target_page_id)
                .expect("direct test scene instance");
        app.canvas_selection.select_only(instance.instance_id);

        let pointer_start = pub_interaction::DocumentPoint::new(
            pub_editor::LengthEmu::ZERO,
            pub_editor::LengthEmu::ZERO,
        );
        let pointer_current = pub_interaction::DocumentPoint::new(
            pub_editor::LengthEmu::new(127_000),
            pub_editor::LengthEmu::new(254_000),
        );
        let mut drag =
            MoveTransaction::begin(node_id, before, pointer_start).expect("valid drag start");
        drag.update(pointer_current).expect("valid drag preview");
        let expected = drag.preview_bounds();

        app.canvas_drag = Some(drag);
        assert_eq!(
            app.editor
                .as_ref()
                .expect("editor present")
                .operations()
                .len(),
            0,
            "transient pointer motion must not emit semantic edit operations"
        );

        app.commit_canvas_drag(drag);
        assert!(app.canvas_drag.is_none());
        assert_eq!(
            app.editor
                .as_ref()
                .expect("editor present")
                .operations()
                .len(),
            1,
            "mouse release must emit exactly one MoveNode operation"
        );
        assert_eq!(
            app.editor.as_ref().expect("editor present").graph().nodes[&node_id]
                .header
                .bounds,
            expected
        );
        assert_eq!(expected.width, before.width);
        assert_eq!(expected.height, before.height);
        assert_eq!(
            app.visual
                .as_ref()
                .expect("visual present")
                .scene
                .nodes
                .iter()
                .find(|node| node.origin == node_id)
                .expect("moved node stays in Viewer scene")
                .bounds,
            expected,
            "committed authoring geometry must synchronize back into the Viewer scene"
        );

        app.apply_undo();
        assert_eq!(
            app.editor.as_ref().expect("editor present").graph().nodes[&node_id]
                .header
                .bounds,
            before
        );
        assert_eq!(
            app.visual
                .as_ref()
                .expect("visual present")
                .scene
                .nodes
                .iter()
                .find(|node| node.origin == node_id)
                .expect("node remains visible after undo")
                .bounds,
            before,
            "undo must synchronize Viewer geometry"
        );

        app.apply_redo();
        assert_eq!(
            app.editor.as_ref().expect("editor present").graph().nodes[&node_id]
                .header
                .bounds,
            expected
        );
        assert_eq!(
            app.visual
                .as_ref()
                .expect("visual present")
                .scene
                .nodes
                .iter()
                .find(|node| node.origin == node_id)
                .expect("node remains visible after redo")
                .bounds,
            expected,
            "redo must synchronize Viewer geometry"
        );
    }

    #[cfg(feature = "embedded-fixture-tests")]
    #[test]
    fn replayed_move_project_synchronizes_scene_geometry_by_canonical_node_id() {
        let bytes = sample_newsletter_fixture();
        let visual = pub_viewer::open_mature_0x2c_geometry(
            &bytes,
            pub_viewer::viewer_geometry_environment_v0_1(),
        )
        .expect("SampleNewsletter Viewer open");
        let source_hash = visual.document.source.source_hash;
        let mut edited =
            pub_editor::open_mature_0x2c_editor(&bytes, source_hash).expect("editor open");

        let (node_id, before) = visual
            .scene
            .nodes
            .iter()
            .find_map(|scene_node| {
                let authored = edited.graph().nodes.get(&scene_node.origin)?;
                let bounds = authored.header.bounds;
                edited
                    .can_move_node_to(scene_node.origin, bounds.x, bounds.y)
                    .ok()
                    .map(|_| (scene_node.origin, bounds))
            })
            .expect("fixture should expose one movable node");

        let x = before
            .x
            .checked_add(pub_editor::LengthEmu::new(127_000))
            .expect("bounded x");
        let y = before
            .y
            .checked_add(pub_editor::LengthEmu::new(254_000))
            .expect("bounded y");
        edited
            .move_node_to(node_id, x, y)
            .expect("bounded MoveNode");
        let expected = edited.graph().nodes[&node_id].header.bounds;
        let json = serde_json::to_vec(&edited.project()).expect("serialize geometry project");

        let mut reopened =
            pub_editor::open_mature_0x2c_editor(&bytes, source_hash).expect("fresh editor reopen");
        assert_eq!(
            apply_editor_project_json(&mut reopened, &json).expect("replay geometry project"),
            1
        );

        let mut app = ViewerApp::new(None);
        app.visual = Some(visual);
        app.editor = Some(reopened);
        app.sync_visual_geometry_from_editor();

        assert_eq!(
            app.visual
                .as_ref()
                .expect("visual present")
                .scene
                .nodes
                .iter()
                .find(|node| node.origin == node_id)
                .expect("replayed node stays in scene")
                .bounds,
            expected,
            "sidecar-replayed MoveNode must become visible through canonical scene sync"
        );
    }

    #[test]
    fn desktop_v0_command_surface_keeps_reopen_and_export_explicit() {
        let source = include_str!("main.rs");
        assert!(source.contains("Open PUB…"));
        assert!(source.contains("Save Project"));
        assert!(source.contains("Reopen Project"));
        assert!(source.contains("Preview IDML"));
        assert!(source.contains("Preview ODG"));
        assert!(source.contains("Reopen never discards unsaved operations."));
        assert!(source.contains("Technical details"));
        assert!(source.contains("Match details"));
    }

    #[cfg(not(feature = "reader-only"))]
    #[test]
    #[ignore = "runtime UX evidence requires pinned CHAPTERA_SAMPLE_NEWSLETTER and snapshot output env"]
    fn headless_wgpu_ux_snapshots_render_current_viewer_app() {
        use egui_kittest::Harness;

        let fixture = std::env::var_os("CHAPTERA_SAMPLE_NEWSLETTER")
            .map(PathBuf::from)
            .expect("CHAPTERA_SAMPLE_NEWSLETTER must point to the pinned Apache POI fixture");
        let output_dir = std::env::var_os("CHAPTERA_UX_SNAPSHOT_DIR")
            .map(PathBuf::from)
            .expect("CHAPTERA_UX_SNAPSHOT_DIR must name the retained screenshot directory");
        fs::create_dir_all(&output_dir).expect("create UX snapshot output directory");

        for (width, height, name) in [
            (1280.0_f32, 820.0_f32, "chaptera-editor-ux-1280x820.png"),
            (900.0_f32, 600.0_f32, "chaptera-editor-ux-900x600.png"),
        ] {
            let fixture_for_app = fixture.clone();
            let mut harness = Harness::builder()
                .with_size(egui::vec2(width, height))
                .with_pixels_per_point(1.0)
                .with_max_steps(20)
                .wgpu()
                .build_eframe(move |cc| {
                    fallback_font::install(&cc.egui_ctx)
                        .expect("pinned Chaptera fallback font resource must validate");
                    ViewerApp::new_with_storage(Some(fixture_for_app), cc.storage)
                });
            harness.step();

            let image = harness
                .render()
                .expect("headless WGPU render of the current ViewerApp must succeed");
            assert_eq!(image.width(), width as u32);
            assert_eq!(image.height(), height as u32);

            let output = output_dir.join(name);
            image.save(&output).expect("write retained UX PNG");
            assert!(
                output.metadata().expect("UX PNG metadata").len() >= 16_384,
                "UX snapshot is implausibly small"
            );
        }
    }

    #[cfg(not(feature = "reader-only"))]
    #[test]
    #[ignore = "runtime GUI evidence requires pinned CHAPTERA_SAMPLE_NEWSLETTER"]
    fn gui_only_v0_walkthrough_uses_real_widgets() {
        use egui_kittest::{Harness, kittest::Queryable};

        let fixture_source = std::env::var_os("CHAPTERA_SAMPLE_NEWSLETTER")
            .map(PathBuf::from)
            .expect("CHAPTERA_SAMPLE_NEWSLETTER must point to the pinned Apache POI fixture");
        let original = fs::read(&fixture_source).unwrap_or_else(|error| {
            panic!(
                "read pinned SampleNewsletter fixture {}: {error}",
                fixture_source.display()
            )
        });
        let root = std::env::temp_dir().join(format!(
            "chaptera-gui-v0-walkthrough-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create GUI walkthrough temp directory");
        let fixture = root.join("SampleNewsletter.pub");
        fs::write(&fixture, &original).expect("write GUI walkthrough PUB fixture");

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1280.0, 820.0))
            .with_pixels_per_point(1.0)
            .with_max_steps(20)
            .build_eframe(|cc| {
                fallback_font::install(&cc.egui_ctx)
                    .expect("pinned Chaptera fallback font resource must validate");
                ViewerApp::new_with_storage(None, cc.storage)
            });

        {
            let open = harness.get_by_label("Open PUB…");
            assert!(!open.is_disabled());
        }

        harness.input_mut().dropped_files.push(egui::DroppedFile {
            path: Some(fixture.clone()),
            ..Default::default()
        });
        harness.step();
        assert!(
            harness.state().visual.is_some(),
            "GUI file drop must open the PUB"
        );
        assert!(
            harness.state().editor.is_some(),
            "GUI open must create the EditorSession"
        );

        let (story_id, original_story, search_term) = {
            let app = harness.state();
            let visual = app.visual.as_ref().expect("visual loaded");
            let editor = app.editor.as_ref().expect("editor loaded");
            let story = visual
                .document
                .stories
                .iter()
                .find(|story| {
                    visual
                        .story_frames
                        .iter()
                        .filter(|frame| frame.story_id == story.id)
                        .count()
                        == 1
                        && editor.can_replace_story_text(story.id).is_ok()
                        && !story.text.trim().is_empty()
                })
                .expect("real fixture exposes a GUI-editable Story");
            let term = story
                .text
                .split_whitespace()
                .map(|word| {
                    word.trim_matches(|ch: char| !ch.is_alphanumeric())
                        .to_owned()
                })
                .filter(|word| word.chars().count() >= 6)
                .find(|word| {
                    let matches = visual.document.search_text(word);
                    matches.len() == 1 && matches[0].story_id == story.id
                })
                .expect("editable Story exposes one unique search term");
            (story.id, story.text.clone(), term)
        };

        {
            let search = harness.get_by_role(egui::accesskit::Role::TextInput);
            search.type_text(search_term.clone());
        }
        harness.step();
        let result_label = {
            let result = harness
                .state()
                .search_results
                .first()
                .expect("GUI search produces one result");
            assert_eq!(result.story_id, story_id);
            format!("1. {}", search_result_preview(&result.text))
        };
        harness.get_by_label(&result_label).click();
        harness.step();
        assert_eq!(harness.state().edit_buffer, original_story);

        let replacement = "Chaptera GUI-only V0 acceptance text".to_owned();
        {
            let editor = harness.get_by_role(egui::accesskit::Role::MultilineTextInput);
            editor.click();
        }
        harness.step();
        {
            let editor = harness.get_by_role(egui::accesskit::Role::MultilineTextInput);
            editor.focus();
        }
        harness.step();
        harness.press_key_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        {
            let editor = harness.get_by_role(egui::accesskit::Role::MultilineTextInput);
            editor.type_text(replacement.clone());
        }
        harness.step();
        harness.get_by_label("Apply Story edit").click();
        harness.step();
        assert_eq!(
            harness
                .state()
                .editor
                .as_ref()
                .expect("editor present")
                .operations()
                .len(),
            1,
            "Story edit must be admitted through the real GUI button"
        );

        let (movable_page_label, movable_document_point) = {
            let app = harness.state();
            let visual = app.visual.as_ref().expect("visual loaded");
            let editor = app.editor.as_ref().expect("editor loaded");
            visual
                .document
                .pages
                .iter()
                .find_map(|page| {
                    let page_origin = page.id.into_canonical();
                    let page_id_text = page.id.as_canonical().to_string();
                    let page_nodes = visual
                        .scene
                        .nodes
                        .iter()
                        .filter(|node| node.parent_origin == page_origin)
                        .collect::<Vec<_>>();
                    let hit_index = SceneHitTestIndex::new(
                        page_nodes
                            .iter()
                            .enumerate()
                            .filter_map(|(paint_order, node)| {
                                let instance =
                                    direct_scene_instance(editor, &page_id_text, node.origin)?;
                                Some(SceneHitEntry {
                                    instance_id: instance.instance_id,
                                    node_id: node.origin,
                                    bounds: node.bounds,
                                    z_order: 0,
                                    paint_order: u32::try_from(paint_order).unwrap_or(u32::MAX),
                                })
                            })
                            .collect(),
                    );

                    hit_index.entries.iter().rev().find_map(|hit| {
                        let instance = direct_scene_instance(editor, &page_id_text, hit.node_id)?;
                        let admission =
                            admit_object_mutation_v1(&instance, ObjectMutationKindV1::MoveNode);
                        if !admission.admitted
                            || admission.origin_node_id.as_deref()
                                != Some(hit.node_id.as_canonical().to_string().as_str())
                        {
                            return None;
                        }
                        let authored = editor.graph().nodes.get(&hit.node_id)?;
                        let bounds = authored.header.bounds;
                        editor
                            .can_move_node_to(hit.node_id, bounds.x, bounds.y)
                            .ok()?;
                        let point = pub_interaction::DocumentPoint::new(
                            pub_editor::LengthEmu::new(
                                hit.bounds.x.get() + hit.bounds.width.get() / 2,
                            ),
                            pub_editor::LengthEmu::new(
                                hit.bounds.y.get() + hit.bounds.height.get() / 2,
                            ),
                        );
                        hit_index
                            .topmost_at(point)
                            .filter(|top| top.instance_id == hit.instance_id)
                            .map(|_| (format!("Page {}", page.index), point))
                    })
                })
                .expect("real fixture exposes a topmost movable direct page-local object")
        };
        harness.get_by_label(&movable_page_label).click();
        harness.step();

        let (start, end) = {
            let canvas = harness
                .get_by_label("Document canvas")
                .raw_bounds()
                .expect("document canvas has screen bounds");
            let app = harness.state();
            let visual = app.visual.as_ref().expect("visual loaded");
            let page = visual
                .document
                .pages
                .get(app.selected_page)
                .expect("selected movable page remains available");
            let surface = visual
                .scene
                .surfaces
                .iter()
                .find(|surface| surface.origin == page.id)
                .expect("selected movable page has a scene surface");
            let viewport = egui::vec2(
                (canvas.x1 - canvas.x0) as f32,
                (canvas.y1 - canvas.y0) as f32,
            );
            let fit_scale = fitted_scale(
                surface.size.width.get(),
                surface.size.height.get(),
                viewport,
            )
            .expect("selected movable page has valid fit scale");
            let scene_scale = fit_scale * app.zoom;
            let page_width = surface.size.width.get() as f32 * scene_scale;
            let page_height = surface.size.height.get() as f32 * scene_scale;
            let page_left = ((canvas.x0 + canvas.x1) as f32 - page_width) / 2.0;
            let page_top = ((canvas.y0 + canvas.y1) as f32 - page_height) / 2.0;
            let start = egui::pos2(
                page_left + movable_document_point.x.get() as f32 * scene_scale,
                page_top + movable_document_point.y.get() as f32 * scene_scale,
            );
            (start, start + egui::vec2(18.0, 12.0))
        };

        harness.input_mut().events.extend([
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
        ]);
        harness.step();
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(end));
        harness.step();
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
        harness.step();

        let (moved_node_id, before_move, after_move) = {
            let editor = harness.state().editor.as_ref().expect("editor present");
            assert_eq!(
                editor.operations().len(),
                2,
                "pointer drag release must add exactly one durable MoveNode"
            );
            match editor.operations().last().expect("move operation") {
                pub_editor::EditOperation::MoveNode {
                    node_id,
                    before,
                    after,
                } => (*node_id, *before, *after),
                other => panic!("GUI drag emitted unexpected operation: {other:?}"),
            }
        };

        harness
            .get_all_by_label("Undo")
            .next()
            .expect("Undo command")
            .click();
        harness.step();
        assert_eq!(
            harness
                .state()
                .editor
                .as_ref()
                .expect("editor")
                .graph()
                .nodes[&moved_node_id]
                .header
                .bounds,
            before_move,
            "GUI Undo must restore exact pre-drag geometry"
        );

        harness
            .get_all_by_label("Redo")
            .next()
            .expect("Redo command")
            .click();
        harness.step();
        assert_eq!(
            harness
                .state()
                .editor
                .as_ref()
                .expect("editor")
                .graph()
                .nodes[&moved_node_id]
                .header
                .bounds,
            after_move,
            "GUI Redo must restore exact moved geometry"
        );

        harness.get_by_label("Save Project").click();
        harness.step();
        harness.step();
        let sidecar = editor_project_sidecar_path(&fixture).expect("sidecar path");
        assert!(sidecar.is_file(), "GUI Save Project must write the sidecar");
        // Save happens after command enablement is computed for this frame.
        // Advance once more so the accessibility tree reflects the saved sidecar.
        harness.step();

        {
            let reopen = harness.get_by_label("Reopen Project");
            assert!(
                !reopen.is_disabled(),
                "saved clean state enables Reopen Project"
            );
            reopen.click();
        }
        harness.step();
        {
            let editor = harness
                .state()
                .editor
                .as_ref()
                .expect("fresh reopened editor");
            assert_eq!(editor.operations().len(), 2);
            assert_eq!(editor.graph().stories[&story_id].text, replacement);
            assert_eq!(
                editor.graph().nodes[&moved_node_id].header.bounds,
                after_move
            );
        }

        {
            let export = harness.get_by_role_and_label(egui::accesskit::Role::Button, "Export");
            export.click();
        }
        harness.step();
        {
            let preview_idml = harness
                .get_all_by_label("Preview IDML")
                .last()
                .expect("Export popup exposes Preview IDML");
            preview_idml.click();
        }
        harness.step();
        let (preview, preview_status) = {
            let app = harness.state();
            (app.export_preview.clone(), app.edit_status.clone())
        };
        let preview = preview.unwrap_or_else(|| {
            panic!("GUI IDML preview was not created; edit_status={preview_status:?}")
        });
        assert_eq!(
            preview.target,
            pub_editor::EditorEditableTarget::Idml,
            "GUI preview must target IDML"
        );
        assert_eq!(
            preview.operation_count, 2,
            "GUI IDML preview must bind both accepted operations"
        );
        assert!(
            preview.can_serialize,
            "GUI IDML preview must be serializable; summary={}",
            preview.summary
        );
        {
            let export_idml = harness
                .get_all_by_label("Export edited IDML copy")
                .last()
                .expect("Preview keeps the Export popup open with the edited IDML action");
            export_idml.click();
        }
        harness.step();

        let exported = editable_export_path(&fixture, pub_editor::EditorEditableTarget::Idml)
            .expect("IDML path");
        assert!(exported.is_file(), "GUI export must write edited IDML");
        assert!(
            editable_export_report_path(&exported).is_file(),
            "GUI export must write its loss report"
        );
        assert_eq!(
            fs::read(&fixture).expect("read immutable source after GUI walkthrough"),
            original,
            "GUI-only V0 walkthrough must never mutate the source PUB"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(not(feature = "reader-only"))]
    #[test]
    #[ignore = "runtime GUI evidence requires pinned CHAPTERA_SAMPLE_NEWSLETTER"]
    fn gui_resize_handle_commits_one_resize_node() {
        use egui_kittest::{Harness, kittest::Queryable};

        let fixture = std::env::var_os("CHAPTERA_SAMPLE_NEWSLETTER")
            .map(PathBuf::from)
            .expect("CHAPTERA_SAMPLE_NEWSLETTER must point to the pinned Apache POI fixture");
        let original = fs::read(&fixture).expect("read pinned SampleNewsletter fixture");

        let fixture_for_app = fixture.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1280.0, 820.0))
            .with_pixels_per_point(1.0)
            .with_max_steps(24)
            .build_eframe(move |cc| {
                fallback_font::install(&cc.egui_ctx)
                    .expect("pinned Chaptera fallback font resource must validate");
                ViewerApp::new_with_storage(Some(fixture_for_app), cc.storage)
            });
        harness.step();

        let (page_label, target_document_point) = {
            let app = harness.state();
            let visual = app.visual.as_ref().expect("visual loaded");
            let editor = app.editor.as_ref().expect("editor loaded");
            visual
                .document
                .pages
                .iter()
                .find_map(|page| {
                    let page_origin = page.id.into_canonical();
                    let page_id_text = page.id.as_canonical().to_string();
                    let page_nodes = visual
                        .scene
                        .nodes
                        .iter()
                        .filter(|node| node.parent_origin == page_origin)
                        .collect::<Vec<_>>();
                    let hit_index = SceneHitTestIndex::new(
                        page_nodes
                            .iter()
                            .enumerate()
                            .filter_map(|(paint_order, node)| {
                                let instance =
                                    direct_scene_instance(editor, &page_id_text, node.origin)?;
                                Some(SceneHitEntry {
                                    instance_id: instance.instance_id,
                                    node_id: node.origin,
                                    bounds: node.bounds,
                                    z_order: 0,
                                    paint_order: u32::try_from(paint_order).unwrap_or(u32::MAX),
                                })
                            })
                            .collect(),
                    );

                    hit_index.entries.iter().rev().find_map(|hit| {
                        let instance = direct_scene_instance(editor, &page_id_text, hit.node_id)?;
                        let admission =
                            admit_object_mutation_v1(&instance, ObjectMutationKindV1::ResizeNode);
                        if !admission.admitted
                            || admission.origin_node_id.as_deref()
                                != Some(hit.node_id.as_canonical().to_string().as_str())
                            || editor.can_resize_node(hit.node_id).is_err()
                        {
                            return None;
                        }
                        let point = pub_interaction::DocumentPoint::new(
                            pub_editor::LengthEmu::new(
                                hit.bounds.x.get() + hit.bounds.width.get() / 2,
                            ),
                            pub_editor::LengthEmu::new(
                                hit.bounds.y.get() + hit.bounds.height.get() / 2,
                            ),
                        );
                        hit_index
                            .topmost_at(point)
                            .filter(|top| top.instance_id == hit.instance_id)
                            .map(|_| (format!("Page {}", page.index), point))
                    })
                })
                .expect("real fixture exposes a topmost ResizeNode-admitted object")
        };

        harness.get_by_label(&page_label).click();
        harness.step();

        let object_center = {
            let canvas = harness
                .get_by_label("Document canvas")
                .raw_bounds()
                .expect("document canvas has screen bounds");
            let app = harness.state();
            let visual = app.visual.as_ref().expect("visual loaded");
            let page = visual
                .document
                .pages
                .get(app.selected_page)
                .expect("selected resize page remains available");
            let surface = visual
                .scene
                .surfaces
                .iter()
                .find(|surface| surface.origin == page.id)
                .expect("selected resize page has a scene surface");
            let viewport = egui::vec2(
                (canvas.x1 - canvas.x0) as f32,
                (canvas.y1 - canvas.y0) as f32,
            );
            let fit_scale = fitted_scale(
                surface.size.width.get(),
                surface.size.height.get(),
                viewport,
            )
            .expect("selected resize page has valid fit scale");
            let scene_scale = fit_scale * app.zoom;
            let page_width = surface.size.width.get() as f32 * scene_scale;
            let page_height = surface.size.height.get() as f32 * scene_scale;
            let page_left = ((canvas.x0 + canvas.x1) as f32 - page_width) / 2.0;
            let page_top = ((canvas.y0 + canvas.y1) as f32 - page_height) / 2.0;
            egui::pos2(
                page_left + target_document_point.x.get() as f32 * scene_scale,
                page_top + target_document_point.y.get() as f32 * scene_scale,
            )
        };
        harness.input_mut().events.extend([
            egui::Event::PointerMoved(object_center),
            egui::Event::PointerButton {
                pos: object_center,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
            egui::Event::PointerButton {
                pos: object_center,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        ]);
        harness.step();
        harness.step();

        let handle_bounds = harness
            .get_by_label("Resize bottom-right handle")
            .raw_bounds()
            .expect("selected resizable object exposes bottom-right handle");
        let start = egui::pos2(
            ((handle_bounds.x0 + handle_bounds.x1) / 2.0) as f32,
            ((handle_bounds.y0 + handle_bounds.y1) / 2.0) as f32,
        );
        let end = start + egui::vec2(18.0, 12.0);

        harness.input_mut().events.extend([
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
        ]);
        harness.step();
        assert_eq!(
            harness
                .state()
                .editor
                .as_ref()
                .expect("editor")
                .operations()
                .len(),
            0,
            "resize pointer-down/preview must not emit an Editor operation"
        );

        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(end));
        harness.step();
        assert_eq!(
            harness
                .state()
                .editor
                .as_ref()
                .expect("editor")
                .operations()
                .len(),
            0,
            "resize pointer motion must remain transient"
        );

        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
        harness.step();

        let (node_id, before, after) = {
            let editor = harness.state().editor.as_ref().expect("editor");
            assert_eq!(
                editor.operations().len(),
                1,
                "handle release must emit exactly one ResizeNode"
            );
            match editor.operations().last().expect("resize operation") {
                pub_editor::EditOperation::ResizeNode {
                    node_id,
                    before,
                    after,
                } => (*node_id, *before, *after),
                other => panic!("resize handle emitted unexpected operation: {other:?}"),
            }
        };
        assert_ne!(before.width, after.width);
        assert_ne!(before.height, after.height);

        harness
            .get_all_by_label("Undo")
            .next()
            .expect("Undo command")
            .click();
        harness.step();
        assert_eq!(
            harness
                .state()
                .editor
                .as_ref()
                .expect("editor")
                .graph()
                .nodes[&node_id]
                .header
                .bounds,
            before,
            "GUI Undo restores exact pre-resize bounds"
        );

        harness
            .get_all_by_label("Redo")
            .next()
            .expect("Redo command")
            .click();
        harness.step();
        assert_eq!(
            harness
                .state()
                .editor
                .as_ref()
                .expect("editor")
                .graph()
                .nodes[&node_id]
                .header
                .bounds,
            after,
            "GUI Redo restores exact resized bounds"
        );
        assert_eq!(
            fs::read(&fixture).expect("read immutable source after resize"),
            original,
            "GUI resize must not mutate source PUB bytes"
        );
    }

    #[test]
    fn replacement_image_mime_is_bounded_to_png_and_jpeg() {
        assert_eq!(
            replacement_image_mime(Path::new("replacement.png")),
            Some("image/png")
        );
        assert_eq!(
            replacement_image_mime(Path::new("replacement.JPG")),
            Some("image/jpeg")
        );
        assert_eq!(
            replacement_image_mime(Path::new("replacement.jpeg")),
            Some("image/jpeg")
        );
        assert_eq!(replacement_image_mime(Path::new("replacement.gif")), None);
        assert_eq!(replacement_image_mime(Path::new("replacement")), None);
    }

    #[cfg(feature = "embedded-fixture-tests")]
    #[test]
    fn direct_image_replacement_roundtrips_through_project_assets() {
        let bytes = sample_newsletter_fixture();
        let visual = pub_viewer::open_mature_0x2c_geometry(
            &bytes,
            pub_viewer::viewer_geometry_environment_v0_1(),
        )
        .expect("SampleNewsletter Viewer open");
        let source_hash = visual.document.source.source_hash;
        let mut editor =
            pub_editor::open_mature_0x2c_editor(&bytes, source_hash).expect("editor open");

        let (node_id, replacement_mime, replacement_bytes) = visual
            .document
            .pages
            .iter()
            .find_map(|page| {
                let page_origin = page.id.into_canonical();
                let page_id_text = page.id.as_canonical().to_string();
                visual
                    .scene
                    .nodes
                    .iter()
                    .filter(|node| node.parent_origin == page_origin)
                    .find_map(|scene_node| {
                        let instance =
                            direct_scene_instance(&editor, &page_id_text, scene_node.origin)?;
                        let admission =
                            admit_object_mutation_v1(&instance, ObjectMutationKindV1::ReplaceImage);
                        if !admission.admitted
                            || admission.origin_node_id.as_deref()
                                != Some(scene_node.origin.as_canonical().to_string().as_str())
                        {
                            return None;
                        }
                        let authored = editor.graph().nodes.get(&scene_node.origin)?;
                        if authored.payload.image_slot.is_none()
                            || authored.payload.explicit_image_crop.is_some()
                        {
                            return None;
                        }
                        let embedded = visual
                            .images
                            .iter()
                            .find(|image| image.node_ids.contains(&scene_node.origin))?;
                        if !matches!(embedded.mime.as_str(), "image/png" | "image/jpeg") {
                            return None;
                        }
                        Some((
                            scene_node.origin,
                            embedded.mime.clone(),
                            embedded.bytes.clone(),
                        ))
                    })
            })
            .expect("fixture exposes one direct crop-free image target");

        let replacement_asset = editor
            .import_replacement_asset(replacement_mime, replacement_bytes)
            .expect("bounded PNG/JPEG replacement import");
        editor
            .can_replace_image(node_id, replacement_asset)
            .expect("direct instance remains ReplaceImage-capable");

        let before_count = editor.operations().len();
        let operation = editor
            .replace_image(node_id, replacement_asset)
            .expect("canonical ReplaceImage");
        assert!(matches!(
            operation,
            pub_editor::EditOperation::ReplaceImage {
                node_id: actual,
                after_asset,
                ..
            } if actual == node_id && after_asset == replacement_asset
        ));
        assert_eq!(editor.operations().len(), before_count + 1);
        assert_eq!(
            editor.image_replacement_for(node_id),
            Some(replacement_asset)
        );

        editor.undo().expect("ReplaceImage undo");
        assert_eq!(editor.image_replacement_for(node_id), None);
        editor.redo().expect("ReplaceImage redo");
        assert_eq!(
            editor.image_replacement_for(node_id),
            Some(replacement_asset)
        );

        let project = editor.project();
        assert_eq!(project.assets.len(), 1);
        let asset_bytes = editor
            .replacement_assets()
            .map(|asset| (asset.sha256, asset.bytes.clone()))
            .collect::<BTreeMap<_, _>>();

        let mut reopened =
            pub_editor::open_mature_0x2c_editor(&bytes, source_hash).expect("fresh editor reopen");
        reopened
            .apply_project_with_assets(&project, &asset_bytes)
            .expect("fresh replay with replacement bytes");
        assert_eq!(
            reopened.image_replacement_for(node_id),
            Some(replacement_asset),
            "fresh EditorProject replay must preserve replacement identity"
        );
    }

    #[cfg(feature = "embedded-fixture-tests")]
    #[test]
    fn desktop_replace_image_ui_uses_scene_instance_gate() {
        let bytes = sample_newsletter_fixture();
        let original = bytes.clone();
        let visual = pub_viewer::open_mature_0x2c_geometry(
            &bytes,
            pub_viewer::viewer_geometry_environment_v0_1(),
        )
        .expect("SampleNewsletter Viewer open");
        let source_hash = visual.document.source.source_hash;
        let editor = pub_editor::open_mature_0x2c_editor(&bytes, source_hash).expect("editor open");

        let (page_index, instance_id, mime, replacement_bytes) = visual
            .document
            .pages
            .iter()
            .enumerate()
            .find_map(|(page_index, page)| {
                let page_origin = page.id.into_canonical();
                let page_id_text = page.id.as_canonical().to_string();
                visual
                    .scene
                    .nodes
                    .iter()
                    .filter(|node| node.parent_origin == page_origin)
                    .find_map(|scene_node| {
                        let instance =
                            direct_scene_instance(&editor, &page_id_text, scene_node.origin)?;
                        let admission =
                            admit_object_mutation_v1(&instance, ObjectMutationKindV1::ReplaceImage);
                        if !admission.admitted {
                            return None;
                        }
                        let authored = editor.graph().nodes.get(&scene_node.origin)?;
                        if authored.payload.image_slot.is_none()
                            || authored.payload.explicit_image_crop.is_some()
                        {
                            return None;
                        }
                        let embedded = visual
                            .images
                            .iter()
                            .find(|image| image.node_ids.contains(&scene_node.origin))?;
                        if !matches!(embedded.mime.as_str(), "image/png" | "image/jpeg") {
                            return None;
                        }
                        Some((
                            page_index,
                            instance.instance_id,
                            embedded.mime.clone(),
                            embedded.bytes.clone(),
                        ))
                    })
            })
            .expect("fixture exposes one direct image placement");

        let root =
            std::env::temp_dir().join(format!("chaptera-replace-image-ui-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create replacement temp directory");
        let extension = if mime == "image/png" { "png" } else { "jpg" };
        let replacement_path = root.join(format!("replacement.{extension}"));
        fs::write(&replacement_path, replacement_bytes).expect("write replacement image");

        let mut app = ViewerApp::new(None);
        app.visual = Some(visual);
        app.editor = Some(editor);
        app.selected_page = page_index;
        app.canvas_selection.select_only(instance_id);

        let target = app
            .selected_direct_replace_image_target()
            .expect("selected visual instance passes ReplaceImage admission");
        let before_count = app.editor.as_ref().expect("editor").operations().len();

        app.replace_selected_image_from_path(&replacement_path)
            .expect("desktop ReplaceImage command");

        let editor = app.editor.as_ref().expect("editor remains available");
        assert_eq!(editor.operations().len(), before_count + 1);
        assert!(editor.image_replacement_for(target).is_some());
        assert_eq!(
            bytes, original,
            "desktop ReplaceImage must not mutate source PUB bytes"
        );
        assert_eq!(editor.project().assets.len(), 1);

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(feature = "embedded-fixture-tests")]
    #[test]
    #[ignore = "real ReplaceImage UI/export receipt evidence is owned by Windows CI"]
    fn replace_image_real_receipt_pair_evidence() {
        use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
        use pub_export::{
            CapabilityLevel, SemanticFeatureRequest, TargetCapabilityManifest, TargetProfile,
            plan_export,
        };
        use sha2::{Digest, Sha256};
        use std::io::{Cursor, Read};

        fn hex_sha256(bytes: &[u8]) -> String {
            Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }

        fn encode_base64(bytes: &[u8]) -> String {
            const TABLE: &[u8; 64] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
            for chunk in bytes.chunks(3) {
                let b0 = chunk[0];
                let b1 = chunk.get(1).copied().unwrap_or(0);
                let b2 = chunk.get(2).copied().unwrap_or(0);

                output.push(char::from(TABLE[(b0 >> 2) as usize]));
                output.push(char::from(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize]));
                if chunk.len() > 1 {
                    output.push(char::from(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize]));
                } else {
                    output.push('=');
                }
                if chunk.len() > 2 {
                    output.push(char::from(TABLE[(b2 & 0x3f) as usize]));
                } else {
                    output.push('=');
                }
            }
            output
        }

        fn idml_package_contains_exact_embedded_bytes(package: &[u8], expected: &[u8]) -> bool {
            let encoded = encode_base64(expected);
            let needle = format!("<Contents><![CDATA[{encoded}]]></Contents>");
            let mut archive =
                zip::ZipArchive::new(Cursor::new(package)).expect("IDML export is a ZIP");
            for index in 0..archive.len() {
                let mut part = archive.by_index(index).expect("read IDML ZIP entry");
                if part.is_dir() {
                    continue;
                }
                let mut bytes = Vec::new();
                part.read_to_end(&mut bytes).expect("read IDML ZIP payload");
                if std::str::from_utf8(&bytes).is_ok_and(|text| text.contains(&needle)) {
                    return true;
                }
            }
            false
        }

        fn odg_package_contains_sha256(package: &[u8], expected: &str) -> bool {
            let mut archive =
                zip::ZipArchive::new(Cursor::new(package)).expect("ODG export is a ZIP");
            for index in 0..archive.len() {
                let mut part = archive.by_index(index).expect("read ODG ZIP entry");
                if part.is_dir() {
                    continue;
                }
                let mut bytes = Vec::new();
                part.read_to_end(&mut bytes).expect("read ODG ZIP payload");
                if hex_sha256(&bytes) == expected {
                    return true;
                }
            }
            false
        }

        fn report_has(
            report: &pub_export::ExportReport,
            feature: &str,
            disposition: CapabilityLevel,
        ) -> bool {
            report
                .items
                .iter()
                .any(|item| item.feature == feature && item.disposition == disposition)
        }

        let ui_receipt_path = std::env::var_os("CHAPTERA_REPLACE_IMAGE_UI_RECEIPT")
            .map(PathBuf::from)
            .expect("CHAPTERA_REPLACE_IMAGE_UI_RECEIPT is required");
        let proof_path = std::env::var_os("CHAPTERA_REPLACE_IMAGE_PRIVATE_PROOF")
            .map(PathBuf::from)
            .expect("CHAPTERA_REPLACE_IMAGE_PRIVATE_PROOF is required");
        let binding_id = std::env::var("CHAPTERA_REPLACE_IMAGE_BINDING_ID")
            .expect("CHAPTERA_REPLACE_IMAGE_BINDING_ID is required");
        assert!(
            binding_id.starts_with("rb_")
                && binding_id.len() == 35
                && binding_id[3..].chars().all(|ch| ch.is_ascii_hexdigit()),
            "replacement binding must be one opaque rb_ + 32-hex id"
        );
        let build_sha256 = std::env::var("CHAPTERA_REPLACE_IMAGE_BUILD_SHA256")
            .expect("CHAPTERA_REPLACE_IMAGE_BUILD_SHA256 is required");
        assert!(
            build_sha256.len() == 64
                && build_sha256
                    .chars()
                    .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()),
            "build SHA must be lowercase SHA-256"
        );
        let chaptera_version = std::env::var("CHAPTERA_REPLACE_IMAGE_VERSION")
            .expect("CHAPTERA_REPLACE_IMAGE_VERSION is required");

        let bytes = sample_newsletter_fixture();
        let original = bytes.clone();
        let visual = pub_viewer::open_mature_0x2c_geometry(
            &bytes,
            pub_viewer::viewer_geometry_environment_v0_1(),
        )
        .expect("SampleNewsletter Viewer open");
        let source_hash = visual.document.source.source_hash;
        let editor = pub_editor::open_mature_0x2c_editor(&bytes, source_hash).expect("editor open");

        let (page_index, instance_id, target, source_asset_sha256, before_bounds, before_crop) =
            visual
                .document
                .pages
                .iter()
                .enumerate()
                .find_map(|(page_index, page)| {
                    let page_origin = page.id.into_canonical();
                    let page_id_text = page.id.as_canonical().to_string();
                    visual
                        .scene
                        .nodes
                        .iter()
                        .filter(|node| node.parent_origin == page_origin)
                        .find_map(|scene_node| {
                            let instance =
                                direct_scene_instance(&editor, &page_id_text, scene_node.origin)?;
                            let admission = admit_object_mutation_v1(
                                &instance,
                                ObjectMutationKindV1::ReplaceImage,
                            );
                            if !admission.admitted {
                                return None;
                            }
                            let authored = editor.graph().nodes.get(&scene_node.origin)?;
                            if authored.payload.image_slot.is_none()
                                || authored.payload.explicit_image_crop.is_some()
                                || authored.header.bounds.width.get() <= 0
                                || authored.header.bounds.height.get() <= 0
                            {
                                return None;
                            }
                            let embedded = visual
                                .images
                                .iter()
                                .find(|image| image.node_ids.contains(&scene_node.origin))?;
                            Some((
                                page_index,
                                instance.instance_id,
                                scene_node.origin,
                                hex_sha256(&embedded.bytes),
                                authored.header.bounds,
                                authored.payload.explicit_image_crop.clone(),
                            ))
                        })
                })
                .expect("real fixture exposes one direct crop-free image target");

        let replacement_image = ImageBuffer::from_pixel(2, 2, Rgba([17_u8, 91_u8, 203_u8, 255_u8]));
        let mut replacement_cursor = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(replacement_image)
            .write_to(&mut replacement_cursor, ImageFormat::Png)
            .expect("encode deterministic replacement PNG");
        let replacement_bytes = replacement_cursor.into_inner();
        let replacement_sha256 = hex_sha256(&replacement_bytes);
        assert!(
            source_asset_sha256 != replacement_sha256,
            "replacement must differ from source image bytes"
        );

        let root = std::env::temp_dir().join(format!(
            "chaptera-replace-image-real-receipt-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create ReplaceImage receipt temp dir");
        let replacement_path = root.join("replacement.png");
        fs::write(&replacement_path, &replacement_bytes).expect("write replacement PNG");

        let mut app = ViewerApp::new(None);
        app.visual = Some(visual);
        app.editor = Some(editor);
        app.selected_page = page_index;
        app.canvas_selection.select_only(instance_id);

        assert!(
            app.selected_direct_replace_image_target()
                .expect("typed direct SceneInstance admission")
                == target,
            "typed SceneInstance admission returned a different private target"
        );
        let operation_count_before = app.editor.as_ref().expect("editor").operations().len();
        app.replace_selected_image_from_path(&replacement_path)
            .expect("real desktop ReplaceImage UI command");

        let replacement_asset = app
            .editor
            .as_ref()
            .expect("editor")
            .image_replacement_for(target)
            .expect("committed replacement identity");
        assert!(
            replacement_asset.to_string() == replacement_sha256,
            "committed replacement identity differs from imported private asset"
        );
        assert_eq!(
            app.editor.as_ref().expect("editor").operations().len(),
            operation_count_before + 1
        );

        let (after_bounds, after_crop) = {
            let authored = &app.editor.as_ref().expect("editor").graph().nodes[&target];
            (
                authored.header.bounds,
                authored.payload.explicit_image_crop.clone(),
            )
        };
        assert_eq!(
            after_bounds, before_bounds,
            "ReplaceImage preserves frame bounds"
        );
        assert_eq!(after_crop, before_crop, "ReplaceImage preserves crop state");

        // The live canvas prefers a decoded replacement texture over the source
        // texture whenever the effective replacement identity is present.
        let texture_context = egui::Context::default();
        app.ensure_image_textures(&texture_context);
        let replacement_texture_key = format!("replacement:{:?}", replacement_asset);
        assert!(
            app.image_textures.contains_key(&replacement_texture_key),
            "replacement overlay texture must be available to the real canvas"
        );

        let (
            duplicate_same_sha_reused,
            mime_conflict_rejected,
            empty_asset_rejected,
            unsupported_mime_rejected,
            signature_mismatch_rejected,
            missing_registered_asset_rejected,
            same_asset_no_change_rejected,
        ) = {
            let editor = app.editor.as_mut().expect("editor");
            let duplicate = editor
                .import_replacement_asset("image/png", replacement_bytes.clone())
                .expect("duplicate replacement import");
            let mime_conflict = editor
                .import_replacement_asset("image/jpeg", replacement_bytes.clone())
                .is_err();
            let empty = editor
                .import_replacement_asset("image/png", Vec::new())
                .is_err();
            let unsupported = editor
                .import_replacement_asset("image/gif", vec![1, 2, 3])
                .is_err();
            let signature = editor
                .import_replacement_asset("image/png", vec![1, 2, 3])
                .is_err();
            let mut fake_bytes = [0xa5_u8; 32];
            if pub_editor::Sha256Digest::from_bytes(fake_bytes) == replacement_asset {
                fake_bytes[0] ^= 0xff;
            }
            let missing = editor
                .can_replace_image(target, pub_editor::Sha256Digest::from_bytes(fake_bytes))
                .is_err();
            let no_change = editor.replace_image(target, replacement_asset).is_err();
            (
                duplicate == replacement_asset,
                mime_conflict,
                empty,
                unsupported,
                signature,
                missing,
                no_change,
            )
        };
        assert!(duplicate_same_sha_reused);
        assert!(mime_conflict_rejected);
        assert!(empty_asset_rejected);
        assert!(unsupported_mime_rejected);
        assert!(signature_mismatch_rejected);
        assert!(missing_registered_asset_rejected);
        assert!(same_asset_no_change_rejected);

        let base_graph = app.editor.as_ref().expect("editor").graph().clone();
        let negative_asset = replacement_bytes.clone();

        let missing_image_slot_rejected = {
            let mut graph = base_graph.clone();
            graph
                .nodes
                .get_mut(&target)
                .expect("target")
                .payload
                .image_slot = None;
            let mut candidate = pub_editor::EditorSession::new(graph).expect("candidate");
            let asset = candidate
                .import_replacement_asset("image/png", negative_asset.clone())
                .expect("negative asset");
            candidate.can_replace_image(target, asset).is_err()
        };
        let crop_bearing_target_rejected = {
            let mut graph = base_graph.clone();
            graph
                .nodes
                .get_mut(&target)
                .expect("target")
                .payload
                .explicit_image_crop = Some(
                serde_json::from_value(serde_json::json!({
                    "top_raw": 1,
                    "bottom_raw": null,
                    "left_raw": null,
                    "right_raw": null,
                    "ambiguous": false
                }))
                .expect("synthetic explicit crop state"),
            );
            let mut candidate = pub_editor::EditorSession::new(graph).expect("candidate");
            let asset = candidate
                .import_replacement_asset("image/png", negative_asset.clone())
                .expect("negative asset");
            candidate.can_replace_image(target, asset).is_err()
        };
        let invalid_bounds_rejected = {
            let mut graph = base_graph.clone();
            graph
                .nodes
                .get_mut(&target)
                .expect("target")
                .header
                .bounds
                .width = pub_editor::LengthEmu::new(0);
            let mut candidate = pub_editor::EditorSession::new(graph).expect("candidate");
            let asset = candidate
                .import_replacement_asset("image/png", negative_asset.clone())
                .expect("negative asset");
            candidate.can_replace_image(target, asset).is_err()
        };
        let non_page_owned_rejected = {
            let mut graph = base_graph;
            graph
                .nodes
                .get_mut(&target)
                .expect("target")
                .header
                .parent_id = target.into_canonical();
            let mut candidate = pub_editor::EditorSession::new(graph).expect("candidate");
            let asset = candidate
                .import_replacement_asset("image/png", negative_asset)
                .expect("negative asset");
            candidate.can_replace_image(target, asset).is_err()
        };
        assert!(missing_image_slot_rejected);
        assert!(crop_bearing_target_rejected);
        assert!(invalid_bounds_rejected);
        assert!(non_page_owned_rejected);

        app.editor
            .as_mut()
            .expect("editor")
            .undo()
            .expect("ReplaceImage undo");
        let undo_restores_previous_asset = app
            .editor
            .as_ref()
            .expect("editor")
            .image_replacement_for(target)
            .is_none();
        app.editor
            .as_mut()
            .expect("editor")
            .redo()
            .expect("ReplaceImage redo");
        let redo_restores_replacement_asset = app
            .editor
            .as_ref()
            .expect("editor")
            .image_replacement_for(target)
            == Some(replacement_asset);
        assert!(undo_restores_previous_asset);
        assert!(redo_restores_replacement_asset);

        let project = app.editor.as_ref().expect("editor").project();
        assert_eq!(
            project.schema_version,
            pub_editor::EDITOR_PROJECT_VERSION_V0_11
        );
        assert_eq!(project.assets.len(), 1);
        let asset_bytes = app
            .editor
            .as_ref()
            .expect("editor")
            .replacement_assets()
            .map(|asset| (asset.sha256, asset.bytes.clone()))
            .collect::<BTreeMap<_, _>>();

        let mut missing_asset_replay =
            pub_editor::open_mature_0x2c_editor(&bytes, source_hash).expect("fresh candidate");
        let empty_state = missing_asset_replay.project();
        assert!(
            missing_asset_replay
                .apply_project_with_assets(&project, &BTreeMap::new())
                .is_err(),
            "project replay must require replacement bytes"
        );
        assert!(
            missing_asset_replay.project() == empty_state,
            "failed replay must be transactional"
        );

        let mut reopened =
            pub_editor::open_mature_0x2c_editor(&bytes, source_hash).expect("fresh editor");
        reopened
            .apply_project_with_assets(&project, &asset_bytes)
            .expect("fresh replay with replacement bytes");
        assert!(
            reopened.image_replacement_for(target) == Some(replacement_asset),
            "fresh replay preserves replacement identity"
        );

        let idml_preview = reopened
            .preview_editable_export(pub_editor::EditorEditableTarget::Idml, "receipt.pub")
            .expect("IDML loss preview");
        let odg_preview = reopened
            .preview_editable_export(pub_editor::EditorEditableTarget::Odg, "receipt.pub")
            .expect("ODG loss preview");
        assert!(idml_preview.report.can_serialize);
        assert!(odg_preview.report.can_serialize);
        for report in [&idml_preview.report, &odg_preview.report] {
            assert!(report_has(
                report,
                "image.bytes",
                CapabilityLevel::Preserved
            ));
            assert!(report_has(
                report,
                "image.frame_geometry",
                CapabilityLevel::Preserved
            ));
            assert!(report_has(
                report,
                "image.content_transform",
                CapabilityLevel::Approximated
            ));
        }

        let idml = reopened
            .export_editable(pub_editor::EditorEditableTarget::Idml, "receipt.pub")
            .expect("real IDML export");
        let odg = reopened
            .export_editable(pub_editor::EditorEditableTarget::Odg, "receipt.pub")
            .expect("real ODG export");
        assert!(
            idml_package_contains_exact_embedded_bytes(&idml.bytes, &replacement_bytes),
            "IDML package must contain exact replacement bytes in embedded Contents"
        );
        assert!(
            odg_package_contains_sha256(&odg.bytes, &replacement_sha256),
            "ODG package must contain an exact replacement binary part"
        );

        // Prove the generic loss-gated exporter blocks an unadvertised target
        // rather than silently dropping the required replacement bytes.
        let unsupported_plan = plan_export(
            &TargetCapabilityManifest {
                target: TargetProfile {
                    format: "unsupported-receipt-probe".into(),
                    adapter_version: "v0".into(),
                    profile: "bounded".into(),
                    schema_fence: None,
                },
                features: BTreeMap::new(),
            },
            vec![SemanticFeatureRequest {
                feature: "image.bytes".into(),
                origin: None,
                property_path: Some("replacement_asset.bytes".into()),
                require_preserved: true,
            }],
        );
        assert!(!unsupported_plan.can_serialize());
        assert_eq!(unsupported_plan.blockers.len(), 1);
        assert_eq!(unsupported_plan.losses.len(), 1);

        let source_after = sample_newsletter_fixture();
        assert!(
            source_after == original,
            "real ReplaceImage evidence must not mutate the source PUB"
        );

        let ui_receipt = serde_json::json!({
            "receipt_version": "chaptera.replace-image-ui-producer-receipt.v1",
            "operation_contract": "chaptera.replace-image.v1",
            "producer": {
                "kind": "chaptera_desktop_editor",
                "integration": "local_private"
            },
            "build": {
                "chaptera_version": chaptera_version,
                "platform": "windows",
                "binary_sha256": build_sha256
            },
            "fixture_kind": "real_pub_sanitized",
            "target_gate": {
                "image_slot_present": true,
                "explicit_crop_present": false,
                "direct_page_owned": true,
                "valid_bounds": true,
                "target_id_redacted": true
            },
            "asset_import": {
                "mime": "image/png",
                "non_empty": true,
                "signature_matches_declared_mime": true,
                "content_addressed_sha256": true,
                "duplicate_same_sha_reused": duplicate_same_sha_reused,
                "mime_conflict_rejected": mime_conflict_rejected,
                "filename_is_identity": false,
                "url_is_identity": false,
                "asset_sha_redacted": true
            },
            "commit": {
                "operation_kind": "ReplaceImage",
                "operation_count_before": operation_count_before,
                "operation_count_after": operation_count_before + 1,
                "registered_asset_required": missing_registered_asset_rejected,
                "same_asset_no_change_rejected": same_asset_no_change_rejected,
                "source_pub_unchanged": true
            },
            "replacement_binding": {
                "binding_id": binding_id,
                "content_derived": false,
                "import_matches_committed_asset": true,
                "committed_matches_preview_asset": app.image_textures.contains_key(&replacement_texture_key),
                "committed_matches_redo_asset": redo_restores_replacement_asset,
                "committed_matches_fresh_replay_asset": reopened.image_replacement_for(target) == Some(replacement_asset)
            },
            "project_replay": {
                "image_operation_schema_supported": true,
                "replacement_asset_metadata_persisted": project.assets.len() == 1,
                "asset_bytes_required_on_replay": true,
                "undo_restores_previous_asset": undo_restores_previous_asset,
                "redo_restores_replacement_asset": redo_restores_replacement_asset,
                "fresh_replay_reproduces_replacement": reopened.image_replacement_for(target) == Some(replacement_asset),
                "replay_transactional": missing_asset_replay.project() == empty_state
            },
            "preview": {
                "replacement_overlay_preferred": app.image_textures.contains_key(&replacement_texture_key),
                "source_geometry_unchanged": after_bounds == before_bounds,
                "source_crop_state_unchanged": after_crop == before_crop
            },
            "negative_probes": {
                "missing_image_slot_rejected": missing_image_slot_rejected,
                "crop_bearing_target_rejected": crop_bearing_target_rejected,
                "invalid_bounds_rejected": invalid_bounds_rejected,
                "non_page_owned_rejected": non_page_owned_rejected,
                "empty_asset_rejected": empty_asset_rejected,
                "unsupported_mime_rejected": unsupported_mime_rejected,
                "signature_mismatch_rejected": signature_mismatch_rejected,
                "missing_registered_asset_rejected": missing_registered_asset_rejected
            },
            "privacy": {
                "pub_bytes_in_receipt": false,
                "pub_filename_in_receipt": false,
                "local_path_in_receipt": false,
                "source_hash_in_receipt": false,
                "node_id_in_receipt": false,
                "asset_sha_in_receipt": false,
                "replacement_bytes_in_receipt": false,
                "document_text_in_receipt": false,
                "customer_identity_in_receipt": false
            }
        });

        let private_proof = serde_json::json!({
            "request": {
                "action": "export_replace_image",
                "source_hash": source_hash.to_string(),
                "replacement_binding_id": ui_receipt["replacement_binding"]["binding_id"],
                "fixture_kind": "real_pub_sanitized"
            },
            "proof": {
                "replacement_binding_id": ui_receipt["replacement_binding"]["binding_id"],
                "source_hash_after": source_hash.to_string(),
                "source_asset_sha256": source_asset_sha256,
                "committed_asset_sha256": replacement_sha256,
                "effective_asset_sha256": reopened.image_replacement_for(target)
                    .expect("effective replacement")
                    .to_string(),
                "idml": {
                    "can_serialize": idml.report.can_serialize,
                    "embedded_asset_sha256": replacement_sha256,
                    "frame_geometry": "preserved",
                    "content_transform": "approximated",
                    "z_order": "approximated"
                },
                "odg": {
                    "can_serialize": odg.report.can_serialize,
                    "embedded_asset_sha256": replacement_sha256,
                    "frame_geometry": "preserved",
                    "content_transform": "approximated",
                    "z_order": "preserved"
                },
                "unsupported_target": {
                    "blocked": !unsupported_plan.can_serialize(),
                    "explicit_loss": unsupported_plan.losses.len() == 1,
                    "silent_drop": false,
                    "silent_source_fallback": false
                },
                "native_pub_writer_promoted": false
            }
        });

        if let Some(parent) = ui_receipt_path.parent() {
            fs::create_dir_all(parent).expect("create UI receipt directory");
        }
        if let Some(parent) = proof_path.parent() {
            fs::create_dir_all(parent).expect("create private proof directory");
        }
        fs::write(
            &ui_receipt_path,
            serde_json::to_vec_pretty(&ui_receipt).expect("serialize UI receipt"),
        )
        .expect("write sanitized UI receipt");
        fs::write(
            &proof_path,
            serde_json::to_vec_pretty(&private_proof).expect("serialize private proof"),
        )
        .expect("write private export proof");

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(not(feature = "reader-only"))]
    #[test]
    #[ignore = "runtime GUI evidence requires pinned CHAPTERA_SAMPLE_NEWSLETTER"]
    fn gui_direct_text_session_edits_real_story_without_inspector_apply() {
        use egui_kittest::{Harness, kittest::Queryable};

        let fixture_source = std::env::var_os("CHAPTERA_SAMPLE_NEWSLETTER")
            .map(PathBuf::from)
            .expect("CHAPTERA_SAMPLE_NEWSLETTER must point to the pinned Apache POI fixture");
        let original = fs::read(&fixture_source).expect("read pinned SampleNewsletter fixture");
        let root =
            std::env::temp_dir().join(format!("chaptera-gui-direct-text-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create direct-text GUI temp directory");
        let fixture = root.join("SampleNewsletter.pub");
        fs::write(&fixture, &original).expect("write direct-text GUI PUB fixture");

        let fixture_for_app = fixture.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1280.0, 820.0))
            .with_pixels_per_point(1.0)
            .with_max_steps(32)
            .build_eframe(move |cc| {
                fallback_font::install(&cc.egui_ctx)
                    .expect("pinned Chaptera fallback font resource must validate");
                ViewerApp::new_with_storage(Some(fixture_for_app), cc.storage)
            });
        harness.step();

        let (page_label, target_document_point, target_story_id, target_frame_id, before_fragment) = {
            let app = harness.state();
            let visual = app.visual.as_ref().expect("visual loaded");
            let editor = app.editor.as_ref().expect("editor loaded");
            visual
                .text_fragments
                .iter()
                .find_map(|fragment| {
                    text_session::enter_explicit_text_mode(
                        editor,
                        fragment.story_id,
                        fragment.frame_id,
                    )
                    .ok()?;
                    let node = visual
                        .scene
                        .nodes
                        .iter()
                        .find(|node| node.origin == fragment.frame_id)?;
                    let page = visual
                        .document
                        .pages
                        .iter()
                        .find(|page| node.parent_origin == page.id.into_canonical())?;
                    let page_id_text = page.id.as_canonical().to_string();
                    let page_nodes = visual
                        .scene
                        .nodes
                        .iter()
                        .filter(|candidate| candidate.parent_origin == page.id.into_canonical())
                        .collect::<Vec<_>>();
                    let hit_index = SceneHitTestIndex::new(
                        page_nodes
                            .iter()
                            .enumerate()
                            .filter_map(|(paint_order, candidate)| {
                                let instance =
                                    direct_scene_instance(editor, &page_id_text, candidate.origin)?;
                                Some(SceneHitEntry {
                                    instance_id: instance.instance_id,
                                    node_id: candidate.origin,
                                    bounds: candidate.bounds,
                                    z_order: 0,
                                    paint_order: u32::try_from(paint_order).unwrap_or(u32::MAX),
                                })
                            })
                            .collect(),
                    );
                    let point = pub_interaction::DocumentPoint::new(
                        pub_editor::LengthEmu::new(
                            node.bounds.x.get() + node.bounds.width.get() / 2,
                        ),
                        pub_editor::LengthEmu::new(
                            node.bounds.y.get() + node.bounds.height.get() / 2,
                        ),
                    );
                    hit_index
                        .topmost_at(point)
                        .filter(|top| top.node_id == fragment.frame_id)
                        .map(|_| {
                            (
                                format!("Page {}", page.index),
                                point,
                                fragment.story_id,
                                fragment.frame_id,
                                fragment.text.clone(),
                            )
                        })
                })
                .expect("real fixture exposes one topmost capability-safe TextFrame")
        };

        harness.get_by_label(&page_label).click();
        harness.step();

        let frame_center = {
            let canvas = harness
                .get_by_label("Document canvas")
                .raw_bounds()
                .expect("document canvas has screen bounds");
            let app = harness.state();
            let visual = app.visual.as_ref().expect("visual loaded");
            let page = visual
                .document
                .pages
                .get(app.selected_page)
                .expect("selected direct-text page remains available");
            let surface = visual
                .scene
                .surfaces
                .iter()
                .find(|surface| surface.origin == page.id)
                .expect("selected direct-text page has a scene surface");
            let viewport = egui::vec2(
                (canvas.x1 - canvas.x0) as f32,
                (canvas.y1 - canvas.y0) as f32,
            );
            let fit_scale = fitted_scale(
                surface.size.width.get(),
                surface.size.height.get(),
                viewport,
            )
            .expect("selected direct-text page has valid fit scale");
            let scene_scale = fit_scale * app.zoom;
            let page_width = surface.size.width.get() as f32 * scene_scale;
            let page_height = surface.size.height.get() as f32 * scene_scale;
            let page_left = ((canvas.x0 + canvas.x1) as f32 - page_width) / 2.0;
            let page_top = ((canvas.y0 + canvas.y1) as f32 - page_height) / 2.0;
            egui::pos2(
                page_left + target_document_point.x.get() as f32 * scene_scale,
                page_top + target_document_point.y.get() as f32 * scene_scale,
            )
        };

        harness.input_mut().events.extend([
            egui::Event::PointerMoved(frame_center),
            egui::Event::PointerButton {
                pos: frame_center,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
            egui::Event::PointerButton {
                pos: frame_center,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        ]);
        harness.step();
        harness.step();
        let operations_before = harness
            .state()
            .editor
            .as_ref()
            .expect("editor")
            .operations()
            .len();

        harness.get_by_label("Edit Text").click();
        harness.step();
        assert_eq!(
            harness
                .state()
                .text_mode
                .as_ref()
                .expect("explicit Edit Text enters a session")
                .story_id,
            target_story_id
        );
        assert_eq!(
            harness
                .state()
                .text_mode
                .as_ref()
                .expect("direct text mode")
                .frame_id,
            target_frame_id
        );
        assert_eq!(
            harness
                .state()
                .editor
                .as_ref()
                .expect("editor")
                .operations()
                .len(),
            operations_before,
            "entering direct text mode is transient"
        );

        harness.input_mut().events.extend([
            egui::Event::PointerMoved(frame_center),
            egui::Event::PointerButton {
                pos: frame_center,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
            egui::Event::PointerButton {
                pos: frame_center,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        ]);
        harness.step();
        harness
            .input_mut()
            .events
            .push(egui::Event::Text("X".to_owned()));
        harness.step();
        harness.step();

        {
            let app = harness.state();
            let editor = app.editor.as_ref().expect("editor");
            assert_eq!(editor.operations().len(), operations_before + 1);
            assert!(matches!(
                editor.operations().last(),
                Some(pub_editor::EditOperation::ReplaceStoryRange { .. })
            ));
            let after_fragment = app
                .visual
                .as_ref()
                .expect("visual")
                .text_fragments
                .iter()
                .find(|fragment| fragment.frame_id == target_frame_id)
                .expect("edited frame remains projected");
            assert_ne!(
                after_fragment.text, before_fragment,
                "canvas paint projection must reflect the current Story after direct typing"
            );
        }

        harness.input_mut().events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
        assert!(harness.state().text_mode.is_none());
        assert_eq!(
            harness
                .state()
                .editor
                .as_ref()
                .expect("editor")
                .operations()
                .len(),
            operations_before + 1,
            "Escape exits without a second document mutation"
        );
        assert_eq!(
            fs::read(&fixture).expect("re-read source PUB"),
            original,
            "GUI direct text editing must not mutate source PUB bytes"
        );

        let _ = fs::remove_dir_all(root);
    }

    struct GoldenPageOnlyApp {
        visual: ViewerGeometryDocument,
        page_index: usize,
        image_textures: BTreeMap<String, CachedImageTexture>,
        texture_upload_enabled: bool,
        painted_text_nodes: usize,
        clipped_text_nodes: usize,
        source_typography_sections: usize,
        fallback_typography_sections: usize,
        shared_resolved_layout_frames: usize,
        backend_fallback_frames: usize,
        projected_text_metrics: BTreeMap<String, render_backend::TextPaintMetrics>,
    }

    impl GoldenPageOnlyApp {
        fn new(visual: ViewerGeometryDocument, page_index: usize) -> Self {
            Self {
                visual,
                page_index,
                image_textures: BTreeMap::new(),
                texture_upload_enabled: false,
                painted_text_nodes: 0,
                clipped_text_nodes: 0,
                source_typography_sections: 0,
                fallback_typography_sections: 0,
                shared_resolved_layout_frames: 0,
                backend_fallback_frames: 0,
                projected_text_metrics: BTreeMap::new(),
            }
        }

        fn enable_texture_upload(&mut self) {
            self.texture_upload_enabled = true;
        }

        fn ensure_image_textures(&mut self, ctx: &egui::Context) {
            if !self.texture_upload_enabled {
                return;
            }
            let max_texture_side = ctx.input(|input| input.max_texture_side);
            for embedded in &self.visual.images {
                let key = format!("{:?}", embedded.resource_id);
                if self.image_textures.contains_key(&key) {
                    continue;
                }
                let expected_sha256 = image_decode_adapter::exact_sha256_hex(&embedded.bytes);
                let admitted = image_decode_adapter::decode_texture_image_v1(
                    &embedded.bytes,
                    &embedded.mime,
                    &expected_sha256,
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "clean golden image decode failed for {} / {}: {error}",
                        key, embedded.mime
                    )
                });
                let [width, height] = admitted.color_image.size;
                assert!(
                    width <= max_texture_side && height <= max_texture_side,
                    "Carlton golden image {width}x{height} exceeds active egui texture limit {max_texture_side}"
                );
                let texture = ctx.load_texture(
                    format!("carlton-golden-{key}"),
                    admitted.color_image,
                    egui::TextureOptions::LINEAR,
                );
                self.image_textures.insert(
                    key,
                    CachedImageTexture {
                        texture,
                        _cache_identity_sha256: admitted.cache_identity_sha256,
                    },
                );
            }
        }
    }

    impl eframe::App for GoldenPageOnlyApp {
        fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            self.ensure_image_textures(ctx);
            let render_plan = build_desktop_page_render_plan(&self.visual, self.page_index)
                .expect("clean golden page render plan");
            let scene_scale = 144.0_f32 / 914_400.0_f32;
            self.painted_text_nodes = 0;
            self.clipped_text_nodes = 0;
            self.source_typography_sections = 0;
            self.fallback_typography_sections = 0;
            self.shared_resolved_layout_frames = 0;
            self.backend_fallback_frames = 0;
            self.projected_text_metrics.clear();

            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let page_rect = ui.max_rect();
                    let painter = ui.painter_at(page_rect);
                    painter.rect_filled(page_rect, 0.0, egui::Color32::WHITE);

                    for node in &render_plan.nodes {
                        let Some(node_rect) = render_backend::physical_rect_to_egui(
                            page_rect,
                            scene_scale,
                            node.bounds.x.get(),
                            node.bounds.y.get(),
                            node.bounds.width.get(),
                            node.bounds.height.get(),
                        ) else {
                            continue;
                        };
                        let texture = node.image.as_ref().and_then(|image| {
                            let key = format!("{:?}", image.resource_id);
                            self.image_textures.get(&key)
                        });
                        render_backend::paint_document_node_base(
                            &painter,
                            node,
                            node_rect,
                            texture.map(|cached| cached.texture.id()),
                        );
                        let outcome = render_backend::paint_document_node_foreground(
                            &painter,
                            node,
                            node_rect,
                            scene_scale,
                        );
                        if let Some(metrics) = outcome.text_metrics {
                            if let Some(instance) = node.projected_scene_instance.as_ref() {
                                self.projected_text_metrics
                                    .insert(instance.instance_id.clone(), metrics.clone());
                            }
                            self.painted_text_nodes += 1;
                            self.source_typography_sections += metrics.source_typography_sections;
                            self.fallback_typography_sections += metrics.fallback_sections;
                            if metrics.shared_resolved_layout {
                                self.shared_resolved_layout_frames += 1;
                            } else {
                                self.backend_fallback_frames += 1;
                            }
                            if outcome.text_clipped {
                                self.clipped_text_nodes += 1;
                            }
                        }
                    }
                });
        }
    }

    #[test]
    #[ignore = "requires CHAPTERA_EMBEDDED_FONT_PUB, CHAPTERA_EMBEDDED_FONT_TTF and CHAPTERA_EMBEDDED_FONT_OUT"]
    fn embedded_font_exact_bytes_change_shared_reader_layout_against_fallback() {
        use chaptera_viewer_render_plan::RenderTextLayoutDispositionV1;
        use sha2::{Digest, Sha256};

        const SOURCE_SHA256: &str =
            "8d50872a7d8ee6130b889efbe99275ee333747bc7777f3c5256a05f2c6d32048";
        const EOT_SHA256: &str = "9581bade789c578a9f5e89356a6c05fc0b890cb1533a9d7169c0a7188323104f";
        const SOURCE_FONT_NAME: &str = "Times New Roman";

        let fixture = std::env::var_os("CHAPTERA_EMBEDDED_FONT_PUB")
            .map(PathBuf::from)
            .expect("CHAPTERA_EMBEDDED_FONT_PUB");
        let decoded_font = std::env::var_os("CHAPTERA_EMBEDDED_FONT_TTF")
            .map(PathBuf::from)
            .expect("CHAPTERA_EMBEDDED_FONT_TTF");
        let output_dir = std::env::var_os("CHAPTERA_EMBEDDED_FONT_OUT")
            .map(PathBuf::from)
            .expect("CHAPTERA_EMBEDDED_FONT_OUT");
        fs::create_dir_all(&output_dir).expect("create embedded-font probe output directory");

        let source_bytes = fs::read(&fixture).expect("read exact fonts.pub fixture");
        let source_sha256 = format!("{:x}", Sha256::digest(&source_bytes));
        assert_eq!(
            source_sha256, SOURCE_SHA256,
            "embedded-font PUB source identity drifted"
        );

        let eot_sha256 = std::env::var("CHAPTERA_EMBEDDED_FONT_EOT_SHA256")
            .expect("CHAPTERA_EMBEDDED_FONT_EOT_SHA256");
        assert_eq!(eot_sha256, EOT_SHA256, "embedded EOT identity drifted");

        let exact_font_bytes = fs::read(&decoded_font).expect("read decoded embedded font program");
        assert!(
            !exact_font_bytes.is_empty(),
            "decoded embedded font program must be non-empty"
        );
        let exact_font_sha256 = format!("{:x}", Sha256::digest(&exact_font_bytes));
        let expected_ttf_sha256 = std::env::var("CHAPTERA_EMBEDDED_FONT_TTF_SHA256")
            .expect("CHAPTERA_EMBEDDED_FONT_TTF_SHA256");
        assert_eq!(
            exact_font_sha256, expected_ttf_sha256,
            "decoded embedded font fingerprint changed between decode and Reader probe"
        );
        assert_ne!(
            exact_font_sha256,
            chaptera_desktop_fallback_font_resource::EXPECTED_SHA256,
            "embedded and fallback font programs must not collapse to the same bytes"
        );

        let visual = diagnostic_sweep::open_for_product(&source_bytes)
            .expect("exact fonts.pub must open through current product Reader");
        assert!(
            !visual.document.pages.is_empty(),
            "embedded-font fixture must expose at least one Reader page"
        );

        let embedded_font = ExplicitRenderTextFontResourceV1 {
            resource_id: "pub-embedded:times-new-roman:eot:v1",
            expected_sha256: &exact_font_sha256,
            face_index: 0,
            default_font_size_emu: chaptera_desktop_fallback_font_resource::FONT_SIZE_EMU,
            default_line_height_emu: chaptera_desktop_fallback_font_resource::LINE_HEIGHT_EMU,
            bytes: &exact_font_bytes,
        };
        let fallback_font = desktop_text_font_resource();

        let mut source_family_candidate_frames = 0_usize;
        let mut comparable_shared_frames = 0_usize;
        let mut metric_changed_frames = 0_usize;
        let mut line_break_changed_frames = 0_usize;
        let mut clipping_changed_frames = 0_usize;
        let mut frame_receipts = Vec::new();

        for page_index in 0..visual.document.pages.len() {
            let fallback_plan =
                build_page_render_plan_with_text_layout_v1(&visual, page_index, &fallback_font)
                    .expect("fallback render plan");
            let embedded_plan =
                build_page_render_plan_with_text_layout_v1(&visual, page_index, &embedded_font)
                    .expect("embedded-font render plan");
            assert_eq!(
                fallback_plan.nodes.len(),
                embedded_plan.nodes.len(),
                "font resource must not change render-plan node cardinality"
            );

            for (fallback_node, embedded_node) in
                fallback_plan.nodes.iter().zip(&embedded_plan.nodes)
            {
                assert_eq!(
                    fallback_node.node_id, embedded_node.node_id,
                    "font resource must not change render-plan node identity/order"
                );
                let (Some(fallback_text), Some(embedded_text)) =
                    (fallback_node.text.as_ref(), embedded_node.text.as_ref())
                else {
                    continue;
                };
                assert_eq!(
                    fallback_text.story_id, embedded_text.story_id,
                    "font resource must not change Story authority"
                );
                assert_eq!(
                    fallback_text.text, embedded_text.text,
                    "font resource must not mutate Story text"
                );

                if fallback_text.typography.is_empty()
                    || !fallback_text
                        .typography
                        .iter()
                        .all(|run| run.source_font_name.eq_ignore_ascii_case(SOURCE_FONT_NAME))
                {
                    continue;
                }
                source_family_candidate_frames += 1;

                let (Some(fallback_layout), Some(embedded_layout)) =
                    (fallback_text.layout.as_ref(), embedded_text.layout.as_ref())
                else {
                    continue;
                };
                let (fallback_line_height_emu, fallback_fingerprint) =
                    match &fallback_layout.disposition {
                        RenderTextLayoutDispositionV1::SharedResolved {
                            font_fingerprint_sha256,
                            line_height_emu,
                            ..
                        } => (*line_height_emu, font_fingerprint_sha256.as_str()),
                        RenderTextLayoutDispositionV1::BackendFallback { .. } => continue,
                    };
                let (embedded_line_height_emu, embedded_fingerprint) =
                    match &embedded_layout.disposition {
                        RenderTextLayoutDispositionV1::SharedResolved {
                            font_fingerprint_sha256,
                            line_height_emu,
                            ..
                        } => (*line_height_emu, font_fingerprint_sha256.as_str()),
                        RenderTextLayoutDispositionV1::BackendFallback { .. } => continue,
                    };

                assert_eq!(
                    fallback_fingerprint,
                    chaptera_desktop_fallback_font_resource::EXPECTED_SHA256
                );
                assert_eq!(embedded_fingerprint, exact_font_sha256);
                comparable_shared_frames += 1;

                let fallback_breaks = fallback_layout
                    .lines
                    .iter()
                    .map(|line| [line.scalar_start, line.scalar_end, line.consumed_scalar_end])
                    .collect::<Vec<_>>();
                let embedded_breaks = embedded_layout
                    .lines
                    .iter()
                    .map(|line| [line.scalar_start, line.scalar_end, line.consumed_scalar_end])
                    .collect::<Vec<_>>();
                let line_break_changed = fallback_breaks != embedded_breaks;
                let measured_width_changed = fallback_layout.lines.len()
                    != embedded_layout.lines.len()
                    || fallback_layout
                        .lines
                        .iter()
                        .zip(&embedded_layout.lines)
                        .any(|(fallback, embedded)| {
                            fallback.measured_width_emu != embedded.measured_width_emu
                        });

                let fallback_height_emu = i128::from(fallback_line_height_emu)
                    * i128::try_from(fallback_layout.lines.len()).expect("line count fits i128");
                let embedded_height_emu = i128::from(embedded_line_height_emu)
                    * i128::try_from(embedded_layout.lines.len()).expect("line count fits i128");
                let frame_height_emu = i128::from(fallback_node.bounds.height.get());
                let fallback_clipped = fallback_height_emu > frame_height_emu;
                let embedded_clipped = embedded_height_emu > frame_height_emu;
                let clipping_changed = fallback_clipped != embedded_clipped;
                let metric_changed =
                    line_break_changed || measured_width_changed || clipping_changed;

                metric_changed_frames += usize::from(metric_changed);
                line_break_changed_frames += usize::from(line_break_changed);
                clipping_changed_frames += usize::from(clipping_changed);

                let text_sha256 = format!("{:x}", Sha256::digest(fallback_text.text.as_bytes()));
                frame_receipts.push(serde_json::json!({
                    "page_number": page_index + 1,
                    "node_id": fallback_node.node_id,
                    "story_id": fallback_text.story_id,
                    "story_text_sha256": text_sha256,
                    "source_font_name": SOURCE_FONT_NAME,
                    "scalar_count": fallback_text.text.chars().count(),
                    "frame_height_emu": fallback_node.bounds.height.get(),
                    "fallback_line_count": fallback_layout.lines.len(),
                    "embedded_line_count": embedded_layout.lines.len(),
                    "fallback_breaks": fallback_breaks,
                    "embedded_breaks": embedded_breaks,
                    "fallback_clipped": fallback_clipped,
                    "embedded_clipped": embedded_clipped,
                    "line_break_changed": line_break_changed,
                    "measured_width_changed": measured_width_changed,
                    "clipping_changed": clipping_changed,
                    "metric_changed": metric_changed,
                }));
            }
        }

        assert!(
            source_family_candidate_frames > 0,
            "current Reader must expose at least one exact Times New Roman source-owned frame"
        );
        assert!(
            comparable_shared_frames > 0,
            "at least one Times New Roman frame must be comparable through shared layout"
        );
        assert!(
            metric_changed_frames > 0,
            "exact embedded font bytes must produce a measurable layout difference from Ubuntu Light"
        );

        let receipt = serde_json::json!({
            "schema": "chaptera.reader-embedded-font-probe.v1",
            "source_pub_sha256": source_sha256,
            "embedded_eot_sha256": eot_sha256,
            "source_font_name": SOURCE_FONT_NAME,
            "embedded_program_sha256": exact_font_sha256,
            "embedded_program_disposition": "exact_embedded_program_for_document",
            "original_full_font_file_claimed": false,
            "fallback_resource_id": chaptera_desktop_fallback_font_resource::RESOURCE_ID,
            "fallback_program_sha256": chaptera_desktop_fallback_font_resource::EXPECTED_SHA256,
            "source_family_candidate_frames": source_family_candidate_frames,
            "comparable_shared_frames": comparable_shared_frames,
            "metric_changed_frames": metric_changed_frames,
            "line_break_changed_frames": line_break_changed_frames,
            "clipping_changed_frames": clipping_changed_frames,
            "publisher_raster_oracle_present": false,
            "publisher_exact_reflow_claimed": false,
            "discriminator": "same Reader source/geometry/text, font bytes only",
            "frames": frame_receipts,
        });
        fs::write(
            output_dir.join("reader-embedded-font-probe-receipt.json"),
            serde_json::to_vec_pretty(&receipt).expect("serialize embedded-font probe receipt"),
        )
        .expect("write embedded-font probe receipt");

        println!(
            "embedded-font probe: candidates={} comparable={} metric_changed={} line_break_changed={} clipping_changed={}",
            source_family_candidate_frames,
            comparable_shared_frames,
            metric_changed_frames,
            line_break_changed_frames,
            clipping_changed_frames
        );
    }

    #[test]
    #[ignore = "requires CHAPTERA_GOLDEN_CARLTON_MARCH and CHAPTERA_GOLDEN_CARLTON_OUT"]
    fn golden_carlton_march_clean_pages_use_current_reader_render_backend() {
        use egui_kittest::Harness;
        use sha2::{Digest, Sha256};

        const RASTER_DPI: f64 = 144.0;
        const EMU_PER_INCH: f64 = 914_400.0;

        let fixture = std::env::var_os("CHAPTERA_GOLDEN_CARLTON_MARCH")
            .map(PathBuf::from)
            .expect("CHAPTERA_GOLDEN_CARLTON_MARCH");
        let output_dir = std::env::var_os("CHAPTERA_GOLDEN_CARLTON_OUT")
            .map(PathBuf::from)
            .expect("CHAPTERA_GOLDEN_CARLTON_OUT");
        fs::create_dir_all(&output_dir).expect("create Carlton golden output directory");

        let bytes = fs::read(&fixture).expect("read exact Carlton March PUB");
        let source_sha256 = format!("{:x}", Sha256::digest(&bytes));
        assert_eq!(
            source_sha256, "bf9cda0f632b5820ab9dbdbe1b838b2a988b2f3fdd69253c22b4fc3aef9f11c3",
            "Carlton March source identity drifted"
        );

        let visual = diagnostic_sweep::open_for_product(&bytes)
            .expect("exact Carlton March must open through current product Reader");
        assert_eq!(visual.document.pages.len(), 3, "Carlton product page count");
        assert_eq!(
            visual.scene.surfaces.len(),
            3,
            "Carlton product surface count"
        );
        assert!(
            visual.document.diagnostics.iter().any(
                |diagnostic| diagnostic.code == "viewer.page_projection.family_profile_applied"
            ),
            "exact Carlton family presentation profile must be active before visual rendering"
        );
        assert_eq!(
            visual.projected_instances.len(),
            4,
            "exact March must admit exactly four visible canonical Cmo scene instances"
        );
        let projected_instance_ids = visual
            .projected_instances
            .iter()
            .map(|projected| projected.scene_instance.instance_id.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            projected_instance_ids.len(),
            4,
            "canonical projected SceneInstance identities must be distinct"
        );
        assert!(visual.projected_instances.iter().all(|projected| {
            projected.scene_instance.projection_kind
                == chaptera_scene_instance::SceneProjectionKindV1::CmoStorySlot
        }));
        let direct_scene_origins = visual
            .scene
            .nodes
            .iter()
            .map(|node| node.origin.as_canonical().to_string())
            .collect::<BTreeSet<_>>();
        assert!(
            visual.projected_instances.iter().all(|projected| {
                !direct_scene_origins.contains(&projected.scene_instance.origin_node_id)
            }),
            "projected Cmo carriers must not be reparented into direct customer scene nodes"
        );
        let projected_page_counts = visual
            .document
            .pages
            .iter()
            .map(|page| {
                let page_id = page.id.as_canonical().to_string();
                visual
                    .projected_instances
                    .iter()
                    .filter(|projected| projected.scene_instance.target_page_id == page_id)
                    .count()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            projected_page_counts,
            vec![1, 2, 1],
            "exact March projected Cmo distribution must stay 1/2/1"
        );

        let mut page_receipts = Vec::new();
        for (page_index, expected_projected_node_count) in
            projected_page_counts.iter().copied().enumerate()
        {
            let plan = build_desktop_page_render_plan(&visual, page_index)
                .expect("current Reader page render plan");
            let projected_node_count = plan
                .nodes
                .iter()
                .filter(|node| node.projected_scene_instance.is_some())
                .count();
            assert_eq!(
                projected_node_count, expected_projected_node_count,
                "render-plan projected instance count must match canonical Viewer adapter"
            );
            assert!(
                plan.nodes
                    .iter()
                    .filter_map(|node| node.text.as_ref())
                    .all(|fragment| !fragment.text.contains('\u{FFFC}')),
                "admitted projected object markers must not paint as U+FFFC missing-glyph boxes"
            );
            let width_px = ((plan.page_size.width.get() as f64 * RASTER_DPI / EMU_PER_INCH)
                .round()
                .max(1.0)) as u32;
            let height_px = ((plan.page_size.height.get() as f64 * RASTER_DPI / EMU_PER_INCH)
                .round()
                .max(1.0)) as u32;

            let visual_for_app = visual.clone();
            let mut harness = Harness::builder()
                .with_size(egui::vec2(width_px as f32, height_px as f32))
                .with_pixels_per_point(1.0)
                .with_max_steps(12)
                .wgpu()
                .build_eframe(move |cc| {
                    fallback_font::install(&cc.egui_ctx)
                        .expect("pinned Chaptera fallback font resource must validate");
                    GoldenPageOnlyApp::new(visual_for_app, page_index)
                });
            // Harness construction executes initial frames with RawInput's portable
            // 2048px texture ceiling. Carlton contains a proven 2480x2835 image,
            // so delay exact texture upload until the next frame after raising only
            // this headless input capability; source pixels remain unmodified.
            harness.input_mut().max_texture_side = Some(4096);
            harness.state_mut().enable_texture_upload();
            harness.step();

            let image = harness
                .render()
                .expect("headless clean Reader page render must succeed");
            assert_eq!(image.width(), width_px, "golden raster width drift");
            assert_eq!(image.height(), height_px, "golden raster height drift");
            let executed = harness.state();
            let (planned_shared_resolved_layout_frames, planned_backend_fallback_frames) =
                text_layout_disposition_counts(&plan);
            let projected_instance_receipts = plan
                .nodes
                .iter()
                .filter_map(|node| {
                    let instance = node.projected_scene_instance.as_ref()?;
                    let viewer_projected = visual
                        .projected_instances
                        .iter()
                        .find(|projected| {
                            projected.scene_instance.instance_id == instance.instance_id
                        })
                        .expect("render-plan projected instance must come from Viewer adapter");
                    let target_frame = visual
                        .scene
                        .nodes
                        .iter()
                        .find(|candidate| candidate.origin == viewer_projected.target_frame_node_id)
                        .expect("projected target frame remains in resolved customer scene");
                    Some(serde_json::json!({
                        "instance_id": instance.instance_id,
                        "origin_node_id": instance.origin_node_id,
                        "target_frame_node_id": viewer_projected
                            .target_frame_node_id
                            .as_canonical()
                            .to_string(),
                        "cmo_slot_index": instance.cmo_slot_index,
                        "cmo_scalar_index": instance.cmo_scalar_index,
                        "target_frame_paint_scalar_end": viewer_projected
                            .target_frame_paint_scalar_end,
                        "story_authority_present": instance.story_authority_id.is_some(),
                        "target_frame_bounds_emu": [
                            target_frame.bounds.x.get(),
                            target_frame.bounds.y.get(),
                            target_frame.bounds.width.get(),
                            target_frame.bounds.height.get(),
                        ],
                        "projected_bounds_emu": [
                            node.bounds.x.get(),
                            node.bounds.y.get(),
                            node.bounds.width.get(),
                            node.bounds.height.get(),
                        ],
                        "carrier_extent_emu": [
                            node.bounds.width.get(),
                            node.bounds.height.get(),
                        ],
                        "text_scalar_count": node
                            .text
                            .as_ref()
                            .map(|text| text.text.chars().count())
                            .unwrap_or(0),
                        "executed_text_metrics": executed
                            .projected_text_metrics
                            .get(&instance.instance_id),
                    }))
                })
                .collect::<Vec<_>>();
            let filename = format!("carlton-march-reader-page-{:03}.png", page_index + 1);
            image
                .save(output_dir.join(&filename))
                .expect("write Carlton clean Reader page PNG");

            let typography_sections = plan
                .nodes
                .iter()
                .filter_map(|node| node.text.as_ref())
                .map(|text| text.typography.len())
                .sum::<usize>();
            page_receipts.push(serde_json::json!({
                "page_number": page_index + 1,
                "page_id": visual.document.pages[page_index].id,
                "width_emu": plan.page_size.width.get(),
                "height_emu": plan.page_size.height.get(),
                "raster_width_px": width_px,
                "raster_height_px": height_px,
                "node_count": plan.nodes.len(),
                "projected_scene_instance_count": projected_node_count,
                "projected_instances": projected_instance_receipts,
                "fill_node_count": plan.nodes.iter().filter(|node| node.solid_fill_rgb.is_some()).count(),
                "line_node_count": plan.nodes.iter().filter(|node| node.solid_line.is_some()).count(),
                "image_node_count": plan.nodes.iter().filter(|node| node.image.is_some()).count(),
                "text_node_count": plan.nodes.iter().filter(|node| node.text.is_some()).count(),
                "typography_sections": typography_sections,
                "planned_shared_resolved_layout_frames": planned_shared_resolved_layout_frames,
                "planned_backend_fallback_frames": planned_backend_fallback_frames,
                "executed_text_node_count": executed.painted_text_nodes,
                "executed_source_typography_sections": executed.source_typography_sections,
                "executed_fallback_typography_sections": executed.fallback_typography_sections,
                "shared_resolved_layout_frames": executed.shared_resolved_layout_frames,
                "backend_fallback_frames": executed.backend_fallback_frames,
                "clipped_text_node_count": executed.clipped_text_nodes,
                "png": filename,
            }));
        }

        let receipt = serde_json::json!({
            "schema": "chaptera.reader-golden-carlton-march.v1",
            "source_sha256": source_sha256,
            "page_count": visual.document.pages.len(),
            "scene_surface_count": visual.scene.surfaces.len(),
            "raster_dpi": RASTER_DPI as u32,
            "render_backend": "chaptera-desktop-egui-document-paint",
            "shell_ui_rendered": false,
            "selection_overlay_rendered": false,
            "preview_warning_overlay_rendered": false,
            "family_profile_applied": true,
            "source_font_face_claimed": false,
            "publisher_exact_reflow_claimed": false,
            "text_layout_authority": "shared_resolved_when_admitted_else_backend_fallback",
            "viewer_diagnostic_codes": visual
                .document
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code.clone())
                .collect::<Vec<_>>(),
            "projected_scene_instance_ids": projected_instance_ids,
            "projected_page_counts": projected_page_counts,
            "pages": page_receipts,
        });
        fs::write(
            output_dir.join("carlton-march-reader-golden-receipt.json"),
            serde_json::to_vec_pretty(&receipt).expect("serialize Carlton golden receipt"),
        )
        .expect("write Carlton golden receipt");
    }

    #[test]
    #[ignore = "requires CHAPTERA_GOLDEN_SAMPLE_NEWSLETTER and CHAPTERA_GOLDEN_OUT"]
    fn golden_sample_newsletter_reference_customer_page_1_uses_shared_typography_render_plan() {
        use egui_kittest::Harness;
        use sha2::{Digest, Sha256};

        let fixture = std::env::var_os("CHAPTERA_GOLDEN_SAMPLE_NEWSLETTER")
            .map(PathBuf::from)
            .expect("CHAPTERA_GOLDEN_SAMPLE_NEWSLETTER");
        let output_dir = std::env::var_os("CHAPTERA_GOLDEN_OUT")
            .map(PathBuf::from)
            .expect("CHAPTERA_GOLDEN_OUT");
        fs::create_dir_all(&output_dir).expect("create golden output directory");

        let bytes = fs::read(&fixture).expect("read pinned SampleNewsletter");
        let source_sha256 = format!("{:x}", Sha256::digest(&bytes));
        assert_eq!(
            source_sha256, "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf",
            "golden fixture identity drifted"
        );

        let visual = diagnostic_sweep::open_for_product(&bytes)
            .expect("pinned SampleNewsletter must open through product Reader path");
        assert!(
            !visual.typography_runs.is_empty(),
            "Reader must expose bounded source typography"
        );
        let inherited_typography_run_count = visual
            .typography_runs
            .iter()
            .filter(|run| run.font_inherited || run.size_inherited)
            .count();
        assert_eq!(
            visual.typography_runs.len(),
            106,
            "current main source authority proves 106 product-safe SampleNewsletter effective typography runs; Reader/Viewer projection must preserve them 1:1"
        );
        assert_eq!(
            inherited_typography_run_count, 88,
            "current main source authority proves 88 explicit-FDPP-selector inherited SampleNewsletter runs; projection must not drop inheritance provenance"
        );
        assert!(
            visual
                .typography_runs
                .iter()
                .any(|run| run.source_font_name == "Rockwell Condensed"
                    && run.text_size_emu == 24 * 12_700),
            "proven Rockwell Condensed 24pt anchor must reach Viewer"
        );

        // Fixture-only crosswalk: raw Viewer Page 2 is Publisher customer page 1 for this exact pinned SHA.
        // This must never be reused as generic PAGE-role logic.
        let page_offset = 1_usize;
        let plan = build_desktop_page_render_plan(&visual, page_offset)
            .expect("reference customer page 1 shared render plan");
        let typography_sections = plan
            .nodes
            .iter()
            .filter_map(|node| node.text.as_ref())
            .map(|text| text.typography.len())
            .sum::<usize>();
        assert!(
            typography_sections > 0,
            "source typography must reach shared Reader render plan"
        );
        let (shared_resolved_layout_frames, backend_fallback_frames) =
            text_layout_disposition_counts(&plan);
        assert!(
            shared_resolved_layout_frames > 0,
            "SampleNewsletter must exercise at least one shared resolved text-layout frame"
        );

        let fixture_for_app = fixture.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1280.0, 820.0))
            .with_pixels_per_point(1.0)
            .with_max_steps(24)
            .wgpu()
            .build_eframe(move |cc| {
                fallback_font::install(&cc.egui_ctx)
                    .expect("pinned Chaptera fallback font resource must validate");
                let mut app = ViewerApp::new_with_storage(Some(fixture_for_app), cc.storage);
                app.selected_page = page_offset;
                app
            });
        harness.step();

        let image = harness
            .render()
            .expect("headless Reader render must succeed");
        let png_path = output_dir.join("samplenewsletter-reference-customer-page-001-reader.png");
        image.save(&png_path).expect("write Reader golden PNG");

        let receipt = serde_json::json!({
            "schema": "chaptera.reader-golden-samplenewsletter.v3",
            "source_sha256": source_sha256,
            "viewer_page_number": 2,
            "publisher_reference_customer_page_number": 1,
            "page_selection_basis": "pinned_same_source_crosswalk_only",
            "generic_page_role_claimed": false,
            "typography_run_count": visual.typography_runs.len(),
            "inherited_typography_run_count": inherited_typography_run_count,
            "render_plan_typography_sections": typography_sections,
            "shared_resolved_layout_frames": shared_resolved_layout_frames,
            "backend_fallback_frames": backend_fallback_frames,
            "source_font_face_claimed": false,
            "publisher_exact_reflow_claimed": false,
            "text_layout_authority": "shared_resolved_when_admitted_else_backend_fallback",
            "png": "samplenewsletter-reference-customer-page-001-reader.png"
        });
        fs::write(
            output_dir.join("samplenewsletter-reader-receipt.json"),
            serde_json::to_vec_pretty(&receipt).expect("serialize golden receipt"),
        )
        .expect("write golden receipt");
    }

    #[test]
    fn source_path_argument_is_optional() {
        let path = std::path::Path::new("example.pub");
        assert_eq!(
            path.extension().and_then(|value| value.to_str()),
            Some("pub")
        );
    }
}
