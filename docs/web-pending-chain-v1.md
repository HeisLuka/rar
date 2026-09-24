# WEB-PENDING-CHAIN-01 — bounded client causal chain

This contract allows a single writer to enqueue semantic commands without waiting one network RTT between every edit.

It is deliberately **not** a CRDT, OT layer, Undo grouping policy, IME composition model, or transport-batching policy.

## Causal base

Every operation carries one of:

- `canonical_revision(revision_id)` for the current queue head;
- `pending_after(client_operation_id)` for a command authored against predicted state after another pending command.

The operation keeps stable client operation identity, session incarnation, monotonic client sequence and normalized semantic intent.

## Failure rules

- A rejected/conflicting predecessor blocks dependents.
- Remote canonical advancement while the head is unresolved requires explicit re-resolution.
- Exact duplicate intent is idempotent; identity mismatch fails closed.
- Reconnect restores the same chain rather than flattening it to a stale canonical revision.

## Boundedness

The queue enforces explicit limits for pending count, encoded bytes, age and causal depth. Hitting a limit produces backpressure; it does not create unbounded private history.

## Relationship to recovery

`recovery-v1.mjs` owns durable local crash/reconnect persistence and exact retry planning. This module owns the live in-memory causal relationship among currently unacknowledged semantic commands. A product adapter may persist/restore its snapshot through recovery storage, but the two contracts remain separate.
