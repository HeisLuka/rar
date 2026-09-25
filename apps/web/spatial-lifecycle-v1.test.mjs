import test from "node:test";
import assert from "node:assert/strict";
import { hitTestSnapshot } from "./interaction-v1.mjs";
import {
  SpatialWindowLifecycleV2,
  spatialGenerationKey,
} from "./spatial-lifecycle-v1.mjs";

const identity = (n, pages = ["p1", "p2", "p3"], total = 500) => ({
  document_id: "doc:1",
  revision_id: "rev:" + n,
  layout_environment_id: "env:1",
  window_id: "window:center",
  window_generation: n,
  page_ids: pages,
  shard_fingerprints: pages.map((page) => page + ":sha:" + n),
  total_document_pages: total,
});

const rows = () => [
  { node_id: "off", page_id: "p1", bounds: { x: -100, y: 0, width: 100, height: 100 }, paint_order: 0, z_order: 0 },
  { node_id: "back", page_id: "p1", bounds: { x: 0, y: 0, width: 120, height: 120 }, paint_order: 1, z_order: 1 },
  { node_id: "front", page_id: "p1", bounds: { x: 20, y: 20, width: 80, height: 80 }, paint_order: 2, z_order: 2 },
  { node_id: "p2n", page_id: "p2", bounds: { x: 0, y: 0, width: 100, height: 100 }, paint_order: 0, z_order: 0 },
  { node_id: "p3n", page_id: "p3", bounds: { x: 0, y: 0, width: 100, height: 100 }, paint_order: 0, z_order: 0 },
];

test("point candidates are current-generation only and existing hit-test still chooses winner", () => {
  const index = new SpatialWindowLifecycleV2({ cellSizeEmu: 100 });
  index.replaceWindow(identity(1), rows());
  const g1 = spatialGenerationKey(identity(1));
  const point = { x_emu: 50, y_emu: 50 };
  const result = index.hitTestCandidates("p1", point, g1);
  assert.deepEqual(result.candidates.map((row) => row.node_id), ["back", "front"]);

  const fullSnapshot = { nodes: rows(), stacking_fidelity: "exact" };
  const candidateSnapshot = {
    nodes: result.candidates.map((row) => ({ ...row, bounds: { ...row.bounds } })),
    stacking_fidelity: "exact",
  };
  assert.deepEqual(
    hitTestSnapshot(candidateSnapshot, "p1", point),
    hitTestSnapshot(fullSnapshot, "p1", point),
  );

  index.replaceWindow(identity(2), rows());
  assert.throws(() => index.queryPoint("p1", point, g1), /stale spatial generation/);
});

test("marquee candidates match trusted full-scan oracle", () => {
  const index = new SpatialWindowLifecycleV2({ cellSizeEmu: 50 });
  index.replaceWindow(identity(1), rows());
  const generation = spatialGenerationKey(identity(1));
  const query = { x: -50, y: 0, width: 120, height: 80 };
  const got = index.marqueeCandidates("p1", query, generation).candidates.map((row) => row.node_id);
  const oracle = rows()
    .filter((row) => row.page_id === "p1")
    .filter((row) => !(
      row.bounds.x + row.bounds.width <= query.x
      || query.x + query.width <= row.bounds.x
      || row.bounds.y + row.bounds.height <= query.y
      || query.y + query.height <= row.bounds.y
    ))
    .map((row) => row.node_id)
    .sort();
  assert.deepEqual(got, oracle);
});

test("snap geometry and culling share generation without taking snap policy authority", () => {
  const index = new SpatialWindowLifecycleV2();
  index.replaceWindow(identity(1), rows());
  const generation = spatialGenerationKey(identity(1));
  const query = { x: -200, y: -50, width: 500, height: 500 };
  const snap = index.snapGeometryCandidates("p1", query, generation);
  const cull = index.cullCandidates("p1", query, generation);
  assert.equal(snap.generation_key, generation);
  assert.equal(cull.generation_key, generation);
  assert.deepEqual(
    snap.candidates.map((row) => row.node_id),
    cull.candidates.map((row) => row.node_id),
  );
  assert.equal(index.receipt().authority.snap_policy_authority, false);
});

test("one-node patch rebuild matches clean current-window rebuild", () => {
  const index = new SpatialWindowLifecycleV2({ cellSizeEmu: 100 });
  index.replaceWindow(identity(1), rows());
  const g1 = spatialGenerationKey(identity(1));
  const moved = {
    node_id: "front",
    page_id: "p1",
    bounds: { x: 500, y: 0, width: 80, height: 80 },
    paint_order: 2,
    z_order: 2,
  };
  index.applyNodePatch({
    base_generation_key: g1,
    next_identity: identity(2),
    upsert_nodes: [moved],
  });

  const clean = new SpatialWindowLifecycleV2({ cellSizeEmu: 100 });
  clean.replaceWindow(identity(2), rows().map((row) => row.node_id === "front" ? moved : row));
  const g2 = spatialGenerationKey(identity(2));
  const query = { x: 450, y: -10, width: 200, height: 200 };
  assert.deepEqual(
    index.queryBox("p1", query, g2).candidates,
    clean.queryBox("p1", query, g2).candidates,
  );
});

test("off-page geometry remains queryable", () => {
  const index = new SpatialWindowLifecycleV2();
  index.replaceWindow(identity(1), rows());
  const generation = spatialGenerationKey(identity(1));
  assert.deepEqual(
    index.queryPoint("p1", { x_emu: -50, y_emu: 50 }, generation).candidates.map((row) => row.node_id),
    ["off"],
  );
});

test("unsupported, transformed-unknown, invalid and out-of-window bounds fail closed", () => {
  const index = new SpatialWindowLifecycleV2();
  const bad = [
    ...rows(),
    { node_id: "bad1", page_id: "p1", bounds: { x: 0, y: 0, width: 10, height: 10 }, bounds_fidelity: "unsupported" },
    { node_id: "bad2", page_id: "p1", bounds: { x: 0, y: 0, width: 10, height: 10 }, transformed_bounds_fidelity: "unknown" },
    { node_id: "bad3", page_id: "p1", bounds: { x: 0, y: 0, width: -1, height: 10 } },
    { node_id: "p9n", page_id: "p9", bounds: { x: 0, y: 0, width: 10, height: 10 } },
  ];
  const receipt = index.replaceWindow(identity(1), bad);
  assert.equal(receipt.indexed_nodes, rows().length);
  assert.equal(receipt.metrics.skipped_invalid, 4);
});

test("page eviction and revisit reconstruct page-local index from fresh shard geometry", () => {
  const index = new SpatialWindowLifecycleV2();
  index.replaceWindow(identity(1), rows());
  let generation = spatialGenerationKey(identity(1));
  index.evictPage({
    base_generation_key: generation,
    next_identity: identity(2, ["p1", "p3"]),
    page_id: "p2",
  });
  generation = spatialGenerationKey(identity(2, ["p1", "p3"]));
  assert.equal(index.queryBox("p2", { x: 0, y: 0, width: 100, height: 100 }, generation).candidates.length, 0);

  index.revisitPage({
    base_generation_key: generation,
    next_identity: identity(3),
    page_id: "p2",
    nodes: [rows().find((row) => row.node_id === "p2n")],
  });
  generation = spatialGenerationKey(identity(3));
  assert.deepEqual(
    index.queryPoint("p2", { x_emu: 10, y_emu: 10 }, generation).candidates.map((row) => row.node_id),
    ["p2n"],
  );
});

test("selection reconciliation clears only missing current-window identities", () => {
  const index = new SpatialWindowLifecycleV2();
  index.replaceWindow(identity(1), rows());
  const generation = spatialGenerationKey(identity(1));
  assert.deepEqual(index.reconcileSelection(["front", "missing"], generation), {
    generation_key: generation,
    present: ["front"],
    removed: ["missing"],
  });
});

test("total document page count does not affect equal current-window operation cost", () => {
  const a = new SpatialWindowLifecycleV2({ cellSizeEmu: 100 });
  const b = new SpatialWindowLifecycleV2({ cellSizeEmu: 100 });
  a.replaceWindow(identity(1, ["p1", "p2", "p3"], 100), rows());
  b.replaceWindow(identity(1, ["p1", "p2", "p3"], 500), rows());
  const generation = spatialGenerationKey(identity(1));
  const query = { x: -100, y: 0, width: 300, height: 200 };
  assert.deepEqual(a.queryBox("p1", query, generation).stats, b.queryBox("p1", query, generation).stats);
});

test("lifecycle is disposable runtime state and emits no revision or authoring mutation", () => {
  const index = new SpatialWindowLifecycleV2();
  const receipt = index.replaceWindow(identity(1), rows());
  assert.equal(receipt.authority.canonical_document_state, false);
  assert.equal(receipt.authority.authoring_mutations, 0);
  assert.equal(receipt.authority.revisions_emitted, 0);
  assert.equal(receipt.mechanism.semantic_contract, false);
});
