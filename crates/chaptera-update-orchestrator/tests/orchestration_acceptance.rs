use chaptera_update_engine::RecoveryOutcome;
use chaptera_update_trust::{ChapteraReleaseSemantics, InstalledUpdateContext, UpdateMode, INSTALL_LAYOUT_EPOCH, UPDATE_PROTOCOL_VERSION};
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


fn authenticated_release(mode: UpdateMode, rollback_from: &[&str]) -> ChapteraReleaseSemantics {
    ChapteraReleaseSemantics {
        product_id: "chaptera.reader".into(),
        architecture: "windows-x86_64".into(),
        channel: "stable".into(),
        package_version: "2.0.0".into(),
        install_layout_epoch: INSTALL_LAYOUT_EPOCH,
        update_protocol_version: UPDATE_PROTOCOL_VERSION,
        update_mode: mode,
        state_schema: "reader-state-v1".into(),
        rollback_compatible_from: rollback_from.iter().map(|value| (*value).into()).collect(),
        installed_tree_bytes: 1024,
    }
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


#[test]
fn copied_control_can_continue_existing_prepared_transaction_without_restaging() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let candidate = temp.path().join("candidate");
    seed_tree(&root.join("current"), b"U1", b"reader-v1");
    seed_tree(&candidate, b"U2", b"reader-v2");

    let orchestrator = UpdateOrchestrator::new(&root);
    let control = orchestrator
        .engine()
        .begin_verified_candidate(
            "tx-control-continue",
            "2.0.0",
            &candidate,
            Path::new("chaptera-updater.bin"),
        )
        .unwrap();
    assert_eq!(fs::read(&control).unwrap(), b"U1");

    let _control_lock = InstallLock::try_acquire(&root).unwrap();
    let mut hooks = RecordingHooks::default();
    let outcome = orchestrator.continue_prepared_candidate(&mut hooks).unwrap();

    assert!(matches!(
        outcome,
        ApplyOutcome::Confirmed {
            startup_recovery: RecoveryOutcome::NothingToDo,
            ..
        }
    ));
    assert_eq!(hooks.control_bytes.as_deref(), Some(b"U1".as_slice()));
    assert_eq!(hooks.health_updater_bytes.as_deref(), Some(b"U2".as_slice()));
    assert_eq!(fs::read(root.join("current/chaptera-updater.bin")).unwrap(), b"U2");
    assert!(control.is_file(), "running control bytes must survive terminal confirmation");
    assert!(orchestrator.engine().read_journal().unwrap().is_none());
}

#[test]
fn copied_control_health_failure_rolls_back_without_deleting_its_own_bytes() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let candidate = temp.path().join("candidate");
    seed_tree(&root.join("current"), b"U1", b"reader-v1");
    seed_tree(&candidate, b"U2", b"reader-v2");

    let orchestrator = UpdateOrchestrator::new(&root);
    let control = orchestrator
        .engine()
        .begin_verified_candidate(
            "tx-control-rollback",
            "2.0.0",
            &candidate,
            Path::new("chaptera-updater.bin"),
        )
        .unwrap();

    let _control_lock = InstallLock::try_acquire(&root).unwrap();
    let mut hooks = RecordingHooks {
        health_error: Some("candidate smoke failed".into()),
        ..Default::default()
    };
    let outcome = orchestrator.continue_prepared_candidate(&mut hooks).unwrap();

    assert!(matches!(outcome, ApplyOutcome::RolledBack { .. }));
    assert_eq!(fs::read(root.join("current/chaptera-updater.bin")).unwrap(), b"U1");
    assert!(control.is_file(), "rollback must not unlink the running control updater");
    assert!(orchestrator.engine().read_journal().unwrap().is_none());
}


#[test]
fn installer_required_is_rejected_before_transaction_mutation() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let candidate = temp.path().join("candidate");
    seed_tree(&root.join("current"), b"U1", b"reader-v1");
    seed_tree(&candidate, b"U2", b"reader-v2");

    let orchestrator = UpdateOrchestrator::new(&root);
    let mut hooks = RecordingHooks::default();
    let err = orchestrator
        .apply_authenticated_candidate(
            "tx-installer-required",
            "1.0.0",
            &authenticated_release(UpdateMode::InstallerRequired, &["1.0.0"]),
            installed_context(),
            &candidate,
            Path::new("chaptera-updater.bin"),
            &mut hooks,
        )
        .unwrap_err();

    assert!(matches!(err, OrchestrationError::InstallerRequired { .. }));
    assert!(orchestrator.engine().read_journal().unwrap().is_none());
    assert_eq!(fs::read(root.join("current/chaptera-updater.bin")).unwrap(), b"U1");
}

#[test]
fn payload_swap_without_authenticated_rollback_edge_fails_closed_before_mutation() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let candidate = temp.path().join("candidate");
    seed_tree(&root.join("current"), b"U1", b"reader-v1");
    seed_tree(&candidate, b"U2", b"reader-v2");

    let orchestrator = UpdateOrchestrator::new(&root);
    let mut hooks = RecordingHooks::default();
    let err = orchestrator
        .apply_authenticated_candidate(
            "tx-no-rollback-edge",
            "1.0.0",
            &authenticated_release(UpdateMode::PayloadSwap, &["0.9.0"]),
            installed_context(),
            &candidate,
            Path::new("chaptera-updater.bin"),
            &mut hooks,
        )
        .unwrap_err();

    assert!(matches!(err, OrchestrationError::PolicyRejected { .. }));
    assert!(orchestrator.engine().read_journal().unwrap().is_none());
    assert_eq!(fs::read(root.join("current/chaptera-updater.bin")).unwrap(), b"U1");
}

#[test]
fn authenticated_rollback_compatible_payload_swap_uses_existing_engine() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let candidate = temp.path().join("candidate");
    seed_tree(&root.join("current"), b"U1", b"reader-v1");
    seed_tree(&candidate, b"U2", b"reader-v2");

    let orchestrator = UpdateOrchestrator::new(&root);
    let mut hooks = RecordingHooks::default();
    let outcome = orchestrator
        .apply_authenticated_candidate(
            "tx-authenticated-swap",
            "1.0.0",
            &authenticated_release(UpdateMode::PayloadSwap, &["1.0.0"]),
            installed_context(),
            &candidate,
            Path::new("chaptera-updater.bin"),
            &mut hooks,
        )
        .unwrap();

    assert!(matches!(outcome, ApplyOutcome::Confirmed { .. }));
    assert_eq!(fs::read(root.join("current/chaptera-updater.bin")).unwrap(), b"U2");
}
