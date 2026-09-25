import test from "node:test";
import assert from "node:assert/strict";
import {
  createFramePacer,
  normalizeFrameGenerations,
  sameFrameGenerations,
} from "./frame-pacing-v1.mjs";

function fakeClock() {
  let next = 1;
  const queue = new Map();
  return {
    request(callback) {
      const id = next++;
      queue.set(id, callback);
      return id;
    },
    cancel(id) {
      queue.delete(id);
    },
    async tick(timestamp = 16) {
      const batch = [...queue.entries()];
      queue.clear();
      for (const [, callback] of batch) await callback(timestamp);
    },
    pending() {
      return queue.size;
    },
  };
}

const state = (n, payload = { n }) => ({
  generations: {
    scene: n,
    view: n,
    overlay: n,
    resource: n,
    surface: n,
    renderer: n,
    worker: n,
  },
  payload,
});

test("100 pointer/view invalidations coalesce into one callback with latest state", async () => {
  const clock = fakeClock();
  const frames = [];
  const pacer = createFramePacer({
    requestFrame: clock.request,
    cancelFrame: clock.cancel,
    onFrame: async (frame) => frames.push(frame),
  });
  for (let i = 1; i <= 100; i += 1) pacer.invalidate(["view"], state(i));
  assert.equal(clock.pending(), 1);
  await clock.tick();
  assert.equal(frames.length, 1);
  assert.equal(frames[0].payload.n, 100);
  assert.equal(frames[0].generations.view, 100);
  assert.equal(pacer.stats().coalesced_requests, 99);
  assert.equal(pacer.stats().canonical_mutations_emitted, 0);
});

test("scene resource and overlay dirties merge into one coherent frame", async () => {
  const clock = fakeClock();
  const frames = [];
  const pacer = createFramePacer({ requestFrame: clock.request, onFrame: async (frame) => frames.push(frame) });
  pacer.invalidate(["scene"], state(1));
  pacer.invalidate(["resource", "overlay"], state(2));
  await clock.tick();
  assert.deepEqual(frames[0].dirty, ["overlay", "resource", "scene"]);
  assert.equal(frames[0].generations.scene, 2);
});

test("events during frame work create exactly one bounded follow-up", async () => {
  const clock = fakeClock();
  const frames = [];
  let pacer;
  pacer = createFramePacer({
    requestFrame: clock.request,
    onFrame: async (frame) => {
      frames.push(frame);
      if (frames.length === 1) {
        for (let i = 0; i < 50; i += 1) pacer.invalidate(["overlay"], state(2));
      }
    },
  });
  pacer.invalidate(["view"], state(1));
  await clock.tick();
  assert.equal(clock.pending(), 1);
  assert.equal(pacer.stats().followups_scheduled, 1);
  await clock.tick(32);
  assert.equal(frames.length, 2);
  assert.equal(clock.pending(), 0);
  assert.equal(frames[1].generations.overlay, 2);
});

test("hidden tab retains only latest state and resumes with one fresh frame", async () => {
  const clock = fakeClock();
  const frames = [];
  const pacer = createFramePacer({
    requestFrame: clock.request,
    cancelFrame: clock.cancel,
    onFrame: async (frame) => frames.push(frame),
  });
  pacer.setHidden(true);
  for (let i = 1; i <= 20; i += 1) pacer.invalidate(["scene"], state(i));
  assert.equal(clock.pending(), 0);
  assert.equal(pacer.stats().hidden_invalidations, 20);
  pacer.setHidden(false);
  assert.equal(clock.pending(), 1);
  await clock.tick();
  assert.equal(frames.length, 1);
  assert.equal(frames[0].payload.n, 20);
});

test("hiding a scheduled surface cancels obsolete callback then resumes latest", async () => {
  const clock = fakeClock();
  const frames = [];
  const pacer = createFramePacer({
    requestFrame: clock.request,
    cancelFrame: clock.cancel,
    onFrame: async (frame) => frames.push(frame),
  });
  pacer.invalidate(["view"], state(1));
  assert.equal(clock.pending(), 1);
  pacer.setHidden(true);
  assert.equal(clock.pending(), 0);
  pacer.invalidate(["view"], state(2));
  pacer.setHidden(false);
  assert.equal(clock.pending(), 1);
  await clock.tick();
  assert.equal(frames[0].payload.n, 2);
  assert.equal(pacer.stats().hidden_suppressed_callbacks, 1);
});

test("async completion fence includes worker and surface generations", () => {
  const pacer = createFramePacer({ requestFrame: () => 1, onFrame: async () => {} });
  const current = state(5).generations;
  assert.equal(pacer.acceptAsyncCompletion({ generations: current }, current), true);
  assert.equal(
    pacer.acceptAsyncCompletion(
      { generations: { ...current, worker: 4 } },
      current,
    ),
    false,
  );
  assert.equal(
    pacer.acceptAsyncCompletion(
      { generations: { ...current, surface: 4 } },
      current,
    ),
    false,
  );
  assert.equal(pacer.stats().stale_completions, 2);
});

test("resize/DPR surface generation makes old preparation stale", () => {
  const oldSurface = state(9).generations;
  const newSurface = { ...oldSurface, surface: 10 };
  assert.equal(sameFrameGenerations(oldSurface, newSurface), false);
});

test("generation tuples are bounded non-negative integers", () => {
  assert.deepEqual(normalizeFrameGenerations({ scene: 1 }), {
    scene: 1, view: 0, overlay: 0, resource: 0, surface: 0, renderer: 0, worker: 0,
  });
  assert.throws(() => normalizeFrameGenerations({ worker: -1 }), /non-negative integer/);
});
