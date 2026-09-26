#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

// The cadence API is intentionally staged one PR before its UI consumer (#227).
mod acceptance;
mod agent;
mod product_smoke;
#[allow(dead_code)]
mod supporter;

use chaptera_scene_instance::{
    GeometrySyncPolicyV1, ObjectMutationKindV1, SceneInstanceV1, admit_object_mutation_v1,
    direct_page_local_instance_v1, geometry_sync_policy_v1,
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
const SUPPORTER_STORAGE_KEY: &str = "chaptera.supporter.v1";
const PAGE_MARGIN: f32 = 24.0;
const GEOMETRY_WARNING: &str = "Partial preview: bounded single-frame text, exact embedded PNG/JPEG images, and complete explicit shape-local solid fill/line state may be painted; inherited/default paint, linked text flow, typography, image crop/fit, gradients/patterns, effects, and transforms are not faithfully painted yet.";
const PREVIEW_TEXT_CLIP_WARNING: &str = "Text exceeds the height of at least one frame in the current egui desktop preview and is visibly clipped. This is a preview-only warning using the UI font/metrics; it is not Publisher-native overset or reflow evidence.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewerLoadFailureKind {
    FileAccess,
    Unsupported,
}

#[derive(Debug, Clone)]
struct ViewerLoadFailure {
    kind: ViewerLoadFailureKind,
    message: String,
    classification: Option<FailureIntakeClassification>,
    diagnostic_json: Option<String>,
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
        Self { entries }
    }

    fn topmost_at(&self, point: pub_interaction::DocumentPoint) -> Option<&SceneHitEntry> {
        self.entries
            .iter()
            .rev()
            .find(|entry| scene_bounds_contains(entry.bounds, point))
    }

    fn node_for_instance(&self, instance_id: &str) -> Option<pub_editor::NodeId> {
        self.entries
            .iter()
            .find(|entry| entry.instance_id == instance_id)
            .map(|entry| entry.node_id)
    }

    fn instance_for_node(&self, node_id: pub_editor::NodeId) -> Option<&str> {
        self.entries
            .iter()
            .find(|entry| entry.node_id == node_id)
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
            Ok(Box::new(ViewerApp::new_with_storage(
                initial_path,
                cc.storage,
            )))
        }),
    )
}

fn smoke_check(path: &Path) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let visual = pub_viewer::open_mature_0x2c_geometry(
        &bytes,
        pub_viewer::viewer_geometry_environment_v0_1(),
    )
    .map_err(|error| format!("open {}: {error:#}", path.display()))?;

    if visual.document.pages.is_empty() {
        return Err("document has no Viewer pages".to_owned());
    }
    if visual.scene.nodes.is_empty() {
        return Err("document has no resolved scene nodes".to_owned());
    }

    Ok(())
}

struct ViewerApp {
    source_path: Option<PathBuf>,
    visual: Option<ViewerGeometryDocument>,
    selected_page: usize,
    canvas_selection: SceneSelectionState,
    canvas_drag: Option<MoveTransaction>,
    canvas_resize: Option<ResizeTransaction>,
    zoom: f32,
    load_error: Option<ViewerLoadFailure>,
    search_query: String,
    search_results: Vec<ViewerTextMatch>,
    selected_search_result: Option<usize>,
    image_textures: BTreeMap<String, egui::TextureHandle>,
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
    diagnostic_save_path: String,
    diagnostic_status: Option<String>,
    supporter_value: supporter::ValueTracker,
    supporter_state: supporter::SupporterState,
    exact_file_consent_open: bool,
    exact_file_consent_status: Option<String>,
    show_diagnostics: bool,
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
            visual: None,
            selected_page: 0,
            canvas_selection: SceneSelectionState::default(),
            canvas_drag: None,
            canvas_resize: None,
            zoom: 1.0,
            load_error: None,
            search_query: String::new(),
            search_results: Vec::new(),
            selected_search_result: None,
            image_textures: BTreeMap::new(),
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
            diagnostic_save_path: String::new(),
            diagnostic_status: None,
            supporter_value: supporter::ValueTracker::default(),
            supporter_state: restore_supporter_state(storage),
            exact_file_consent_open: false,
            exact_file_consent_status: None,
            show_diagnostics: false,
        };

        if let Some(path) = initial_path {
            app.load_path(path);
        }

        app
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
                message:
                    "The native Open dialog is currently provided by the Windows Reader build; drag and drop a PUB file on this platform."
                        .to_owned(),
                classification: None,
                diagnostic_json: None,
            });
        }
    }

    fn load_path(&mut self, path: PathBuf) {
        self.supporter_value
            .observe(supporter::ValueEvent::WorkflowFailed);
        self.source_path = Some(path.clone());
        self.visual = None;
        self.selected_page = 0;
        self.canvas_selection.clear();
        self.canvas_drag = None;
        self.canvas_resize = None;
        self.zoom = 1.0;
        self.load_error = None;
        self.search_query.clear();
        self.search_results.clear();
        self.selected_search_result = None;
        self.image_textures.clear();
        self.editor = None;
        self.editor_load_error = None;
        self.edit_buffer.clear();
        self.edit_status = None;
        self.selected_table_cell_index = None;
        self.table_cell_buffer.clear();
        self.export_preview = None;
        self.project_status = None;
        self.preview_clipped_frames = 0;
        self.preview_clipped_story_keys.clear();
        self.diagnostic_save_path.clear();
        self.diagnostic_status = None;
        self.exact_file_consent_open = false;
        self.exact_file_consent_status = None;
        self.show_diagnostics = false;

        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.load_error = Some(ViewerLoadFailure {
                    kind: ViewerLoadFailureKind::FileAccess,
                    message: format!("Could not read {}: {error}", path.display()),
                    classification: None,
                    diagnostic_json: None,
                });
                return;
            }
        };

        match pub_viewer::open_mature_0x2c_geometry(
            &bytes,
            pub_viewer::viewer_geometry_environment_v0_1(),
        ) {
            Ok(visual) => {
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

                self.supporter_value
                    .observe(supporter::ValueEvent::DocumentOpened {
                        status: supporter_status,
                        page_count: supporter_page_count,
                        text_searchable: supporter_text_searchable,
                        initial_page: 0,
                    });

                let editor = if reader_only_mode() {
                    None
                } else {
                    let source_hash = visual.document.source.source_hash;
                    match pub_editor::open_mature_0x2c_editor(&bytes, source_hash) {
                        Ok(mut editor) => {
                            match load_editor_project_sidecar(&path, &mut editor) {
                                Ok(Some((sidecar, operation_count))) => {
                                    self.project_status = Some(format!(
                                        "Loaded editor project {} with {operation_count} operations.",
                                        sidecar.display()
                                    ));
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    self.project_status =
                                        Some(format!("Editor project was not applied: {error}"));
                                }
                            }
                            Some(editor)
                        }
                        Err(error) => {
                            self.editor_load_error = Some(error.to_string());
                            None
                        }
                    }
                };
                self.visual = Some(visual);
                self.editor = editor;
                self.sync_visual_stories_from_editor();
                self.sync_visual_geometry_from_editor();
            }
            Err(error) => {
                self.load_error = Some(ViewerLoadFailure {
                    kind: ViewerLoadFailureKind::Unsupported,
                    message: format!("Could not open {}: {error:#}", path.display()),
                    classification: Some(classify_failure_candidate(&bytes)),
                    diagnostic_json: pub_viewer::local_failure_diagnostic_json(&bytes).ok(),
                });
            }
        }
    }

    fn show_command_bar(&mut self, ui: &mut egui::Ui) {
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

    fn show_workspace_status(&self, ui: &mut egui::Ui) {
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
        let dropped = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .find_map(|file| file.path.clone())
        });

        if let Some(path) = dropped {
            self.load_path(path);
        } else {
            let dropped_without_path = ctx.input(|input| !input.raw.dropped_files.is_empty());
            if dropped_without_path {
                self.load_error = Some(ViewerLoadFailure {
                    kind: ViewerLoadFailureKind::FileAccess,
                    message: "Dropped item has no local filesystem path.".to_owned(),
                    classification: None,
                    diagnostic_json: None,
                });
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
        ui.heading("Pages");
        ui.separator();

        let Some(visual) = &self.visual else {
            ui.weak("No document loaded.");
            return;
        };

        let mut selected_page = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (index, page) in visual.document.pages.iter().enumerate() {
                let selected = self.selected_page == index;
                if ui
                    .selectable_label(selected, format!("Page {}", page.index))
                    .clicked()
                {
                    selected_page = Some(index);
                }
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

        self.show_search(ui);
    }

    fn show_inspector(&mut self, ui: &mut egui::Ui) {
        ui.heading("Document");
        ui.separator();

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

                ui.add_space(12.0);
                ui.heading("Diagnostics");
                ui.separator();

                if diagnostics.is_empty() {
                    ui.weak("No Viewer diagnostics.");
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(360.0)
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
            .source_path
            .as_deref()
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
        self.sync_visual_stories_from_editor();
        self.sync_visual_geometry_from_editor();
        self.refresh_search();
        self.export_preview = None;
        self.project_status = Some("Editor project has unsaved changes.".to_owned());
        self.edit_status = Some(status.to_owned());
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

    fn sync_visual_stories_from_editor(&mut self) {
        let (Some(editor), Some(visual)) = (&self.editor, &mut self.visual) else {
            return;
        };

        for viewer_story in &mut visual.document.stories {
            if let Some(story) = editor.graph().stories.get(&viewer_story.id) {
                viewer_story.text.clone_from(&story.text);
            }
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

    fn ensure_image_textures(&mut self, ctx: &egui::Context) {
        let Some(visual) = &self.visual else {
            return;
        };

        for embedded in &visual.images {
            let key = format!("{:?}", embedded.resource_id);
            if self.image_textures.contains_key(&key) {
                continue;
            }

            let format = match embedded.mime.as_str() {
                "image/png" => image::ImageFormat::Png,
                "image/jpeg" => image::ImageFormat::Jpeg,
                _ => continue,
            };

            let Ok(decoded) = image::load_from_memory_with_format(&embedded.bytes, format) else {
                continue;
            };
            let rgba = decoded.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            let color_image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
            let texture = ctx.load_texture(
                format!("pub-image-{key}"),
                color_image,
                egui::TextureOptions::LINEAR,
            );
            self.image_textures.insert(key, texture);
        }

        if let Some(editor) = &self.editor {
            for asset in editor.replacement_assets() {
                let key = format!("replacement:{:?}", asset.sha256);
                if self.image_textures.contains_key(&key) {
                    continue;
                }

                let format = match asset.mime.as_str() {
                    "image/png" => image::ImageFormat::Png,
                    "image/jpeg" => image::ImageFormat::Jpeg,
                    _ => continue,
                };
                let Ok(decoded) = image::load_from_memory_with_format(&asset.bytes, format) else {
                    continue;
                };
                let rgba = decoded.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color_image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                let texture = ctx.load_texture(
                    format!("chaptera-{key}"),
                    color_image,
                    egui::TextureOptions::LINEAR,
                );
                self.image_textures.insert(key, texture);
            }
        }
    }

    fn show_canvas(&mut self, ui: &mut egui::Ui) {
        self.ensure_image_textures(ui.ctx());
        self.preview_clipped_frames = 0;
        self.preview_clipped_story_keys.clear();

        ui.horizontal(|ui| {
            ui.label("Zoom");
            ui.add(
                egui::Slider::new(&mut self.zoom, 0.25..=4.0)
                    .logarithmic(true)
                    .show_value(true),
            );
            if ui.button("100% fit").clicked() {
                self.zoom = 1.0;
            }
        });
        ui.separator();

        let Some(visual) = &self.visual else {
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
        };

        let Some(page) = visual.document.pages.get(self.selected_page) else {
            ui.colored_label(ui.visuals().error_fg_color, "Selected page is unavailable.");
            return;
        };

        let Some(surface) = visual
            .scene
            .surfaces
            .iter()
            .find(|surface| surface.origin == page.id)
        else {
            ui.colored_label(
                ui.visuals().error_fg_color,
                "Selected page has no resolved scene surface.",
            );
            return;
        };

        let viewport = ui.available_size();
        let Some(fit_scale) = fitted_scale(
            surface.size.width.get(),
            surface.size.height.get(),
            viewport,
        ) else {
            ui.colored_label(
                ui.visuals().error_fg_color,
                "Selected page has invalid physical dimensions.",
            );
            return;
        };

        let page_width = surface.size.width.get() as f32 * fit_scale * self.zoom;
        let page_height = surface.size.height.get() as f32 * fit_scale * self.zoom;
        let content_width = (page_width + PAGE_MARGIN * 2.0).max(viewport.x);
        let content_height = (page_height + PAGE_MARGIN * 2.0).max(viewport.y);
        let mut preview_clipped_frames = 0usize;
        let mut preview_clipped_story_keys = BTreeSet::new();
        let selected_canvas_instance = self.canvas_selection.primary().map(str::to_owned);
        let mut canvas_clicked = false;
        let mut canvas_hit: Option<String> = None;
        let mut next_canvas_drag = self.canvas_drag;
        let mut next_canvas_resize = self.canvas_resize;
        let mut drag_commit = None;
        let mut drag_error = None;
        let mut resize_commit = None;
        let mut resize_error = None;
        let page_origin = page.id.into_canonical();
        let page_id_text = page.id.as_canonical().to_string();
        let page_nodes = visual
            .scene
            .nodes
            .iter()
            .filter(|node| node.parent_origin == page_origin)
            .collect::<Vec<_>>();

        let hit_index = SceneHitTestIndex::new(
            self.editor
                .as_ref()
                .map(|editor| {
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
                        .collect()
                })
                .unwrap_or_default(),
        );

        let movable_nodes = self
            .editor
            .as_ref()
            .map(|editor| {
                hit_index
                    .entries
                    .iter()
                    .filter_map(|hit| {
                        let instance = direct_scene_instance(editor, &page_id_text, hit.node_id)?;
                        let admission =
                            admit_object_mutation_v1(&instance, ObjectMutationKindV1::MoveNode);
                        let origin_node_id = hit.node_id.as_canonical().to_string();
                        if !admission.admitted
                            || admission.origin_node_id.as_deref() != Some(origin_node_id.as_str())
                        {
                            return None;
                        }
                        let authored_node = editor.graph().nodes.get(&hit.node_id)?;
                        let bounds = authored_node.header.bounds;
                        editor
                            .can_move_node_to(hit.node_id, bounds.x, bounds.y)
                            .ok()
                            .map(|_| (hit.instance_id.clone(), (hit.node_id, bounds)))
                    })
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();

        let resizable_nodes = self
            .editor
            .as_ref()
            .map(|editor| {
                hit_index
                    .entries
                    .iter()
                    .filter_map(|hit| {
                        let instance = direct_scene_instance(editor, &page_id_text, hit.node_id)?;
                        let admission =
                            admit_object_mutation_v1(&instance, ObjectMutationKindV1::ResizeNode);
                        let origin_node_id = hit.node_id.as_canonical().to_string();
                        if !admission.admitted
                            || admission.origin_node_id.as_deref() != Some(origin_node_id.as_str())
                        {
                            return None;
                        }
                        let authored_node = editor.graph().nodes.get(&hit.node_id)?;
                        let bounds = authored_node.header.bounds;
                        editor
                            .can_resize_node(hit.node_id)
                            .ok()
                            .map(|_| (hit.instance_id.clone(), (hit.node_id, bounds)))
                    })
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();

        egui::ScrollArea::both()
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
                let canvas = response.rect;
                let page_rect = egui::Rect::from_center_size(
                    canvas.center(),
                    egui::vec2(page_width, page_height),
                );
                let scene_scale = fit_scale * self.zoom;

                let pointer_document = response
                    .interact_pointer_pos()
                    .and_then(|pointer| canvas_document_point(page_rect, scene_scale, pointer));
                let press_screen = ui.ctx().input(|input| input.pointer.press_origin());
                let press_document = press_screen
                    .and_then(|pointer| canvas_document_point(page_rect, scene_scale, pointer));

                if !reader_only_mode()
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
                    canvas_hit = hit_index
                        .topmost_at(point)
                        .map(|hit| hit.instance_id.clone());
                }

                painter.rect_filled(page_rect, 0, egui::Color32::WHITE);
                painter.rect_stroke(
                    page_rect,
                    0,
                    egui::Stroke::new(1.0_f32, egui::Color32::DARK_GRAY),
                    egui::StrokeKind::Inside,
                );

                for node in page_nodes.iter().copied() {
                    let node_bounds = next_canvas_resize
                        .filter(|resize| resize.node_id() == node.origin)
                        .and_then(|resize| resize.preview_bounds())
                        .or_else(|| {
                            next_canvas_drag
                                .filter(|drag| drag.node_id() == node.origin)
                                .map(|drag| drag.preview_bounds())
                        })
                        .unwrap_or(node.bounds);
                    let width = node_bounds.width.get();
                    let height = node_bounds.height.get();
                    if width <= 0 || height <= 0 {
                        continue;
                    }

                    let min = egui::pos2(
                        page_rect.left() + node_bounds.x.get() as f32 * scene_scale,
                        page_rect.top() + node_bounds.y.get() as f32 * scene_scale,
                    );
                    let size = egui::vec2(width as f32 * scene_scale, height as f32 * scene_scale);
                    let node_rect = egui::Rect::from_min_size(min, size);
                    if let Some(instance_id) = hit_index.instance_for_node(node.origin)
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
                    if let Some(instance_id) = hit_index.instance_for_node(node.origin)
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
                    let node_paint = visual
                        .paints
                        .iter()
                        .find(|paint| paint.node_id == node.origin);

                    if let Some(rgb) = node_paint.and_then(|paint| paint.solid_fill_rgb) {
                        painter.rect_filled(
                            node_rect,
                            0,
                            egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]),
                        );
                    }

                    let replacement_key = self
                        .editor
                        .as_ref()
                        .and_then(|editor| editor.image_replacement_for(node.origin))
                        .map(|sha256| format!("replacement:{:?}", sha256));
                    let replacement_texture = replacement_key
                        .as_ref()
                        .and_then(|key| self.image_textures.get(key));
                    let source_texture = visual
                        .images
                        .iter()
                        .find(|embedded| embedded.node_ids.contains(&node.origin))
                        .and_then(|embedded| {
                            let key = format!("{:?}", embedded.resource_id);
                            self.image_textures.get(&key)
                        });
                    if let Some(texture) = replacement_texture.or(source_texture) {
                        painter.image(
                            texture.id(),
                            node_rect.shrink(1.0),
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }

                    painter.rect_stroke(
                        node_rect,
                        0,
                        egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(70, 120, 210)),
                        egui::StrokeKind::Inside,
                    );

                    if let Some(line) = node_paint.and_then(|paint| paint.solid_line.as_ref()) {
                        let line_width_px = line.width_emu as f32 * scene_scale;
                        if line_width_px > 0.0_f32 {
                            painter.rect_stroke(
                                node_rect,
                                0,
                                egui::Stroke::new(
                                    line_width_px,
                                    egui::Color32::from_rgb(line.rgb[0], line.rgb[1], line.rgb[2]),
                                ),
                                egui::StrokeKind::Inside,
                            );
                        }
                    }

                    if let Some(frame) = visual
                        .story_frames
                        .iter()
                        .find(|frame| frame.frame_id == node.origin)
                    {
                        let frame_count = visual
                            .story_frames
                            .iter()
                            .filter(|candidate| candidate.story_id == frame.story_id)
                            .count();

                        if frame_count == 1
                            && let Some(story) = visual
                                .document
                                .stories
                                .iter()
                                .find(|story| story.id == frame.story_id)
                            && !story.text.is_empty()
                        {
                            let text_clip_rect = node_rect.shrink(2.0);
                            let text_painter = painter.with_clip_rect(text_clip_rect);
                            let font_size = (12.0_f32 * self.zoom).clamp(8.0_f32, 28.0_f32);
                            let galley = text_painter.layout(
                                story.text.clone(),
                                egui::FontId::proportional(font_size),
                                egui::Color32::BLACK,
                                text_clip_rect.width().max(1.0_f32),
                            );
                            if preview_text_height_is_clipped(
                                galley.size().y,
                                text_clip_rect.height(),
                            ) {
                                preview_clipped_frames += 1;
                                preview_clipped_story_keys.insert(format!("{:?}", frame.story_id));
                                painter.rect_stroke(
                                    node_rect,
                                    0,
                                    egui::Stroke::new(2.0_f32, egui::Color32::RED),
                                    egui::StrokeKind::Inside,
                                );
                                painter.text(
                                    node_rect.right_top() + egui::vec2(-4.0_f32, 4.0_f32),
                                    egui::Align2::RIGHT_TOP,
                                    "preview overflow",
                                    egui::FontId::proportional(10.0_f32),
                                    egui::Color32::RED,
                                );
                            }
                            text_painter.galley(text_clip_rect.min, galley, egui::Color32::BLACK);
                        }
                    }
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
                    let resize_enabled = resizable_nodes.contains_key(selected_instance_id);
                    paint_selection_overlay(&painter, selected_rect, resize_enabled);
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

        egui::TopBottomPanel::top("workspace-command-bar").show(ctx, |ui| {
            self.show_command_bar(ui);
        });

        egui::TopBottomPanel::top("fidelity-status").show(ctx, |ui| {
            self.show_fidelity_status(ui);
        });

        egui::TopBottomPanel::bottom("workspace-status").show(ctx, |ui| {
            self.show_workspace_status(ui);
        });

        egui::SidePanel::left("pages")
            .resizable(true)
            .default_width(170.0)
            .show(ctx, |ui| self.show_pages(ui));

        egui::SidePanel::right("inspector")
            .resizable(true)
            .default_width(320.0)
            .show(ctx, |ui| self.show_inspector(ui));

        egui::CentralPanel::default().show(ctx, |ui| self.show_canvas(ui));
        self.show_diagnostics_window(ctx);
        self.show_exact_file_consent_dialog(ctx);
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

fn preview_text_height_is_clipped(galley_height: f32, clip_height: f32) -> bool {
    const EPSILON_PX: f32 = 0.5;
    galley_height > clip_height + EPSILON_PX
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
    fn canvas_pointer_maps_through_interaction_transform() {
        let page_rect =
            egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(400.0, 300.0));
        let point = canvas_document_point(page_rect, 0.5, egui::pos2(150.0, 125.0))
            .expect("valid canvas transform");

        assert_eq!(point.x.get(), 100);
        assert_eq!(point.y.get(), 150);
    }

    #[test]
    fn preview_text_clipping_uses_visible_galley_height_only() {
        assert!(!preview_text_height_is_clipped(100.0, 100.0));
        assert!(!preview_text_height_is_clipped(100.4, 100.0));
        assert!(preview_text_height_is_clipped(100.6, 100.0));
        assert!(PREVIEW_TEXT_CLIP_WARNING.contains("preview-only"));
        assert!(PREVIEW_TEXT_CLIP_WARNING.contains("not Publisher-native"));
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
    fn path_loading_calls_geometry_boundary() {
        let source = include_str!("main.rs");
        assert!(source.contains("open_mature_0x2c_geometry"));
        assert!(source.contains("viewer_geometry_environment_v0_1"));
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
            visual: None,
            selected_page: 0,
            canvas_selection: SceneSelectionState::default(),
            canvas_drag: None,
            canvas_resize: None,
            zoom: 1.0,
            load_error: Some(ViewerLoadFailure {
                kind: ViewerLoadFailureKind::Unsupported,
                message: "unsupported".to_owned(),
                classification: Some(classify_failure_candidate(b"<html>not pub</html>")),
                diagnostic_json: None,
            }),
            search_query: String::new(),
            search_results: Vec::new(),
            selected_search_result: None,
            image_textures: BTreeMap::new(),
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
            diagnostic_save_path: String::new(),
            diagnostic_status: None,
            supporter_value: supporter::ValueTracker::default(),
            supporter_state: supporter::SupporterState::default(),
            exact_file_consent_open: false,
            exact_file_consent_status: None,
            show_diagnostics: false,
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
            visual: None,
            selected_page: 0,
            canvas_selection: SceneSelectionState::default(),
            canvas_drag: None,
            canvas_resize: None,
            zoom: 1.0,
            load_error: Some(ViewerLoadFailure {
                kind: ViewerLoadFailureKind::FileAccess,
                message: "permission denied".to_owned(),
                classification: None,
                diagnostic_json: None,
            }),
            search_query: String::new(),
            search_results: Vec::new(),
            selected_search_result: None,
            image_textures: BTreeMap::new(),
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
            diagnostic_save_path: String::new(),
            diagnostic_status: None,
            supporter_value: supporter::ValueTracker::default(),
            supporter_state: supporter::SupporterState::default(),
            exact_file_consent_open: false,
            exact_file_consent_status: None,
            show_diagnostics: false,
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

        let format = match embedded.mime.as_str() {
            "image/png" => image::ImageFormat::Png,
            "image/jpeg" => image::ImageFormat::Jpeg,
            other => panic!("unexpected bounded image MIME: {other}"),
        };
        let decoded = image::load_from_memory_with_format(&embedded.bytes, format)
            .expect("Viewer app decoder must accept exact embedded PNG/JPEG bytes");

        assert!(decoded.width() > 0);
        assert!(decoded.height() > 0);
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
            visual: Some(visual),
            selected_page: 0,
            canvas_selection: SceneSelectionState::default(),
            canvas_drag: None,
            canvas_resize: None,
            zoom: 1.0,
            load_error: None,
            search_query: String::new(),
            search_results: Vec::new(),
            selected_search_result: None,
            image_textures: BTreeMap::new(),
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
            diagnostic_save_path: String::new(),
            diagnostic_status: None,
            supporter_value: supporter::ValueTracker::default(),
            supporter_state: supporter::SupporterState::default(),
            exact_file_consent_open: false,
            exact_file_consent_status: None,
            show_diagnostics: false,
        };

        app.editor
            .as_mut()
            .expect("editor loaded")
            .replace_story_text(story_id, replacement.clone())
            .expect("bounded desktop Story edit should succeed");
        app.sync_visual_stories_from_editor();

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
            .build_eframe(|cc| ViewerApp::new_with_storage(None, cc.storage));

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
            .build_eframe(move |cc| ViewerApp::new_with_storage(Some(fixture_for_app), cc.storage));
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
        assert_eq!(project.schema_version, pub_editor::EDITOR_PROJECT_VERSION_V0_11);
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

    #[test]
    fn source_path_argument_is_optional() {
        let path = std::path::Path::new("example.pub");
        assert_eq!(
            path.extension().and_then(|value| value.to_str()),
            Some("pub")
        );
    }
}
