# PREFLIGHT-01 continuous fidelity diagnostics

This is a public-safe source-neutral diagnostic engine over already-grounded state.

It does **not** parse PUB, choose export dispositions, serialize output, or invent target planning.

Stable v1 machine codes:

- `TEXT_OVERFLOW`
- `RESOURCE_MISSING`
- `RESOURCE_MODIFIED`
- `SEMANTIC_UNSUPPORTED`
- `SEMANTIC_OPAQUE`
- `OUTPUT_RISK`

Every diagnostic is object-scoped by page/node and, when available, Story origin.
The machine counts and human-readable summary are generated from the same normalized diagnostic list.

The CI acceptance proves that fixing one bounded overflow issue removes only `TEXT_OVERFLOW` after revalidation and preserves unrelated resource/semantic/output warnings.
