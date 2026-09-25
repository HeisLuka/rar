import test from "node:test";
import assert from "node:assert/strict";
import {
  WorkerBoundaryV1,
  transferableArrayBuffer,
  workerDerivedKindsV1,
} from "./worker-boundary-v1.mjs";

const g = (n) => ({ scene: n, view: n, resource: n, surface: n, overlay: n });

test("queue and live bytes are bounded instead of growing under storms", () => {
  const b = new WorkerBoundaryV1({ maxQueued: 2, maxInflight: 1, maxBytes: 100 });
  b.setCurrentGenerations(g(1));
  assert.ok(b.submit({ kind: "spatial_index", bytes: 40 }));
  assert.ok(b.submit({ kind: "segment_plan", bytes: 40 }));
  assert.equal(b.submit({ kind: "material_prepare", bytes: 1 }), null);
  assert.equal(b.snapshot().dropped_backpressure, 1);
  assert.equal(b.snapshot().queued, 2);
  assert.equal(b.snapshot().live_bytes, 80);
});

test("newer generation supersedes queued derived work before dispatch", () => {
  const b = new WorkerBoundaryV1();
  b.setCurrentGenerations(g(1));
  b.submit({ kind: "scene_parse", bytes: 10 });
  b.submit({ kind: "segment_plan", bytes: 10 });
  b.setCurrentGenerations(g(2));
  assert.equal(b.snapshot().queued, 0);
  assert.equal(b.snapshot().superseded_queued, 2);
});

test("stale in-flight result cannot publish and graphics resources are closed", () => {
  const b = new WorkerBoundaryV1({ maxInflight: 1 });
  b.setCurrentGenerations(g(1));
  const submitted = b.submit({ kind: "image_prepare", bytes: 10, transport: "transfer" });
  const job = b.dispatchNext();
  assert.equal(job.id, submitted.id);
  b.setCurrentGenerations(g(2));
  let closeCount = 0;
  const result = { bitmap: { close() { closeCount += 1; } } };
  const completion = b.complete(job.id, { generations: g(2), result });
  assert.equal(completion.accepted, false);
  assert.equal(completion.reason, "stale_generation");
  assert.equal(closeCount, 1);
  assert.equal(b.snapshot().released_stale_graphics, 1);
});

test("current result is accepted without closing its graphics resource", () => {
  const b = new WorkerBoundaryV1();
  b.setCurrentGenerations(g(3));
  const submitted = b.submit({ kind: "image_prepare", bytes: 10 });
  b.dispatchNext();
  let closed = false;
  const completion = b.complete(submitted.id, { generations: g(3), result: { close() { closed = true; } } });
  assert.equal(completion.accepted, true);
  assert.equal(closed, false);
});

test("worker restart invalidates disposable state and rebuilds from latest browser inputs", () => {
  const b = new WorkerBoundaryV1();
  b.setCurrentGenerations(g(4));
  b.submit({ kind: "scene_parse", bytes: 10 });
  b.dispatchNext();
  b.submit({ kind: "spatial_index", bytes: 10 });
  const receipt = b.restart({ generations: g(5) });
  assert.equal(receipt.worker_generation, 2);
  assert.deepEqual(receipt.rebuild_from_generations, g(5));
  assert.equal(receipt.canonical_authority, "server_editor_session");
  assert.equal(b.snapshot().queued, 0);
  assert.equal(b.snapshot().inflight, 0);
  assert.equal(b.snapshot().restart_invalidated, 2);
});

test("transfer and clone accounting remain separate", () => {
  const b = new WorkerBoundaryV1({ maxQueued: 4, maxInflight: 4 });
  b.setCurrentGenerations(g(1));
  b.submit({ kind: "scene_parse", bytes: 100, transport: "transfer" });
  b.submit({ kind: "segment_plan", bytes: 20, transport: "clone" });
  b.dispatchNext();
  b.dispatchNext();
  assert.equal(b.snapshot().bytes_transferred, 100);
  assert.equal(b.snapshot().bytes_cloned, 20);
});

test("SharedArrayBuffer and canonical mutation workloads are not baseline-admitted", () => {
  const b = new WorkerBoundaryV1();
  b.setCurrentGenerations(g(1));
  assert.throws(
    () => b.submit({ kind: "scene_parse", bytes: 1, sharedArrayBuffer: true }),
    /not a V1 baseline/,
  );
  assert.throws(
    () => b.submit({ kind: "replace_story_text", bytes: 1 }),
    /unsupported Worker derived workload kind/,
  );
  assert.equal(workerDerivedKindsV1().includes("image_prepare"), true);
  assert.equal(b.snapshot().raw_pub_allowed, false);
  assert.equal(b.snapshot().worker_authority, "browser_derived_disposable_only");
});

test("transfer helper exposes exact ownership-transfer list", () => {
  const value = transferableArrayBuffer(64);
  assert.equal(value.payload.byteLength, 64);
  assert.equal(value.transfer[0], value.payload);
});
