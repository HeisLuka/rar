#!/usr/bin/env node
import fs from "node:fs";
import { performance } from "node:perf_hooks";
import { BrowserObservabilityV1 } from "../apps/web/observability-v1.mjs";
import { RendererTelemetryV1 } from "../apps/web/render-telemetry-v1.mjs";

const ctx = { protocol_version:"chaptera.trace-context.v1", trace_id:"trace:telemetry-bench", interaction_id:"interaction:telemetry-bench", session_incarnation:"session:telemetry-bench", operation_class:"other", browser_family:"unknown" };
const iterations = 20000;
const rows = {};
for (const mode of ["disabled","counters","full"]) {
  const obs = new BrowserObservabilityV1({sessionIncarnation:ctx.session_incarnation,browserFamily:"unknown",idFactory:(p)=>p+":benchmark-12345678",maxSpans:256});
  const t = new RendererTelemetryV1({observability:obs,mode});
  const start = performance.now();
  for(let i=0;i<iterations;i++) {
    t.recordStage("frame",ctx,{durationMs:0.1,labels:{backend_family:"fake",browser_family:"unknown",dirty_class:"view",frame_outcome:"presented",workload_class:"simple",protocol_major:"v1"}});
    t.recordCache("texture","lookup");
    t.recordUpload({logicalBytes:64,physicalBytes:64});
  }
  const elapsed = performance.now()-start;
  rows[mode] = { elapsed_ms: elapsed, ns_per_iteration: elapsed*1e6/iterations };
}
const receipt = { schema:"chaptera.renderer-telemetry-overhead.v1", iterations, real_pub:false, representative:false, modes:rows };
fs.mkdirSync("target/render-telemetry",{recursive:true}); fs.writeFileSync("target/render-telemetry/receipt.json",JSON.stringify(receipt,null,2)+"\n"); console.log(JSON.stringify(receipt));
