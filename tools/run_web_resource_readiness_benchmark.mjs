import fs from "node:fs";
import { performance } from "node:perf_hooks";
import { ResourceReadinessRuntimeV1 } from "../apps/web/resource-readiness-v1.mjs";

function run(dependentCount) {
  const runtime=new ResourceReadinessRuntimeV1({document_id:"doc:bench",revision_id:"rev:1",snapshot_id:"snap:1"});
  const identity={resource_id:"image:shared",content_hash:"a".repeat(64),derivative_id:"full"};
  for(let i=0;i<dependentCount;i++) runtime.registerCompiledDependency({
    cache_entry_id:`cache:${i}`,page_id:`p:${Math.floor(i/100)}`,segment_id:`s:${i}`,consumer_kind:"image",...identity
  });
  for(let i=0;i<5000;i++) runtime.registerCompiledDependency({
    cache_entry_id:`other:${i}`,page_id:`other:${Math.floor(i/100)}`,consumer_kind:"image",
    resource_id:"image:other",content_hash:"b".repeat(64),derivative_id:"full"
  });
  const token=runtime.beginRequest(identity);
  const t0=performance.now();
  const ready=runtime.completeReady(token,{binding_identity:"decoded:bench"});
  const readyToVisibleMs=performance.now()-t0;
  return {
    dependent_count:dependentCount,
    unrelated_dependency_count:5000,
    invalidated_count:ready.invalidation.cache_entry_ids.length,
    repaint_page_count:ready.invalidation.page_ids.length,
    ready_to_invalidation_ms:readyToVisibleMs,
    canonical_revision_id:runtime.receipt().canonical_revision_id,
    authoring_operations_emitted:runtime.receipt().authority.authoring_operations_emitted,
  };
}
const receipt={
  schema:"chaptera.web-resource-readiness-benchmark.v1",
  measurement_class:"synthetic_reverse_dependency_invalidation",
  real_pub:false,
  representative:false,
  cases:[run(10),run(100),run(1000)],
  limitations:["Node/CI timing is not browser fetch/decode latency; invalidated counts are the bounded dependency evidence."]
};
fs.mkdirSync("target/web-resource-readiness",{recursive:true});
fs.writeFileSync("target/web-resource-readiness/receipt.json",JSON.stringify(receipt,null,2)+"\n");
console.log(JSON.stringify(receipt));
