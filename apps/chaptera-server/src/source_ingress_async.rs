use std::{
    fmt,
    pin::Pin,
    task::{Context, Poll},
};

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, ReadBuf};

use crate::{
    blob_store::{BlobStoreError, BlobStoreService, CreateBindingRequest, ResourceKind},
    source_ingress::{IngressError, UploadRecord, UploadState},
    source_ingress_sqlite::SqliteSourceIngressRepository,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSecurityScanReceipt {
    pub validation_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceSecurityScanOutcome {
    Accepted(SourceSecurityScanReceipt),
    Rejected { code: &'static str },
}

#[async_trait]
pub trait AsyncSourceSecurityScanner: Send + Sync {
    /// Inspect one exact quarantined source stream.
    ///
    /// Accepted scanners must consume the stream to EOF. Early EOF-free return
    /// is rejected by the runtime so an incomplete scanner can never authorize
    /// durable source promotion.
    async fn scan(
        &self,
        input: &mut (dyn AsyncRead + Unpin + Send),
    ) -> Result<SourceSecurityScanOutcome, IngressError>;
}

#[derive(Clone)]
pub struct AsyncSourceValidationRuntime {
    repo: SqliteSourceIngressRepository,
    blob_store: BlobStoreService,
}

impl AsyncSourceValidationRuntime {
    pub fn new(repo: SqliteSourceIngressRepository, blob_store: BlobStoreService) -> Self {
        Self { repo, blob_store }
    }

    pub async fn validate_and_promote(
        &self,
        tenant_id: &str,
        upload_id: &str,
        now_ms: u64,
        scanner: &dyn AsyncSourceSecurityScanner,
    ) -> Result<UploadRecord, IngressError> {
        require_ident(tenant_id, "tenant_id")?;
        require_ident(upload_id, "upload_id")?;

        let mut upload = self
            .repo
            .get(upload_id)
            .await?
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
                next.upload_generation =
                    next.upload_generation.checked_add(1).ok_or_else(|| {
                        IngressError::new(
                            "upload_generation_overflow",
                            "upload generation cannot advance",
                        )
                    })?;
                upload = self
                    .repo
                    .compare_and_swap(&upload.upload_id, upload.upload_generation, next)
                    .await?;
            }
            UploadState::Validating => {}
        }

        let storage_generation = upload.object_version.as_deref().ok_or_else(|| {
            IngressError::new(
                "object_identity_missing",
                "validation object version missing",
            )
        })?;
        let object_etag = upload.object_etag.as_deref().ok_or_else(|| {
            IngressError::new("object_identity_missing", "validation object etag missing")
        })?;
        let observed_len = upload.observed_byte_len.ok_or_else(|| {
            IngressError::new(
                "object_identity_missing",
                "validation object length missing",
            )
        })?;
        if observed_len == 0 || observed_len != upload.expected_byte_len {
            return Err(IngressError::new(
                "validated_length_mismatch",
                "completed quarantine length differs from the upload reservation",
            ));
        }

        let first = self
            .blob_store
            .open_quarantine_exact(
                &upload.tenant_id,
                &upload.upload_id,
                storage_generation,
                object_etag,
                observed_len,
            )
            .await
            .map_err(blob_error)?;

        let mut inspected = AsyncHashingBoundedReader::new(first, upload.expected_byte_len);
        let scan_outcome = scanner.scan(&mut inspected).await?;
        let scan_receipt = match scan_outcome {
            SourceSecurityScanOutcome::Accepted(receipt) => {
                inspected.require_consumed_exact(observed_len)?;
                require_ident(&receipt.validation_profile, "validation_profile")?;
                receipt
            }
            SourceSecurityScanOutcome::Rejected { code } => {
                require_terminal_code(code)?;
                return self.reject(upload, code, now_ms).await;
            }
        };
        let canonical_sha256 = inspected.sha256_hex();

        let second = self
            .blob_store
            .open_quarantine_exact(
                &upload.tenant_id,
                &upload.upload_id,
                storage_generation,
                object_etag,
                observed_len,
            )
            .await
            .map_err(blob_error)?;
        let mut promotion_input = second;
        let binding = self
            .blob_store
            .create_canonical_binding(
                CreateBindingRequest {
                    tenant_id: upload.tenant_id.clone(),
                    project_id: None,
                    document_id: None,
                    content_sha256: canonical_sha256.clone(),
                    byte_len: observed_len,
                    canonical_mime: None,
                    resource_kind: ResourceKind::PubSource,
                    validation_profile: scan_receipt.validation_profile,
                    now_ms,
                },
                &mut *promotion_input,
            )
            .await
            .map_err(blob_error)?;

        if binding.content_sha256 != canonical_sha256 || binding.byte_len != observed_len {
            return Err(IngressError::new(
                "durable_binding_hash_mismatch",
                "durable source binding differs from inspected source identity",
            ));
        }

        let mut next = upload.clone();
        next.state = UploadState::ValidatedDurable;
        next.upload_generation = next.upload_generation.checked_add(1).ok_or_else(|| {
            IngressError::new(
                "upload_generation_overflow",
                "upload generation cannot advance",
            )
        })?;
        next.canonical_sha256 = Some(canonical_sha256);
        next.durable_binding_id = Some(binding.binding_id);
        next.completed_at_ms = Some(now_ms);
        next.terminal_code = None;

        self.repo
            .compare_and_swap(&upload.upload_id, upload.upload_generation, next)
            .await
    }

    async fn reject(
        &self,
        upload: UploadRecord,
        code: &'static str,
        now_ms: u64,
    ) -> Result<UploadRecord, IngressError> {
        let mut next = upload.clone();
        next.state = UploadState::Rejected;
        next.upload_generation = next.upload_generation.checked_add(1).ok_or_else(|| {
            IngressError::new(
                "upload_generation_overflow",
                "upload generation cannot advance",
            )
        })?;
        next.completed_at_ms = Some(now_ms);
        next.terminal_code = Some(code.to_owned());
        self.repo
            .compare_and_swap(&upload.upload_id, upload.upload_generation, next)
            .await
    }
}

fn blob_error(error: BlobStoreError) -> IngressError {
    IngressError::new(error.code, error.message)
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

fn require_terminal_code(value: &str) -> Result<(), IngressError> {
    if value.is_empty()
        || value.len() > 96
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(IngressError::new(
            "invalid_terminal_code",
            "validation rejection code is not bounded",
        ));
    }
    Ok(())
}

struct AsyncHashingBoundedReader {
    inner: Box<dyn AsyncRead + Unpin + Send>,
    max_bytes: u64,
    bytes_read: u64,
    saw_eof: bool,
    hasher: Sha256,
}

impl fmt::Debug for AsyncHashingBoundedReader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AsyncHashingBoundedReader")
            .field("max_bytes", &self.max_bytes)
            .field("bytes_read", &self.bytes_read)
            .field("saw_eof", &self.saw_eof)
            .finish_non_exhaustive()
    }
}

impl AsyncHashingBoundedReader {
    fn new(inner: Box<dyn AsyncRead + Unpin + Send>, max_bytes: u64) -> Self {
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
                "security scanner did not consume exact quarantine object to EOF",
            ));
        }
        if self.bytes_read != expected {
            return Err(IngressError::new(
                "validated_length_mismatch",
                "security scanner byte count differs from completed object",
            ));
        }
        Ok(())
    }

    fn sha256_hex(&self) -> String {
        format!("{:x}", self.hasher.clone().finalize())
    }
}

impl AsyncRead for AsyncHashingBoundedReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buffer.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buffer);
        let Poll::Ready(result) = poll else {
            return Poll::Pending;
        };
        result?;

        let after = buffer.filled().len();
        let count = after.saturating_sub(before);
        if count == 0 {
            self.saw_eof = true;
            return Poll::Ready(Ok(()));
        }

        let count_u64 = u64::try_from(count)
            .map_err(|_| std::io::Error::other("async source byte count overflow"))?;
        let next = self
            .bytes_read
            .checked_add(count_u64)
            .ok_or_else(|| std::io::Error::other("async source byte count overflow"))?;
        if next > self.max_bytes {
            return Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "bounded async validation overflow",
            )));
        }
        self.hasher.update(&buffer.filled()[before..after]);
        self.bytes_read = next;
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, duplex};

    use super::*;

    #[tokio::test]
    async fn hashing_reader_requires_exact_eof_and_preserves_sha() {
        let (mut writer, reader) = duplex(64);
        let write = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            writer.write_all(b"publisher").await.unwrap();
        });

        let mut hashing = AsyncHashingBoundedReader::new(Box::new(reader), 9);
        let mut bytes = Vec::new();
        hashing.read_to_end(&mut bytes).await.unwrap();
        write.await.unwrap();

        assert_eq!(bytes, b"publisher");
        hashing.require_consumed_exact(9).unwrap();
        assert_eq!(
            hashing.sha256_hex(),
            format!("{:x}", Sha256::digest(b"publisher"))
        );
    }

    #[tokio::test]
    async fn accepted_scanner_must_consume_to_eof() {
        struct EarlyScanner;

        #[async_trait]
        impl AsyncSourceSecurityScanner for EarlyScanner {
            async fn scan(
                &self,
                input: &mut (dyn AsyncRead + Unpin + Send),
            ) -> Result<SourceSecurityScanOutcome, IngressError> {
                let mut byte = [0_u8; 1];
                use tokio::io::AsyncReadExt;
                input
                    .read_exact(&mut byte)
                    .await
                    .map_err(|error| IngressError::new("scanner_read_failed", error.to_string()))?;
                Ok(SourceSecurityScanOutcome::Accepted(
                    SourceSecurityScanReceipt {
                        validation_profile: "chaptera-untrusted-pub-v1".to_owned(),
                    },
                ))
            }
        }

        let (mut writer, reader) = duplex(64);
        let write = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            writer.write_all(b"publisher").await.unwrap();
        });
        let mut hashing = AsyncHashingBoundedReader::new(Box::new(reader), 9);
        let outcome = EarlyScanner.scan(&mut hashing).await.unwrap();
        assert!(matches!(outcome, SourceSecurityScanOutcome::Accepted(_)));
        assert_eq!(
            hashing.require_consumed_exact(9).unwrap_err().code,
            "validator_incomplete_read"
        );
        drop(hashing);
        write.await.unwrap();
    }
}
