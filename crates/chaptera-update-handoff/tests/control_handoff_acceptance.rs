use chaptera_update_engine::{RecoveryOutcome, UpdateEngine};
use chaptera_update_handoff::{
    prepare_control_handoff, read_control_request, receipt_path, started_path,
    validate_request_against_engine, ControlReceipt, HandoffError,
};
use chaptera_update_orchestrator::InstallLock;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::tempdir;

fn copy_probe(dst: &Path) {
    let probe = PathBuf::from(env!("CARGO_BIN_EXE_chaptera-update-control-probe"));
    fs::create_dir_all(dst.parent().unwrap()).unwrap();
    fs::copy(probe, dst).unwrap();
}

fn seed_candidate(path: &Path, updater_bytes: &[u8]) {
    fs::create_dir_all(path).unwrap();
    fs::write(path.join("chaptera-updater.exe"), updater_bytes).unwrap();
    fs::write(path.join("reader.bin"), b"candidate-reader").unwrap();
}

fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.is_file() {
        assert!(Instant::now() < deadline, "timed out waiting for {}", path.display());
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn copied_u1_process_cannot_take_over_until_parent_releases_install_lock() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let candidate = temp.path().join("candidate");
    let updater_rel = Path::new("chaptera-updater.exe");

    copy_probe(&root.join("current").join(updater_rel));
    fs::write(root.join("current/reader.bin"), b"reader-v1").unwrap();
    seed_candidate(&candidate, b"U2-not-yet-usable");

    let parent_lock = InstallLock::try_acquire(&root).unwrap();
    let engine = UpdateEngine::new(&root);
    let control_updater = engine
        .begin_verified_candidate("tx-handoff", "2.0.0", &candidate, updater_rel)
        .unwrap();
    let handoff = prepare_control_handoff(&engine).unwrap();
    assert_eq!(handoff.control_updater, control_updater);

    let started = started_path(&handoff.request_path);
    let receipt = receipt_path(&handoff.request_path);
    let mut child = handoff.spawn().unwrap();

    wait_for_file(&started);
    assert!(
        child.try_wait().unwrap().is_none(),
        "control child exited while parent still owned install lock"
    );
    assert!(
        !receipt.exists(),
        "control child produced ownership receipt before parent released install lock"
    );

    drop(parent_lock);

    let status = child.wait().unwrap();
    assert!(status.success(), "copied control updater exited with {status}");
    wait_for_file(&receipt);

    let receipt: ControlReceipt =
        serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
    assert_eq!(receipt.transaction_id, "tx-handoff");

    let actual_exe = fs::canonicalize(receipt.executable).unwrap();
    let expected_control = fs::canonicalize(&control_updater).unwrap();
    assert_eq!(
        actual_exe, expected_control,
        "child must execute from transaction-local copied U1 control path"
    );
    assert!(
        !actual_exe.starts_with(fs::canonicalize(root.join("current")).unwrap()),
        "control process must not be executing from current/"
    );

    let request = read_control_request(&handoff.request_path).unwrap();
    validate_request_against_engine(&request, &engine).unwrap();

    assert_eq!(
        fs::read(root.join("current/chaptera-updater.exe")).unwrap(),
        fs::read(PathBuf::from(env!("CARGO_BIN_EXE_chaptera-update-control-probe"))).unwrap(),
        "handoff alone must not switch current"
    );

    assert_eq!(
        engine.recover().unwrap(),
        RecoveryOutcome::PreparedTransactionAborted
    );
}

#[test]
fn request_mismatch_is_rejected_before_control_can_continue_transaction() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let candidate = temp.path().join("candidate");
    let updater_rel = Path::new("chaptera-updater.exe");

    copy_probe(&root.join("current").join(updater_rel));
    seed_candidate(&candidate, b"U2");

    let engine = UpdateEngine::new(&root);
    engine
        .begin_verified_candidate("tx-request", "2.0.0", &candidate, updater_rel)
        .unwrap();
    let handoff = prepare_control_handoff(&engine).unwrap();

    let mut request = read_control_request(&handoff.request_path).unwrap();
    request.transaction_id = "tx-other".into();

    let err = validate_request_against_engine(&request, &engine).unwrap_err();
    assert!(matches!(err, HandoffError::Mismatch(ref field) if field == "transaction_id"));

    assert_eq!(
        engine.recover().unwrap(),
        RecoveryOutcome::PreparedTransactionAborted
    );
}

#[test]
fn handoff_requires_prepared_journal_and_existing_control_copy() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("install");
    let engine = UpdateEngine::new(&root);

    let missing = prepare_control_handoff(&engine).unwrap_err();
    assert!(matches!(missing, HandoffError::JournalMissing));
}
