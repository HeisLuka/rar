# VISREG-01 — structured visual regression oracle V1

This public-safe validation layer compares two `chaptera.scene.v1` snapshots under
the same document/source/Layout Environment and emits a deterministic structured
report.

The first gate intentionally does **not** use pixel equality as semantic truth.
Differences are localized by page, canonical NodeId or StoryId and separated into:

- `structure` — page/node identity, hierarchy or kind changes;
- `layout` — page geometry, node bounds/transforms and stacking changes;
- `text_layout` — Story text/fidelity and Story-frame topology changes;
- `render_input` — paint/resource binding or content changes;
- `render` — downstream render-artifact/region differences supplied as secondary evidence;
- `contract` — diagnostic/capability/fidelity-state changes.

Snapshot/revision ids are evidence metadata, not regression classes. Inputs must
share the same source identity and Layout Environment; mismatches fail closed
rather than being mislabeled as visual regressions.

## Determinism

The report is normalized, sorted by stage/page/origin/code and hashed as
`report_hash`. Re-running the same comparison must produce identical canonical
JSON.

## CI contract

The synthetic contract uses the existing source-free Viewer→Scene fixture and
creates three intentional mutations:

1. one node geometry change;
2. one Story text change;
3. one render-artifact/region change with identical Scene state.

CI requires these to be classified as layout, text-layout and render-only
respectively. This proves stage separation and origin localization; it is not a
claim of representative real-PUB fidelity coverage.
