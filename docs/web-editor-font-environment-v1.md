# WEB-FONT-ENV-01 — browser/server font and layout-environment fence V1

The browser must be able to answer **which authoritative layout/font environment this preview represents** without becoming a second typography engine.

## Protocol split

Scene V1 remains the source-neutral document/view projection.

`BrowserFontEnvironmentV1` is a sidecar policy contract bound to exactly one:

- document;
- revision;
- Scene snapshot;
- Layout Environment;
- font-set fingerprint.

A sidecar with any mismatched identity is invalid for that scene.

## Preview authority

V1 declares one of:

- `server_positioned_glyphs` — browser renders server-resolved glyph positions; browser font metrics may not relayout;
- `server_frame_geometry_only` — Story/frame geometry is server-derived but browser-native text layout is preview/composition only;
- `server_render_only` — browser must not perform document-text layout for the authoritative preview.

A Scene whose `render.text` capability is not `supported` cannot be relabeled `server_positioned_glyphs`.

## Font delivery

Every font descriptor carries stable server-provided fingerprint + face index and one explicit disposition:

- `deliver_exact`;
- `deliver_subset`;
- `substitute_explicit`;
- `server_render_only`;
- `blocked`.

There is no implicit system/host font lookup.

Exact/subset delivery requires an explicit content hash + resource id + opaque fetch handle.

Substitution requires one explicit fingerprinted fallback resource. It is never “Arial if installed” or another environment-dependent guess.

Server-only/blocked states expose no font resource handle.

## Diagnostics and privacy

Missing/restricted/blocked/substituted state is explicit and user-visible through typed diagnostics. Local font paths, host font names chosen by lookup, raw font bytes and filesystem paths are not protocol fields.

Licensing/legal judgement is upstream policy input; this contract only transports the resulting technical disposition.

## Editing/IME

Browser-native text composition may use local UI metrics transiently. On commit:

1. browser sends semantic Story-text intent;
2. server reshapes/relayouts under its explicit Layout Environment;
3. new Scene snapshot is authoritative;
4. browser reconciles to that snapshot;
5. overflow/reflow/substitution changes remain visible.

Browser composition metrics never rewrite canonical line breaks/frame geometry by themselves.

## Remaining closure receipts

This public slice proves protocol/environment fencing and no-implicit-fallback behavior. WEB-FONT-ENV-01 remains IN PROGRESS until representative real Story frames are exercised in actual browser engines and compared to authoritative server output, including at least Chromium plus one independent engine where infrastructure permits.


## Rar real StoryFrame closure path

The current pinned real `SampleNewsletter.pub` Scene is explicitly a geometry-only typography projection:

- `render.text=partial`;
- `preview_authority=server_frame_geometry_only`;
- browser-native text metrics are never document authority;
- no browser/system font lookup is promoted into canonical layout state.

The real closure probe therefore compares Chromium and Firefox against the same server-derived StoryFrame geometry, not against an invented claim of Publisher-equivalent browser typography. Each browser must preserve the canonical frame bounds exactly through the Scene render plan. Native text width/height observations are retained only to demonstrate that browser-local metrics can differ without changing the authoritative Scene.

A future `server_positioned_glyphs` mode requires a Scene whose `render.text` capability is actually `supported`; this V1 gate fails closed rather than relabeling the current partial projection.
