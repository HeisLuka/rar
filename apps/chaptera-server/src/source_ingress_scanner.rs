use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use async_trait::async_trait;
use rand::{RngCore, rngs::OsRng};
use serde::Deserialize;
use tokio::{
    fs::File,
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
    time::timeout,
};

use crate::{
    source_ingress::IngressError,
    source_ingress_async::{
        AsyncSourceSecurityScanner, SourceSecurityScanOutcome, SourceSecurityScanReceipt,
    },
};

const SECURITY_PROFILE_V1: &str = "chaptera-untrusted-pub-v1";
const RESULT_PROTOCOL_V1: &str = "chaptera.untrusted-pub-scan-result.v1";
const NETWORK_POLICY_V1: &str = "seccomp_default_deny";
const RECEIPT_MAX_BYTES: usize = 1024 * 1024;
const COPY_BUFFER_BYTES: usize = 64 * 1024;
const MIB: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolatedPubScannerPolicy {
    pub max_file_bytes: u64,
    pub max_cfb_entries: u64,
    pub max_declared_stream_bytes: u64,
    pub wall_timeout_ms: u64,
    pub address_space_mb: u64,
    pub cpu_seconds: u64,
    pub open_files: u64,
    pub output_file_mb: u64,
}

impl IsolatedPubScannerPolicy {
    pub fn validate(&self) -> Result<(), IngressError> {
        for (field, value) in [
            ("max_file_bytes", self.max_file_bytes),
            ("max_cfb_entries", self.max_cfb_entries),
            (
                "max_declared_stream_bytes",
                self.max_declared_stream_bytes,
            ),
            ("wall_timeout_ms", self.wall_timeout_ms),
            ("address_space_mb", self.address_space_mb),
            ("cpu_seconds", self.cpu_seconds),
            ("open_files", self.open_files),
            ("output_file_mb", self.output_file_mb),
        ] {
            if value == 0 {
                return Err(IngressError::new(
                    "invalid_scanner_policy",
                    format!("{field} must be positive"),
                ));
            }
        }
        if self.address_space_mb < 64 {
            return Err(IngressError::new(
                "invalid_scanner_policy",
                "address_space_mb must be at least 64",
            ));
        }
        if self.open_files < 16 {
            return Err(IngressError::new(
                "invalid_scanner_policy",
                "open_files must be at least 16",
            ));
        }
        self.address_space_mb.checked_mul(MIB).ok_or_else(|| {
            IngressError::new(
                "invalid_scanner_policy",
                "address-space byte conversion overflowed",
            )
        })?;
        self.output_file_mb.checked_mul(MIB).ok_or_else(|| {
            IngressError::new(
                "invalid_scanner_policy",
                "output-file byte conversion overflowed",
            )
        })?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct IsolatedPubScannerConfig {
    pub python_program: PathBuf,
    pub isolation_script: PathBuf,
    pub worker_program: PathBuf,
    pub temp_root: PathBuf,
    pub policy: IsolatedPubScannerPolicy,
}

impl IsolatedPubScannerConfig {
    pub fn validate(&self) -> Result<(), IngressError> {
        for (field, path) in [
            ("python_program", self.python_program.as_path()),
            ("isolation_script", self.isolation_script.as_path()),
            ("worker_program", self.worker_program.as_path()),
            ("temp_root", self.temp_root.as_path()),
        ] {
            if !path.is_absolute() {
                return Err(IngressError::new(
                    "scanner_path_not_absolute",
                    format!("{field} must be an absolute path"),
                ));
            }
        }
        self.policy.validate()
    }
}

#[derive(Clone)]
pub struct IsolatedPubSecurityScanner {
    config: IsolatedPubScannerConfig,
}

impl IsolatedPubSecurityScanner {
    pub fn new(config: IsolatedPubScannerConfig) -> Result<Self, IngressError> {
        config.validate()?;
        Ok(Self { config })
    }

    async fn spool_source(
        &self,
        input: &mut (dyn AsyncRead + Unpin + Send),
        path: &Path,
    ) -> Result<Option<u64>, IngressError> {
        let mut output = File::create(path)
            .await
            .map_err(|_| IngressError::new("scanner_temp_failed", "cannot create scanner input"))?;
        let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
        let mut written = 0_u64;

        loop {
            let count = input
                .read(&mut buffer)
                .await
                .map_err(|_| IngressError::new("scanner_input_failed", "cannot read source stream"))?;
            if count == 0 {
                break;
            }
            let count = u64::try_from(count).map_err(|_| {
                IngressError::new("scanner_input_failed", "source byte count overflowed")
            })?;
            written = written.checked_add(count).ok_or_else(|| {
                IngressError::new("scanner_input_failed", "source byte count overflowed")
            })?;
            if written > self.config.policy.max_file_bytes {
                return Ok(None);
            }
            output
                .write_all(&buffer[..usize::try_from(count).expect("read count fits usize")])
                .await
                .map_err(|_| {
                    IngressError::new("scanner_temp_failed", "cannot write scanner input")
                })?;
        }

        output
            .flush()
            .await
            .map_err(|_| IngressError::new("scanner_temp_failed", "cannot flush scanner input"))?;
        output
            .sync_all()
            .await
            .map_err(|_| IngressError::new("scanner_temp_failed", "cannot sync scanner input"))?;
        Ok(Some(written))
    }

    async fn run_worker(
        &self,
        input_path: &Path,
        output_dir: &Path,
    ) -> Result<IsolatedWorkerControl, IngressError> {
        let policy = &self.config.policy;
        let mut command = Command::new(&self.config.python_program);
        command
            .arg(&self.config.isolation_script)
            .arg("run")
            .arg("--output-dir")
            .arg(output_dir)
            .arg("--input")
            .arg(input_path)
            .arg("--timeout")
            .arg(format_seconds(policy.wall_timeout_ms))
            .arg("--address-space-mb")
            .arg(policy.address_space_mb.to_string())
            .arg("--cpu-seconds")
            .arg(policy.cpu_seconds.to_string())
            .arg("--open-files")
            .arg(policy.open_files.to_string())
            .arg("--output-file-mb")
            .arg(policy.output_file_mb.to_string())
            .arg("--clear-environment")
            .arg("--")
            .arg(&self.config.worker_program)
            .arg("inspect")
            .arg("--max-file-bytes")
            .arg(policy.max_file_bytes.to_string())
            .arg("--max-cfb-entries")
            .arg(policy.max_cfb_entries.to_string())
            .arg("--max-declared-stream-bytes")
            .arg(policy.max_declared_stream_bytes.to_string())
            .stdin(Stdio::null())
            .kill_on_drop(true);

        let outer_timeout = Duration::from_millis(
            policy
                .wall_timeout_ms
                .checked_add(5_000)
                .ok_or_else(|| {
                    IngressError::new(
                        "invalid_scanner_policy",
                        "scanner outer timeout overflowed",
                    )
                })?,
        );
        let output = timeout(outer_timeout, command.output())
            .await
            .map_err(|_| {
                IngressError::new(
                    "source_security_launcher_timeout",
                    "isolated scanner launcher exceeded its outer control timeout",
                )
            })?
            .map_err(|_| {
                IngressError::new(
                    "source_security_launcher_failed",
                    "cannot execute canonical isolated-worker launcher",
                )
            })?;

        let control: IsolatedWorkerControl = serde_json::from_slice(&output.stdout).map_err(|_| {
            IngressError::new(
                "source_security_control_invalid",
                "isolated-worker launcher returned an invalid control receipt",
            )
        })?;

        if output.status.success() {
            if control.status != "success"
                || control.timed_out
                || control.network_policy != NETWORK_POLICY_V1
            {
                return Err(IngressError::new(
                    "source_security_control_invalid",
                    "isolated-worker success receipt violates the security contract",
                ));
            }
            return Ok(control);
        }

        if control.status == "timeout" || control.timed_out {
            return Ok(control);
        }
        Err(IngressError::new(
            "source_security_worker_failed",
            "isolated PUB scanner failed without a publishable security receipt",
        ))
    }

    async fn read_scan_receipt(
        &self,
        output_dir: &Path,
    ) -> Result<PubScanResultReceipt, IngressError> {
        let bytes = tokio::fs::read(output_dir.join("result.json"))
            .await
            .map_err(|_| {
                IngressError::new(
                    "source_security_receipt_missing",
                    "isolated PUB scanner did not publish result.json",
                )
            })?;
        if bytes.len() > RECEIPT_MAX_BYTES {
            return Err(IngressError::new(
                "source_security_receipt_too_large",
                "isolated PUB scanner receipt exceeds the control-plane limit",
            ));
        }
        serde_json::from_slice(&bytes).map_err(|_| {
            IngressError::new(
                "source_security_receipt_invalid",
                "isolated PUB scanner receipt is invalid",
            )
        })
    }
}

#[async_trait]
impl AsyncSourceSecurityScanner for IsolatedPubSecurityScanner {
    async fn scan(
        &self,
        input: &mut (dyn AsyncRead + Unpin + Send),
    ) -> Result<SourceSecurityScanOutcome, IngressError> {
        let temp = TempScanDir::create(&self.config.temp_root)?;
        let input_path = temp.path().join("source.pub");
        let output_dir = temp.path().join("result");

        let Some(spooled_len) = self.spool_source(input, &input_path).await? else {
            return Ok(SourceSecurityScanOutcome::Rejected {
                code: "pub_policy_rejected",
            });
        };
        if spooled_len == 0 {
            return Ok(SourceSecurityScanOutcome::Rejected {
                code: "pub_parse_failed",
            });
        }

        let control = self.run_worker(&input_path, &output_dir).await?;
        if control.status == "timeout" || control.timed_out {
            return Ok(SourceSecurityScanOutcome::Rejected {
                code: "pub_scan_timed_out",
            });
        }

        let receipt = self.read_scan_receipt(&output_dir).await?;
        if receipt.protocol_version != RESULT_PROTOCOL_V1
            || receipt.security_profile != SECURITY_PROFILE_V1
            || !receipt.filesystem_confinement
        {
            return Err(IngressError::new(
                "source_security_receipt_invalid",
                "isolated PUB scanner receipt does not prove the required security profile",
            ));
        }
        if receipt.byte_len != spooled_len {
            return Err(IngressError::new(
                "source_security_receipt_length_mismatch",
                "isolated PUB scanner receipt length differs from spooled exact source",
            ));
        }
        require_sha256(&receipt.sha256)?;

        match receipt.status.as_str() {
            "accepted_cfb" => Ok(SourceSecurityScanOutcome::Accepted(
                SourceSecurityScanReceipt {
                    validation_profile: SECURITY_PROFILE_V1.to_owned(),
                    inspected_sha256: receipt.sha256,
                    inspected_byte_len: receipt.byte_len,
                },
            )),
            "parse_failed" => Ok(SourceSecurityScanOutcome::Rejected {
                code: "pub_parse_failed",
            }),
            "rejected_by_policy" => Ok(SourceSecurityScanOutcome::Rejected {
                code: "pub_policy_rejected",
            }),
            _ => Err(IngressError::new(
                "source_security_receipt_invalid",
                "isolated PUB scanner returned an unknown status",
            )),
        }
    }
}

#[derive(Debug, Deserialize)]
struct IsolatedWorkerControl {
    status: String,
    timed_out: bool,
    network_policy: String,
}

#[derive(Debug, Deserialize)]
struct PubScanResultReceipt {
    protocol_version: String,
    security_profile: String,
    status: String,
    byte_len: u64,
    sha256: String,
    filesystem_confinement: bool,
}

fn format_seconds(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    let remainder = milliseconds % 1000;
    if remainder == 0 {
        seconds.to_string()
    } else {
        format!("{seconds}.{remainder:03}")
    }
}

fn require_sha256(value: &str) -> Result<(), IngressError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(IngressError::new(
            "source_security_receipt_invalid",
            "isolated PUB scanner SHA-256 is not canonical lowercase hex",
        ));
    }
    Ok(())
}

struct TempScanDir {
    path: PathBuf,
}

impl TempScanDir {
    fn create(root: &Path) -> Result<Self, IngressError> {
        if !root.is_absolute() {
            return Err(IngressError::new(
                "scanner_path_not_absolute",
                "scanner temp_root must be absolute",
            ));
        }
        fs::create_dir_all(root).map_err(|_| {
            IngressError::new(
                "scanner_temp_failed",
                "cannot create scanner temporary root",
            )
        })?;

        for _ in 0..16 {
            let mut random = [0_u8; 16];
            OsRng.try_fill_bytes(&mut random).map_err(|_| {
                IngressError::new(
                    "scanner_temp_failed",
                    "cannot allocate random scanner temp identity",
                )
            })?;
            let name = random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let path = root.join(format!("source-scan-{name}"));
            match fs::create_dir(&path) {
                Ok(()) => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).map_err(
                            |_| {
                                IngressError::new(
                                    "scanner_temp_failed",
                                    "cannot restrict scanner temp permissions",
                                )
                            },
                        )?;
                    }
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => {
                    return Err(IngressError::new(
                        "scanner_temp_failed",
                        "cannot create scanner temp directory",
                    ));
                }
            }
        }

        Err(IngressError::new(
            "scanner_temp_failed",
            "cannot allocate unique scanner temp directory",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempScanDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_requires_explicit_bounded_values() {
        let mut policy = IsolatedPubScannerPolicy {
            max_file_bytes: 1024,
            max_cfb_entries: 8,
            max_declared_stream_bytes: 2048,
            wall_timeout_ms: 1000,
            address_space_mb: 64,
            cpu_seconds: 1,
            open_files: 16,
            output_file_mb: 1,
        };
        policy.validate().unwrap();

        policy.open_files = 15;
        assert_eq!(
            policy.validate().unwrap_err().code,
            "invalid_scanner_policy"
        );
    }

    #[test]
    fn milliseconds_render_without_float_rounding() {
        assert_eq!(format_seconds(15_000), "15");
        assert_eq!(format_seconds(15_250), "15.250");
        assert_eq!(format_seconds(1), "0.001");
    }
}
