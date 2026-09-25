import fs from "node:fs";
import { RenderFallbackControllerV1 } from "../apps/web/render-fallback-v1.mjs";

const candidates = [
  { backend_id: "canvas", renderer_kind: "canvas2d", preference_rank: 10 },
  { backend_id: "svg", renderer_kind: "svg", preference_rank: 20 }
];
const probes = {
  canvas: { available: true, surface_compatible: true, capabilities: ["rects", "text", "images"] },
  svg: { available: true, surface_compatible: true, capabilities: ["rects", "text", "images"] }
};
const semantic_state = {
  document_id: "receipt:doc",
  revision_id: "receipt:rev",
  view_state: { zoom: 1 },
  selection: { node_id: "node:1" },
  focus: { owner: "canvas" },
  resource_readiness: { image: "ready" },
  pending_canonical_operation: null
};
const c = new RenderFallbackControllerV1({
  candidates,
  probes,
  requirements: { mandatory_capabilities: ["rects", "text", "images"] },
  semantic_state,
  policy: { same_backend_retry_limit: 1, circuit_failure_threshold: 2, cooldown_ms: 1000 }
});
c.initialSelect(0);
c.publishCoherentFrame({ backend_id: "canvas", generation: 1, coherent: true });
c.reportLoss({ backend_id: "canvas", now_ms: 100 });
c.publishCoherentFrame({ backend_id: "canvas", generation: 2, coherent: true });
c.reportLoss({ backend_id: "canvas", now_ms: 200 });
c.publishCoherentFrame({ backend_id: "svg", generation: 3, coherent: true });
const out = {
  ...c.receipt(),
  stale_generation_fenced: !c.completionIsCurrent(1),
  focus_selection_preserved: c.invariantSnapshot().selection.node_id === "node:1",
  document_truth_preserved: c.invariantSnapshot().revision_id === "receipt:rev"
};
fs.mkdirSync("target/web-render-fallback", { recursive: true });
fs.writeFileSync("target/web-render-fallback/receipt.json", JSON.stringify(out, null, 2));
console.log(JSON.stringify(out, null, 2));
