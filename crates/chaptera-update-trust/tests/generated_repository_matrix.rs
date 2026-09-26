use chaptera_update_trust::load_local_tuf_repository;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;
use tough::{IntoVec, TargetName};
use url::Url;

fn run<I, S>(args: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args: Vec<_> = args.into_iter().map(|v| v.as_ref().to_owned()).collect();
    let status = Command::new("tuftool")
        .args(&args)
        .status()
        .expect("tuftool must be installed for ignored generated-matrix test");
    assert!(status.success(), "tuftool failed for args: {args:?}");
}

fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        // tuftool emits alias symlinks for some repository entries. Snapshot
        // copies materialize the referenced bytes instead of retaining links so
        // v1/v2 fixture directories remain self-contained.
        let metadata = fs::metadata(&src_path).unwrap();
        if metadata.is_dir() {
            copy_tree(&src_path, &dst_path);
        } else if metadata.is_file() {
            fs::copy(src_path, dst_path).unwrap();
        } else {
            panic!("unexpected repository entry");
        }
    }
}

fn overwrite_regular_files(dir: &Path, bytes: &[u8]) -> usize {
    let mut overwritten = 0;
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let ty = entry.file_type().unwrap();
        if ty.is_dir() {
            overwritten += overwrite_regular_files(&path, bytes);
        } else if ty.is_file() {
            fs::write(path, bytes).unwrap();
            overwritten += 1;
        }
    }
    overwritten
}

struct GeneratedRepository {
    trusted_root: Vec<u8>,
    metadata_v1: PathBuf,
    targets_v1: PathBuf,
    metadata_v2: PathBuf,
    targets_v2: PathBuf,
}

fn generate_repository_versions(wrk: &Path) -> GeneratedRepository {
    let root_dir = wrk.join("root");
    let keys_dir = wrk.join("keys");
    let input_dir = wrk.join("input");
    let repo_dir = wrk.join("repo");
    let metadata_dir = repo_dir.join("metadata");
    let targets_dir = repo_dir.join("targets");

    fs::create_dir_all(&root_dir).unwrap();
    fs::create_dir_all(&keys_dir).unwrap();
    fs::create_dir_all(&input_dir).unwrap();

    let root = root_dir.join("root.json");
    let key = keys_dir.join("root.pem");
    let payload = input_dir.join("payload.bin");
    fs::write(&payload, b"chaptera-update-payload-v1").unwrap();

    run(["root", "init", root.to_str().unwrap()]);
    run(["root", "expire", root.to_str().unwrap(), "in 6 weeks"]);
    for role in ["root", "snapshot", "targets", "timestamp"] {
        run([
            "root",
            "set-threshold",
            root.to_str().unwrap(),
            role,
            "1",
        ]);
    }
    run([
        "root",
        "gen-rsa-key",
        root.to_str().unwrap(),
        key.to_str().unwrap(),
        "--role",
        "root",
    ]);
    for role in ["snapshot", "targets", "timestamp"] {
        run([
            "root",
            "add-key",
            root.to_str().unwrap(),
            "-k",
            key.to_str().unwrap(),
            "--role",
            role,
        ]);
    }
    run([
        "root",
        "sign",
        root.to_str().unwrap(),
        "-k",
        key.to_str().unwrap(),
    ]);

    run([
        "create",
        "--root",
        root.to_str().unwrap(),
        "--key",
        key.to_str().unwrap(),
        "--add-targets",
        input_dir.to_str().unwrap(),
        "--targets-expires",
        "in 3 weeks",
        "--targets-version",
        "1",
        "--snapshot-expires",
        "in 3 weeks",
        "--snapshot-version",
        "1",
        "--timestamp-expires",
        "in 1 week",
        "--timestamp-version",
        "1",
        "--outdir",
        repo_dir.to_str().unwrap(),
    ]);

    let metadata_v1 = wrk.join("metadata-v1");
    let targets_v1 = wrk.join("targets-v1");
    copy_tree(&metadata_dir, &metadata_v1);
    copy_tree(&targets_dir, &targets_v1);

    fs::write(&payload, b"chaptera-update-payload-v2").unwrap();
    let metadata_url = Url::from_directory_path(&metadata_dir).unwrap().to_string();
    run([
        "update",
        "--root",
        root.to_str().unwrap(),
        "--key",
        key.to_str().unwrap(),
        "--add-targets",
        input_dir.to_str().unwrap(),
        "--targets-expires",
        "in 3 weeks",
        "--targets-version",
        "2",
        "--snapshot-expires",
        "in 3 weeks",
        "--snapshot-version",
        "2",
        "--timestamp-expires",
        "in 1 week",
        "--timestamp-version",
        "2",
        "--outdir",
        repo_dir.to_str().unwrap(),
        "--metadata-url",
        &metadata_url,
    ]);

    let metadata_v2 = wrk.join("metadata-v2");
    let targets_v2 = wrk.join("targets-v2");
    copy_tree(&metadata_dir, &metadata_v2);
    copy_tree(&targets_dir, &targets_v2);

    GeneratedRepository {
        trusted_root: fs::read(&root).unwrap(),
        metadata_v1,
        targets_v1,
        metadata_v2,
        targets_v2,
    }
}

#[tokio::test]
#[ignore = "requires pinned tuftool; run in Chaptera update metadata trust CI"]
async fn persistent_datastore_rejects_replay_and_target_substitution() {
    let temp = tempdir().unwrap();
    let wrk = temp.path();
    let generated = generate_repository_versions(wrk);
    let datastore = wrk.join("datastore");

    let repo_v1 = load_local_tuf_repository(
        &generated.trusted_root,
        &generated.metadata_v1,
        &generated.targets_v1,
        &datastore,
    )
    .await
    .expect("v1 repository must verify");
    assert_eq!(repo_v1.timestamp().signed.version.get(), 1);

    let repo_v2 = load_local_tuf_repository(
        &generated.trusted_root,
        &generated.metadata_v2,
        &generated.targets_v2,
        &datastore,
    )
    .await
    .expect("v2 repository must verify using same datastore");
    assert_eq!(repo_v2.timestamp().signed.version.get(), 2);

    let target_name = TargetName::new("payload.bin").unwrap();
    let stream = repo_v2
        .read_target(&target_name)
        .await
        .expect("target lookup should succeed")
        .expect("target must exist");
    let bytes = stream.into_vec().await.expect("valid target bytes");
    assert_eq!(bytes, b"chaptera-update-payload-v2");

    let tampered = overwrite_regular_files(&generated.targets_v2, b"tampered-target-bytes");
    assert!(tampered > 0, "generated target snapshot must contain target bytes");
    match repo_v2.read_target(&target_name).await {
        Err(_) => {}
        Ok(Some(stream)) => {
            assert!(
                stream.into_vec().await.is_err(),
                "tampered target bytes must fail hash/length verification"
            );
        }
        Ok(None) => panic!("target unexpectedly disappeared"),
    }

    let replay = load_local_tuf_repository(
        &generated.trusted_root,
        &generated.metadata_v1,
        &generated.targets_v1,
        &datastore,
    )
    .await;
    assert!(
        replay.is_err(),
        "same datastore must reject replay of older signed metadata"
    );
}

#[tokio::test]
#[ignore = "requires pinned tuftool; run in Chaptera update metadata trust CI"]
async fn high_water_survives_binary_updater_rollback() {
    let temp = tempdir().unwrap();
    let wrk = temp.path();
    let generated = generate_repository_versions(wrk);

    let product_root = wrk.join("product");
    let current = product_root.join("current");
    let staging = product_root.join(".staging");
    let rollback = product_root.join(".rollback");
    let durable_datastore = wrk.join("trusted-metadata");

    assert!(
        !durable_datastore.starts_with(&product_root),
        "trusted metadata high-water must live outside the versioned product tree"
    );

    // These marker files model the updater binary identity inside each versioned
    // product tree; the trust datastore deliberately lives outside both trees.
    fs::create_dir_all(&current).unwrap();
    fs::write(current.join("updater.version"), b"U1").unwrap();

    assert_eq!(
        fs::read_to_string(current.join("updater.version")).unwrap(),
        "U1"
    );
    let repo_v1 = load_local_tuf_repository(
        &generated.trusted_root,
        &generated.metadata_v1,
        &generated.targets_v1,
        &durable_datastore,
    )
    .await
    .expect("U1 must accept repository v1");
    assert_eq!(repo_v1.timestamp().signed.version.get(), 1);

    fs::create_dir_all(&staging).unwrap();
    fs::write(staging.join("updater.version"), b"U2").unwrap();

    fs::rename(&current, &rollback).unwrap();
    fs::rename(&staging, &current).unwrap();
    assert_eq!(
        fs::read_to_string(current.join("updater.version")).unwrap(),
        "U2"
    );

    let repo_v2 = load_local_tuf_repository(
        &generated.trusted_root,
        &generated.metadata_v2,
        &generated.targets_v2,
        &durable_datastore,
    )
    .await
    .expect("U2 must accept repository v2 using the same durable datastore");
    assert_eq!(repo_v2.timestamp().signed.version.get(), 2);

    let high_water_entries = fs::read_dir(&durable_datastore).unwrap().count();
    assert!(
        high_water_entries > 0,
        "durable datastore must contain trusted metadata after v2 acceptance"
    );

    // Simulate product rollback: the candidate/U2 tree leaves current and the
    // previously confirmed U1 tree becomes current again. The durable trust
    // datastore is deliberately untouched because it is not under product_root.
    fs::rename(&current, &staging).unwrap();
    fs::rename(&rollback, &current).unwrap();

    assert_eq!(
        fs::read_to_string(current.join("updater.version")).unwrap(),
        "U1",
        "binary rollback must restore the older updater"
    );
    assert_eq!(
        fs::read_dir(&durable_datastore).unwrap().count(),
        high_water_entries,
        "binary rollback must not roll back trusted metadata state"
    );

    let replay_from_rolled_back_u1 = load_local_tuf_repository(
        &generated.trusted_root,
        &generated.metadata_v1,
        &generated.targets_v1,
        &durable_datastore,
    )
    .await;
    assert!(
        replay_from_rolled_back_u1.is_err(),
        "rolled-back U1 must still reject signed metadata older than persisted high-water"
    );
}
