# CLOUD-LINK-BENCH-01D

Coordination bootstrap for the draft PR. Replace or remove this file before the PR is marked ready if the implementation no longer needs it.

Source of truth: Notion task CLOUD-LINK-BENCH-01D.

## Goal

Measurement only: attribute hosted first-bind p95 tail to exact durable substeps before changing SQLite or SourceIngress behavior.

## Required acceptance

On the pinned tiny SourceIngress fixture, collect repeated hosted first-bind samples for:

- admission reserve;
- source issue;
- stored-state CAS;
- validation-job enqueue;
- validation-job claim;
- validation execution;
- validation publish;
- validated reload;
- project/genesis commit.

Separately time deterministic scanner and in-process provider work so their wall time can be subtracted from validation.

Report:

- raw samples;
- p50 / p95 / max per substep;
- correlation of each substep with total wall time;
- residual classification showing whether tail is concentrated in SQLite durability/transaction boundaries or elsewhere.

## Constraints

- Preserve production SQLite/WAL settings.
- Preserve SourceIngress, BlobStore, security, and idempotency semantics.
- Do not change synchronous or journal pragmas in this task.
- Use a dedicated 01D workflow so 01/01B/01C are not rerun on every iteration.

## Proven prerequisite

- CLOUD-LINK-BENCH-01C is DONE.
- Prior implementation path: #880, #889, #896, #899.
