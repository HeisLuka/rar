use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use chaptera_server::{
    blob_store::{
        BindingLifecycle, BlobBindingRepository, BlobNamespace, PhysicalBlobRecord,
        ResourceBinding, ResourceKind,
    },
    schema_migration::SqliteMigrationRuntime,
    sqlite_blob_metadata::SqliteBlobBindingRepository,
};
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};

static NEXT_DB: AtomicU64 = AtomicU64::new(1);

fn temp_db(label: &str) -> PathBuf {
    let serial = NEXT_DB.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!(
        "chaptera-blob-metadata-{label}-{}-{serial}.sqlite",
        std::process::id()
    ))
}

fn cleanup(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}

async fn migrated_repo(label: &str) -> (SqliteBlobBindingRepository, PathBuf) {
    let path = temp_db(label);
    SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
        .unwrap()
        .migrate_up()
        .await
        .unwrap();
    let repo = SqliteBlobBindingRepository::open(&path, 2, Duration::from_secs(2))
        .await
        .unwrap();
    (repo, path)
}

fn physical(tenant: &str, id: &str, hash_ch: char) -> PhysicalBlobRecord {
    PhysicalBlobRecord {
        physical_blob_id: id.into(),
        tenant_id: tenant.into(),
        content_sha256: std::iter::repeat_n(hash_ch, 64).collect(),
        byte_len: 123,
        canonical_mime: Some("application/octet-stream".into()),
        object_namespace: BlobNamespace::Canonical,
        object_locator: format!("canonical/{tenant}/{id}"),
        storage_generation: "generation-1".into(),
        created_at_ms: 100,
        delete_eligible_at_ms: Some(1_000),
        deleted: false,
    }
}

fn binding(tenant: &str, binding_id: &str, physical: &PhysicalBlobRecord) -> ResourceBinding {
    ResourceBinding {
        binding_id: binding_id.into(),
        tenant_id: tenant.into(),
        project_id: Some("project-1".into()),
        document_id: Some("doc-1".into()),
        physical_blob_id: physical.physical_blob_id.clone(),
        content_sha256: physical.content_sha256.clone(),
        byte_len: physical.byte_len,
        resource_kind: ResourceKind::ExportArtifact,
        validation_profile: "export:idml:bounded-editable".into(),
        lifecycle_state: BindingLifecycle::Active,
        created_at_ms: 101,
        retired_at_ms: None,
    }
}

#[tokio::test]
async fn exact_retry_survives_restart_without_identity_drift() {
    let (repo, path) = migrated_repo("retry").await;
    let p = physical("tenant-a", "blob-1", 'a');
    let b = binding("tenant-a", "binding-1", &p);

    assert_eq!(
        repo.commit_physical_and_binding(p.clone(), b.clone())
            .await
            .unwrap(),
        b
    );
    assert_eq!(
        repo.commit_physical_and_binding(p.clone(), b.clone())
            .await
            .unwrap(),
        b
    );

    repo.close().await;
    let reopened = SqliteBlobBindingRepository::open(&path, 2, Duration::from_secs(2))
        .await
        .unwrap();

    assert_eq!(reopened.get_physical("blob-1").await.unwrap(), Some(p));
    assert_eq!(
        reopened.get_binding("binding-1").await.unwrap(),
        Some(b.clone())
    );
    assert_eq!(
        reopened
            .find_physical_by_content("tenant-a", &b.content_sha256, b.byte_len)
            .await
            .unwrap()
            .unwrap()
            .physical_blob_id,
        "blob-1"
    );

    reopened.close().await;
    cleanup(&path);
}

#[tokio::test]
async fn concurrent_exact_retry_converges_to_one_identity() {
    let (repo, path) = migrated_repo("concurrent-retry").await;
    let p = physical("tenant-a", "blob-1", 'f');
    let b = binding("tenant-a", "binding-1", &p);

    let left_repo = repo.clone();
    let right_repo = repo.clone();
    let left_p = p.clone();
    let right_p = p.clone();
    let left_b = b.clone();
    let right_b = b.clone();

    let (left, right) = tokio::join!(
        async move {
            left_repo
                .commit_physical_and_binding(left_p, left_b)
                .await
        },
        async move {
            right_repo
                .commit_physical_and_binding(right_p, right_b)
                .await
        }
    );

    assert_eq!(left.unwrap(), b);
    assert_eq!(right.unwrap(), b);
    assert_eq!(
        repo.get_physical("blob-1").await.unwrap(),
        Some(p)
    );
    assert_eq!(
        repo.get_binding("binding-1").await.unwrap(),
        Some(b)
    );

    repo.close().await;
    cleanup(&path);
}

#[tokio::test]
async fn changed_retry_and_cross_tenant_alias_fail_closed() {
    let (repo, path) = migrated_repo("conflict").await;
    let p = physical("tenant-a", "blob-1", 'b');
    let b = binding("tenant-a", "binding-1", &p);
    repo.commit_physical_and_binding(p.clone(), b.clone())
        .await
        .unwrap();

    let mut changed = b.clone();
    changed.document_id = Some("doc-other".into());
    let error = repo.commit_binding(changed).await.unwrap_err();
    assert_eq!(error.code, "binding_collision");

    let mut cross_tenant = binding("tenant-b", "binding-2", &p);
    cross_tenant.content_sha256 = p.content_sha256.clone();
    let error = repo.commit_binding(cross_tenant).await.unwrap_err();
    assert_eq!(error.code, "binding_physical_mismatch");

    assert!(
        repo.find_physical_by_content("tenant-b", &p.content_sha256, p.byte_len)
            .await
            .unwrap()
            .is_none()
    );

    let p2 = physical("tenant-b", "blob-2", 'b');
    let b2 = binding("tenant-b", "binding-2", &p2);
    repo.commit_physical_and_binding(p2.clone(), b2)
        .await
        .unwrap();
    assert_ne!(
        repo.find_physical_by_content("tenant-a", &p.content_sha256, p.byte_len)
            .await
            .unwrap()
            .unwrap()
            .physical_blob_id,
        repo.find_physical_by_content("tenant-b", &p2.content_sha256, p2.byte_len)
            .await
            .unwrap()
            .unwrap()
            .physical_blob_id
    );

    repo.close().await;
    cleanup(&path);
}

#[tokio::test]
async fn binding_conflict_rolls_back_new_physical_without_orphan_metadata() {
    let (repo, path) = migrated_repo("rollback").await;

    let first_physical = physical("tenant-a", "blob-1", 'd');
    let first_binding = binding("tenant-a", "binding-1", &first_physical);
    repo.commit_physical_and_binding(first_physical.clone(), first_binding.clone())
        .await
        .unwrap();

    let second_physical = physical("tenant-a", "blob-2", 'e');
    let conflicting_binding = binding("tenant-a", "binding-1", &second_physical);
    let error = repo
        .commit_physical_and_binding(second_physical.clone(), conflicting_binding)
        .await
        .unwrap_err();
    assert_eq!(error.code, "binding_collision");

    assert_eq!(
        repo.get_binding("binding-1").await.unwrap(),
        Some(first_binding)
    );
    assert!(
        repo.get_physical("blob-2").await.unwrap().is_none(),
        "failed binding commit must roll back the newly inserted physical metadata"
    );
    assert!(
        repo.find_physical_by_content(
            "tenant-a",
            &second_physical.content_sha256,
            second_physical.byte_len,
        )
        .await
        .unwrap()
        .is_none()
    );

    repo.close().await;
    cleanup(&path);
}

#[tokio::test]
async fn generation_fenced_delete_is_idempotent() {
    let (repo, path) = migrated_repo("delete").await;
    let p = physical("tenant-a", "blob-1", 'c');
    let b = binding("tenant-a", "binding-1", &p);
    repo.commit_physical_and_binding(p, b).await.unwrap();

    let error = repo
        .mark_physical_deleted("blob-1", "generation-stale")
        .await
        .unwrap_err();
    assert_eq!(error.code, "storage_generation_mismatch");

    let deleted = repo
        .mark_physical_deleted("blob-1", "generation-1")
        .await
        .unwrap();
    assert!(deleted.deleted);

    let retry = repo
        .mark_physical_deleted("blob-1", "generation-1")
        .await
        .unwrap();
    assert_eq!(retry, deleted);

    repo.close().await;
    cleanup(&path);
}

#[tokio::test]
async fn unmigrated_open_fails_without_bootstrapping_schema() {
    let path = temp_db("unmigrated");
    fs::File::create(&path).unwrap();

    let error = match SqliteBlobBindingRepository::open(&path, 1, Duration::from_secs(1)).await {
        Ok(_) => panic!("unmigrated blob metadata repository must fail closed"),
        Err(error) => error,
    };
    assert_eq!(error.code, "sqlite_schema_missing");

    let pool = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(false),
    )
    .await
    .unwrap();
    let user_tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('physical_blobs','resource_bindings')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(user_tables, 0);
    pool.close().await;

    cleanup(&path);
}
