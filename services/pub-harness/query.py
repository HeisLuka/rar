#!/usr/bin/env python3
from __future__ import annotations

from typing import Any
from store import PubHarnessStore


class PubHarnessQuery:
    """Deterministic read API intended to sit behind MCP tools."""

    def __init__(self, store: PubHarnessStore):
        self.store = store

    def task_get(self, task_id: str) -> dict[str, Any] | None:
        for kind in ("research_task", "implementation_task"):
            row = self.store.get(kind, task_id)
            if row:
                row["relations"] = self.store.relations_from(kind, task_id)
                return row
        return None

    def task_children(self, task_id: str):
        task = self.task_get(task_id)
        if not task:
            return []
        kind = task["entity_type"]
        return self.store.relations_from(kind, task_id, "child_task")

    def task_next(
        self,
        *,
        lane: str,
        owner: str | None = None,
        limit: int = 5,
    ):
        lane_map = {
            "local_research": ("research_task", "Ready", "runnable"),
            "implementation": ("implementation_task", None, None),
        }
        if lane not in lane_map:
            raise ValueError(f"unknown lane: {lane}")
        entity_type, lifecycle, agent_gate = lane_map[lane]
        rows = self.store.search(
            entity_type=entity_type,
            lifecycle=lifecycle,
            agent_gate=agent_gate,
            execution_owner=owner,
            limit=limit * 4,
        )
        if lane == "implementation":
            rows = [r for r in rows if r.get("dispatch_state") in (None, "Runnable now")]
        return rows[:limit]

    def blockers_list(self, priority: str | None = None, limit: int = 100):
        rows = self.store.search(priority=priority, limit=limit * 3)
        return [r for r in rows if (r.get("blocker") or "").strip()][:limit]

    def search(self, query: str, entity_type: str | None = None, limit: int = 25):
        return self.store.text_search(query, entity_type=entity_type, limit=limit)
