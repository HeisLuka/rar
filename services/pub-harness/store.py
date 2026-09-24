#!/usr/bin/env python3
from __future__ import annotations

import json
import sqlite3
from pathlib import Path
from typing import Any, Iterable

SCHEMA_PATH = Path(__file__).with_name("schema.sql")


class PubHarnessStore:
    def __init__(self, db_path: str | Path):
        self.db_path = Path(db_path)
        self.db_path.parent.mkdir(parents=True, exist_ok=True)
        self.conn = sqlite3.connect(self.db_path)
        self.conn.row_factory = sqlite3.Row
        self.conn.execute("PRAGMA foreign_keys=ON")
        self.conn.execute("PRAGMA journal_mode=WAL")

    def close(self) -> None:
        self.conn.close()

    def init_schema(self) -> None:
        self.conn.executescript(SCHEMA_PATH.read_text(encoding="utf-8"))
        self.conn.commit()

    def upsert_entity(self, entity: dict[str, Any]) -> None:
        allowed = {
            "entity_type","entity_id","notion_page_id","notion_url","title",
            "lifecycle","priority","agent_gate","dispatch_state","execution_owner",
            "execution_workspace","authority_domain","blocker","body_markdown",
            "created_at","edited_at","content_hydrated","raw_json",
        }
        row = {k: entity.get(k) for k in allowed}
        if not row["entity_type"] or not row["entity_id"] or not row["title"]:
            raise ValueError("entity_type, entity_id and title are required")
        if isinstance(row.get("raw_json"), (dict, list)):
            row["raw_json"] = json.dumps(row["raw_json"], ensure_ascii=False, sort_keys=True)
        row["content_hydrated"] = int(bool(row.get("content_hydrated")))
        columns = sorted(row)
        placeholders = ",".join("?" for _ in columns)
        updates = ",".join(f"{c}=excluded.{c}" for c in columns if c not in {"entity_type","entity_id"})
        values = [row[c] for c in columns]
        self.conn.execute(
            f"""INSERT INTO entities ({','.join(columns)})
                VALUES ({placeholders})
                ON CONFLICT(entity_type, entity_id) DO UPDATE SET {updates}""",
            values,
        )
        self._refresh_fts(row["entity_type"], row["entity_id"])
        self.conn.commit()

    def replace_relations(
        self,
        src_type: str,
        src_id: str,
        relation: str,
        targets: Iterable[tuple[str, str]],
    ) -> None:
        self.conn.execute(
            "DELETE FROM relations WHERE src_type=? AND src_id=? AND relation=?",
            (src_type, src_id, relation),
        )
        self.conn.executemany(
            """INSERT OR IGNORE INTO relations
               (src_type,src_id,relation,dst_type,dst_id)
               VALUES (?,?,?,?,?)""",
            [(src_type, src_id, relation, t, i) for t, i in targets],
        )
        self.conn.commit()

    def get(self, entity_type: str, entity_id: str) -> dict[str, Any] | None:
        row = self.conn.execute(
            "SELECT * FROM entities WHERE entity_type=? AND entity_id=?",
            (entity_type, entity_id),
        ).fetchone()
        return dict(row) if row else None

    def relations_from(self, entity_type: str, entity_id: str, relation: str | None = None):
        if relation is None:
            rows = self.conn.execute(
                """SELECT * FROM relations
                   WHERE src_type=? AND src_id=?
                   ORDER BY relation,dst_type,dst_id""",
                (entity_type, entity_id),
            ).fetchall()
        else:
            rows = self.conn.execute(
                """SELECT * FROM relations
                   WHERE src_type=? AND src_id=? AND relation=?
                   ORDER BY dst_type,dst_id""",
                (entity_type, entity_id, relation),
            ).fetchall()
        return [dict(r) for r in rows]

    def search(
        self,
        *,
        entity_type: str | None = None,
        lifecycle: str | None = None,
        priority: str | None = None,
        agent_gate: str | None = None,
        dispatch_state: str | None = None,
        execution_owner: str | None = None,
        workspace_contains: str | None = None,
        limit: int = 50,
    ):
        where, args = [], []
        filters = {
            "entity_type": entity_type,
            "lifecycle": lifecycle,
            "priority": priority,
            "agent_gate": agent_gate,
            "dispatch_state": dispatch_state,
            "execution_owner": execution_owner,
        }
        for key, value in filters.items():
            if value is not None:
                where.append(f"{key}=?")
                args.append(value)
        if workspace_contains:
            where.append("execution_workspace LIKE ?")
            args.append(f"%{workspace_contains}%")
        sql = "SELECT * FROM entities"
        if where:
            sql += " WHERE " + " AND ".join(where)
        sql += """ ORDER BY
          CASE priority WHEN 'P0' THEN 0 WHEN 'P1' THEN 1 WHEN 'P2' THEN 2 ELSE 9 END,
          entity_id
          LIMIT ?"""
        args.append(limit)
        return [dict(r) for r in self.conn.execute(sql, args).fetchall()]

    def text_search(self, query: str, entity_type: str | None = None, limit: int = 25):
        sql = """SELECT e.*, bm25(entities_fts) AS rank
                 FROM entities_fts
                 JOIN entities e
                   ON e.entity_type=entities_fts.entity_type
                  AND e.entity_id=entities_fts.entity_id
                 WHERE entities_fts MATCH ?"""
        args: list[Any] = [query]
        if entity_type:
            sql += " AND e.entity_type=?"
            args.append(entity_type)
        sql += " ORDER BY rank LIMIT ?"
        args.append(limit)
        return [dict(r) for r in self.conn.execute(sql, args).fetchall()]

    def set_watermark(self, source_key: str, **values: Any) -> None:
        current = self.conn.execute(
            "SELECT * FROM sync_watermarks WHERE source_key=?", (source_key,)
        ).fetchone()
        row = dict(current) if current else {"source_key": source_key}
        row.update(values)
        cols = ["source_key","last_cursor","last_edited_at","last_full_sync_at","last_success_at","last_error"]
        self.conn.execute(
            f"""INSERT INTO sync_watermarks ({','.join(cols)})
                VALUES ({','.join('?' for _ in cols)})
                ON CONFLICT(source_key) DO UPDATE SET
                  last_cursor=excluded.last_cursor,
                  last_edited_at=excluded.last_edited_at,
                  last_full_sync_at=excluded.last_full_sync_at,
                  last_success_at=excluded.last_success_at,
                  last_error=excluded.last_error""",
            [row.get(c) for c in cols],
        )
        self.conn.commit()

    def log_change(self, observed_at: str, source: str, operation: str, **payload: Any) -> None:
        self.conn.execute(
            """INSERT INTO change_log
               (observed_at,source,entity_type,entity_id,operation,payload_json)
               VALUES (?,?,?,?,?,?)""",
            (
                observed_at,
                source,
                payload.pop("entity_type", None),
                payload.pop("entity_id", None),
                operation,
                json.dumps(payload, ensure_ascii=False, sort_keys=True),
            ),
        )
        self.conn.commit()

    def _refresh_fts(self, entity_type: str, entity_id: str) -> None:
        row = self.conn.execute(
            """SELECT entity_type,entity_id,title,
                      COALESCE(body_markdown,'') body_markdown,
                      COALESCE(blocker,'') blocker
               FROM entities WHERE entity_type=? AND entity_id=?""",
            (entity_type, entity_id),
        ).fetchone()
        self.conn.execute(
            "DELETE FROM entities_fts WHERE entity_type=? AND entity_id=?",
            (entity_type, entity_id),
        )
        self.conn.execute(
            """INSERT INTO entities_fts(entity_type,entity_id,title,body_markdown,blocker)
               VALUES (?,?,?,?,?)""",
            tuple(row),
        )
