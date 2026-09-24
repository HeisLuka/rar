#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from store import PubHarnessStore

BASE_URL = "https://api.notion.com/v1"
HERE = Path(__file__).resolve().parent
DEFAULT_CONFIG = HERE / "notion_sources.json"


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


class NotionClient:
    def __init__(self, token: str, version: str):
        self.token = token
        self.version = version

    def query_data_source(
        self,
        data_source_id: str,
        *,
        start_cursor: str | None = None,
        edited_on_or_after: str | None = None,
        page_size: int = 100,
    ) -> dict[str, Any]:
        body: dict[str, Any] = {"page_size": min(page_size, 100)}
        if start_cursor:
            body["start_cursor"] = start_cursor
        if edited_on_or_after:
            body["filter"] = {
                "timestamp": "last_edited_time",
                "last_edited_time": {"on_or_after": edited_on_or_after},
            }
        return self._json(
            f"/data_sources/{data_source_id}/query",
            method="POST",
            body=body,
        )

    def _json(self, path: str, *, method: str, body: dict[str, Any] | None = None):
        data = None if body is None else json.dumps(body).encode("utf-8")
        request = urllib.request.Request(
            BASE_URL + path,
            data=data,
            method=method,
            headers={
                "Authorization": f"Bearer {self.token}",
                "Notion-Version": self.version,
                "Content-Type": "application/json",
                "User-Agent": "pub-harness-v0",
            },
        )
        while True:
            try:
                with urllib.request.urlopen(request, timeout=30) as response:
                    return json.loads(response.read().decode("utf-8"))
            except urllib.error.HTTPError as exc:
                if exc.code != 429:
                    detail = exc.read().decode("utf-8", errors="replace")
                    raise RuntimeError(f"Notion HTTP {exc.code}: {detail}") from exc
                retry_after = float(exc.headers.get("Retry-After", "1"))
                time.sleep(max(retry_after, 1.0))


def prop_value(prop: dict[str, Any] | None) -> Any:
    if not prop:
        return None
    kind = prop.get("type")
    value = prop.get(kind) if kind else None
    if kind in {"title", "rich_text"}:
        return "".join(item.get("plain_text", "") for item in (value or []))
    if kind in {"select", "status"}:
        return (value or {}).get("name")
    if kind == "multi_select":
        return [item.get("name") for item in (value or [])]
    if kind == "relation":
        return [item.get("id") for item in (value or []) if item.get("id")]
    if kind == "formula":
        formula = value or {}
        ftype = formula.get("type")
        return formula.get(ftype) if ftype else None
    if kind == "rollup":
        return value
    if kind == "people":
        return [item.get("name") or item.get("id") for item in (value or [])]
    if kind == "date":
        return (value or {}).get("start")
    if kind in {"number", "checkbox", "url", "email", "phone_number"}:
        return value
    return value


def load_config(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def semantic_id(page: dict[str, Any], source: dict[str, Any]) -> str:
    props = page.get("properties", {})
    raw = prop_value(props.get(source["id_property"]))
    if raw:
        return str(raw).strip()
    title = prop_value(props.get(source["title_property"]))
    if title:
        # Implementation tasks currently use the task title as their stable visible identity.
        return str(title).strip()
    return page["id"]


def page_to_entity(page: dict[str, Any], source: dict[str, Any]) -> dict[str, Any]:
    props = page.get("properties", {})
    entity = {
        "entity_type": source["entity_type"],
        "entity_id": semantic_id(page, source),
        "notion_page_id": page["id"],
        "notion_url": page.get("url"),
        "title": str(prop_value(props.get(source["title_property"])) or semantic_id(page, source)),
        "created_at": page.get("created_time"),
        "edited_at": page.get("last_edited_time"),
        "content_hydrated": False,
        "raw_json": {"archived": page.get("archived"), "in_trash": page.get("in_trash")},
    }
    for local_name, notion_name in source.get("fields", {}).items():
        value = prop_value(props.get(notion_name))
        if isinstance(value, (dict, list)):
            value = json.dumps(value, ensure_ascii=False, sort_keys=True)
        entity[local_name] = value
    return entity


def relation_page_ids(page: dict[str, Any], source: dict[str, Any]):
    props = page.get("properties", {})
    for local_name, spec in source.get("relations", {}).items():
        ids = prop_value(props.get(spec["property"])) or []
        yield local_name, spec["target_type"], list(ids)


def resolve_semantic_id(
    store: PubHarnessStore,
    target_type: str,
    notion_page_id: str,
) -> str | None:
    row = store.conn.execute(
        """SELECT entity_id FROM entities
           WHERE entity_type=? AND notion_page_id=?""",
        (target_type, notion_page_id),
    ).fetchone()
    return row["entity_id"] if row else None


def sync_source(
    client: NotionClient,
    store: PubHarnessStore,
    source: dict[str, Any],
    *,
    full: bool,
) -> dict[str, Any]:
    watermark = store.conn.execute(
        "SELECT * FROM sync_watermarks WHERE source_key=?",
        (source["key"],),
    ).fetchone()
    since = None if full or not watermark else watermark["last_edited_at"]
    cursor = None
    pages: list[dict[str, Any]] = []
    newest = since

    while True:
        response = client.query_data_source(
            source["data_source_id"],
            start_cursor=cursor,
            edited_on_or_after=since,
        )
        batch = response.get("results", [])
        pages.extend(batch)
        for page in batch:
            edited = page.get("last_edited_time")
            if edited and (newest is None or edited > newest):
                newest = edited
        if not response.get("has_more"):
            break
        cursor = response.get("next_cursor")
        if not cursor:
            raise RuntimeError("Notion returned has_more without next_cursor")

    # Pass 1: identities and primitive metadata.
    for page in pages:
        store.upsert_entity(page_to_entity(page, source))

    # Pass 2: relations. Resolve semantic IDs when the target source is mirrored.
    # Unknown targets remain explicit notion-page edges rather than being dropped.
    for page in pages:
        src_id = semantic_id(page, source)
        for relation_name, target_type, page_ids in relation_page_ids(page, source):
            targets = []
            for notion_page_id in page_ids:
                target_id = resolve_semantic_id(store, target_type, notion_page_id)
                if target_id:
                    targets.append((target_type, target_id))
                else:
                    targets.append(("notion_page", notion_page_id))
            store.replace_relations(source["entity_type"], src_id, relation_name, targets)

    now = utc_now()
    values = {
        "last_edited_at": newest or since,
        "last_success_at": now,
        "last_error": None,
        "last_cursor": None,
    }
    if full:
        values["last_full_sync_at"] = now
    store.set_watermark(source["key"], **values)
    return {"source": source["key"], "rows": len(pages), "since": since, "newest": newest}


def main() -> int:
    parser = argparse.ArgumentParser(description="Mirror bounded PUB Notion metadata into local SQLite.")
    parser.add_argument("--db", default=os.environ.get("PUB_HARNESS_DB", "pub-harness.db"))
    parser.add_argument("--config", default=str(DEFAULT_CONFIG))
    parser.add_argument("--source", action="append", help="Source key; repeat to select multiple.")
    parser.add_argument("--full", action="store_true", help="Ignore edit watermark and reconcile all rows.")
    args = parser.parse_args()

    token = os.environ.get("NOTION_TOKEN")
    if not token:
        raise SystemExit("NOTION_TOKEN is required")

    config = load_config(Path(args.config))
    selected = set(args.source or [])
    sources = [s for s in config["sources"] if not selected or s["key"] in selected]
    unknown = selected - {s["key"] for s in sources}
    if unknown:
        raise SystemExit(f"Unknown source key(s): {', '.join(sorted(unknown))}")

    store = PubHarnessStore(args.db)
    store.init_schema()
    client = NotionClient(token, config["notion_version"])
    try:
        for source in sources:
            try:
                result = sync_source(client, store, source, full=args.full)
                print(json.dumps(result, ensure_ascii=False))
            except Exception as exc:
                store.set_watermark(source["key"], last_error=str(exc))
                raise
    finally:
        store.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
