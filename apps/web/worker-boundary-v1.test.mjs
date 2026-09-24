import test from "node:test"; import assert from "node:assert/strict";
import {WorkerBoundaryV1, transferableArrayBuffer} from "./worker-boundary-v1.mjs";
const g=n=>({scene:n,view:n,resource:n,surface:n});
test("bounded queue drops instead of growing",()=>{const b=new WorkerBoundaryV1({maxInflight:2,maxBytes:100}); assert.ok(b.submit({generations:g(1),bytes:40})); assert.ok(b.submit({generations:g(1),bytes:40})); assert.equal(b.submit({generations:g(1),bytes:1}),null); assert.equal(b.snapshot().dropped,1);});
test("stale result cannot publish",()=>{const b=new WorkerBoundaryV1(); const j=b.submit({generations:g(1),bytes:10}); assert.equal(b.complete(j.id,g(2)).accepted,false); assert.equal(b.snapshot().stale,1);});
test("restart invalidates worker local generation without authority loss",()=>{const b=new WorkerBoundaryV1(); b.submit({generations:g(1),bytes:10}); const next=b.restart(); assert.equal(next,2); assert.equal(b.snapshot().inflight,0); assert.equal(b.snapshot().canonical_authority,"server_editor_session");});
test("transfer and clone accounting are separate",()=>{const b=new WorkerBoundaryV1(); b.submit({generations:g(1),bytes:100,transport:"transfer"}); b.submit({generations:g(1),bytes:20,transport:"clone"}); assert.equal(b.snapshot().bytes_transferred,100); assert.equal(b.snapshot().bytes_cloned,20);});
test("helper exposes transfer list",()=>{const x=transferableArrayBuffer(64); assert.equal(x.payload.byteLength,64); assert.equal(x.transfer[0],x.payload);});
