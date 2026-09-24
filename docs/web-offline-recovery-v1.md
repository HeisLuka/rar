# WEB-OFFLINE-01 — browser local recovery V1

The historical task name says "offline", but V1 does **not** promise an offline
authoring mode. This contract implements best-effort browser recovery continuity
around a server-authoritative editor.

## Contract

- Normalized intent is persisted before the UI may claim it is recoverable.
- Retry/restart preserves the exact client operation id; it never invents a new
  identity for an unknown outcome.
- A sent/unknown operation is looked up by operation identity before resend.
- Recovery re-bootstraps current RevisionId, lifecycle generation, AuthZ, app
  version and command-semantic version.
- A stale base revision requires refresh; the client recovery layer never
  silently rebases or auto-merges it.
- Auth revoke, lifecycle mismatch and semantic-version mismatch quarantine the
  pending record instead of blindly replaying it.
- IndexedDB is the V1 small/queryable durable store.
- Web Locks and BroadcastChannel may coordinate sibling tabs, but correctness
  does not depend on them and they are never document authority.
- Local storage write/quota failure is surfaced as a typed recovery-storage
  failure; V1 does not silently claim durability.

## Evidence boundary

Node tests prove the deterministic recovery state machine. Chromium and Firefox
CI use a persistent browser profile to prove actual IndexedDB cross-tab
visibility and restart persistence. This is still not canonical-service
reconnect acceptance, eviction-guarantee evidence, Safari/mobile coverage or
full offline/PWA support.
