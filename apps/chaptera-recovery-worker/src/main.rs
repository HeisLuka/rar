use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const JOB_VERSION: &str = "chaptera.rescue-worker-job.v1";
const EVENT_VERSION: &str = "chaptera.rescue-worker-event.v1";
const RESULT_VERSION: &str = "chaptera.rescue-worker-result.v1";
const WORKER_ID: &str = "chaptera-recovery-worker/v0";

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Job {
    protocol_version: String,
    job_id: String,
    source: Source,
    operation: String,
    output: Output,
    limits: Limits,
    policy: Policy,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Source {
    path: String,
    sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Output {
    job_directory: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Limits {
    wall_time_ms: u64,
    cpu_time_ms: u64,
    memory_bytes: u64,
    output_bytes: u64,
    artifact_count: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Policy {
    source_mutation_allowed: bool,
    native_pub_delivery_allowed: bool,
}

#[derive(Debug, Serialize)]
struct Event<'a> {
    protocol_version: &'static str,
    job_id: &'a str,
    event: &'static str,
    phase: &'static str,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct ExecutorState {
    id: &'static str,
    available: bool,
}

#[derive(Debug, Serialize)]
struct ResultEnvelope<'a> {
    protocol_version: &'static str,
    job_id: &'a str,
    status: &'static str,
    source_sha256: &'a str,
    source_unchanged: bool,
    executor: ExecutorState,
    limits: &'a Limits,
    producer_receipt: Option<ProducerReceipt>,
    code: &'static str,
    message: &'static str,
}

#[derive(Debug, Serialize)]
struct ProducerReceipt {
    relative_path: String,
    sha256: String,
}

#[derive(Debug, Serialize)]
struct SelfCheck {
    protocol_version: &'static str,
    worker_id: &'static str,
    executable: &'static str,
    short_lived_process: bool,
    source_mutation_allowed: bool,
    embedded_recovery_executor: bool,
    job_protocol: &'static str,
    event_protocol: &'static str,
    result_protocol: &'static str,
    progress_percent_claimed: bool,
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_job_id(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }
    value.bytes().enumerate().all(|(index, byte)| {
        if matches!(index, 8 | 13 | 18 | 23) {
            byte == b'-'
        } else {
            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
        }
    })
}

fn validate_job(job: &Job) -> Result<(), String> {
    if job.protocol_version != JOB_VERSION {
        return Err(format!(
            "unsupported protocol_version: {}",
            job.protocol_version
        ));
    }
    if !valid_job_id(&job.job_id) {
        return Err("job_id must be canonical lowercase UUID text".to_owned());
    }
    if job.operation != "bounded_recovery" {
        return Err("operation must be bounded_recovery".to_owned());
    }
    if !is_lower_sha256(&job.source.sha256) {
        return Err("source.sha256 must be lowercase SHA-256".to_owned());
    }
    if job.source.path.trim().is_empty() || job.output.job_directory.trim().is_empty() {
        return Err("source.path and output.job_directory are required".to_owned());
    }
    if job.policy.source_mutation_allowed {
        return Err("source mutation is forbidden".to_owned());
    }
    if job.limits.wall_time_ms < 1000
        || job.limits.cpu_time_ms < 1000
        || job.limits.memory_bytes < 16 * 1024 * 1024
    {
        return Err("worker resource limits are below the protocol minimum".to_owned());
    }
    if job.limits.artifact_count > 100_000 {
        return Err("artifact_count limit exceeds the protocol maximum".to_owned());
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn emit<T: Serialize>(value: &T) -> Result<(), String> {
    let line = serde_json::to_string(value).map_err(|error| format!("serialize JSONL: {error}"))?;
    println!("{line}");
    Ok(())
}

fn emit_event(
    job_id: &str,
    event: &'static str,
    phase: &'static str,
    status: &'static str,
    code: Option<&'static str>,
    message: Option<&'static str>,
) -> Result<(), String> {
    emit(&Event {
        protocol_version: EVENT_VERSION,
        job_id,
        event,
        phase,
        status,
        code,
        message,
    })
}

fn executor_unavailable(job: &Job, source_sha: &str) -> Result<i32, String> {
    emit_event(
        &job.job_id,
        "phase",
        "executor",
        "executor_unavailable",
        Some("executor_unavailable"),
        Some("No recovery executor is connected to the public worker shell."),
    )?;

    let after = sha256_file(Path::new(&job.source.path))?;
    if after != source_sha {
        return Err("source identity changed while worker was running".to_owned());
    }

    emit_event(
        &job.job_id,
        "finished",
        "finished",
        "executor_unavailable",
        Some("executor_unavailable"),
        Some("Worker stopped fail-closed without producing recovery artifacts."),
    )?;
    emit(&ResultEnvelope {
        protocol_version: RESULT_VERSION,
        job_id: &job.job_id,
        status: "executor_unavailable",
        source_sha256: source_sha,
        source_unchanged: true,
        executor: ExecutorState {
            id: WORKER_ID,
            available: false,
        },
        limits: &job.limits,
        producer_receipt: None,
        code: "executor_unavailable",
        message: "No authorized recovery executor is connected; no recovery success is claimed.",
    })?;
    Ok(3)
}

fn run_job(job: Job) -> Result<i32, String> {
    validate_job(&job)?;
    emit_event(
        &job.job_id,
        "started",
        "admission",
        "running",
        None,
        Some("Job admitted by the public worker protocol shell."),
    )?;

    emit_event(
        &job.job_id,
        "phase",
        "source_verification",
        "running",
        None,
        Some("Verifying immutable source identity."),
    )?;
    let source = PathBuf::from(&job.source.path);
    let before = sha256_file(&source)?;
    if before != job.source.sha256 {
        emit_event(
            &job.job_id,
            "finished",
            "finished",
            "failed",
            Some("source_identity_mismatch"),
            Some("Selected source bytes do not match the admitted source SHA-256."),
        )?;
        emit(&ResultEnvelope {
            protocol_version: RESULT_VERSION,
            job_id: &job.job_id,
            status: "failed",
            source_sha256: &job.source.sha256,
            source_unchanged: true,
            executor: ExecutorState {
                id: WORKER_ID,
                available: false,
            },
            limits: &job.limits,
            producer_receipt: None,
            code: "source_identity_mismatch",
            message: "Recovery executor was not started.",
        })?;
        return Ok(2);
    }

    executor_unavailable(&job, &before)
}

fn self_check() -> SelfCheck {
    SelfCheck {
        protocol_version: "chaptera.recovery-worker-self-check.v1",
        worker_id: WORKER_ID,
        executable: "chaptera-recovery-worker.exe",
        short_lived_process: true,
        source_mutation_allowed: false,
        embedded_recovery_executor: false,
        job_protocol: JOB_VERSION,
        event_protocol: EVENT_VERSION,
        result_protocol: RESULT_VERSION,
        progress_percent_claimed: false,
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|arg| arg == "--self-check") {
        match serde_json::to_string_pretty(&self_check()) {
            Ok(value) => {
                println!("{value}");
                return;
            }
            Err(error) => {
                eprintln!("serialize self-check: {error}");
                std::process::exit(2);
            }
        }
    }

    let mut input = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut input) {
        eprintln!("read worker job: {error}");
        std::process::exit(2);
    }
    let job: Job = match serde_json::from_str(&input) {
        Ok(job) => job,
        Err(error) => {
            eprintln!("parse worker job: {error}");
            std::process::exit(2);
        }
    };
    match run_job(job) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("worker protocol failure: {error}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn job(path: &Path, sha: &str) -> Job {
        Job {
            protocol_version: JOB_VERSION.to_owned(),
            job_id: "11111111-1111-4111-8111-111111111111".to_owned(),
            source: Source {
                path: path.display().to_string(),
                sha256: sha.to_owned(),
            },
            operation: "bounded_recovery".to_owned(),
            output: Output {
                job_directory: "job-output".to_owned(),
            },
            limits: Limits {
                wall_time_ms: 60_000,
                cpu_time_ms: 30_000,
                memory_bytes: 256 * 1024 * 1024,
                output_bytes: 64 * 1024 * 1024,
                artifact_count: 1000,
            },
            policy: Policy {
                source_mutation_allowed: false,
                native_pub_delivery_allowed: false,
            },
        }
    }

    fn temp_source(name: &str) -> PathBuf {
        let path = env::temp_dir().join(format!(
            "chaptera-worker-{name}-{}-{}.pub",
            std::process::id(),
            env::var("GITHUB_RUN_ID").unwrap_or_else(|_| "local".to_owned())
        ));
        fs::write(&path, b"synthetic worker protocol source").expect("write source");
        path
    }

    #[test]
    fn canonical_job_admission_passes() {
        let path = temp_source("admission");
        let sha = sha256_file(&path).expect("hash source");
        validate_job(&job(&path, &sha)).expect("valid job");
        fs::remove_file(path).ok();
    }

    #[test]
    fn mutation_permission_is_rejected() {
        let path = temp_source("mutation");
        let sha = sha256_file(&path).expect("hash source");
        let mut value = job(&path, &sha);
        value.policy.source_mutation_allowed = true;
        assert!(validate_job(&value).is_err());
        fs::remove_file(path).ok();
    }

    #[test]
    fn public_shell_has_no_embedded_recovery_executor() {
        let check = self_check();
        assert!(!check.embedded_recovery_executor);
        assert!(!check.source_mutation_allowed);
        assert!(!check.progress_percent_claimed);
        assert_eq!(check.executable, "chaptera-recovery-worker.exe");
    }

    #[test]
    fn hashing_does_not_mutate_source() {
        let path = temp_source("hash");
        let before = fs::read(&path).expect("read before");
        let _ = sha256_file(&path).expect("hash");
        let after = fs::read(&path).expect("read after");
        assert_eq!(before, after);
        fs::remove_file(path).ok();
    }
}
