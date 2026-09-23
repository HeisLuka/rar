# WEB-ACCEPTANCE-01 — real PUB acceptance preflight

This contract prevents a synthetic browser demo from being reported as the first real Web Editor acceptance.

## Pinned real fixture

The acceptance input is the official Apache POI `SampleNewsletter.pub` fixture:

- family: mature 0x2C;
- Git blob: `94900925af5832c493784f3cb51563f838a64df8`;
- bytes: `291840`;
- SHA-256: `6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf`.

Public CI retrieves the immutable upstream Git blob and verifies both byte count and SHA-256 before doing anything else.

## Three evidence planes

Final closure requires three separate real receipts.

1. **Viewer geometry receipt** — canonical private core parses the real PUB and emits only sanitized source-free Viewer geometry JSON. Public `miy` adapts that JSON into Scene V1.
2. **Revision producer receipt** — canonical EditorSession processes a MoveNodeTo against the same source hash and emits the already-versioned `core_integration=true` revision receipt.
3. **Browser acceptance receipt** — a real browser opens the resulting scene, selects the canonical NodeId, performs transient drag preview, commits exactly one MoveNode, verifies undo/redo/reopen/export and records browser/runtime/output evidence.

No one receipt substitutes for another.

## Fail-closed rule

`tools/validate_web_acceptance_preflight.py` has two modes:

- `preflight` verifies the pinned real fixture and classifies missing real receipts without claiming product acceptance;
- `strict` exits non-zero until all three real receipts are present, mutually consistent and schema-valid.

A green preflight therefore means **the gate is correctly wired**, not that Web Editor acceptance has passed.

## Private/public boundary

`pub-rs` remains private and is not copied into public `miy`.

Only sanitized data receipts cross the boundary. The public validator rejects closure unless:

- revision receipt names the pinned source SHA;
- Viewer-derived Scene V1 names the same source and canonical baseline revision;
- browser receipt names the same initial/accepted revisions and exact initial Scene snapshot;
- source SHA remains unchanged;
- native PUB save remains disabled.

## Current expected state

Until the two canonical private-core receipts are produced, the expected machine result is:

`claim = preflight_only_not_product_acceptance`.

That state is intentional and must not be upgraded by screenshots, synthetic fixtures or unit-test-only evidence.
