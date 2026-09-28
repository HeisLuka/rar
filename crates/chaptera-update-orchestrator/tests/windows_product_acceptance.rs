use chaptera_update_engine::RecoveryOutcome;
use chaptera_update_orchestrator::{ApplyOutcome, UpdateHooks, UpdateOrchestrator};
use std::fs;
use std::path::Path;
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
