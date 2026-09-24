import fs from "node:fs";
import { performance } from "node:perf_hooks";
import { SpatialWindowV1, identityKey } from "../apps/web/spatial-lifecycle-v1.mjs";

function identity(totalPages){
  return {document_id:"doc:bench",revision_id:"rev:1",layout_environment_id:"env:1",window_id:"w:bench",window_generation:1,
    page_ids:["p49","p50","p51"],shard_fingerprints:["s49","s50","s51"],total_document_pages:totalPages};
}
function nodes(){
  const out=[];
  for(let p=49;p<=51;p++) for(let i=0;i<2000;i++) out.push({
    node_id:`p${p}:n${i}`,page_id:`p${p}`,
    bounds:{x:(i%50)*10000-20000,y:Math.floor(i/50)*10000,width:8000,height:8000},paint_order:i,z_order:i
  });
  return out;
}
function run(totalPages){
  const s=new SpatialWindowV1({cellSizeEmu:50000,maxQueryCells:4096});
  const data=nodes(); const t0=performance.now(); s.replaceWindow(identity(totalPages),data); const buildMs=performance.now()-t0;
  const g=identityKey(identity(totalPages)); const q={x:100000,y:100000,width:200000,height:200000};
  const samples=[]; let last;
  for(let i=0;i<1000;i++){ const t=performance.now(); last=s.queryBox("p50",q,g); samples.push(performance.now()-t); }
  const patchNode={node_id:"p50:n10",page_id:"p50",bounds:{x:900000,y:900000,width:8000,height:8000},paint_order:10,z_order:10};
  const next={...identity(totalPages),revision_id:"rev:2",window_generation:2,shard_fingerprints:["s49","s50b","s51"]};
  const p0=performance.now(); s.applyNodePatch({base_generation_key:g,next_identity:next,upsert_nodes:[patchNode]}); const patchMs=performance.now()-p0;
  samples.sort((a,b)=>a-b);
  return {total_document_pages:totalPages,indexed_window_nodes:data.length,build_ms:buildMs,query_p50_ms:samples[Math.floor(samples.length/2)],
    query_stats:last.stats,patch_ms:patchMs,receipt:s.receipt()};
}
const a=run(100),b=run(500);
const receipt={schema:"chaptera.interaction-spatial-lifecycle-benchmark.v1",measurement_class:"synthetic_equal_three_page_window",
  real_pub:false,representative:false,cases:[a,b],
  operation_cost_equal:a.query_stats.cells_visited===b.query_stats.cells_visited && a.query_stats.candidate_checks===b.query_stats.candidate_checks,
  limitations:["Timing is Node/CI host behavior; deterministic operation counts are the document-length independence evidence."]};
fs.mkdirSync("target/spatial-lifecycle",{recursive:true}); fs.writeFileSync("target/spatial-lifecycle/receipt.json",JSON.stringify(receipt,null,2)+"\n");
console.log(JSON.stringify(receipt));
