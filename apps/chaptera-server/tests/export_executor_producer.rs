use std::{
    collections::BTreeMap,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use chaptera_server::{
    blob_store::{
        BlobIdGenerator, BlobProvider, BlobStoreError, BlobStoreService, ProviderCapabilities,
        ProviderError, ProviderErrorKind, ProviderGrant, ProviderGrantRequest,
        ProviderObjectMetadata,
    },
    derived_artifacts::{DerivedArtifactFenceV1, SqliteDerivedArtifactStore},
    export_executor::{
        EXPORT_JOB_PAYLOAD_SCHEMA_V1, ExactRevisionEditableExporter, ExactRevisionStateProvider,
        ExportExecutorError, ExportJobPayloadV1, ExportPublicationCommitFuture,
        ExportPublicationCommitter, ExportPublishAuthFuture, ExportPublishAuthorizer,
        IDML_BOUNDED_EDITABLE_PROFILE, PublishedExportJobExecutor,
    },
    export_publication::{
        ExportPublicationInputV1, ExportPublicationPrepareOutcomeV1, SqliteExportPublicationStore,
    },
    job_executor_registry::JobExecutorRegistry,
    job_queue::{EnqueueRequest, JobKind, JobRecord, JobStatus, SqliteJobQueue},
    job_worker::{JobExecutor, WorkerControl, WorkerLoop, WorkerLoopConfig},
    quota_admission::{SqliteQuotaAdmissionConfig, SqliteQuotaJobAdmission},
    quota_store::{QuotaConfig, SqliteQuotaAuthority},
    revision_materializer::{
        ExactRevisionMaterializationReceipt, ExactRevisionMaterializedState,
        MATERIALIZATION_RECEIPT_SCHEMA_V1,
    },
    schema_migration::SqliteMigrationRuntime,
    sqlite_blob_metadata::SqliteBlobBindingRepository,
};
use pub_editor::{Sha256Digest, open_mature_0x2c_editor};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt};

const CANONICAL_SCHEMA: &str = "chaptera.cdm.authoring-revision.v1";

#[derive(Clone)]
struct FixtureStateProvider {
    state: ExactRevisionMaterializedState,
}

#[async_trait]
impl ExactRevisionStateProvider for FixtureStateProvider {
    async fn materialize_state(
        &self,
        tenant_id: &str,
        document_id: &str,
        exact_revision_id: &str,
    ) -> Result<ExactRevisionMaterializedState, ExportExecutorError> {
        if tenant_id != self.state.receipt.tenant_id
            || document_id != self.state.receipt.document_id
            || exact_revision_id != self.state.receipt.requested_revision_id
        {
            return Err(ExportExecutorError::new(
                "fixture_identity_mismatch",
                "test provider received a different exact revision identity",
            ));
        }
        Ok(self.state.clone())
    }
}

#[derive(Clone)]
struct StoredObject {
    bytes: Vec<u8>,
    generation: String,
    etag: String,
}

#[derive(Default)]
struct MemoryBlobProvider {
    objects: Mutex<BTreeMap<String, StoredObject>>,
}

impl MemoryBlobProvider {
    fn object_count(&self) -> usize {
        self.objects.lock().expect("memory provider lock").len()
    }
}

#[async_trait]
impl BlobProvider for MemoryBlobProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            hard_create_only: true,
            hard_exact_or_max_upload_size: true,
            signed_content_type: true,
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
        input
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| ProviderError::new(ProviderErrorKind::Other, "memory_read_failed"))?;
        if bytes.len() as u64 != expected_byte_len {
            return Err(ProviderError::new(
                ProviderErrorKind::Other,
                "memory_length_mismatch",
            ));
        }

        let mut objects = self.objects.lock().expect("memory provider lock");
        if objects.contains_key(object_locator) {
            return Err(ProviderError::new(
                ProviderErrorKind::AlreadyExists,
                "memory_already_exists",
            ));
        }
        let ordinal = objects.len() + 1;
        let object = StoredObject {
            bytes,
            generation: format!("gen-{ordinal}"),
            etag: format!("etag-{ordinal}"),
        };
        let metadata = ProviderObjectMetadata {
            generation: object.generation.clone(),
            byte_len: expected_byte_len,
            etag: object.etag.clone(),
        };
        objects.insert(object_locator.to_owned(), object);
        Ok(metadata)
    }

    async fn head_exact(
        &self,
        object_locator: &str,
    ) -> Result<Option<ProviderObjectMetadata>, ProviderError> {
        Ok(self
            .objects
            .lock()
            .expect("memory provider lock")
            .get(object_locator)
            .map(|object| ProviderObjectMetadata {
                generation: object.generation.clone(),
                byte_len: object.bytes.len() as u64,
                etag: object.etag.clone(),
            }))
    }

    async fn open_read(
        &self,
        object_locator: &str,
        generation: &str,
    ) -> Result<Box<dyn AsyncRead + Unpin + Send>, ProviderError> {
        let object = self
            .objects
            .lock()
            .expect("memory provider lock")
            .get(object_locator)
            .cloned()
            .ok_or_else(|| {
                ProviderError::new(ProviderErrorKind::NotFound, "memory_object_missing")
            })?;
        if object.generation != generation {
            return Err(ProviderError::new(
                ProviderErrorKind::NotFound,
                "memory_generation_mismatch",
            ));
        }
        Ok(Box::new(Cursor::new(object.bytes)))
    }

    async fn delete_exact(
        &self,
        object_locator: &str,
        generation: &str,
    ) -> Result<(), ProviderError> {
        let mut objects = self.objects.lock().expect("memory provider lock");
        let object = objects.get(object_locator).ok_or_else(|| {
            ProviderError::new(ProviderErrorKind::NotFound, "memory_object_missing")
        })?;
        if object.generation != generation {
            return Err(ProviderError::new(
                ProviderErrorKind::NotFound,
                "memory_generation_mismatch",
            ));
        }
        objects.remove(object_locator);
        Ok(())
    }

    async fn issue_grant(
        &self,
        request: &ProviderGrantRequest,
    ) -> Result<ProviderGrant, ProviderError> {
        Ok(ProviderGrant {
            opaque_url: format!("memory://{}", request.object_locator),
            expires_at_ms: request.expires_at_ms,
        })
    }
}

#[derive(Default)]
struct SequenceBlobIds {
    physical: AtomicUsize,
    binding: AtomicUsize,
}

impl BlobIdGenerator for SequenceBlobIds {
    fn next_physical_blob_id(&self) -> Result<String, BlobStoreError> {
        Ok(format!(
            "blob-{}",
            self.physical.fetch_add(1, Ordering::SeqCst) + 1
        ))
    }

    fn next_binding_id(&self) -> Result<String, BlobStoreError> {
        Ok(format!(
            "binding-{}",
            self.binding.fetch_add(1, Ordering::SeqCst) + 1
        ))
    }
}

#[derive(Default)]
struct CountingAuthorizer {
    calls: AtomicUsize,
}

impl CountingAuthorizer {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ExportPublishAuthorizer for CountingAuthorizer {
    fn authorize<'a>(
        &'a self,
        job: &'a JobRecord,
        payload: &'a ExportJobPayloadV1,
    ) -> ExportPublishAuthFuture<'a> {
        Box::pin(async move {
            if job.job_kind != JobKind::Export
                || job.tenant_id != payload.tenant_id
                || payload.requesting_principal_id != "principal:fixture"
            {
                return Err(ExportExecutorError::new(
                    "export_publish_unauthorized",
                    "test authorization boundary rejected mismatched job identity",
                ));
            }
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

struct TestPublicationCommitter {
    store: SqliteExportPublicationStore,
    calls: AtomicUsize,
}

impl TestPublicationCommitter {
    fn new(store: SqliteExportPublicationStore) -> Self {
        Self {
            store,
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ExportPublicationCommitter for TestPublicationCommitter {
    fn commit_authorized<'a>(
        &'a self,
        _job: &'a JobRecord,
        payload: &'a ExportJobPayloadV1,
        input: ExportPublicationInputV1,
        created_at_ms: i64,
    ) -> ExportPublicationCommitFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let store = self.store.clone();
        Box::pin(async move {
            if payload.requesting_principal_id != "principal:fixture" {
                return Err(ExportExecutorError::new(
                    "export_publish_unauthorized",
                    "test publication barrier lost requesting principal identity",
                ));
            }
            let prepared = store
                .prepare(input, created_at_ms)
                .await
                .map_err(|error| ExportExecutorError::new(error.code, error.message))?;
            Ok(match prepared {
                ExportPublicationPrepareOutcomeV1::Prepared(record)
                | ExportPublicationPrepareOutcomeV1::AlreadyPrepared(record) => record.effect_key,
            })
        })
    }
}

fn fixture_path() -> PathBuf {
    std::env::var_os("CHAPTERA_EXPORT_FIXTURE")
        .map(PathBuf::from)
        .expect("CHAPTERA_EXPORT_FIXTURE must point to the pinned real PUB fixture")
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn exact_edited_fixture_state() -> (ExactRevisionMaterializedState, ExportJobPayloadV1) {
    let source_bytes = fs::read(fixture_path()).expect("read pinned SampleNewsletter.pub");
    let source_sha256 = sha256_hex(&source_bytes);
    assert_eq!(
        source_sha256,
        "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf"
    );

    let source_hash = Sha256Digest::from_str(&source_sha256).expect("fixture SHA-256");
    let mut session =
        open_mature_0x2c_editor(&source_bytes, source_hash).expect("open canonical real PUB");

    let story_id = session
        .graph()
        .stories
        .keys()
        .copied()
        .find(|story_id| session.can_replace_story_text(*story_id).is_ok())
        .expect("SampleNewsletter must expose one bounded editable story");
    let before = session
        .graph()
        .stories
        .get(&story_id)
        .expect("chosen story exists")
        .text
        .clone();
    session
        .replace_story_text(story_id, format!("{before} Chaptera export proof"))
        .expect("apply real story edit");
    let project = session.project();
    assert!(
        !project.operations.is_empty(),
        "fixture project must contain a real edit"
    );

    let project_json = serde_json::to_vec(&project).expect("serialize fixture project");
    let project_sha256 = sha256_hex(&project_json);
    let canonical_revision = "c".repeat(64);
    let service_revision = "service-rev-1".to_owned();

    let state = ExactRevisionMaterializedState {
        receipt: ExactRevisionMaterializationReceipt {
            schema_version: MATERIALIZATION_RECEIPT_SCHEMA_V1.to_owned(),
            tenant_id: "tenant:fixture".to_owned(),
            document_id: "document:fixture".to_owned(),
            source_binding_id: "binding:fixture".to_owned(),
            source_sha256,
            baseline_revision_id: "service-rev-0".to_owned(),
            baseline_cursor: 0,
            requested_revision_id: service_revision.clone(),
            canonical_revision_schema_version: CANONICAL_SCHEMA.to_owned(),
            canonical_authoring_revision_id: canonical_revision.clone(),
            replayed_edges: 1,
            project_sha256,
            authoring_root_hash: None,
            project,
        },
        source_bytes,
    };

    let payload = ExportJobPayloadV1 {
        schema_version: EXPORT_JOB_PAYLOAD_SCHEMA_V1.to_owned(),
        tenant_id: "tenant:fixture".to_owned(),
        document_id: "document:fixture".to_owned(),
        requesting_principal_id: "principal:fixture".to_owned(),
        exact_revision_id: service_revision,
        canonical_authoring_revision_id: canonical_revision,
        target_profile: IDML_BOUNDED_EDITABLE_PROFILE.to_owned(),
        layout_environment_id: format!("sha256:{}", "d".repeat(64)),
    };
    (state, payload)
}

fn now_ms() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after UNIX epoch")
            .as_millis(),
    )
    .expect("milliseconds fit i64")
}

fn temp_db(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after UNIX epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "chaptera-export-{label}-{}-{nonce}.sqlite",
        std::process::id()
    ))
}

fn cleanup_db(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}

#[tokio::test]
#[ignore = "requires pinned real PUB fixture from CLOUD-EXPORT-EXECUTOR-01 acceptance"]
async fn exact_edited_real_pub_produces_deterministic_idml_and_loss_report() {
    let (state, payload) = exact_edited_fixture_state();
    let exporter = ExactRevisionEditableExporter::new(Arc::new(FixtureStateProvider { state }));

    let first = exporter
        .produce(&payload)
        .await
        .expect("produce exact edited IDML");
    let second = exporter
        .produce(&payload)
        .await
        .expect("exact retry produces the same edited IDML");

    assert!(!first.artifact_bytes.is_empty());
    assert!(
        first.artifact_bytes.starts_with(b"PK"),
        "IDML must be a ZIP package"
    );
    assert!(!first.loss_report_json.is_empty());
    let loss_json: serde_json::Value =
        serde_json::from_slice(&first.loss_report_json).expect("canonical LossReport JSON");
    assert!(loss_json.is_object());

    assert_eq!(first.artifact_bytes, second.artifact_bytes);
    assert_eq!(first.artifact_sha256, second.artifact_sha256);
    assert_eq!(first.loss_report_json, second.loss_report_json);
    assert_eq!(first.loss_report_sha256, second.loss_report_sha256);
    assert_eq!(first.exact_revision_id, payload.exact_revision_id);
    assert_eq!(
        first.canonical_authoring_revision_id,
        payload.canonical_authoring_revision_id
    );
    assert_eq!(first.layout_environment_id, payload.layout_environment_id);
}

#[tokio::test]
#[ignore = "requires pinned real PUB fixture from CLOUD-EXPORT-EXECUTOR-01 acceptance"]
async fn registered_export_executor_runs_real_worker_vertical_to_terminal_publication() {
    let (state, payload) = exact_edited_fixture_state();
    let project_sha256 = state.receipt.project_sha256.clone();
    let db = temp_db("worker-vertical");
    SqliteMigrationRuntime::new(&db, Duration::from_secs(2))
        .expect("migration runtime")
        .migrate_up()
        .await
        .expect("migrate full Chaptera SQLite schema");

    let queue = SqliteJobQueue::open(&db, 2, Duration::from_secs(2))
        .await
        .expect("open real SQLite job queue");
    let quota = SqliteQuotaAuthority::open(
        &db,
        2,
        Duration::from_secs(2),
        QuotaConfig {
            shared_capacity: 2,
            semantic_headroom: 1,
            export_cap: 1,
            background_cap: 1,
        },
    )
    .await
    .expect("open real SQLite quota authority");
    let artifacts = SqliteDerivedArtifactStore::open(&db, 2, Duration::from_secs(2))
        .await
        .expect("open real derived-artifact store");
    let publications = SqliteExportPublicationStore::open(&db, 2, Duration::from_secs(2))
        .await
        .expect("open real export-publication store");
    let blob_repo = SqliteBlobBindingRepository::open(&db, 2, Duration::from_secs(2))
        .await
        .expect("open real SQLite blob metadata");
    let provider = Arc::new(MemoryBlobProvider::default());
    let blob_store = BlobStoreService::new(
        provider.clone(),
        Arc::new(blob_repo.clone()),
        Arc::new(SequenceBlobIds::default()),
    );

    let producer = Arc::new(ExactRevisionEditableExporter::new(Arc::new(
        FixtureStateProvider { state },
    )));
    let authorizer = Arc::new(CountingAuthorizer::default());
    let publication_committer = Arc::new(TestPublicationCommitter::new(publications.clone()));
    let export_executor: Arc<dyn JobExecutor> = Arc::new(PublishedExportJobExecutor::new(
        producer,
        blob_store,
        artifacts.clone(),
        publication_committer.clone(),
        authorizer.clone(),
    ));
    let registry =
        JobExecutorRegistry::new(vec![(JobKind::Export, export_executor)]).expect("registry");
    registry
        .require_kinds(&[JobKind::Export])
        .expect("Export must be concretely registered");

    let admission = Arc::new(
        SqliteQuotaJobAdmission::new(
            quota.clone(),
            SqliteQuotaAdmissionConfig::conservative_v0(Duration::from_secs(2)),
        )
        .expect("quota admission"),
    );
    let control = WorkerControl::default();
    let worker = WorkerLoop::new(
        queue.clone(),
        Arc::new(registry),
        admission,
        control.clone(),
        WorkerLoopConfig {
            owner: "export-worker-proof".to_owned(),
            allowed_kinds: vec![JobKind::Export],
            lease_duration: Duration::from_secs(5),
            heartbeat_interval: Duration::from_millis(200),
            idle_poll_interval: Duration::from_millis(10),
            drain_timeout: Duration::from_secs(2),
        },
    )
    .expect("construct real worker loop");

    let job_id = "export-job-proof";
    queue
        .enqueue(EnqueueRequest {
            job_id: job_id.to_owned(),
            tenant_id: payload.tenant_id.clone(),
            job_kind: JobKind::Export,
            payload_schema_version: 1,
            payload: serde_json::to_vec(&payload).expect("serialize export payload"),
            idempotency_key: "export-proof-v1".to_owned(),
            max_attempts: 3,
            now_ms: now_ms(),
        })
        .await
        .expect("enqueue exact-revision export job");

    assert!(
        publications
            .get_visible_by_job(&payload.tenant_id, job_id)
            .await
            .expect("query publication before worker")
            .is_none(),
        "prepared export must not be visible before the queue terminal effect"
    );

    let worker_task = tokio::spawn(async move { worker.run().await });
    let mut final_job = None;
    for _ in 0..500 {
        let job = queue
            .get(job_id)
            .await
            .expect("read queued export")
            .expect("export job exists");
        if job.status.terminal() {
            final_job = Some(job);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    control.request_drain();
    let receipt = worker_task
        .await
        .expect("join export worker")
        .expect("worker loop completes cleanly");
    let final_job = final_job.expect("export job reaches terminal state");

    assert_eq!(final_job.status, JobStatus::Succeeded);
    assert_eq!(receipt.claimed, 1);
    assert_eq!(receipt.admitted, 1);
    assert_eq!(receipt.succeeded, 1);
    assert_eq!(receipt.failed, 0);
    assert_eq!(receipt.requeued, 0);
    assert_eq!(receipt.lease_lost, 0);
    assert_eq!(
        authorizer.calls(),
        1,
        "early authorization preflight must run once"
    );
    assert_eq!(
        publication_committer.calls(),
        1,
        "final logical publication must pass through the publication authority seam"
    );

    let visible = publications
        .get_visible_by_job(&payload.tenant_id, job_id)
        .await
        .expect("query visible publication")
        .expect("queue terminal effect makes prepared publication visible");
    assert_eq!(visible.input.exact_revision_id, payload.exact_revision_id);
    assert_eq!(
        visible.input.canonical_revision_id,
        payload.canonical_authoring_revision_id
    );
    assert_eq!(visible.input.target_profile, payload.target_profile);
    assert_eq!(
        visible.input.layout_environment_id,
        payload.layout_environment_id
    );

    let fence = DerivedArtifactFenceV1 {
        document_id: payload.document_id.clone(),
        service_revision_id: payload.exact_revision_id.clone(),
        canonical_revision_id: payload.canonical_authoring_revision_id.clone(),
        stage: "export".to_owned(),
        stage_version: payload.target_profile.clone(),
        environment_fingerprint: payload.layout_environment_id.clone(),
        input_fingerprint: format!("sha256:{project_sha256}"),
    };
    let derived = artifacts
        .resolve(&fence)
        .await
        .expect("resolve exact export artifact fence")
        .expect("derived export artifact exists");
    assert_eq!(derived.content_hash, visible.input.artifact_content_hash);
    assert_eq!(
        provider.object_count(),
        2,
        "worker must persist the edited artifact and canonical LossReport"
    );
    assert_eq!(
        quota
            .usage(&payload.tenant_id, now_ms())
            .await
            .expect("read quota after terminal completion")
            .export,
        0,
        "terminal completion must release the export reservation"
    );

    queue.close().await;
    quota.close().await;
    artifacts.close().await;
    publications.close().await;
    blob_repo.close().await;
    cleanup_db(&db);
}
