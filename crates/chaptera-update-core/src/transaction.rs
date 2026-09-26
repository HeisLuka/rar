use crate::{
    InstallLayout, JournalStore, RecoveryDirective, UpdateJournal, UpdatePhase, reconcile, tree_sha256,
};
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failpoint {
    None,
    AfterOriginExitedJournal,
    AfterPreviousRename,
    AfterPreviousRetainedJournal,
    AfterCandidateRename,
    AfterCandidatePromotedJournal,
    AfterRollbackStartedJournal,
    AfterFailedCandidateRename,
    AfterPreviousRestoreRename,
    AfterPreviousRestoredJournal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfCheckResult {
    Pass,
    Fail,
}

pub trait ProductSelfCheck {
    fn check(&self, current_tree: &Path) -> Result<SelfCheckResult>;
}

impl<F> ProductSelfCheck for F
where
    F: Fn(&Path) -> Result<SelfCheckResult>,
{
    fn check(&self, current_tree: &Path) -> Result<SelfCheckResult> {
        self(current_tree)
    }
}

#[derive(Debug)]
pub enum TransactionOutcome {
    CandidateConfirmed,
    PreviousRestored,
    RepairRequired(String),
    InjectedCrash(Failpoint),
}

pub struct UpdateTransaction<'a, C> {
    layout: &'a InstallLayout,
    journal_store: JournalStore,
    journal: UpdateJournal,
    self_check: C,
    failpoint: Failpoint,
}

impl<'a, C: ProductSelfCheck> UpdateTransaction<'a, C> {
    pub fn new(
        layout: &'a InstallLayout,
        journal: UpdateJournal,
        self_check: C,
    ) -> Self {
        Self {
            layout,
            journal_store: JournalStore::new(layout.root()),
            journal,
            self_check,
            failpoint: Failpoint::None,
        }
    }

    pub fn with_failpoint(mut self, failpoint: Failpoint) -> Self {
        self.failpoint = failpoint;
        self
    }

    pub fn journal(&self) -> &UpdateJournal {
        &self.journal
    }

    pub fn into_journal(self) -> UpdateJournal {
        self.journal
    }

    pub fn execute(mut self) -> Result<TransactionOutcome> {
        let _mutation_lock = self.layout.acquire_mutation_lock()?;

        loop {
            match reconcile(self.layout, &self.journal)? {
                RecoveryDirective::AwaitOriginExit => {
                    self.advance(UpdatePhase::OriginExited)?;
                    if self.hit(Failpoint::AfterOriginExitedJournal) {
                        return Ok(TransactionOutcome::InjectedCrash(
                            Failpoint::AfterOriginExitedJournal,
                        ));
                    }
                }
                RecoveryDirective::RetainPrevious => {
                    let rollback = self.rollback_path()?;
                    fs::create_dir_all(
                        rollback
                            .parent()
                            .context("rollback path must have parent")?,
                    )
                    .context("create rollback parent")?;
                    fs::rename(self.layout.current(), &rollback)
                        .context("retain previous Chaptera tree")?;
                    if self.hit(Failpoint::AfterPreviousRename) {
                        return Ok(TransactionOutcome::InjectedCrash(
                            Failpoint::AfterPreviousRename,
                        ));
                    }
                }
                RecoveryDirective::RecordPreviousRetained => {
                    self.advance(UpdatePhase::PreviousRetained)?;
                    if self.hit(Failpoint::AfterPreviousRetainedJournal) {
                        return Ok(TransactionOutcome::InjectedCrash(
                            Failpoint::AfterPreviousRetainedJournal,
                        ));
                    }
                }
                RecoveryDirective::PromoteCandidate => {
                    let staging = self.staging_path()?;
                    fs::rename(&staging, self.layout.current())
                        .context("promote candidate Chaptera tree")?;
                    if self.hit(Failpoint::AfterCandidateRename) {
                        return Ok(TransactionOutcome::InjectedCrash(
                            Failpoint::AfterCandidateRename,
                        ));
                    }
                }
                RecoveryDirective::RecordCandidatePromoted => {
                    self.advance(UpdatePhase::CandidatePromotedUnconfirmed)?;
                    if self.hit(Failpoint::AfterCandidatePromotedJournal) {
                        return Ok(TransactionOutcome::InjectedCrash(
                            Failpoint::AfterCandidatePromotedJournal,
                        ));
                    }
                }
                RecoveryDirective::RunCandidateSelfCheck => {
                    match self.self_check.check(&self.layout.current())? {
                        SelfCheckResult::Pass => {
                            self.advance(UpdatePhase::CandidateConfirmed)?;
                            return Ok(TransactionOutcome::CandidateConfirmed);
                        }
                        SelfCheckResult::Fail if self.journal.rollback_compatible => {
                            self.advance(UpdatePhase::RollbackStarted)?;
                            if self.hit(Failpoint::AfterRollbackStartedJournal) {
                                return Ok(TransactionOutcome::InjectedCrash(
                                    Failpoint::AfterRollbackStartedJournal,
                                ));
                            }
                        }
                        SelfCheckResult::Fail => {
                            self.advance(UpdatePhase::RepairRequired)?;
                            return Ok(TransactionOutcome::RepairRequired(
                                "candidate self-check failed and release edge is not rollback-compatible"
                                    .into(),
                            ));
                        }
                    }
                }
                RecoveryDirective::MoveCandidateAsideAndRestorePrevious => {
                    self.move_candidate_aside()?;
                    if self.hit(Failpoint::AfterFailedCandidateRename) {
                        return Ok(TransactionOutcome::InjectedCrash(
                            Failpoint::AfterFailedCandidateRename,
                        ));
                    }
                    self.restore_previous()?;
                    if self.hit(Failpoint::AfterPreviousRestoreRename) {
                        return Ok(TransactionOutcome::InjectedCrash(
                            Failpoint::AfterPreviousRestoreRename,
                        ));
                    }
                }
                RecoveryDirective::RestorePrevious => {
                    self.restore_previous()?;
                    if self.hit(Failpoint::AfterPreviousRestoreRename) {
                        return Ok(TransactionOutcome::InjectedCrash(
                            Failpoint::AfterPreviousRestoreRename,
                        ));
                    }
                }
                RecoveryDirective::RecordPreviousRestored => {
                    self.advance(UpdatePhase::PreviousRestored)?;
                    if self.hit(Failpoint::AfterPreviousRestoredJournal) {
                        return Ok(TransactionOutcome::InjectedCrash(
                            Failpoint::AfterPreviousRestoredJournal,
                        ));
                    }
                }
                RecoveryDirective::RunPreviousSelfCheck => {
                    match self.self_check.check(&self.layout.current())? {
                        SelfCheckResult::Pass => {
                            self.advance(UpdatePhase::RollbackConfirmed)?;
                            return Ok(TransactionOutcome::PreviousRestored);
                        }
                        SelfCheckResult::Fail => {
                            self.advance(UpdatePhase::RepairRequired)?;
                            return Ok(TransactionOutcome::RepairRequired(
                                "restored predecessor failed product self-check".into(),
                            ));
                        }
                    }
                }
                RecoveryDirective::Confirmed => {
                    return Ok(TransactionOutcome::CandidateConfirmed);
                }
                RecoveryDirective::RolledBack => {
                    return Ok(TransactionOutcome::PreviousRestored);
                }
                RecoveryDirective::RepairRequired { reason } => {
                    if self.journal.phase != UpdatePhase::RepairRequired {
                        self.advance(UpdatePhase::RepairRequired)?;
                    }
                    return Ok(TransactionOutcome::RepairRequired(reason));
                }
            }
        }
    }

    fn advance(&mut self, phase: UpdatePhase) -> Result<()> {
        self.journal.advance(phase);
        self.journal_store.persist(&mut self.journal)
    }

    fn staging_path(&self) -> Result<PathBuf> {
        self.layout.staging_for(&self.journal.attempt_id)
    }

    fn rollback_path(&self) -> Result<PathBuf> {
        self.layout
            .rollback_for(&self.journal.from_version, &self.journal.attempt_id)
    }

    fn failed_candidate_path(&self) -> Result<PathBuf> {
        self.layout.failed_candidate_for(&self.journal.attempt_id)
    }

    fn move_candidate_aside(&self) -> Result<()> {
        let current = self.layout.current();
        let failed = self.failed_candidate_path()?;
        if failed.exists() {
            bail!("failed-candidate quarantine path already exists: {}", failed.display());
        }
        fs::rename(&current, &failed).context("move failed candidate aside")
    }

    fn restore_previous(&self) -> Result<()> {
        let rollback = self.rollback_path()?;
        fs::rename(&rollback, self.layout.current()).context("restore predecessor tree")
    }

    fn hit(&self, failpoint: Failpoint) -> bool {
        self.failpoint == failpoint
    }
}

pub fn resume_transaction<C: ProductSelfCheck>(
    layout: &InstallLayout,
    self_check: C,
) -> Result<Option<TransactionOutcome>> {
    let store = JournalStore::new(layout.root());
    let Some(journal) = store.load_latest()? else {
        return Ok(None);
    };
    UpdateTransaction::new(layout, journal, self_check)
        .execute()
        .map(Some)
}

pub fn prepare_payload_swap(
    layout: &InstallLayout,
    mut journal: UpdateJournal,
) -> Result<UpdateJournal> {
    let _mutation_lock = layout.acquire_mutation_lock()?;
    let store = JournalStore::new(layout.root());

    if let Some(existing) = store.load_latest()? {
        if !existing.phase.is_terminal() {
            let same_attempt = existing.product_id == journal.product_id
                && existing.architecture == journal.architecture
                && existing.channel == journal.channel
                && existing.from_version == journal.from_version
                && existing.to_version == journal.to_version
                && existing.install_layout_epoch == journal.install_layout_epoch
                && existing.update_protocol_version == journal.update_protocol_version
                && existing.candidate_tree_sha256 == journal.candidate_tree_sha256;
            if same_attempt {
                return Ok(existing);
            }
            bail!(
                "another Chaptera update attempt is active: {} {} -> {}",
                existing.attempt_id,
                existing.from_version,
                existing.to_version
            );
        }

        // Journal generation is a monotonic install-root high-water mark,
        // not a per-attempt counter. Seeding from the last terminal attempt
        // ensures a new current journal outranks its retained .prev copy.
        journal.generation = existing.generation;
    }

    let current_sha = tree_sha256(&layout.current())?;
    if current_sha != journal.previous_tree_sha256 {
        bail!("current tree digest does not match update predecessor");
    }
    let staging = layout.staging_for(&journal.attempt_id)?;
    let candidate_sha = tree_sha256(&staging)?;
    if candidate_sha != journal.candidate_tree_sha256 {
        bail!("staged candidate digest does not match authenticated candidate");
    }

    store.persist(&mut journal)?;
    Ok(journal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree_sha256;
    use std::fs;
    use tempfile::tempdir;

    struct Fixture {
        _temp: tempfile::TempDir,
        layout: InstallLayout,
        journal: UpdateJournal,
    }

    fn fixture() -> Fixture {
        let temp = tempdir().unwrap();
        let layout = InstallLayout::new(temp.path());
        fs::create_dir_all(layout.current()).unwrap();
        fs::write(layout.current().join("reader.exe"), b"old-good").unwrap();
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
        fs::write(staging.join("reader.exe"), b"new-good").unwrap();
        journal.candidate_tree_sha256 = tree_sha256(&staging).unwrap();

        Fixture {
            _temp: temp,
            layout,
            journal,
        }
    }

    fn check_expected(path: &Path) -> Result<SelfCheckResult> {
        let bytes = fs::read(path.join("reader.exe"))?;
        if bytes == b"new-good" || bytes == b"old-good" {
            Ok(SelfCheckResult::Pass)
        } else {
            Ok(SelfCheckResult::Fail)
        }
    }

    #[test]
    fn happy_path_confirms_candidate() {
        let f = fixture();
        let journal = prepare_payload_swap(&f.layout, f.journal).unwrap();
        let outcome = UpdateTransaction::new(&f.layout, journal, check_expected)
            .execute()
            .unwrap();
        assert!(matches!(outcome, TransactionOutcome::CandidateConfirmed));
        assert_eq!(
            fs::read(f.layout.current().join("reader.exe")).unwrap(),
            b"new-good"
        );
    }

    #[test]
    fn failing_candidate_rolls_back_exact_predecessor() {
        let f = fixture();
        fs::write(
            f.layout.staging_for(&f.journal.attempt_id).unwrap().join("reader.exe"),
            b"new-bad",
        )
        .unwrap();
        let mut journal = f.journal;
        journal.candidate_tree_sha256 =
            tree_sha256(&f.layout.staging_for(&journal.attempt_id).unwrap()).unwrap();
        let journal = prepare_payload_swap(&f.layout, journal).unwrap();

        let outcome = UpdateTransaction::new(&f.layout, journal, |path: &Path| {
            let bytes = fs::read(path.join("reader.exe"))?;
            Ok(if bytes == b"old-good" {
                SelfCheckResult::Pass
            } else {
                SelfCheckResult::Fail
            })
        })
        .execute()
        .unwrap();

        assert!(matches!(outcome, TransactionOutcome::PreviousRestored));
        assert_eq!(
            fs::read(f.layout.current().join("reader.exe")).unwrap(),
            b"old-good"
        );
    }

    #[test]
    fn every_destructive_failpoint_resumes_to_candidate_or_predecessor() {
        let failpoints = [
            Failpoint::AfterOriginExitedJournal,
            Failpoint::AfterPreviousRename,
            Failpoint::AfterPreviousRetainedJournal,
            Failpoint::AfterCandidateRename,
            Failpoint::AfterCandidatePromotedJournal,
        ];

        for failpoint in failpoints {
            let f = fixture();
            let journal = prepare_payload_swap(&f.layout, f.journal).unwrap();
            let first = UpdateTransaction::new(&f.layout, journal, check_expected)
                .with_failpoint(failpoint)
                .execute()
                .unwrap();
            assert!(matches!(first, TransactionOutcome::InjectedCrash(_)));

            let resumed = resume_transaction(&f.layout, check_expected)
                .unwrap()
                .unwrap();
            assert!(
                matches!(
                    resumed,
                    TransactionOutcome::CandidateConfirmed
                        | TransactionOutcome::PreviousRestored
                ),
                "unexpected resumed outcome at {failpoint:?}: {resumed:?}"
            );
        }
    }

    #[test]
    fn repeated_prepare_reuses_same_active_attempt() {
        let f = fixture();
        let first = prepare_payload_swap(&f.layout, f.journal.clone()).unwrap();

        let mut retry = f.journal;
        retry.attempt_id = uuid::Uuid::now_v7().to_string();
        let second = prepare_payload_swap(&f.layout, retry).unwrap();

        assert_eq!(second.attempt_id, first.attempt_id);
        assert_eq!(second.generation, first.generation);
    }

    #[test]
    fn competing_prepare_is_rejected_while_attempt_is_active() {
        let f = fixture();
        let _first = prepare_payload_swap(&f.layout, f.journal.clone()).unwrap();

        let mut competing = f.journal;
        competing.attempt_id = uuid::Uuid::now_v7().to_string();
        competing.to_version = "0.3.0".into();
        assert!(prepare_payload_swap(&f.layout, competing).is_err());
    }

    #[test]
    fn journal_generation_is_monotonic_across_attempts() {
        let f = fixture();
        let first = prepare_payload_swap(&f.layout, f.journal).unwrap();
        let outcome = UpdateTransaction::new(&f.layout, first.clone(), check_expected)
            .execute()
            .unwrap();
        assert!(matches!(outcome, TransactionOutcome::CandidateConfirmed));

        let store = JournalStore::new(f.layout.root());
        let terminal = store.load_latest().unwrap().unwrap();
        assert!(terminal.phase.is_terminal());

        // Make the confirmed candidate the predecessor for a later update.
        let previous = tree_sha256(&f.layout.current()).unwrap();
        let mut next = UpdateJournal::new_payload_swap(
            "chaptera.reader",
            "windows-x86_64",
            "stable",
            "0.2.0",
            "0.3.0",
            1,
            1,
            previous,
            "",
            true,
            "reader-state-v1",
        );
        let staging = f.layout.staging_for(&next.attempt_id).unwrap();
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("reader.exe"), b"newer-good").unwrap();
        next.candidate_tree_sha256 = tree_sha256(&staging).unwrap();

        let prepared = prepare_payload_swap(&f.layout, next).unwrap();
        assert!(prepared.generation > terminal.generation);
        assert_eq!(
            store.load_latest().unwrap().unwrap().attempt_id,
            prepared.attempt_id
        );
    }

    #[test]
    fn rollback_failpoints_resume_to_previous() {
        let failpoints = [
            Failpoint::AfterRollbackStartedJournal,
            Failpoint::AfterFailedCandidateRename,
            Failpoint::AfterPreviousRestoreRename,
            Failpoint::AfterPreviousRestoredJournal,
        ];

        for failpoint in failpoints {
            let f = fixture();
            let staging = f.layout.staging_for(&f.journal.attempt_id).unwrap();
            fs::write(staging.join("reader.exe"), b"new-bad").unwrap();
            let mut journal = f.journal;
            journal.candidate_tree_sha256 = tree_sha256(&staging).unwrap();
            let journal = prepare_payload_swap(&f.layout, journal).unwrap();

            let check = |path: &Path| -> Result<SelfCheckResult> {
                let bytes = fs::read(path.join("reader.exe"))?;
                Ok(if bytes == b"old-good" {
                    SelfCheckResult::Pass
                } else {
                    SelfCheckResult::Fail
                })
            };

            let first = UpdateTransaction::new(&f.layout, journal, check)
                .with_failpoint(failpoint)
                .execute()
                .unwrap();
            assert!(matches!(first, TransactionOutcome::InjectedCrash(_)));

            let resumed = resume_transaction(&f.layout, check).unwrap().unwrap();
            assert!(matches!(resumed, TransactionOutcome::PreviousRestored));
            assert_eq!(
                fs::read(f.layout.current().join("reader.exe")).unwrap(),
                b"old-good"
            );
        }
    }
}
