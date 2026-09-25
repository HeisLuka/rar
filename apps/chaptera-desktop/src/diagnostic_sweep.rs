use pub_viewer::{FailureIntakeClass, ViewerGeometryDocument, classify_failure_candidate};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, TryRecvError},
};
use std::thread;

pub const MAX_FOLDER_SWEEP_DEPTH: usize = 10;
pub const FOLDER_SWEEP_SCHEMA: &str = "chaptera.pub-folder-sweep.v1";
const REPRESENTATIVE_PATH_LIMIT: usize = 5;

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct FolderSweepProgress {
    pub discovered: usize,
    pub scanned: usize,
    pub opened: usize,
    pub failed: usize,
    pub failure_groups: usize,
    pub current_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FolderSweepFileResult {
    pub relative_path: String,
    pub depth: usize,
    pub byte_len: Option<u64>,
    pub sha256: Option<String>,
    pub opened: bool,
    pub format: Option<String>,
    pub format_version: Option<String>,
    pub intake_class: Option<String>,
    pub failure_group_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FolderSweepTraversalIssue {
    pub relative_path: String,
    pub depth: usize,
    pub failure_group_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FolderSweepFailureGroup {
    pub id: String,
    pub stage: String,
    pub intake_class: Option<String>,
    pub normalized_message: String,
    pub count: usize,
    pub representative_paths: Vec<String>,
    pub affected_paths: Vec<String>,
    pub sample_full_diagnostic: String,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct FolderSweepTotals {
    pub discovered: usize,
    pub scanned: usize,
    pub opened: usize,
    pub failed: usize,
    pub traversal_issues: usize,
    pub failure_groups: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FolderSweepReport {
    pub schema: String,
    pub app_version: String,
    pub root: String,
    pub max_depth: usize,
    pub completed: bool,
    pub cancelled: bool,
    pub totals: FolderSweepTotals,
    pub files: Vec<FolderSweepFileResult>,
    pub traversal_issues: Vec<FolderSweepTraversalIssue>,
    pub failure_groups: Vec<FolderSweepFailureGroup>,
}

#[derive(Debug, Clone)]
pub enum FolderSweepEvent {
    Started { discovered: usize },
    Progress(FolderSweepProgress),
    Finished(FolderSweepReport),
    Fatal(String),
}

pub struct FolderSweepHandle {
    receiver: Receiver<FolderSweepEvent>,
    cancel: Arc<AtomicBool>,
}

impl FolderSweepHandle {
    pub fn try_recv(&self) -> Result<FolderSweepEvent, TryRecvError> {
        self.receiver.try_recv()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone)]
struct PubCandidate {
    path: PathBuf,
    relative_path: String,
    depth: usize,
}

#[derive(Debug, Clone)]
struct TraversalFailure {
    relative_path: String,
    depth: usize,
    message: String,
}

#[derive(Debug)]
struct FailureGroupBuilder {
    id: String,
    stage: String,
    intake_class: Option<String>,
    normalized_message: String,
    count: usize,
    representative_paths: Vec<String>,
    affected_paths: Vec<String>,
    sample_full_diagnostic: String,
}

impl FailureGroupBuilder {
    fn finish(self) -> FolderSweepFailureGroup {
        FolderSweepFailureGroup {
            id: self.id,
            stage: self.stage,
            intake_class: self.intake_class,
            normalized_message: self.normalized_message,
            count: self.count,
            representative_paths: self.representative_paths,
            affected_paths: self.affected_paths,
            sample_full_diagnostic: self.sample_full_diagnostic,
        }
    }
}

pub fn open_for_product(bytes: &[u8]) -> Result<ViewerGeometryDocument, String> {
    pub_viewer::open_mature_0x2c_geometry(
        bytes,
        pub_viewer::viewer_geometry_environment_v0_1(),
    )
    .map_err(|error| format!("{error:#}"))
}

pub fn start_folder_sweep(root: PathBuf) -> FolderSweepHandle {
    let (sender, receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);

    thread::spawn(move || {
        let result = run_folder_sweep(&root, &worker_cancel, |event| {
            let _ = sender.send(event);
        });
        if let Err(error) = result {
            let _ = sender.send(FolderSweepEvent::Fatal(error));
        }
    });

    FolderSweepHandle { receiver, cancel }
}

pub fn write_report(report: &FolderSweepReport, path: &Path) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(report)
        .map_err(|error| format!("serialize folder sweep report: {error}"))?;
    fs::write(path, bytes).map_err(|error| format!("write {}: {error}", path.display()))
}

fn run_folder_sweep(
    root: &Path,
    cancel: &AtomicBool,
    mut emit: impl FnMut(FolderSweepEvent),
) -> Result<(), String> {
    let metadata =
        fs::metadata(root).map_err(|error| format!("read scan root {}: {error}", root.display()))?;
    if !metadata.is_dir() {
        return Err(format!("scan root is not a directory: {}", root.display()));
    }

    let mut traversal_failures = Vec::new();
    let mut candidates = Vec::new();
    collect_pub_candidates(
        root,
        root,
        0,
        MAX_FOLDER_SWEEP_DEPTH,
        cancel,
        &mut candidates,
        &mut traversal_failures,
    );
    candidates.sort_by(|left, right| {
        left.relative_path
            .to_ascii_lowercase()
            .cmp(&right.relative_path.to_ascii_lowercase())
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });
    traversal_failures.sort_by(|left, right| {
        left.relative_path
            .to_ascii_lowercase()
            .cmp(&right.relative_path.to_ascii_lowercase())
            .then_with(|| left.relative_path.cmp(&right.relative_path))
    });

    emit(FolderSweepEvent::Started {
        discovered: candidates.len(),
    });

    let mut files = Vec::with_capacity(candidates.len());
    let mut traversal_issues = Vec::new();
    let mut groups = BTreeMap::<String, FailureGroupBuilder>::new();
    let mut progress = FolderSweepProgress {
        discovered: candidates.len(),
        ..FolderSweepProgress::default()
    };

    for failure in traversal_failures {
        let group_id = record_failure(
            &mut groups,
            "filesystem_traversal",
            None,
            &failure.message,
            &failure.relative_path,
        );
        traversal_issues.push(FolderSweepTraversalIssue {
            relative_path: failure.relative_path,
            depth: failure.depth,
            failure_group_id: group_id,
        });
    }
    progress.failure_groups = groups.len();

    for candidate in candidates {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        progress.current_path = Some(candidate.relative_path.clone());
        let result = scan_pub_candidate(&candidate, &mut groups);
        progress.scanned += 1;
        if result.opened {
            progress.opened += 1;
        } else {
            progress.failed += 1;
        }
        progress.failure_groups = groups.len();
        files.push(result);
        emit(FolderSweepEvent::Progress(progress.clone()));
    }

    let cancelled = cancel.load(Ordering::Relaxed);
    let mut failure_groups = groups
        .into_values()
        .map(FailureGroupBuilder::finish)
        .collect::<Vec<_>>();
    failure_groups.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.id.cmp(&right.id))
    });

    let report = FolderSweepReport {
        schema: FOLDER_SWEEP_SCHEMA.to_owned(),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        root: root.display().to_string(),
        max_depth: MAX_FOLDER_SWEEP_DEPTH,
        completed: !cancelled,
        cancelled,
        totals: FolderSweepTotals {
            discovered: progress.discovered,
            scanned: progress.scanned,
            opened: progress.opened,
            failed: progress.failed,
            traversal_issues: traversal_issues.len(),
            failure_groups: failure_groups.len(),
        },
        files,
        traversal_issues,
        failure_groups,
    };
    emit(FolderSweepEvent::Finished(report));
    Ok(())
}

fn scan_pub_candidate(
    candidate: &PubCandidate,
    groups: &mut BTreeMap<String, FailureGroupBuilder>,
) -> FolderSweepFileResult {
    let bytes = match fs::read(&candidate.path) {
        Ok(bytes) => bytes,
        Err(error) => {
            let diagnostic = format!("read file: {error}");
            let group_id = record_failure(
                groups,
                "filesystem_read",
                None,
                &diagnostic,
                &candidate.relative_path,
            );
            return FolderSweepFileResult {
                relative_path: candidate.relative_path.clone(),
                depth: candidate.depth,
                byte_len: fs::metadata(&candidate.path).ok().map(|metadata| metadata.len()),
                sha256: None,
                opened: false,
                format: None,
                format_version: None,
                intake_class: None,
                failure_group_id: Some(group_id),
            };
        }
    };

    let byte_len = bytes.len() as u64;
    let sha256 = sha256_hex(&bytes);
    match open_for_product(&bytes) {
        Ok(visual) => FolderSweepFileResult {
            relative_path: candidate.relative_path.clone(),
            depth: candidate.depth,
            byte_len: Some(byte_len),
            sha256: Some(sha256),
            opened: true,
            format: Some(visual.document.source.format.to_string()),
            format_version: visual.document.source.format_version.clone(),
            intake_class: None,
            failure_group_id: None,
        },
        Err(error) => {
            let intake = intake_class_name(classify_failure_candidate(&bytes).class).to_owned();
            let group_id = record_failure(
                groups,
                "viewer_open",
                Some(&intake),
                &error,
                &candidate.relative_path,
            );
            FolderSweepFileResult {
                relative_path: candidate.relative_path.clone(),
                depth: candidate.depth,
                byte_len: Some(byte_len),
                sha256: Some(sha256),
                opened: false,
                format: None,
                format_version: None,
                intake_class: Some(intake),
                failure_group_id: Some(group_id),
            }
        }
    }
}

fn collect_pub_candidates(
    root: &Path,
    directory: &Path,
    depth: usize,
    max_depth: usize,
    cancel: &AtomicBool,
    candidates: &mut Vec<PubCandidate>,
    failures: &mut Vec<TraversalFailure>,
) {
    if cancel.load(Ordering::Relaxed) {
        return;
    }

    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            failures.push(TraversalFailure {
                relative_path: relative_display(root, directory),
                depth,
                message: format!("read directory: {error}"),
            });
            return;
        }
    };

    let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        let left_name = left.file_name().to_string_lossy().into_owned();
        let right_name = right.file_name().to_string_lossy().into_owned();
        left_name
            .to_ascii_lowercase()
            .cmp(&right_name.to_ascii_lowercase())
            .then_with(|| left_name.cmp(&right_name))
    });

    for entry in entries {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let path = entry.path();
        let relative = relative_display(root, &path);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                failures.push(TraversalFailure {
                    relative_path: relative,
                    depth,
                    message: format!("read metadata: {error}"),
                });
                continue;
            }
        };

        if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            continue;
        }

        if metadata.is_file() {
            if has_pub_extension(&path) {
                candidates.push(PubCandidate {
                    path,
                    relative_path: relative,
                    depth,
                });
            }
            continue;
        }

        if metadata.is_dir() && depth < max_depth {
            collect_pub_candidates(
                root,
                &path,
                depth + 1,
                max_depth,
                cancel,
                candidates,
                failures,
            );
        }
    }
}

fn record_failure(
    groups: &mut BTreeMap<String, FailureGroupBuilder>,
    stage: &str,
    intake_class: Option<&str>,
    full_diagnostic: &str,
    relative_path: &str,
) -> String {
    let normalized_message = normalize_failure_message(full_diagnostic);
    let mut key_material = String::new();
    key_material.push_str(stage);
    key_material.push('\n');
    if let Some(intake_class) = intake_class {
        key_material.push_str(intake_class);
    }
    key_material.push('\n');
    key_material.push_str(&normalized_message);
    let id = format!("F-{}", &sha256_hex(key_material.as_bytes())[..16]);

    let group = groups.entry(id.clone()).or_insert_with(|| FailureGroupBuilder {
        id: id.clone(),
        stage: stage.to_owned(),
        intake_class: intake_class.map(str::to_owned),
        normalized_message,
        count: 0,
        representative_paths: Vec::new(),
        affected_paths: Vec::new(),
        sample_full_diagnostic: full_diagnostic.to_owned(),
    });
    group.count += 1;
    if group.representative_paths.len() < REPRESENTATIVE_PATH_LIMIT {
        group.representative_paths.push(relative_path.to_owned());
    }
    group.affected_paths.push(relative_path.to_owned());
    id
}

fn normalize_failure_message(message: &str) -> String {
    message
        .split_whitespace()
        .map(normalize_failure_token)
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_failure_token(token: &str) -> String {
    let trimmed = token.trim_matches(|ch: char| {
        matches!(ch, ',' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}' | '"' | '\'')
    });
    if trimmed.len() == 64 && trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return token.replace(trimmed, "<sha256>");
    }
    if let Some(hex) = trimmed.strip_prefix("0x")
        && hex.len() >= 12
        && hex.chars().all(|ch| ch.is_ascii_hexdigit())
    {
        return token.replace(trimmed, "<hex>");
    }
    token.to_owned()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn relative_display(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    if relative.as_os_str().is_empty() {
        ".".to_owned()
    } else {
        relative.to_string_lossy().replace('\\', "/")
    }
}

fn has_pub_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pub"))
}

fn intake_class_name(class: FailureIntakeClass) -> &'static str {
    match class {
        FailureIntakeClass::PubHighValue => "pub_high_value",
        FailureIntakeClass::PubDamaged => "pub_damaged",
        FailureIntakeClass::PubPossible => "pub_possible",
        FailureIntakeClass::ArchiveWithPub => "archive_with_pub",
        FailureIntakeClass::NotPub => "not_pub",
        FailureIntakeClass::SuspiciousPolyglot => "suspicious_polyglot",
    }
}

#[cfg(target_os = "windows")]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(target_os = "windows"))]
fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "chaptera-folder-sweep-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn pub_extension_is_case_insensitive() {
        assert!(has_pub_extension(Path::new("a.pub")));
        assert!(has_pub_extension(Path::new("B.PUB")));
        assert!(has_pub_extension(Path::new("Mixed.PuB")));
        assert!(!has_pub_extension(Path::new("notpub.pubx")));
    }

    #[test]
    fn depth_ten_is_included_and_depth_eleven_is_excluded() {
        let root = temp_dir("depth");
        fs::write(root.join("root.PUB"), b"x").expect("root file");
        let mut directory = root.clone();
        for depth in 1..=11 {
            directory = directory.join(format!("d{depth}"));
            fs::create_dir_all(&directory).expect("nested dir");
            fs::write(directory.join(format!("depth-{depth}.pub")), b"x").expect("nested file");
        }

        let cancel = AtomicBool::new(false);
        let mut candidates = Vec::new();
        let mut failures = Vec::new();
        collect_pub_candidates(
            &root,
            &root,
            0,
            MAX_FOLDER_SWEEP_DEPTH,
            &cancel,
            &mut candidates,
            &mut failures,
        );
        let paths = candidates
            .iter()
            .map(|candidate| candidate.relative_path.as_str())
            .collect::<Vec<_>>();

        assert!(paths.contains(&"root.PUB"));
        assert!(paths.iter().any(|path| path.ends_with("depth-10.pub")));
        assert!(!paths.iter().any(|path| path.ends_with("depth-11.pub")));
        assert!(failures.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn duplicate_failures_collapse_but_distinct_failures_do_not() {
        let mut groups = BTreeMap::new();
        let first = record_failure(
            &mut groups,
            "viewer_open",
            Some("pub_high_value"),
            "read /Quill/QuillSub/CONTENTS: No such stream",
            "a.pub",
        );
        let second = record_failure(
            &mut groups,
            "viewer_open",
            Some("pub_high_value"),
            "read /Quill/QuillSub/CONTENTS: No such stream",
            "nested/b.pub",
        );
        let third = record_failure(
            &mut groups,
            "viewer_open",
            Some("pub_high_value"),
            "Contents record truncated at field 0x21",
            "c.pub",
        );

        assert_eq!(first, second);
        assert_ne!(first, third);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups.get(&first).expect("group").count, 2);
    }

    #[test]
    fn failure_normalization_keeps_format_semantics_but_removes_volatile_hex() {
        let normalized = normalize_failure_message(
            "family 0x2C missing /Quill/QuillSub/CONTENTS ptr 0x7ffdeadbeef12345",
        );
        assert!(normalized.contains("0x2C"));
        assert!(normalized.contains("/Quill/QuillSub/CONTENTS"));
        assert!(normalized.contains("<hex>"));
    }

    #[test]
    fn discovery_order_can_be_made_deterministic() {
        let root = temp_dir("order");
        for name in ["z.pub", "A.PUB", "m.PuB"] {
            fs::write(root.join(name), b"x").expect("fixture");
        }
        let cancel = AtomicBool::new(false);
        let mut candidates = Vec::new();
        let mut failures = Vec::new();
        collect_pub_candidates(
            &root,
            &root,
            0,
            MAX_FOLDER_SWEEP_DEPTH,
            &cancel,
            &mut candidates,
            &mut failures,
        );
        candidates.sort_by(|left, right| {
            left.relative_path
                .to_ascii_lowercase()
                .cmp(&right.relative_path.to_ascii_lowercase())
                .then_with(|| left.relative_path.cmp(&right.relative_path))
        });
        assert_eq!(
            candidates
                .into_iter()
                .map(|candidate| candidate.relative_path)
                .collect::<Vec<_>>(),
            vec!["A.PUB", "m.PuB", "z.pub"]
        );
        assert!(failures.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_directory_loop_is_not_followed() {
        use std::os::unix::fs::symlink;

        let root = temp_dir("symlink");
        fs::write(root.join("one.pub"), b"x").expect("fixture");
        symlink(&root, root.join("loop")).expect("symlink");

        let cancel = AtomicBool::new(false);
        let mut candidates = Vec::new();
        let mut failures = Vec::new();
        collect_pub_candidates(
            &root,
            &root,
            0,
            MAX_FOLDER_SWEEP_DEPTH,
            &cancel,
            &mut candidates,
            &mut failures,
        );
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].relative_path, "one.pub");
        assert!(failures.is_empty());
        let _ = fs::remove_dir_all(root);
    }
}
