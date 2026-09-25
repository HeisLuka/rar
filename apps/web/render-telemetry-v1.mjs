import { normalizeTraceContextV1 } from "./observability-v1.mjs";

export const RENDER_TELEMETRY_SCHEMA = "chaptera.renderer-telemetry.v1";
export const RENDER_TELEMETRY_OVERHEAD_SCHEMA = "chaptera.renderer-telemetry-overhead.v1";

const MODES = new Set(["disabled", "counters", "full"]);
const BACKENDS = new Set(["svg", "canvas2d", "webgl2-hybrid", "wgpu", "software", "fake", "unknown"]);
const BROWSERS = new Set(["chromium", "firefox", "webkit", "other", "unknown"]);
const DIRTY = new Set(["view", "overlay", "resource", "scene", "surface", "fidelity", "mixed"]);
const OUTCOMES = new Set(["presented", "superseded", "cancelled", "failed", "unknown"]);
const WORKLOADS = new Set(["simple", "text-heavy", "image-heavy", "effect-heavy", "stress", "unknown"]);
const STAGES = new Set([
  "frame", "queue_wait", "coalesce", "patch_apply", "cull", "spatial", "segment_plan",
  "worker_wait", "worker_compute", "worker_transfer", "material_resolve", "resource_wait",
  "upload_plan", "upload_execute", "submit_build", "backend_submit", "present",
]);
const CACHES = new Set(["glyph", "path", "texture", "pipeline", "binding", "command", "spatial", "segment", "clip-mask"]);
const CACHE_EVENTS = new Set(["lookup", "hit", "miss", "insert", "eviction", "rebuild", "stale_reject"]);
const FORBIDDEN_METRIC_LABELS = new Set([
  "document_id", "revision_id", "node_id", "story_id", "resource_id", "shard_hash",
  "file_name", "content_hash", "url", "client_operation_id", "trace_id", "interaction_id",
]);
const CORRELATION_KEYS = new Set([
  "renderer_frame_id", "request_generation", "scene_generation", "view_generation",
  "overlay_generation", "worker_generation", "material_generation", "device_generation",
  "surface_generation",
]);

function finiteNonNegative(value, label) {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
    throw new TypeError(label + " must be non-negative");
  }
  return value;
}

function integer(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) throw new TypeError(label + " must be a non-negative integer");
  return value;
}

function boundedCode(value, label) {
  if (typeof value !== "string" || !/^[a-z0-9_:-]{1,64}$/.test(value)) {
    throw new TypeError(label + " must be a bounded machine-readable code");
  }
  return value;
}

function percentile(sorted, p) {
  if (sorted.length === 0) return null;
  const index = Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * p) - 1));
  return Math.round(sorted[index] * 1000) / 1000;
}

function summarizeStage(row) {
  const samples = [...row.samples_ms].sort((a, b) => a - b);
  return {
    count: row.count,
    duration_ms_total: Math.round(row.duration_ms_total * 1000) / 1000,
    duration_ms_max: Math.round(row.duration_ms_max * 1000) / 1000,
    sampled_count: samples.length,
    p50_ms: percentile(samples, 0.50),
    p95_ms: percentile(samples, 0.95),
    p99_ms: percentile(samples, 0.99),
    samples_truncated: row.count > samples.length,
  };
}

export function boundedRendererMetricLabels(labels = {}) {
  for (const key of Object.keys(labels)) {
    if (FORBIDDEN_METRIC_LABELS.has(key)) throw new Error("high-cardinality metric label forbidden: " + key);
  }
  const out = {
    backend_family: labels.backend_family ?? "unknown",
    browser_family: labels.browser_family ?? "unknown",
    dirty_class: labels.dirty_class ?? "mixed",
    frame_outcome: labels.frame_outcome ?? "unknown",
    workload_class: labels.workload_class ?? "unknown",
    protocol_major: labels.protocol_major ?? "v1",
    device_class: labels.device_class ?? "unknown",
  };
  if (!BACKENDS.has(out.backend_family)) throw new TypeError("unsupported backend_family");
  if (!BROWSERS.has(out.browser_family)) throw new TypeError("unsupported browser_family");
  if (!DIRTY.has(out.dirty_class)) throw new TypeError("unsupported dirty_class");
  if (!OUTCOMES.has(out.frame_outcome)) throw new TypeError("unsupported frame_outcome");
  if (!WORKLOADS.has(out.workload_class)) throw new TypeError("unsupported workload_class");
  if (!/^v[0-9]+$/.test(out.protocol_major)) throw new TypeError("protocol_major must be a bounded major version");
  if (typeof out.device_class !== "string" || !/^[a-z0-9_-]{1,24}$/.test(out.device_class)) {
    throw new TypeError("device_class must be bounded");
  }
  return Object.freeze(out);
}

export class RendererTelemetryV1 {
  constructor({
    observability,
    mode = "counters",
    realPub = false,
    representativeCorpus = false,
    technologyDecisionAllowed = false,
    maxTraceCorrelation = 128,
    maxStageSamples = 2048,
  } = {}) {
    if (!MODES.has(mode)) throw new TypeError("unsupported telemetry mode");
    if (!observability || typeof observability.mark !== "function" || typeof observability.receipt !== "function") {
      throw new TypeError("existing BrowserObservabilityV1 instance required");
    }
    if (technologyDecisionAllowed && !representativeCorpus) {
      throw new TypeError("technologyDecisionAllowed requires representativeCorpus");
    }
    this.observability = observability;
    this.mode = mode;
    this.realPub = Boolean(realPub);
    this.representativeCorpus = Boolean(representativeCorpus);
    this.technologyDecisionAllowed = Boolean(technologyDecisionAllowed);
    this.maxTraceCorrelation = Math.max(1, integer(maxTraceCorrelation, "maxTraceCorrelation"));
    this.maxStageSamples = Math.max(1, integer(maxStageSamples, "maxStageSamples"));
    this.stage = {};
    this.cache = {};
    this.scheduler = {
      requests_received: 0,
      requests_coalesced: 0,
      requests_superseded: 0,
      stale_cancelled: 0,
      stale_submitted: 0,
      max_pending: 0,
      max_in_flight: 0,
      max_pending_age_ms: 0,
      max_presented_generation_lag: 0,
    };
    this.worker = {
      clone_bytes: 0,
      transferable_bytes: 0,
      stale_results: 0,
      restarts: 0,
      max_queue_depth: 0,
      max_in_flight_bytes: 0,
    };
    this.upload = {
      scene_patch_bytes: 0,
      logical_bytes: 0,
      physical_bytes: 0,
      texture_material_bytes: 0,
    };
    this.memory = {
      resident_bytes: 0,
      pinned_bytes: 0,
      reclaimable_bytes: 0,
      bytes_requested: 0,
      bytes_reclaimed: 0,
      churn_bytes: 0,
      inability_to_reclaim: 0,
    };
    this.traceCorrelation = [];
    this.unknownTimings = {
      gpu_execution_ms: { state: "unknown", reason: "gpu_timing_unsupported" },
      submit_to_present_ms: { state: "unknown", reason: "present_timing_unsupported" },
      gpu_memory_bytes: { state: "unknown", reason: "gpu_memory_unsupported" },
    };
  }

  recordStage(name, context, { durationMs = 0, outcome = "success", labels = {} } = {}) {
    if (!STAGES.has(name)) throw new TypeError("unsupported renderer stage");
    const normalized = normalizeTraceContextV1(context);
    if (this.mode === "disabled") return null;
    const metricLabels = boundedRendererMetricLabels(labels);
    const duration = finiteNonNegative(durationMs, "durationMs");
    const row = this.stage[name] ?? { count: 0, duration_ms_total: 0, duration_ms_max: 0, samples_ms: [] };
    row.count += 1;
    row.duration_ms_total += duration;
    row.duration_ms_max = Math.max(row.duration_ms_max, duration);
    if (this.mode === "full" && row.samples_ms.length < this.maxStageSamples) row.samples_ms.push(duration);
    this.stage[name] = row;
    if (this.mode === "full") {
      this.observability.mark("renderer." + name, normalized, { outcome, durationMs: duration });
    }
    return { stage: name, labels: metricLabels };
  }

  recordCache(cache, event, count = 1) {
    if (this.mode === "disabled") return;
    if (!CACHES.has(cache) || !CACHE_EVENTS.has(event)) throw new TypeError("unsupported cache counter");
    const key = cache + "." + event;
    this.cache[key] = (this.cache[key] ?? 0) + integer(count, "count");
  }

  recordScheduler({
    requestsReceived = 0,
    requestsCoalesced = 0,
    requestsSuperseded = 0,
    staleCancelled = 0,
    staleSubmitted = 0,
    pending = 0,
    inFlight = 0,
    oldestPendingAgeMs = 0,
    presentedGenerationLag = 0,
  } = {}) {
    if (this.mode === "disabled") return;
    this.scheduler.requests_received += integer(requestsReceived, "requestsReceived");
    this.scheduler.requests_coalesced += integer(requestsCoalesced, "requestsCoalesced");
    this.scheduler.requests_superseded += integer(requestsSuperseded, "requestsSuperseded");
    this.scheduler.stale_cancelled += integer(staleCancelled, "staleCancelled");
    this.scheduler.stale_submitted += integer(staleSubmitted, "staleSubmitted");
    this.scheduler.max_pending = Math.max(this.scheduler.max_pending, integer(pending, "pending"));
    this.scheduler.max_in_flight = Math.max(this.scheduler.max_in_flight, integer(inFlight, "inFlight"));
    this.scheduler.max_pending_age_ms = Math.max(
      this.scheduler.max_pending_age_ms,
      finiteNonNegative(oldestPendingAgeMs, "oldestPendingAgeMs"),
    );
    this.scheduler.max_presented_generation_lag = Math.max(
      this.scheduler.max_presented_generation_lag,
      integer(presentedGenerationLag, "presentedGenerationLag"),
    );
  }

  recordWorker({
    cloneBytes = 0,
    transferableBytes = 0,
    staleResults = 0,
    restarts = 0,
    queueDepth = 0,
    inFlightBytes = 0,
  } = {}) {
    if (this.mode === "disabled") return;
    this.worker.clone_bytes += integer(cloneBytes, "cloneBytes");
    this.worker.transferable_bytes += integer(transferableBytes, "transferableBytes");
    this.worker.stale_results += integer(staleResults, "staleResults");
    this.worker.restarts += integer(restarts, "restarts");
    this.worker.max_queue_depth = Math.max(this.worker.max_queue_depth, integer(queueDepth, "queueDepth"));
    this.worker.max_in_flight_bytes = Math.max(this.worker.max_in_flight_bytes, integer(inFlightBytes, "inFlightBytes"));
  }

  recordUpload({
    scenePatchBytes = 0,
    logicalBytes = 0,
    physicalBytes = 0,
    textureMaterialBytes = 0,
  } = {}) {
    if (this.mode === "disabled") return;
    this.upload.scene_patch_bytes += integer(scenePatchBytes, "scenePatchBytes");
    this.upload.logical_bytes += integer(logicalBytes, "logicalBytes");
    this.upload.physical_bytes += integer(physicalBytes, "physicalBytes");
    this.upload.texture_material_bytes += integer(textureMaterialBytes, "textureMaterialBytes");
  }

  recordMemory({
    residentBytes = 0,
    pinnedBytes = 0,
    reclaimableBytes = 0,
    bytesRequested = 0,
    bytesReclaimed = 0,
    churnBytes = 0,
    inabilityToReclaim = 0,
  } = {}) {
    if (this.mode === "disabled") return;
    this.memory.resident_bytes = integer(residentBytes, "residentBytes");
    this.memory.pinned_bytes = integer(pinnedBytes, "pinnedBytes");
    this.memory.reclaimable_bytes = integer(reclaimableBytes, "reclaimableBytes");
    this.memory.bytes_requested += integer(bytesRequested, "bytesRequested");
    this.memory.bytes_reclaimed += integer(bytesReclaimed, "bytesReclaimed");
    this.memory.churn_bytes += integer(churnBytes, "churnBytes");
    this.memory.inability_to_reclaim += integer(inabilityToReclaim, "inabilityToReclaim");
  }

  correlate(context, generations = {}) {
    if (this.mode !== "full") return;
    const normalized = normalizeTraceContextV1(context);
    const row = { trace_id: normalized.trace_id };
    for (const [key, value] of Object.entries(generations)) {
      if (CORRELATION_KEYS.has(key) && value != null) row[key] = value;
    }
    this.traceCorrelation.push(row);
    if (this.traceCorrelation.length > this.maxTraceCorrelation) {
      this.traceCorrelation.splice(0, this.traceCorrelation.length - this.maxTraceCorrelation);
    }
  }

  recordTimingUnknown(metric, reason) {
    if (!Object.hasOwn(this.unknownTimings, metric)) throw new TypeError("unsupported unknown timing/memory metric");
    this.unknownTimings[metric] = { state: "unknown", reason: boundedCode(reason, "reason") };
  }

  receipt(traceId, {
    backendFamily = "unknown",
    browserFamily = "unknown",
    deviceClass = "unknown",
    workloadClass = "unknown",
    protocolMajor = "v1",
    rendererVersion = "unknown",
    frameCount = null,
  } = {}) {
    const labels = boundedRendererMetricLabels({
      backend_family: backendFamily,
      browser_family: browserFamily,
      workload_class: workloadClass,
      protocol_major: protocolMajor,
      device_class: deviceClass,
    });
    const logical = this.upload.logical_bytes;
    const stageSummary = {};
    for (const [name, row] of Object.entries(this.stage)) stageSummary[name] = summarizeStage(row);
    const count = frameCount == null ? (stageSummary.frame?.count ?? 0) : integer(frameCount, "frameCount");
    return {
      schema: RENDER_TELEMETRY_SCHEMA,
      mode: this.mode,
      semantic_authority: false,
      evidence_authority: {
        real_pub: this.realPub,
        representative_corpus: this.representativeCorpus,
        technology_decision_allowed: this.technologyDecisionAllowed,
      },
      environment: {
        backend_family: labels.backend_family,
        browser_family: labels.browser_family,
        device_class: labels.device_class,
        workload_class: labels.workload_class,
        protocol_major: labels.protocol_major,
        renderer_version: String(rendererVersion).slice(0, 64),
      },
      frame_count: count,
      stages: stageSummary,
      scheduler: structuredClone(this.scheduler),
      cache_counters: structuredClone(this.cache),
      worker: structuredClone(this.worker),
      upload: {
        ...this.upload,
        amplification_ratio: logical === 0 ? null : this.upload.physical_bytes / logical,
      },
      memory: structuredClone(this.memory),
      timings: structuredClone(this.unknownTimings),
      trace_correlation: this.mode === "full" ? structuredClone(this.traceCorrelation) : [],
      trace: this.mode === "full"
        ? this.observability.receipt(traceId)
        : { trace_id: traceId, span_count: 0, spans: [], contains_document_payload: false, semantic_authority: false },
      contains_document_payload: false,
    };
  }
}

export function validateCacheDenominators(cacheCounters) {
  for (const cache of CACHES) {
    const lookup = cacheCounters[cache + ".lookup"] ?? 0;
    const hit = cacheCounters[cache + ".hit"] ?? 0;
    const miss = cacheCounters[cache + ".miss"] ?? 0;
    if (hit + miss > lookup) return false;
  }
  return true;
}
