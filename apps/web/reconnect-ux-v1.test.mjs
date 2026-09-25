import test from "node:test";
import assert from "node:assert/strict";
import { WebReconnectUxV1, bindBrowserNetworkHintsV1, deriveReconnectBannerV1 } from "./reconnect-ux-v1.mjs";

function facts(overrides = {}) {
  return {
    transport_state:"connected",
    recovery_state:"clean",
    editor_product_state:"saved",
    can_edit:true,
    pending_count:0,
    ...overrides,
  };
}

test("healthy canonical session hides banner",()=>{
  const b=deriveReconnectBannerV1(facts());
  assert.equal(b.visible,false);
  assert.equal(b.kind,"hidden");
});

test("sent_unknown is checking outcome and never offers blind retry",()=>{
  const b=deriveReconnectBannerV1(facts({transport_state:"disconnected",recovery_state:"sent_unknown",editor_product_state:"checking_save_status"}));
  assert.equal(b.kind,"checking_outcome");
  assert.equal(b.can_retry_connection,false);
  assert.equal(b.editing_fenced,true);
});

test("locally durable pending work says Not synced, not Saved or offline mode",()=>{
  const b=deriveReconnectBannerV1(facts({transport_state:"disconnected",recovery_state:"locally_durable",editor_product_state:"not_synced",pending_count:2}));
  assert.equal(b.kind,"not_synced");
  assert.match(b.message,/stored locally/i);
  assert.doesNotMatch(b.message,/offline mode/i);
});

test("storage failure pauses editing instead of claiming safe local recovery",()=>{
  const b=deriveReconnectBannerV1(facts({transport_state:"disconnected",recovery_state:"storage_failed",editor_product_state:"recovery_unavailable",can_edit:false}));
  assert.equal(b.kind,"recovery_unavailable");
  assert.equal(b.editing_fenced,true);
});

test("stale/quarantined state requires attention and never auto-rebases",()=>{
  const b=deriveReconnectBannerV1(facts({recovery_state:"stale",editor_product_state:"needs_attention",attention_code:"stale_base_revision"}));
  assert.equal(b.kind,"needs_attention");
  assert.equal(b.message,"stale_base_revision");
  assert.equal(b.can_retry_connection,false);
});

test("read-only authority dominates transport appearance",()=>{
  const b=deriveReconnectBannerV1(facts({transport_state:"connected",editor_product_state:"read_only",read_only_reason:"Writer session moved to another tab",can_edit:false}));
  assert.equal(b.kind,"read_only");
  assert.equal(b.editing_fenced,true);
});

test("browser online hint cannot erase authoritative reconnecting state",()=>{
  let current=facts({transport_state:"disconnected",recovery_state:"clean",editor_product_state:"preparing_editor",can_edit:false});
  const ux=new WebReconnectUxV1({
    statusProvider:{currentReconnectFacts(){return structuredClone(current);}},
    reconnectCommand:async()=>{},
  });
  ux.setBrowserNetworkHint("online");
  const state=ux.state();
  assert.equal(state.browser_network_hint,"online");
  assert.equal(state.banner.kind,"reconnecting");
});

test("retry calls only transport reconnect command and re-derives authority state",async()=>{
  let current=facts({transport_state:"disconnected",recovery_state:"clean",editor_product_state:"preparing_editor",can_edit:false});
  let calls=0;
  const ux=new WebReconnectUxV1({
    statusProvider:{currentReconnectFacts(){return structuredClone(current);}},
    reconnectCommand:async()=>{calls++; current=facts();},
  });
  const final=await ux.retryConnection();
  assert.equal(calls,1);
  assert.equal(final.banner.visible,false);
});

class Target {
  constructor(){this.h=new Map();}
  addEventListener(t,f){const a=this.h.get(t)||[];a.push(f);this.h.set(t,a);}
  removeEventListener(t,f){this.h.set(t,(this.h.get(t)||[]).filter(x=>x!==f));}
  emit(t){for(const f of this.h.get(t)||[])f({});}
}

test("online/offline browser events are stored only as non-authoritative hints",()=>{
  const target=new Target();
  const hints=[];
  const binding=bindBrowserNetworkHintsV1({controller:{setBrowserNetworkHint(v){hints.push(v);}},target});
  target.emit("offline");
  target.emit("online");
  assert.deepEqual(hints,["offline","online"]);
  binding.destroy();
});
