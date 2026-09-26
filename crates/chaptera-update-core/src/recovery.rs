use crate::{InstallLayout, UpdateJournal, UpdatePhase, tree_sha256};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeIdentity {
    Missing,
    Candidate,
    Previous,
    Unknown { sha256: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "directive")]
pub enum RecoveryDirective {
    AwaitOriginExit,
    RetainPrevious,
    RecordPreviousRetained,
    PromoteCandidate,
    RecordCandidatePromoted,
    RunCandidateSelfCheck,
    MoveCandidateAsideAndRestorePrevious,
    RestorePrevious,
    RecordPreviousRestored,
    RunPreviousSelfCheck,
    Confirmed,
    RolledBack,
    RepairRequired { reason: String },
}

pub fn reconcile(layout: &InstallLayout, journal: &UpdateJournal) -> Result<RecoveryDirective> {
    let current = classify(
        &layout.current(),
        &journal.previous_tree_sha256,
        &journal.candidate_tree_sha256,
    )?;
    let staging = classify(
        &layout.staging_for(&journal.attempt_id)?,
        &journal.previous_tree_sha256,
        &journal.candidate_tree_sha256,
    )?;
    let rollback = classify(
        &layout.rollback_for(&journal.from_version, &journal.attempt_id)?,
        &journal.previous_tree_sha256,
        &journal.candidate_tree_sha256,
    )?;

    use RecoveryDirective as D;
    use TreeIdentity as T;
    use UpdatePhase as P;

    let directive = match journal.phase {
        P::Prepared => match (&current, &staging, &rollback) {
            (T::Previous, T::Candidate, T::Missing) => D::AwaitOriginExit,
            (T::Missing, T::Candidate, T::Previous) => D::RecordPreviousRetained,
            (T::Candidate, T::Missing, T::Previous) => D::RecordCandidatePromoted,
            _ => repair(journal, current, staging, rollback),
        },
        P::OriginExited => match (&current, &staging, &rollback) {
            (T::Previous, T::Candidate, T::Missing) => D::RetainPrevious,
            (T::Missing, T::Candidate, T::Previous) => D::RecordPreviousRetained,
            (T::Candidate, T::Missing, T::Previous) => D::RecordCandidatePromoted,
            _ => repair(journal, current, staging, rollback),
        },
        P::PreviousRetained => match (&current, &staging, &rollback) {
            (T::Missing, T::Candidate, T::Previous) => D::PromoteCandidate,
            (T::Candidate, T::Missing, T::Previous) => D::RecordCandidatePromoted,
            _ => repair(journal, current, staging, rollback),
        },
        P::CandidatePromotedUnconfirmed => match (&current, &rollback) {
            (T::Candidate, T::Previous) => D::RunCandidateSelfCheck,
            (T::Missing, T::Previous) if journal.rollback_compatible => D::RestorePrevious,
            (T::Previous, T::Missing) if journal.rollback_compatible => D::RecordPreviousRestored,
            _ => repair(journal, current, staging, rollback),
        },
        P::CandidateConfirmed => match current {
            T::Candidate => D::Confirmed,
            other => D::RepairRequired {
                reason: format!(
                    "confirmed candidate tree no longer matches expected digest: {other:?}"
                ),
            },
        },
        P::RollbackStarted => {
            if !journal.rollback_compatible {
                D::RepairRequired {
                    reason: "rollback entered for an edge not declared rollback-compatible".into(),
                }
            } else {
                match (&current, &rollback) {
                    (T::Candidate, T::Previous) => D::MoveCandidateAsideAndRestorePrevious,
                    (T::Missing, T::Previous) => D::RestorePrevious,
                    (T::Previous, T::Missing) => D::RecordPreviousRestored,
                    _ => repair(journal, current, staging, rollback),
                }
            }
        }
        P::PreviousRestored => match current {
            T::Previous => D::RunPreviousSelfCheck,
            other => D::RepairRequired {
                reason: format!(
                    "restored predecessor tree does not match expected digest: {other:?}"
                ),
            },
        },
        P::RollbackConfirmed => match current {
            T::Previous => D::RolledBack,
            other => D::RepairRequired {
                reason: format!(
                    "rollback-confirmed tree does not match predecessor digest: {other:?}"
                ),
            },
        },
        P::RepairRequired => D::RepairRequired {
            reason: "journal is already terminal RepairRequired".into(),
        },
    };

    Ok(directive)
}

fn classify(path: &Path, previous: &str, candidate: &str) -> Result<TreeIdentity> {
    if !path.exists() {
        return Ok(TreeIdentity::Missing);
    }
    let sha256 = tree_sha256(path)?;
    if sha256 == previous {
        Ok(TreeIdentity::Previous)
    } else if sha256 == candidate {
        Ok(TreeIdentity::Candidate)
    } else {
        Ok(TreeIdentity::Unknown { sha256 })
    }
}

fn repair(
    journal: &UpdateJournal,
    current: TreeIdentity,
    staging: TreeIdentity,
    rollback: TreeIdentity,
) -> RecoveryDirective {
    RecoveryDirective::RepairRequired {
        reason: format!(
            "ambiguous update topology at phase {:?}: current={current:?}, staging={staging:?}, rollback={rollback:?}",
            journal.phase
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UpdateJournal;
    use std::fs;
    use tempfile::tempdir;

    struct Fixture {
        layout: InstallLayout,
        journal: UpdateJournal,
        staging: std::path::PathBuf,
        rollback: std::path::PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempdir().unwrap();
        let root = dir.keep();
        let layout = InstallLayout::new(&root);
        fs::create_dir_all(layout.current()).unwrap();
        fs::write(layout.current().join("reader.exe"), b"old").unwrap();

        let previous = tree_sha256(&layout.current()).unwrap();
        let mut journal = UpdateJournal::new_payload_swap(
            "chaptera.reader",
            "windows-x86_64",
            "stable",
            "0.1.0",
            "0.2.0",
            1,
            1,
            previous,
            "",
            true,
            "reader-state-v1",
        );
        let staging = layout.staging_for(&journal.attempt_id).unwrap();
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("reader.exe"), b"new").unwrap();
        journal.candidate_tree_sha256 = tree_sha256(&staging).unwrap();
        let rollback = layout
            .rollback_for(&journal.from_version, &journal.attempt_id)
            .unwrap();

        Fixture {
            layout,
            journal,
            staging,
            rollback,
        }
    }

    #[test]
    fn detects_crash_after_previous_rename_before_phase_persist() {
        let mut f = fixture();
        f.journal.phase = UpdatePhase::OriginExited;
        fs::create_dir_all(f.rollback.parent().unwrap()).unwrap();
        fs::rename(f.layout.current(), &f.rollback).unwrap();

        assert_eq!(
            reconcile(&f.layout, &f.journal).unwrap(),
            RecoveryDirective::RecordPreviousRetained
        );
    }

    #[test]
    fn detects_crash_after_candidate_promotion_before_phase_persist() {
        let mut f = fixture();
        f.journal.phase = UpdatePhase::PreviousRetained;
        fs::create_dir_all(f.rollback.parent().unwrap()).unwrap();
        fs::rename(f.layout.current(), &f.rollback).unwrap();
        fs::rename(&f.staging, f.layout.current()).unwrap();

        assert_eq!(
            reconcile(&f.layout, &f.journal).unwrap(),
            RecoveryDirective::RecordCandidatePromoted
        );
    }

    #[test]
    fn detects_previous_already_restored_during_rollback() {
        let mut f = fixture();
        f.journal.phase = UpdatePhase::RollbackStarted;
        fs::create_dir_all(f.rollback.parent().unwrap()).unwrap();
        fs::rename(f.layout.current(), &f.rollback).unwrap();
        fs::rename(&f.staging, f.layout.current()).unwrap();

        let failed = f.layout.failed_candidate_for(&f.journal.attempt_id).unwrap();
        fs::rename(f.layout.current(), failed).unwrap();
        fs::rename(&f.rollback, f.layout.current()).unwrap();

        assert_eq!(
            reconcile(&f.layout, &f.journal).unwrap(),
            RecoveryDirective::RecordPreviousRestored
        );
    }

    #[test]
    fn ambiguous_unknown_tree_fails_closed() {
        let mut f = fixture();
        f.journal.phase = UpdatePhase::CandidateConfirmed;
        fs::write(f.layout.current().join("reader.exe"), b"corrupt").unwrap();

        assert!(matches!(
            reconcile(&f.layout, &f.journal).unwrap(),
            RecoveryDirective::RepairRequired { .. }
        ));
    }
}
