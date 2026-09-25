const GENERATION_KEYS = ["scene", "view", "resource", "surface", "overlay"];
const WORK_KINDS = new Set([
  "scene_parse",
  "spatial_index",
  "segment_plan",
  "material_prepare",
  "image_prepare",
  "offscreen_render",
]);
const TRANSPORTS = new Set(["clone", "transfer"]);

function nonNegativeInt(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) throw new TypeError(label + " must be a non-negative integer");
  return value;
}

function normalizeGenerations(value = {}) {
  const out = {};
  for (const key of GENERATION_KEYS) out[key] = nonNegativeInt(value[key] ?? 0, key);
  return Object.freeze(out);
}

function sameGenerations(left, right) {
  return GENERATION_KEYS.every((key) => left[key] === right[key]);
}

function releaseGraphicsResult(result) {
  const candidates = [];
  if (result && typeof result.close === "function") candidates.push(result);
  if (result?.bitmap && typeof result.bitmap.close === "function") candidates.push(result.bitmap);
  if (Array.isArray(result?.resources)) {
    for (const resource of result.resources) {
      if (resource && typeof resource.close === "function") candidates.push(resource);
    }
  }
  let released = 0;
  const seen = new Set();
  for (const candidate of candidates) {
    if (seen.has(candidate)) continue;
    seen.add(candidate);
    candidate.close();
    released += 1;
  }
  return released;
}

export class WorkerBoundaryV1 {
  constructor({
    maxQueued = 2,
    maxInflight = 2,
    maxBytes = 8 * 1024 * 1024,
    now = () => performance.now(),
  } = {}) {
    this.maxQueued = Math.max(1, nonNegativeInt(maxQueued, "maxQueued"));
    this.maxInflight = Math.max(1, nonNegativeInt(maxInflight, "maxInflight"));
    this.maxBytes = Math.max(1, nonNegativeInt(maxBytes, "maxBytes"));
    if (typeof now !== "function") throw new TypeError("now must be a function");
    this.now = now;
    this.workerGeneration = 1;
    this.currentGenerations = normalizeGenerations();
    this.queue = [];
    this.inflight = new Map();
    this.seq = 0;
    this.metrics = {
      submitted: 0,
      dispatched: 0,
      dropped_backpressure: 0,
      superseded_queued: 0,
      stale_results: 0,
      accepted_results: 0,
      bytes_cloned: 0,
      bytes_transferred: 0,
      peak_queued: 0,
      peak_inflight: 0,
      peak_live_bytes: 0,
      max_queue_age_ms: 0,
      restarts: 0,
      restart_invalidated: 0,
      released_stale_graphics: 0,
    };
  }

  #liveBytes() {
    return this.queue.reduce((sum, job) => sum + job.bytes, 0)
      + [...this.inflight.values()].reduce((sum, job) => sum + job.bytes, 0);
  }

  #snapshotFence(requestId) {
    return Object.freeze({
      ...this.currentGenerations,
      worker: this.workerGeneration,
      request: requestId,
    });
  }

  setCurrentGenerations(generations = {}) {
    const next = normalizeGenerations(generations);
    this.currentGenerations = next;
    const kept = [];
    for (const job of this.queue) {
      if (sameGenerations(job.fence, next)) {
        kept.push(job);
      } else {
        this.metrics.superseded_queued += 1;
      }
    }
    this.queue = kept;
    return this.snapshot();
  }

  submit({
    kind,
    generations = this.currentGenerations,
    bytes = 0,
    transport = "clone",
    sharedArrayBuffer = false,
  } = {}) {
    if (!WORK_KINDS.has(kind)) throw new TypeError("unsupported Worker derived workload kind");
    if (!TRANSPORTS.has(transport)) throw new TypeError("unsupported Worker transport");
    if (sharedArrayBuffer) throw new Error("SharedArrayBuffer is not a V1 baseline transport");
    const normalized = normalizeGenerations(generations);
    if (!sameGenerations(normalized, this.currentGenerations)) {
      throw new Error("submission generations must match current browser derived state");
    }
    bytes = nonNegativeInt(bytes, "bytes");
    if (this.queue.length >= this.maxQueued || this.#liveBytes() + bytes > this.maxBytes) {
      this.metrics.dropped_backpressure += 1;
      return null;
    }
    const id = ++this.seq;
    const job = Object.freeze({
      id,
      kind,
      bytes,
      transport,
      fence: this.#snapshotFence(id),
      queued_at_ms: this.now(),
    });
    this.queue.push(job);
    this.metrics.submitted += 1;
    this.metrics.peak_queued = Math.max(this.metrics.peak_queued, this.queue.length);
    this.metrics.peak_live_bytes = Math.max(this.metrics.peak_live_bytes, this.#liveBytes());
    return job;
  }

  dispatchNext() {
    if (this.inflight.size >= this.maxInflight) return null;
    const job = this.queue.shift();
    if (!job) return null;
    const queueAge = Math.max(0, this.now() - job.queued_at_ms);
    this.metrics.max_queue_age_ms = Math.max(this.metrics.max_queue_age_ms, queueAge);
    const dispatched = Object.freeze({ ...job, dispatched_at_ms: this.now() });
    this.inflight.set(job.id, dispatched);
    this.metrics.dispatched += 1;
    this.metrics.peak_inflight = Math.max(this.metrics.peak_inflight, this.inflight.size);
    this.metrics.peak_live_bytes = Math.max(this.metrics.peak_live_bytes, this.#liveBytes());
    if (job.transport === "transfer") this.metrics.bytes_transferred += job.bytes;
    else this.metrics.bytes_cloned += job.bytes;
    return dispatched;
  }

  complete(id, { generations = this.currentGenerations, result = null } = {}) {
    const job = this.inflight.get(id);
    if (!job) return Object.freeze({ accepted: false, reason: "unknown_or_released" });
    this.inflight.delete(id);
    const current = normalizeGenerations(generations);
    const currentFence = { ...current, worker: this.workerGeneration, request: id };
    const accepted = sameGenerations(job.fence, currentFence)
      && job.fence.worker === currentFence.worker
      && job.fence.request === currentFence.request;
    if (!accepted) {
      const released = releaseGraphicsResult(result);
      this.metrics.released_stale_graphics += released;
      this.metrics.stale_results += 1;
      return Object.freeze({ accepted: false, reason: "stale_generation", released_graphics: released, job });
    }
    this.metrics.accepted_results += 1;
    return Object.freeze({ accepted: true, reason: "current", released_graphics: 0, job });
  }

  restart({ generations = this.currentGenerations } = {}) {
    const next = normalizeGenerations(generations);
    const invalidated = this.queue.length + this.inflight.size;
    this.queue = [];
    this.inflight.clear();
    this.workerGeneration += 1;
    this.currentGenerations = next;
    this.metrics.restarts += 1;
    this.metrics.restart_invalidated += invalidated;
    return Object.freeze({
      worker_generation: this.workerGeneration,
      rebuild_from_generations: next,
      canonical_authority: "server_editor_session",
    });
  }

  snapshot() {
    return Object.freeze({
      ...this.metrics,
      worker_generation: this.workerGeneration,
      queued: this.queue.length,
      inflight: this.inflight.size,
      live_bytes: this.#liveBytes(),
      current_generations: this.currentGenerations,
      shared_array_buffer_baseline: false,
      raw_pub_allowed: false,
      canonical_authority: "server_editor_session",
      worker_authority: "browser_derived_disposable_only",
    });
  }
}

export function transferableArrayBuffer(byteLength) {
  byteLength = nonNegativeInt(byteLength, "byteLength");
  const payload = new ArrayBuffer(byteLength);
  return Object.freeze({ payload, transfer: Object.freeze([payload]) });
}

export function workerDerivedKindsV1() {
  return Object.freeze([...WORK_KINDS]);
}
