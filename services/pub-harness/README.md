# PUB Harness v0

Local deterministic state layer for Codex/agent workflows.

## Boundary

This directory contains only public-safe code and schema.

Do **not** commit:
- Notion credentials or tokens;
- a live `pub-harness.db`;
- private/customer page bodies;
- private runtime artifacts.

The live SQLite database is local state. Notion remains the human-facing control plane and canonical source for task/knowledge records until a specific authority contract says otherwise.

## v0 architecture

```
Notion authority DBs
      |
      | bounded sync
      v
SQLite + WAL + FTS5
      |
      | typed query surface
      v
Codex / local agents
```

The agent is intentionally not given arbitrary SQL. It should use bounded operations such as:
- `task_get(id)`
- `task_next(lane, owner, limit)`
- `task_children(id)`
- `blockers_list(priority)`
- `search(query, entity_type)`

## Initial mirrored entity classes

1. Research Tasks
2. Implementation Tasks
3. Agent Runs
4. Canonical Claims
5. Knowledge Change Events
6. Change Proposals

v0 starts with task metadata + relations. Page body hydration is lazy and should happen only for pages the agent actually needs.

## Local smoke test

```bash
cd services/pub-harness
python test_store.py
```

No external Python packages are required for the store/query core.
