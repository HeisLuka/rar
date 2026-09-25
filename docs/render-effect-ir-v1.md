# RENDER-EFFECT-IR-01 — source-neutral RenderEffect V1

RenderEffect V1 extends the disposable RenderScene contract with deterministic resolved effect tables. It does not decode Publisher or OfficeArt properties.

Admitted V1 semantic records are masks (alpha/luminance coverage), bounded blur, simple resolved shadow, and a small source-neutral filter chain. Effect groups carry explicit opacity, isolation, blend and composite modes. Clips remain a separate geometry table: a mask is never serialized as a clip.

Every effect carries a fidelity state. A genuine Publisher-origin effect may be marked exact only when an upstream resolved-effect producer receipt is supplied; otherwise partial/unsupported semantics remain diagnostics. Backend surfaces, texture ids, shader/pass handles and raw OfficeArt/PUB parser fields are rejected from the IR.

Effect regions use signed EMU coordinates and remain explicit because blur/shadow/filter pixels may extend outside source bounds. RenderScene atoms may reference an effect group, but paint order and atom identity do not change merely because an effect is attached.

ScenePatch carries bounded id-keyed effect-table deltas. Parameter-only changes update the effect table without recompiling unrelated node atoms. Attach/remove changes only the affected node reference plus the relevant effect/group rows. Pure and in-place patch application remain equal to a clean full compile.
