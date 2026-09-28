use chaptera_update_engine::{RecoveryOutcome, UpdateEngine, UpdatePhase};
use chaptera_update_orchestrator::InstallLock;
use sha2::Digest;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn usage() -> ! {
    eprintln!("usage:");
    eprintln!("  chaptera-update-recovery-probe prepare-fault <install-root> <candidate-source> <transaction-id> <candidate-version> <updater-relative-path> <prepared|previous-retained|candidate-activated>");
    eprintln!("  chaptera-update-recovery-probe inspect <install-root>");
    eprintln!("  chaptera-update-recovery-probe recover <install-root>");
    std::process::exit(2);
}

fn phase_name(phase: UpdatePhase) -> &'static str {
    match phase {
        UpdatePhase::Preparing => "preparing",
        UpdatePhase::Prepared => "prepared",
        UpdatePhase::PreviousRetained => "previous_retained",
        UpdatePhase::CandidateActivated => "candidate_activated",
        UpdatePhase::CandidateConfirmed => "candidate_confirmed",
        UpdatePhase::RolledBack => "rolled_back",
    }
}

fn recovery_name(outcome: RecoveryOutcome) -> &'static str {
    match outcome {
        RecoveryOutcome::NothingToDo => "nothing_to_do",
        RecoveryOutcome::PreparedTransactionAborted => "prepared_transaction_aborted",
        RecoveryOutcome::UnconfirmedCandidateRolledBack => "unconfirmed_candidate_rolled_back",
        RecoveryOutcome::ConfirmedCandidateRetained => "confirmed_candidate_retained",
        RecoveryOutcome::RolledBackTransactionFinalized => "rolled_back_transaction_finalized",
    }
}

fn tree_fingerprint(root: &Path) -> Result<String, String> {
    let mut entries = Vec::<(PathBuf, u64, String)>::new();

    fn walk(base: &Path, dir: &Path, out: &mut Vec<(PathBuf, u64, String)>) -> Result<(), String> {
        let mut children = fs::read_dir(dir)
            .map_err(|e| format!("read_dir {}: {e}", dir.display()))?
            .map(|e| e.map_err(|err| err.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        children.sort_by_key(|entry| entry.file_name());

        for entry in children {
            let path = entry.path();
            let metadata = entry
                .metadata()
                .map_err(|e| format!("metadata {}: {e}", path.display()))?;
            if metadata.is_dir() {
                walk(base, &path, out)?;
            } else if metadata.is_file() {
                let rel = path
                    .strip_prefix(base)
                    .map_err(|e| format!("strip_prefix {}: {e}", path.display()))?
                    .to_path_buf();
                let bytes = fs::read(&path)
                    .map_err(|e| format!("read {}: {e}", path.display()))?;
                let digest = format!("{:x}", sha2::Sha256::digest(&bytes));
                out.push((rel, metadata.len(), digest));
            }
        }
        Ok(())
    }

    if !root.is_dir() {
        return Err(format!("tree missing: {}", root.display()));
    }
    walk(root, root, &mut entries)?;
    let mut canonical = Vec::<u8>::new();
    for (path, len, digest) in entries {
        canonical.extend_from_slice(path.to_string_lossy().replace('\\', "/").as_bytes());
        canonical.push(b'|');
        canonical.extend_from_slice(len.to_string().as_bytes());
        canonical.push(b'|');
        canonical.extend_from_slice(digest.as_bytes());
        canonical.push(b'\n');
    }
    Ok(format!("{:x}", sha2::Sha256::digest(&canonical)))
}

fn inspect(engine: &UpdateEngine) -> Result<(), String> {
    match engine.read_journal().map_err(|e| e.to_string())? {
        Some(journal) => {
            println!("active=true");
            println!("transaction_id={}", journal.transaction_id);
            println!("candidate_version={}", journal.candidate_version);
            println!("phase={}", phase_name(journal.phase));
        }
        None => {
            println!("active=false");
        }
    }

    let current = engine.current_dir();
    println!("current_exists={}", current.is_dir());
    if current.is_dir() {
        println!("current_tree_sha256={}", tree_fingerprint(&current)?);
    }
    Ok(())
}

fn main() {

    let args = env::args().collect::<Vec<_>>();
    if args.len() < 3 {
        usage();
    }

    let result = match args[1].as_str() {
        "inspect" if args.len() == 3 => {
            let engine = UpdateEngine::new(&args[2]);
            inspect(&engine)
        }
        "recover" if args.len() == 3 => {
            let root = PathBuf::from(&args[2]);
            let _lock = InstallLock::try_acquire(&root).map_err(|e| e.to_string());
            match _lock {
                Ok(_guard) => {
                    let engine = UpdateEngine::new(&root);
                    let before = engine.read_journal().map_err(|e| e.to_string());
                    match before {
                        Ok(before) => {
                            if let Some(journal) = before.as_ref() {
                                println!("before_transaction_id={}", journal.transaction_id);
                                println!("before_phase={}", phase_name(journal.phase));
                            } else {
                                println!("before_transaction_id=");
                                println!("before_phase=none");
                            }
                            match engine.recover().map_err(|e| e.to_string()) {
                                Ok(outcome) => {
                                    println!("recovery_outcome={}", recovery_name(outcome));
                                    match engine.cleanup_orphaned_transactions().map_err(|e| e.to_string()) {
                                        Ok(removed) => {
                                            println!("orphan_transactions_removed={removed}");
                                            inspect(&engine)
                                        }
                                        Err(e) => Err(e),
                                    }
                                }
                                Err(e) => Err(e),
                            }
                        }
                        Err(e) => Err(e),
                    }
                }
                Err(e) => Err(e),
            }
        }
        "prepare-fault" if args.len() == 8 => {
            let root = PathBuf::from(&args[2]);
            let candidate = PathBuf::from(&args[3]);
            let transaction_id = &args[4];
            let candidate_version = &args[5];
            let updater_relative = PathBuf::from(&args[6]);
            let target_phase = &args[7];

            match InstallLock::try_acquire(&root).map_err(|e| e.to_string()) {
                Ok(_guard) => {
                    let engine = UpdateEngine::new(&root);
                    if let Err(e) = engine.recover() {
                        Err(format!("startup recovery failed: {e}"))
                    } else if let Err(e) = engine.cleanup_orphaned_transactions() {
                        Err(format!("startup cleanup failed: {e}"))
                    } else {
                        match engine.begin_verified_candidate(
                            transaction_id,
                            candidate_version,
                            &candidate,
                            &updater_relative,
                        ) {
                            Err(e) => Err(e.to_string()),
                            Ok(_) => {
                                let step_result = match target_phase.as_str() {
                                    "prepared" => Ok(()),
                                    "previous-retained" => engine.retain_previous().map_err(|e| e.to_string()),
                                    "candidate-activated" => {
                                        engine.retain_previous().map_err(|e| e.to_string())
                                            .and_then(|_| engine.activate_candidate().map_err(|e| e.to_string()))
                                    }
                                    _ => Err(format!("unsupported target phase: {target_phase}")),
                                };

                                match step_result {
                                    Ok(()) => {
                                        let journal = engine
                                            .read_journal()
                                            .map_err(|e| e.to_string())?
                                            .ok_or_else(|| "active journal missing after fault preparation".to_string())?;
                                        println!("prepared_fault=true");
                                        println!("transaction_id={}", journal.transaction_id);
                                        println!("candidate_version={}", journal.candidate_version);
                                        println!("phase={}", phase_name(journal.phase));
                                        println!("current_exists={}", engine.current_dir().is_dir());
                                        if engine.current_dir().is_dir() {
                                            println!("current_tree_sha256={}", tree_fingerprint(&engine.current_dir())?);
                                        }
                                        Ok(())
                                    }
                                    Err(e) => Err(e),
                                }
                            }
                        }
                    }
                }
                Err(e) => Err(e),
            }
        }
        _ => usage(),
    };

    if let Err(error) = result {
        eprintln!("error={error}");
        std::process::exit(1);
    }
}
