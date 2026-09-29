use chaptera_update_engine::{RecoveryOutcome, UpdateEngine};
use chaptera_update_orchestrator::{ApplyOutcome, UpdateHooks, UpdateOrchestrator};
use chaptera_update_trust::{
    ChapteraReleaseSemantics, InstalledUpdateContext, UpdateMode, INSTALL_LAYOUT_EPOCH,
    UPDATE_PROTOCOL_VERSION,
};
use std::collections::BTreeMap;
use std::fs;
#[cfg(windows)]
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;
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


#[cfg(windows)]
fn wait_for_process_exit(
    child: &mut std::process::Child,
    timeout: std::time::Duration,
) -> Option<std::process::ExitStatus> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait().expect("poll Reader process") {
            Some(status) => return Some(status),
            None if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            None => return None,
        }
    }
}

#[cfg(windows)]
#[test]
fn real_installed_reader_restart_manager_quiesces_two_live_processes() {
    let Some(root) = std::env::var_os("CHAPTERA_UPDATE_ACCEPT_INSTALL_ROOT").map(PathBuf::from)
    else {
        eprintln!("installed Restart Manager acceptance skipped: CHAPTERA_UPDATE_ACCEPT_INSTALL_ROOT unset");
        return;
    };
    let fixture = PathBuf::from(
        std::env::var_os("CHAPTERA_UPDATE_ACCEPT_PUB")
            .expect("CHAPTERA_UPDATE_ACCEPT_PUB must accompany install root"),
    );
    let reader = root.join("current").join("chaptera-reader.exe");
    assert!(reader.is_file(), "installed Reader executable missing");

    let original_pub = fs::read(&fixture).expect("read source PUB before live Reader launch");

    let probe_dir = root.join("acceptance-rm-probes");
    fs::create_dir_all(&probe_dir).expect("create Restart Manager probe directory");
    let first_receipt = probe_dir.join("first.json");
    let second_receipt = probe_dir.join("second.json");

    let mut first = Command::new(&reader)
        .arg("--reader-activation-probe-v1")
        .arg(&fixture)
        .arg(&first_receipt)
        .arg("60000")
        .spawn()
        .expect("launch first installed Reader activation probe");
    let mut second = Command::new(&reader)
        .arg("--reader-activation-probe-v1")
        .arg(&fixture)
        .arg(&second_receipt)
        .arg("60000")
        .spawn()
        .expect("launch second installed Reader activation probe");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while (!first_receipt.is_file() || !second_receipt.is_file())
        && std::time::Instant::now() < deadline
    {
        if first.try_wait().expect("poll first Reader").is_some() {
            panic!("first Reader activation probe exited before receipt");
        }
        if second.try_wait().expect("poll second Reader").is_some() {
            panic!("second Reader activation probe exited before receipt");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(first_receipt.is_file(), "first Reader activation receipt missing");
    assert!(second_receipt.is_file(), "second Reader activation receipt missing");
    assert!(
        first.try_wait().expect("poll first Reader").is_none(),
        "first Reader exited before quiesce"
    );
    assert!(
        second.try_wait().expect("poll second Reader").is_none(),
        "second Reader exited before quiesce"
    );
    assert_ne!(first.id(), second.id(), "two Reader activations reused one PID");

    let report = chaptera_update_orchestrator::windows_restart_manager::quiesce_file_resource(
        &reader,
    )
    .expect("Restart Manager must gracefully quiesce installed Reader processes");

    let affected = report
        .before
        .iter()
        .map(|process| process.pid)
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        affected.contains(&first.id()),
        "Restart Manager did not account for first Reader PID {}: {:?}",
        first.id(),
        report.before
    );
    assert!(
        affected.contains(&second.id()),
        "Restart Manager did not account for second Reader PID {}: {:?}",
        second.id(),
        report.before
    );
    assert!(
        report.after.is_empty(),
        "Restart Manager left affected Reader processes: {:?}",
        report.after
    );
    assert_eq!(report.reboot_reasons_before, 0);
    assert_eq!(report.reboot_reasons_after, 0);

    let first_status = wait_for_process_exit(&mut first, std::time::Duration::from_secs(10))
        .expect("first Reader remained alive after Restart Manager shutdown");
    let second_status = wait_for_process_exit(&mut second, std::time::Duration::from_secs(10))
        .expect("second Reader remained alive after Restart Manager shutdown");
    assert!(
        first_status.success(),
        "first Reader did not exit cleanly: {first_status}"
    );
    assert!(
        second_status.success(),
        "second Reader did not exit cleanly: {second_status}"
    );

    assert_eq!(
        fs::read(&fixture).expect("read source PUB after quiesce"),
        original_pub,
        "Reader activation/quiesce mutated source PUB"
    );

    println!(
        "CHAPTERA_READER_RM_LIVE_RECEIPT affected_before={} first_pid={} second_pid={} affected_after=0 source_unchanged=true",
        report.before.len(),
        first.id(),
        second.id(),
    );
}

#[cfg(windows)]
#[test]
fn real_installed_reader_process_restart_and_file_lock_recovery_matrix() {
    let Some(root) = std::env::var_os("CHAPTERA_UPDATE_ACCEPT_INSTALL_ROOT").map(PathBuf::from) else {
        eprintln!("installed fault-recovery acceptance skipped: CHAPTERA_UPDATE_ACCEPT_INSTALL_ROOT unset");
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
    let original_tree = snapshot_tree(&current);
    let original_pub = fs::read(&fixture).unwrap();
    let original_external = fs::read(&external_state).unwrap();
    run_real_reader_smoke(&current, &fixture).unwrap();

    let temp = tempdir().unwrap();

    // Simulate process death after the previous tree was durably retained but
    // before candidate activation. Recovery is performed by a fresh engine
    // instance, which represents the next updater process.
    let candidate_retained = temp.path().join("candidate-retained");
    copy_tree(&current, &candidate_retained);
    fs::write(candidate_retained.join("acceptance-version.txt"), b"retained-fault").unwrap();

    {
        let engine = UpdateEngine::new(&root);
        engine
            .begin_verified_candidate(
                "installed-fault-after-retain",
                "0.2.0-retain-fault",
                &candidate_retained,
                Path::new("chaptera-reader.exe"),
            )
            .unwrap();
        engine.retain_previous().unwrap();
        assert!(!current.exists());
        // Drop without recovery: this is the simulated crashed updater.
    }

    let restarted = UpdateEngine::new(&root);
    assert_eq!(
        restarted.recover().unwrap(),
        RecoveryOutcome::UnconfirmedCandidateRolledBack
    );
    assert_eq!(
        snapshot_tree(&current),
        original_tree,
        "fresh process must restore byte-identical A after retained-tree crash"
    );
    restarted.cleanup_orphaned_transactions().unwrap();
    run_real_reader_smoke(&current, &fixture).unwrap();

    // Simulate process death after candidate activation but before durable
    // confirmation. A fresh process must reject the unconfirmed candidate and
    // restore the predecessor exactly.
    let candidate_activated = temp.path().join("candidate-activated");
    copy_tree(&current, &candidate_activated);
    fs::write(
        candidate_activated.join("acceptance-version.txt"),
        b"activated-fault",
    )
    .unwrap();

    {
        let engine = UpdateEngine::new(&root);
        engine
            .begin_verified_candidate(
                "installed-fault-after-activate",
                "0.2.0-activate-fault",
                &candidate_activated,
                Path::new("chaptera-reader.exe"),
            )
            .unwrap();
        engine.retain_previous().unwrap();
        engine.activate_candidate().unwrap();
        assert_eq!(
            fs::read(current.join("acceptance-version.txt")).unwrap(),
            b"activated-fault"
        );
        // Drop without confirm/recover.
    }

    let restarted = UpdateEngine::new(&root);
    assert_eq!(
        restarted.recover().unwrap(),
        RecoveryOutcome::UnconfirmedCandidateRolledBack
    );
    assert_eq!(
        snapshot_tree(&current),
        original_tree,
        "fresh process must restore byte-identical A after activation crash"
    );
    restarted.cleanup_orphaned_transactions().unwrap();
    run_real_reader_smoke(&current, &fixture).unwrap();

    // Hold the installed executable with Windows share mode 0. Candidate
    // preparation must fail rather than mutating current behind the live lock.
    // Once the file handle disappears, a fresh engine owns recovery.
    let candidate_locked = temp.path().join("candidate-locked");
    copy_tree(&current, &candidate_locked);
    fs::write(candidate_locked.join("acceptance-version.txt"), b"locked-fault").unwrap();

    let reader = current.join("chaptera-reader.exe");
    let exclusive_reader = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&reader)
        .expect("exclusive installed Reader handle");

    let engine = UpdateEngine::new(&root);
    let error = engine
        .begin_verified_candidate(
            "installed-fault-reader-lock",
            "0.2.0-lock-fault",
            &candidate_locked,
            Path::new("chaptera-reader.exe"),
        )
        .expect_err("exclusive Reader lock must fail candidate preparation");
    assert!(
        error.to_string().contains("I/O error"),
        "unexpected locked-file error: {error}"
    );
    drop(exclusive_reader);
    drop(engine);

    let restarted = UpdateEngine::new(&root);
    assert_eq!(
        restarted.recover().unwrap(),
        RecoveryOutcome::PreparedTransactionAborted
    );
    assert_eq!(
        snapshot_tree(&current),
        original_tree,
        "locked preparation recovery must leave installed A byte-identical"
    );
    restarted.cleanup_orphaned_transactions().unwrap();

    run_real_reader_smoke(&current, &fixture).unwrap();
    assert_eq!(fs::read(&fixture).unwrap(), original_pub);
    assert_eq!(fs::read(&external_state).unwrap(), original_external);

    println!(
        "CHAPTERA_INSTALLED_FAULT_RECOVERY_RECEIPT retained_restart=pass activated_restart=pass exclusive_reader_lock=pass final_reader_smoke=pass source_pub_unchanged=true external_state_unchanged=true"
    );
}
