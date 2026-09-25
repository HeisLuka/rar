import test from "node:test";
import assert from "node:assert/strict";
import { ResourceReadinessRuntimeV1 } from "./resource-readiness-v1.mjs";

const base = () => new ResourceReadinessRuntimeV1({
  document_id: "doc:1",
  revision_id: "rev:canonical-1",
  snapshot_id: "snapshot:1",
});
const image = { resource_id:"image:1", content_hash:"a".repeat(64), derivative_id:"full" };
const font = { resource_id:"font:1", content_hash:"b".repeat(64), derivative_id:"font-exact" };

test("pending image becomes ready under same revision and repaints dependent page", () => {
  const r=base();
  r.registerCompiledDependency({cache_entry_id:"page:p1:image",page_id:"p1",segment_id:"s1",consumer_kind:"image",...image});
  const token=r.beginRequest(image);
  assert.equal(r.resourceState(image).state,"fetching");
  r.markDecoding(token);
  const ready=r.completeReady(token,{binding_identity:"decoded:image:1:g1"});
  assert.equal(ready.accepted,true);
  assert.deepEqual(ready.invalidation.cache_entry_ids,["page:p1:image"]);
  assert.deepEqual(r.drainRepaintQueue(),["p1"]);
  assert.equal(r.receipt().canonical_revision_id,"rev:canonical-1");
  assert.equal(r.receipt().metrics.canonical_revisions_emitted,0);
  assert.equal(r.receipt().metrics.scene_patches_emitted,0);
});

test("fallback font is temporary visual, never exact fulfillment", () => {
  const r=base();
  r.registerCompiledDependency({cache_entry_id:"glyph:p1",page_id:"p1",segment_id:"text:1",consumer_kind:"glyph",...font});
  const token=r.beginRequest(font);
  const temp=r.setTemporaryVisual(token,"fallback_font");
  assert.equal(temp.fulfills_exact_resource,false);
  assert.equal(r.resourceState(font).state,"fetching");
  const ready=r.completeReady(token,{binding_identity:"glyph-cache:font:1:g1"});
  assert.equal(ready.state,"ready");
  assert.equal(r.resourceState(font).temporary_visual,null);
  assert.deepEqual(ready.invalidation.cache_entry_ids,["glyph:p1"]);
});

test("shared resource invalidates only registered dependents", () => {
  const r=base();
  for(const page of ["p1","p2","p3"]) r.registerCompiledDependency({
    cache_entry_id:`image:${page}`,page_id:page,segment_id:"img",consumer_kind:"image",...image
  });
  r.registerCompiledDependency({cache_entry_id:"unrelated",page_id:"p9",consumer_kind:"image",resource_id:"image:2",content_hash:"c".repeat(64),derivative_id:"full"});
  const token=r.beginRequest(image);
  const ready=r.completeReady(token,{binding_identity:"decoded"});
  assert.deepEqual(ready.invalidation.cache_entry_ids,["image:p1","image:p2","image:p3"]);
  assert.deepEqual(ready.invalidation.page_ids,["p1","p2","p3"]);
  assert.equal(r.invalidatedCacheEntries().includes("unrelated"),false);
});

test("stale completion cannot overwrite newer request generation", () => {
  const r=base();
  const oldToken=r.beginRequest(image);
  const newToken=r.beginRequest(image);
  const stale=r.completeReady(oldToken,{binding_identity:"old"});
  assert.deepEqual(stale,{accepted:false,reason:"stale_generation"});
  assert.equal(r.resourceState(image).state,"fetching");
  const current=r.completeReady(newToken,{binding_identity:"new"});
  assert.equal(current.accepted,true);
  assert.equal(r.resourceState(image).binding_identity,"new");
  assert.equal(r.receipt().metrics.stale_completions_rejected,1);
});

test("failed/blocked are explicit terminal states without auto retry loop", () => {
  const r=base();
  r.registerCompiledDependency({cache_entry_id:"img",page_id:"p1",consumer_kind:"image",...image});
  const t=r.beginRequest(image);
  const failed=r.completeTerminal(t,{state:"failed",reason_code:"decode_failed"});
  assert.equal(failed.auto_retry_scheduled,false);
  assert.equal(r.resourceState(image).state,"failed");
  assert.throws(()=>r.completeTerminal(t,{state:"failed",reason_code:"again"}),/requires fetching\/decoding/);

  const t2=r.beginRequest(font);
  const blocked=r.completeTerminal(t2,{state:"blocked",reason_code:"policy_blocked"});
  assert.equal(blocked.state,"blocked");
  assert.equal(blocked.auto_retry_scheduled,false);
});

test("eviction then reload reconstructs from immutable identity", () => {
  const r=base();
  r.registerCompiledDependency({cache_entry_id:"img",page_id:"p1",consumer_kind:"image",...image});
  let t=r.beginRequest(image);
  r.completeReady(t,{binding_identity:"decoded:g1"});
  const before=r.resourceState(image).material_generation;
  assert.equal(r.evict(image).evicted,true);
  assert.equal(r.resourceState(image).state,"evicted");
  t=r.beginRequest(image);
  r.completeReady(t,{binding_identity:"decoded:g2"});
  assert.equal(r.resourceState(image).state,"ready");
  assert.equal(r.resourceState(image).material_generation,before+1);
  assert.equal(r.receipt().canonical_revision_id,"rev:canonical-1");
});

test("backend reset invalidates residency without changing revision", () => {
  const r=base();
  r.registerCompiledDependency({cache_entry_id:"img",page_id:"p1",consumer_kind:"image",...image});
  const t=r.beginRequest(image); r.completeReady(t,{binding_identity:"gpu:1"});
  const reset=r.backendReset();
  assert.equal(reset.backend_generation,2);
  assert.deepEqual(reset.cache_entry_ids,["img"]);
  assert.equal(r.resourceState(image).state,"evicted");
  assert.equal(r.receipt().canonical_revision_id,"rev:canonical-1");
  const t2=r.beginRequest(image); r.completeReady(t2,{binding_identity:"gpu:2"});
  assert.equal(r.resourceState(image).state,"ready");
});

test("derivatives of same ResourceId do not alias readiness", () => {
  const r=base();
  const preview={...image,derivative_id:"preview"};
  r.registerCompiledDependency({cache_entry_id:"full",page_id:"p1",consumer_kind:"image",...image});
  r.registerCompiledDependency({cache_entry_id:"preview",page_id:"p1",consumer_kind:"image",...preview});
  const t=r.beginRequest(image); const ready=r.completeReady(t,{binding_identity:"full"});
  assert.deepEqual(ready.invalidation.cache_entry_ids,["full"]);
  assert.equal(r.resourceState(preview).state,"unrequested");
});

test("readiness runtime emits zero authoring operations and revisions", () => {
  const r=base();
  const t=r.beginRequest(image); r.markDecoding(t); r.completeReady(t,{binding_identity:"decoded"});
  const receipt=r.receipt();
  assert.equal(receipt.authority.runtime_only,true);
  assert.equal(receipt.authority.authoring_operations_emitted,0);
  assert.equal(receipt.authority.scene_patches_emitted,0);
  assert.equal(receipt.authority.canonical_revisions_emitted,0);
});
