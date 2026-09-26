# WEB-RESOURCE-SCHEDULER-01

Coordination bootstrap for the draft PR. Replace or remove this file before the PR is marked ready if the implementation no longer needs it.

Source of truth: Notion task WEB-RESOURCE-SCHEDULER-01.

## Goal

Add a backend-independent WebResourceScheduler over current page/window resource demand and WEB-RESOURCE-READINESS generations.

The scheduler owns when derived browser work is admitted and prioritized. It must not become a new document, resource-identity, authorization, cache, decode, derivative, GPU-residency, or frame-scheduling authority.

## Required semantics

- Priority classes are bounded and explicit:
  - VisibleBlocking
  - VisibleUpgrade
  - NearWindow
  - Speculative
- Current-page blocking demand preempts speculative work.
- Equivalent resource/derivative work deduplicates.
- In-flight neighbor prefetch is promoted rather than restarted when it becomes visible.
- Navigation, LOD, hidden-tab, and memory-pressure changes cancel or demote obsolete speculation where useful.
- Correctness remains fenced by resource identity + WEB-RESOURCE-READINESS generation; cancellation is only an optimization.
- Visible blocking work cannot starve.
- Speculation may starve under sustained visible pressure.

## Stage admission

Bound and measure expensive stages separately:

- cache lookup / metadata validation;
- network fetch / integrity stream;
- decode / color transform / materialization;
- optional delivery-artifact decode.

Admission must account for configurable concurrent-item limits plus bytes/pixels/working-set estimates. Unknown-cost decode work uses conservative admission.

Reserve estimated decode working set before expensive image work so one huge image cannot accidentally overlap with an unbounded set of other decodes.

## Policy constraints

Do not hard-code universal production concurrency, byte budgets, or a fixed +/-1 neighbor radius into semantics.

Deterministic fake-scheduler tests close behavior now. Representative WEB-SCENE-SCALE receipts decide production budgets later.

## Telemetry

Emit bounded queue/admission/prefetch-usefulness signals, including:

- demand count by priority class;
- queue depth/age;
- admission wait;
- in-flight fetch bytes;
- decoded-pixel/byte estimate;
- deduped demands;
- promotions;
- cancellations/supersedes;
- stale completions;
- current-page blocking count;
- neighbor prefetch usefulness.

No ResourceId or DocumentId in metric labels.

## Required acceptance tests

- 500-page manifest does not enqueue all document resources on first page.
- Visible blocking beats neighbor/speculative work.
- Existing neighbor work is promoted instead of duplicated.
- Direct jump P1 -> P80 reprioritizes and drops obsolete speculation.
- Shared resource creates one equivalent active work item.
- Stale cancelled completion cannot publish over newer demand.
- Huge-image decode cannot create unbounded working-set overlap.
- Unknown-cost resources use conservative admission.
- Sustained interaction cannot starve visible blocking work.
- Hidden tab stops speculative growth; resume recomputes latest demand.
- Memory pressure sheds speculation before visible blocking work.
- Scheduler/cache/readiness activity emits zero canonical EditOperations / Scene revisions.

## Proven prerequisites

- WEB-RESOURCE-READINESS-01: DONE.
- WEB-SCENE-PROTOCOL-01: DONE.

## Integration guidance

Start with deterministic fake cache/fetch/decode adapters and current Scene/resource descriptors.

Do not wait for IMAGE-DERIVATIVE-ENCODE-01, an external S3 receipt, or a final real-PUB corpus.

When WEB-RESOURCE-DELIVERY-CACHE-01 lands, compose it as the scheduler fetch/cache adapter rather than duplicating its authority.

## Non-goals

- canonical page/resource mutation;
- resource authorization or signed-grant format;
- HTTP/CDN implementation;
- decoder/encoder semantics;
- GPU texture residency;
- fixed production concurrency constants;
- fixed neighbor-prefetch radius;
- offline document authority.
