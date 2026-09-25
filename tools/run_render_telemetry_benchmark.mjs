#!/usr/bin/env node
import fs from "node:fs";
import { performance } from "node:perf_hooks";
import { BrowserObservabilityV1 } from "../apps/web/observability-v1.mjs";
import { RendererTelemetryV1, RENDER_TELEMETRY_OVERHEAD_SCHEMA } from "../apps/web/render-telemetry-v1.mjs";

const ctx = {
  protocol_version: "chaptera.trace-context.v1",
  trace_id: "trace:telemetry-bench",
  interaction_id: "interaction:telemetry-bench",
  session_incarnation: "session:telemetry-bench",
  operation_class: "other",
  browser_family: "unknown",
};
const labels = {
  backend_family: "fake",
  browser_family: "unknown",
  dirty_class: "view",
  frame_outcome: "presented",
  workload_class: "stress",
  protocol_major: "v1",
  device_class: "ci",
};
const iterations = 20000;
const rows = {};

for (const mode of ["disabled", "counters", "full"]) {
  const obs = new BrowserObservabilityV1({
    sessionIncarnation: ctx.session_incarnation,
    browserFamily: "unknown",
    idFactory: (prefix) => prefix + ":benchmark-12345678",
    maxSpans: 256,
  });
  const telemetry = new RendererTelemetryV1({
    observability: obs,
    mode,
    realPub: false,
    representativeCorpus: false,
    maxStageSamples: 512,
  });
  const heapBefore = process.memoryUsage().heapUsed;
  const start = performance.now();
  for (let i = 0; i < iterations; i += 1) {
    telemetry.recordStage("frame", ctx, { durationMs: 0.1, labels });
    telemetry.recordCache("texture", "lookup");
    telemetry.recordUpload({ logicalBytes: 64, physicalBytes: 64 });
    telemetry.recordScheduler({ requestsReceived: 1, pending: i % 3 });
  }
  const elapsed = performance.now() - start;
  const heapAfter = process.memoryUsage().heapUsed;
  rows[mode] = {
    elapsed_ms: Math.round(elapsed * 1000) / 1000,
    ns_per_iteration: Math.round((elapsed * 1e6 / iterations) * 1000) / 1000,
    approximate_heap_delta_bytes: heapAfter - heapBefore,
    heap_measurement: "process_memory_usage_approximate_not_gc_normalized",
  };
}

const disabled = rows.disabled.ns_per_iteration;
for (const mode of ["counters", "full"]) {
  rows[mode].overhead_vs_disabled_ratio = disabled === 0 ? null : rows[mode].ns_per_iteration / disabled;
}

const receipt = {
  schema: RENDER_TELEMETRY_OVERHEAD_SCHEMA,
  iterations,
  real_pub: false,
  representative_corpus: false,
  technology_decision_allowed: false,
  modes: rows,
};
fs.mkdirSync("target/render-telemetry", { recursive: true });
fs.writeFileSync("target/render-telemetry/receipt.json", JSON.stringify(receipt, null, 2) + "\n");
console.log(JSON.stringify(receipt));
