# PGUIDES-WRITE-CAUSAL-01

Bounded native experiment seam for `PUB-T-631 / U-GEO-05`.

## Exact question

For a real Publisher shape that already persists `pGuides` + `pAdjustHandles` and exposes `Shape.Adjustments.Count > 0`, what does a native `Adjustments(1)` edit actually rewrite across Save + fresh reopen?

This is producer/write causality only. Shared OfficeArt grammar/read preservation is already owned by the existing `pub-cfb` / `pub-escher` stack and prior T380/T405 evidence.

## Canonical fixture

The operation accepts exactly one local fixture identity under `PUB_RESEARCH_FIXTURE_ROOT`:

- name: `REG-TST2-pub97-to-pub2007-with-pub2007.pub`
- public provenance: `fosnola/libmspub-test/testset/12.0`
- size: 272,896 bytes
- SHA-256: `28481deeac014133b960e09376bfd31c2ec14bfb2a0679cb3c010855a9eeec29`

The file is not downloaded at runtime and is not copied into this repository. The native runner searches only for that exact basename and admits exactly one SHA match.

Prior evidence already establishes 11 dynamic shapes in this fixture carrying `pGuides / 0xC156` and `pAdjustHandles / 0xC155`, plus connection sites and materialized geometry arrays.

## Preflight

Publisher opens the exact fixture read-only, recursively scans page/group shape collections, and records all shapes with `Adjustments.Count > 0`.

The first deterministic candidate in page/path order is selected. Its collection path is reused in every fresh arm; the packet does not add a Tag or otherwise mutate identity merely to track the object.

Adjustment A/B are not hardcoded. An unsaved preflight probes candidate values, restores the original value after every attempt, and accepts the first two distinct COM readbacks that can be restored exactly enough. No preflight document is saved.

## Causal arms

Every arm starts from a fresh exact fixture copy:

- `C0` — no adjustment mutation, SaveAs control.
- `A` — set only `Adjustments(1)=A`, then SaveAs.
- `B` — set only `Adjustments(1)=B`, then SaveAs.
- `R` — set A, restore the original value in the same session, then SaveAs.

Each successful Save is followed by process close and fresh Publisher reopen.

## COM/native observations

For the selected shape, every relevant phase records:

- Name / Shape.ID / Shape.Type / AutoShapeType;
- collection path;
- Left / Top / Width / Height / Rotation;
- Nodes.Count where the COM surface exposes it;
- full `Adjustments.Count` and all readable adjustment values;
- requested value and COM readback;
- output PUB size/SHA.

No new PDF export is required in this task. `OBS-FOPT-001 / ADJUST-REAL-01` already proves, on this exact fixture family and Publisher 2019/build12527, that changing `adjustValue` changes native rendered geometry. T631 asks what Publisher itself persists when the user/API changes that slider.

## Raw projection probe

`tools/research-runner/pguides-probe` is deliberately task-specific and reuses:

- `pub-cfb::read_stream_path`;
- `pub-escher::inspect_sp_containers`.

It does **not** implement another CFB or OfficeArt parser.

For source/C0/A/B/R it emits only the FOPT state relevant to this discriminator:

- `pVertices 0x0145`;
- `pSegmentInfo 0x0146`;
- `adjustValue1..8 0x0147..0x014E`;
- `pConnectionSites 0x0151`;
- `pConnectionSitesDir 0x0152`;
- `pAdjustHandles 0x0155`;
- `pGuides 0x0156`;
- `pInscribe 0x0157`.

Each complex value is represented by exact source span, length and SHA-256; scalar `op` is preserved as u32/i32.

If `cargo` is available on the Publisher runner, the raw probe runs in the same packet. If not, COM/native execution remains valid and `raw-probe-status.json` explicitly records `deferred_no_cargo`; the private PUB outputs remain the source for a Rust-capable local follow-up.

## Decision rules

- only adjust scalar changes; guides/handles and materialized arrays remain stable → adjustment is bounded as evaluator input with stable formula/handle definition state;
- `pGuides` changes → inspect the exact changed SG payload before naming formula semantics;
- `pAdjustHandles` changes without guide changes → handle constraints are mutable persisted state in this fixture;
- vertices/segments change while symbolic definition stays stable → preserve symbolic and materialized projections separately;
- restore arm differs from C0 after returning the COM value to original → Publisher retains additional history/normalization state; do not collapse to a scalar-only writer law.

All conclusions remain bounded to this exact fixture family and Publisher 16.0/build12527.
