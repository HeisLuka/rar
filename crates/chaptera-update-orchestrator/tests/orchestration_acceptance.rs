use chaptera_update_engine::RecoveryOutcome;
use chaptera_update_orchestrator::{
    ApplyOutcome, InstallLock, OrchestrationError, UpdateHooks, UpdateOrchestrator,
};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

fn seed_tree(path: &Path, updater: &[u8], reader: &[u8]) {
    fs::create_dir_all(path).unwrap();
    fs::write(path.join("chaptera-updater.bin"), updater).unwrap();
    fs::write(path.join("reader.bin"), reader).unwrap();
}

#[derive(Default)]
struct RecordingHooks {
    quiesce_error: Option<String>,
    health_error: Option<String>,
    control_bytes: Option<Vec<u8>>,
    health_updater_bytes: Option<Vec<u8>>,
}

impl UpdateHooks for RecordingHooks {
    fn quiesce(&mut self, control_updater: &Path) -> Result<(), String> {
        self.control_bytes = Some(fs::read(control_updater).map_err(|e| e.to_string())?);
        if let Some(reason) = self.quiesce_error.clone() {
            return Err(reason);
        }
        Ok(())
    }

    fn health_check(&mut self, current_tree: &Path) -> Result<(), String> {
        self.health_updater_bytes = Some(
            fs::read(current_tree.join("chaptera-updater.bin")).map_err(|e| e.to_string())?,
        );
        if let Some(reason) = self.health_error.clone() {
            return Err(reason);
        }
        Ok(())
    }
}

#[test]
fn os_lock_contends_but_stale_lock_file_does_not() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");

    let first = InstallLock::try_acquire(&root).unwrap();
    assert!(first.path().is_file());

    let second = InstallLock::try_acquire(&root).unwrap_err();
    assert!(matches!(second, OrchestrationError::LockBusy));

    drop(first);

    // The diagnostic file intentionally remains. A released OS lock must make
    // it reusable instead of turning a crash artifact into a permanent block.
    assert!(root.join(".chaptera-install.lock").is_file());
    let third = InstallLock::try_acquire(&root).unwrap();
    drop(third);
}

#[test]
fn held_lock_blocks_apply_before_any_transaction_mutation() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let candidate = temp.path().join("candidate");
    seed_tree(&root.join("current"), b"U1", b"reader-v1");
    seed_tree(&candidate, b"U2", b"reader-v2");

    let _held = InstallLock::try_acquire(&root).unwrap();
    let orchestrator = UpdateOrchestrator::new(&root);
    let mut hooks = RecordingHooks::default();
    let err = orchestrator
        .apply_verified_candidate(
            "tx-locked",
            "2.0.0",
            &candidate,
            Path::new("chaptera-updater.bin"),
            &mut hooks,
        )
        .unwrap_err();

    assert!(matches!(err, OrchestrationError::LockBusy));
    assert_eq!(fs::read(root.join("current/chaptera-updater.bin")).unwrap(), b"U1");
    assert!(orchestrator.engine().read_journal().unwrap().is_none());
}

#[test]
fn quiesce_failure_aborts_prepared_candidate_without_switching_current() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let candidate = temp.path().join("candidate");
    seed_tree(&root.join("current"), b"U1", b"reader-v1");
    seed_tree(&candidate, b"U2", b"reader-v2");

    let orchestrator = UpdateOrchestrator::new(&root);
    let mut hooks = RecordingHooks {
        quiesce_error: Some("reader refused to stop".into()),
        ..Default::default()
    };

    let err = orchestrator
        .apply_verified_candidate(
            "tx-quiesce",
            "2.0.0",
            &candidate,
            Path::new("chaptera-updater.bin"),
            &mut hooks,
        )
        .unwrap_err();

    assert!(matches!(err, OrchestrationError::QuiesceFailed { .. }));
    assert_eq!(hooks.control_bytes.as_deref(), Some(b"U1".as_slice()));
    assert_eq!(fs::read(root.join("current/chaptera-updater.bin")).unwrap(), b"U1");
    assert!(orchestrator.engine().read_journal().unwrap().is_none());
}

#[test]
fn failed_health_check_rolls_unconfirmed_u2_back_to_u1() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let candidate = temp.path().join("candidate");
    seed_tree(&root.join("current"), b"U1", b"reader-v1");
    seed_tree(&candidate, b"U2", b"reader-v2");

    let orchestrator = UpdateOrchestrator::new(&root);
    let mut hooks = RecordingHooks {
        health_error: Some("candidate smoke failed".into()),
        ..Default::default()
    };

    let outcome = orchestrator
        .apply_verified_candidate(
            "tx-health-fail",
            "2.0.0",
            &candidate,
            Path::new("chaptera-updater.bin"),
            &mut hooks,
        )
        .unwrap();

    assert_eq!(hooks.control_bytes.as_deref(), Some(b"U1".as_slice()));
    assert_eq!(hooks.health_updater_bytes.as_deref(), Some(b"U2".as_slice()));
    assert!(matches!(
        outcome,
        ApplyOutcome::RolledBack {
            ref reason,
            startup_recovery: RecoveryOutcome::NothingToDo,
            ..
        } if reason == "candidate smoke failed"
    ));
    assert_eq!(fs::read(root.join("current/chaptera-updater.bin")).unwrap(), b"U1");
    assert!(orchestrator.engine().read_journal().unwrap().is_none());
}

#[test]
fn healthy_u2_is_confirmed_only_after_health_hook_succeeds() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let candidate = temp.path().join("candidate");
    let external = temp.path().join("user-state.txt");
    seed_tree(&root.join("current"), b"U1", b"reader-v1");
    seed_tree(&candidate, b"U2", b"reader-v2");
    fs::write(&external, b"must-survive").unwrap();

    let orchestrator = UpdateOrchestrator::new(&root);
    let mut hooks = RecordingHooks::default();
    let outcome = orchestrator
        .apply_verified_candidate(
            "tx-health-pass",
            "2.0.0",
            &candidate,
            Path::new("chaptera-updater.bin"),
            &mut hooks,
        )
        .unwrap();

    assert_eq!(hooks.control_bytes.as_deref(), Some(b"U1".as_slice()));
    assert_eq!(hooks.health_updater_bytes.as_deref(), Some(b"U2".as_slice()));
    assert!(matches!(
        outcome,
        ApplyOutcome::Confirmed {
            startup_recovery: RecoveryOutcome::NothingToDo,
            ..
        }
    ));
    assert_eq!(fs::read(root.join("current/chaptera-updater.bin")).unwrap(), b"U2");
    assert_eq!(fs::read(&external).unwrap(), b"must-survive");
    assert!(orchestrator.engine().read_journal().unwrap().is_none());
}

#[test]
fn stale_unconfirmed_u2_is_recovered_before_next_transaction_uses_u1_control() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let candidate_v2 = temp.path().join("candidate-v2");
    let candidate_v3 = temp.path().join("candidate-v3");
    seed_tree(&root.join("current"), b"U1", b"reader-v1");
    seed_tree(&candidate_v2, b"U2", b"reader-v2");
    seed_tree(&candidate_v3, b"U3", b"reader-v3");

    let orchestrator = UpdateOrchestrator::new(&root);

    // Simulate a prior process dying after U2 activation but before health
    // confirmation. The next owner must recover U1 while holding the OS lock.
    orchestrator
        .engine()
        .begin_verified_candidate(
            "tx-stale",
            "2.0.0",
            &candidate_v2,
            Path::new("chaptera-updater.bin"),
        )
        .unwrap();
    orchestrator.engine().retain_previous().unwrap();
    orchestrator.engine().activate_candidate().unwrap();
    assert_eq!(fs::read(root.join("current/chaptera-updater.bin")).unwrap(), b"U2");

    let mut hooks = RecordingHooks::default();
    let outcome = orchestrator
        .apply_verified_candidate(
            "tx-next",
            "3.0.0",
            &candidate_v3,
            Path::new("chaptera-updater.bin"),
            &mut hooks,
        )
        .unwrap();

    assert_eq!(
        hooks.control_bytes.as_deref(),
        Some(b"U1".as_slice()),
        "next transaction must be controlled by recovered confirmed U1, not stale U2"
    );
    assert!(matches!(
        outcome,
        ApplyOutcome::Confirmed {
            startup_recovery: RecoveryOutcome::UnconfirmedCandidateRolledBack,
            ..
        }
    ));
    assert_eq!(fs::read(root.join("current/chaptera-updater.bin")).unwrap(), b"U3");
}
