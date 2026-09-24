# RENDER-SCENE-IR-01 — RenderScene v1

RenderScene v1 is a disposable source-neutral compiled renderer representation. It is reproducible from resolved source-neutral scene inputs and must not become authoring/layout truth.

V1 keeps deterministic page order, exact signed EMU bounds, transform/paint/resource tables, stable NodeId-to-render-atom indirection, explicit paint sequence and shaped-glyph provenance.

Frame NodeId and image ResourceId remain separate identities. Glyph runs carry StoryId, frame NodeId and scalar range. Unsupported primitive kinds remain explicit diagnostics.

The representation intentionally excludes GPU buffer offsets, backend handles, atlas slots and parser/PUB types.
