use chaptera_update_trust::load_local_tuf_repository;
use std::path::PathBuf;
use tempfile::tempdir;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

#[tokio::test]
async fn accepts_signed_root_rotation_chain() {
    let base = fixtures().join("rotated-root");
    let trusted_root = tokio::fs::read(base.join("1.root.json")).await.unwrap();
    let datastore = tempdir().unwrap();

    let repo = load_local_tuf_repository(
        &trusted_root,
        &base,
        &base.join("targets"),
        datastore.path(),
    )
    .await
    .expect("rotated-root fixture must verify");

    assert_eq!(repo.root().signed.version.get(), 2);
}

#[tokio::test]
async fn rejects_expired_timestamp_metadata() {
    let base = fixtures().join("expired-repository");
    let metadata = base.join("metadata");
    let trusted_root = tokio::fs::read(metadata.join("1.root.json"))
        .await
        .unwrap();
    let datastore = tempdir().unwrap();

    let err = load_local_tuf_repository(
        &trusted_root,
        &metadata,
        &base.join("targets"),
        datastore.path(),
    )
    .await
    .expect_err("expired repository must fail closed");

    let rendered = format!("{err:#}").to_ascii_lowercase();
    assert!(
        rendered.contains("expired"),
        "expected expiry failure, got: {rendered}"
    );
}
