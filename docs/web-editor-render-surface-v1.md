# WEB-RENDER-01 — evidence-backed browser render surface V1

## Decision

Chaptera Web V1 keeps the existing **SVG renderer as the primary browser scene surface**.

The bounded policy is:

- `svg` — V1 primary renderer;
- `canvas2d` — retained scale lane / explicit alternative;
- `webgl2-hybrid` — optional experimental accelerator only;
- automatic node-count/backend switching — **not defined in V1**.

The product shell already defaulted to SVG. WEB-RENDER-01 therefore validates and names the existing default rather than replacing it by taste.

Synthetic measurements are not allowed to invent a production switch threshold. Canvas2D may become an automatic high-density path only after representative real-PUB evidence establishes a safe trigger and fidelity equivalence.

## Real-PUB evidence

The authoritative visible-render receipt is the corrected WEB-RENDER-01 V2 run from rar#599.

Input is the pinned, source-free SampleNewsletter `BrowserSceneSnapshotV1`:

- 53,630 serialized bytes;
- 8 pages;
- 68 nodes;
- 44 Stories;
- 1 resource;
- 51 explicit diagnostics;
- partial fidelity remains explicit.

V2 corrects the earlier V1 benchmark bug: page order 0 is genuinely empty, while document nodes begin on page order 1. The benchmark now focuses the first populated page and fails if an available renderer screenshot contains no visible document pixels.

### Visible first paint and pan

On the focused real page:

| Browser | Renderer | First paint | Pan p50 | Pan p95 | DOM elements |
| --- | --- | ---: | ---: | ---: | ---: |
| Chromium | SVG | ~11.3 ms | ~1.3 ms | ~1.9 ms | 141 |
| Chromium | Canvas2D | ~39.9 ms | ~0.8 ms | ~1.2 ms | 2 |
| Chromium | WebGL2-hybrid | ~33.1 ms | ~0.9 ms | ~1.7 ms | 65 |
| Firefox | SVG | ~11 ms | ~1 ms | ~3 ms | 141 |
| Firefox | Canvas2D | ~33 ms | ~2 ms | ~2 ms | 2 |
| Firefox | WebGL2-hybrid | unavailable | — | — | — |

Both Chromium and Firefox V2 jobs pass the image-level visible-document gate.

The Chromium WebGL2-hybrid screenshot is not equivalent to the SVG/Canvas screenshots on this partial Scene: frame/geometry chrome present in SVG/Canvas is missing from the hybrid output. Together with Firefox WebGL2 unavailability, this excludes WebGL2-hybrid as the sole or primary V1 path.

## Color-surface evidence

WEB-COLOR-SURFACE-01 / rar#577 measures an explicit SDR sRGB + alpha fixture independently of document rendering.

For the required cross-browser surfaces:

- Canvas2D and SVG are available in Chromium and Firefox;
- Canvas2D ↔ SVG maximum same-browser channel delta is 0;
- each surface stays within 1 channel value of the expected fixture;
- Chromium WebGL2 is within 1 channel value of Canvas/SVG;
- Firefox WebGL2 is unavailable and is recorded as a capability fact, not hidden by fallback;
- screenshot/readback evidence does **not** claim absolute monitor colorimetry.

Wide-gamut P3, HDR and WebGPU remain outside the V1 closure boundary.

## Scale evidence

Scale receipts are deliberately classified as synthetic and do not represent corpus percentiles.

They still expose the implementation trade-off:

- SVG DOM count grows with visible nodes;
- Canvas2D keeps the base renderer at a constant small DOM footprint;
- at 1,000 and 5,000 synthetic nodes Canvas2D has materially lower pan/update cost than full-scene SVG;
- bounding rendering to a small page window collapses SVG DOM/update cost sharply.

Therefore V1 keeps Canvas2D available as a scale lane, but does not guess a production node-count threshold from synthetic data.

## Authority boundary

All renderers consume only `BrowserSceneSnapshotV1`.

They never receive raw PUB bytes, CFB paths, parser records, SourceRefs, writer internals or filesystem carriers.

Browser rendering is display authority only:

- canonical EMU geometry and Node identity remain in Scene;
- browser text remains `browser_preview_only`;
- browser font metrics are not Publisher reflow authority;
- selection and transient Move previews are overlay state, not Scene mutation;
- unsupported/partial fidelity remains explicit.

## V1 policy contract

`apps/web/render-v1.mjs` exports `RENDERER_POLICY_V1`:

- `primary = "svg"`;
- `retained_scale_lane = "canvas2d"`;
- `optional_accelerators = ["webgl2-hybrid"]`;
- `automatic_switch_threshold = null`.

`BrowserEditorShellV1` consumes the named primary policy instead of embedding a second renderer choice.

Changing the V1 primary or adding an automatic scale switch requires new representative real-PUB receipts; a synthetic benchmark alone is insufficient.
