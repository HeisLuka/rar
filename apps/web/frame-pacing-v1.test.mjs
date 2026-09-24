import test from "node:test";
import assert from "node:assert/strict";
import { createFramePacer } from "./frame-pacing-v1.mjs";

function fakeClock() {
  let next=1; const q=new Map();
  return {
    request(cb){ const id=next++; q.set(id,cb); return id; },
    cancel(id){ q.delete(id); },
    async tick(ts=16){ const batch=[...q.entries()]; q.clear(); for(const [,cb] of batch) await cb(ts); },
    pending(){ return q.size; }
  };
}
const state=(n)=>({generations:{scene:n,view:n,overlay:n,resource:n,surface:n,renderer:n},payload:{n}});

test("100 invalidations coalesce to one callback and latest state", async()=>{
 const c=fakeClock(); const frames=[];
 const p=createFramePacer({requestFrame:c.request,cancelFrame:c.cancel,onFrame:async f=>frames.push(f)});
 for(let i=1;i<=100;i++) p.invalidate(["view"],state(i));
 assert.equal(c.pending(),1); await c.tick();
 assert.equal(frames.length,1); assert.equal(frames[0].payload.n,100); assert.equal(p.stats().coalesced,99);
});

test("mixed dirties merge", async()=>{
 const c=fakeClock(); const frames=[];
 const p=createFramePacer({requestFrame:c.request,onFrame:async f=>frames.push(f)});
 p.invalidate(["scene"],state(1)); p.invalidate(["resource","overlay"],state(2)); await c.tick();
 assert.deepEqual(frames[0].dirty,["overlay","resource","scene"]);
});

test("events during render create at most one follow-up", async()=>{
 const c=fakeClock(); const frames=[]; let p;
 p=createFramePacer({requestFrame:c.request,onFrame:async f=>{frames.push(f); if(frames.length===1){for(let i=0;i<20;i++) p.invalidate(["overlay"],state(2));}}});
 p.invalidate(["view"],state(1)); await c.tick(); assert.equal(c.pending(),1); await c.tick(); assert.equal(frames.length,2);
});

test("hidden resume paints latest only", async()=>{
 const c=fakeClock(); const frames=[]; const p=createFramePacer({requestFrame:c.request,cancelFrame:c.cancel,onFrame:async f=>frames.push(f)});
 p.setHidden(true); for(let i=1;i<=5;i++) p.invalidate(["scene"],state(i)); assert.equal(c.pending(),0);
 p.setHidden(false); assert.equal(c.pending(),1); await c.tick(); assert.equal(frames[0].payload.n,5);
});

test("stale async completion is rejected including surface generation",()=>{
 const c=fakeClock(); const p=createFramePacer({requestFrame:c.request,onFrame:async()=>{}});
 assert.equal(p.acceptAsyncCompletion({generations:state(1).generations},state(2).generations),false);
 assert.equal(p.acceptAsyncCompletion({generations:state(2).generations},state(2).generations),true);
});
