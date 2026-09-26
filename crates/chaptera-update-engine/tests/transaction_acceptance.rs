use chaptera_update_engine::{RecoveryOutcome, UpdateEngine, UpdatePhase};
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn seed_current(root: &Path) {
    let current = root.join("current");
    fs::create_dir_all(&current).unwrap();
    fs::write(current.join("chaptera-updater.bin"), b"U1").unwrap();
    fs::write(current.join("reader.bin"), b"reader-v1").unwrap();
}

fn seed_candidate(base: &Path) {
    fs::create_dir_all(base).unwrap();
    fs::write(base.join("chaptera-updater.bin"), b"U2").unwrap();
    fs::write(base.join("reader.bin"), b"reader-v2").unwrap();
}

#[test]
fn confirmed_candidate_keeps_u1_as_control_until_confirmation() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let candidate = temp.path().join("verified-candidate");
    let sentinel = temp.path().join("external-user-state.txt");
    seed_current(&root);
    seed_candidate(&candidate);
    fs::write(&sentinel, b"must-survive").unwrap();

    let engine = UpdateEngine::new(&root);
    let control = engine
        .begin_verified_candidate(
            "tx-confirm",
            "2.0.0",
            &candidate,
            Path::new("chaptera-updater.bin"),
        )
        .unwrap();

    assert_eq!(fs::read(&control).unwrap(), b"U1");
    assert_eq!(engine.read_journal().unwrap().unwrap().phase, UpdatePhase::Prepared);

    engine.retain_previous().unwrap();
    engine.activate_candidate().unwrap();

    assert_eq!(fs::read(root.join("current/chaptera-updater.bin")).unwrap(), b"U2");
    assert_eq!(fs::read(&control).unwrap(), b"U1");
    assert_eq!(
        engine.read_journal().unwrap().unwrap().phase,
        UpdatePhase::CandidateActivated
    );

    engine.confirm_candidate().unwrap();

    assert_eq!(fs::read(root.join("current/chaptera-updater.bin")).unwrap(), b"U2");
    assert!(!control.exists());
    assert!(engine.read_journal().unwrap().is_none());
    assert!(engine.journal_previous_path().is_file());
    assert_eq!(fs::read(&sentinel).unwrap(), b"must-survive");
}

#[test]
fn crash_after_previous_retained_restores_u1() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let candidate = temp.path().join("verified-candidate");
    seed_current(&root);
    seed_candidate(&candidate);

    let engine = UpdateEngine::new(&root);
    engine
        .begin_verified_candidate(
            "tx-retained",
            "2.0.0",
            &candidate,
            Path::new("chaptera-updater.bin"),
        )
        .unwrap();
    engine.retain_previous().unwrap();

    assert!(!root.join("current").exists());
    assert_eq!(
        engine.recover().unwrap(),
        RecoveryOutcome::UnconfirmedCandidateRolledBack
    );
    assert_eq!(fs::read(root.join("current/chaptera-updater.bin")).unwrap(), b"U1");
    assert!(engine.read_journal().unwrap().is_none());
}

#[test]
fn crash_after_candidate_activation_rolls_back_unconfirmed_u2() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let candidate = temp.path().join("verified-candidate");
    let sentinel = temp.path().join("external-user-state.txt");
    seed_current(&root);
    seed_candidate(&candidate);
    fs::write(&sentinel, b"outside-product-tree").unwrap();

    let engine = UpdateEngine::new(&root);
    engine
        .begin_verified_candidate(
            "tx-activated",
            "2.0.0",
            &candidate,
            Path::new("chaptera-updater.bin"),
        )
        .unwrap();
    engine.retain_previous().unwrap();
    engine.activate_candidate().unwrap();

    assert_eq!(fs::read(root.join("current/chaptera-updater.bin")).unwrap(), b"U2");
    assert_eq!(
        engine.recover().unwrap(),
        RecoveryOutcome::UnconfirmedCandidateRolledBack
    );
    assert_eq!(fs::read(root.join("current/chaptera-updater.bin")).unwrap(), b"U1");
    assert_eq!(fs::read(&sentinel).unwrap(), b"outside-product-tree");
}

#[test]
fn rejects_updater_path_traversal_before_mutating_layout() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let candidate = temp.path().join("verified-candidate");
    seed_current(&root);
    seed_candidate(&candidate);

    let engine = UpdateEngine::new(&root);
    let error = engine
        .begin_verified_candidate("tx-bad-path", "2.0.0", &candidate, Path::new("../escape.exe"))
        .unwrap_err();

    assert!(error.to_string().contains("safe relative path"));
    assert!(engine.read_journal().unwrap().is_none());
    assert_eq!(fs::read(root.join("current/chaptera-updater.bin")).unwrap(), b"U1");
}


#[test]
fn crash_after_current_rename_before_phase_persist_restores_u1() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let candidate = temp.path().join("verified-candidate");
    seed_current(&root);
    seed_candidate(&candidate);

    let engine = UpdateEngine::new(&root);
    engine
        .begin_verified_candidate(
            "tx-pre-retain-journal",
            "2.0.0",
            &candidate,
            Path::new("chaptera-updater.bin"),
        )
        .unwrap();

    let paths = engine
        .paths_for("tx-pre-retain-journal", Path::new("chaptera-updater.bin"))
        .unwrap();
    fs::create_dir_all(&paths.rollback_transaction).unwrap();

    // Fault injection: retain_previous() has moved current, but the process dies
    // before PreviousRetained can be persisted.
    fs::rename(root.join("current"), &paths.previous_tree).unwrap();
    assert_eq!(engine.read_journal().unwrap().unwrap().phase, UpdatePhase::Prepared);

    assert_eq!(
        engine.recover().unwrap(),
        RecoveryOutcome::PreparedTransactionAborted
    );
    assert_eq!(fs::read(root.join("current/chaptera-updater.bin")).unwrap(), b"U1");
}

#[test]
fn crash_after_candidate_rename_before_phase_persist_rolls_back_u2() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let candidate = temp.path().join("verified-candidate");
    seed_current(&root);
    seed_candidate(&candidate);

    let engine = UpdateEngine::new(&root);
    engine
        .begin_verified_candidate(
            "tx-pre-activate-journal",
            "2.0.0",
            &candidate,
            Path::new("chaptera-updater.bin"),
        )
        .unwrap();
    engine.retain_previous().unwrap();

    let paths = engine
        .paths_for("tx-pre-activate-journal", Path::new("chaptera-updater.bin"))
        .unwrap();

    // Fault injection: activate_candidate() has renamed the candidate into
    // current, but CandidateActivated was not persisted yet.
    fs::rename(&paths.staged_candidate, root.join("current")).unwrap();
    assert_eq!(
        engine.read_journal().unwrap().unwrap().phase,
        UpdatePhase::PreviousRetained
    );

    assert_eq!(
        engine.recover().unwrap(),
        RecoveryOutcome::UnconfirmedCandidateRolledBack
    );
    assert_eq!(fs::read(root.join("current/chaptera-updater.bin")).unwrap(), b"U1");
}
