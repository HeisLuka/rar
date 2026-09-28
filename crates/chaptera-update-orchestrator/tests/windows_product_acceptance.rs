use chaptera_update_engine::RecoveryOutcome;
use chaptera_update_orchestrator::{ApplyOutcome, UpdateHooks, UpdateOrchestrator};
use chaptera_update_trust::{
    ChapteraReleaseSemantics, InstalledUpdateContext, UpdateMode, INSTALL_LAYOUT_EPOCH,
    UPDATE_PROTOCOL_VERSION,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

#[derive(Default)]
struct Hooks {
    fail_health: bool,
}

impl UpdateHooks for Hooks {
    fn quiesce(&mut self, _control_updater: &Path) -> Result<(), String> {
        Ok(())
    }

    fn health_check(&mut self, _current_tree: &Path) -> Result<(), String> {
        if self.fail_health {
            Err("synthetic candidate health failure".into())
        } else {
            Ok(())
        }
    }
}

fn seed_tree(path: &Path, updater: &[u8], reader: &[u8]) {
    fs::create_dir_all(path).unwrap();
    fs::write(path.join("chaptera-updater.bin"), updater).unwrap();
    fs::write(path.join("reader.bin"), reader).unwrap();
}

#[test]
fn windows_product_update_crash_rollback_recovery_cycle() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let candidate_b = temp.path().join("candidate-b");
    let candidate_c = temp.path().join("candidate-c");
    let external_state = temp.path().join("user-state.txt");

    seed_tree(&root.join("current"), b"U1", b"reader-A");
    seed_tree(&candidate_b, b"U2", b"reader-B");
    seed_tree(&candidate_c, b"U3", b"reader-C");
    fs::write(&external_state, b"must-survive").unwrap();

    let orchestrator = UpdateOrchestrator::new(&root);

    let mut healthy = Hooks::default();
    let outcome = orchestrator
        .apply_verified_candidate(
            "accept-a-to-b",
            "2.0.0",
            &candidate_b,
            Path::new("chaptera-updater.bin"),
            &mut healthy,
        )
        .unwrap();
    assert!(matches!(
        outcome,
        ApplyOutcome::Confirmed {
            startup_recovery: RecoveryOutcome::NothingToDo,
            ..
        }
    ));
    assert_eq!(fs::read(root.join("current/reader.bin")).unwrap(), b"reader-B");
    assert_eq!(fs::read(&external_state).unwrap(), b"must-survive");

    let mut failing = Hooks { fail_health: true };
    let outcome = orchestrator
        .apply_verified_candidate(
            "accept-b-to-c-bad",
            "3.0.0",
            &candidate_c,
            Path::new("chaptera-updater.bin"),
            &mut failing,
        )
        .unwrap();
    assert!(matches!(outcome, ApplyOutcome::RolledBack { .. }));
    assert_eq!(fs::read(root.join("current/reader.bin")).unwrap(), b"reader-B");
    assert_eq!(fs::read(&external_state).unwrap(), b"must-survive");

    let crash_candidate = temp.path().join("candidate-c-crash");
    seed_tree(&crash_candidate, b"U3", b"reader-C");
    orchestrator
        .engine()
        .begin_verified_candidate(
            "accept-crash-before-confirm",
            "3.0.0",
            &crash_candidate,
            Path::new("chaptera-updater.bin"),
        )
        .unwrap();
    orchestrator.engine().retain_previous().unwrap();
    orchestrator.engine().activate_candidate().unwrap();
    assert_eq!(fs::read(root.join("current/reader.bin")).unwrap(), b"reader-C");

    let restarted = UpdateOrchestrator::new(&root);
    assert_eq!(
        restarted.engine().recover().unwrap(),
        RecoveryOutcome::UnconfirmedCandidateRolledBack
    );
    assert_eq!(fs::read(root.join("current/reader.bin")).unwrap(), b"reader-B");
    assert_eq!(fs::read(&external_state).unwrap(), b"must-survive");
}

struct RealReaderHooks {
    fixture: PathBuf,
    fail_after_smoke: bool,
}

impl UpdateHooks for RealReaderHooks {
    fn quiesce(&mut self, _control_updater: &Path) -> Result<(), String> {
        Ok(())
    }

    fn health_check(&mut self, current_tree: &Path) -> Result<(), String> {
        run_real_reader_smoke(current_tree, &self.fixture)?;
        if self.fail_after_smoke {
            Err("synthetic post-smoke candidate rejection".into())
        } else {
            Ok(())
        }
    }
}

fn run_real_reader_smoke(current_tree: &Path, fixture: &Path) -> Result<(), String> {
    let reader = current_tree.join("chaptera-reader.exe");
    let status = Command::new(&reader)
        .arg("--smoke-check")
        .arg(fixture)
        .status()
        .map_err(|error| format!("launch {}: {error}", reader.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Reader smoke failed for {} with {status}",
            reader.display()
        ))
    }
}

fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let source = entry.path();
        let target = dst.join(entry.file_name());
        let metadata = entry.metadata().unwrap();
        if metadata.is_dir() {
            copy_tree(&source, &target);
        } else if metadata.is_file() {
            fs::copy(&source, &target).unwrap();
        } else {
            panic!("unsupported acceptance fixture entry: {}", source.display());
        }
    }
}

fn snapshot_tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let mut entries = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let metadata = entry.metadata().unwrap();
            if metadata.is_dir() {
                walk(root, &path, out);
            } else if metadata.is_file() {
                out.insert(path.strip_prefix(root).unwrap().to_path_buf(), fs::read(path).unwrap());
            } else {
                panic!("unsupported acceptance tree entry: {}", path.display());
            }
        }
    }

    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn tree_bytes(root: &Path) -> u64 {
    snapshot_tree(root)
        .values()
        .map(|bytes| bytes.len() as u64)
        .sum::<u64>()
        .max(1)
}

fn installed_context() -> InstalledUpdateContext<'static> {
    InstalledUpdateContext {
        product_id: "chaptera.reader",
        architecture: "windows-x86_64",
        channel: "stable",
        install_layout_epoch: INSTALL_LAYOUT_EPOCH,
        max_update_protocol_version: UPDATE_PROTOCOL_VERSION,
    }
}

fn release(version: &str, rollback_from: &str, installed_tree_bytes: u64) -> ChapteraReleaseSemantics {
    ChapteraReleaseSemantics {
        product_id: "chaptera.reader".into(),
        architecture: "windows-x86_64".into(),
        channel: "stable".into(),
        package_version: version.into(),
        install_layout_epoch: INSTALL_LAYOUT_EPOCH,
        update_protocol_version: UPDATE_PROTOCOL_VERSION,
        update_mode: UpdateMode::PayloadSwap,
        state_schema: "reader-state-v1".into(),
        rollback_compatible_from: vec![rollback_from.into()],
        installed_tree_bytes,
    }
}

#[test]
fn real_installed_reader_update_rollback_cycle() {
    let Some(root) = std::env::var_os("CHAPTERA_UPDATE_ACCEPT_INSTALL_ROOT").map(PathBuf::from) else {
        eprintln!("real installed Reader acceptance skipped: CHAPTERA_UPDATE_ACCEPT_INSTALL_ROOT unset");
        return;
    };
    let fixture = PathBuf::from(
        std::env::var_os("CHAPTERA_UPDATE_ACCEPT_PUB")
            .expect("CHAPTERA_UPDATE_ACCEPT_PUB must accompany install root"),
    );
    let external_state = PathBuf::from(
        std::env::var_os("CHAPTERA_UPDATE_ACCEPT_EXTERNAL_STATE")
            .expect("CHAPTERA_UPDATE_ACCEPT_EXTERNAL_STATE must accompany install root"),
    );

    let current = root.join("current");
    assert!(current.join("chaptera-reader.exe").is_file());
    let original_pub = fs::read(&fixture).unwrap();
    let original_external = fs::read(&external_state).unwrap();

    run_real_reader_smoke(&current, &fixture).unwrap();

    let temp = tempdir().unwrap();
    let candidate_b = temp.path().join("candidate-b");
    copy_tree(&current, &candidate_b);
    fs::write(candidate_b.join("acceptance-version.txt"), b"B").unwrap();

    let orchestrator = UpdateOrchestrator::new(&root);
    let mut healthy = RealReaderHooks {
        fixture: fixture.clone(),
        fail_after_smoke: false,
    };
    let b_release = release("0.2.0-acceptance", "0.1.0-preview", tree_bytes(&candidate_b));
    let outcome = orchestrator
        .apply_authenticated_candidate(
            "real-reader-a-to-b",
            "0.1.0-preview",
            &b_release,
            installed_context(),
            &candidate_b,
            Path::new("chaptera-reader.exe"),
            &mut healthy,
        )
        .unwrap();
    assert!(matches!(outcome, ApplyOutcome::Confirmed { .. }));
    assert_eq!(fs::read(current.join("acceptance-version.txt")).unwrap(), b"B");
    assert_eq!(fs::read(&fixture).unwrap(), original_pub);
    assert_eq!(fs::read(&external_state).unwrap(), original_external);

    let confirmed_b = snapshot_tree(&current);

    let candidate_c = temp.path().join("candidate-c");
    copy_tree(&current, &candidate_c);
    fs::write(candidate_c.join("acceptance-version.txt"), b"C").unwrap();

    let mut reject_after_real_smoke = RealReaderHooks {
        fixture: fixture.clone(),
        fail_after_smoke: true,
    };
    let c_release = release("0.3.0-bad-acceptance", "0.2.0-acceptance", tree_bytes(&candidate_c));
    let outcome = orchestrator
        .apply_authenticated_candidate(
            "real-reader-b-to-c-bad",
            "0.2.0-acceptance",
            &c_release,
            installed_context(),
            &candidate_c,
            Path::new("chaptera-reader.exe"),
            &mut reject_after_real_smoke,
        )
        .unwrap();
    assert!(matches!(outcome, ApplyOutcome::RolledBack { .. }));

    assert_eq!(
        snapshot_tree(&current),
        confirmed_b,
        "failed candidate rollback must restore byte-identical confirmed B tree"
    );
    assert_eq!(fs::read(current.join("acceptance-version.txt")).unwrap(), b"B");
    run_real_reader_smoke(&current, &fixture).unwrap();
    assert_eq!(fs::read(&fixture).unwrap(), original_pub);
    assert_eq!(fs::read(&external_state).unwrap(), original_external);
}
