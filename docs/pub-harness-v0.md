# PUB Harness v0 — Notion mirror + deterministic Codex state

Status: implementation baseline.

## Problem

PUB currently has strong authority structures in Notion—Research Tasks, Implementation Tasks, Agent Runs, Claims, Knowledge Change Events, Change Proposals, readiness views, Gora/Ouroboros maintenance—but an agent repeatedly querying these surfaces directly pays three costs:

1. hosted Query Data Source quota / API pressure;
2. repeated reconstruction of the same graph;
3. dependence on broad SQL-like scans for questions that are actually exact graph lookups.

The harness therefore needs a local machine state layer without replacing Notion as the human control plane.

## Decision

Use a local SQLite database with WAL and FTS5.

Do not add pgvector in v0. Most current PUB retrieval is deterministic by IDs, statuses, gates, owners, dependencies and relations. Semantic/vector retrieval can be added later for long-form historical evidence if measured need appears.

Do not expose raw SQL to Codex. The machine surface is a small typed API.

## Authority boundary

- Notion remains authority for current task / claim / change-control records.
- SQLite is a **derived mirror and operational index**, not a second source of truth.
- A local row carries Notion identity + edit watermark.
- Writes back to Notion are a separate, explicit operation and must use optimistic/freshness checks.
- Local DB, credentials and private page content are never committed to `HeisLuka/rar`.

## Sync model

### Phase A — cheap metadata sync

Mirror IDs, titles, lifecycle/status, priority, owner, gate, blocker, workspace, timestamps and explicit relations.

This is enough to answer most:
- next task;
- dependency;
- blocker;
- stale routing;
- owner/lane;
- graph traversal questions.

### Phase B — lazy page hydration

Fetch page Markdown only when:
- the agent opens a task;
- FTS content is explicitly requested;
- a maintenance cycle needs exact prose.

This avoids reading thousands of full page bodies on every sync.

### Phase C — bounded refresh

Each source keeps a watermark in `sync_watermarks`.

A refresh should:
1. request pages changed since the last successful watermark where the provider supports it;
2. upsert changed metadata;
3. replace explicit relation edges;
4. hydrate bodies only on demand;
5. advance the watermark only after the batch succeeds;
6. log material local changes in `change_log`.

A periodic full reconciliation remains useful as a repair pass, but should not be the normal read path.

## Typed agent surface

First operations:

- `task_get(task_id)`
- `task_children(task_id)`
- `task_next(lane, owner?, limit?)`
- `blockers_list(priority?)`
- `search(query, entity_type?)`
- `sync_status()` — next implementation slice
- `refresh_changed(source)` — next implementation slice

Later:
- `evidence_for(id)`
- `claims_for(id)`
- `runs_for(id)`
- `readiness(product_or_capability)`
- `gora_frontier()`
- `ouroboros_due()`

## PUB mapping

Research Tasks map naturally to:
- `entity_type=research_task`
- Task ID → `entity_id`
- Lifecycle, Priority, Agent Gate, Dispatch State, Execution Owner, Execution Workspace, Blocker
- Depends On / Blocks / Child Tasks → explicit edges

Implementation Tasks map to:
- `entity_type=implementation_task`
- Status/Dispatch State/Priority/Execution Owner/Execution Workspace/Gate
- Depends On Tasks / Blocks Tasks / Research Dependencies / Capability Contracts → explicit edges

The same generic entity + relation core can mirror Agent Runs, Claims and change-control records without forcing the harness to duplicate the whole Notion formula/rollup layer.

## Why not duplicate all Notion formulas

Notion contains many formulas/rollups that are useful as human dashboards but are not stable SQL columns and some are unavailable to Query Data Source.

The harness should first mirror primitive facts and explicit relations, then calculate deterministic machine views locally.

That gives:
- reproducible behavior;
- fewer remote calls;
- inspectable logic;
- no dependence on hidden rollup/query behavior.

## Promotion trigger to Postgres

Stay on SQLite until one of these becomes real:
- multiple hosts need concurrent writes to the same state;
- GitHub Actions and local agents need a single shared live store;
- local DB size or write contention becomes measurable;
- a server-side harness becomes the authority for leases/events.

At that point preserve the typed API and move the storage adapter to Postgres. pgvector remains optional.
