import test from "node:test";
import assert from "node:assert/strict";
import { BrowserObservabilityV1 } from "./observability-v1.mjs";
import {
  RendererTelemetryV1,
  boundedRendererMetricLabels,
  validateCacheDenominators,
} from "./render-telemetry-v1.mjs";

const makeObs = () => new BrowserObservabilityV1({
  sessionIncarnation: "session:12345678",
  browserFamily: "chromium",
  idFactory: (prefix) => prefix + ":12345678",
});

const ctx = {
  protocol_version: "chaptera.trace-context.v1",
  trace_id: "trace:12345678",
  interaction_id: "inter:12345678",
  session_incarnation: "session:12345678",
  operation_class: "other",
  browser_family: "chromium",
};

const labels = {
  backend_family: "fake",
  browser_family: "chromium",
  dirty_class: "scene",
  frame_outcome: "presented",
  workload_class: "simple",
  protocol_major: "v1",
  device_class: "test",
};

test("disabled mode preserves output contract while emitting no telemetry facts", () => {
  const t = new RendererTelemetryV1({ observability: makeObs(), mode: "disabled" });
  t.recordStage("frame", ctx, { durationMs: 2, labels });
  t.recordCache("texture", "hit");
  t.recordScheduler({ requestsReceived: 1, pending: 1 });
  t.recordWorker({ cloneBytes: 100 });
  t.recordUpload({ logicalBytes: 10, physicalBytes: 20 });
  t.recordMemory({ residentBytes: 4096 });
  const r = t.receipt(ctx.trace_id, { ...labels, backendFamily: "fake", browserFamily: "chromium", deviceClass: "test", workloadClass: "simple" });
  assert.deepEqual(r.stages, {});
  assert.deepEqual(r.cache_counters, {});
  assert.equal(r.trace.span_count, 0);
  assert.equal(r.contains_document_payload, false);
});

test("full mode extends existing trace spine without inventing semantic identity", () => {
  const obs = makeObs();
  const t = new RendererTelemetryV1({ observability: obs, mode: "full" });
  t.recordStage("queue_wait", ctx, { durationMs: 1.5, labels });
  t.correlate(ctx, { request_generation: 7, device_generation: 2, resource_id: "must_not_leak" });
  const r = t.receipt(ctx.trace_id, { backendFamily: "fake", browserFamily: "chromium", deviceClass: "test", workloadClass: "simple" });
  assert.equal(r.trace.span_count, 1);
  assert.equal(r.trace.spans[0].name, "renderer.queue_wait");
  assert.equal(r.trace.spans[0].client_operation_id, undefined);
  assert.equal(r.trace_correlation[0].request_generation, 7);
  assert.ok(!("resource_id" in r.trace_correlation[0]));
});

test("metric labels reject high-cardinality semantic identities", () => {
  assert.throws(
    () => boundedRendererMetricLabels({ backend_family: "fake", document_id: "doc:1" }),
    /forbidden/,
  );
  assert.throws(
    () => boundedRendererMetricLabels({ backend_family: "fake", trace_id: "trace:1" }),
    /forbidden/,
  );
});

test("stage distributions are bounded and explicit", () => {
  const t = new RendererTelemetryV1({ observability: makeObs(), mode: "full", maxStageSamples: 3 });
  for (const ms of [1, 2, 3, 4]) t.recordStage("frame", ctx, { durationMs: ms, labels });
  const stage = t.receipt(ctx.trace_id, { backendFamily: "fake", browserFamily: "chromium", deviceClass: "test", workloadClass: "simple" }).stages.frame;
  assert.equal(stage.count, 4);
  assert.equal(stage.sampled_count, 3);
  assert.equal(stage.samples_truncated, true);
  assert.equal(stage.p50_ms, 2);
  assert.equal(stage.p95_ms, 3);
  assert.equal(stage.duration_ms_max, 4);
});

test("cache denominators upload amplification worker scheduler and memory are machine-checkable", () => {
  const t = new RendererTelemetryV1({ observability: makeObs(), mode: "counters" });
  t.recordCache("texture", "lookup", 3);
  t.recordCache("texture", "hit", 2);
  t.recordCache("texture", "miss", 1);
  t.recordScheduler({ requestsReceived: 5, requestsCoalesced: 2, pending: 3, inFlight: 1, oldestPendingAgeMs: 8.5, presentedGenerationLag: 2 });
  t.recordWorker({ cloneBytes: 100, transferableBytes: 900, staleResults: 2, restarts: 1, queueDepth: 4, inFlightBytes: 2048 });
  t.recordUpload({ scenePatchBytes: 1000, logicalBytes: 64, physicalBytes: 4 * 1024 * 1024, textureMaterialBytes: 512 });
  t.recordMemory({ residentBytes: 4096, pinnedBytes: 1024, reclaimableBytes: 2048, bytesRequested: 512, bytesReclaimed: 256, churnBytes: 128, inabilityToReclaim: 1 });
  const r = t.receipt(ctx.trace_id, { backendFamily: "fake", browserFamily: "chromium", deviceClass: "test", workloadClass: "stress" });
  assert.equal(validateCacheDenominators(r.cache_counters), true);
  assert.equal(r.upload.amplification_ratio, 65536);
  assert.equal(r.worker.transferable_bytes, 900);
  assert.equal(r.scheduler.max_pending, 3);
  assert.equal(r.memory.reclaimable_bytes, 2048);
});

test("unavailable GPU present and memory facts remain explicit unknown", () => {
  const r = new RendererTelemetryV1({ observability: makeObs(), mode: "counters" })
    .receipt(ctx.trace_id, { backendFamily: "fake", browserFamily: "chromium", deviceClass: "test", workloadClass: "simple" });
  assert.equal(r.timings.gpu_execution_ms.state, "unknown");
  assert.equal(r.timings.submit_to_present_ms.state, "unknown");
  assert.equal(r.timings.gpu_memory_bytes.state, "unknown");
});

test("evidence flags prevent synthetic data from authorizing a technology decision", () => {
  assert.throws(
    () => new RendererTelemetryV1({ observability: makeObs(), representativeCorpus: false, technologyDecisionAllowed: true }),
    /requires representativeCorpus/,
  );
  const r = new RendererTelemetryV1({
    observability: makeObs(),
    mode: "counters",
    realPub: false,
    representativeCorpus: false,
  }).receipt(ctx.trace_id, { backendFamily: "fake", browserFamily: "chromium", deviceClass: "test", workloadClass: "stress" });
  assert.deepEqual(r.evidence_authority, {
    real_pub: false,
    representative_corpus: false,
    technology_decision_allowed: false,
  });
});

test("no user payload or arbitrary diagnostic content enters the receipt", () => {
  const t = new RendererTelemetryV1({ observability: makeObs(), mode: "full" });
  t.recordStage("frame", ctx, { durationMs: 1, labels });
  t.recordTimingUnknown("gpu_execution_ms", "timer_query_unavailable");
  const serialized = JSON.stringify(t.receipt(ctx.trace_id, { backendFamily: "fake", browserFamily: "chromium", deviceClass: "test", workloadClass: "simple" }));
  for (const forbidden of ["document_id", "resource_id", "file_name", "signed_url", "clipboard"]) {
    assert.equal(serialized.includes(forbidden), false);
  }
});
