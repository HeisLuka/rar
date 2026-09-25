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
use pub_interaction::{MoveTransaction, ScreenPoint, ViewTransform};
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

fn reader_only_mode() -> bool {
    cfg!(feature = "reader-only")
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

    if first_arg.as_deref() == Some(std::ffi::OsStr::new("--ux-snapshot-v1")) {
        let Some(fixture) = args.next().map(PathBuf::from) else {
            eprintln!("usage: chaptera --ux-snapshot-v1 FIXTURE OUTPUT.png WIDTH HEIGHT");
            std::process::exit(2);
        };
        let Some(output) = args.next().map(PathBuf::from) else {
            eprintln!("usage: chaptera --ux-snapshot-v1 FIXTURE OUTPUT.png WIDTH HEIGHT");
            std::process::exit(2);
        };
        let Some(width) = args
            .next()
            .and_then(|value| value.to_str().and_then(|value| value.parse::<f32>().ok()))
        else {
            eprintln!("ux snapshot WIDTH must be a positive number");
            std::process::exit(2);
        };
        let Some(height) = args
            .next()
            .and_then(|value| value.to_str().and_then(|value| value.parse::<f32>().ok()))
        else {
            eprintln!("ux snapshot HEIGHT must be a positive number");
            std::process::exit(2);
        };
        if args.next().is_some() || width < 1.0 || height < 1.0 {
            eprintln!("usage: chaptera --ux-snapshot-v1 FIXTURE OUTPUT.png WIDTH HEIGHT");
            std::process::exit(2);
        }
        if output.extension().and_then(|value| value.to_str()) != Some("png") {
            eprintln!("ux snapshot output must end in .png");
            std::process::exit(2);
        }

        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title(APP_TITLE)
                .with_inner_size([width, height])
                .with_min_inner_size([width, height])
                .with_max_inner_size([width, height])
                .with_resizable(false)
                .with_drag_and_drop(false),
            ..Default::default()
        };
        return eframe::run_native(
            APP_TITLE,
            options,
            Box::new(move |cc| {
                Ok(Box::new(SnapshotApp::new(
                    ViewerApp::new_with_storage(Some(fixture), cc.storage),
                    output,
                )))
            }),
        );
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

struct SnapshotApp {
    inner: ViewerApp,
    output: PathBuf,
    settle_frames: u8,
    requested: bool,
}

impl SnapshotApp {
    fn new(inner: ViewerApp, output: PathBuf) -> Self {
        Self {
            inner,
            output,
            settle_frames: 0,
            requested: false,
        }
    }

    fn write_png(&self, color_image: &egui::ColorImage) -> Result<(), String> {
        if let Some(parent) = self.output.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create {}: {error}", parent.display()))?;
        }
        let width = u32::try_from(color_image.size[0])
            .map_err(|_| "screenshot width exceeds u32".to_owned())?;
        let height = u32::try_from(color_image.size[1])
            .map_err(|_| "screenshot height exceeds u32".to_owned())?;
        let mut rgba = Vec::with_capacity(color_image.pixels.len() * 4);
        for pixel in &color_image.pixels {
            rgba.extend_from_slice(&pixel.to_array());
        }
        let image = image::RgbaImage::from_raw(width, height, rgba)
            .ok_or_else(|| "screenshot RGBA dimensions are inconsistent".to_owned())?;
        image
            .save(&self.output)
            .map_err(|error| format!("write {}: {error}", self.output.display()))
    }
}

impl eframe::App for SnapshotApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.inner.update(ctx, frame);

        let screenshot = ctx.input(|input| {
            input.raw.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = screenshot {
            if let Err(error) = self.write_png(&image) {
                eprintln!("{error}");
                std::process::exit(2);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        if !self.requested {
            if self.settle_frames < 3 {
                self.settle_frames += 1;
                ctx.request_repaint();
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                    egui::UserData::default(),
                ));
                self.requested = true;
                ctx.request_repaint();
            }
        }
    }
}

struct ViewerApp {
    source_path: Option<PathBuf>,
    visual: Option<ViewerGeometryDocument>,
    selected_page: usize,
    canvas_selection: SceneSelectionState,
    canvas_drag: Option<MoveTransaction>,
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
        };

        if let Some(path) = initial_path {
            app.load_path(path);
        }

        app
    }

    fn load_path(&mut self, path: PathBuf) {
        self.supporter_value
            .observe(supporter::ValueEvent::WorkflowFailed);
        self.source_path = Some(path.clone());
        self.visual = None;
        self.selected_page = 0;
        self.canvas_selection.clear();
        self.canvas_drag = None;
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
            ui.strong("Chaptera Editor");
            ui.separator();

            if ui.button("Open PUB…").clicked() {
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
                    self.edit_status = Some(
                        "The native Open dialog is part of Windows Portable V0; drag and drop a PUB file on this platform."
                            .to_owned(),
                    );
                }
            }

            if let Some(label) = document_label {
                ui.label(label);
            } else {
                ui.weak("Open or drop a .pub file to begin");
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
                ui.label(format!("{operation_count} edit operation(s)"));
                if let Some(fidelity) = self.fidelity_status() {
                    ui.label("·");
                    ui.label(format!("Fidelity: {}", fidelity_status_label(fidelity)));
                }
            } else {
                ui.weak("No document open");
                ui.label("·");
                ui.label("Source files stay local and are never overwritten.");
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

    fn show_fidelity_status(&self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Fidelity:");

            match self.fidelity_status() {
                Some(status) => {
                    ui.strong(fidelity_status_label(status));
                    ui.label(fidelity_status_summary(status));
                }
                None => {
                    ui.weak("Not evaluated");
                    ui.label("Open a PUB file to evaluate the current Viewer scope.");
                }
            }
        });

        if self.visual.as_ref().is_some_and(|visual| {
            visual
                .document
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "viewer.visual.geometry_only")
        }) {
            ui.small(GEOMETRY_WARNING);
        }

        if self.preview_clipped_frames > 0 {
            ui.add_space(4.0);
            ui.strong(format!(
                "Preview text clipping: {} frame(s)",
                self.preview_clipped_frames
            ));
            ui.small(PREVIEW_TEXT_CLIP_WARNING);
        }
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
            ui.small("The original PUB stays unchanged; edits live in the Chaptera project.");
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
                if let Some(classification) = &error.classification {
                    ui.add_space(8.0);
                    ui.strong(failure_intake_label(classification.class));
                    ui.label(failure_intake_summary(classification.class));
                    ui.small(
                        "This classification was computed locally. Chaptera did not send this file or its contents anywhere.",
                    );

                    if exact_file_consent_cta_visible(classification.class) {
                        ui.add_space(10.0);
                        if ui
                            .button("Send this file to help Chaptera support it")
                            .clicked()
                        {
                            self.exact_file_consent_open = true;
                            self.exact_file_consent_status = None;
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
                ui.small("Drag to move supported page-local objects. Projected objects stay read-only.");
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
            ui.label(format!("Engine: {}", visual.scene.environment.engine_revision));
            ui.label("Source SHA-256");
            ui.monospace(format!("{:?}", source.source_hash));
            if let Some(instance_id) = self.canvas_selection.primary() {
                ui.label("Selected scene instance");
                ui.monospace(instance_id);
            }
        });

        ui.add_space(16.0);
        ui.heading("Fidelity");
        ui.separator();
        let fidelity = visual.document.fidelity_status();
        ui.strong(fidelity_status_label(fidelity));
        ui.label(fidelity_status_summary(fidelity));
        let warning_count = visual
            .document
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == ViewerDiagnosticSeverity::FidelityWarning)
            .count();
        ui.label(format!("Known fidelity warnings: {warning_count}"));

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

        ui.add_space(16.0);
        ui.heading("Diagnostics");
        ui.separator();

        if visual.document.diagnostics.is_empty() {
            ui.weak("No Viewer diagnostics.");
        } else {
            egui::ScrollArea::vertical().show(ui, |ui| {
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

        if !reader_only_mode() {
            self.show_editor_controls(ui);
        }

        if let Some(error) = &self.load_error {
            ui.add_space(12.0);
            ui.colored_label(ui.visuals().error_fg_color, &error.message);
        }
    }

    fn show_exact_file_consent_dialog(&mut self, ctx: &egui::Context) {
        if !self.exact_file_consent_open {
            return;
        }

        let eligible = self
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
                "current edits differ from the saved EditorProject; save before reopening".to_owned(),
            );
        }

        self.load_path(source_path);
        if self.editor.is_none() {
            return Err("fresh editor session could not be opened".to_owned());
        }
        self.edit_status =
            Some("Reopened source and replayed the saved EditorProject in a fresh session.".to_owned());
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

    fn commit_canvas_drag(&mut self, drag: MoveTransaction) {
        self.canvas_drag = None;
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
                    ui.heading("Open a Publisher file");
                    ui.label("Use Open PUB… above, or drag and drop a .pub file here.");
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
        let mut drag_commit = None;
        let mut drag_error = None;
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
                let press_document = ui
                    .ctx()
                    .input(|input| input.pointer.press_origin())
                    .and_then(|pointer| canvas_document_point(page_rect, scene_scale, pointer));

                if !reader_only_mode()
                    && response.drag_started_by(egui::PointerButton::Primary)
                    && let (Some(pointer_start), Some(pointer_current)) =
                        (press_document, pointer_document)
                    && let Some(hit) = hit_index.topmost_at(pointer_start)
                {
                    canvas_hit = Some(hit.instance_id.clone());
                    if let Some((node_id, before)) = movable_nodes.get(&hit.instance_id).copied() {
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
                } else if !reader_only_mode()
                    && response.drag_stopped_by(egui::PointerButton::Primary)
                {
                    if let (Some(mut drag), Some(point)) = (next_canvas_drag, pointer_document) {
                        match drag.update(point) {
                            Ok(_) => drag_commit = Some(drag),
                            Err(error) => {
                                next_canvas_drag = None;
                                drag_error = Some(format!("Object move cancelled: {error}"));
                            }
                        }
                    } else {
                        next_canvas_drag = None;
                    }
                } else if !reader_only_mode()
                    && response.dragged_by(egui::PointerButton::Primary)
                    && let (Some(mut drag), Some(point)) = (next_canvas_drag, pointer_document)
                {
                    match drag.update(point) {
                        Ok(_) => next_canvas_drag = Some(drag),
                        Err(error) => {
                            next_canvas_drag = None;
                            drag_error = Some(format!("Object move cancelled: {error}"));
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
                    let node_bounds = next_canvas_drag
                        .filter(|drag| drag.node_id() == node.origin)
                        .map(|drag| drag.preview_bounds())
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

                    if let Some(embedded) = visual
                        .images
                        .iter()
                        .find(|embedded| embedded.node_ids.contains(&node.origin))
                    {
                        let key = format!("{:?}", embedded.resource_id);
                        if let Some(texture) = self.image_textures.get(&key) {
                            painter.image(
                                texture.id(),
                                node_rect.shrink(1.0),
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                egui::Color32::WHITE,
                            );
                        }
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
                    let selected_bounds = next_canvas_drag
                        .filter(|drag| drag.node_id() == node.origin)
                        .map(|drag| drag.preview_bounds())
                        .unwrap_or(node.bounds);
                    let min = egui::pos2(
                        page_rect.left() + selected_bounds.x.get() as f32 * scene_scale,
                        page_rect.top() + selected_bounds.y.get() as f32 * scene_scale,
                    );
                    let size = egui::vec2(
                        selected_bounds.width.get() as f32 * scene_scale,
                        selected_bounds.height.get() as f32 * scene_scale,
                    );
                    paint_selection_overlay(&painter, egui::Rect::from_min_size(min, size));
                }
            });

        let drag_instance = next_canvas_drag.and_then(|drag| {
            hit_index
                .instance_for_node(drag.node_id())
                .map(str::to_owned)
        });
        if canvas_clicked || drag_commit.is_some() || next_canvas_drag.is_some() {
            if let Some(instance_id) = canvas_hit.or(drag_instance) {
                self.canvas_selection.select_only(instance_id);
            } else if canvas_clicked {
                self.canvas_selection.clear();
            }
        }

        if let Some(error) = drag_error {
            self.canvas_drag = None;
            self.edit_status = Some(error);
        } else if let Some(drag) = drag_commit {
            self.commit_canvas_drag(drag);
        } else {
            self.canvas_drag = next_canvas_drag;
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

fn paint_selection_overlay(painter: &egui::Painter, rect: egui::Rect) {
    let accent = egui::Color32::from_rgb(232, 126, 36);
    painter.rect_stroke(
        rect.expand(2.0),
        0,
        egui::Stroke::new(2.0_f32, accent),
        egui::StrokeKind::Inside,
    );

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
        let bytes = decode_base64_fixture(include_str!(
            "../../../vendor/producer-a/crates/pub-quill/tests/fixtures/SampleNewsletter.pub.b64"
        ));
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
        };

        assert_eq!(app.fidelity_status(), None);
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
    fn search_preview_is_single_line_and_bounded() {
        let preview = search_result_preview(
            "this is a deliberately long\nsearch result that should be shortened for the list",
        );

        assert!(!preview.contains('\n'));
        assert!(preview.ends_with('…'));
        assert!(preview.chars().count() <= 49);
    }

    #[cfg(feature = "embedded-fixture-tests")]
    fn decode_base64_fixture(input: &str) -> Vec<u8> {
        let mut output = Vec::with_capacity(input.len() * 3 / 4);
        let mut buffer = 0_u32;
        let mut bits = 0_u8;

        for byte in input.bytes() {
            let value = match byte {
                b'A'..=b'Z' => byte - b'A',
                b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' => break,
                byte if byte.is_ascii_whitespace() => continue,
                other => panic!("unexpected base64 byte: {other:#04x}"),
            };

            buffer = (buffer << 6) | u32::from(value);
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                output.push((buffer >> bits) as u8);
                buffer &= if bits == 0 { 0 } else { (1_u32 << bits) - 1 };
            }
        }

        output
    }

    #[cfg(feature = "embedded-fixture-tests")]
    #[test]
    fn real_pub_exposes_decodable_exact_image_bound_to_scene_node() {
        let pub_bytes = decode_base64_fixture(include_str!(
            "../../../vendor/producer-a/crates/pub-quill/tests/fixtures/SampleNewsletter.pub.b64"
        ));
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
        let bytes = decode_base64_fixture(include_str!(
            "../../../vendor/producer-a/crates/pub-quill/tests/fixtures/SampleNewsletter.pub.b64"
        ));
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
        let bytes = decode_base64_fixture(include_str!(
            "../../../vendor/producer-a/crates/pub-quill/tests/fixtures/SampleNewsletter.pub.b64"
        ));
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
        let bytes = decode_base64_fixture(include_str!(
            "../../../vendor/producer-a/crates/pub-quill/tests/fixtures/SampleNewsletter.pub.b64"
        ));
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

    #[test]
    fn ux_snapshot_mode_is_bounded_and_source_driven() {
        let source = include_str!("main.rs");
        assert!(source.contains("--ux-snapshot-v1"));
        assert!(source.contains("ViewportCommand::Screenshot"));
        assert!(source.contains("ViewerApp::new_with_storage(Some(fixture)"));
        assert!(source.contains("with_min_inner_size([width, height])"));
        assert!(source.contains("with_max_inner_size([width, height])"));
        let production_source = source
            .split_once("#[cfg(test)]")
            .map_or(source, |(production, _)| production);
        assert!(!production_source.contains("or start with: chaptera FILE.pub"));
    }

    #[cfg(feature = "embedded-fixture-tests")]
    #[test]
    fn gui_only_v0_walkthrough_uses_real_widgets() {
        use egui_kittest::{Harness, kittest::Queryable};

        let original = decode_base64_fixture(include_str!(
            "../../../vendor/producer-a/crates/pub-quill/tests/fixtures/SampleNewsletter.pub.b64"
        ));
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
        harness.run();
        assert!(harness.state().visual.is_some(), "GUI file drop must open the PUB");
        assert!(harness.state().editor.is_some(), "GUI open must create the EditorSession");

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
            let search = harness.get_by_value("");
            search.type_text(search_term.clone());
        }
        harness.run();
        let result_label = {
            let result = harness
                .state()
                .search_results
                .first()
                .expect("GUI search produces one result");
            assert_eq!(result.story_id, story_id);
            format!("1. {}", search_result_preview(&result.text))
        };
        {
            harness.get_by_label(&result_label).click();
        }
        harness.run();
        assert_eq!(harness.state().edit_buffer, original_story);

        let replacement = "Chaptera GUI-only V0 acceptance text".to_owned();
        {
            let editor = harness.get_by_value(&original_story);
            editor.key_combination(&[
                egui_kittest::kittest::Key::Control,
                egui_kittest::kittest::Key::A,
            ]);
            editor.type_text(replacement.clone());
        }
        harness.run();
        {
            harness.get_by_label("Apply Story edit").click();
        }
        harness.run();
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

        let (start, end) = {
            let object = harness
                .get_all_by_label("Movable canvas object")
                .last()
                .expect("real canvas exposes at least one movable object");
            let bounds = object.raw_bounds().expect("movable object has screen bounds");
            let start = egui::pos2(
                ((bounds.x0 + bounds.x1) / 2.0) as f32,
                ((bounds.y0 + bounds.y1) / 2.0) as f32,
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
        harness.run_ok();

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

        {
            harness
                .get_all_by_label("Undo")
                .next()
                .expect("Undo command")
                .click();
        }
        harness.run();
        assert_eq!(
            harness.state().editor.as_ref().expect("editor").graph().nodes[&moved_node_id]
                .header
                .bounds,
            before_move,
            "GUI Undo must restore exact pre-drag geometry"
        );

        {
            harness
                .get_all_by_label("Redo")
                .next()
                .expect("Redo command")
                .click();
        }
        harness.run();
        assert_eq!(
            harness.state().editor.as_ref().expect("editor").graph().nodes[&moved_node_id]
                .header
                .bounds,
            after_move,
            "GUI Redo must restore exact moved geometry"
        );

        {
            harness.get_by_label("Save Project").click();
        }
        harness.run();
        let sidecar = editor_project_sidecar_path(&fixture).expect("sidecar path");
        assert!(sidecar.is_file(), "GUI Save Project must write the sidecar");

        {
            let reopen = harness.get_by_label("Reopen Project");
            assert!(!reopen.is_disabled(), "saved clean state enables Reopen Project");
            reopen.click();
        }
        harness.run();
        {
            let editor = harness.state().editor.as_ref().expect("fresh reopened editor");
            assert_eq!(editor.operations().len(), 2);
            assert_eq!(editor.graph().stories[&story_id].text, replacement);
            assert_eq!(editor.graph().nodes[&moved_node_id].header.bounds, after_move);
        }

        {
            harness.get_by_label("Export").click();
        }
        harness.run();
        {
            harness.get_by_label("Preview IDML").click();
        }
        harness.run();
        assert!(
            harness
                .state()
                .export_preview
                .as_ref()
                .is_some_and(|preview| {
                    preview.target == pub_editor::EditorEditableTarget::Idml
                        && preview.operation_count == 2
                        && preview.can_serialize
                }),
            "GUI IDML preview must admit the current edited state"
        );
        {
            harness.get_by_label("Export").click();
        }
        harness.run();
        {
            harness.get_by_label("Export edited IDML copy").click();
        }
        harness.run();

        let exported =
            editable_export_path(&fixture, pub_editor::EditorEditableTarget::Idml).expect("IDML path");
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

    #[test]
    fn source_path_argument_is_optional() {
        let path = std::path::Path::new("example.pub");
        assert_eq!(
            path.extension().and_then(|value| value.to_str()),
            Some("pub")
        );
    }
}
