#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TARGET = path.join(ROOT, "target", "web-scene-transport");

function uuid(prefix, n) {
  return prefix + "-0000-4000-8000-" + String(n).padStart(12, "0");
}
function makeScene(pageCount, nodesPerPage) {
  const pages=[]; const nodes=[]; let nodeCounter=0;
  for(let p=0;p<pageCount;p+=1){
    const pageId=uuid("10000000",p+1);
    pages.push({page_id:pageId,order:p,width_emu:7772400,height_emu:10058400});
    for(let n=0;n<nodesPerPage;n+=1){
      nodeCounter+=1;
      nodes.push({
        node_id:uuid("20000000",nodeCounter),page_id:pageId,parent_node_id:null,kind:"shape",
        bounds:{x:400000+(n%5)*1200000,y:400000+Math.floor(n/5)*1200000,width:800000,height:800000},
        z_order:n,paint_order:n,paint_id:null,resource_id:null,
        transform:{a:"1",b:"0",c:"0",d:"1",tx:0,ty:0}
      });
    }
  }
  return {
    protocol_version:"chaptera.scene.v1",
    document_id:"19999999-9999-4999-8999-999999999999",
    source_hash:"d".repeat(64),
    revision_id:"sha256:"+"d".repeat(64),
    snapshot_id:"sha256:"+"0".repeat(64),
    layout_environment:{
      environment_id:"sha256:"+"1".repeat(64),
      engine_revision:"synthetic-transport-compression-v1",
      font_set_fingerprint:"sha256:"+"2".repeat(64),
      resource_fingerprint:"sha256:"+"3".repeat(64)
    },
    stacking_fidelity:"exact",pages,nodes,stories:[],story_frames:[],paints:[],resources:[],diagnostics:[],
    capabilities:[{key:"render.geometry",state:"supported",note:"synthetic transport compression only"}],
    fidelity:{state:"partial",reasons:["synthetic_multipage_not_real_pub"]}
  };
}
function windowScene(full,center,radius=1){
  const lo=Math.max(0,center-radius), hi=Math.min(full.pages.length-1,center+radius);
  const pages=full.pages.filter(p=>p.order>=lo&&p.order<=hi);
  const ids=new Set(pages.map(p=>p.page_id));
  return {...full,pages,nodes:full.nodes.filter(n=>ids.has(n.page_id))};
}
function measure(obj){
  const raw=Buffer.from(JSON.stringify(obj));
  const gzip=zlib.gzipSync(raw,{level:9});
  const brotli=zlib.brotliCompressSync(raw,{params:{[zlib.constants.BROTLI_PARAM_QUALITY]:11}});
  return {
    raw_bytes:raw.length,
    gzip_bytes:gzip.length,
    brotli_bytes:brotli.length,
    gzip_ratio:raw.length/gzip.length,
    brotli_ratio:raw.length/brotli.length
  };
}

fs.mkdirSync(TARGET,{recursive:true});
const cases=[];
for(const pages of [10,100,500]){
  const full=makeScene(pages,10);
  const windowed=windowScene(full,Math.floor(pages/2),1);
  cases.push({
    source_pages:pages,
    nodes_per_page:10,
    full:measure(full),
    window3:measure(windowed)
  });
}
const receipt={
  receipt_kind:"chaptera.synthetic-scene-transport-compression.v1",
  real_pub:false,
  representative_corpus:false,
  transport_decision_allowed:false,
  cases,
  note:"Synthetic JSON compression baseline only. Useful to separate transfer-byte effects from browser working-set/render costs; not a production CDN/compression policy."
};
const out=path.join(TARGET,"receipt.json");
fs.writeFileSync(out,JSON.stringify(receipt,null,2)+"\n");
process.stdout.write(JSON.stringify(receipt,null,2)+"\n");
