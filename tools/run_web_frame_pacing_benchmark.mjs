#!/usr/bin/env node
import fs from "node:fs";
import { chromium, firefox } from "playwright";
const engine=process.argv[2]||"chromium"; const browser=await ({chromium,firefox})[engine].launch({headless:true});
try{
 const page=await browser.newPage();
 const r=await page.evaluate(async()=>{
  const samples=[]; let scheduled=0,run=0,latest=0,pending=false;
  function invalidate(v){ latest=v; if(pending)return; pending=true; scheduled++; requestAnimationFrame(t=>{pending=false;run++;samples.push({t,latest});}); }
  for(let i=1;i<=1000;i++) invalidate(i);
  await new Promise(r=>setTimeout(r,50));
  const burst1={scheduled,run,latestPaint:samples.at(-1)?.latest};
  document.body.style.width="100px"; for(let i=1001;i<=1200;i++) invalidate(i);
  await new Promise(r=>setTimeout(r,50));
  return {burst1,final:{scheduled,run,latestPaint:samples.at(-1)?.latest},raf_available:typeof requestAnimationFrame==="function"};
 });
 const receipt={schema:"chaptera.web-frame-pacing-benchmark.v1",browser:engine,real_pub:false,representative:false,...r};
 fs.mkdirSync("target/web-frame-pacing",{recursive:true}); fs.writeFileSync(`target/web-frame-pacing/${engine}.json`,JSON.stringify(receipt,null,2)); console.log(JSON.stringify(receipt));
} finally {await browser.close();}
