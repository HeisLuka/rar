import { normalizeTraceContextV1 } from "./observability-v1.mjs";

export const RENDER_TELEMETRY_SCHEMA = "chaptera.renderer-telemetry.v1";
const MODES = new Set(["disabled", "counters", "full"]);
const BACKENDS = new Set(["svg", "canvas2d", "webgl2-hybrid", "wgpu", "software", "fake", "unknown"]);
const DIRTY = new Set(["view", "overlay", "resource", "scene", "surface", "fidelity", "mixed"]);
const OUTCOMES = new Set(["presented", "superseded", "cancelled", "failed", "unknown"]);
const WORKLOADS = new Set(["simple", "text-heavy", "image-heavy", "effect-heavy", "stress", "unknown"]);
const STAGES = new Set([
  "frame", "queue_wait", "coalesce", "patch_apply", "cull", "spatial", "segment_plan",
  "worker_wait", "worker_compute", "worker_transfer", "material_resolve", "upload_plan",
  "upload_execute", "submit_build", "backend_submit", "present"
]);
const CACHES = new Set(["glyph", "path", "texture", "pipeline", "binding", "command", "spatial", "segment", "clip-mask"]);
const CACHE_EVENTS = new Set(["lookup", "hit", "miss", "insert", "eviction", "rebuild", "stale_reject"]);
const FORBIDDEN_METRIC_LABELS = new Set(["document_id", "revision_id", "node_id", "story_id", "resource_id", "shard_hash", "file_name", "content_hash", "url"]);

function nonNegative(value, label) {
  if (!Number.isFinite(value) || value < 0) throw new TypeError(label + " must be non-negative");
  return value;
}
function integer(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) throw new TypeError(label + " must be a non-negative integer");
  return value;
}
function boundedLabels(labels = {}) {
  for (const key of Object.keys(labels)) if (FORBIDDEN_METRIC_LABELS.has(key)) throw new Error("high-cardinality metric label forbidden: " + key);
  const out = {
    backend_family: labels.backend_family ?? "unknown",
    browser_family: labels.browser_family ?? "unknown",
    dirty_class: labels.dirty_class ?? "mixed",
    frame_outcome: labels.frame_outcome ?? "unknown",
    workload_class: labels.workload_class ?? "unknown",
    protocol_major: labels.protocol_major ?? "v1",
  };
  if (!BACKENDS.has(out.backend_family)) throw new TypeError("unsupported backend_family");
  if (!DIRTY.has(out.dirty_class)) throw new TypeError("unsupported dirty_class");
  if (!OUTCOMES.has(out.frame_outcome)) throw new TypeError("unsupported frame_outcome");
  if (!WORKLOADS.has(out.workload_class)) throw new TypeError("unsupported workload_class");
  if (typeof out.browser_family !== "string" || out.browser_family.length > 24) throw new TypeError("browser_family must be bounded");
  if (!/^v[0-9]+$/.test(out.protocol_major)) throw new TypeError("protocol_major must be bounded major version");
  return Object.freeze(out);
}

export class RendererTelemetryV1 {
  constructor({ observability, mode = "counters", realPub = false, representative = false, maxTraceCorrelation = 128 } = {}) {
    if (!MODES.has(mode)) throw new TypeError("unsupported telemetry mode");
    if (!observability || typeof observability.mark !== "function" || typeof observability.receipt !== "function") throw new TypeError("existing BrowserObservabilityV1 instance required");
    this.observability = observability;
    this.mode = mode;
    this.realPub = Boolean(realPub);
    this.representative = Boolean(representative);
    this.maxTraceCorrelation = integer(maxTraceCorrelation, "maxTraceCorrelation") || 1;
    this.stage = {};
    this.cache = {};
    this.worker = { clone_bytes: 0, transferable_bytes: 0, stale_results: 0, restarts: 0 };
    this.upload = { logical_bytes: 0, physical_bytes: 0 };
    this.memory = { resident_bytes: 0, pinned_bytes: 0, reclaimed_bytes: 0, churn_bytes: 0 };
    this.traceCorrelation = [];
    this.unknownTimings = {
      gpu_execution_ms: { state: "unknown", reason: "gpu_timing_unsupported" },
      submit_to_present_ms: { state: "unknown", reason: "present_timing_unsupported" },
    };
  }

  recordStage(name, context, { durationMs = 0, outcome = "success", labels = {} } = {}) {
    if (!STAGES.has(name)) throw new TypeError("unsupported renderer stage");
    const normalized = normalizeTraceContextV1(context);
    if (this.mode === "disabled") return null;
    const metricLabels = boundedLabels(labels);
    const row = this.stage[name] ?? { count: 0, duration_ms: 0 };
    row.count += 1; row.duration_ms += nonNegative(durationMs, "durationMs"); this.stage[name] = row;
    if (this.mode === "full") this.observability.mark("renderer." + name, normalized, { outcome, durationMs });
    return { stage: name, labels: metricLabels };
  }

  recordCache(cache, event, count = 1) {
    if (this.mode === "disabled") return;
    if (!CACHES.has(cache) || !CACHE_EVENTS.has(event)) throw new TypeError("unsupported cache counter");
    const key = cache + "." + event;
    this.cache[key] = (this.cache[key] ?? 0) + integer(count, "count");
  }

  recordWorker({ cloneBytes = 0, transferableBytes = 0, staleResults = 0, restarts = 0 } = {}) {
    if (this.mode === "disabled") return;
    this.worker.clone_bytes += integer(cloneBytes, "cloneBytes");
    this.worker.transferable_bytes += integer(transferableBytes, "transferableBytes");
    this.worker.stale_results += integer(staleResults, "staleResults");
    this.worker.restarts += integer(restarts, "restarts");
  }

  recordUpload({ logicalBytes = 0, physicalBytes = 0 } = {}) {
    if (this.mode === "disabled") return;
    this.upload.logical_bytes += integer(logicalBytes, "logicalBytes");
    this.upload.physical_bytes += integer(physicalBytes, "physicalBytes");
  }

  recordMemory({ residentBytes = 0, pinnedBytes = 0, reclaimedBytes = 0, churnBytes = 0 } = {}) {
    if (this.mode === "disabled") return;
    this.memory.resident_bytes = integer(residentBytes, "residentBytes");
    this.memory.pinned_bytes = integer(pinnedBytes, "pinnedBytes");
    this.memory.reclaimed_bytes += integer(reclaimedBytes, "reclaimedBytes");
    this.memory.churn_bytes += integer(churnBytes, "churnBytes");
  }

  correlate(context, generations = {}) {
    if (this.mode !== "full") return;
    const normalized = normalizeTraceContextV1(context);
    const allowed = ["renderer_frame_id", "request_generation", "scene_generation", "view_generation", "overlay_generation", "worker_generation", "material_generation", "device_generation", "surface_generation"];
    const row = { trace_id: normalized.trace_id };
    for (const key of allowed) if (generations[key] != null) row[key] = generations[key];
    this.traceCorrelation.push(row);
    if (this.traceCorrelation.length > this.maxTraceCorrelation) this.traceCorrelation.splice(0, this.traceCorrelation.length - this.maxTraceCorrelation);
  }

  recordTimingUnknown(metric, reason) {
    if (!["gpu_execution_ms", "submit_to_present_ms"].includes(metric)) throw new TypeError("unsupported timing metric");
    if (!/^[a-z0-9_:-]{3,64}$/.test(reason)) throw new TypeError("reason must be bounded code");
    this.unknownTimings[metric] = { state: "unknown", reason };
  }

  receipt(traceId) {
    const logical = this.upload.logical_bytes;
    const amplification = logical === 0 ? null : this.upload.physical_bytes / logical;
    return {
      schema: RENDER_TELEMETRY_SCHEMA,
      mode: this.mode,
      semantic_authority: false,
      evidence_authority: { real_pub: this.realPub, representative: this.representative },
      stages: structuredClone(this.stage),
      cache_counters: structuredClone(this.cache),
      worker: structuredClone(this.worker),
      upload: { ...this.upload, amplification_ratio: amplification },
      memory: structuredClone(this.memory),
      timings: structuredClone(this.unknownTimings),
      trace_correlation: this.mode === "full" ? structuredClone(this.traceCorrelation) : [],
      trace: this.mode === "full" ? this.observability.receipt(traceId) : { trace_id: traceId, span_count: 0, spans: [] },
    };
  }
}

export { boundedLabels };
