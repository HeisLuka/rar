use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chaptera_server::{
    quota_store::{
        QuotaConfig, QuotaWorkClass, ReserveOutcome, ReserveRequest, SqliteQuotaAuthority,
    },
    schema_migration::SqliteMigrationRuntime,
};

const CHILD_ENV: &str = "CHAPTERA_QUOTA_CRASH_CHILD";
const PATH_ENV: &str = "CHAPTERA_QUOTA_CRASH_PATH";

fn temp_db(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    env::temp_dir().join(format!(
        "chaptera-quota-{label}-{}-{stamp}.sqlite",
        std::process::id()
    ))
}

fn cleanup(path: &Path) {
    for candidate in [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ] {
        let _ = fs::remove_file(candidate);
    }
}

fn config() -> QuotaConfig {
    QuotaConfig {
        shared_capacity: 10,
        semantic_headroom: 5,
        export_cap: 4,
        background_cap: 4,
    }
}

fn request() -> ReserveRequest {
    ReserveRequest {
        tenant_id: "tenant-crash".to_owned(),
        reservation_id: "reservation-crash-1".to_owned(),
        work_class: QuotaWorkClass::Export,
        amount: 2,
        request_hash: "a".repeat(64),
    }
}

#[test]
#[ignore = "spawned only by quota process crash acceptance parent"]
fn child_reserves_then_aborts_before_release_or_ack() {
    if env::var_os(CHILD_ENV).is_none() {
        return;
    }

    let path = PathBuf::from(env::var(PATH_ENV).expect("quota crash child database path"));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async {
        let authority = SqliteQuotaAuthority::open(&path, 2, Duration::from_secs(2), config())
            .await
            .unwrap();

        let outcome = authority
            .reserve(request(), 1_000, Duration::from_secs(30))
            .await
            .unwrap();
        assert!(matches!(outcome, ReserveOutcome::Reserved(_)));

        // Deliberately skip close/release/ACK. synchronous=FULL + COMMIT has
        // returned to the authority, then the process dies immediately.
        std::process::abort();
    });
}

#[test]
fn process_crash_after_reserve_recovers_exact_active_reservation() {
    let path = temp_db("reserve-before-ack");

    let migration_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    migration_runtime.block_on(async {
        SqliteMigrationRuntime::new(&path, Duration::from_secs(2))
            .unwrap()
            .migrate_up()
            .await
            .unwrap();
    });
    drop(migration_runtime);

    let child = Command::new(env::current_exe().unwrap())
        .args([
            "--exact",
            "child_reserves_then_aborts_before_release_or_ack",
            "--ignored",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .env(PATH_ENV, &path)
        .status()
        .expect("spawn quota crash-boundary child");

    assert!(
        !child.success(),
        "quota crash child unexpectedly returned success instead of dying after reserve"
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async {
        let reopened = SqliteQuotaAuthority::open(&path, 2, Duration::from_secs(2), config())
            .await
            .unwrap();

        let usage = reopened.usage("tenant-crash", 1_001).await.unwrap();
        assert_eq!(usage.export, 2);

        let retry = reopened
            .reserve(request(), 1_001, Duration::from_secs(30))
            .await
            .unwrap();
        let recovered = match retry {
            ReserveOutcome::Existing(row) => row,
            ReserveOutcome::Reserved(_) => {
                panic!("exact retry after crash created a duplicate reservation")
            }
        };
        assert_eq!(recovered.lease_generation, 1);
        assert_eq!(recovered.lease_expires_at_ms, 31_000);
        assert!(recovered.released_at_ms.is_none());

        reopened.close().await;
    });

    cleanup(&path);
}
