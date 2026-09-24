# ENGINE-COPY-LEDGER-01 — source-neutral copy ledger contract

This contract standardizes how the canonical Chaptera runtime can report byte ownership, copies and materialization without exposing raw PUB bytes or private source.

Every event classifies one materialization as exactly one of:

- `required_transform` — a new representation is semantically required (for example text → glyph arrays or encoded image → decoded pixels);
- `required_serialization` — process/network/file/output boundary requires materialization;
- `required_lifetime_detach` — a consumer must safely outlive the current backing and no shared owner exists;
- `avoidable_duplicate` — the same semantic payload is owned/materialized again only because of the current API/runtime shape.

The receipt records stable payload identity, logical bytes, materialized/shared bytes, instances and optional allocation/live/peak/retained counters. Unknown counters are represented as `null`, never as fabricated zeroes.

The summary is recomputed from event data and reports per-payload-class unique logical bytes, materialized bytes, avoidable duplicate bytes, and a materialization amplification ratio. This is a diagnostic ratio, not a fidelity/performance score.

The checked-in fixture is deliberately `synthetic_contract_fixture` and has `technology_decision_allowed=false`. It proves the public receipt/validator only. ENGINE-COPY-LEDGER-01 remains open until an authorized canonical runtime produces a sanitized `real_pub_source_free` receipt across representative real PUB workloads.


## Public-safety and optimization-spine integration

The receipt schema is fail-closed on unexpected fields in the receipt envelope, producer/runtime identity, equivalence/evidence blocks and every materialization event. A local/private producer must extend the versioned schema deliberately rather than attaching source paths, document text, raw bytes or private ad-hoc metadata to a public receipt.

`optimization_receipt_v1.copy_ledger_measurement()` normalizes a validated ledger into the shared `chaptera.optimization.measurement.v1` spine. Copy/materialization optimizations therefore use the same exact workload/runtime compatibility checks, metric-specific regression budgets, correctness/fidelity fence and real-corpus authority rules as other Chaptera optimization work. There is no separate zero-copy score or comparator.
