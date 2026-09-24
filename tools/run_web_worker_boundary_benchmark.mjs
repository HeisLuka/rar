#!/usr/bin/env node
import fs from "node:fs"; import { chromium, firefox } from "playwright";
const name=process.argv[2]||"chromium"; const browser=await ({chromium,firefox})[name].launch({headless:true});
try { const page=await browser.newPage(); const result=await page.evaluate(async()=>{
 const source=`onmessage=e=>{ const b=e.data.buffer; postMessage({bytes:b.byteLength,g:e.data.g}, e.data.transferBack?[b]:[]); }`;
 const url=URL.createObjectURL(new Blob([source],{type:"text/javascript"})); const w=new Worker(url);
 const one=async(size,transfer)=>new Promise((resolve,reject)=>{const b=new ArrayBuffer(size); const before=b.byteLength; const t0=performance.now(); w.onmessage=e=>resolve({elapsed_ms:performance.now()-t0,before,after:b.byteLength,reported:e.data.bytes}); w.onerror=reject; w.postMessage({buffer:b,g:1,transferBack:false},transfer?[b]:[]);});
 const clone=await one(1024*1024,false); const transfer=await one(1024*1024,true); w.terminate(); URL.revokeObjectURL(url);
 return {clone,transfer,transfer_detached:transfer.after===0,shared_array_buffer_available:typeof SharedArrayBuffer!=="undefined",cross_origin_isolated:self.crossOriginIsolated===true};
 });
 const receipt={schema:"chaptera.web-worker-boundary-benchmark.v1",browser:name,real_pub:false,representative:false,authority:"derived_browser_work_only",...result}; fs.mkdirSync("target/web-worker-boundary",{recursive:true}); fs.writeFileSync(`target/web-worker-boundary/${name}.json`,JSON.stringify(receipt,null,2)); console.log(JSON.stringify(receipt));
} finally { await browser.close(); }
