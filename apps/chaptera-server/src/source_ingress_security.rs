use std::{
    fmt,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use chaptera_untrusted_pub_scan::{
    PubScanPolicyV1, PubScanResultV1, PubScanStatusV1, SECURITY_PROFILE_V1,
};
use rand::RngCore;
use sha2::{Digest, Sha256};
use tokio::{
    fs,
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    process::Command,
    time::timeout,
};

use crate::{
    source_ingress::IngressError,
    source_ingress_async::{
        AsyncSourceSecurityScanner, SourceSecurityScanOutcome, SourceSecurityScanReceipt,
    },
};

pub const SOURCE_SECURITY_PROFILE_V1: &str = "chaptera-source-ingress-security-v1";
const CLAMD_MAX_REPLY_BYTES: usize = 64 * 1024;
const STREAM_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct SourceSecurityScannerConfig {
    pub clamd_endpoint: SocketAddr,
    pub clamd_connect_timeout: Duration,
    pub clamd_io_timeout: Duration,
    pub isolation_python: PathBuf,
    pub isolation_harness: PathBuf,
    pub worker_binary: PathBuf,
    pub worker_wall_timeout: Duration,
    pub worker_address_space_mb: u64,
    pub worker_cpu_seconds: u64,
    pub worker_open_files: u64,
    pub worker_output_file_mb: u64,
    pub policy: PubScanPolicyV1,
    pub temp_root: PathBuf,
}

impl SourceSecurityScannerConfig {
    pub fn validate(&self) -> Result<(), IngressError> {
        if !self.clamd_endpoint.ip().is_loopback() {
            return Err(IngressError::new(
                "source_scanner_config_invalid",
                "clamd endpoint must be loopback",
            ));
        }
        if self.clamd_connect_timeout.is_zero()
            || self.clamd_io_timeout.is_zero()
            || self.worker_wall_timeout.is_zero()
        {
            return Err(IngressError::new(
                "source_scanner_config_invalid",
                "scanner timeouts must be positive",
            ));
        }
        if self.worker_address_space_mb < 64
            || self.worker_cpu_seconds == 0
            || self.worker_open_files < 16
            || self.worker_output_file_mb == 0
        {
            return Err(IngressError::new(
                "source_scanner_config_invalid",
                "worker limits are outside the admitted range",
            ));
        }
        self.policy
            .validate()
            .map_err(|error| IngressError::new("source_scanner_config_invalid", error))?;
        for (path, label) in [
            (&self.isolation_python, "isolation_python"),
            (&self.isolation_harness, "isolation_harness"),
            (&self.worker_binary, "worker_binary"),
            (&self.temp_root, "temp_root"),
        ] {
            if path.as_os_str().is_empty() {
                return Err(IngressError::new(
                    "source_scanner_config_invalid",
                    format!("{label} must be configured"),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StructuralScanEvidence {
    status: PubScanStatusV1,
    byte_len: u64,
    sha256: String,
    filesystem_confinement: bool,
}

#[async_trait]
trait StructuralScanRunner: Send + Sync {
    async fn scan_file(
        &self,
        input_path: &Path,
        output_dir: &Path,
        config: &SourceSecurityScannerConfig,
    ) -> Result<StructuralScanEvidence, IngressError>;
}

#[derive(Debug, Default)]
struct IsolatedPubWorkerRunner;

#[async_trait]
impl StructuralScanRunner for IsolatedPubWorkerRunner {
    async fn scan_file(
        &self,
        input_path: &Path,
        output_dir: &Path,
        config: &SourceSecurityScannerConfig,
    ) -> Result<StructuralScanEvidence, IngressError> {
        let timeout_seconds = config.worker_wall_timeout.as_secs_f64().to_string();
        let max_file_bytes = config.policy.max_file_bytes.to_string();
        let max_cfb_entries = config.policy.max_cfb_entries.to_string();
        let max_declared_stream_bytes = config.policy.max_declared_stream_bytes.to_string();

        let child = Command::new(&config.isolation_python)
            .arg(&config.isolation_harness)
            .arg("run")
            .arg("--output-dir")
            .arg(output_dir)
            .arg("--input")
            .arg(input_path)
            .arg("--timeout")
            .arg(timeout_seconds)
            .arg("--address-space-mb")
            .arg(config.worker_address_space_mb.to_string())
            .arg("--cpu-seconds")
            .arg(config.worker_cpu_seconds.to_string())
            .arg("--open-files")
            .arg(config.worker_open_files.to_string())
            .arg("--output-file-mb")
            .arg(config.worker_output_file_mb.to_string())
            .arg("--")
            .arg(&config.worker_binary)
            .arg("inspect")
            .arg("--max-file-bytes")
            .arg(max_file_bytes)
            .arg("--max-cfb-entries")
            .arg(max_cfb_entries)
            .arg("--max-declared-stream-bytes")
            .arg(max_declared_stream_bytes)
            .kill_on_drop(true)
            .output();

        let process_timeout = config
            .worker_wall_timeout
            .checked_add(Duration::from_secs(5))
            .ok_or_else(|| {
                IngressError::new(
                    "source_scanner_config_invalid",
                    "worker timeout overflow",
                )
            })?;

        let output = timeout(process_timeout, child)
            .await
            .map_err(|_| {
                IngressError::new(
                    "source_structural_scan_timeout",
                    "isolated PUB worker exceeded parent timeout",
                )
            })?
            .map_err(|_| {
                IngressError::new(
                    "source_structural_scan_failed",
                    "isolated PUB worker could not be started",
                )
            })?;

        if !output.status.success() {
            return Err(IngressError::new(
                "source_structural_scan_failed",
                "isolated PUB worker rejected or failed before producing an accepted result",
            ));
        }

        let result_bytes = fs::read(output_dir.join("result.json"))
            .await
            .map_err(|_| {
                IngressError::new(
                    "source_structural_scan_receipt_missing",
                    "isolated PUB worker result is missing",
                )
            })?;
        if result_bytes.len() > 256 * 1024 {
            return Err(IngressError::new(
                "source_structural_scan_receipt_invalid",
                "isolated PUB worker result is too large",
            ));
        }
        let result: PubScanResultV1 = serde_json::from_slice(&result_bytes).map_err(|_| {
            IngressError::new(
                "source_structural_scan_receipt_invalid",
                "isolated PUB worker result is malformed",
            )
        })?;

        if result.protocol_version != "chaptera.untrusted-pub-scan-result.v1"
            || result.security_profile != SECURITY_PROFILE_V1
        {
            return Err(IngressError::new(
                "source_structural_scan_receipt_invalid",
                "isolated PUB worker returned an unsupported receipt profile",
            ));
        }

        Ok(StructuralScanEvidence {
            status: result.status,
            byte_len: result.byte_len,
            sha256: result.sha256,
            filesystem_confinement: result.filesystem_confinement,
        })
    }
}

pub struct ProductionSourceSecurityScanner {
    config: SourceSecurityScannerConfig,
    structural: Arc<dyn StructuralScanRunner>,
}

impl fmt::Debug for ProductionSourceSecurityScanner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProductionSourceSecurityScanner")
            .field("clamd_endpoint", &self.config.clamd_endpoint)
            .field("policy", &self.config.policy)
            .finish_non_exhaustive()
    }
}

impl ProductionSourceSecurityScanner {
    pub fn new(config: SourceSecurityScannerConfig) -> Result<Self, IngressError> {
        config.validate()?;
        Ok(Self {
            config,
            structural: Arc::new(IsolatedPubWorkerRunner),
        })
    }

    #[cfg(test)]
    fn with_runner(
        config: SourceSecurityScannerConfig,
        structural: Arc<dyn StructuralScanRunner>,
    ) -> Result<Self, IngressError> {
        config.validate()?;
        Ok(Self { config, structural })
    }

    async fn scan_inner(
        &self,
        input: &mut (dyn AsyncRead + Unpin + Send),
    ) -> Result<SourceSecurityScanOutcome, IngressError> {
        let temp = ScanTempDir::create(&self.config.temp_root).await?;
        let input_path = temp.path().join("source.pub");
        let output_dir = temp.path().join("worker-result");
        let mut file = fs::File::create(&input_path).await.map_err(|_| {
            IngressError::new(
                "source_scanner_temp_failed",
                "scanner could not create bounded private input",
            )
        })?;

        let mut clamd = timeout(
            self.config.clamd_connect_timeout,
            TcpStream::connect(self.config.clamd_endpoint),
        )
        .await
        .map_err(|_| {
            IngressError::new(
                "source_malware_scanner_unavailable",
                "clamd connection timed out",
            )
        })?
        .map_err(|_| {
            IngressError::new(
                "source_malware_scanner_unavailable",
                "clamd connection failed",
            )
        })?;

        timeout(
            self.config.clamd_io_timeout,
            clamd.write_all(b"zINSTREAM\0"),
        )
        .await
        .map_err(|_| {
            IngressError::new(
                "source_malware_scanner_timeout",
                "clamd command write timed out",
            )
        })?
        .map_err(|_| {
            IngressError::new(
                "source_malware_scanner_failed",
                "clamd command write failed",
            )
        })?;

        let mut buffer = vec![0_u8; STREAM_CHUNK_BYTES];
        let mut total = 0_u64;
        let mut hasher = Sha256::new();

        loop {
            let count = input.read(&mut buffer).await.map_err(|_| {
                IngressError::new(
                    "source_scanner_read_failed",
                    "scanner could not read exact quarantine stream",
                )
            })?;
            if count == 0 {
                break;
            }
            total = total
                .checked_add(u64::try_from(count).map_err(|_| {
                    IngressError::new("source_scanner_overflow", "scanner byte count overflow")
                })?)
                .ok_or_else(|| {
                    IngressError::new("source_scanner_overflow", "scanner byte count overflow")
                })?;
            if total > self.config.policy.max_file_bytes {
                return Ok(SourceSecurityScanOutcome::Rejected {
                    code: "pub_size_limit",
                });
            }

            let chunk = &buffer[..count];
            hasher.update(chunk);
            file.write_all(chunk).await.map_err(|_| {
                IngressError::new(
                    "source_scanner_temp_failed",
                    "scanner could not materialize bounded private input",
                )
            })?;

            let length = u32::try_from(count).map_err(|_| {
                IngressError::new(
                    "source_malware_scanner_failed",
                    "clamd INSTREAM chunk length overflow",
                )
            })?;
            timeout(
                self.config.clamd_io_timeout,
                clamd.write_all(&length.to_be_bytes()),
            )
            .await
            .map_err(|_| {
                IngressError::new(
                    "source_malware_scanner_timeout",
                    "clamd chunk header write timed out",
                )
            })?
            .map_err(|_| {
                IngressError::new(
                    "source_malware_scanner_failed",
                    "clamd chunk header write failed",
                )
            })?;
            timeout(self.config.clamd_io_timeout, clamd.write_all(chunk))
                .await
                .map_err(|_| {
                    IngressError::new(
                        "source_malware_scanner_timeout",
                        "clamd payload write timed out",
                    )
                })?
                .map_err(|_| {
                    IngressError::new(
                        "source_malware_scanner_failed",
                        "clamd payload write failed",
                    )
                })?;
        }

        file.flush().await.map_err(|_| {
            IngressError::new(
                "source_scanner_temp_failed",
                "scanner could not flush private input",
            )
        })?;
        drop(file);

        timeout(
            self.config.clamd_io_timeout,
            clamd.write_all(&0_u32.to_be_bytes()),
        )
        .await
        .map_err(|_| {
            IngressError::new(
                "source_malware_scanner_timeout",
                "clamd terminator write timed out",
            )
        })?
        .map_err(|_| {
            IngressError::new(
                "source_malware_scanner_failed",
                "clamd terminator write failed",
            )
        })?;

        let malware = read_clamd_reply(&mut clamd, self.config.clamd_io_timeout).await?;
        match malware {
            ClamdOutcome::Clean => {}
            ClamdOutcome::Detected => {
                return Ok(SourceSecurityScanOutcome::Rejected {
                    code: "malware_detected",
                });
            }
        }

        let source_sha256 = format!("{:x}", hasher.finalize());
        let structural = self
            .structural
            .scan_file(&input_path, &output_dir, &self.config)
            .await?;

        if structural.byte_len != total || structural.sha256 != source_sha256 {
            return Err(IngressError::new(
                "source_structural_scan_identity_mismatch",
                "isolated PUB worker receipt does not match the malware-scanned bytes",
            ));
        }
        if !structural.filesystem_confinement {
            return Err(IngressError::new(
                "source_structural_scan_unconfined",
                "isolated PUB worker did not prove filesystem confinement",
            ));
        }

        match structural.status {
            PubScanStatusV1::AcceptedCfb => Ok(SourceSecurityScanOutcome::Accepted(
                SourceSecurityScanReceipt {
                    validation_profile: SOURCE_SECURITY_PROFILE_V1.to_owned(),
                },
            )),
            PubScanStatusV1::ParseFailed => Ok(SourceSecurityScanOutcome::Rejected {
                code: "pub_structure_invalid",
            }),
            PubScanStatusV1::RejectedByPolicy => Ok(SourceSecurityScanOutcome::Rejected {
                code: "pub_policy_rejected",
            }),
        }
    }
}

#[async_trait]
impl AsyncSourceSecurityScanner for ProductionSourceSecurityScanner {
    async fn scan(
        &self,
        input: &mut (dyn AsyncRead + Unpin + Send),
    ) -> Result<SourceSecurityScanOutcome, IngressError> {
        self.scan_inner(input).await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClamdOutcome {
    Clean,
    Detected,
}

async fn read_clamd_reply(
    stream: &mut TcpStream,
    io_timeout: Duration,
) -> Result<ClamdOutcome, IngressError> {
    let reply = timeout(io_timeout, async {
        let mut bytes = Vec::new();
        let mut one = [0_u8; 1];
        loop {
            let count = stream.read(&mut one).await?;
            if count == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "clamd reply ended before NUL",
                ));
            }
            if one[0] == 0 {
                break;
            }
            if bytes.len() >= CLAMD_MAX_REPLY_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "clamd reply too large",
                ));
            }
            bytes.push(one[0]);
        }
        Ok::<Vec<u8>, std::io::Error>(bytes)
    })
    .await
    .map_err(|_| {
        IngressError::new(
            "source_malware_scanner_timeout",
            "clamd reply timed out",
        )
    })?
    .map_err(|_| {
        IngressError::new(
            "source_malware_scanner_failed",
            "clamd reply was unavailable or malformed",
        )
    })?;

    let reply = String::from_utf8(reply).map_err(|_| {
        IngressError::new(
            "source_malware_scanner_failed",
            "clamd reply was not valid UTF-8",
        )
    })?;
    let trimmed = reply.trim();
    if trimmed == "stream: OK" {
        return Ok(ClamdOutcome::Clean);
    }
    if trimmed.starts_with("stream: ") && trimmed.ends_with(" FOUND") {
        return Ok(ClamdOutcome::Detected);
    }
    Err(IngressError::new(
        "source_malware_scanner_failed",
        "clamd returned an unsupported result",
    ))
}

struct ScanTempDir {
    path: PathBuf,
}

impl ScanTempDir {
    async fn create(root: &Path) -> Result<Self, IngressError> {
        fs::create_dir_all(root).await.map_err(|_| {
            IngressError::new(
                "source_scanner_temp_failed",
                "scanner temp root is unavailable",
            )
        })?;

        for _ in 0..8 {
            let mut random = [0_u8; 16];
            rand::thread_rng().fill_bytes(&mut random);
            let name = format!(
                "chaptera-source-scan-{}",
                random.iter().map(|byte| format!("{byte:02x}")).collect::<String>()
            );
            let path = root.join(name);
            match fs::create_dir(&path).await {
                Ok(()) => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                            .await
                            .map_err(|_| {
                                IngressError::new(
                                    "source_scanner_temp_failed",
                                    "scanner temp permissions could not be constrained",
                                )
                            })?;
                    }
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => {
                    return Err(IngressError::new(
                        "source_scanner_temp_failed",
                        "scanner temp directory could not be created",
                    ));
                }
            }
        }

        Err(IngressError::new(
            "source_scanner_temp_failed",
            "scanner temp directory collision budget exhausted",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScanTempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt, duplex},
        net::TcpListener,
    };

    use super::*;

    fn base_config(endpoint: SocketAddr, temp_root: PathBuf) -> SourceSecurityScannerConfig {
        SourceSecurityScannerConfig {
            clamd_endpoint: endpoint,
            clamd_connect_timeout: Duration::from_secs(2),
            clamd_io_timeout: Duration::from_secs(2),
            isolation_python: PathBuf::from("python3"),
            isolation_harness: PathBuf::from("tools/migration_pdf_worker_isolation.py"),
            worker_binary: PathBuf::from("target/debug/chaptera-untrusted-pub-worker"),
            worker_wall_timeout: Duration::from_secs(15),
            worker_address_space_mb: 1024,
            worker_cpu_seconds: 10,
            worker_open_files: 64,
            worker_output_file_mb: 32,
            policy: PubScanPolicyV1::default(),
            temp_root,
        }
    }

    async fn spawn_clamd(reply: &'static [u8], expected: Vec<u8>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut command = [0_u8; 10];
            stream.read_exact(&mut command).await.unwrap();
            assert_eq!(&command, b"zINSTREAM\0");

            let mut payload = Vec::new();
            loop {
                let mut length = [0_u8; 4];
                stream.read_exact(&mut length).await.unwrap();
                let length = u32::from_be_bytes(length) as usize;
                if length == 0 {
                    break;
                }
                let start = payload.len();
                payload.resize(start + length, 0);
                stream.read_exact(&mut payload[start..]).await.unwrap();
            }
            assert_eq!(payload, expected);
            stream.write_all(reply).await.unwrap();
        });
        address
    }

    struct FakeStructuralRunner {
        status: PubScanStatusV1,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl StructuralScanRunner for FakeStructuralRunner {
        async fn scan_file(
            &self,
            input_path: &Path,
            _output_dir: &Path,
            _config: &SourceSecurityScannerConfig,
        ) -> Result<StructuralScanEvidence, IngressError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let bytes = fs::read(input_path).await.unwrap();
            Ok(StructuralScanEvidence {
                status: self.status.clone(),
                byte_len: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                filesystem_confinement: true,
            })
        }
    }

    async fn scan_bytes(
        scanner: &ProductionSourceSecurityScanner,
        bytes: &[u8],
    ) -> Result<SourceSecurityScanOutcome, IngressError> {
        let (mut writer, mut reader) = duplex(bytes.len().max(1) * 2 + 32);
        let payload = bytes.to_vec();
        let write = tokio::spawn(async move {
            writer.write_all(&payload).await.unwrap();
            writer.shutdown().await.unwrap();
        });
        let result = scanner.scan(&mut reader).await;
        write.await.unwrap();
        result
    }

    #[test]
    fn non_loopback_clamd_is_rejected() {
        let temp = env::temp_dir();
        let config = base_config("192.0.2.1:3310".parse().unwrap(), temp);
        assert_eq!(
            config.validate().unwrap_err().code,
            "source_scanner_config_invalid"
        );
    }

    #[tokio::test]
    async fn clean_clamd_plus_confined_structural_accepts_exact_stream() {
        let bytes = b"exact Publisher payload".to_vec();
        let endpoint = spawn_clamd(b"stream: OK\0", bytes.clone()).await;
        let runner = Arc::new(FakeStructuralRunner {
            status: PubScanStatusV1::AcceptedCfb,
            calls: AtomicUsize::new(0),
        });
        let scanner = ProductionSourceSecurityScanner::with_runner(
            base_config(endpoint, env::temp_dir()),
            runner.clone(),
        )
        .unwrap();

        let outcome = scan_bytes(&scanner, &bytes).await.unwrap();
        assert!(matches!(
            outcome,
            SourceSecurityScanOutcome::Accepted(SourceSecurityScanReceipt {
                validation_profile
            }) if validation_profile == SOURCE_SECURITY_PROFILE_V1
        ));
        assert_eq!(runner.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn malware_detection_rejects_before_structural_worker() {
        let bytes = b"eicar-like-test-payload".to_vec();
        let endpoint =
            spawn_clamd(b"stream: Eicar-Test-Signature FOUND\0", bytes.clone()).await;
        let runner = Arc::new(FakeStructuralRunner {
            status: PubScanStatusV1::AcceptedCfb,
            calls: AtomicUsize::new(0),
        });
        let scanner = ProductionSourceSecurityScanner::with_runner(
            base_config(endpoint, env::temp_dir()),
            runner.clone(),
        )
        .unwrap();

        let outcome = scan_bytes(&scanner, &bytes).await.unwrap();
        assert!(matches!(
            outcome,
            SourceSecurityScanOutcome::Rejected {
                code: "malware_detected"
            }
        ));
        assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn structural_parse_failure_is_bounded_rejection() {
        let bytes = b"invalid Publisher payload".to_vec();
        let endpoint = spawn_clamd(b"stream: OK\0", bytes.clone()).await;
        let runner = Arc::new(FakeStructuralRunner {
            status: PubScanStatusV1::ParseFailed,
            calls: AtomicUsize::new(0),
        });
        let scanner = ProductionSourceSecurityScanner::with_runner(
            base_config(endpoint, env::temp_dir()),
            runner,
        )
        .unwrap();

        let outcome = scan_bytes(&scanner, &bytes).await.unwrap();
        assert!(matches!(
            outcome,
            SourceSecurityScanOutcome::Rejected {
                code: "pub_structure_invalid"
            }
        ));
    }

    #[tokio::test]
    async fn unsupported_clamd_reply_fails_closed() {
        let bytes = b"payload".to_vec();
        let endpoint = spawn_clamd(b"stream: scanner unavailable ERROR\0", bytes.clone()).await;
        let runner = Arc::new(FakeStructuralRunner {
            status: PubScanStatusV1::AcceptedCfb,
            calls: AtomicUsize::new(0),
        });
        let scanner = ProductionSourceSecurityScanner::with_runner(
            base_config(endpoint, env::temp_dir()),
            runner,
        )
        .unwrap();

        assert_eq!(
            scan_bytes(&scanner, &bytes).await.unwrap_err().code,
            "source_malware_scanner_failed"
        );
    }

    #[tokio::test]
    async fn real_isolated_runner_accepts_pinned_pub_when_ci_fixture_is_available() {
        let Ok(pub_path) = env::var("CHAPTERA_TEST_PUB") else {
            return;
        };
        let Ok(worker) = env::var("CHAPTERA_TEST_UNTRUSTED_WORKER") else {
            return;
        };
        let harness = env::var("CHAPTERA_TEST_ISOLATION_HARNESS")
            .unwrap_or_else(|_| "tools/migration_pdf_worker_isolation.py".to_owned());

        let root = ScanTempDir::create(&env::temp_dir()).await.unwrap();
        let output = root.path().join("result");
        let config = SourceSecurityScannerConfig {
            clamd_endpoint: "127.0.0.1:3310".parse().unwrap(),
            isolation_python: PathBuf::from("python3"),
            isolation_harness: PathBuf::from(harness),
            worker_binary: PathBuf::from(worker),
            ..base_config("127.0.0.1:3310".parse().unwrap(), env::temp_dir())
        };

        let evidence = IsolatedPubWorkerRunner
            .scan_file(Path::new(&pub_path), &output, &config)
            .await
            .unwrap();
        assert_eq!(evidence.status, PubScanStatusV1::AcceptedCfb);
        assert!(evidence.filesystem_confinement);
        assert_eq!(evidence.sha256.len(), 64);
        assert!(evidence.byte_len > 0);
    }
}
