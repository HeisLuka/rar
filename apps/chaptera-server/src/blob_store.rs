use std::{
    fmt,
    io::{self, Read, Write},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const COPY_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobStoreError {
    pub code: &'static str,
    pub message: String,
}

impl BlobStoreError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for BlobStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for BlobStoreError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlobNamespace {
    Quarantine,
    Canonical,
    Checkpoint,
    Derived,
    Export,
}

impl BlobNamespace {
    fn path_segment(self) -> &'static str {
        match self {
            Self::Quarantine => "quarantine",
            Self::Canonical => "canonical",
            Self::Checkpoint => "checkpoints",
            Self::Derived => "derived",
            Self::Export => "exports",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    PubSource,
    Asset,
    Checkpoint,
    DerivedArtifact,
    ExportArtifact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingLifecycle {
    Active,
    Retired,
    PurgeEligible,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub hard_create_only: bool,
    pub hard_exact_or_max_upload_size: bool,
    pub signed_content_type: bool,
    pub strong_head_after_put: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderObjectMetadata {
    pub generation: String,
    pub byte_len: u64,
    pub etag: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorKind {
    AlreadyExists,
    UnknownOutcome,
    NotFound,
    AccessDenied,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    pub kind: ProviderErrorKind,
    pub code: String,
}

impl ProviderError {
    pub fn new(kind: ProviderErrorKind, code: impl Into<String>) -> Self {
        Self {
            kind,
            code: code.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantOperation {
    UploadCreateOnly,
    Download,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderGrantRequest {
    pub tenant_id: String,
    pub object_locator: String,
    pub operation: GrantOperation,
    pub expected_byte_len: Option<u64>,
    pub required_content_type: Option<String>,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderGrant {
    pub opaque_url: String,
    pub expires_at_ms: u64,
}

pub trait BlobProvider: Send + Sync {
    fn capabilities(&self) -> ProviderCapabilities;

    fn create_immutable(
        &self,
        object_locator: &str,
        expected_byte_len: u64,
        input: &mut dyn Read,
    ) -> Result<ProviderObjectMetadata, ProviderError>;

    fn head_exact(
        &self,
        object_locator: &str,
    ) -> Result<Option<ProviderObjectMetadata>, ProviderError>;

    fn open_read(
        &self,
        object_locator: &str,
        generation: &str,
    ) -> Result<Box<dyn Read + Send>, ProviderError>;

    fn delete_exact(&self, object_locator: &str, generation: &str) -> Result<(), ProviderError>;

    fn issue_grant(&self, request: &ProviderGrantRequest) -> Result<ProviderGrant, ProviderError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalBlobRecord {
    pub physical_blob_id: String,
    pub tenant_id: String,
    pub content_sha256: String,
    pub byte_len: u64,
    pub canonical_mime: Option<String>,
    pub object_namespace: BlobNamespace,
    pub object_locator: String,
    pub storage_generation: String,
    pub created_at_ms: u64,
    pub delete_eligible_at_ms: Option<u64>,
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceBinding {
    pub binding_id: String,
    pub tenant_id: String,
    pub project_id: Option<String>,
    pub document_id: Option<String>,
    pub physical_blob_id: String,
    pub content_sha256: String,
    pub byte_len: u64,
    pub resource_kind: ResourceKind,
    pub validation_profile: String,
    pub lifecycle_state: BindingLifecycle,
    pub created_at_ms: u64,
    pub retired_at_ms: Option<u64>,
}

pub trait BlobIdGenerator: Send + Sync {
    fn next_physical_blob_id(&self) -> Result<String, BlobStoreError>;
    fn next_binding_id(&self) -> Result<String, BlobStoreError>;
}

pub trait BlobBindingRepository: Send + Sync {
    fn find_physical_by_content(
        &self,
        tenant_id: &str,
        content_sha256: &str,
        byte_len: u64,
    ) -> Result<Option<PhysicalBlobRecord>, BlobStoreError>;

    fn get_physical(
        &self,
        physical_blob_id: &str,
    ) -> Result<Option<PhysicalBlobRecord>, BlobStoreError>;

    fn get_binding(&self, binding_id: &str) -> Result<Option<ResourceBinding>, BlobStoreError>;

    fn commit_physical_and_binding(
        &self,
        physical: PhysicalBlobRecord,
        binding: ResourceBinding,
    ) -> Result<ResourceBinding, BlobStoreError>;

    fn commit_binding(&self, binding: ResourceBinding) -> Result<ResourceBinding, BlobStoreError>;

    fn mark_physical_deleted(
        &self,
        physical_blob_id: &str,
        expected_generation: &str,
    ) -> Result<PhysicalBlobRecord, BlobStoreError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateBindingRequest {
    pub tenant_id: String,
    pub project_id: Option<String>,
    pub document_id: Option<String>,
    pub content_sha256: String,
    pub byte_len: u64,
    pub canonical_mime: Option<String>,
    pub resource_kind: ResourceKind,
    pub validation_profile: String,
    pub now_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectUploadGrant {
    pub tenant_id: String,
    pub upload_id: String,
    pub object_locator: String,
    pub expected_byte_len: u64,
    pub required_content_type: Option<String>,
    pub provider_capabilities: ProviderCapabilities,
    pub opaque_url: String,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadGrant {
    pub tenant_id: String,
    pub binding_id: String,
    pub opaque_url: String,
    pub expires_at_ms: u64,
}

#[derive(Clone)]
pub struct BlobStoreService {
    provider: Arc<dyn BlobProvider>,
    repo: Arc<dyn BlobBindingRepository>,
    ids: Arc<dyn BlobIdGenerator>,
}

impl BlobStoreService {
    pub fn new(
        provider: Arc<dyn BlobProvider>,
        repo: Arc<dyn BlobBindingRepository>,
        ids: Arc<dyn BlobIdGenerator>,
    ) -> Self {
        Self {
            provider,
            repo,
            ids,
        }
    }

    pub fn provider_capabilities(&self) -> ProviderCapabilities {
        self.provider.capabilities()
    }

    pub fn create_canonical_binding(
        &self,
        request: CreateBindingRequest,
        input: &mut dyn Read,
    ) -> Result<ResourceBinding, BlobStoreError> {
        validate_create_request(&request)?;

        if let Some(existing) = self.repo.find_physical_by_content(
            &request.tenant_id,
            &request.content_sha256,
            request.byte_len,
        )? {
            self.verify_physical_exact(&existing)?;
            return self.bind_existing(&request, &existing);
        }

        let physical_blob_id = self.ids.next_physical_blob_id()?;
        require_ident(&physical_blob_id, "physical_blob_id")?;
        let binding_id = self.ids.next_binding_id()?;
        require_ident(&binding_id, "binding_id")?;

        let object_locator = object_locator(
            BlobNamespace::Canonical,
            &request.tenant_id,
            &physical_blob_id,
        );

        let mut hashing = HashingBoundedReader::new(input, request.byte_len);
        let create_result =
            self.provider
                .create_immutable(&object_locator, request.byte_len, &mut hashing);

        let metadata = match create_result {
            Ok(metadata) => metadata,
            Err(error) if error.kind == ProviderErrorKind::UnknownOutcome => self
                .provider
                .head_exact(&object_locator)
                .map_err(provider_error)?
                .ok_or_else(|| {
                    BlobStoreError::new(
                        "provider_unknown_unreconciled",
                        "create outcome is unknown and exact object is absent",
                    )
                })?,
            Err(error) if error.kind == ProviderErrorKind::AlreadyExists => {
                return Err(BlobStoreError::new(
                    "physical_key_collision",
                    "server-generated create-once object key already exists",
                ));
            }
            Err(error) => return Err(provider_error(error)),
        };

        hashing.require_exact_eof(request.byte_len)?;
        let written_sha = hashing.sha256_hex();
        if written_sha != request.content_sha256 {
            return Err(BlobStoreError::new(
                "content_hash_mismatch",
                "streamed bytes differ from expected content SHA-256",
            ));
        }
        validate_provider_metadata(&metadata, request.byte_len)?;

        let physical = PhysicalBlobRecord {
            physical_blob_id: physical_blob_id.clone(),
            tenant_id: request.tenant_id.clone(),
            content_sha256: request.content_sha256.clone(),
            byte_len: request.byte_len,
            canonical_mime: request.canonical_mime.clone(),
            object_namespace: BlobNamespace::Canonical,
            object_locator,
            storage_generation: metadata.generation,
            created_at_ms: request.now_ms,
            delete_eligible_at_ms: None,
            deleted: false,
        };
        self.verify_physical_exact(&physical)?;

        let binding = ResourceBinding {
            binding_id,
            tenant_id: request.tenant_id,
            project_id: request.project_id,
            document_id: request.document_id,
            physical_blob_id,
            content_sha256: request.content_sha256,
            byte_len: request.byte_len,
            resource_kind: request.resource_kind,
            validation_profile: request.validation_profile,
            lifecycle_state: BindingLifecycle::Active,
            created_at_ms: request.now_ms,
            retired_at_ms: None,
        };

        self.repo.commit_physical_and_binding(physical, binding)
    }

    pub fn stream_binding_verified(
        &self,
        tenant_id: &str,
        binding_id: &str,
        output: &mut dyn Write,
    ) -> Result<u64, BlobStoreError> {
        require_ident(tenant_id, "tenant_id")?;
        require_ident(binding_id, "binding_id")?;
        let binding = self.repo.get_binding(binding_id)?.ok_or_else(|| {
            BlobStoreError::new("binding_not_found", "resource binding not found")
        })?;
        if binding.tenant_id != tenant_id {
            return Err(BlobStoreError::new(
                "cross_tenant_binding",
                "resource binding belongs to another tenant",
            ));
        }
        if binding.lifecycle_state != BindingLifecycle::Active {
            return Err(BlobStoreError::new(
                "binding_not_active",
                "resource binding is not active",
            ));
        }

        let physical = self
            .repo
            .get_physical(&binding.physical_blob_id)?
            .ok_or_else(|| BlobStoreError::new("physical_blob_missing", "physical blob missing"))?;
        assert_binding_matches_physical(&binding, &physical)?;
        self.copy_and_verify(&physical, output)
    }

    pub fn issue_download_grant(
        &self,
        tenant_id: &str,
        binding_id: &str,
        now_ms: u64,
        expires_at_ms: u64,
    ) -> Result<DownloadGrant, BlobStoreError> {
        validate_expiry(now_ms, expires_at_ms)?;
        let binding = self.repo.get_binding(binding_id)?.ok_or_else(|| {
            BlobStoreError::new("binding_not_found", "resource binding not found")
        })?;
        if binding.tenant_id != tenant_id {
            return Err(BlobStoreError::new(
                "cross_tenant_binding",
                "resource binding belongs to another tenant",
            ));
        }
        if binding.lifecycle_state != BindingLifecycle::Active {
            return Err(BlobStoreError::new(
                "binding_not_active",
                "resource binding is not active",
            ));
        }
        let physical = self
            .repo
            .get_physical(&binding.physical_blob_id)?
            .ok_or_else(|| BlobStoreError::new("physical_blob_missing", "physical blob missing"))?;
        assert_binding_matches_physical(&binding, &physical)?;
        self.verify_physical_exact(&physical)?;

        let grant = self
            .provider
            .issue_grant(&ProviderGrantRequest {
                tenant_id: tenant_id.to_owned(),
                object_locator: physical.object_locator,
                operation: GrantOperation::Download,
                expected_byte_len: Some(binding.byte_len),
                required_content_type: physical.canonical_mime,
                expires_at_ms,
            })
            .map_err(provider_error)?;
        if grant.expires_at_ms != expires_at_ms {
            return Err(BlobStoreError::new(
                "grant_expiry_mismatch",
                "provider download grant expiry differs from requested fence",
            ));
        }

        Ok(DownloadGrant {
            tenant_id: tenant_id.to_owned(),
            binding_id: binding_id.to_owned(),
            opaque_url: grant.opaque_url,
            expires_at_ms,
        })
    }

    pub fn issue_quarantine_upload_grant(
        &self,
        tenant_id: &str,
        upload_id: &str,
        expected_byte_len: u64,
        required_content_type: Option<String>,
        now_ms: u64,
        expires_at_ms: u64,
    ) -> Result<DirectUploadGrant, BlobStoreError> {
        require_ident(tenant_id, "tenant_id")?;
        require_ident(upload_id, "upload_id")?;
        if expected_byte_len == 0 {
            return Err(BlobStoreError::new(
                "invalid_upload_size",
                "direct upload byte length must be positive",
            ));
        }
        validate_expiry(now_ms, expires_at_ms)?;

        let capabilities = self.provider.capabilities();
        if !capabilities.hard_create_only {
            return Err(BlobStoreError::new(
                "direct_upload_create_only_unproven",
                "provider does not prove hard create-only upload semantics",
            ));
        }
        if !capabilities.hard_exact_or_max_upload_size {
            return Err(BlobStoreError::new(
                "direct_upload_size_fence_unproven",
                "provider does not prove a hard per-upload size fence",
            ));
        }
        if required_content_type.is_some() && !capabilities.signed_content_type {
            return Err(BlobStoreError::new(
                "direct_upload_content_type_unproven",
                "provider cannot bind declared content type into the upload grant",
            ));
        }

        let object_locator = object_locator(BlobNamespace::Quarantine, tenant_id, upload_id);
        let grant = self
            .provider
            .issue_grant(&ProviderGrantRequest {
                tenant_id: tenant_id.to_owned(),
                object_locator: object_locator.clone(),
                operation: GrantOperation::UploadCreateOnly,
                expected_byte_len: Some(expected_byte_len),
                required_content_type: required_content_type.clone(),
                expires_at_ms,
            })
            .map_err(provider_error)?;
        if grant.expires_at_ms != expires_at_ms {
            return Err(BlobStoreError::new(
                "grant_expiry_mismatch",
                "provider upload grant expiry differs from requested fence",
            ));
        }

        Ok(DirectUploadGrant {
            tenant_id: tenant_id.to_owned(),
            upload_id: upload_id.to_owned(),
            object_locator,
            expected_byte_len,
            required_content_type,
            provider_capabilities: capabilities,
            opaque_url: grant.opaque_url,
            expires_at_ms,
        })
    }

    pub fn delete_physical_if_eligible(
        &self,
        tenant_id: &str,
        physical_blob_id: &str,
        now_ms: u64,
    ) -> Result<PhysicalBlobRecord, BlobStoreError> {
        let physical = self
            .repo
            .get_physical(physical_blob_id)?
            .ok_or_else(|| BlobStoreError::new("physical_blob_missing", "physical blob missing"))?;
        if physical.tenant_id != tenant_id {
            return Err(BlobStoreError::new(
                "cross_tenant_physical_blob",
                "physical blob belongs to another tenant",
            ));
        }
        if physical.deleted {
            return Ok(physical);
        }
        let eligible = physical.delete_eligible_at_ms.ok_or_else(|| {
            BlobStoreError::new(
                "blob_not_delete_eligible",
                "physical blob has no explicit delete eligibility fence",
            )
        })?;
        if now_ms < eligible {
            return Err(BlobStoreError::new(
                "blob_not_delete_eligible",
                "physical blob delete eligibility time has not arrived",
            ));
        }

        match self
            .provider
            .delete_exact(&physical.object_locator, &physical.storage_generation)
        {
            Ok(()) => {}
            Err(error) if error.kind == ProviderErrorKind::NotFound => {}
            Err(error) => return Err(provider_error(error)),
        }
        self.repo
            .mark_physical_deleted(physical_blob_id, &physical.storage_generation)
    }

    fn bind_existing(
        &self,
        request: &CreateBindingRequest,
        physical: &PhysicalBlobRecord,
    ) -> Result<ResourceBinding, BlobStoreError> {
        if physical.tenant_id != request.tenant_id
            || physical.content_sha256 != request.content_sha256
            || physical.byte_len != request.byte_len
            || physical.deleted
        {
            return Err(BlobStoreError::new(
                "dedupe_candidate_mismatch",
                "tenant-local dedupe candidate does not match exact content identity",
            ));
        }
        let binding_id = self.ids.next_binding_id()?;
        require_ident(&binding_id, "binding_id")?;
        self.repo.commit_binding(ResourceBinding {
            binding_id,
            tenant_id: request.tenant_id.clone(),
            project_id: request.project_id.clone(),
            document_id: request.document_id.clone(),
            physical_blob_id: physical.physical_blob_id.clone(),
            content_sha256: request.content_sha256.clone(),
            byte_len: request.byte_len,
            resource_kind: request.resource_kind,
            validation_profile: request.validation_profile.clone(),
            lifecycle_state: BindingLifecycle::Active,
            created_at_ms: request.now_ms,
            retired_at_ms: None,
        })
    }

    fn verify_physical_exact(&self, physical: &PhysicalBlobRecord) -> Result<(), BlobStoreError> {
        let metadata = self
            .provider
            .head_exact(&physical.object_locator)
            .map_err(provider_error)?
            .ok_or_else(|| {
                BlobStoreError::new("physical_blob_missing", "provider object missing")
            })?;
        if metadata.generation != physical.storage_generation {
            return Err(BlobStoreError::new(
                "storage_generation_mismatch",
                "provider object generation differs from durable metadata",
            ));
        }
        if metadata.byte_len != physical.byte_len {
            return Err(BlobStoreError::new(
                "blob_length_mismatch",
                "provider object length differs from durable metadata",
            ));
        }
        let mut sink = io::sink();
        self.copy_and_verify(physical, &mut sink)?;
        Ok(())
    }

    fn copy_and_verify(
        &self,
        physical: &PhysicalBlobRecord,
        output: &mut dyn Write,
    ) -> Result<u64, BlobStoreError> {
        let reader = self
            .provider
            .open_read(&physical.object_locator, &physical.storage_generation)
            .map_err(provider_error)?;
        let mut reader = HashingOwnedReader::new(reader, physical.byte_len);
        let mut buffer = [0_u8; COPY_BUFFER_BYTES];
        loop {
            let count = reader
                .read(&mut buffer)
                .map_err(|error| BlobStoreError::new("blob_read_failed", error.to_string()))?;
            if count == 0 {
                break;
            }
            output
                .write_all(&buffer[..count])
                .map_err(|error| BlobStoreError::new("blob_output_failed", error.to_string()))?;
        }
        reader.require_exact_eof(physical.byte_len)?;
        if reader.sha256_hex() != physical.content_sha256 {
            return Err(BlobStoreError::new(
                "blob_integrity_mismatch",
                "provider bytes do not match authorized binding content hash",
            ));
        }
        Ok(reader.bytes_read)
    }
}

fn validate_create_request(request: &CreateBindingRequest) -> Result<(), BlobStoreError> {
    require_ident(&request.tenant_id, "tenant_id")?;
    if let Some(project_id) = &request.project_id {
        require_ident(project_id, "project_id")?;
    }
    if let Some(document_id) = &request.document_id {
        require_ident(document_id, "document_id")?;
    }
    require_sha256(&request.content_sha256)?;
    if request.byte_len == 0 {
        return Err(BlobStoreError::new(
            "invalid_blob_length",
            "canonical blob length must be positive",
        ));
    }
    require_ident(&request.validation_profile, "validation_profile")?;
    if let Some(mime) = &request.canonical_mime {
        if mime.is_empty() || mime.len() > 256 || mime.chars().any(char::is_control) {
            return Err(BlobStoreError::new(
                "invalid_canonical_mime",
                "canonical MIME must be a bounded display token",
            ));
        }
    }
    Ok(())
}

fn assert_binding_matches_physical(
    binding: &ResourceBinding,
    physical: &PhysicalBlobRecord,
) -> Result<(), BlobStoreError> {
    if binding.tenant_id != physical.tenant_id
        || binding.physical_blob_id != physical.physical_blob_id
        || binding.content_sha256 != physical.content_sha256
        || binding.byte_len != physical.byte_len
        || physical.deleted
    {
        return Err(BlobStoreError::new(
            "binding_physical_mismatch",
            "resource binding and physical blob metadata diverge",
        ));
    }
    Ok(())
}

fn validate_provider_metadata(
    metadata: &ProviderObjectMetadata,
    expected_byte_len: u64,
) -> Result<(), BlobStoreError> {
    require_ident(&metadata.generation, "storage_generation")?;
    require_ident(&metadata.etag, "object_etag")?;
    if metadata.byte_len != expected_byte_len {
        return Err(BlobStoreError::new(
            "provider_length_mismatch",
            "provider write metadata differs from expected byte length",
        ));
    }
    Ok(())
}

fn validate_expiry(now_ms: u64, expires_at_ms: u64) -> Result<(), BlobStoreError> {
    if expires_at_ms <= now_ms {
        return Err(BlobStoreError::new(
            "invalid_grant_expiry",
            "grant expiry must be after issue time",
        ));
    }
    Ok(())
}

fn object_locator(namespace: BlobNamespace, tenant_id: &str, opaque_id: &str) -> String {
    format!("{}/{}/{}", namespace.path_segment(), tenant_id, opaque_id)
}

fn require_ident(value: &str, label: &'static str) -> Result<(), BlobStoreError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(BlobStoreError::new(
            "invalid_identifier",
            format!("{label} is not a bounded opaque identifier"),
        ));
    }
    Ok(())
}

fn require_sha256(value: &str) -> Result<(), BlobStoreError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(BlobStoreError::new(
            "invalid_content_hash",
            "content SHA-256 must be lowercase hexadecimal",
        ));
    }
    Ok(())
}

fn provider_error(error: ProviderError) -> BlobStoreError {
    let code = match error.kind {
        ProviderErrorKind::AlreadyExists => "provider_already_exists",
        ProviderErrorKind::UnknownOutcome => "provider_unknown_outcome",
        ProviderErrorKind::NotFound => "provider_not_found",
        ProviderErrorKind::AccessDenied => "provider_access_denied",
        ProviderErrorKind::Other => "provider_failure",
    };
    BlobStoreError::new(code, format!("provider outcome {}", error.code))
}

struct HashingBoundedReader<'a> {
    inner: &'a mut dyn Read,
    max_bytes: u64,
    bytes_read: u64,
    saw_eof: bool,
    hasher: Sha256,
}

impl<'a> HashingBoundedReader<'a> {
    fn new(inner: &'a mut dyn Read, max_bytes: u64) -> Self {
        Self {
            inner,
            max_bytes,
            bytes_read: 0,
            saw_eof: false,
            hasher: Sha256::new(),
        }
    }

    fn require_exact_eof(&self, expected: u64) -> Result<(), BlobStoreError> {
        if !self.saw_eof || self.bytes_read != expected {
            return Err(BlobStoreError::new(
                "blob_length_mismatch",
                "provider did not consume exact byte stream to EOF",
            ));
        }
        Ok(())
    }

    fn sha256_hex(&self) -> String {
        hex_digest(self.hasher.clone().finalize())
    }
}

impl Read for HashingBoundedReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = self.max_bytes.saturating_sub(self.bytes_read);
        let allowed = usize::try_from(remaining.saturating_add(1))
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        if allowed == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "blob write exceeds expected length",
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
                "blob write exceeds expected length",
            ));
        }
        self.hasher.update(&buffer[..count]);
        Ok(count)
    }
}

struct HashingOwnedReader {
    inner: Box<dyn Read + Send>,
    max_bytes: u64,
    bytes_read: u64,
    saw_eof: bool,
    hasher: Sha256,
}

impl HashingOwnedReader {
    fn new(inner: Box<dyn Read + Send>, max_bytes: u64) -> Self {
        Self {
            inner,
            max_bytes,
            bytes_read: 0,
            saw_eof: false,
            hasher: Sha256::new(),
        }
    }

    fn require_exact_eof(&self, expected: u64) -> Result<(), BlobStoreError> {
        if !self.saw_eof || self.bytes_read != expected {
            return Err(BlobStoreError::new(
                "blob_length_mismatch",
                "provider read differs from authorized binding length",
            ));
        }
        Ok(())
    }

    fn sha256_hex(&self) -> String {
        hex_digest(self.hasher.clone().finalize())
    }
}

impl Read for HashingOwnedReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = self.max_bytes.saturating_sub(self.bytes_read);
        let allowed = usize::try_from(remaining.saturating_add(1))
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        if allowed == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "blob read exceeds authorized length",
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
                "blob read exceeds authorized length",
            ));
        }
        self.hasher.update(&buffer[..count]);
        Ok(count)
    }
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

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        io::{Cursor, Read},
        sync::{
            Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    use super::*;

    #[derive(Default)]
    struct SequenceIds {
        physical: AtomicUsize,
        binding: AtomicUsize,
    }

    impl BlobIdGenerator for SequenceIds {
        fn next_physical_blob_id(&self) -> Result<String, BlobStoreError> {
            Ok(format!(
                "blob-{}",
                self.physical.fetch_add(1, Ordering::SeqCst)
            ))
        }

        fn next_binding_id(&self) -> Result<String, BlobStoreError> {
            Ok(format!(
                "binding-{}",
                self.binding.fetch_add(1, Ordering::SeqCst)
            ))
        }
    }

    #[derive(Default)]
    struct MemoryRepo {
        physical: Mutex<BTreeMap<String, PhysicalBlobRecord>>,
        bindings: Mutex<BTreeMap<String, ResourceBinding>>,
    }

    impl BlobBindingRepository for MemoryRepo {
        fn find_physical_by_content(
            &self,
            tenant_id: &str,
            content_sha256: &str,
            byte_len: u64,
        ) -> Result<Option<PhysicalBlobRecord>, BlobStoreError> {
            Ok(self
                .physical
                .lock()
                .unwrap()
                .values()
                .find(|record| {
                    record.tenant_id == tenant_id
                        && record.content_sha256 == content_sha256
                        && record.byte_len == byte_len
                        && !record.deleted
                })
                .cloned())
        }

        fn get_physical(
            &self,
            physical_blob_id: &str,
        ) -> Result<Option<PhysicalBlobRecord>, BlobStoreError> {
            Ok(self.physical.lock().unwrap().get(physical_blob_id).cloned())
        }

        fn get_binding(&self, binding_id: &str) -> Result<Option<ResourceBinding>, BlobStoreError> {
            Ok(self.bindings.lock().unwrap().get(binding_id).cloned())
        }

        fn commit_physical_and_binding(
            &self,
            physical: PhysicalBlobRecord,
            binding: ResourceBinding,
        ) -> Result<ResourceBinding, BlobStoreError> {
            if self
                .physical
                .lock()
                .unwrap()
                .contains_key(&physical.physical_blob_id)
            {
                return Err(BlobStoreError::new(
                    "physical_blob_collision",
                    "physical blob id already exists",
                ));
            }
            self.physical
                .lock()
                .unwrap()
                .insert(physical.physical_blob_id.clone(), physical);
            self.commit_binding(binding)
        }

        fn commit_binding(
            &self,
            binding: ResourceBinding,
        ) -> Result<ResourceBinding, BlobStoreError> {
            let mut bindings = self.bindings.lock().unwrap();
            if bindings.contains_key(&binding.binding_id) {
                return Err(BlobStoreError::new(
                    "binding_collision",
                    "binding id already exists",
                ));
            }
            bindings.insert(binding.binding_id.clone(), binding.clone());
            Ok(binding)
        }

        fn mark_physical_deleted(
            &self,
            physical_blob_id: &str,
            expected_generation: &str,
        ) -> Result<PhysicalBlobRecord, BlobStoreError> {
            let mut physical = self.physical.lock().unwrap();
            let record = physical.get_mut(physical_blob_id).ok_or_else(|| {
                BlobStoreError::new("physical_blob_missing", "physical blob missing")
            })?;
            if record.storage_generation != expected_generation {
                return Err(BlobStoreError::new(
                    "storage_generation_mismatch",
                    "delete generation differs",
                ));
            }
            record.deleted = true;
            Ok(record.clone())
        }
    }

    #[derive(Clone)]
    struct FakeObject {
        bytes: Vec<u8>,
        generation: String,
        etag: String,
    }

    struct FakeProvider {
        capabilities: ProviderCapabilities,
        objects: Mutex<BTreeMap<String, FakeObject>>,
        grant_secret: String,
        next_unknown_create: AtomicBool,
    }

    impl FakeProvider {
        fn new(capabilities: ProviderCapabilities) -> Self {
            Self {
                capabilities,
                objects: Mutex::new(BTreeMap::new()),
                grant_secret: "fake-secret".into(),
                next_unknown_create: AtomicBool::new(false),
            }
        }

        fn with_unknown_create(capabilities: ProviderCapabilities) -> Self {
            let provider = Self::new(capabilities);
            provider.next_unknown_create.store(true, Ordering::SeqCst);
            provider
        }

        fn grant_token(&self, request: &ProviderGrantRequest) -> String {
            let operation = match request.operation {
                GrantOperation::UploadCreateOnly => "upload",
                GrantOperation::Download => "download",
            };
            let payload = format!(
                "{}|{}|{}|{}|{}|{}",
                request.tenant_id,
                request.object_locator,
                operation,
                request.expected_byte_len.unwrap_or(0),
                request.expires_at_ms,
                self.grant_secret
            );
            format!("fake://{}", sha256_hex(payload.as_bytes()))
        }

        fn exercise_grant(
            &self,
            grant: &ProviderGrant,
            request: &ProviderGrantRequest,
            now_ms: u64,
        ) -> Result<(), ProviderError> {
            if now_ms >= grant.expires_at_ms {
                return Err(ProviderError::new(
                    ProviderErrorKind::AccessDenied,
                    "grant_expired",
                ));
            }
            if grant.opaque_url != self.grant_token(request)
                || grant.expires_at_ms != request.expires_at_ms
            {
                return Err(ProviderError::new(
                    ProviderErrorKind::AccessDenied,
                    "grant_scope_or_signature_mismatch",
                ));
            }
            Ok(())
        }
    }

    impl BlobProvider for FakeProvider {
        fn capabilities(&self) -> ProviderCapabilities {
            self.capabilities.clone()
        }

        fn create_immutable(
            &self,
            object_locator: &str,
            expected_byte_len: u64,
            input: &mut dyn Read,
        ) -> Result<ProviderObjectMetadata, ProviderError> {
            let mut bytes = Vec::new();
            input
                .read_to_end(&mut bytes)
                .map_err(|_| ProviderError::new(ProviderErrorKind::Other, "read_failed"))?;
            if bytes.len() as u64 != expected_byte_len {
                return Err(ProviderError::new(
                    ProviderErrorKind::Other,
                    "length_mismatch",
                ));
            }

            let mut objects = self.objects.lock().unwrap();
            if objects.contains_key(object_locator) {
                return Err(ProviderError::new(
                    ProviderErrorKind::AlreadyExists,
                    "precondition_failed",
                ));
            }
            let generation = format!("generation-{}", objects.len() + 1);
            let etag = sha256_hex(&bytes);
            objects.insert(
                object_locator.to_owned(),
                FakeObject {
                    bytes,
                    generation: generation.clone(),
                    etag: etag.clone(),
                },
            );
            let metadata = ProviderObjectMetadata {
                generation,
                byte_len: expected_byte_len,
                etag,
            };
            if self.next_unknown_create.swap(false, Ordering::SeqCst) {
                return Err(ProviderError::new(
                    ProviderErrorKind::UnknownOutcome,
                    "connection_lost_after_put",
                ));
            }
            Ok(metadata)
        }

        fn head_exact(
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
                    byte_len: object.bytes.len() as u64,
                    etag: object.etag.clone(),
                }))
        }

        fn open_read(
            &self,
            object_locator: &str,
            generation: &str,
        ) -> Result<Box<dyn Read + Send>, ProviderError> {
            let object = self
                .objects
                .lock()
                .unwrap()
                .get(object_locator)
                .cloned()
                .ok_or_else(|| ProviderError::new(ProviderErrorKind::NotFound, "not_found"))?;
            if object.generation != generation {
                return Err(ProviderError::new(
                    ProviderErrorKind::NotFound,
                    "generation_mismatch",
                ));
            }
            Ok(Box::new(Cursor::new(object.bytes)))
        }

        fn delete_exact(
            &self,
            object_locator: &str,
            generation: &str,
        ) -> Result<(), ProviderError> {
            let mut objects = self.objects.lock().unwrap();
            let object = objects
                .get(object_locator)
                .ok_or_else(|| ProviderError::new(ProviderErrorKind::NotFound, "not_found"))?;
            if object.generation != generation {
                return Err(ProviderError::new(
                    ProviderErrorKind::NotFound,
                    "generation_mismatch",
                ));
            }
            objects.remove(object_locator);
            Ok(())
        }

        fn issue_grant(
            &self,
            request: &ProviderGrantRequest,
        ) -> Result<ProviderGrant, ProviderError> {
            Ok(ProviderGrant {
                opaque_url: self.grant_token(request),
                expires_at_ms: request.expires_at_ms,
            })
        }
    }

    fn capabilities() -> ProviderCapabilities {
        ProviderCapabilities {
            hard_create_only: true,
            hard_exact_or_max_upload_size: true,
            signed_content_type: true,
            strong_head_after_put: true,
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        hex_digest(Sha256::digest(bytes))
    }

    fn service(provider: Arc<FakeProvider>) -> (BlobStoreService, Arc<MemoryRepo>) {
        let repo = Arc::new(MemoryRepo::default());
        let service =
            BlobStoreService::new(provider, repo.clone(), Arc::new(SequenceIds::default()));
        (service, repo)
    }

    fn create_request(tenant_id: &str, bytes: &[u8]) -> CreateBindingRequest {
        CreateBindingRequest {
            tenant_id: tenant_id.into(),
            project_id: Some(format!("project-{tenant_id}")),
            document_id: Some(format!("document-{tenant_id}")),
            content_sha256: sha256_hex(bytes),
            byte_len: bytes.len() as u64,
            canonical_mime: Some("application/x-mspublisher".into()),
            resource_kind: ResourceKind::PubSource,
            validation_profile: "pub-source-v1".into(),
            now_ms: 100,
        }
    }

    #[test]
    fn create_and_verified_read_start_from_tenant_binding_not_hash() {
        let provider = Arc::new(FakeProvider::new(capabilities()));
        let (service, _repo) = service(provider);
        let bytes = b"canonical-pub";
        let mut input = Cursor::new(bytes.to_vec());
        let binding = service
            .create_canonical_binding(create_request("tenant-a", bytes), &mut input)
            .unwrap();

        let mut output = Vec::new();
        service
            .stream_binding_verified("tenant-a", &binding.binding_id, &mut output)
            .unwrap();
        assert_eq!(output, bytes);

        let error = service
            .stream_binding_verified("tenant-b", &binding.binding_id, &mut Vec::new())
            .unwrap_err();
        assert_eq!(error.code, "cross_tenant_binding");
    }

    #[test]
    fn tenant_local_dedupe_reuses_physical_blob_but_not_binding_identity() {
        let provider = Arc::new(FakeProvider::new(capabilities()));
        let (service, repo) = service(provider);
        let bytes = b"same-content";

        let mut first_input = Cursor::new(bytes.to_vec());
        let first = service
            .create_canonical_binding(create_request("tenant-a", bytes), &mut first_input)
            .unwrap();
        let mut second_input = Cursor::new(bytes.to_vec());
        let second = service
            .create_canonical_binding(create_request("tenant-a", bytes), &mut second_input)
            .unwrap();

        assert_ne!(first.binding_id, second.binding_id);
        assert_eq!(first.physical_blob_id, second.physical_blob_id);
        assert_eq!(repo.physical.lock().unwrap().len(), 1);
        assert_eq!(repo.bindings.lock().unwrap().len(), 2);
    }

    #[test]
    fn cross_tenant_content_does_not_reuse_physical_blob() {
        let provider = Arc::new(FakeProvider::new(capabilities()));
        let (service, _repo) = service(provider);
        let bytes = b"same-content";

        let mut first_input = Cursor::new(bytes.to_vec());
        let first = service
            .create_canonical_binding(create_request("tenant-a", bytes), &mut first_input)
            .unwrap();
        let mut second_input = Cursor::new(bytes.to_vec());
        let second = service
            .create_canonical_binding(create_request("tenant-b", bytes), &mut second_input)
            .unwrap();

        assert_ne!(first.physical_blob_id, second.physical_blob_id);
    }

    #[test]
    fn unknown_create_outcome_reconciles_by_exact_head_and_read() {
        let provider = Arc::new(FakeProvider::with_unknown_create(capabilities()));
        let (service, _repo) = service(provider);
        let bytes = b"unknown-outcome";
        let mut input = Cursor::new(bytes.to_vec());

        let binding = service
            .create_canonical_binding(create_request("tenant-a", bytes), &mut input)
            .unwrap();

        let mut output = Vec::new();
        service
            .stream_binding_verified("tenant-a", &binding.binding_id, &mut output)
            .unwrap();
        assert_eq!(output, bytes);
    }

    #[test]
    fn direct_upload_requires_provider_hard_size_and_create_only_capabilities() {
        let mut weak = capabilities();
        weak.hard_exact_or_max_upload_size = false;
        let provider = Arc::new(FakeProvider::new(weak));
        let (service, _repo) = service(provider);

        let error = service
            .issue_quarantine_upload_grant(
                "tenant-a",
                "upload-1",
                123,
                Some("application/x-mspublisher".into()),
                100,
                200,
            )
            .unwrap_err();
        assert_eq!(error.code, "direct_upload_size_fence_unproven");
    }

    #[test]
    fn signed_grant_is_exactly_scoped_and_expires() {
        let provider = Arc::new(FakeProvider::new(capabilities()));
        let (service, _repo) = service(provider.clone());
        let grant = service
            .issue_quarantine_upload_grant(
                "tenant-a",
                "upload-1",
                123,
                Some("application/x-mspublisher".into()),
                100,
                200,
            )
            .unwrap();

        let exact = ProviderGrantRequest {
            tenant_id: "tenant-a".into(),
            object_locator: grant.object_locator.clone(),
            operation: GrantOperation::UploadCreateOnly,
            expected_byte_len: Some(123),
            required_content_type: Some("application/x-mspublisher".into()),
            expires_at_ms: 200,
        };
        provider
            .exercise_grant(
                &ProviderGrant {
                    opaque_url: grant.opaque_url.clone(),
                    expires_at_ms: grant.expires_at_ms,
                },
                &exact,
                150,
            )
            .unwrap();

        let mut substituted = exact.clone();
        substituted.object_locator = "quarantine/tenant-a/upload-2".into();
        assert!(
            provider
                .exercise_grant(
                    &ProviderGrant {
                        opaque_url: grant.opaque_url.clone(),
                        expires_at_ms: grant.expires_at_ms,
                    },
                    &substituted,
                    150,
                )
                .is_err()
        );
        assert!(
            provider
                .exercise_grant(
                    &ProviderGrant {
                        opaque_url: grant.opaque_url,
                        expires_at_ms: grant.expires_at_ms,
                    },
                    &exact,
                    200,
                )
                .is_err()
        );
    }

    #[test]
    fn provider_create_only_rejects_same_key_overwrite() {
        let provider = FakeProvider::new(capabilities());
        let locator = "quarantine/tenant-a/upload-1";
        let mut first = Cursor::new(b"one".to_vec());
        provider.create_immutable(locator, 3, &mut first).unwrap();

        let mut second = Cursor::new(b"two".to_vec());
        let error = provider
            .create_immutable(locator, 3, &mut second)
            .unwrap_err();
        assert_eq!(error.kind, ProviderErrorKind::AlreadyExists);

        let mut read = provider.open_read(locator, "generation-1").unwrap();
        let mut bytes = Vec::new();
        read.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"one");
    }
}
