# RENDER-PATCH-APPLY-01 — renderer-owned in-place apply

ScenePatch v1 keeps a pure `apply_patch(base, patch)` API as its compatibility and correctness oracle. That API intentionally deep-copies the complete base RenderScene before applying a patch.

The renderer hot path may instead call `apply_patch_in_place(scene, patch)` only when it exclusively owns that disposable compiled RenderScene. RenderScene v1 is renderer state, not authoring/layout truth; no AuthoringModel, parser graph or shared authority state is eligible for this API.

## Failure boundary

A wrong `base_render_scene_id` is rejected before any mutation. After mutation begins, final target identity is still verified. If that final identity check fails, `ScenePatchApplyPoisoned` is raised and the renderer-owned scene must be discarded and rebuilt. The API does not pretend to provide rollback without a copy.

## Compatibility proof

CI keeps the pure API and existing ScenePatch JSON vocabulary unchanged. Tests prove pure apply, in-place apply and clean full compile are identical for:

- signed/off-page one-node geometry changes;
- interleaved multi-page paint order;
- resource-table deltas;
- diagnostic/order-authority deltas.

A separate 2k/10k receipt compares pure-copy and in-place timing. Timing is evidence only; semantic and identity equivalence is the gate.
