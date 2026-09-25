use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::env;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const JOB_VERSION: &str = "chaptera.rescue-worker-job.v1";
const EVENT_VERSION: &str = "chaptera.rescue-worker-event.v1";
const RESULT_VERSION: &str = "chaptera.rescue-worker-result.v1";
const PRODUCER_RECEIPT_VERSION: &str = "chaptera.rescue-recovery-producer-receipt.v1";
const WORKER_ID: &str = "chaptera-recovery-worker/v0";
const FENCE_MODE_ENV: &str = "CHAPTERA_RECOVERY_FENCE_MODE";
const WINDOWS_FENCE_MODE: &str = "windows_job_object_v1";
const PRODUCER_RECEIPT_NAME: &str = "producer-receipt.json";

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

#[derive(Debug, Clone)]
struct LaunchConfig {
    program: PathBuf,
    args: Vec<String>,
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
    id: String,
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
    external_recovery_executor_supported: bool,
    external_executor_requires_windows_job_object: bool,
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

fn parse_launch_config(args: &[String]) -> Result<Option<LaunchConfig>, String> {
    if args.is_empty() {
        return Ok(None);
    }

    let mut program: Option<PathBuf> = None;
    let mut executor_args = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--executor" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--executor requires an exact executable path".to_owned())?;
                if program.is_some() {
                    return Err("--executor may be supplied only once".to_owned());
                }
                program = Some(PathBuf::from(value));
            }
            "--executor-arg" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--executor-arg requires a value".to_owned())?;
                executor_args.push(value.clone());
            }
            other => return Err(format!("unsupported worker argument: {other}")),
        }
        index += 1;
    }

    let Some(program) = program else {
        return Err("--executor-arg requires --executor".to_owned());
    };
    let metadata = fs::metadata(&program)
        .map_err(|error| format!("stat executor {}: {error}", program.display()))?;
    if !metadata.is_file() {
        return Err("--executor must name an existing file".to_owned());
    }

    Ok(Some(LaunchConfig {
        program,
        args: executor_args,
    }))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
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

fn executor_identity(config: &LaunchConfig) -> Result<String, String> {
    let mut digest = Sha256::new();
    digest.update(b"chaptera.external-recovery-executor.v1\0");
    digest.update(sha256_file(&config.program)?.as_bytes());

    for arg in &config.args {
        let path = Path::new(arg);
        if path.is_file() {
            digest.update(b"file\0");
            digest.update(sha256_file(path)?.as_bytes());
        } else {
            digest.update(b"literal\0");
            digest.update(arg.as_bytes());
        }
        digest.update(b"\0");
    }

    Ok(format!("external-sha256:{:x}", digest.finalize()))
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

fn emit_result(
    job: &Job,
    source_sha: &str,
    status: &'static str,
    executor: ExecutorState,
    producer_receipt: Option<ProducerReceipt>,
    code: &'static str,
    message: &'static str,
) -> Result<(), String> {
    emit(&ResultEnvelope {
        protocol_version: RESULT_VERSION,
        job_id: &job.job_id,
        status,
        source_sha256: source_sha,
        source_unchanged: true,
        executor,
        limits: &job.limits,
        producer_receipt,
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
        Some("No recovery executor is configured for this worker launch."),
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
    emit_result(
        job,
        source_sha,
        "executor_unavailable",
        ExecutorState {
            id: "unconfigured".to_owned(),
            available: false,
        },
        None,
        "executor_unavailable",
        "No authorized recovery executor is configured; no recovery success is claimed.",
    )?;
    Ok(3)
}

#[cfg(windows)]
fn process_is_in_job() -> Result<bool, String> {
    use std::ffi::c_void;
    use std::ptr;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn IsProcessInJob(
            process_handle: *mut c_void,
            job_handle: *mut c_void,
            result: *mut i32,
        ) -> i32;
    }

    let mut in_job = 0_i32;
    let ok = unsafe { IsProcessInJob(GetCurrentProcess(), ptr::null_mut(), &mut in_job) };
    if ok == 0 {
        return Err("IsProcessInJob failed".to_owned());
    }
    Ok(in_job != 0)
}

#[cfg(not(windows))]
fn process_is_in_job() -> Result<bool, String> {
    Ok(false)
}

fn require_executor_fence(job: &Job, executor_id: &str) -> Result<Option<i32>, String> {
    let declared = env::var(FENCE_MODE_ENV).unwrap_or_default();
    if declared == WINDOWS_FENCE_MODE && process_is_in_job()? {
        return Ok(None);
    }

    emit_event(
        &job.job_id,
        "phase",
        "executor",
        "failed",
        Some("fence_required"),
        Some("External recovery executor requires an admitted Windows Job Object launch."),
    )?;
    emit_event(
        &job.job_id,
        "finished",
        "finished",
        "failed",
        Some("fence_required"),
        Some("Executor was not started because the worker fence was not proven."),
    )?;
    emit_result(
        job,
        &job.source.sha256,
        "failed",
        ExecutorState {
            id: executor_id.to_owned(),
            available: true,
        },
        None,
        "fence_required",
        "Authorized executor was configured but not started outside the required Windows Job Object fence.",
    )?;
    Ok(Some(4))
}

fn prepare_job_directory(job: &Job, source: &Path) -> Result<PathBuf, String> {
    let directory = PathBuf::from(&job.output.job_directory);
    if directory.exists() {
        if !directory.is_dir() {
            return Err("output.job_directory exists but is not a directory".to_owned());
        }
        if fs::read_dir(&directory)
            .map_err(|error| format!("read job directory: {error}"))?
            .next()
            .is_some()
        {
            return Err("output.job_directory must be empty before executor launch".to_owned());
        }
    } else {
        fs::create_dir_all(&directory)
            .map_err(|error| format!("create job directory {}: {error}", directory.display()))?;
    }

    let canonical_source = fs::canonicalize(source)
        .map_err(|error| format!("canonicalize source {}: {error}", source.display()))?;
    let canonical_directory = fs::canonicalize(&directory).map_err(|error| {
        format!(
            "canonicalize job directory {}: {error}",
            directory.display()
        )
    })?;
    if canonical_source.starts_with(&canonical_directory) {
        return Err("source must not live inside the executor output directory".to_owned());
    }
    Ok(canonical_directory)
}

fn directory_usage(root: &Path) -> Result<(u64, u64), String> {
    fn visit(path: &Path, bytes: &mut u64, count: &mut u64) -> Result<(), String> {
        for entry in
            fs::read_dir(path).map_err(|error| format!("read {}: {error}", path.display()))?
        {
            let entry = entry.map_err(|error| format!("read directory entry: {error}"))?;
            let child = entry.path();
            let metadata = fs::symlink_metadata(&child)
                .map_err(|error| format!("stat {}: {error}", child.display()))?;
            if metadata.file_type().is_symlink() {
                return Err("executor output must not contain symbolic links".to_owned());
            }
            if metadata.is_dir() {
                visit(&child, bytes, count)?;
            } else if metadata.is_file() {
                *count = count.saturating_add(1);
                *bytes = bytes.saturating_add(metadata.len());
            }
        }
        Ok(())
    }

    let mut bytes = 0_u64;
    let mut count = 0_u64;
    visit(root, &mut bytes, &mut count)?;
    Ok((bytes, count))
}

fn validate_minimal_producer_receipt(path: &Path, source_sha: &str) -> Result<(), String> {
    let value: Value = serde_json::from_slice(
        &fs::read(path).map_err(|error| format!("read producer receipt: {error}"))?,
    )
    .map_err(|error| format!("parse producer receipt: {error}"))?;

    let object = value
        .as_object()
        .ok_or_else(|| "producer receipt must be a JSON object".to_owned())?;
    if object.get("receipt_version").and_then(Value::as_str) != Some(PRODUCER_RECEIPT_VERSION) {
        return Err("producer receipt has the wrong receipt_version".to_owned());
    }
    if value
        .pointer("/fixture/source_sha256")
        .and_then(Value::as_str)
        != Some(source_sha)
    {
        return Err("producer receipt source identity does not match the admitted job".to_owned());
    }

    for pointer in [
        "/source_immutability/before_sha256",
        "/source_immutability/after_sha256",
    ] {
        if value.pointer(pointer).and_then(Value::as_str) != Some(source_sha) {
            return Err(
                "producer receipt source immutability hashes do not match the job".to_owned(),
            );
        }
    }
    if value
        .pointer("/source_immutability/unchanged")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err("producer receipt must claim source_immutability.unchanged=true".to_owned());
    }
    if value
        .pointer("/recovery/fabricated_bytes")
        .and_then(Value::as_u64)
        != Some(0)
        || value
            .pointer("/recovery/silent_drops")
            .and_then(Value::as_u64)
            != Some(0)
    {
        return Err(
            "producer receipt must keep fabricated_bytes and silent_drops at zero".to_owned(),
        );
    }
    for pointer in [
        "/privacy/raw_pub_bytes",
        "/privacy/document_text",
        "/privacy/local_paths",
        "/privacy/customer_identity",
        "/privacy/credentials",
    ] {
        if value.pointer(pointer).and_then(Value::as_bool) != Some(false) {
            return Err("producer receipt privacy boundary is not source-free".to_owned());
        }
    }
    Ok(())
}

fn external_executor(job: &Job, source_sha: &str, config: &LaunchConfig) -> Result<i32, String> {
    let executor_id = executor_identity(config)?;
    if let Some(code) = require_executor_fence(job, &executor_id)? {
        return Ok(code);
    }

    let source = PathBuf::from(&job.source.path);
    let job_directory = prepare_job_directory(job, &source)?;
    let receipt_path = job_directory.join(PRODUCER_RECEIPT_NAME);

    emit_event(
        &job.job_id,
        "phase",
        "executor",
        "running",
        None,
        Some(
            "Launching the configured authorized recovery executor under the inherited worker fence.",
        ),
    )?;

    let status = Command::new(&config.program)
        .args(&config.args)
        .current_dir(&job_directory)
        .env("CHAPTERA_RECOVERY_JOB_ID", &job.job_id)
        .env("CHAPTERA_RECOVERY_SOURCE", &source)
        .env("CHAPTERA_RECOVERY_SOURCE_SHA256", source_sha)
        .env("CHAPTERA_RECOVERY_JOB_DIRECTORY", &job_directory)
        .env("CHAPTERA_RECOVERY_PRODUCER_RECEIPT", &receipt_path)
        .env(
            "CHAPTERA_RECOVERY_NATIVE_PUB_DELIVERY_ALLOWED",
            if job.policy.native_pub_delivery_allowed {
                "true"
            } else {
                "false"
            },
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("spawn authorized recovery executor: {error}"))?;

    let after = sha256_file(&source)?;
    if after != source_sha {
        return Err("source identity changed during external recovery execution".to_owned());
    }

    let (output_bytes, artifact_count) = directory_usage(&job_directory)?;
    if output_bytes > job.limits.output_bytes || artifact_count > job.limits.artifact_count {
        emit_event(
            &job.job_id,
            "finished",
            "finished",
            "resource_limited",
            Some("output_limit"),
            Some("Executor output exceeded the admitted byte/artifact ceiling."),
        )?;
        emit_result(
            job,
            source_sha,
            "resource_limited",
            ExecutorState {
                id: executor_id,
                available: true,
            },
            None,
            "output_limit",
            "Executor output was rejected because it exceeded the admitted output ceiling.",
        )?;
        return Ok(5);
    }

    if !status.success() {
        emit_event(
            &job.job_id,
            "finished",
            "finished",
            "failed",
            Some("executor_failed"),
            Some("Authorized recovery executor exited unsuccessfully."),
        )?;
        emit_result(
            job,
            source_sha,
            "failed",
            ExecutorState {
                id: executor_id,
                available: true,
            },
            None,
            "executor_failed",
            "Authorized recovery executor exited unsuccessfully; no recovery success is claimed.",
        )?;
        return Ok(6);
    }

    emit_event(
        &job.job_id,
        "phase",
        "result_validation",
        "running",
        None,
        Some("Validating the producer receipt transport/privacy/source boundary."),
    )?;

    if !receipt_path.is_file() {
        emit_event(
            &job.job_id,
            "finished",
            "finished",
            "failed",
            Some("producer_receipt_missing"),
            Some("Executor completed without the required producer receipt."),
        )?;
        emit_result(
            job,
            source_sha,
            "failed",
            ExecutorState {
                id: executor_id,
                available: true,
            },
            None,
            "producer_receipt_missing",
            "Executor completed without the fixed producer-receipt.json contract.",
        )?;
        return Ok(7);
    }

    if let Err(error) = validate_minimal_producer_receipt(&receipt_path, source_sha) {
        eprintln!("producer receipt admission failed: {error}");
        emit_event(
            &job.job_id,
            "finished",
            "finished",
            "failed",
            Some("producer_receipt_invalid"),
            Some("Executor producer receipt failed the worker admission boundary."),
        )?;
        emit_result(
            job,
            source_sha,
            "failed",
            ExecutorState {
                id: executor_id,
                available: true,
            },
            None,
            "producer_receipt_invalid",
            "Producer receipt failed source/privacy/zero-fabrication admission; downstream consumer was not invoked.",
        )?;
        return Ok(8);
    }

    let receipt_sha256 = sha256_file(&receipt_path)?;
    emit_event(
        &job.job_id,
        "phase",
        "result_validation",
        "succeeded",
        None,
        Some("Producer receipt passed worker transport admission."),
    )?;
    emit_event(
        &job.job_id,
        "finished",
        "finished",
        "succeeded",
        None,
        Some("Executor completed and emitted a producer receipt for downstream Rescue validation."),
    )?;
    emit_result(
        job,
        source_sha,
        "succeeded",
        ExecutorState {
            id: executor_id,
            available: true,
        },
        Some(ProducerReceipt {
            relative_path: PRODUCER_RECEIPT_NAME.to_owned(),
            sha256: receipt_sha256,
        }),
        "ok",
        "Execution succeeded; recovery/product outcome remains authoritative only after the Rescue producer-receipt consumer validates the receipt.",
    )?;
    Ok(0)
}

fn run_job(job: Job, launch: Option<&LaunchConfig>) -> Result<i32, String> {
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
        emit_result(
            &job,
            &job.source.sha256,
            "failed",
            ExecutorState {
                id: "not_started".to_owned(),
                available: launch.is_some(),
            },
            None,
            "source_identity_mismatch",
            "Recovery executor was not started.",
        )?;
        return Ok(2);
    }

    match launch {
        Some(config) => external_executor(&job, &before, config),
        None => executor_unavailable(&job, &before),
    }
}

fn self_check() -> SelfCheck {
    SelfCheck {
        protocol_version: "chaptera.recovery-worker-self-check.v1",
        worker_id: WORKER_ID,
        executable: "chaptera-recovery-worker.exe",
        short_lived_process: true,
        source_mutation_allowed: false,
        embedded_recovery_executor: false,
        external_recovery_executor_supported: true,
        external_executor_requires_windows_job_object: true,
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

    let launch = match parse_launch_config(&args[1..]) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("worker launch configuration failure: {error}");
            std::process::exit(2);
        }
    };

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
    match run_job(job, launch.as_ref()) {
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
        assert!(check.external_recovery_executor_supported);
        assert!(check.external_executor_requires_windows_job_object);
        assert!(!check.source_mutation_allowed);
        assert!(!check.progress_percent_claimed);
        assert_eq!(check.executable, "chaptera-recovery-worker.exe");
    }

    #[test]
    fn executor_args_without_executor_are_rejected() {
        let args = vec!["--executor-arg".to_owned(), "x".to_owned()];
        assert!(parse_launch_config(&args).is_err());
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
