use std::{fmt::Write as _, sync::Arc, time::Duration};

use aws_config::BehaviorVersion;
use rand::{RngCore, rngs::OsRng};

use crate::{
    blob_store::{BlobIdGenerator, BlobStoreError, BlobStoreService},
    config::ChapteraConfig,
    s3_blob_provider::S3BlobProvider,
    sqlite_blob_metadata::SqliteBlobBindingRepository,
};

const RANDOM_ID_BYTES: usize = 16;

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemBlobIdGenerator;

impl BlobIdGenerator for SystemBlobIdGenerator {
    fn next_physical_blob_id(&self) -> Result<String, BlobStoreError> {
        random_id("blob")
    }

    fn next_binding_id(&self) -> Result<String, BlobStoreError> {
        random_id("binding")
    }
}

#[derive(Clone)]
pub struct BlobStoreRuntime {
    service: BlobStoreService,
    metadata: SqliteBlobBindingRepository,
}

impl BlobStoreRuntime {
    pub async fn open(config: &ChapteraConfig) -> Result<Self, BlobStoreError> {
        require_s3_compatible(&config.storage.provider)?;

        let metadata = SqliteBlobBindingRepository::open(
            &config.sqlite.path,
            config.sqlite.pool_max,
            Duration::from_millis(config.sqlite.busy_timeout_ms),
        )
        .await?;

        let shared_config = aws_config::load_defaults(BehaviorVersion::latest()).await;
        let client = aws_sdk_s3::Client::new(&shared_config);
        let provider = S3BlobProvider::new(
            client,
            config.storage.quarantine_namespace.clone(),
            config.storage.private_namespace.clone(),
            None,
        )
        .map_err(|error| {
            BlobStoreError::new("blob_provider_config_invalid", bounded_message(&error.code))
        })?;

        let service = BlobStoreService::new(
            Arc::new(provider),
            Arc::new(metadata.clone()),
            Arc::new(SystemBlobIdGenerator),
        );

        Ok(Self { service, metadata })
    }

    pub fn service(&self) -> &BlobStoreService {
        &self.service
    }

    pub fn metadata(&self) -> &SqliteBlobBindingRepository {
        &self.metadata
    }

    pub async fn close(&self) {
        self.metadata.close().await;
    }
}

fn require_s3_compatible(provider: &str) -> Result<(), BlobStoreError> {
    if provider == "s3-compatible" {
        Ok(())
    } else {
        Err(BlobStoreError::new(
            "storage_provider_unsupported",
            format!("configured storage provider {provider:?} is unsupported by Cloud V0 runtime"),
        ))
    }
}

fn random_id(prefix: &'static str) -> Result<String, BlobStoreError> {
    let mut bytes = [0_u8; RANDOM_ID_BYTES];
    OsRng.try_fill_bytes(&mut bytes).map_err(|error| {
        BlobStoreError::new("blob_id_random_failed", bounded_message(&error.to_string()))
    })?;

    let mut out = String::with_capacity(prefix.len() + 1 + RANDOM_ID_BYTES * 2);
    out.push_str(prefix);
    out.push(':');
    for byte in bytes {
        write!(&mut out, "{byte:02x}")
            .expect("writing random blob identifier into String cannot fail");
    }
    Ok(out)
}

fn bounded_message(message: &str) -> String {
    message.chars().take(512).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn system_ids_are_bounded_opaque_and_do_not_cross_identity_kinds() {
        let ids = SystemBlobIdGenerator;
        let mut physical = BTreeSet::new();
        let mut bindings = BTreeSet::new();

        for _ in 0..128 {
            let physical_id = ids.next_physical_blob_id().unwrap();
            let binding_id = ids.next_binding_id().unwrap();

            assert!(physical_id.starts_with("blob:"));
            assert!(binding_id.starts_with("binding:"));
            assert_eq!(physical_id.len(), "blob:".len() + RANDOM_ID_BYTES * 2);
            assert_eq!(binding_id.len(), "binding:".len() + RANDOM_ID_BYTES * 2);
            assert!(
                physical_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b':')
            );
            assert!(
                binding_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b':')
            );
            assert!(physical.insert(physical_id));
            assert!(bindings.insert(binding_id));
        }
    }

    #[test]
    fn unsupported_storage_provider_fails_closed_before_runtime_construction() {
        let error = require_s3_compatible("filesystem").unwrap_err();
        assert_eq!(error.code, "storage_provider_unsupported");
    }
}
