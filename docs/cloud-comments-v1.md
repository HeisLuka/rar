# CLOUD-COMMENTS-01 — comments/review service V1

This public reference contract materializes the comment-store and semantic-anchor
decision already selected by the Rar comment-anchor modelcheck.

## Contract

- Comments are separate collaboration state, not RevisionStream mutations.
- Create/reply/resolve/reopen/delete are idempotent commands under current AuthZ.
- Reads re-check current document access.
- Node anchors use canonical NodeId; move/resize/page move preserve identity.
- Story anchors use StoryId plus canonical Unicode-scalar [start,end) with
  start/right and end/left boundary affinity.
- Full replacement/deletion of the anchored Story target explicitly orphans the
  thread rather than silently retargeting it.
- Fork/copy inherits no comments by default.
- Hard document purge removes comments and is terminal.

## Evidence boundary

The CI receipt is a public executable service-reference receipt. It does not
claim deployed persistence, real EditorSession Story split/merge integration,
notification delivery, moderation, regional retention, or accessibility
acceptance.
