export const FRAME_PACING_SCHEMA = "chaptera.web-frame-pacing.v2";
export const DIRTY_CLASSES = Object.freeze([
  "view", "overlay", "resource", "scene", "surface", "fidelity",
]);

const GENERATION_KEYS = Object.freeze([
  "scene", "view", "overlay", "resource", "surface", "renderer", "worker",
]);

function nonNegativeInt(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new TypeError(label + " must be a non-negative integer");
  }
  return value;
}

export function normalizeFrameGenerations(value = {}) {
  const out = {};
  for (const key of GENERATION_KEYS) {
    out[key] = nonNegativeInt(value[key] ?? 0, key);
  }
  return Object.freeze(out);
}

export function sameFrameGenerations(left, right) {
  const a = normalizeFrameGenerations(left);
  const b = normalizeFrameGenerations(right);
  return GENERATION_KEYS.every((key) => a[key] === b[key]);
}

function normalizeDirty(classes) {
  if (!classes || typeof classes[Symbol.iterator] !== "function") {
    throw new TypeError("dirty classes must be iterable");
  }
  const out = [];
  for (const value of classes) {
    if (!DIRTY_CLASSES.includes(value)) {
      throw new TypeError("unknown dirty class: " + value);
    }
    if (!out.includes(value)) out.push(value);
  }
  if (out.length === 0) throw new TypeError("at least one dirty class is required");
  return out;
}

function freezeState(state) {
  if (!state || typeof state !== "object") throw new TypeError("frame state is required");
  return Object.freeze({
    generations: normalizeFrameGenerations(state.generations),
    payload: state.payload,
  });
}

export function createFramePacer({
  requestFrame,
  cancelFrame = () => {},
  now = () => performance.now(),
  onFrame,
} = {}) {
  if (typeof requestFrame !== "function") throw new TypeError("requestFrame is required");
  if (typeof cancelFrame !== "function") throw new TypeError("cancelFrame must be a function");
  if (typeof now !== "function") throw new TypeError("now must be a function");
  if (typeof onFrame !== "function") throw new TypeError("onFrame is required");

  let scheduledHandle = null;
  let running = false;
  let hidden = false;
  let followUpNeeded = false;
  let dirty = new Set();
  let latestState = null;
  let frameSeq = 0;
  let visibleFrame = null;

  const stats = {
    invalidation_requests: 0,
    callbacks_scheduled: 0,
    callbacks_run: 0,
    coalesced_requests: 0,
    followups_scheduled: 0,
    stale_completions: 0,
    accepted_completions: 0,
    hidden_suppressed_callbacks: 0,
    hidden_invalidations: 0,
    max_dirty_classes: 0,
  };

  const schedule = (isFollowUp = false) => {
    if (hidden || scheduledHandle !== null) return false;
    scheduledHandle = requestFrame(run);
    stats.callbacks_scheduled += 1;
    if (isFollowUp) stats.followups_scheduled += 1;
    return true;
  };

  const run = async (timestamp) => {
    scheduledHandle = null;
    if (hidden || !latestState || dirty.size === 0) return;

    running = true;
    followUpNeeded = false;
    const state = latestState;
    const frameDirty = Object.freeze([...dirty].sort());
    dirty.clear();

    const snapshot = Object.freeze({
      schema: FRAME_PACING_SCHEMA,
      frame_seq: ++frameSeq,
      timestamp_ms: Number.isFinite(timestamp) ? timestamp : now(),
      dirty: frameDirty,
      generations: state.generations,
      payload: state.payload,
      semantic_authority: false,
    });
    stats.callbacks_run += 1;

    let outcome = "completed";
    try {
      await onFrame(snapshot);
      visibleFrame = Object.freeze({
        frame_seq: snapshot.frame_seq,
        generations: snapshot.generations,
        dirty: snapshot.dirty,
      });
    } catch (error) {
      outcome = "failed";
      throw error;
    } finally {
      running = false;
      if (outcome === "completed" && !hidden && (followUpNeeded || dirty.size > 0)) {
        schedule(true);
      }
    }
  };

  return Object.freeze({
    invalidate(classes, state) {
      const normalizedDirty = normalizeDirty(classes);
      const normalizedState = freezeState(state);
      stats.invalidation_requests += 1;
      latestState = normalizedState;
      for (const cls of normalizedDirty) dirty.add(cls);
      stats.max_dirty_classes = Math.max(stats.max_dirty_classes, dirty.size);

      if (hidden) {
        stats.hidden_invalidations += 1;
        return Object.freeze({ scheduled: false, reason: "hidden_latest_state_retained" });
      }
      if (running) {
        followUpNeeded = true;
        stats.coalesced_requests += 1;
        return Object.freeze({ scheduled: false, reason: "running_followup_needed" });
      }
      if (scheduledHandle !== null) {
        stats.coalesced_requests += 1;
        return Object.freeze({ scheduled: false, reason: "already_scheduled" });
      }
      schedule(false);
      return Object.freeze({ scheduled: true, reason: "scheduled" });
    },

    setHidden(value) {
      const next = Boolean(value);
      if (next === hidden) return;
      hidden = next;
      if (hidden && scheduledHandle !== null) {
        cancelFrame(scheduledHandle);
        scheduledHandle = null;
        stats.hidden_suppressed_callbacks += 1;
      }
      if (!hidden && latestState && dirty.size > 0 && !running) schedule(false);
    },

    acceptAsyncCompletion(completion, currentGenerations) {
      if (!completion || typeof completion !== "object") {
        throw new TypeError("completion is required");
      }
      const ok = sameFrameGenerations(completion.generations, currentGenerations);
      if (ok) stats.accepted_completions += 1;
      else stats.stale_completions += 1;
      return ok;
    },

    currentState() {
      return latestState;
    },

    visibleFrame() {
      return visibleFrame;
    },

    stats() {
      return Object.freeze({
        ...stats,
        pending: scheduledHandle !== null,
        running,
        hidden,
        follow_up_needed: followUpNeeded,
        dirty: Object.freeze([...dirty].sort()),
        latest_generations: latestState?.generations ?? null,
        visible_frame: visibleFrame,
        semantic_authority: false,
        canonical_mutations_emitted: 0,
      });
    },
  });
}
