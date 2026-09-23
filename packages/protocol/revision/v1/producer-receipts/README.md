# Canonical revision producer receipts

This directory accepts sanitized, source-free evidence emitted by the canonical server-side EditorSession integration.

A real receipt must validate against `../producer-receipt.schema.json` and pass:

```bash
python tools/validate_revision_producer_receipt.py packages/protocol/revision/v1/producer-receipts/<receipt>.json
```

A receipt is data evidence, not source code. It must not contain raw PUB bytes, CFB/Quill/Escher carriers, local filesystem paths, credentials, private repository URLs, or private implementation code.

The first closure receipt for WEB-REVISION-ADAPTER-01 should prove one bounded `MoveNodeTo` arm against the canonical EditorSession path, including replay, stale-base and idempotency probes.

Synthetic receipts do not close the gate.
