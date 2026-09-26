use std::{
    collections::BTreeMap,
    env, fs,
    io::Cursor,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use chaptera_server::{
    blob_store::{
        BlobIdGenerator, BlobProvider, BlobStoreError, BlobStoreService, ProviderCapabilities,
        ProviderError, ProviderErrorKind, ProviderGrant, ProviderGrantRequest,
        ProviderObjectMetadata,
    },
    job_queue::{JobKind, SqliteJobQueue},
    job_worker::{CancellationFlag, JobExecutor},
    project_persistence_sqlite::{SqliteProjectPersistence, plan_project_identity},
    schema_migration::SqliteMigrationRuntime,
    source_baseline::{IsolatedSourceBaselineProducer, SourceBaselineProducerConfig},
    source_ingress::{
        ConsumeUploadRequest, IssueUploadRequest, UploadState, plan_upload_candidate,
        upload_admission_reservation_id,
    },
    source_ingress_async::{
        AsyncSourceSecurityScanner, AsyncSourceValidationRuntime, SourceSecurityScanOutcome,
        SourceSecurityScanReceipt,
    },
    source_ingress_sqlite::SqliteSourceIngressRepository,
    source_validation_job::{
        SOURCE_VALIDATION_JOB_PAYLOAD_SCHEMA_V1, SourceValidationJobExecutor,
        SourceValidationJobPayloadV1, SourceValidationJobQueue,
    },
    sqlite_blob_metadata::SqliteBlobBindingRepository,
    upload_admission::{
        SqliteUploadAdmissionAuthority, UploadAdmissionConfig, UploadAdmissionRequest,
    },
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
use tokio::io::{AsyncRead, AsyncReadExt};

const FIXTURE_SOURCE_COMMIT: &str = "942d95d85b15d0dfdb3bc9ba1b4f273f277757c8";
const MAX_SOURCE_BYTES: u64 = 512 * 1024 * 1024;

type BenchResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Clone)]
struct StoredObject {
    bytes: Vec<u8>,
    generation: String,
    etag: String,
}

#[derive(Debug, Default)]
struct ProviderInner {
    objects: Mutex<BTreeMap<String, StoredObject>>,
    puts: AtomicU64,
    gets: AtomicU64,
    heads: AtomicU64,
    deletes: AtomicU64,
    bytes_written: AtomicU64,
    bytes_read: AtomicU64,
}

#[derive(Debug, Clone, Default)]
struct CountingProvider {
    inner: Arc<ProviderInner>,
}

#[derive(Debug, Clone, Serialize)]
struct ProviderMetrics {
    object_puts: u64,
    object_gets: u64,
    object_heads: u64,
    object_deletes: u64,
    object_bytes_written: u64,
    object_bytes_read: u64,
    persisted_object_bytes: u64,
    quarantine_object_bytes: u64,
    canonical_object_bytes: u64,
    persisted_object_count: u64,
}

#[derive(Debug, Clone, Serialize)]
struct ProviderOperationDelta {
    object_puts: u64,
    object_gets: u64,
    object_heads: u64,
    object_deletes: u64,
    object_bytes_written: u64,
    object_bytes_read: u64,
}

impl ProviderOperationDelta {
    fn between(before: &ProviderMetrics, after: &ProviderMetrics) -> Self {
        Self {
            object_puts: after.object_puts.saturating_sub(before.object_puts),
            object_gets: after.object_gets.saturating_sub(before.object_gets),
            object_heads: after.object_heads.saturating_sub(before.object_heads),
            object_deletes: after.object_deletes.saturating_sub(before.object_deletes),
            object_bytes_written: after
                .object_bytes_written
                .saturating_sub(before.object_bytes_written),
            object_bytes_read: after
                .object_bytes_read
                .saturating_sub(before.object_bytes_read),
        }
    }
}

impl CountingProvider {
    fn metrics(&self) -> ProviderMetrics {
        let objects = self.inner.objects.lock().expect("provider object lock");
        let mut persisted = 0_u64;
        let mut quarantine = 0_u64;
        let mut canonical = 0_u64;
        for (locator, object) in objects.iter() {
            let len = u64::try_from(object.bytes.len()).unwrap_or(u64::MAX);
            persisted = persisted.saturating_add(len);
            if locator.starts_with("quarantine/") {
                quarantine = quarantine.saturating_add(len);
            } else if locator.starts_with("canonical/") {
                canonical = canonical.saturating_add(len);
            }
        }
        ProviderMetrics {
            object_puts: self.inner.puts.load(Ordering::Relaxed),
            object_gets: self.inner.gets.load(Ordering::Relaxed),
            object_heads: self.inner.heads.load(Ordering::Relaxed),
            object_deletes: self.inner.deletes.load(Ordering::Relaxed),
            object_bytes_written: self.inner.bytes_written.load(Ordering::Relaxed),
            object_bytes_read: self.inner.bytes_read.load(Ordering::Relaxed),
            persisted_object_bytes: persisted,
            quarantine_object_bytes: quarantine,
            canonical_object_bytes: canonical,
            persisted_object_count: u64::try_from(objects.len()).unwrap_or(u64::MAX),
        }
    }
}

fn record_provider_phase(
    deltas: &mut BTreeMap<String, ProviderOperationDelta>,
    checkpoint: &mut ProviderMetrics,
    provider: &CountingProvider,
    phase: &'static str,
) {
    let after = provider.metrics();
    deltas.insert(
        phase.to_owned(),
        ProviderOperationDelta::between(checkpoint, &after),
    );
    *checkpoint = after;
}

#[async_trait]
impl BlobProvider for CountingProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            hard_create_only: false,
            hard_exact_or_max_upload_size: false,
            signed_content_type: false,
            strong_head_after_put: true,
        }
    }

    async fn create_immutable(
        &self,
        object_locator: &str,
        expected_byte_len: u64,
        mut input: Box<dyn AsyncRead + Unpin + Send>,
    ) -> Result<ProviderObjectMetadata, ProviderError> {
        let mut bytes = Vec::new();
        input.read_to_end(&mut bytes).await.map_err(|_| {
            ProviderError::new(ProviderErrorKind::Other, "bench_provider_read_failed")
        })?;
        let byte_len = u64::try_from(bytes.len()).map_err(|_| {
            ProviderError::new(ProviderErrorKind::Other, "bench_provider_len_overflow")
        })?;
        if byte_len != expected_byte_len {
            return Err(ProviderError::new(
                ProviderErrorKind::Other,
                "bench_provider_length_mismatch",
            ));
        }

        let etag = format!("{:x}", Sha256::digest(&bytes));
        let generation = format!("gen-{}", &etag[..16]);
        let mut objects = self.inner.objects.lock().expect("provider object lock");
        if objects.contains_key(object_locator) {
            return Err(ProviderError::new(
                ProviderErrorKind::AlreadyExists,
                "bench_provider_create_only_conflict",
            ));
        }
        objects.insert(
            object_locator.to_owned(),
            StoredObject {
                bytes,
                generation: generation.clone(),
                etag: etag.clone(),
            },
        );
        self.inner.puts.fetch_add(1, Ordering::Relaxed);
        self.inner
            .bytes_written
            .fetch_add(byte_len, Ordering::Relaxed);

        Ok(ProviderObjectMetadata {
            generation,
            byte_len,
            etag,
        })
    }

    async fn head_exact(
        &self,
        object_locator: &str,
    ) -> Result<Option<ProviderObjectMetadata>, ProviderError> {
        self.inner.heads.fetch_add(1, Ordering::Relaxed);
        let objects = self.inner.objects.lock().expect("provider object lock");
        Ok(objects
            .get(object_locator)
            .map(|object| ProviderObjectMetadata {
                generation: object.generation.clone(),
                byte_len: u64::try_from(object.bytes.len()).unwrap_or(u64::MAX),
                etag: object.etag.clone(),
            }))
    }

    async fn open_read(
        &self,
        object_locator: &str,
        generation: &str,
    ) -> Result<Box<dyn AsyncRead + Unpin + Send>, ProviderError> {
        let object = {
            let objects = self.inner.objects.lock().expect("provider object lock");
            objects.get(object_locator).cloned()
        }
        .ok_or_else(|| {
            ProviderError::new(ProviderErrorKind::NotFound, "bench_provider_not_found")
        })?;
        if object.generation != generation {
            return Err(ProviderError::new(
                ProviderErrorKind::NotFound,
                "bench_provider_generation_mismatch",
            ));
        }
        let byte_len = u64::try_from(object.bytes.len()).unwrap_or(u64::MAX);
        self.inner.gets.fetch_add(1, Ordering::Relaxed);
        self.inner.bytes_read.fetch_add(byte_len, Ordering::Relaxed);
        Ok(Box::new(Cursor::new(object.bytes)))
    }

    async fn delete_exact(
        &self,
        object_locator: &str,
        generation: &str,
    ) -> Result<(), ProviderError> {
        let mut objects = self.inner.objects.lock().expect("provider object lock");
        let object = objects.get(object_locator).ok_or_else(|| {
            ProviderError::new(ProviderErrorKind::NotFound, "bench_provider_not_found")
        })?;
        if object.generation != generation {
            return Err(ProviderError::new(
                ProviderErrorKind::NotFound,
                "bench_provider_generation_mismatch",
            ));
        }
        objects.remove(object_locator);
        self.inner.deletes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn issue_grant(
        &self,
        request: &ProviderGrantRequest,
    ) -> Result<ProviderGrant, ProviderError> {
        let _ = request;
        Err(ProviderError::new(
            ProviderErrorKind::AccessDenied,
            "bench_provider_direct_grants_disabled",
        ))
    }
}

#[derive(Debug, Default)]
struct BenchIds {
    next: AtomicU64,
}

impl BlobIdGenerator for BenchIds {
    fn next_physical_blob_id(&self) -> Result<String, BlobStoreError> {
        Ok(format!(
            "physical-bench-{}",
            self.next.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn next_binding_id(&self) -> Result<String, BlobStoreError> {
        Ok(format!(
            "binding-bench-{}",
            self.next.fetch_add(1, Ordering::Relaxed)
        ))
    }
}

struct ConsumeAllScanner;

#[async_trait]
impl AsyncSourceSecurityScanner for ConsumeAllScanner {
    async fn scan(
        &self,
        input: &mut (dyn AsyncRead + Unpin + Send),
    ) -> Result<SourceSecurityScanOutcome, chaptera_server::source_ingress::IngressError> {
        let copied = tokio::io::copy(input, &mut tokio::io::sink())
            .await
            .map_err(|error| {
                chaptera_server::source_ingress::IngressError::new(
                    "bench_scanner_read_failed",
                    error.to_string(),
                )
            })?;
        if copied == 0 {
            return Err(chaptera_server::source_ingress::IngressError::new(
                "bench_scanner_empty",
                "benchmark fixture unexpectedly contains zero bytes",
            ));
        }
        Ok(SourceSecurityScanOutcome::Accepted(
            SourceSecurityScanReceipt {
                validation_profile: "cloud-link-bench-consume-all-v1".to_owned(),
            },
        ))
    }
}

#[derive(Debug, Serialize)]
struct BenchReceipt {
    protocol_version: &'static str,
    build_sha: String,
    fixture_source_commit: &'static str,
    harness_scope: &'static str,
    http_layer_measured: bool,
    scanner_profile: &'static str,
    price_model_applied: bool,
    runs: Vec<FixtureReceipt>,
}

#[derive(Debug, Serialize)]
struct FixtureReceipt {
    fixture_id: String,
    fixture_file: String,
    fixture_sha256: String,
    fixture_bytes: u64,
    total_wall_ms: u64,
    phase_wall_ms: BTreeMap<String, u64>,
    logical_http_round_trips: u64,
    measured_http_request_count: Option<u64>,
    source_payload_client_bytes_sent: u64,
    measured_http_control_bytes: Option<u64>,
    provider: ProviderMetrics,
    provider_phase_deltas: BTreeMap<String, ProviderOperationDelta>,
    durable_row_counts: BTreeMap<String, i64>,
    sqlite_bytes_before: u64,
    sqlite_bytes_after: u64,
    sqlite_storage_delta_bytes: i64,
    parent_process_cpu_ms: Option<u64>,
    parent_process_rss_start_bytes: Option<u64>,
    parent_process_rss_end_bytes: Option<u64>,
    parent_process_peak_rss_bytes: Option<u64>,
    source_validation_jobs_enqueued: u64,
    source_validation_jobs_executed: u64,
    project_id: String,
    document_id: String,
    genesis_revision_id: String,
    canonical_source_binding_id: String,
    canonical_source_sha256: String,
}

fn unix_now_ms() -> BenchResult<i64> {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    Ok(i64::try_from(millis)?)
}

fn elapsed_ms(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("chaptera-server lives at apps/chaptera-server")
        .to_path_buf()
}

async fn sha256_file(path: &Path) -> BenchResult<(String, u64)> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total = total
            .checked_add(u64::try_from(read)?)
            .ok_or("fixture length overflow")?;
    }
    Ok((format!("{:x}", hasher.finalize()), total))
}

fn sqlite_storage_bytes(path: &Path) -> u64 {
    ["", "-wal", "-shm"]
        .iter()
        .filter_map(|suffix| fs::metadata(format!("{}{}", path.display(), suffix)).ok())
        .map(|metadata| metadata.len())
        .fold(0_u64, u64::saturating_add)
}

fn proc_rss_bytes(field: &str) -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with(field))?;
    let kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    kib.checked_mul(1024)
}

fn proc_cpu_ticks() -> Option<u64> {
    let stat = fs::read_to_string("/proc/self/stat").ok()?;
    let close = stat.rfind(')')?;
    let fields: Vec<&str> = stat[close + 1..].split_whitespace().collect();
    let utime = fields.get(11)?.parse::<u64>().ok()?;
    let stime = fields.get(12)?.parse::<u64>().ok()?;
    utime.checked_add(stime)
}

fn clock_ticks_per_second() -> Option<u64> {
    let output = Command::new("getconf").arg("CLK_TCK").output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}

fn cpu_delta_ms(start: Option<u64>, end: Option<u64>, hz: Option<u64>) -> Option<u64> {
    let ticks = end?.checked_sub(start?)?;
    let hz = hz?;
    if hz == 0 {
        return None;
    }
    ticks.checked_mul(1000)?.checked_div(hz)
}

async fn seed_principal(db: &Path, principal_id: &str) -> BenchResult<()> {
    let options = SqliteConnectOptions::new()
        .filename(db)
        .create_if_missing(false)
        .foreign_keys(true);
    let mut conn = SqliteConnection::connect_with(&options).await?;
    sqlx::query(
        "INSERT INTO principals(principal_id, created_at_ms, disabled_at_ms) VALUES (?, ?, NULL)",
    )
    .bind(principal_id.as_bytes())
    .bind(unix_now_ms()?)
    .execute(&mut conn)
    .await?;
    conn.close().await?;
    Ok(())
}

async fn durable_row_counts(db: &Path) -> BenchResult<BTreeMap<String, i64>> {
    let tables = [
        "uploads",
        "upload_admission_reservations",
        "jobs",
        "job_effects",
        "physical_blobs",
        "resource_bindings",
        "projects",
        "documents",
        "upload_consumptions",
        "revision_identity_bindings",
        "authz_documents",
        "authz_principal_grants",
        "authz_audit_events",
    ];
    let options = SqliteConnectOptions::new()
        .filename(db)
        .create_if_missing(false)
        .foreign_keys(true);
    let mut conn = SqliteConnection::connect_with(&options).await?;
    let mut counts = BTreeMap::new();
    for table in tables {
        let exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?")
                .bind(table)
                .fetch_one(&mut conn)
                .await?;
        if exists == 1 {
            let sql = format!("SELECT COUNT(*) FROM \"{table}\"");
            let count: i64 = sqlx::query_scalar(&sql).fetch_one(&mut conn).await?;
            counts.insert(table.to_owned(), count);
        }
    }
    conn.close().await?;
    Ok(counts)
}

fn cleanup_case(root: &Path) {
    let _ = fs::remove_dir_all(root);
}

async fn run_fixture(fixture_id: &str, fixture_path: &Path) -> BenchResult<FixtureReceipt> {
    let overall = Instant::now();
    let cpu_start = proc_cpu_ticks();
    let rss_start = proc_rss_bytes("VmRSS:");
    let hz = clock_ticks_per_second();
    let (fixture_sha256, fixture_bytes) = sha256_file(fixture_path).await?;
    if fixture_bytes == 0 || fixture_bytes > MAX_SOURCE_BYTES {
        return Err(format!("fixture {fixture_id} outside benchmark size envelope").into());
    }

    let case_root = env::temp_dir().join(format!(
        "chaptera-cloud-link-bench-{}-{fixture_id}",
        std::process::id()
    ));
    cleanup_case(&case_root);
    fs::create_dir_all(&case_root)?;
    let db = case_root.join("chaptera.sqlite");

    SqliteMigrationRuntime::new(&db, Duration::from_secs(2))?
        .migrate_up()
        .await?;
    let principal_id = format!("principal-{fixture_id}");
    let tenant_id = format!("tenant-{fixture_id}");
    seed_principal(&db, &principal_id).await?;
    let sqlite_bytes_before = sqlite_storage_bytes(&db);

    let busy = Duration::from_secs(2);
    let source_repo = SqliteSourceIngressRepository::open(&db, 4, busy).await?;
    let admission = SqliteUploadAdmissionAuthority::open(
        &db,
        4,
        busy,
        UploadAdmissionConfig {
            principal_concurrent_cap: 2,
            tenant_concurrent_cap: 8,
            principal_bytes_cap: 512 * 1024 * 1024,
            tenant_bytes_cap: 2 * 1024 * 1024 * 1024,
            max_single_upload_bytes: 512 * 1024 * 1024,
            lease_duration: Duration::from_secs(3600),
            retention: Duration::from_secs(7 * 24 * 3600),
        },
    )
    .await?;
    let blob_repo = SqliteBlobBindingRepository::open(&db, 4, busy).await?;
    let provider = CountingProvider::default();
    let blob_store = BlobStoreService::new(
        Arc::new(provider.clone()),
        Arc::new(blob_repo.clone()),
        Arc::new(BenchIds::default()),
    );
    let job_queue = SqliteJobQueue::open(&db, 4, busy).await?;
    let validation_jobs = SourceValidationJobQueue::new(job_queue.clone());
    let projects = SqliteProjectPersistence::open(&db, 4, busy).await?;

    let mut phases = BTreeMap::new();
    let mut provider_phase_deltas = BTreeMap::new();
    let mut provider_checkpoint = provider.metrics();
    let now = unix_now_ms()?;
    let now_u64 = u64::try_from(now)?;
    let upload_id = format!("upload-{fixture_id}");
    let idempotency_key = format!("bench-issue-{fixture_id}");

    let phase = Instant::now();
    let candidate = plan_upload_candidate(
        MAX_SOURCE_BYTES,
        upload_id.clone(),
        IssueUploadRequest {
            tenant_id: tenant_id.clone(),
            principal_id: principal_id.clone(),
            expected_byte_len: fixture_bytes,
            declared_content_type: Some("application/x-mspublisher".to_owned()),
            idempotency_key: idempotency_key.clone(),
            now_ms: now_u64,
            expires_at_ms: now_u64 + 30 * 60 * 1000,
        },
    )?;
    let admission_request = UploadAdmissionRequest {
        reservation_id: upload_admission_reservation_id(&tenant_id, &idempotency_key)?,
        tenant_id: tenant_id.clone(),
        principal_id: principal_id.clone(),
        expected_bytes: i64::try_from(fixture_bytes)?,
        request_hash: candidate.request_hash.clone(),
    };
    admission.reserve(admission_request.clone(), now).await?;
    let issued = source_repo.issue_idempotent(candidate).await?;
    phases.insert("issue_and_admission".to_owned(), elapsed_ms(phase));
    record_provider_phase(
        &mut provider_phase_deltas,
        &mut provider_checkpoint,
        &provider,
        "issue_and_admission",
    );

    let phase = Instant::now();
    let mut fixture = tokio::fs::File::open(fixture_path).await?;
    let quarantine = blob_store
        .create_quarantine_streamed(&tenant_id, &upload_id, fixture_bytes, &mut fixture)
        .await?;
    phases.insert("streamed_upload".to_owned(), elapsed_ms(phase));
    record_provider_phase(
        &mut provider_phase_deltas,
        &mut provider_checkpoint,
        &provider,
        "streamed_upload",
    );

    let phase = Instant::now();
    let inspected = blob_store
        .inspect_quarantine_upload(&tenant_id, &upload_id)
        .await?
        .ok_or("quarantine object absent after streamed upload")?;
    if inspected != quarantine {
        return Err("quarantine object identity changed before completion".into());
    }
    let mut stored = issued.clone();
    stored.state = UploadState::StoredUnverified;
    stored.upload_generation = stored
        .upload_generation
        .checked_add(1)
        .ok_or("upload generation")?;
    stored.object_version = Some(inspected.storage_generation.clone());
    stored.object_etag = Some(inspected.etag.clone());
    stored.observed_byte_len = Some(inspected.byte_len);
    let stored = source_repo
        .compare_and_swap(&upload_id, issued.upload_generation, stored)
        .await?;

    let payload = SourceValidationJobPayloadV1 {
        schema_version: SOURCE_VALIDATION_JOB_PAYLOAD_SCHEMA_V1.to_owned(),
        tenant_id: tenant_id.clone(),
        upload_id: upload_id.clone(),
        principal_id: principal_id.clone(),
        expected_upload_generation: stored.upload_generation,
        admission_reservation_id: admission_request.reservation_id.clone(),
        admission_expected_bytes: admission_request.expected_bytes,
        admission_request_hash: admission_request.request_hash.clone(),
    };
    let enqueued = validation_jobs.enqueue(payload, unix_now_ms()?).await?;
    phases.insert("complete_and_enqueue".to_owned(), elapsed_ms(phase));
    record_provider_phase(
        &mut provider_phase_deltas,
        &mut provider_checkpoint,
        &provider,
        "complete_and_enqueue",
    );

    let phase = Instant::now();
    let lease = job_queue
        .claim_one(
            "cloud-link-bench",
            unix_now_ms()?,
            30_000,
            &[JobKind::Parse],
        )
        .await?
        .ok_or("source validation job was not claimable")?;
    if lease.job.job_id != enqueued.job_id {
        return Err("claimed unexpected source validation job".into());
    }
    let validation = AsyncSourceValidationRuntime::new(source_repo.clone(), blob_store.clone());
    let executor = SourceValidationJobExecutor::new(
        source_repo.clone(),
        Arc::new(validation),
        Arc::new(ConsumeAllScanner),
        admission.clone(),
    );
    let success = executor
        .execute(&lease.job, CancellationFlag::default())
        .await
        .map_err(|failure| {
            format!(
                "source validation job failed: {} retryable={}",
                failure.terminal_code, failure.retryable
            )
        })?;
    job_queue
        .publish_success(&lease, unix_now_ms()?, &success.effect_key)
        .await?;
    let validated = source_repo
        .get(&upload_id)
        .await?
        .ok_or("validated upload disappeared")?;
    if validated.state != UploadState::ValidatedDurable {
        return Err(format!("unexpected validated state: {:?}", validated.state).into());
    }
    phases.insert("validation_and_promotion".to_owned(), elapsed_ms(phase));
    record_provider_phase(
        &mut provider_phase_deltas,
        &mut provider_checkpoint,
        &provider,
        "validation_and_promotion",
    );

    let phase = Instant::now();
    let consume = ConsumeUploadRequest {
        tenant_id: tenant_id.clone(),
        upload_id: upload_id.clone(),
        expected_upload_generation: validated.upload_generation,
        workspace_id: format!("workspace-{fixture_id}"),
        name: format!("Cloud Link Bench {fixture_id}"),
        client_idempotency_id: format!("bench-create-{fixture_id}"),
        now_ms: u64::try_from(unix_now_ms()?)?,
    };
    let planned = plan_project_identity(&consume)?;
    let binding_id = validated
        .durable_binding_id
        .clone()
        .ok_or("validated upload has no durable binding")?;
    let source_sha = validated
        .canonical_sha256
        .clone()
        .ok_or("validated upload has no canonical sha")?;

    let root = repo_root();
    let baseline_temp = case_root.join("baseline-temp");
    fs::create_dir_all(&baseline_temp)?;
    let baseline = IsolatedSourceBaselineProducer::new(
        SourceBaselineProducerConfig {
            isolation_python: PathBuf::from(
                env::var("PYTHON").unwrap_or_else(|_| "python3".to_owned()),
            ),
            isolation_harness: root.join("tools/migration_pdf_worker_isolation.py"),
            worker_binary: PathBuf::from(env!("CARGO_BIN_EXE_chaptera")),
            worker_wall_timeout: Duration::from_secs(30),
            worker_address_space_mb: 1024,
            worker_cpu_seconds: 20,
            worker_open_files: 64,
            worker_output_file_mb: 32,
            temp_root: baseline_temp,
        },
        blob_store.clone(),
    )?
    .produce(
        &tenant_id,
        &binding_id,
        &source_sha,
        fixture_bytes,
        &planned.document_id,
    )
    .await?;
    phases.insert("baseline_materialization".to_owned(), elapsed_ms(phase));
    record_provider_phase(
        &mut provider_phase_deltas,
        &mut provider_checkpoint,
        &provider,
        "baseline_materialization",
    );

    let phase = Instant::now();
    let project = projects
        .create_project_from_upload(consume, baseline)
        .await?;
    phases.insert("project_genesis_commit".to_owned(), elapsed_ms(phase));
    record_provider_phase(
        &mut provider_phase_deltas,
        &mut provider_checkpoint,
        &provider,
        "project_genesis_commit",
    );

    let durable_row_counts = durable_row_counts(&db).await?;
    let provider_metrics = provider.metrics();
    let sqlite_bytes_after = sqlite_storage_bytes(&db);

    projects.close().await;
    job_queue.close().await;
    blob_repo.close().await;
    admission.close().await;
    source_repo.close().await;

    let cpu_end = proc_cpu_ticks();
    let rss_end = proc_rss_bytes("VmRSS:");
    let peak_rss = proc_rss_bytes("VmHWM:");

    let receipt = FixtureReceipt {
        fixture_id: fixture_id.to_owned(),
        fixture_file: fixture_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("<unknown>")
            .to_owned(),
        fixture_sha256,
        fixture_bytes,
        total_wall_ms: elapsed_ms(overall),
        phase_wall_ms: phases,
        logical_http_round_trips: 4,
        measured_http_request_count: None,
        source_payload_client_bytes_sent: fixture_bytes,
        measured_http_control_bytes: None,
        provider: provider_metrics,
        provider_phase_deltas,
        durable_row_counts,
        sqlite_bytes_before,
        sqlite_bytes_after,
        sqlite_storage_delta_bytes: i64::try_from(sqlite_bytes_after)
            .unwrap_or(i64::MAX)
            .saturating_sub(i64::try_from(sqlite_bytes_before).unwrap_or(i64::MAX)),
        parent_process_cpu_ms: cpu_delta_ms(cpu_start, cpu_end, hz),
        parent_process_rss_start_bytes: rss_start,
        parent_process_rss_end_bytes: rss_end,
        parent_process_peak_rss_bytes: peak_rss,
        source_validation_jobs_enqueued: 1,
        source_validation_jobs_executed: 1,
        project_id: project.project_id,
        document_id: project.document_id,
        genesis_revision_id: project.genesis_revision_id,
        canonical_source_binding_id: binding_id,
        canonical_source_sha256: source_sha,
    };

    cleanup_case(&case_root);
    Ok(receipt)
}

#[tokio::test]
async fn cloud_link_first_bind_measurement_receipt() -> BenchResult<()> {
    let Some(fixtures_root) = env::var_os("CHAPTERA_CLOUD_LINK_BENCH_FIXTURES") else {
        eprintln!(
            "CHAPTERA_CLOUD_LINK_BENCH_FIXTURES not set; dedicated CLOUD-LINK-BENCH-01 workflow owns execution"
        );
        return Ok(());
    };
    let output_path = env::var_os("CHAPTERA_CLOUD_LINK_BENCH_RECEIPT")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("out/cloud-link-bench-01.json"));
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let fixture_root = PathBuf::from(fixtures_root);
    let fixture_specs = [
        ("f0-simple", "Simple.pub"),
        ("f1-newsletter", "SampleNewsletter.pub"),
        ("f2-brochure", "SampleBrochure.pub"),
    ];

    let mut runs = Vec::new();
    for (fixture_id, filename) in fixture_specs {
        runs.push(run_fixture(fixture_id, &fixture_root.join(filename)).await?);
    }

    let receipt = BenchReceipt {
        protocol_version: "chaptera.cloud-link-first-bind-bench.v0",
        build_sha: env::var("GITHUB_SHA").unwrap_or_else(|_| "local".to_owned()),
        fixture_source_commit: FIXTURE_SOURCE_COMMIT,
        harness_scope: "production SQLite/BlobStore/SourceIngress validation/baseline/project authorities; deterministic in-process object provider and consume-all scanner; HTTP/AuthN framing and external S3/ClamD are not measured",
        http_layer_measured: false,
        scanner_profile: "cloud-link-bench-consume-all-v1",
        price_model_applied: false,
        runs,
    };

    fs::write(&output_path, serde_json::to_vec_pretty(&receipt)?)?;
    println!("CLOUD_LINK_BENCH_RECEIPT={}", output_path.display());
    Ok(())
}
