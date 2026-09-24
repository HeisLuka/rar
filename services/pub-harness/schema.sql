PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;

CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS entities (
  entity_type TEXT NOT NULL,
  entity_id TEXT NOT NULL,
  notion_page_id TEXT,
  notion_url TEXT,
  title TEXT NOT NULL,
  lifecycle TEXT,
  priority TEXT,
  agent_gate TEXT,
  dispatch_state TEXT,
  execution_owner TEXT,
  execution_workspace TEXT,
  authority_domain TEXT,
  blocker TEXT,
  body_markdown TEXT,
  created_at TEXT,
  edited_at TEXT,
  content_hydrated INTEGER NOT NULL DEFAULT 0,
  raw_json TEXT,
  PRIMARY KEY (entity_type, entity_id)
);

CREATE TABLE IF NOT EXISTS relations (
  src_type TEXT NOT NULL,
  src_id TEXT NOT NULL,
  relation TEXT NOT NULL,
  dst_type TEXT NOT NULL,
  dst_id TEXT NOT NULL,
  PRIMARY KEY (src_type, src_id, relation, dst_type, dst_id)
);

CREATE INDEX IF NOT EXISTS idx_entities_type_state
  ON entities(entity_type, lifecycle, priority, agent_gate, dispatch_state);
CREATE INDEX IF NOT EXISTS idx_entities_owner
  ON entities(execution_owner, execution_workspace);
CREATE INDEX IF NOT EXISTS idx_relations_src
  ON relations(src_type, src_id, relation);
CREATE INDEX IF NOT EXISTS idx_relations_dst
  ON relations(dst_type, dst_id, relation);

-- Stored-content FTS is deliberate. It permits ordinary DELETE/INSERT refreshes
-- across the SQLite versions commonly bundled with Python on Windows.
CREATE VIRTUAL TABLE IF NOT EXISTS entities_fts USING fts5(
  entity_type UNINDEXED,
  entity_id UNINDEXED,
  title,
  body_markdown,
  blocker
);

CREATE TABLE IF NOT EXISTS sync_watermarks (
  source_key TEXT PRIMARY KEY,
  last_cursor TEXT,
  last_edited_at TEXT,
  last_full_sync_at TEXT,
  last_success_at TEXT,
  last_error TEXT
);

CREATE TABLE IF NOT EXISTS change_log (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  observed_at TEXT NOT NULL,
  source TEXT NOT NULL,
  entity_type TEXT,
  entity_id TEXT,
  operation TEXT NOT NULL,
  payload_json TEXT
);
