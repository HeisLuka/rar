# RENDER-PATCH-INDEX-01 — linear ScenePatch ownership index

The original ScenePatch v1 diff was semantically correct but generated every node comparison by rescanning all compiled primitive tables. A scene with N nodes and roughly N primitives therefore performed O(N²)-like primitive visits even when only one node changed.

This gate changes only the implementation strategy. ScenePatch v1 remains the same coarse source-neutral synchronization vocabulary: removed NodeIds, node upserts, page/resource/order deltas and diagnostics.

The indexed path builds one NodeId → {rects, images, glyph_runs} ownership map for each compiled scene, preserving compiler table order and holding references rather than deep-copying unchanged atoms. Only changed target atoms are deep-copied into the emitted patch.

Correctness does not depend on wall-clock thresholds. CI proves exact single-pass primitive visit counts, byte/semantic equivalence with the legacy diff on a representative fixture, and apply(full_compile(N), patch(N→N+1)) == full_compile(N+1). A separate 2k/10k receipt records timing as evidence only.

This task does not close the genuine-PUB arm of RENDER-BENCH-01 and does not introduce GPU, PUB/parser or authoring-operation types.
