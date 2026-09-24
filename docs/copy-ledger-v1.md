# ENGINE-COPY-LEDGER-01 — copy/materialization receipt contract

`chaptera.copy-ledger.v1` is the source-neutral producer schema for measuring internal byte/data movement without publishing private document contents or implementation-specific runtime structures.

It feeds the existing `chaptera.optimization.measurement.v1` / `chaptera.optimization.receipt.v1` comparison spine. It does not create a second performance scoring system.

## Boundary

The public receipt contains only opaque workload identity, structural counts, runtime identity, aggregate allocation/memory metrics, and explicit copy/materialization sites. It must not contain source paths, filenames, document text, raw/source bytes, customer IDs, or tenant IDs.

A private/local producer may inspect real Chaptera/PUB runtime state, but the emitted receipt must already be source-free before it reaches this repository.

## Copy classes

Every measured materialization site is classified exactly once:

- `REQUIRED_TRANSFORM` — representation genuinely changes, such as decode/shaping/image decode;
- `REQUIRED_SERIALIZATION` — network/project/output/file serialization;
- `REQUIRED_LIFETIME_DETACH` — a consumer must safely outlive the only current backing;
- `AVOIDABLE_DUPLICATE` — semantically identical payload is owned/materialized again because of the internal API shape.

The adapter recomputes totals from sites. A producer cannot provide a separate unverified `total_copied_bytes` field.

## Scenario metrics

Each scenario records allocation count, allocated bytes, peak live bytes, serialized bytes, explicit copy/materialization sites, semantic/output equivalence fences, and optional `tracked_unique_payload_bytes`.

If the producer cannot defend a unique-payload denominator, `copy_amplification_ratio` is `unknown`, never zero or guessed.

## Workload identity

`fixture_binding_id` is opaque and non-content-derived. It lets baseline and candidate measurements bind to the same local/private fixture without exposing a source hash or path.

The existing optimization comparator requires exact workload and runtime identity equality between baseline and candidate.

## Evidence authority

Public synthetic fixtures validate only the contract and local optimization mechanics. They cannot authorize product technology decisions.

A sanitized local producer may set `real_product_source_free=true` only when the numbers came from a real product workload and the emitted receipt contains no private source material. That enables the existing optimization spine to treat the measurement as real-corpus evidence.

## Expected local producer flow

```text
active rar checkout + private/local runtime slice
  -> instrument known allocation/copy seams
  -> execute fixed workload
  -> classify every measured materialization
  -> emit chaptera.copy-ledger.v1 JSON with opaque fixture binding
  -> run public adapter/validator
  -> compare baseline/candidate through optimization_receipt_v1.py
```

The producer implementation language is not part of this schema. Current durable engine work is expected to use Rust; this repository only standardizes the public-safe evidence boundary.
