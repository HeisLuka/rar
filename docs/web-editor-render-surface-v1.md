# WEB-RENDER-01 — browser render surface preflight

This repository currently contains a **synthetic/protocol-fixture benchmark preflight**, not the final renderer technology decision.

The final decision remains blocked on the retained source-free real-PUB Scene V1 handoff owned by WEB-SCENE-ADAPTER-01.

## Candidate surfaces

The same BrowserSceneSnapshotV1 render plan is exercised through:

1. SVG/DOM;
2. Canvas2D;
3. WebGL2-hybrid geometry with DOM text/diagnostic layer and Canvas2D transient overlay.

The accelerated path is included for measurement, not because it is presumed better.

## Authority boundary

All candidates consume only BrowserSceneSnapshotV1.

They do not receive raw PUB bytes, parser records, CFB paths, SourceRefs or writer types.

The render plan preserves canonical NodeId and exact Scene EMU values. View conversion creates CSS-pixel display coordinates without modifying the snapshot.

Browser text is explicitly tagged browser_preview_only. DOM/Canvas metrics are not canonical line-break, overflow or authoring-layout authority.

## Layering

Canonical Scene V1 feeds the base page/object/resource/text-preview layer.

Transient UI state feeds a separate selection overlay and MoveGesture preview overlay.

Selection and drag preview redraw independently. They never rewrite Scene node bounds.

## Current fixture coverage

Protocol fixtures cover:

- simple text frame with non-authoritative browser text;
- image resource descriptor with opaque fetch handle;
- group + table + shape;
- partial/unsupported node with negative off-page geometry.

Synthetic stress covers 5,000 exact-stacking shape nodes. It is explicitly marked synthetic_stress and must never be represented as a real-PUB corpus percentile.

## Measurements

The headless-browser harness records per input:

- JSON parse time;
- payload bytes;
- page/node/Story/resource/diagnostic counts;
- first renderer construction plus two-animation-frame paint latency;
- repeated zoom/pan CPU update cost;
- selection overlay update cost;
- transient preview update cost;
- DOM element count;
- JS heap when the browser exposes it;
- WebGL2 availability.

Resource decode is reported separately as not measured because public protocol fixtures expose opaque resource handles, not production-fetchable image bytes.

## Why there is no winner yet

A synthetic 5,000-rectangle benchmark can expose obvious scaling problems, but it cannot tell us the real distribution of page count, text density, images, tables/groups, off-page scratch objects, diagnostics, or resource decode cost.

Every receipt therefore states real_pub=false, representative_corpus=false and technology_decision_allowed=false.

No SVG/Canvas/WebGL choice is evidence-backed until representative real source-free Scene V1 snapshots are benchmarked through the same harness.

## Closure boundary

This preflight can prove:

- all three candidates consume one public scene contract;
- exact geometry/order is shared;
- negative/off-page geometry survives;
- zoom/pan does not mutate canonical coordinates;
- selection/transient preview is an independent layer;
- missing/partial state is visible;
- renderer public API rejects raw/private source carrier fields.

WEB-RENDER-01 remains IN PROGRESS after this preflight. Final closure requires the real-PUB Scene receipt, corpus complexity measurements, browser benchmark evidence and an explicit technology decision with rejected alternatives.
