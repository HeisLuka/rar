# Canonical revision producer receipts

This directory accepts sanitized, source-free evidence emitted by the canonical server-side EditorSession integration.

A real receipt must validate against `../producer-receipt.schema.json` and pass:

```bash
python tools/validate_revision_producer_receipt.py packages/protocol/revision/v1/producer-receipts/<receipt>.json
```

A receipt is data evidence, not source code. It must not contain raw PUB bytes, CFB/Quill/Escher carriers, local filesystem paths, credentials, private repository URLs, or private implementation code.

The first closure receipt for WEB-REVISION-ADAPTER-01 should prove one bounded `MoveNodeTo` arm against the canonical EditorSession path, including replay, stale-base and idempotency probes.

Synthetic receipts do not close the gate.


## V1 public allowlist

The V1 receipt schema is a source-free allowlist for the canonical EditorProject and
MoveNode commit arm used by WEB-REVISION-ADAPTER-01. It rejects unknown request,
accepted-result, project, operation and asset fields instead of relying on the semantic
validator to ignore them. Editor replacement asset bytes are never part of EditorProject
and are not admitted here.

The bounded probes are also typed fail-closed evidence: `stale_base.code` must be
`stale_revision`, `idempotency_conflict.code` must be `idempotency_conflict`,
and both must prove zero executor calls and no revision movement.
