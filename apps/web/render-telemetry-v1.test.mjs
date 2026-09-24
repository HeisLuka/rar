import test from "node:test";
import assert from "node:assert/strict";
import { BrowserObservabilityV1 } from "./observability-v1.mjs";
import { RendererTelemetryV1, boundedLabels } from "./render-telemetry-v1.mjs";

const ctx = { protocol_version: "chaptera.trace-context.v1", trace_id: "trace:12345678", interaction_id: "inter:12345678", session_incarnation: "session:12345678", operation_class: "other", browser_family: "chromium" };

test("disabled mode emits no counters or spans", () => {
  const obs = new BrowserObservabilityV1(); const t = new RendererTelemetryV1({observability:obs,mode:"disabled"});
  t.recordStage("frame",ctx,{durationMs:2,labels:{backend_family:"fake"}}); t.recordCache("texture","hit"); t.recordUpload({logicalBytes:10,physicalBytes:20});
  const r=t.receipt(ctx.trace_id); assert.deepEqual(r.stages,{}); assert.deepEqual(r.cache_counters,{}); assert.equal(r.trace.span_count,0);
});

test("full mode extends existing trace spine", () => {
  const obs = new BrowserObservabilityV1(); const t = new RendererTelemetryV1({observability:obs,mode:"full"});
  t.recordStage("queue_wait",ctx,{durationMs:1.5,labels:{backend_family:"fake",browser_family:"chromium",dirty_class:"scene",frame_outcome:"presented",workload_class:"simple",protocol_major:"v1"}});
  t.correlate(ctx,{request_generation:7,device_generation:2,resource_id:"must_not_leak"});
  const r=t.receipt(ctx.trace_id); assert.equal(r.trace.span_count,1); assert.equal(r.trace.spans[0].name,"renderer.queue_wait"); assert.equal(r.trace_correlation[0].request_generation,7); assert.ok(!("resource_id" in r.trace_correlation[0]));
});

test("high cardinality identities are forbidden metric labels", () => {
  assert.throws(()=>boundedLabels({backend_family:"fake",document_id:"doc:1"}),/forbidden/);
});

test("cache worker upload and memory facts are bounded", () => {
  const t = new RendererTelemetryV1({observability:new BrowserObservabilityV1(),mode:"counters"});
  t.recordCache("texture","lookup",3); t.recordCache("texture","hit",2); t.recordCache("texture","miss",1);
  t.recordWorker({cloneBytes:100,transferableBytes:900,staleResults:2,restarts:1}); t.recordUpload({logicalBytes:1000,physicalBytes:1250}); t.recordMemory({residentBytes:4096,pinnedBytes:1024,reclaimedBytes:512,churnBytes:256});
  const r=t.receipt(ctx.trace_id); assert.equal(r.cache_counters["texture.hit"],2); assert.equal(r.upload.amplification_ratio,1.25); assert.equal(r.worker.transferable_bytes,900); assert.equal(r.memory.resident_bytes,4096);
});

test("unsupported GPU and present timings stay explicit unknown", () => {
  const r = new RendererTelemetryV1({observability:new BrowserObservabilityV1(),mode:"counters"}).receipt(ctx.trace_id);
  assert.equal(r.timings.gpu_execution_ms.state,"unknown"); assert.equal(r.timings.submit_to_present_ms.state,"unknown");
});
