# WEB-TEXT-PROTOCOL-01 — StoryRangeIntentV1

This public-safe V1 contract bridges browser text surfaces to canonical Story mutation without making DOM/contenteditable state document truth.

## Canonical intent

Durable text mutation is:

`replace_story_range(story_id, [start_scalar,end_scalar), replacement_text)`

Range indices are Unicode scalar positions. Browser textarea/DOM offsets are UTF-16 code-unit offsets and must be converted explicitly. A UTF-16 offset that splits a surrogate pair is rejected.

Browser-native Backspace/grapheme behavior is **not** a canonical mutation primitive. Rar browser evidence showed cross-engine divergence for ZWJ emoji deletion, so the browser must observe/normalize the resulting replacement and send one explicit scalar range.

## Composition

`compositionupdate` is transient UI state only and emits no durable intent. `compositionend` may produce one normalized Story range intent if the Story text changed.

Native OS IME acceptance remains outside this public contract gate; headless Rar evidence does not claim real IME coverage.

## Causality

Multiple unacknowledged local intents form an explicit chain with `depends_on_client_operation_id`. They remain anchored to the known base revision. Acceptance is processed in causal order.

If an operation is rejected/stale, it and all dependent pending intents are invalidated and the client must refresh/reconcile. There is no generic hidden rebase in V1.

## Authority fence

Wire requests may contain only the user intent. Browser code cannot supply authoritative before-text hashes, resulting text hashes, line breaks, overflow, glyph layout, DOM ranges or browser metrics as document truth.

The server revision seam requires the authoritative executor to return the canonical operation plus server-derived `before_text_hash` and `after_text_hash`.

## Idempotency

The existing RevisionKernel rules apply:
- same document + same client operation id + same request hash → same accepted result;
- same id + different payload → fail closed;
- stale base revision → reject before authoritative executor runs;
- source identity is immutable.

## Limits

This closes the public/browser-safe protocol slice only. It does not close real canonical Story acceptance, real OS IME, server authoritative text layout/reflow, linked-frame reconciliation, undo grouping, or same-Story collaboration.
