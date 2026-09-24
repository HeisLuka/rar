use std::{
    env,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chaptera_server::sqlite_store::{
    AppendOutcome, RevisionEdge, SqliteRevisionStore, encode_canonical_event,
};

const CHILD_ENV: &str = "CHAPTERA_SQLITE_CRASH_CHILD";
const PATH_ENV: &str = "CHAPTERA_SQLITE_CRASH_PATH";

fn temp_db(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    env::temp_dir().join(format!(
        "chaptera-sqlite-{label}-{}-{stamp}.sqlite",
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

fn hash(byte: char) -> String {
    byte.to_string().repeat(64)
}

fn accepted_edge() -> RevisionEdge {
    RevisionEdge {
        document_id: "doc-crash-boundary".to_owned(),
        parent_revision: "rev-0".to_owned(),
        parent_cursor: 0,
        operation_id: "op-crash-1".to_owned(),
        request_hash: hash('a'),
        canonical_event: encode_canonical_event(b"durable-before-ack-crash").unwrap(),
        child_revision: "rev-1".to_owned(),
        child_cursor: 1,
        resulting_state_hash: hash('b'),
        authoring_root_hash: Some(hash('c')),
        semantic_schema_version: 1,
        committed_at_ms: 1_000,
    }
}

#[test]
#[ignore = "spawned only by process crash acceptance parent"]
fn child_commits_then_aborts_before_ack() {
    if env::var_os(CHILD_ENV).is_none() {
        return;
    }

    let path = PathBuf::from(env::var(PATH_ENV).expect("crash child database path"));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async {
        let store = SqliteRevisionStore::open(&path, 2, Duration::from_secs(2))
            .await
            .unwrap();

        let outcome = store.append_edge(accepted_edge()).await.unwrap();
        assert!(matches!(outcome, AppendOutcome::Committed(_)));

        // Intentionally do not close the pool and do not return a success/ACK
        // to the parent. abort() models a process death immediately after the
        // synchronous=FULL SQLite commit becomes observable to the caller.
        std::process::abort();
    });
}

#[test]
fn process_crash_after_durable_commit_before_ack_recovers_exact_edge() {
    let path = temp_db("commit-before-ack");

    let child = Command::new(env::current_exe().unwrap())
        .args([
            "--exact",
            "child_commits_then_aborts_before_ack",
            "--ignored",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .env(PATH_ENV, &path)
        .status()
        .expect("spawn crash-boundary child");

    assert!(
        !child.success(),
        "crash child unexpectedly returned success instead of dying after commit"
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async {
        let expected = accepted_edge();
        let reopened = SqliteRevisionStore::open(&path, 2, Duration::from_secs(2))
            .await
            .unwrap();

        let stored = reopened
            .read_edge(&expected.document_id, &expected.parent_revision)
            .await
            .unwrap()
            .expect("durable edge must survive process death before ACK");
        assert_eq!(stored, expected);

        let retry = reopened.append_edge(expected.clone()).await.unwrap();
        assert_eq!(retry, AppendOutcome::AlreadyCommitted(expected.clone()));

        let replay = reopened
            .load_document_edges(&expected.document_id)
            .await
            .unwrap();
        assert_eq!(replay, vec![expected]);

        reopened.close().await;
    });

    cleanup(&path);
}
