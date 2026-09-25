# WEB-RENDER-01 — browser render surface decision

WEB-RENDER-01 is now evidence-backed for the V1 browser surface:

- **primary:** Canvas2D;
- **compatibility fallback:** SVG;
- **optional accelerated experiment:** WebGL2-hybrid.

This is a browser display/runtime decision only. Browser rendering remains non-authoritative for canonical authoring/layout truth.

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

## Decision evidence

The decision combines three receipts rather than treating one microbenchmark as authority.

### Cross-browser color/alpha surface

The WEB-COLOR-SURFACE-01 receipt proves the required V1 SDR/sRGB/explicit-alpha contract in Chromium and Firefox:

- Canvas2D and SVG are available in both engines;
- cross-browser maximum channel delta is 1 for both required surfaces;
- WebGL2-hybrid is available in Chromium but unavailable in Firefox headless;
- wide-gamut/HDR is not required for V1 and WebGPU is not an active renderer candidate.

### Pinned real-PUB Scene

The retained SampleNewsletter Scene V1 measurement is a real source-free PUB-derived scene:

- 53,630 serialized bytes;
- 8 pages;
- 68 nodes;
- 44 stories;
- 1 resource;
- 51 diagnostics.

Chromium first render:
- SVG: 14.2 ms;
- Canvas2D: 33.8 ms;
- WebGL2-hybrid: 33.1 ms.

Firefox first render:
- SVG: 15 ms;
- Canvas2D: 32 ms;
- WebGL2-hybrid: unavailable.

All available surfaces keep interaction-update measurements comfortably bounded for this scene. SVG wins first paint on this small real document, but that alone is not sufficient because the V1 surface must also remain bounded as scene cardinality grows.

### 5,000-node scale pressure

The synthetic stress receipt is not representative corpus evidence, but it is valid scale-pressure evidence:

Chromium:
- SVG: 87.3 ms first render, 35.2 ms pan/zoom p50, 41.8 ms p95, 5,004 DOM elements;
- Canvas2D: 47.5 ms first render, 13.9 ms p50, 17.9 ms p95, 2 DOM elements;
- WebGL2-hybrid: 49.8 ms first render, 14.4 ms p50, 16.2 ms p95, 3 DOM elements.

Firefox:
- SVG: 68 ms first render, 33 ms pan/zoom p50, 35 ms p95, 5,004 DOM elements;
- Canvas2D: 29 ms first render, 14 ms p50, 21 ms p95, 2 DOM elements;
- WebGL2-hybrid: unavailable.

### Bounded V1 conclusion

Canvas2D is the V1 primary because it is available in both required browsers, satisfies the same bounded SDR/sRGB/alpha contract as SVG, and avoids SVG's DOM/cardinality pressure at 5,000 nodes.

SVG remains the compatibility/fallback surface because it is cross-browser, color-equivalent within the V1 tolerance, and fastest on the current small real SampleNewsletter scene.

WebGL2-hybrid is not a required V1 path because Firefox availability is not proven. It may remain an optional accelerated experiment where capability detection succeeds; product correctness must not depend on it.

This decision does **not** claim that Canvas2D is permanently optimal for every future corpus or that synthetic stress is representative. Re-open the decision only with materially different representative corpus evidence or a changed browser capability floor.

## Closure boundary

This preflight can prove:

- all three candidates consume one public scene contract;
- exact geometry/order is shared;
- negative/off-page geometry survives;
- zoom/pan does not mutate canonical coordinates;
- selection/transient preview is an independent layer;
- missing/partial state is visible;
- renderer public API rejects raw/private source carrier fields.

WEB-RENDER-01 closes when this decision contract, the retained real-PUB benchmark receipt, and the cross-browser color-surface receipt are green on current main. Browser resource delivery/image decode and canonical authoring/layout authority remain owned by their separate tasks.
