use std::{
    fmt,
    io::{self, Read},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngressError {
    pub code: &'static str,
    pub message: String,
}

impl IngressError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for IngressError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for IngressError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadPurpose {
    PubSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UploadState {
    Issued,
    StoredUnverified,
    Validating,
    ValidatedDurable,
    Consumed,
    Rejected,
    Expired,
}

impl UploadState {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Consumed | Self::Rejected | Self::Expired)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadRecord {
    pub upload_id: String,
    pub tenant_id: String,
    pub principal_id: String,
    pub purpose: UploadPurpose,
    pub expected_byte_len: u64,
    pub declared_content_type: Option<String>,
    pub physical_upload_ref: String,
    pub state: UploadState,
    pub upload_generation: u64,
    pub object_version: Option<String>,
    pub object_etag: Option<String>,
    pub observed_byte_len: Option<u64>,
    pub canonical_sha256: Option<String>,
    pub durable_binding_id: Option<String>,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub completed_at_ms: Option<u64>,
    pub terminal_code: Option<String>,
    pub idempotency_key: String,
    pub request_hash: String,
}

impl UploadRecord {
    pub fn quarantine_cleanup_eligible(&self) -> bool {
        matches!(
            self.state,
            UploadState::ValidatedDurable
                | UploadState::Consumed
                | UploadState::Rejected
                | UploadState::Expired
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadTransport {
    Direct { grant: String, expires_at_ms: u64 },
    Streamed { path: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueUploadRequest {
    pub tenant_id: String,
    pub principal_id: String,
    pub expected_byte_len: u64,
    pub declared_content_type: Option<String>,
    pub idempotency_key: String,
    pub now_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueUploadResult {
    pub upload: UploadRecord,
    pub transport: UploadTransport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteUploadResult {
    pub upload: UploadRecord,
    pub enqueue_validation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectMetadata {
    pub version: String,
    pub etag: String,
    pub byte_len: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableSourceBinding {
    pub binding_id: String,
    pub source_sha256: String,
    pub durable_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectCreateRequest {
    pub tenant_id: String,
    pub upload_id: String,
    pub durable_binding_id: String,
    pub source_sha256: String,
    pub workspace_id: String,
    pub name: String,
    pub client_idempotency_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectCreateResult {
    pub project_id: String,
    pub document_id: String,
    pub genesis_revision_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumeUploadRequest {
    pub tenant_id: String,
    pub upload_id: String,
    pub expected_upload_generation: u64,
    pub workspace_id: String,
    pub name: String,
    pub client_idempotency_id: String,
    pub now_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumptionReceipt {
    pub upload_id: String,
    pub tenant_id: String,
    pub idempotency_key: String,
    pub request_hash: String,
    pub committed_at_ms: u64,
    pub project: ProjectCreateResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationRejection {
    pub code: &'static str,
}

impl ValidationRejection {
    pub fn new(code: &'static str) -> Self {
        Self { code }
    }
}

pub trait UploadIdGenerator: Send + Sync {
    fn next_upload_id(&self) -> Result<String, IngressError>;
}

pub trait UploadRepository: Send + Sync {
    fn issue_idempotent(&self, candidate: UploadRecord) -> Result<UploadRecord, IngressError>;

    fn get(&self, upload_id: &str) -> Result<Option<UploadRecord>, IngressError>;

    fn compare_and_swap(
        &self,
        upload_id: &str,
        expected_generation: u64,
        next: UploadRecord,
    ) -> Result<UploadRecord, IngressError>;

    fn find_consumption(
        &self,
        tenant_id: &str,
        idempotency_key: &str,
    ) -> Result<Option<ConsumptionReceipt>, IngressError>;

    fn commit_consumption(
        &self,
        upload_id: &str,
        expected_generation: u64,
        receipt: ConsumptionReceipt,
    ) -> Result<ConsumptionReceipt, IngressError>;
}

pub trait QuarantineStore: Send + Sync {
    fn issue_transport(&self, upload: &UploadRecord) -> Result<UploadTransport, IngressError>;

    fn put_streamed_create_only(
        &self,
        physical_ref: &str,
        input: &mut dyn Read,
    ) -> Result<(), IngressError>;

    fn head(&self, physical_ref: &str) -> Result<Option<ObjectMetadata>, IngressError>;

    fn open_exact(
        &self,
        physical_ref: &str,
        version: &str,
        etag: &str,
    ) -> Result<Box<dyn Read + Send>, IngressError>;
}

pub trait SourceInspector: Send + Sync {
    fn inspect(&self, input: &mut dyn Read) -> Result<(), ValidationRejection>;
}

pub trait DurableSourceStore: Send + Sync {
    /// This operation must be idempotent for the same tenant + content hash.
    /// Returning success means immutable bytes and the tenant-safe binding are durable.
    fn promote_immutable(
        &self,
        tenant_id: &str,
        upload_id: &str,
        source_sha256: &str,
        input: &mut dyn Read,
    ) -> Result<DurableSourceBinding, IngressError>;
}

pub trait ProjectCreationPort: Send + Sync {
    /// Must be idempotent by tenant + client_idempotency_id.
    fn create_from_validated_source(
        &self,
        request: &ProjectCreateRequest,
    ) -> Result<ProjectCreateResult, IngressError>;
}

#[derive(Clone)]
pub struct SourceIngressService {
    max_source_bytes: u64,
    repo: Arc<dyn UploadRepository>,
    ids: Arc<dyn UploadIdGenerator>,
    quarantine: Arc<dyn QuarantineStore>,
    inspector: Arc<dyn SourceInspector>,
    durable_sources: Arc<dyn DurableSourceStore>,
    projects: Arc<dyn ProjectCreationPort>,
}

pub fn upload_admission_reservation_id(
    tenant_id: &str,
    idempotency_key: &str,
) -> Result<String, IngressError> {
    require_ident(tenant_id, "tenant_id")?;
    require_ident(idempotency_key, "idempotency_key")?;
    #[derive(Serialize)]
    struct Identity<'a> {
        protocol: &'static str,
        tenant_id: &'a str,
        idempotency_key: &'a str,
    }
    let digest = hash_serialized(&Identity {
        protocol: "chaptera.upload-admission-reservation.v1",
        tenant_id,
        idempotency_key,
    })?;
    Ok(format!("upload-admission-{digest}"))
}

pub fn plan_upload_candidate(
    max_source_bytes: u64,
    upload_id: String,
    request: IssueUploadRequest,
) -> Result<UploadRecord, IngressError> {
    if max_source_bytes == 0 {
        return Err(IngressError::new(
            "invalid_config",
            "max_source_bytes must be positive",
        ));
    }
    require_ident(&upload_id, "upload_id")?;
    require_ident(&request.tenant_id, "tenant_id")?;
    require_ident(&request.principal_id, "principal_id")?;
    require_ident(&request.idempotency_key, "idempotency_key")?;
    if request.expected_byte_len == 0 || request.expected_byte_len > max_source_bytes {
        return Err(IngressError::new(
            "upload_size_rejected",
            "expected_byte_len is outside the bounded PUB source class",
        ));
    }
    if request.expires_at_ms <= request.now_ms {
        return Err(IngressError::new(
            "invalid_expiry",
            "upload expiry must be after issue time",
        ));
    }
    if let Some(content_type) = &request.declared_content_type
        && (content_type.len() > 256 || content_type.chars().any(char::is_control))
    {
        return Err(IngressError::new(
            "invalid_content_type",
            "declared content type is not a bounded display hint",
        ));
    }

    let request_hash = issue_request_hash(&request)?;
    let physical_upload_ref = format!("quarantine/{}/{}", request.tenant_id, upload_id);

    Ok(UploadRecord {
        upload_id,
        tenant_id: request.tenant_id,
        principal_id: request.principal_id,
        purpose: UploadPurpose::PubSource,
        expected_byte_len: request.expected_byte_len,
        declared_content_type: request.declared_content_type,
        physical_upload_ref,
        state: UploadState::Issued,
        upload_generation: 0,
        object_version: None,
        object_etag: None,
        observed_byte_len: None,
        canonical_sha256: None,
        durable_binding_id: None,
        created_at_ms: request.now_ms,
        expires_at_ms: request.expires_at_ms,
        completed_at_ms: None,
        terminal_code: None,
        idempotency_key: request.idempotency_key,
        request_hash,
    })
}

impl SourceIngressService {
    pub fn new(
        max_source_bytes: u64,
        repo: Arc<dyn UploadRepository>,
        ids: Arc<dyn UploadIdGenerator>,
        quarantine: Arc<dyn QuarantineStore>,
        inspector: Arc<dyn SourceInspector>,
        durable_sources: Arc<dyn DurableSourceStore>,
        projects: Arc<dyn ProjectCreationPort>,
    ) -> Result<Self, IngressError> {
        if max_source_bytes == 0 {
            return Err(IngressError::new(
                "invalid_config",
                "max_source_bytes must be positive",
            ));
        }
        Ok(Self {
            max_source_bytes,
            repo,
            ids,
            quarantine,
            inspector,
            durable_sources,
            projects,
        })
    }

    pub fn issue_upload(
        &self,
        request: IssueUploadRequest,
    ) -> Result<IssueUploadResult, IngressError> {
        let upload_id = self.ids.next_upload_id()?;
        let candidate = plan_upload_candidate(self.max_source_bytes, upload_id, request)?;
        let upload = self.repo.issue_idempotent(candidate)?;
        let transport = self.quarantine.issue_transport(&upload)?;
        Ok(IssueUploadResult { upload, transport })
    }

    pub fn put_streamed_content(
        &self,
        tenant_id: &str,
        principal_id: &str,
        upload_id: &str,
        input: &mut dyn Read,
    ) -> Result<UploadRecord, IngressError> {
        let upload = self.authorized_upload(tenant_id, principal_id, upload_id)?;
        if upload.state != UploadState::Issued {
            return Err(IngressError::new(
                "upload_not_writable",
                "streamed content is accepted only while upload is ISSUED",
            ));
        }

        let mut bounded = BoundedReader::new(input, upload.expected_byte_len);
        self.quarantine
            .put_streamed_create_only(&upload.physical_upload_ref, &mut bounded)?;

        let mut probe = [0_u8; 1];
        match bounded.read(&mut probe) {
            Ok(0) => {}
            Ok(_) => {
                return Err(IngressError::new(
                    "upload_too_large",
                    "streamed upload exceeded reserved byte length",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                return Err(IngressError::new(
                    "upload_too_large",
                    "streamed upload exceeded reserved byte length",
                ));
            }
            Err(error) => {
                return Err(IngressError::new("upload_stream_failed", error.to_string()));
            }
        }
        if bounded.bytes_read != upload.expected_byte_len {
            return Err(IngressError::new(
                "upload_length_mismatch",
                "streamed upload length differs from reservation",
            ));
        }
        Ok(upload)
    }

    pub fn complete_upload(
        &self,
        tenant_id: &str,
        principal_id: &str,
        upload_id: &str,
        now_ms: u64,
    ) -> Result<CompleteUploadResult, IngressError> {
        let upload = self.authorized_upload(tenant_id, principal_id, upload_id)?;

        match upload.state {
            UploadState::StoredUnverified => {
                return Ok(CompleteUploadResult {
                    upload,
                    enqueue_validation: true,
                });
            }
            UploadState::Validating
            | UploadState::ValidatedDurable
            | UploadState::Consumed
            | UploadState::Rejected
            | UploadState::Expired => {
                return Ok(CompleteUploadResult {
                    upload,
                    enqueue_validation: false,
                });
            }
            UploadState::Issued => {}
        }

        if now_ms >= upload.expires_at_ms {
            let expired =
                self.transition_terminal(upload, UploadState::Expired, "upload_expired", now_ms)?;
            return Ok(CompleteUploadResult {
                upload: expired,
                enqueue_validation: false,
            });
        }

        let metadata = self
            .quarantine
            .head(&upload.physical_upload_ref)?
            .ok_or_else(|| {
                IngressError::new(
                    "quarantine_object_missing",
                    "completion cannot fabricate success before bytes exist",
                )
            })?;
        validate_object_metadata(&metadata)?;

        if metadata.byte_len != upload.expected_byte_len {
            return Err(IngressError::new(
                "upload_length_mismatch",
                "quarantine object length differs from reservation",
            ));
        }

        let mut next = upload.clone();
        next.state = UploadState::StoredUnverified;
        next.upload_generation = next.upload_generation.saturating_add(1);
        next.object_version = Some(metadata.version);
        next.object_etag = Some(metadata.etag);
        next.observed_byte_len = Some(metadata.byte_len);
        next.completed_at_ms = Some(now_ms);

        let upload =
            self.repo
                .compare_and_swap(&upload.upload_id, upload.upload_generation, next)?;
        Ok(CompleteUploadResult {
            upload,
            enqueue_validation: true,
        })
    }

    pub fn validate_source(
        &self,
        tenant_id: &str,
        upload_id: &str,
        now_ms: u64,
    ) -> Result<UploadRecord, IngressError> {
        let mut upload = self
            .repo
            .get(upload_id)?
            .ok_or_else(|| IngressError::new("upload_not_found", "upload does not exist"))?;
        if upload.tenant_id != tenant_id {
            return Err(IngressError::new(
                "tenant_mismatch",
                "upload is outside authenticated tenant",
            ));
        }

        match upload.state {
            UploadState::ValidatedDurable
            | UploadState::Consumed
            | UploadState::Rejected
            | UploadState::Expired => return Ok(upload),
            UploadState::Issued => {
                return Err(IngressError::new(
                    "upload_not_complete",
                    "validation requires STORED_UNVERIFIED",
                ));
            }
            UploadState::StoredUnverified => {
                let mut next = upload.clone();
                next.state = UploadState::Validating;
                next.upload_generation = next.upload_generation.saturating_add(1);
                upload = self.repo.compare_and_swap(
                    &upload.upload_id,
                    upload.upload_generation,
                    next,
                )?;
            }
            UploadState::Validating => {}
        }

        let version = upload.object_version.as_deref().ok_or_else(|| {
            IngressError::new(
                "object_identity_missing",
                "validation object version missing",
            )
        })?;
        let etag = upload.object_etag.as_deref().ok_or_else(|| {
            IngressError::new("object_identity_missing", "validation object etag missing")
        })?;
        let observed_len = upload.observed_byte_len.ok_or_else(|| {
            IngressError::new(
                "object_identity_missing",
                "validation object length missing",
            )
        })?;

        let first = self
            .quarantine
            .open_exact(&upload.physical_upload_ref, version, etag)?;
        let mut inspected = HashingBoundedReader::new(first, upload.expected_byte_len);
        if let Err(rejection) = self.inspector.inspect(&mut inspected) {
            return self.reject_validation(upload, rejection.code, now_ms);
        }
        inspected.require_consumed_exact(observed_len)?;
        let canonical_sha256 = inspected.sha256_hex();

        let second = self
            .quarantine
            .open_exact(&upload.physical_upload_ref, version, etag)?;
        let mut promoted_reader = HashingBoundedReader::new(second, upload.expected_byte_len);
        let binding = self.durable_sources.promote_immutable(
            &upload.tenant_id,
            &upload.upload_id,
            &canonical_sha256,
            &mut promoted_reader,
        )?;
        promoted_reader.require_consumed_exact(observed_len)?;
        let promoted_hash = promoted_reader.sha256_hex();
        if promoted_hash != canonical_sha256 {
            return Err(IngressError::new(
                "quarantine_object_changed",
                "durable promotion bytes differ from inspected exact object",
            ));
        }
        if binding.source_sha256 != canonical_sha256 {
            return Err(IngressError::new(
                "durable_binding_hash_mismatch",
                "durable binding does not name server-authoritative source hash",
            ));
        }
        require_ident(&binding.binding_id, "durable_binding_id")?;

        let mut next = upload.clone();
        next.state = UploadState::ValidatedDurable;
        next.upload_generation = next.upload_generation.saturating_add(1);
        next.canonical_sha256 = Some(canonical_sha256);
        next.durable_binding_id = Some(binding.binding_id);
        next.completed_at_ms = Some(now_ms);
        next.terminal_code = None;

        self.repo
            .compare_and_swap(&upload.upload_id, upload.upload_generation, next)
    }

    pub fn consume_into_project(
        &self,
        request: ConsumeUploadRequest,
    ) -> Result<ProjectCreateResult, IngressError> {
        require_ident(&request.tenant_id, "tenant_id")?;
        require_ident(&request.client_idempotency_id, "client_idempotency_id")?;
        require_ident(&request.workspace_id, "workspace_id")?;
        if request.name.is_empty() || request.name.len() > 512 {
            return Err(IngressError::new(
                "invalid_project_name",
                "project name must be bounded and non-empty",
            ));
        }

        let request_hash = consumption_request_hash(&request)?;
        if let Some(prior) = self
            .repo
            .find_consumption(&request.tenant_id, &request.client_idempotency_id)?
        {
            if prior.request_hash != request_hash || prior.upload_id != request.upload_id {
                return Err(IngressError::new(
                    "idempotency_conflict",
                    "project creation idempotency key was reused with different input",
                ));
            }
            return Ok(prior.project);
        }

        let upload = self
            .repo
            .get(&request.upload_id)?
            .ok_or_else(|| IngressError::new("upload_not_found", "upload does not exist"))?;
        if upload.tenant_id != request.tenant_id {
            return Err(IngressError::new(
                "tenant_mismatch",
                "validated source is outside authenticated tenant",
            ));
        }
        if upload.state != UploadState::ValidatedDurable {
            return Err(IngressError::new(
                "source_not_validated_durable",
                "project creation may consume only VALIDATED_DURABLE source",
            ));
        }
        if upload.upload_generation != request.expected_upload_generation {
            return Err(IngressError::new(
                "stale_upload_generation",
                "project creation upload generation is stale",
            ));
        }

        let source_sha256 = upload.canonical_sha256.clone().ok_or_else(|| {
            IngressError::new("validated_source_missing", "canonical source hash missing")
        })?;
        let durable_binding_id = upload.durable_binding_id.clone().ok_or_else(|| {
            IngressError::new("validated_source_missing", "durable binding missing")
        })?;

        let project_request = ProjectCreateRequest {
            tenant_id: request.tenant_id.clone(),
            upload_id: request.upload_id.clone(),
            durable_binding_id,
            source_sha256,
            workspace_id: request.workspace_id,
            name: request.name,
            client_idempotency_id: request.client_idempotency_id.clone(),
        };
        let project = self
            .projects
            .create_from_validated_source(&project_request)?;

        let receipt = ConsumptionReceipt {
            upload_id: request.upload_id.clone(),
            tenant_id: request.tenant_id,
            idempotency_key: request.client_idempotency_id,
            request_hash,
            committed_at_ms: request.now_ms,
            project,
        };
        let committed = self.repo.commit_consumption(
            &request.upload_id,
            request.expected_upload_generation,
            receipt,
        )?;
        Ok(committed.project)
    }

    pub fn expire_if_due(
        &self,
        upload_id: &str,
        now_ms: u64,
    ) -> Result<UploadRecord, IngressError> {
        let upload = self
            .repo
            .get(upload_id)?
            .ok_or_else(|| IngressError::new("upload_not_found", "upload does not exist"))?;
        if now_ms < upload.expires_at_ms
            || !matches!(
                upload.state,
                UploadState::Issued | UploadState::StoredUnverified
            )
        {
            return Ok(upload);
        }
        self.transition_terminal(upload, UploadState::Expired, "upload_expired", now_ms)
    }

    fn authorized_upload(
        &self,
        tenant_id: &str,
        principal_id: &str,
        upload_id: &str,
    ) -> Result<UploadRecord, IngressError> {
        let upload = self
            .repo
            .get(upload_id)?
            .ok_or_else(|| IngressError::new("upload_not_found", "upload does not exist"))?;
        if upload.tenant_id != tenant_id || upload.principal_id != principal_id {
            return Err(IngressError::new(
                "upload_access_denied",
                "upload does not belong to authenticated tenant principal",
            ));
        }
        Ok(upload)
    }

    fn reject_validation(
        &self,
        upload: UploadRecord,
        code: &'static str,
        now_ms: u64,
    ) -> Result<UploadRecord, IngressError> {
        if !valid_terminal_code(code) {
            return Err(IngressError::new(
                "invalid_terminal_code",
                "validation rejection code is not bounded",
            ));
        }
        self.transition_terminal(upload, UploadState::Rejected, code, now_ms)
    }

    fn transition_terminal(
        &self,
        upload: UploadRecord,
        state: UploadState,
        code: &'static str,
        now_ms: u64,
    ) -> Result<UploadRecord, IngressError> {
        let mut next = upload.clone();
        next.state = state;
        next.upload_generation = next.upload_generation.saturating_add(1);
        next.completed_at_ms = Some(now_ms);
        next.terminal_code = Some(code.to_owned());
        self.repo
            .compare_and_swap(&upload.upload_id, upload.upload_generation, next)
    }
}

fn validate_object_metadata(metadata: &ObjectMetadata) -> Result<(), IngressError> {
    require_ident(&metadata.version, "object_version")?;
    require_ident(&metadata.etag, "object_etag")?;
    if metadata.byte_len == 0 {
        return Err(IngressError::new(
            "empty_upload",
            "PUB source object must not be empty",
        ));
    }
    Ok(())
}

fn require_ident(value: &str, label: &'static str) -> Result<(), IngressError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(IngressError::new(
            "invalid_identifier",
            format!("{label} is not a bounded opaque identifier"),
        ));
    }
    Ok(())
}

fn valid_terminal_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn issue_request_hash(request: &IssueUploadRequest) -> Result<String, IngressError> {
    #[derive(Serialize)]
    struct Fingerprint<'a> {
        protocol: &'static str,
        tenant_id: &'a str,
        principal_id: &'a str,
        purpose: &'static str,
        expected_byte_len: u64,
        declared_content_type: &'a Option<String>,
    }
    hash_serialized(&Fingerprint {
        protocol: "chaptera.source-upload-issue.v1",
        tenant_id: &request.tenant_id,
        principal_id: &request.principal_id,
        purpose: "pub_source",
        expected_byte_len: request.expected_byte_len,
        declared_content_type: &request.declared_content_type,
    })
}

fn consumption_request_hash(request: &ConsumeUploadRequest) -> Result<String, IngressError> {
    #[derive(Serialize)]
    struct Fingerprint<'a> {
        protocol: &'static str,
        tenant_id: &'a str,
        upload_id: &'a str,
        expected_upload_generation: u64,
        workspace_id: &'a str,
        name: &'a str,
    }
    hash_serialized(&Fingerprint {
        protocol: "chaptera.project-from-upload.v1",
        tenant_id: &request.tenant_id,
        upload_id: &request.upload_id,
        expected_upload_generation: request.expected_upload_generation,
        workspace_id: &request.workspace_id,
        name: &request.name,
    })
}

fn hash_serialized<T: Serialize>(value: &T) -> Result<String, IngressError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| IngressError::new("request_hash_failed", error.to_string()))?;
    Ok(sha256_hex(&bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_digest(Sha256::digest(bytes))
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = digest.as_ref();
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

struct BoundedReader<'a> {
    inner: &'a mut dyn Read,
    max_bytes: u64,
    bytes_read: u64,
}

impl<'a> BoundedReader<'a> {
    fn new(inner: &'a mut dyn Read, max_bytes: u64) -> Self {
        Self {
            inner,
            max_bytes,
            bytes_read: 0,
        }
    }
}

impl Read for BoundedReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = self.max_bytes.saturating_sub(self.bytes_read);
        let allowed = usize::try_from(remaining.saturating_add(1))
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        if allowed == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bounded upload overflow",
            ));
        }
        let count = self.inner.read(&mut buffer[..allowed])?;
        self.bytes_read = self
            .bytes_read
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "byte count overflow"))?;
        if self.bytes_read > self.max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bounded upload overflow",
            ));
        }
        Ok(count)
    }
}

struct HashingBoundedReader {
    inner: Box<dyn Read + Send>,
    max_bytes: u64,
    bytes_read: u64,
    saw_eof: bool,
    hasher: Sha256,
}

impl HashingBoundedReader {
    fn new(inner: Box<dyn Read + Send>, max_bytes: u64) -> Self {
        Self {
            inner,
            max_bytes,
            bytes_read: 0,
            saw_eof: false,
            hasher: Sha256::new(),
        }
    }

    fn require_consumed_exact(&self, expected: u64) -> Result<(), IngressError> {
        if !self.saw_eof {
            return Err(IngressError::new(
                "validator_incomplete_read",
                "validation/promotion runtime did not consume exact object to EOF",
            ));
        }
        if self.bytes_read != expected {
            return Err(IngressError::new(
                "validated_length_mismatch",
                "validation/promotion byte count differs from completed object",
            ));
        }
        Ok(())
    }

    fn sha256_hex(&self) -> String {
        hex_digest(self.hasher.clone().finalize())
    }
}

impl Read for HashingBoundedReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = self.max_bytes.saturating_sub(self.bytes_read);
        let allowed = usize::try_from(remaining.saturating_add(1))
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        if allowed == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bounded validation overflow",
            ));
        }
        let count = self.inner.read(&mut buffer[..allowed])?;
        if count == 0 {
            self.saw_eof = true;
            return Ok(0);
        }
        self.bytes_read = self
            .bytes_read
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "byte count overflow"))?;
        if self.bytes_read > self.max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bounded validation overflow",
            ));
        }
        self.hasher.update(&buffer[..count]);
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        io::{Cursor, Read},
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use super::*;

    const STREAM_BUFFER_BYTES: usize = 64 * 1024;

    #[derive(Default)]
    struct MemoryRepo {
        uploads: Mutex<BTreeMap<String, UploadRecord>>,
        idempotency: Mutex<BTreeMap<(String, String), String>>,
        consumptions: Mutex<BTreeMap<(String, String), ConsumptionReceipt>>,
    }

    impl UploadRepository for MemoryRepo {
        fn issue_idempotent(&self, candidate: UploadRecord) -> Result<UploadRecord, IngressError> {
            let key = (
                candidate.tenant_id.clone(),
                candidate.idempotency_key.clone(),
            );
            if let Some(upload_id) = self.idempotency.lock().unwrap().get(&key).cloned() {
                let current = self
                    .uploads
                    .lock()
                    .unwrap()
                    .get(&upload_id)
                    .cloned()
                    .unwrap();
                if current.request_hash != candidate.request_hash {
                    return Err(IngressError::new(
                        "idempotency_conflict",
                        "issue key reused with different request",
                    ));
                }
                return Ok(current);
            }
            self.idempotency
                .lock()
                .unwrap()
                .insert(key, candidate.upload_id.clone());
            self.uploads
                .lock()
                .unwrap()
                .insert(candidate.upload_id.clone(), candidate.clone());
            Ok(candidate)
        }

        fn get(&self, upload_id: &str) -> Result<Option<UploadRecord>, IngressError> {
            Ok(self.uploads.lock().unwrap().get(upload_id).cloned())
        }

        fn compare_and_swap(
            &self,
            upload_id: &str,
            expected_generation: u64,
            next: UploadRecord,
        ) -> Result<UploadRecord, IngressError> {
            let mut uploads = self.uploads.lock().unwrap();
            let current = uploads
                .get(upload_id)
                .ok_or_else(|| IngressError::new("upload_not_found", "missing upload"))?;
            if current.upload_generation != expected_generation {
                return Err(IngressError::new(
                    "stale_upload_generation",
                    "compare-and-swap generation mismatch",
                ));
            }
            uploads.insert(upload_id.to_owned(), next.clone());
            Ok(next)
        }

        fn find_consumption(
            &self,
            tenant_id: &str,
            idempotency_key: &str,
        ) -> Result<Option<ConsumptionReceipt>, IngressError> {
            Ok(self
                .consumptions
                .lock()
                .unwrap()
                .get(&(tenant_id.to_owned(), idempotency_key.to_owned()))
                .cloned())
        }

        fn commit_consumption(
            &self,
            upload_id: &str,
            expected_generation: u64,
            receipt: ConsumptionReceipt,
        ) -> Result<ConsumptionReceipt, IngressError> {
            let key = (receipt.tenant_id.clone(), receipt.idempotency_key.clone());
            if let Some(prior) = self.consumptions.lock().unwrap().get(&key).cloned() {
                if prior.request_hash != receipt.request_hash
                    || prior.upload_id != receipt.upload_id
                {
                    return Err(IngressError::new(
                        "idempotency_conflict",
                        "consumption key reused with different request",
                    ));
                }
                return Ok(prior);
            }
            let mut uploads = self.uploads.lock().unwrap();
            let current = uploads
                .get(upload_id)
                .cloned()
                .ok_or_else(|| IngressError::new("upload_not_found", "missing upload"))?;
            if current.upload_generation != expected_generation
                || current.state != UploadState::ValidatedDurable
            {
                return Err(IngressError::new(
                    "stale_upload_generation",
                    "consumption compare-and-swap failed",
                ));
            }
            let mut next = current;
            next.state = UploadState::Consumed;
            next.upload_generation = next.upload_generation.saturating_add(1);
            uploads.insert(upload_id.to_owned(), next);
            self.consumptions
                .lock()
                .unwrap()
                .insert(key, receipt.clone());
            Ok(receipt)
        }
    }

    struct SequenceIds(AtomicUsize);

    impl SequenceIds {
        fn new() -> Self {
            Self(AtomicUsize::new(1))
        }
    }

    impl UploadIdGenerator for SequenceIds {
        fn next_upload_id(&self) -> Result<String, IngressError> {
            Ok(format!("upload-{}", self.0.fetch_add(1, Ordering::SeqCst)))
        }
    }

    #[derive(Default)]
    struct MemoryQuarantine {
        objects: Mutex<BTreeMap<String, Vec<u8>>>,
    }

    impl QuarantineStore for MemoryQuarantine {
        fn issue_transport(&self, upload: &UploadRecord) -> Result<UploadTransport, IngressError> {
            Ok(UploadTransport::Streamed {
                path: format!("/v1/uploads/{}/content", upload.upload_id),
            })
        }

        fn put_streamed_create_only(
            &self,
            physical_ref: &str,
            input: &mut dyn Read,
        ) -> Result<(), IngressError> {
            let mut objects = self.objects.lock().unwrap();
            if objects.contains_key(physical_ref) {
                return Err(IngressError::new(
                    "quarantine_object_exists",
                    "create-only quarantine object already exists",
                ));
            }
            let mut bytes = Vec::new();
            input
                .read_to_end(&mut bytes)
                .map_err(|error| IngressError::new("quarantine_write_failed", error.to_string()))?;
            objects.insert(physical_ref.to_owned(), bytes);
            Ok(())
        }

        fn head(&self, physical_ref: &str) -> Result<Option<ObjectMetadata>, IngressError> {
            Ok(self
                .objects
                .lock()
                .unwrap()
                .get(physical_ref)
                .map(|bytes| ObjectMetadata {
                    version: "version-1".to_owned(),
                    etag: sha256_hex(bytes),
                    byte_len: bytes.len() as u64,
                }))
        }

        fn open_exact(
            &self,
            physical_ref: &str,
            version: &str,
            etag: &str,
        ) -> Result<Box<dyn Read + Send>, IngressError> {
            if version != "version-1" {
                return Err(IngressError::new(
                    "object_version_mismatch",
                    "quarantine version changed",
                ));
            }
            let bytes = self
                .objects
                .lock()
                .unwrap()
                .get(physical_ref)
                .cloned()
                .ok_or_else(|| IngressError::new("quarantine_object_missing", "missing object"))?;
            if sha256_hex(&bytes) != etag {
                return Err(IngressError::new(
                    "object_etag_mismatch",
                    "quarantine etag changed",
                ));
            }
            Ok(Box::new(Cursor::new(bytes)))
        }
    }

    struct AcceptInspector;

    impl SourceInspector for AcceptInspector {
        fn inspect(&self, input: &mut dyn Read) -> Result<(), ValidationRejection> {
            let mut buffer = [0_u8; STREAM_BUFFER_BYTES];
            loop {
                match input.read(&mut buffer) {
                    Ok(0) => return Ok(()),
                    Ok(_) => {}
                    Err(_) => return Err(ValidationRejection::new("source_read_failed")),
                }
            }
        }
    }

    struct RejectInspector;

    impl SourceInspector for RejectInspector {
        fn inspect(&self, input: &mut dyn Read) -> Result<(), ValidationRejection> {
            let mut buffer = [0_u8; 8];
            let _ = input.read(&mut buffer);
            Err(ValidationRejection::new("malware_detected"))
        }
    }

    #[derive(Default)]
    struct MemoryDurable {
        bindings: Mutex<BTreeMap<(String, String), DurableSourceBinding>>,
    }

    impl DurableSourceStore for MemoryDurable {
        fn promote_immutable(
            &self,
            tenant_id: &str,
            _upload_id: &str,
            source_sha256: &str,
            input: &mut dyn Read,
        ) -> Result<DurableSourceBinding, IngressError> {
            let mut bytes = Vec::new();
            input
                .read_to_end(&mut bytes)
                .map_err(|error| IngressError::new("durable_write_failed", error.to_string()))?;
            if sha256_hex(&bytes) != source_sha256 {
                return Err(IngressError::new(
                    "durable_hash_mismatch",
                    "durable write source hash mismatch",
                ));
            }
            let key = (tenant_id.to_owned(), source_sha256.to_owned());
            let mut bindings = self.bindings.lock().unwrap();
            if let Some(existing) = bindings.get(&key) {
                return Ok(existing.clone());
            }
            let binding = DurableSourceBinding {
                binding_id: format!("binding-{}", &source_sha256[..16]),
                source_sha256: source_sha256.to_owned(),
                durable_ref: format!("source/{tenant_id}/{source_sha256}"),
            };
            bindings.insert(key, binding.clone());
            Ok(binding)
        }
    }

    #[derive(Default)]
    struct MemoryProjects {
        calls: AtomicUsize,
        results: Mutex<BTreeMap<(String, String), ProjectCreateResult>>,
    }

    impl ProjectCreationPort for MemoryProjects {
        fn create_from_validated_source(
            &self,
            request: &ProjectCreateRequest,
        ) -> Result<ProjectCreateResult, IngressError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let key = (
                request.tenant_id.clone(),
                request.client_idempotency_id.clone(),
            );
            let mut results = self.results.lock().unwrap();
            if let Some(result) = results.get(&key) {
                return Ok(result.clone());
            }
            let suffix = &request.source_sha256[..12];
            let result = ProjectCreateResult {
                project_id: format!("project-{suffix}"),
                document_id: format!("document-{suffix}"),
                genesis_revision_id: format!("revision-{suffix}"),
            };
            results.insert(key, result.clone());
            Ok(result)
        }
    }

    fn service_with_inspector(
        inspector: Arc<dyn SourceInspector>,
    ) -> (
        SourceIngressService,
        Arc<MemoryRepo>,
        Arc<MemoryQuarantine>,
        Arc<MemoryProjects>,
    ) {
        let repo = Arc::new(MemoryRepo::default());
        let quarantine = Arc::new(MemoryQuarantine::default());
        let projects = Arc::new(MemoryProjects::default());
        let service = SourceIngressService::new(
            1024 * 1024,
            repo.clone(),
            Arc::new(SequenceIds::new()),
            quarantine.clone(),
            inspector,
            Arc::new(MemoryDurable::default()),
            projects.clone(),
        )
        .unwrap();
        (service, repo, quarantine, projects)
    }

    fn issue(service: &SourceIngressService, len: usize) -> IssueUploadResult {
        service
            .issue_upload(IssueUploadRequest {
                tenant_id: "tenant-a".into(),
                principal_id: "principal-a".into(),
                expected_byte_len: len as u64,
                declared_content_type: Some("application/x-mspublisher".into()),
                idempotency_key: "issue-1".into(),
                now_ms: 100,
                expires_at_ms: 10_000,
            })
            .unwrap()
    }

    fn upload_and_complete(service: &SourceIngressService, bytes: &[u8]) -> UploadRecord {
        let issued = issue(service, bytes.len());
        let mut cursor = Cursor::new(bytes.to_vec());
        service
            .put_streamed_content(
                "tenant-a",
                "principal-a",
                &issued.upload.upload_id,
                &mut cursor,
            )
            .unwrap();
        service
            .complete_upload("tenant-a", "principal-a", &issued.upload.upload_id, 200)
            .unwrap()
            .upload
    }

    #[test]
    fn issue_is_idempotent_and_conflicts_on_different_request() {
        let (service, _repo, _quarantine, _projects) =
            service_with_inspector(Arc::new(AcceptInspector));
        let first = issue(&service, 4);
        let second = issue(&service, 4);
        assert_eq!(first.upload.upload_id, second.upload.upload_id);

        let error = service
            .issue_upload(IssueUploadRequest {
                tenant_id: "tenant-a".into(),
                principal_id: "principal-a".into(),
                expected_byte_len: 5,
                declared_content_type: Some("application/x-mspublisher".into()),
                idempotency_key: "issue-1".into(),
                now_ms: 101,
                expires_at_ms: 10_000,
            })
            .unwrap_err();
        assert_eq!(error.code, "idempotency_conflict");
    }

    #[test]
    fn streamed_upload_complete_validate_and_consume_never_bind_quarantine() {
        let (service, repo, _quarantine, projects) =
            service_with_inspector(Arc::new(AcceptInspector));
        let bytes = b"safe-pub-fixture";
        let completed = upload_and_complete(&service, bytes);
        assert_eq!(completed.state, UploadState::StoredUnverified);
        assert!(completed.durable_binding_id.is_none());

        let validated = service
            .validate_source("tenant-a", &completed.upload_id, 300)
            .unwrap();
        assert_eq!(validated.state, UploadState::ValidatedDurable);
        assert_eq!(
            validated.canonical_sha256.as_deref(),
            Some(sha256_hex(bytes).as_str())
        );
        assert!(validated.durable_binding_id.is_some());
        assert!(validated.quarantine_cleanup_eligible());

        let project = service
            .consume_into_project(ConsumeUploadRequest {
                tenant_id: "tenant-a".into(),
                upload_id: validated.upload_id.clone(),
                expected_upload_generation: validated.upload_generation,
                workspace_id: "workspace-a".into(),
                name: "Imported PUB".into(),
                client_idempotency_id: "create-project-1".into(),
                now_ms: 500,
            })
            .unwrap();
        let replay = service
            .consume_into_project(ConsumeUploadRequest {
                tenant_id: "tenant-a".into(),
                upload_id: validated.upload_id.clone(),
                expected_upload_generation: validated.upload_generation,
                workspace_id: "workspace-a".into(),
                name: "Imported PUB".into(),
                client_idempotency_id: "create-project-1".into(),
                now_ms: 900,
            })
            .unwrap();

        assert_eq!(project, replay);
        assert_eq!(projects.calls.load(Ordering::SeqCst), 1);
        let consumptions = repo.consumptions.lock().unwrap();
        let receipt = consumptions
            .get(&("tenant-a".to_owned(), "create-project-1".to_owned()))
            .unwrap();
        assert_eq!(receipt.committed_at_ms, 500);
        drop(consumptions);
        assert_eq!(
            repo.get(&validated.upload_id).unwrap().unwrap().state,
            UploadState::Consumed
        );
    }

    #[test]
    fn project_creation_rejects_unvalidated_quarantine_source() {
        let (service, _repo, _quarantine, _projects) =
            service_with_inspector(Arc::new(AcceptInspector));
        let completed = upload_and_complete(&service, b"not-yet-validated");

        let error = service
            .consume_into_project(ConsumeUploadRequest {
                tenant_id: "tenant-a".into(),
                upload_id: completed.upload_id,
                expected_upload_generation: completed.upload_generation,
                workspace_id: "workspace-a".into(),
                name: "Must fail".into(),
                client_idempotency_id: "create-project-1".into(),
                now_ms: 500,
            })
            .unwrap_err();
        assert_eq!(error.code, "source_not_validated_durable");
    }

    #[test]
    fn validation_rejection_persists_bounded_code_without_binding() {
        let (service, _repo, _quarantine, _projects) =
            service_with_inspector(Arc::new(RejectInspector));
        let completed = upload_and_complete(&service, b"hostile-fixture");
        let rejected = service
            .validate_source("tenant-a", &completed.upload_id, 300)
            .unwrap();

        assert_eq!(rejected.state, UploadState::Rejected);
        assert_eq!(rejected.terminal_code.as_deref(), Some("malware_detected"));
        assert!(rejected.canonical_sha256.is_none());
        assert!(rejected.durable_binding_id.is_none());
        assert!(rejected.quarantine_cleanup_eligible());
    }

    #[test]
    fn streamed_content_must_match_exact_reserved_length() {
        let (service, _repo, _quarantine, _projects) =
            service_with_inspector(Arc::new(AcceptInspector));
        let issued = issue(&service, 4);
        let mut short = Cursor::new(b"abc".to_vec());
        let error = service
            .put_streamed_content(
                "tenant-a",
                "principal-a",
                &issued.upload.upload_id,
                &mut short,
            )
            .unwrap_err();
        assert_eq!(error.code, "upload_length_mismatch");
    }

    #[test]
    fn complete_replay_reenqueues_stored_unverified_after_crash_window() {
        let (service, _repo, _quarantine, _projects) =
            service_with_inspector(Arc::new(AcceptInspector));
        let completed = upload_and_complete(&service, b"abcd");
        let replay = service
            .complete_upload("tenant-a", "principal-a", &completed.upload_id, 201)
            .unwrap();
        assert_eq!(replay.upload.state, UploadState::StoredUnverified);
        assert!(replay.enqueue_validation);
    }

    #[test]
    fn expiry_is_terminal_and_cleanup_eligible() {
        let (service, _repo, _quarantine, _projects) =
            service_with_inspector(Arc::new(AcceptInspector));
        let issued = issue(&service, 4);
        let expired = service
            .expire_if_due(&issued.upload.upload_id, 20_000)
            .unwrap();
        assert_eq!(expired.state, UploadState::Expired);
        assert_eq!(expired.terminal_code.as_deref(), Some("upload_expired"));
        assert!(expired.quarantine_cleanup_eligible());
    }
}
