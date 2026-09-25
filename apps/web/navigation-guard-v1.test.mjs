import test from "node:test";
import assert from "node:assert/strict";
import { WebNavigationGuardV1, bindBeforeUnloadGuardV1, evaluateNavigationSafetyV1 } from "./navigation-guard-v1.mjs";

function facts(overrides={}){
  return {
    editor_product_state:"saved",
    pending_intent_count:0,
    canonical_frontier_durable:true,
    local_recovery_durable:false,
    recovery_storage_available:true,
    unrecoverable_intent:false,
    ...overrides,
  };
}

test("saved/no-pending never produces nuisance warning",()=>{
  const d=evaluateNavigationSafetyV1(facts());
  assert.equal(d.warn,false);
  assert.equal(d.reason,"no_pending_intent");
});

test("locally durable pending work is safe to leave without claiming canonical save",()=>{
  const d=evaluateNavigationSafetyV1(facts({
    editor_product_state:"not_synced",pending_intent_count:2,canonical_frontier_durable:false,local_recovery_durable:true
  }));
  assert.equal(d.warn,false);
  assert.equal(d.reason,"local_recovery_durable");
});

test("recovery storage failure with pending intent warns",()=>{
  const d=evaluateNavigationSafetyV1(facts({
    editor_product_state:"recovery_unavailable",pending_intent_count:1,canonical_frontier_durable:false,local_recovery_durable:false,recovery_storage_available:false
  }));
  assert.equal(d.warn,true);
  assert.equal(d.reason,"recovery_storage_unavailable");
});

test("pending unknown work without durable local recovery warns",()=>{
  const d=evaluateNavigationSafetyV1(facts({
    editor_product_state:"checking_save_status",pending_intent_count:1,canonical_frontier_durable:false,local_recovery_durable:false
  }));
  assert.equal(d.warn,true);
  assert.equal(d.reason,"pending_intent_not_durably_recoverable");
});

test("read-only with zero pending intent is safe",()=>{
  const d=evaluateNavigationSafetyV1(facts({editor_product_state:"read_only",canonical_frontier_durable:false}));
  assert.equal(d.warn,false);
});

test("explicit unrecoverable intent always warns even if transport looks healthy",()=>{
  const d=evaluateNavigationSafetyV1(facts({pending_intent_count:1,canonical_frontier_durable:false,unrecoverable_intent:true}));
  assert.equal(d.warn,true);
  assert.equal(d.reason,"explicit_unrecoverable_intent");
});

class Target {
  constructor(){this.h=new Map();}
  addEventListener(t,f){const a=this.h.get(t)||[];a.push(f);this.h.set(t,a);}
  removeEventListener(t,f){this.h.set(t,(this.h.get(t)||[]).filter(x=>x!==f));}
  emit(t,e){for(const f of this.h.get(t)||[])f(e);}
}

test("beforeunload warns only from the shared guard predicate and does not attempt saving",()=>{
  const target=new Target();
  let saveCalls=0;
  let current=facts({pending_intent_count:1,canonical_frontier_durable:false,local_recovery_durable:false,recovery_storage_available:false});
  const guard=new WebNavigationGuardV1({statusProvider:{currentNavigationFacts(){return structuredClone(current);}}});
  const binding=bindBeforeUnloadGuardV1({guard,target});
  let prevented=false;
  const event={preventDefault(){prevented=true;},returnValue:null,sendBeacon(){saveCalls++;}};
  target.emit("beforeunload",event);
  assert.equal(prevented,true);
  assert.equal(event.returnValue,"");
  assert.equal(saveCalls,0);
  current=facts();
  prevented=false;
  event.returnValue=null;
  target.emit("beforeunload",event);
  assert.equal(prevented,false);
  assert.equal(event.returnValue,null);
  binding.destroy();
});

test("app router consumes the same safety decision",()=>{
  let current=facts({pending_intent_count:1,canonical_frontier_durable:false,local_recovery_durable:true});
  const guard=new WebNavigationGuardV1({statusProvider:{currentNavigationFacts(){return structuredClone(current);}}});
  assert.equal(guard.allowAppNavigation(),true);
  current=facts({pending_intent_count:1,canonical_frontier_durable:false,local_recovery_durable:false,recovery_storage_available:false});
  assert.equal(guard.allowAppNavigation(),false);
});
