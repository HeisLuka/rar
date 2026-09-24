# Browser scene protocol V1

Owner: `WEB-SCENE-PROTOCOL-01`

Architecture base: `miy/main@7bf8ebac20283c00868388c6556834cfc767359d`

## Purpose

This protocol is the source-neutral boundary between the server-authoritative Chaptera document/layout pipeline and the browser.

It is **not** the PUB parser model, `ViewerGeometryDocument`, `BoundedResolvedScene`, or the renderer's internal GPU/scene IR.

## Snapshot first

V1 requires a deterministic full snapshot:

`chaptera.scene.v1`

The snapshot carries:
- immutable source identity;
- immutable revision identity;
- derived snapshot identity;
- authoritative Layout Environment identity;
- canonical page/node/Story/resource identities;
- exact integer EMU geometry and exact decimal/EMU affine transforms;
- source-neutral Story text needed for bounded browser rendering;
- bounded fill/stroke paint projection;
- content-addressed/opaque resource descriptors;
- typed diagnostics, capabilities and fidelity state.

The browser may render and hit-test this projection. It does not become authoring or layout authority.

Text delivery does **not** make browser text layout authoritative. If exact typography/glyph placement is not represented, the capability/fidelity state must say so.

## Canonical scalar rules

- canonical entity ids use lowercase UUID text;
- source/content hashes use lowercase 64-hex SHA-256;
- revision/snapshot/environment identities use `sha256:<64 lowercase hex>`;
- document geometry uses signed integer EMU;
- width/height are positive integer EMU;
- screen pixels/floating point do not cross this protocol.

## Field allowlist

V1 exposes only:
- source/document/revision/snapshot identity;
- Layout Environment fences;
- pages;
- canonical nodes and hierarchy;
- Story text;
- Story-to-frame topology;
- bounded source-neutral paints;
- resource descriptors/opaque fetch handles;
- diagnostics;
- capabilities;
- fidelity state.

It does not expose CFB/Quill/Escher carriers, parser offsets, byte ranges, source paths, filesystem paths, raw embedded bytes, native writer state, layout caches, renderer slots, GPU buffers or browser presence/selection state.

## Normalized order

A producer must normalize arrays before snapshot identity is calculated.

1. pages: `(order, page_id)`;
2. nodes: page order, then known stacking values first, unknown (`null`) stacking last, then `node_id`;
3. stories: `story_id`;
4. story frames: `(story_id, frame_ordinal, node_id)`;
5. paints: `paint_id`;
6. resources: `resource_id`;
7. diagnostics: severity rank `info < warning < error`, then `(code, origin_node_id-or-empty, message_key)`;
8. capabilities: `(key, state, note-or-empty)`;
9. fidelity reasons: lexical order.

Object keys are serialized lexically for canonical byte receipts.

## Transform and stacking authority

Scene V1 preserves source-free affine transforms exactly: `a/b/c/d` are normalized decimal strings and `tx/ty` are integer EMU. The browser may apply these values for rendering; it must not round them into authoring truth.

Stacking is separately fenced by `stacking_fidelity`:
- `exact` requires concrete `z_order` and `paint_order` for every node;
- `partial` means only a bounded subset is authoritative;
- `unknown` means the producer has no grounded authored/paint order.

The current bounded Viewer geometry producer does not expose stacking authority, so the Viewer adapter emits `stacking_fidelity=unknown` and `null` node orders. Sorting unknown-order nodes by `node_id` is only deterministic wire normalization; it is not a paint-order claim.

## Snapshot identity

`snapshot_id` is derived, not chosen by the browser.

To compute it:

1. normalize the snapshot;
2. remove `snapshot_id`;
3. serialize UTF-8 JSON with lexical object keys, no insignificant whitespace, and no ASCII escaping;
4. compute SHA-256 over those exact bytes;
5. encode as `sha256:<lowercase hex>`.

The browser may verify this identity. Only the server associates a snapshot with an authoritative revision.

Changing authoring revision or Layout Environment must therefore produce a different normalized input unless the producer can prove the complete normalized snapshot is byte-identical.

## Story/text boundary

V1 transports canonical `story_id`, Unicode Story text and explicit Story-frame ordering.

`text_fidelity` is explicit per Story:
- `exact`;
- `partial`;
- `unsupported`;
- `opaque`.

V1 intentionally does not invent an independent browser rich-text authoring model. Rich styling/glyph-level projection must be added only through a later versioned allowlist when grounded by the server projection.

## Paint boundary

A node may reference a bounded `paint_id`. V1 paint descriptors contain only source-neutral RGBA fill and stroke/width state.

Missing/inherited/default paint that cannot yet be represented exactly must remain visible through capability/fidelity/diagnostic state rather than silently becoming a browser default.

## Resource boundary

Scene JSON never embeds arbitrary source bytes.

Resources use:
- canonical `resource_id`;
- safe type/MIME metadata;
- optional content hash/length when exact bytes are known;
- typed availability;
- an opaque `fetch_handle` when browser delivery is permitted.

`fetch_handle` is an opaque token, not a URL/path. Product routing resolves it through an authenticated resource service.

Forbidden in the scene protocol:
- filesystem/source paths;
- CFB stream names;
- parser carrier refs;
- byte-range provenance;
- raw PUB bytes;
- embedded arbitrary resource byte arrays.

## Typed fidelity and capability state

Absence of a field never means "unsupported".

Global `fidelity.state` is:
- `supported`;
- `partial`;
- `unsupported`.

Specific machine-readable reasons live in `fidelity.reasons` and diagnostics.

Capabilities are typed entries with `key + state + note`, where state is `supported | partial | unsupported`. This lets the browser explain exactly which interaction/rendering classes are available without inferring support from missing JSON.

## Delta semantics

`chaptera.scene.delta.v1` is a reserved optional optimization over the full snapshot contract.

A producer may defer delta production and always send full snapshots without violating V1.

If used, a delta is valid only for its exact document/base revision/base snapshot/Layout Environment fence. Mismatch means **do not apply**; request/receive a full snapshot.

Delta transport does not contain semantic edit commands, collaboration ordering, presence, or mutable renderer slots.

## Protocol/private-source boundary

The protocol package must remain buildable/readable without access to the closed canonical Rust implementation.

That does not authorize re-defining document semantics in the browser. The server adapter is responsible for translating canonical model/layout values into this contract via an explicit field allowlist.

## Golden fixtures and CI

Canonical synthetic golden fixtures cover:
- simple Story text + text frame;
- exact image resource with opaque fetch handle;
- group/table-bearing hierarchy + bounded paint;
- explicit unsupported node/diagnostic with negative off-page EMU.

The four standalone fixture files are the only synthetic fixture source of truth.

CI validates both JSON Schemas, canonical ordering, snapshot hash derivation, ID/reference integrity, opaque resource handles, absence of parser/private-source keys and fixture coverage.

The generated synthetic receipts are explicitly tagged `real_pub_measurement = false`. CI also exercises a dependency-free `ViewerGeometryDocument` JSON → Scene V1 adapter on source-neutral synthetic producer fixtures; those fixtures do not substitute for a real-PUB payload measurement.

## Security

Protocol producers use an allowlist. Adding a field to an internal parser/layout structure must never automatically expose it to the browser.

The protocol has no field for raw carrier data. A compatibility extension requires a versioned schema change.

## Outstanding acceptance receipt

Synthetic fixtures verify wire invariants only.

`WEB-SCENE-PROTOCOL-01` is **not Done** until a server-side producer over representative real supported PUBs:
1. maps canonical Viewer/Resolved Scene state into this V1 allowlist;
2. records actual normalized snapshot payload sizes;
3. proves no parser-private carrier/path/raw-byte state leaks;
4. demonstrates downstream renderer/interaction consumers can deserialize the V1 snapshot without importing parser/layout internals as their public API.

That producer-side real-PUB receipt is the remaining closure condition after protocol CI is green.
