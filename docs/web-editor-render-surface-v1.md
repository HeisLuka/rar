# WEB-RENDER-01 — browser render surface V0 decision

The bounded visible-page V0 browser editor uses **SVG as its primary renderer**.

Canvas2D remains a retained scale/fallback lane. WebGL2-hybrid remains optional/experimental and product correctness does not depend on its availability. No synthetic threshold is used to auto-switch renderers.

## Evidence correction

The first real-PUB benchmark (rar#583 / V1) was structurally valid but targeted page order 0 of SampleNewsletter. That page is empty, while all 68 Scene nodes are on later pages. Its screenshots were therefore blank page chrome and its timings are not renderer-decision evidence.

rar#599 corrected the benchmark by focusing the first populated page and added an image-level gate that rejects blank real-PUB screenshots. The V2 receipt is the authoritative real visible-page measurement.

## V2 real visible SampleNewsletter

Pinned BrowserSceneSnapshotV1:

- 53,630 serialized bytes;
- 8 pages;
- 68 nodes;
- 44 stories;
- 1 resource;
- 51 diagnostics;
- focused page order 1.

Chromium:

- SVG first render: about 11.3 ms;
- Canvas2D: about 39.9 ms;
- WebGL2-hybrid: about 33.1 ms;
- SVG pan/zoom p50: about 1.3 ms;
- Canvas2D pan/zoom p50: about 0.8 ms.

Firefox:

- SVG first render: about 11 ms;
- Canvas2D: about 33 ms;
- WebGL2-hybrid: unavailable;
- SVG pan/zoom p50: about 1 ms;
- Canvas2D pan/zoom p50: about 2 ms.

For the current representative bounded visible-page case, SVG has the strongest first-paint evidence in both required browser engines.

## Scale pressure

The retained synthetic 5,000-node stress probe is useful pressure evidence, but it is not representative corpus evidence.

It shows Canvas2D scaling materially better than SVG at high node cardinality and avoids SVG's roughly 5,000-element DOM growth. This keeps Canvas2D valuable as a retained scale/fallback lane.

It does **not** justify inventing a numeric automatic switch threshold. Such a threshold requires representative multi-document corpus evidence and is not part of V0.

## Color and alpha contract

WEB-COLOR-SURFACE-01 exercises the active candidate surfaces in Chromium and Firefox under the bounded V1 target:

- SDR;
- sRGB;
- explicit alpha;
- deterministic pixel/readback comparison.

Canvas2D and SVG are required cross-browser surfaces and pass with maximum channel delta 1. WebGL2-hybrid must satisfy the same contract when available; explicit unavailability is a compatibility fact, not a false failure. WebGPU is not an active renderer candidate.

The readback receipt does not claim absolute monitor colorimetry. P3/HDR remain optional future capabilities.

## Product binding

BrowserEditorShellV1 already defaults to `rendererKind = "svg"`. This matches the corrected visible-page V2 evidence, so closure does not require another renderer switch.

The V0 decision is therefore:

1. **SVG primary** for the bounded visible-page editor;
2. **Canvas2D retained scale/fallback lane**;
3. **WebGL2-hybrid optional/experimental**;
4. no automatic renderer threshold without representative evidence.

## Authority boundary

All renderers consume only BrowserSceneSnapshotV1. They do not receive raw PUB bytes, parser records, CFB paths, SourceRefs or writer types.

Browser rendering is display/runtime authority only. Canonical layout, authoring geometry, text flow and export truth remain server/canonical-model responsibilities.

## Re-open rule

Re-open the renderer decision only when materially broader representative corpus evidence demonstrates that the bounded SVG V0 choice is no longer adequate, or when the supported browser capability floor changes.

Synthetic stress by itself is not sufficient to overturn the V0 product default.
