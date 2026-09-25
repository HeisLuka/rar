use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chaptera_server::{
    revision_materializer::{
        AuthorizedDocumentSource, DocumentSourceAuthority, EDITOR_REVISION_EVENT_SCHEMA_V1,
        EDITOR_REVISION_EVENT_SEMANTIC_SCHEMA_VERSION, EditorReplayEngine, EditorRevisionEventV1,
        ExactRevisionMaterializer, ExactSourceLoader, PubEditorReplayEngine,
        RevisionMaterializerError, encode_editor_revision_event_v1, project_sha256,
    },
    schema_migration::SqliteMigrationRuntime,
    sqlite_store::{RevisionEdge, SqliteRevisionStore, encode_canonical_event},
};
use pub_editor::{
    EDITOR_PROJECT_VERSION_V0_2, EDITOR_PROJECT_VERSION_V0_4, EditOperation, EditorProject,
    LengthEmu, RectEmu, Sha256Digest,
};
use sha2::{Digest, Sha256};
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};

static TEST_ID: AtomicU64 = AtomicU64::new(1);

const NODE_ID: &str = "007d9898-568b-5125-b519-8d88243aabfb";
const SAMPLE_SOURCE_SHA256: &str =
    "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf";

#[derive(Clone)]
struct StaticAuthority {
    source: AuthorizedDocumentSource,
}

#[async_trait]
impl DocumentSourceAuthority for StaticAuthority {
    async fn resolve_document_source(
        &self,
        _tenant_id: &str,
        _document_id: &str,
    ) -> Result<AuthorizedDocumentSource, RevisionMaterializerError> {
        Ok(self.source.clone())
    }
}

#[derive(Clone)]
struct MemorySourceLoader {
    bytes: Arc<Vec<u8>>,
}

#[async_trait]
impl ExactSourceLoader for MemorySourceLoader {
    async fn load_exact_source(
        &self,
        _source: &AuthorizedDocumentSource,
    ) -> Result<Vec<u8>, RevisionMaterializerError> {
        Ok(self.bytes.as_ref().clone())
    }
}

#[derive(Clone)]
struct FakeEditor {
    initial_rect: RectEmu,
}

impl FakeEditor {
    fn digest(source_sha256: &str) -> Result<Sha256Digest, RevisionMaterializerError> {
        Sha256Digest::from_str(source_sha256).map_err(|error| {
            RevisionMaterializerError::new(
                "invalid_source_hash",
                format!("fake editor source hash invalid: {error}"),
            )
        })
    }
}

impl EditorReplayEngine for FakeEditor {
    fn baseline_project(
        &self,
        _source_bytes: &[u8],
        source_sha256: &str,
    ) -> Result<EditorProject, RevisionMaterializerError> {
        Ok(EditorProject {
            schema_version: EDITOR_PROJECT_VERSION_V0_2.to_owned(),
            source_hash: Self::digest(source_sha256)?,
            assets: Vec::new(),
            operations: Vec::new(),
        })
    }

    fn replay_project(
        &self,
        _source_bytes: &[u8],
        source_sha256: &str,
        project: &EditorProject,
    ) -> Result<EditorProject, RevisionMaterializerError> {
        if project.source_hash != Self::digest(source_sha256)? {
            return Err(RevisionMaterializerError::new(
                "project_source_mismatch",
                "fake canonical editor saw another source",
            ));
        }
        if !project.assets.is_empty() {
            return Err(RevisionMaterializerError::new(
                "editor_asset_replay_unsupported",
                "fake editor does not admit assets",
            ));
        }

        let mut rect = self.initial_rect;
        for operation in &project.operations {
            match operation {
                EditOperation::MoveNode { before, after, .. } => {
                    if *before != rect {
                        return Err(RevisionMaterializerError::new(
                            "editor_replay_rejected",
                            "MoveNode before-state does not match current canonical state",
                        ));
                    }
                    rect = *after;
                }
                _ => {
                    return Err(RevisionMaterializerError::new(
                        "editor_replay_rejected",
                        "synthetic test engine admits only MoveNode",
                    ));
                }
            }
        }

        let expected_schema = if project.operations.is_empty() {
            EDITOR_PROJECT_VERSION_V0_2
        } else {
            EDITOR_PROJECT_VERSION_V0_4
        };
        if project.schema_version != expected_schema {
            return Err(RevisionMaterializerError::new(
                "editor_replay_rejected",
                "project schema does not match canonical operation family",
            ));
        }
        Ok(project.clone())
    }
}

fn temp_db(label: &str) -> PathBuf {
    let serial = TEST_ID.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!(
        "chaptera-revision-materializer-{label}-{}-{serial}.db",
        std::process::id()
    ))
}

async fn open_store(label: &str) -> (SqliteRevisionStore, PathBuf) {
    let path = temp_db(label);
    let migration = SqliteMigrationRuntime::new(&path, Duration::from_secs(1)).unwrap();
    migration.migrate_up().await.unwrap();
    let store = SqliteRevisionStore::open(&path, 2, Duration::from_secs(1))
        .await
        .unwrap();
    (store, path)
}

async fn cleanup_store(store: &SqliteRevisionStore, path: &Path) {
    store.close().await;
    for candidate in [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ] {
        let _ = fs::remove_file(candidate);
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").unwrap();
    }
    out
}

fn hash_char(ch: char) -> String {
    std::iter::repeat_n(ch, 64).collect()
}

fn rect(x: i64, y: i64, width: i64, height: i64) -> RectEmu {
    RectEmu::new(
        LengthEmu::new(x),
        LengthEmu::new(y),
        LengthEmu::new(width),
        LengthEmu::new(height),
    )
}

fn move_operation(before: RectEmu, after: RectEmu) -> EditOperation {
    serde_json::from_value(serde_json::json!({
        "kind": "move_node",
        "node_id": NODE_ID,
        "before": before,
        "after": after,
    }))
    .unwrap()
}

fn append_project_operation(mut project: EditorProject, operation: EditOperation) -> EditorProject {
    project.operations.push(operation);
    project.schema_version = EDITOR_PROJECT_VERSION_V0_4.to_owned();
    project
}

fn authority(
    source_bytes: &[u8],
    document_id: &str,
    source_sha256: &str,
) -> AuthorizedDocumentSource {
    AuthorizedDocumentSource {
        tenant_id: "tenant-a".into(),
        document_id: document_id.into(),
        binding_id: "binding-source-1".into(),
        source_sha256: source_sha256.into(),
        byte_len: source_bytes.len() as u64,
        baseline_revision_id: "r0".into(),
        baseline_cursor: 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn edge_for(
    document_id: &str,
    parent_revision: &str,
    child_revision: &str,
    parent_cursor: i64,
    operation_id: &str,
    source_sha256: &str,
    before_project: &EditorProject,
    after_project: &EditorProject,
    operation: EditOperation,
    root: &str,
) -> RevisionEdge {
    let event = EditorRevisionEventV1 {
        schema_version: EDITOR_REVISION_EVENT_SCHEMA_V1.into(),
        source_sha256: source_sha256.into(),
        before_project_sha256: project_sha256(before_project).unwrap(),
        after_project_sha256: project_sha256(after_project).unwrap(),
        authoring_root_hash: Some(root.into()),
        operation,
    };
    RevisionEdge {
        document_id: document_id.into(),
        parent_revision: parent_revision.into(),
        parent_cursor,
        operation_id: operation_id.into(),
        request_hash: hash_char('c'),
        canonical_event: encode_editor_revision_event_v1(&event).unwrap(),
        child_revision: child_revision.into(),
        child_cursor: parent_cursor + 1,
        resulting_state_hash: project_sha256(after_project).unwrap(),
        authoring_root_hash: Some(root.into()),
        semantic_schema_version: EDITOR_REVISION_EVENT_SEMANTIC_SCHEMA_VERSION,
        committed_at_ms: 1,
    }
}

fn materializer(
    source: AuthorizedDocumentSource,
    bytes: Vec<u8>,
    store: SqliteRevisionStore,
    editor: Arc<dyn EditorReplayEngine>,
) -> ExactRevisionMaterializer {
    ExactRevisionMaterializer::new(
        Arc::new(StaticAuthority { source }),
        Arc::new(MemorySourceLoader {
            bytes: Arc::new(bytes),
        }),
        store,
        editor,
    )
}

#[test]
fn project_hash_matches_existing_rar_revision_kernel_law() {
    let project = EditorProject {
        schema_version: EDITOR_PROJECT_VERSION_V0_2.into(),
        source_hash: Sha256Digest::from_str(SAMPLE_SOURCE_SHA256).unwrap(),
        assets: Vec::new(),
        operations: Vec::new(),
    };
    assert_eq!(
        project_sha256(&project).unwrap(),
        "575fbcb664f2a6b672a05861a4d2aff6aca339204a50e3401d04e1920d946348"
    );
}

#[tokio::test]
async fn exact_prefix_materializes_r0_r1_r2_and_ignores_corrupt_future_tail_for_r1() {
    let bytes = b"synthetic-pub-source".to_vec();
    let source_sha256 = sha256_hex(&bytes);
    let initial = rect(0, 0, 100, 50);
    let editor: Arc<dyn EditorReplayEngine> = Arc::new(FakeEditor {
        initial_rect: initial,
    });
    let baseline = editor.baseline_project(&bytes, &source_sha256).unwrap();
    let op1 = move_operation(initial, rect(10, 20, 100, 50));
    let p1 = append_project_operation(baseline.clone(), op1.clone());
    let op2 = move_operation(rect(10, 20, 100, 50), rect(30, 40, 100, 50));
    let p2 = append_project_operation(p1.clone(), op2.clone());

    let (store, path) = open_store("prefix").await;
    store
        .append_edge(edge_for(
            "doc-a",
            "r0",
            "r1",
            0,
            "op-1",
            &source_sha256,
            &baseline,
            &p1,
            op1,
            &hash_char('a'),
        ))
        .await
        .unwrap();
    store
        .append_edge(edge_for(
            "doc-a",
            "r1",
            "r2",
            1,
            "op-2",
            &source_sha256,
            &p1,
            &p2,
            op2,
            &hash_char('b'),
        ))
        .await
        .unwrap();

    let m = materializer(
        authority(&bytes, "doc-a", &source_sha256),
        bytes.clone(),
        store.clone(),
        editor.clone(),
    );

    let r0 = m.materialize("tenant-a", "doc-a", "r0").await.unwrap();
    assert_eq!(r0.replayed_edges, 0);
    assert_eq!(r0.project, baseline);

    let r1 = m.materialize("tenant-a", "doc-a", "r1").await.unwrap();
    assert_eq!(r1.replayed_edges, 1);
    assert_eq!(r1.project, p1);
    assert_eq!(r1.authoring_root_hash, Some(hash_char('a')));

    let r2 = m.materialize("tenant-a", "doc-a", "r2").await.unwrap();
    assert_eq!(r2.replayed_edges, 2);
    assert_eq!(r2.project, p2);

    let r1_again = m.materialize("tenant-a", "doc-a", "r1").await.unwrap();
    assert_eq!(
        serde_json::to_vec(&r1).unwrap(),
        serde_json::to_vec(&r1_again).unwrap()
    );

    let pool = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(false),
    )
    .await
    .unwrap();
    sqlx::query(
        "UPDATE revision_edges SET canonical_event = ? WHERE document_id = ? AND child_revision = ?",
    )
    .bind(vec![1_u8, 2, 3])
    .bind(b"doc-a".as_slice())
    .bind(b"r2".as_slice())
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let historical = m.materialize("tenant-a", "doc-a", "r1").await.unwrap();
    assert_eq!(historical.project, p1);

    let error = m.materialize("tenant-a", "doc-a", "r2").await.unwrap_err();
    assert_eq!(error.code, "canonical_event_corrupt");

    cleanup_store(&store, &path).await;
}

async fn single_edge_error(
    label: &str,
    mut source: AuthorizedDocumentSource,
    bytes: Vec<u8>,
    mut event: EditorRevisionEventV1,
    resulting_state_hash: String,
    edge_root: Option<String>,
    requested_revision: &str,
) -> RevisionMaterializerError {
    let (store, path) = open_store(label).await;
    let edge = RevisionEdge {
        document_id: "doc-a".into(),
        parent_revision: "r0".into(),
        parent_cursor: 0,
        operation_id: "op-1".into(),
        request_hash: hash_char('c'),
        canonical_event: encode_editor_revision_event_v1(&event).unwrap(),
        child_revision: "r1".into(),
        child_cursor: 1,
        resulting_state_hash,
        authoring_root_hash: edge_root,
        semantic_schema_version: EDITOR_REVISION_EVENT_SEMANTIC_SCHEMA_VERSION,
        committed_at_ms: 1,
    };
    store.append_edge(edge).await.unwrap();

    if source.binding_id.is_empty() {
        source.binding_id = "binding-source-1".into();
    }

    let editor: Arc<dyn EditorReplayEngine> = Arc::new(FakeEditor {
        initial_rect: rect(0, 0, 100, 50),
    });
    let m = materializer(source, bytes, store.clone(), editor);
    let error = m
        .materialize("tenant-a", "doc-a", requested_revision)
        .await
        .unwrap_err();
    cleanup_store(&store, &path).await;
    // Keep the event mutable parameter meaningful to callers and prevent a
    // future accidental Copy simplification from hiding test setup changes.
    event.schema_version.clear();
    error
}

#[tokio::test]
async fn materializer_fails_closed_on_source_before_state_root_state_and_schema_mismatch() {
    let bytes = b"synthetic-pub-source".to_vec();
    let source_sha256 = sha256_hex(&bytes);
    let fake = FakeEditor {
        initial_rect: rect(0, 0, 100, 50),
    };
    let baseline = fake.baseline_project(&bytes, &source_sha256).unwrap();
    let op = move_operation(rect(0, 0, 100, 50), rect(10, 20, 100, 50));
    let p1 = append_project_operation(baseline.clone(), op.clone());

    let base_event = EditorRevisionEventV1 {
        schema_version: EDITOR_REVISION_EVENT_SCHEMA_V1.into(),
        source_sha256: source_sha256.clone(),
        before_project_sha256: project_sha256(&baseline).unwrap(),
        after_project_sha256: project_sha256(&p1).unwrap(),
        authoring_root_hash: Some(hash_char('a')),
        operation: op,
    };

    let mut wrong_source = base_event.clone();
    wrong_source.source_sha256 = hash_char('d');
    assert_eq!(
        single_edge_error(
            "wrong-source",
            authority(&bytes, "doc-a", &source_sha256),
            bytes.clone(),
            wrong_source,
            project_sha256(&p1).unwrap(),
            Some(hash_char('a')),
            "r1",
        )
        .await
        .code,
        "event_source_mismatch"
    );

    let mut wrong_before = base_event.clone();
    wrong_before.before_project_sha256 = hash_char('e');
    assert_eq!(
        single_edge_error(
            "wrong-before",
            authority(&bytes, "doc-a", &source_sha256),
            bytes.clone(),
            wrong_before,
            project_sha256(&p1).unwrap(),
            Some(hash_char('a')),
            "r1",
        )
        .await
        .code,
        "before_state_mismatch"
    );

    let mut wrong_root = base_event.clone();
    wrong_root.authoring_root_hash = Some(hash_char('b'));
    assert_eq!(
        single_edge_error(
            "wrong-root",
            authority(&bytes, "doc-a", &source_sha256),
            bytes.clone(),
            wrong_root,
            project_sha256(&p1).unwrap(),
            Some(hash_char('a')),
            "r1",
        )
        .await
        .code,
        "authoring_root_mismatch"
    );

    assert_eq!(
        single_edge_error(
            "wrong-state",
            authority(&bytes, "doc-a", &source_sha256),
            bytes.clone(),
            base_event.clone(),
            hash_char('f'),
            Some(hash_char('a')),
            "r1",
        )
        .await
        .code,
        "revision_state_hash_mismatch"
    );

    let mut wrong_schema = base_event.clone();
    wrong_schema.schema_version = "chaptera.editor-revision-event.v99".into();
    let (store, path) = open_store("wrong-schema").await;
    let payload = serde_json::to_vec(&wrong_schema).unwrap();
    store
        .append_edge(RevisionEdge {
            document_id: "doc-a".into(),
            parent_revision: "r0".into(),
            parent_cursor: 0,
            operation_id: "op-1".into(),
            request_hash: hash_char('c'),
            canonical_event: encode_canonical_event(&payload).unwrap(),
            child_revision: "r1".into(),
            child_cursor: 1,
            resulting_state_hash: project_sha256(&p1).unwrap(),
            authoring_root_hash: Some(hash_char('a')),
            semantic_schema_version: EDITOR_REVISION_EVENT_SEMANTIC_SCHEMA_VERSION,
            committed_at_ms: 1,
        })
        .await
        .unwrap();
    let m = materializer(
        authority(&bytes, "doc-a", &source_sha256),
        bytes.clone(),
        store.clone(),
        Arc::new(fake.clone()),
    );
    assert_eq!(
        m.materialize("tenant-a", "doc-a", "r1")
            .await
            .unwrap_err()
            .code,
        "unsupported_event_schema"
    );
    cleanup_store(&store, &path).await;

    let mut wrong_document = authority(&bytes, "doc-other", &source_sha256);
    wrong_document.document_id = "doc-other".into();
    let (store, path) = open_store("wrong-document").await;
    let m = materializer(wrong_document, bytes.clone(), store.clone(), Arc::new(fake));
    assert_eq!(
        m.materialize("tenant-a", "doc-a", "r0")
            .await
            .unwrap_err()
            .code,
        "source_document_mismatch"
    );
    cleanup_store(&store, &path).await;
}

#[tokio::test]
async fn requested_revision_missing_and_prefix_gap_fail_closed() {
    let bytes = b"synthetic-pub-source".to_vec();
    let source_sha256 = sha256_hex(&bytes);
    let editor: Arc<dyn EditorReplayEngine> = Arc::new(FakeEditor {
        initial_rect: rect(0, 0, 100, 50),
    });
    let baseline = editor.baseline_project(&bytes, &source_sha256).unwrap();
    let op1 = move_operation(rect(0, 0, 100, 50), rect(10, 20, 100, 50));
    let p1 = append_project_operation(baseline.clone(), op1.clone());
    let op2 = move_operation(rect(10, 20, 100, 50), rect(30, 40, 100, 50));
    let p2 = append_project_operation(p1.clone(), op2.clone());

    let (store, path) = open_store("gap").await;
    store
        .append_edge(edge_for(
            "doc-a",
            "r0",
            "r1",
            0,
            "op-1",
            &source_sha256,
            &baseline,
            &p1,
            op1,
            &hash_char('a'),
        ))
        .await
        .unwrap();
    store
        .append_edge(edge_for(
            "doc-a",
            "r1",
            "r2",
            1,
            "op-2",
            &source_sha256,
            &p1,
            &p2,
            op2,
            &hash_char('b'),
        ))
        .await
        .unwrap();

    let m = materializer(
        authority(&bytes, "doc-a", &source_sha256),
        bytes.clone(),
        store.clone(),
        editor,
    );
    assert_eq!(
        m.materialize("tenant-a", "doc-a", "missing")
            .await
            .unwrap_err()
            .code,
        "requested_revision_not_found"
    );

    let pool = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(false),
    )
    .await
    .unwrap();
    sqlx::query("DELETE FROM revision_edges WHERE document_id = ? AND child_revision = ?")
        .bind(b"doc-a".as_slice())
        .bind(b"r1".as_slice())
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    assert_eq!(
        m.materialize("tenant-a", "doc-a", "r2")
            .await
            .unwrap_err()
            .code,
        "revision_chain_corrupt"
    );

    cleanup_store(&store, &path).await;
}

#[tokio::test]
#[ignore = "requires hash-pinned Apache POI SampleNewsletter.pub"]
async fn real_sample_newsletter_materializes_exact_historical_revision() {
    let fixture = std::env::var("CHAPTERA_SAMPLE_NEWSLETTER_PUB")
        .expect("CHAPTERA_SAMPLE_NEWSLETTER_PUB must point to the hash-pinned public fixture");
    let bytes = fs::read(&fixture).unwrap();
    assert_eq!(bytes.len(), 291_840);
    assert_eq!(sha256_hex(&bytes), SAMPLE_SOURCE_SHA256);

    let editor: Arc<dyn EditorReplayEngine> = Arc::new(PubEditorReplayEngine);
    let baseline = editor
        .baseline_project(&bytes, SAMPLE_SOURCE_SHA256)
        .unwrap();
    assert_eq!(
        project_sha256(&baseline).unwrap(),
        "575fbcb664f2a6b672a05861a4d2aff6aca339204a50e3401d04e1920d946348"
    );

    let before = rect(526_710, 1_191_292, 4_436_165, 587_274);
    let after1 = rect(653_710, 1_445_292, 4_436_165, 587_274);
    let after2 = rect(780_710, 1_699_292, 4_436_165, 587_274);
    let op1 = move_operation(before, after1);
    let p1 = append_project_operation(baseline.clone(), op1.clone());
    assert_eq!(
        editor
            .replay_project(&bytes, SAMPLE_SOURCE_SHA256, &p1)
            .unwrap(),
        p1
    );
    let op2 = move_operation(after1, after2);
    let p2 = append_project_operation(p1.clone(), op2.clone());
    assert_eq!(
        editor
            .replay_project(&bytes, SAMPLE_SOURCE_SHA256, &p2)
            .unwrap(),
        p2
    );

    let (store, path) = open_store("real-sample-newsletter").await;
    store
        .append_edge(edge_for(
            "sample-newsletter",
            "r0",
            "r1",
            0,
            "move-1",
            SAMPLE_SOURCE_SHA256,
            &baseline,
            &p1,
            op1,
            &hash_char('a'),
        ))
        .await
        .unwrap();
    store
        .append_edge(edge_for(
            "sample-newsletter",
            "r1",
            "r2",
            1,
            "move-2",
            SAMPLE_SOURCE_SHA256,
            &p1,
            &p2,
            op2,
            &hash_char('b'),
        ))
        .await
        .unwrap();

    let m = materializer(
        authority(&bytes, "sample-newsletter", SAMPLE_SOURCE_SHA256),
        bytes.clone(),
        store.clone(),
        editor,
    );
    let r0 = m
        .materialize("tenant-a", "sample-newsletter", "r0")
        .await
        .unwrap();
    let r1 = m
        .materialize("tenant-a", "sample-newsletter", "r1")
        .await
        .unwrap();
    let r2 = m
        .materialize("tenant-a", "sample-newsletter", "r2")
        .await
        .unwrap();
    assert_eq!(r0.project, baseline);
    assert_eq!(r1.project, p1);
    assert_eq!(r2.project, p2);
    assert_eq!(r1.replayed_edges, 1);
    assert_eq!(r2.replayed_edges, 2);

    let repeat = m
        .materialize("tenant-a", "sample-newsletter", "r1")
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_vec(&r1).unwrap(),
        serde_json::to_vec(&repeat).unwrap()
    );

    let pool = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(false),
    )
    .await
    .unwrap();
    sqlx::query(
        "UPDATE revision_edges SET canonical_event = ? WHERE document_id = ? AND child_revision = ?",
    )
    .bind(vec![0_u8])
    .bind(b"sample-newsletter".as_slice())
    .bind(b"r2".as_slice())
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let historical = m
        .materialize("tenant-a", "sample-newsletter", "r1")
        .await
        .unwrap();
    assert_eq!(historical.project, p1);
    assert_eq!(
        m.materialize("tenant-a", "sample-newsletter", "r2")
            .await
            .unwrap_err()
            .code,
        "canonical_event_corrupt"
    );

    cleanup_store(&store, &path).await;
}
