use std::{
    collections::BTreeMap,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use chaptera_server::{
    blob_store::{
        BlobBindingRepository, BlobIdGenerator, BlobProvider, BlobStoreError, BlobStoreService,
        GrantOperation, ProviderCapabilities, ProviderError, ProviderErrorKind, ProviderGrant,
        ProviderGrantRequest, ProviderObjectMetadata, ResourceKind,
    },
    derived_artifacts::SqliteDerivedArtifactStore,
    export_executor::{
        EXPORT_JOB_PAYLOAD_SCHEMA_V1, ExactRevisionEditableExporter, ExactRevisionStateProvider,
        ExportExecutorError, ExportJobPayloadV1, ExportPublishAuthFuture, ExportPublishAuthorizer,
        IDML_BOUNDED_EDITABLE_PROFILE, PublishedExportJobExecutor,
    },
    export_publication::SqliteExportPublicationStore,
    job_queue::{EnqueueRequest, JobKind, JobStatus, SqliteJobQueue},
    job_worker::{CancellationFlag, JobExecutor, WorkerControl, WorkerLoop, WorkerLoopConfig},
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
                "vertical provider received a different exact revision identity",
            ));
        }
        Ok(self.state.clone())
    }
}

#[derive(Default)]
struct SequenceIds {
    physical: AtomicU64,
    binding: AtomicU64,
}

impl BlobIdGenerator for SequenceIds {
    fn next_physical_blob_id(&self) -> Result<String, BlobStoreError> {
        Ok(format!(
            "physical-export-{}",
            self.physical.fetch_add(1, Ordering::SeqCst) + 1
        ))
    }

    fn next_binding_id(&self) -> Result<String, BlobStoreError> {
        Ok(format!(
            "binding-export-{}",
            self.binding.fetch_add(1, Ordering::SeqCst) + 1
        ))
    }
}

#[derive(Clone)]
struct StoredObject {
    generation: String,
    bytes: Vec<u8>,
}

#[derive(Default)]
struct MemoryBlobProvider {
    objects: Mutex<BTreeMap<String, StoredObject>>,
}

impl MemoryBlobProvider {
    fn bytes(&self, locator: &str) -> Vec<u8> {
        self.objects
            .lock()
            .unwrap()
            .get(locator)
            .expect("provider object exists")
            .bytes
            .clone()
    }

    fn object_count(&self) -> usize {
        self.objects.lock().unwrap().len()
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
        if u64::try_from(bytes.len()).ok() != Some(expected_byte_len) {
            return Err(ProviderError::new(
                ProviderErrorKind::Other,
                "memory_length_mismatch",
            ));
        }

        let mut objects = self.objects.lock().unwrap();
        if objects.contains_key(object_locator) {
            return Err(ProviderError::new(
                ProviderErrorKind::AlreadyExists,
                "memory_already_exists",
            ));
        }
        let generation = format!("gen-{:x}", Sha256::digest(&bytes));
        objects.insert(
            object_locator.to_owned(),
            StoredObject {
                generation: generation.clone(),
                bytes,
            },
        );
        Ok(ProviderObjectMetadata {
            generation,
            byte_len: expected_byte_len,
            etag: "memory-etag".to_owned(),
        })
    }

    async fn head_exact(
        &self,
        object_locator: &str,
    ) -> Result<Option<ProviderObjectMetadata>, ProviderError> {
        Ok(self
            .objects
            .lock()
            .unwrap()
            .get(object_locator)
            .map(|object| ProviderObjectMetadata {
                generation: object.generation.clone(),
                byte_len: u64::try_from(object.bytes.len()).unwrap(),
                etag: "memory-etag".to_owned(),
            }))
    }

    async fn open_read(
        &self,
        object_locator: &str,
        generation: &str,
    ) -> Result<Box<dyn AsyncRead + Unpin + Send>, ProviderError> {
        let objects = self.objects.lock().unwrap();
        let object = objects
            .get(object_locator)
            .ok_or_else(|| ProviderError::new(ProviderErrorKind::NotFound, "memory_not_found"))?;
        if object.generation != generation {
            return Err(ProviderError::new(
                ProviderErrorKind::NotFound,
                "memory_generation_mismatch",
            ));
        }
        Ok(Box::new(Cursor::new(object.bytes.clone())))
    }

    async fn delete_exact(
        &self,
        object_locator: &str,
        generation: &str,
    ) -> Result<(), ProviderError> {
        let mut objects = self.objects.lock().unwrap();
        match objects.get(object_locator) {
            Some(object) if object.generation == generation => {
                objects.remove(object_locator);
                Ok(())
            }
            Some(_) => Err(ProviderError::new(
                ProviderErrorKind::NotFound,
                "memory_generation_mismatch",
            )),
            None => Err(ProviderError::new(
                ProviderErrorKind::NotFound,
                "memory_not_found",
            )),
        }
    }

    async fn issue_grant(
        &self,
        request: &ProviderGrantRequest,
    ) -> Result<ProviderGrant, ProviderError> {
        let operation = match request.operation {
            GrantOperation::UploadCreateOnly => "upload",
            GrantOperation::Download => "download",
        };
        Ok(ProviderGrant {
            opaque_url: format!("memory://{operation}/{}", request.object_locator),
            expires_at_ms: request.expires_at_ms,
        })
    }
}

struct AllowPublish {
    calls: AtomicU64,
}

impl AllowPublish {
    fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
        }
    }
}

impl ExportPublishAuthorizer for AllowPublish {
    fn authorize<'a>(
        &'a self,
        _job: &'a chaptera_server::job_queue::JobRecord,
        payload: &'a ExportJobPayloadV1,
    ) -> ExportPublishAuthFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if payload.requesting_principal_id != "principal:export-vertical" {
                return Err(ExportExecutorError::new(
                    "export_publish_unauthorized",
                    "requesting principal identity was not preserved into worker execution",
                ));
            }
            Ok(())
        })
    }
}

fn now_ms() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

fn fixture_path() -> PathBuf {
    std::env::var_os("CHAPTERA_EXPORT_FIXTURE")
        .map(PathBuf::from)
        .expect("CHAPTERA_EXPORT_FIXTURE must point to the pinned real PUB fixture")
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn temp_db() -> PathBuf {
    std::env::temp_dir().join(format!(
        "chaptera-export-vertical-{}-{}.sqlite",
        std::process::id(),
        now_ms()
    ))
}

fn cleanup(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}

fn quota_config() -> QuotaConfig {
    QuotaConfig {
        shared_capacity: 4,
        semantic_headroom: 1,
        export_cap: 2,
        background_cap: 2,
    }
}

fn worker_config() -> WorkerLoopConfig {
    WorkerLoopConfig {
        owner: "export-vertical-worker".to_owned(),
        allowed_kinds: vec![JobKind::Export],
        lease_duration: Duration::from_secs(2),
        heartbeat_interval: Duration::from_millis(100),
        idle_poll_interval: Duration::from_millis(10),
        drain_timeout: Duration::from_secs(1),
    }
}

fn build_materialized_state() -> (ExactRevisionMaterializedState, ExportJobPayloadV1) {
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
        .expect("fixture exposes editable story");
    let before = session.graph().stories[&story_id].text.clone();
    session
        .replace_story_text(story_id, format!("{before} Chaptera worker vertical"))
        .expect("apply real edit");
    let project = session.project();
    let project_sha256 = sha256_hex(&serde_json::to_vec(&project).unwrap());

    let canonical_revision = "c".repeat(64);
    let service_revision = "service-rev-export-vertical".to_owned();
    let payload = ExportJobPayloadV1 {
        schema_version: EXPORT_JOB_PAYLOAD_SCHEMA_V1.to_owned(),
        tenant_id: "tenant:export-vertical".to_owned(),
        document_id: "document:export-vertical".to_owned(),
        requesting_principal_id: "principal:export-vertical".to_owned(),
        exact_revision_id: service_revision.clone(),
        canonical_authoring_revision_id: canonical_revision.clone(),
        target_profile: IDML_BOUNDED_EDITABLE_PROFILE.to_owned(),
        layout_environment_id: format!("sha256:{}", "d".repeat(64)),
    };
    let state = ExactRevisionMaterializedState {
        receipt: ExactRevisionMaterializationReceipt {
            schema_version: MATERIALIZATION_RECEIPT_SCHEMA_V1.to_owned(),
            tenant_id: payload.tenant_id.clone(),
            document_id: payload.document_id.clone(),
            source_binding_id: "binding:source-fixture".to_owned(),
            source_sha256,
            baseline_revision_id: "service-rev-baseline".to_owned(),
            baseline_cursor: 0,
            requested_revision_id: service_revision,
            canonical_revision_schema_version: CANONICAL_SCHEMA.to_owned(),
            canonical_authoring_revision_id: canonical_revision,
            replayed_edges: 1,
            project_sha256,
            authoring_root_hash: None,
            project,
        },
        source_bytes,
    };
    (state, payload)
}

#[tokio::test]
#[ignore = "requires pinned real PUB fixture from CLOUD-EXPORT-EXECUTOR-01 acceptance"]
async fn real_queue_quota_executor_blob_publication_terminal_vertical() {
    let db = temp_db();
    cleanup(&db);
    SqliteMigrationRuntime::new(&db, Duration::from_secs(2))
        .unwrap()
        .migrate_up()
        .await
        .unwrap();

    let queue = SqliteJobQueue::open(&db, 4, Duration::from_secs(2))
        .await
        .unwrap();
    let quota = SqliteQuotaAuthority::open(&db, 4, Duration::from_secs(2), quota_config())
        .await
        .unwrap();
    let admission = Arc::new(
        SqliteQuotaJobAdmission::new(
            quota.clone(),
            SqliteQuotaAdmissionConfig::conservative_v0(Duration::from_secs(2)),
        )
        .unwrap(),
    );

    let provider = Arc::new(MemoryBlobProvider::default());
    let blob_repo = Arc::new(
        SqliteBlobBindingRepository::open(&db, 4, Duration::from_secs(2))
            .await
            .unwrap(),
    );
    let blob_store = BlobStoreService::new(
        provider.clone(),
        blob_repo.clone(),
        Arc::new(SequenceIds::default()),
    );
    let artifacts = SqliteDerivedArtifactStore::open(&db, 4, Duration::from_secs(2))
        .await
        .unwrap();
    let publications = SqliteExportPublicationStore::open(&db, 4, Duration::from_secs(2))
        .await
        .unwrap();

    let (state, payload) = build_materialized_state();
    let producer = Arc::new(ExactRevisionEditableExporter::new(Arc::new(
        FixtureStateProvider { state },
    )));
    let authorizer = Arc::new(AllowPublish::new());
    let executor = Arc::new(PublishedExportJobExecutor::new(
        producer,
        blob_store,
        artifacts,
        publications.clone(),
        authorizer.clone(),
    ));

    let job_id = "job-export-vertical";
    queue
        .enqueue(EnqueueRequest {
            job_id: job_id.to_owned(),
            tenant_id: payload.tenant_id.clone(),
            job_kind: JobKind::Export,
            payload_schema_version: 1,
            payload: serde_json::to_vec(&payload).unwrap(),
            idempotency_key: "idem-export-vertical".to_owned(),
            max_attempts: 3,
            now_ms: now_ms(),
        })
        .await
        .unwrap();

    let control = WorkerControl::default();
    let worker = WorkerLoop::new(
        queue.clone(),
        executor.clone(),
        admission,
        control.clone(),
        worker_config(),
    )
    .unwrap();
    let worker_task = tokio::spawn(async move { worker.run().await });

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let terminal = loop {
        let job = queue.get(job_id).await.unwrap().expect("job exists");
        if job.status.terminal() {
            break job;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "export vertical did not reach terminal state"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    control.request_drain();
    let worker_receipt = worker_task.await.unwrap().unwrap();

    assert_eq!(terminal.status, JobStatus::Succeeded, "{terminal:?}");
    assert_eq!(worker_receipt.succeeded, 1);
    assert_eq!(worker_receipt.failed, 0);
    assert_eq!(worker_receipt.cancelled, 0);
    assert!(authorizer.calls.load(Ordering::SeqCst) >= 2);

    let visible = publications
        .get_visible_by_job(&payload.tenant_id, job_id)
        .await
        .unwrap()
        .expect("successful queue job makes matching prepared publication visible");
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

    let artifact_binding = blob_repo
        .get_binding(&visible.input.artifact_binding_id)
        .await
        .unwrap()
        .expect("artifact binding exists");
    let loss_binding = blob_repo
        .get_binding(&visible.input.loss_binding_id)
        .await
        .unwrap()
        .expect("loss binding exists");
    assert_eq!(artifact_binding.resource_kind, ResourceKind::ExportArtifact);
    assert_eq!(loss_binding.resource_kind, ResourceKind::ExportArtifact);

    let artifact_physical = blob_repo
        .get_physical(&artifact_binding.physical_blob_id)
        .await
        .unwrap()
        .expect("artifact physical exists");
    let artifact_bytes = provider.bytes(&artifact_physical.object_locator);
    assert!(
        artifact_bytes.starts_with(b"PK"),
        "published IDML is a ZIP package"
    );
    assert_eq!(
        format!("sha256:{}", sha256_hex(&artifact_bytes)),
        visible.input.artifact_content_hash
    );
    assert!(provider.object_count() >= 2);

    let usage = quota.usage(&payload.tenant_id, now_ms()).await.unwrap();
    assert_eq!(
        usage.export, 0,
        "WorkerLoop releases export quota after terminal success"
    );

    // A duplicate executor delivery over the same durable job must converge to
    // the same logical effect/publication even if BlobStore allocates new
    // logical binding handles over the already deduped physical bytes.
    let retry = executor
        .execute(&terminal, CancellationFlag::default())
        .await
        .expect("duplicate delivery converges");
    assert_eq!(retry.effect_key, visible.effect_key);
    let visible_after_retry = publications
        .get_visible_by_job(&payload.tenant_id, job_id)
        .await
        .unwrap()
        .expect("publication remains visible");
    assert_eq!(visible_after_retry, visible);
    assert_eq!(
        provider.object_count(),
        2,
        "physical bytes are deduped on retry"
    );

    publications.close().await;
    quota.close().await;
    blob_repo.close().await;
    queue.close().await;
    cleanup(&db);
}
