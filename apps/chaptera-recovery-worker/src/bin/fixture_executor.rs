use serde_json::json;
use sha2::{Digest, Sha256};
use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

const HEALTHY_CONTROL_SHA256: &str =
    "3ab75a6a9196e0a51fc9b0aa759459501c71d313030c06652aabffbae0a2ab09";

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

fn required_env(name: &str) -> Result<String, String> {
    env::var(name).map_err(|_| format!("missing {name}"))
}

fn run() -> Result<(), String> {
    // This binary is deliberately not a recovery implementation. It exists
    // only to prove the external-executor process boundary on a healthy,
    // public Apache POI control. A real damaged-file run must use the
    // authorized local producer from RESCUE-LOCAL-PRODUCER-01.
    if !process_is_in_job()? {
        return Err("fixture executor is not contained by a Windows Job Object".to_owned());
    }

    let source = PathBuf::from(required_env("CHAPTERA_RECOVERY_SOURCE")?);
    let admitted_sha = required_env("CHAPTERA_RECOVERY_SOURCE_SHA256")?;
    let job_directory = PathBuf::from(required_env("CHAPTERA_RECOVERY_JOB_DIRECTORY")?);
    let receipt_path = PathBuf::from(required_env("CHAPTERA_RECOVERY_PRODUCER_RECEIPT")?);

    if admitted_sha != HEALTHY_CONTROL_SHA256 {
        return Err("fixture executor accepts only the pinned healthy control".to_owned());
    }
    let before = sha256_file(&source)?;
    if before != HEALTHY_CONTROL_SHA256 {
        return Err("healthy control bytes do not match the pinned SHA-256".to_owned());
    }

    let canonical_job = fs::canonicalize(&job_directory)
        .map_err(|error| format!("canonicalize job directory: {error}"))?;
    let receipt_parent = receipt_path
        .parent()
        .ok_or_else(|| "producer receipt has no parent".to_owned())?;
    let canonical_receipt_parent = fs::canonicalize(receipt_parent)
        .map_err(|error| format!("canonicalize receipt parent: {error}"))?;
    if canonical_receipt_parent != canonical_job {
        return Err(
            "fixture receipt must be written directly inside the admitted job directory".to_owned(),
        );
    }

    let after = sha256_file(&source)?;
    if after != before {
        return Err("fixture executor observed source mutation".to_owned());
    }

    let receipt = json!({
        "receipt_version": "chaptera.rescue-recovery-producer-receipt.v1",
        "fixture": {
            "kind": "healthy_control",
            "source_sha256": HEALTHY_CONTROL_SHA256
        },
        "source_immutability": {
            "before_sha256": before,
            "after_sha256": after,
            "unchanged": true
        },
        "recovery": {
            "recovery_class": "diagnostic_only",
            "repair_plan_id": "healthy-control-no-repair",
            "action_ids": ["diagnostic-only"],
            "fabricated_bytes": 0,
            "silent_drops": 0
        },
        "artifacts": [],
        "loss": {
            "known_loss": false,
            "items": []
        },
        "native_validation": {
            "state": "not_attempted"
        },
        "proposed_route": "diagnostic_only",
        "privacy": {
            "raw_pub_bytes": false,
            "document_text": false,
            "local_paths": false,
            "customer_identity": false,
            "credentials": false
        }
    });

    fs::write(
        &receipt_path,
        serde_json::to_vec_pretty(&receipt)
            .map_err(|error| format!("serialize fixture receipt: {error}"))?,
    )
    .map_err(|error| format!("write fixture receipt: {error}"))?;
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("fixture executor failure: {error}");
        std::process::exit(2);
    }
}
