# WEB-REVISION-01 — immutable history transition seam

The public V1 revision kernel now has an explicit history-transition path for
single-user Undo/Redo.

The browser intent contains only `{kind: "undo"}` or `{kind: "redo"}`.
It does not name an authoritative target revision, state id, or EditorProject.
The authoritative executor owns the actual EditorSession transition and returns
the resulting canonical project.

Each accepted history transition creates a fresh immutable revision node. The
resulting `state_id` may equal an earlier revision's state id:

```text
R0 / S0
  --MoveNode--> R1 / S1
  --Undo-----> R2 / S0
  --Redo-----> R3 / S1
```

Thus content-equivalent state identity and history identity remain distinct.
Old revisions remain addressable. Exact retry is idempotent; stale base,
source mismatch, and same-id/different-intent fail before authoritative
execution.

This closes a public-kernel DoD gap only. The executor used by public tests is
synthetic. Real closure still requires WEB-REVISION-ADAPTER-01 to drive the
same envelope from canonical `EditorSession::undo/redo`.
