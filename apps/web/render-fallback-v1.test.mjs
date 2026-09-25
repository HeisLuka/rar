import test from "node:test";
import assert from "node:assert/strict";

import {
  RenderFallbackControllerV1,
  selectBackendV1
} from "./render-fallback-v1.mjs";

const CANDIDATES = [
  { backend_id: "preferred-canvas", renderer_kind: "canvas2d", preference_rank: 10 },
  { backend_id: "fallback-svg", renderer_kind: "svg", preference_rank: 20 },
  { backend_id: "hybrid", renderer_kind: "webgl2-hybrid", preference_rank: 30 }
];

const REQUIREMENTS = {
  mandatory_capabilities: ["rects", "text", "images"]
};

const SEMANTIC_STATE = {
  document_id: "doc:1",
  revision_id: "rev:9",
  shard_id: "shard:0",
  view_state: { zoom: 1.25, pan_x: 17, pan_y: -3 },
  selection: { node_id: "node:7" },
  focus: { owner: "story", story_id: "story:2", ime_active: true },
  accessibility_order: ["node:1", "node:7"],
  resource_readiness: {
    "image:a": { state: "ready", resource_id: "image:a" }
  },
  pending_canonical_operation: { client_operation_id: "op:pending:1" }
};

function probes(overrides = {}) {
  return {
    "preferred-canvas": {
      available: true,
      surface_compatible: true,
      capabilities: ["rects", "text", "images"]
    },
    "fallback-svg": {
      available: true,
      surface_compatible: true,
      capabilities: ["rects", "text", "images"]
    },
    "hybrid": {
      available: true,
      surface_compatible: true,
      capabilities: ["rects", "text", "images"]
    },
    ...overrides
  };
}

function controller(options = {}) {
  return new RenderFallbackControllerV1({
    candidates: options.candidates ?? CANDIDATES,
    probes: options.probes ?? probes(),
    requirements: options.requirements ?? REQUIREMENTS,
    semantic_state: options.semantic_state ?? SEMANTIC_STATE,
    policy: options.policy ?? {
      same_backend_retry_limit: 1,
      circuit_failure_threshold: 2,
      cooldown_ms: 1000
    },
    now_ms: 0
  });
}

test("configured preference selects deterministically without universal backend order", () => {
  const reorderedKinds = [
    { backend_id: "svg-first", renderer_kind: "svg", preference_rank: 0 },
    { backend_id: "canvas-second", renderer_kind: "canvas2d", preference_rank: 1 }
  ];
  const result = selectBackendV1({
    candidates: reorderedKinds,
    probes: {
      "svg-first": { available: true, surface_compatible: true, capabilities: ["rects", "text", "images"] },
      "canvas-second": { available: true, surface_compatible: true, capabilities: ["rects", "text", "images"] }
    },
    requirements: REQUIREMENTS,
    now_ms: 0
  });
  assert.equal(result.selected_backend_id, "svg-first");
  assert.equal(result.renderer_kind, "svg");
});

test("unavailable preferred backend falls to next compatible with bounded reason", () => {
  const result = selectBackendV1({
    candidates: CANDIDATES,
    probes: probes({
      "preferred-canvas": {
        available: false,
        reason_code: "api_unavailable",
        capabilities: []
      }
    }),
    requirements: REQUIREMENTS,
    now_ms: 0
  });
  assert.equal(result.selected_backend_id, "fallback-svg");
  assert.equal(result.rejected[0].reason_code, "api_unavailable");
});

test("available but incompatible backend is rejected", () => {
  const result = selectBackendV1({
    candidates: CANDIDATES,
    probes: probes({
      "preferred-canvas": {
        available: true,
        surface_compatible: true,
        capabilities: ["rects", "text"]
      }
    }),
    requirements: REQUIREMENTS,
    now_ms: 0
  });
  assert.equal(result.selected_backend_id, "fallback-svg");
  assert.equal(result.rejected[0].reason_code, "workload_capability_missing");
  assert.deepEqual(result.rejected[0].missing_capabilities, ["images"]);
});

test("backend becomes active only after coherent eligible frame", () => {
  const c = controller();
  const selection = c.initialSelect(0);
  assert.equal(selection.selected_backend_id, "preferred-canvas");
  assert.equal(c.state, "initializing");
  assert.equal(c.publishCoherentFrame({
    backend_id: "preferred-canvas",
    generation: 1,
    coherent: false
  }).published, false);
  assert.equal(c.state, "initializing");
  const published = c.publishCoherentFrame({
    backend_id: "preferred-canvas",
    generation: 1,
    coherent: true
  });
  assert.equal(published.published, true);
  assert.equal(c.state, "active");
  assert.equal(c.generation, 1);
});

test("one recoverable loss recreates same backend generation without canonical mutation", () => {
  const c = controller();
  c.initialSelect(0);
  c.publishCoherentFrame({ backend_id: "preferred-canvas", generation: 1, coherent: true });
  const before = c.invariantSnapshot();
  const action = c.reportLoss({ backend_id: "preferred-canvas", now_ms: 100 });
  assert.equal(action.action, "recover_same_backend");
  assert.equal(action.generation, 2);
  assert.equal(action.semantic_state_preserved, true);
  c.publishCoherentFrame({ backend_id: "preferred-canvas", generation: 2, coherent: true });
  assert.deepEqual(c.invariantSnapshot(), before);
  const receipt = c.receipt();
  assert.equal(receipt.canonical_operations_emitted, 0);
  assert.equal(receipt.authoring_revisions_emitted, 0);
  assert.equal(receipt.layout_revisions_emitted, 0);
});

test("repeated loss opens circuit and switches to next compatible backend", () => {
  const c = controller();
  c.initialSelect(0);
  c.publishCoherentFrame({ backend_id: "preferred-canvas", generation: 1, coherent: true });
  c.reportLoss({ backend_id: "preferred-canvas", now_ms: 100 });
  c.publishCoherentFrame({ backend_id: "preferred-canvas", generation: 2, coherent: true });
  const action = c.reportLoss({ backend_id: "preferred-canvas", now_ms: 200 });
  assert.equal(action.action, "switch_backend");
  assert.equal(action.backend_id, "fallback-svg");
  assert.equal(action.generation, 3);
  assert.equal(c.circuits["preferred-canvas"].open_until_ms, 1200);
  c.publishCoherentFrame({ backend_id: "fallback-svg", generation: 3, coherent: true });
  assert.equal(c.activeBackendId, "fallback-svg");
  assert.equal(c.switchCount, 1);
});

test("stale old generation frame/resource completion cannot publish after switch", () => {
  const c = controller();
  c.initialSelect(0);
  c.publishCoherentFrame({ backend_id: "preferred-canvas", generation: 1, coherent: true });
  c.reportLoss({ backend_id: "preferred-canvas", now_ms: 100 });
  c.publishCoherentFrame({ backend_id: "preferred-canvas", generation: 2, coherent: true });
  c.reportLoss({ backend_id: "preferred-canvas", now_ms: 200 });
  assert.equal(c.completionIsCurrent(1), false);
  assert.equal(c.publishCoherentFrame({
    backend_id: "preferred-canvas",
    generation: 2,
    coherent: true
  }).published, false);
  assert.equal(c.publishCoherentFrame({
    backend_id: "fallback-svg",
    generation: 3,
    coherent: true
  }).published, true);
});

test("selection focus view resources and pending operation survive fallback", () => {
  const c = controller();
  const before = c.invariantSnapshot();
  c.initialSelect(0);
  c.publishCoherentFrame({ backend_id: "preferred-canvas", generation: 1, coherent: true });
  c.reportLoss({ backend_id: "preferred-canvas", now_ms: 100 });
  c.publishCoherentFrame({ backend_id: "preferred-canvas", generation: 2, coherent: true });
  c.reportLoss({ backend_id: "preferred-canvas", now_ms: 200 });
  c.publishCoherentFrame({ backend_id: "fallback-svg", generation: 3, coherent: true });
  assert.deepEqual(c.invariantSnapshot(), before);
  assert.equal(c.invariantSnapshot().selection.node_id, "node:7");
  assert.equal(c.invariantSnapshot().focus.story_id, "story:2");
  assert.equal(c.invariantSnapshot().view_state.zoom, 1.25);
  assert.equal(c.invariantSnapshot().resource_readiness["image:a"].state, "ready");
});

test("no compatible backend keeps semantic session truth alive", () => {
  const none = {
    "preferred-canvas": { available: false, reason_code: "api_unavailable", capabilities: [] },
    "fallback-svg": { available: true, surface_compatible: true, capabilities: ["rects"] },
    "hybrid": { available: false, reason_code: "initialization_failed", capabilities: [] }
  };
  const c = controller({ probes: none });
  const before = c.invariantSnapshot();
  const selection = c.initialSelect(0);
  assert.equal(selection.selected_backend_id, null);
  assert.equal(c.state, "no_compatible_backend");
  assert.deepEqual(c.invariantSnapshot(), before);
  assert.equal(c.receipt().canonical_operations_emitted, 0);
});

test("anti-flap prevents immediate re-promotion while circuit is open", () => {
  const c = controller();
  c.initialSelect(0);
  c.publishCoherentFrame({ backend_id: "preferred-canvas", generation: 1, coherent: true });
  c.reportLoss({ backend_id: "preferred-canvas", now_ms: 100 });
  c.publishCoherentFrame({ backend_id: "preferred-canvas", generation: 2, coherent: true });
  c.reportLoss({ backend_id: "preferred-canvas", now_ms: 200 });
  assert.equal(c.canRepromote("preferred-canvas", 201), false);
  assert.equal(c.canRepromote("preferred-canvas", 1200), true);
  const selection = selectBackendV1({
    candidates: CANDIDATES,
    probes: probes(),
    requirements: REQUIREMENTS,
    circuits: c.circuits,
    now_ms: 201
  });
  assert.equal(selection.selected_backend_id, "fallback-svg");
});

test("initialization failure fences failed backend and selects another candidate", () => {
  const c = controller();
  c.initialSelect(0);
  const action = c.reportInitializationFailure({
    backend_id: "preferred-canvas",
    now_ms: 10
  });
  assert.equal(action.action, "switch_backend");
  assert.equal(action.backend_id, "fallback-svg");
  assert.equal(action.generation, 1);
  assert.equal(c.state, "switching_backend");
});
