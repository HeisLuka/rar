# RENDER-BENCH-01 — public benchmark harness

This harness measures the actual public RenderScene v1 + ScenePatch v1 path.

It includes product-grounded source-neutral workloads for image-heavy pages, shaped-text/overset, overlapping objects, off-page geometry and multi-page scenes, plus synthetic 10k/50k/100k primitive stress cases.

The receipt records compile time, compiled JSON size, peak Python allocation tracing, transform/resource table sizes, incremental patch generation/apply, page/node frame preparation, first-visible-page compile, preview-overlay update cost and a 16-up output-sheet instancing witness without cloned authoring nodes.

The current public Rar repository still lacks a genuine PUB-derived Scene receipt. Therefore this benchmark is deliberately marked `real_pub_scene_present=false` and cannot by itself close the complete RENDER-BENCH-01 acceptance gate. GPU/backend timings are also explicitly deferred to backend work.
