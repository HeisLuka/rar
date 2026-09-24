# RENDER-PATCH-01 — ScenePatch v1

ScenePatch v1 is a renderer synchronization diff between two compiled RenderScene revisions. It is not an authoring operation log.

The initial patch vocabulary is deliberately coarse: removed NodeIds, full compiled node upserts, page/resource replacements when changed, order deltas and diagnostics replacement. A node may own several RenderAtomIds.

The core invariant is:

`apply(full_compile(N), patch(N -> N+1)) == full_compile(N+1)`

Transient pointer motion is outside ScenePatch. Preview/overlay state produces zero durable patches. One accepted semantic edit upstream creates one new scene revision and then one bounded patch.
