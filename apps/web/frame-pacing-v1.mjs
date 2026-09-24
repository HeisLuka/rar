export const DIRTY_CLASSES = Object.freeze(["view","overlay","resource","scene","surface","fidelity"]);

function assertGenerationTuple(value) {
  if (!value || typeof value !== "object") throw new TypeError("generation tuple required");
  return Object.freeze({
    scene: value.scene ?? 0,
    view: value.view ?? 0,
    overlay: value.overlay ?? 0,
    resource: value.resource ?? 0,
    surface: value.surface ?? 0,
    renderer: value.renderer ?? 0
  });
}

export function createFramePacer({ requestFrame, cancelFrame = () => {}, now = () => 0, onFrame }) {
  if (typeof requestFrame !== "function" || typeof onFrame !== "function") throw new TypeError("requestFrame/onFrame required");
  let scheduled = null;
  let running = false;
  let followUpNeeded = false;
  let hidden = false;
  let dirty = new Set();
  let latestState = null;
  let frameSeq = 0;
  const stats = { requests:0, scheduled:0, run:0, coalesced:0, stale_completions:0, hidden_suppressed:0 };

  const schedule = () => {
    if (hidden || scheduled !== null) return;
    scheduled = requestFrame(run);
    stats.scheduled += 1;
  };

  const run = async (timestamp) => {
    scheduled = null;
    if (hidden || !latestState) return;
    running = true;
    const snapshot = Object.freeze({
      frame_seq: ++frameSeq,
      timestamp: Number.isFinite(timestamp) ? timestamp : now(),
      dirty: Object.freeze([...dirty].sort()),
      generations: assertGenerationTuple(latestState.generations),
      payload: latestState.payload
    });
    dirty.clear();
    followUpNeeded = false;
    stats.run += 1;
    try { await onFrame(snapshot); } finally {
      running = false;
      if ((followUpNeeded || dirty.size) && !hidden) schedule();
    }
  };

  return Object.freeze({
    invalidate(classes, state) {
      stats.requests += 1;
      latestState = state;
      for (const cls of classes) {
        if (!DIRTY_CLASSES.includes(cls)) throw new Error("unknown dirty class: " + cls);
        dirty.add(cls);
      }
      if (running) followUpNeeded = true;
      else if (scheduled !== null) stats.coalesced += 1;
      else schedule();
    },
    setHidden(value) {
      hidden = Boolean(value);
      if (hidden && scheduled !== null) {
        cancelFrame(scheduled);
        scheduled = null;
        stats.hidden_suppressed += 1;
      } else if (!hidden && latestState && dirty.size) {
        schedule();
      }
    },
    acceptAsyncCompletion(completion, currentGenerations) {
      const expected = assertGenerationTuple(currentGenerations);
      const got = assertGenerationTuple(completion.generations);
      const keys = ["scene","view","overlay","resource","surface","renderer"];
      const ok = keys.every((k) => got[k] === expected[k]);
      if (!ok) stats.stale_completions += 1;
      return ok;
    },
    stats() { return Object.freeze({ ...stats, pending: scheduled !== null, running, dirty:[...dirty].sort() }); }
  });
}
