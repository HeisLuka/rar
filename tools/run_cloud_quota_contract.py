#!/usr/bin/env python3
import json,pathlib,sys
ROOT=pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/"services"/"cloud-quota"))
from quota_v1 import CloudQuotaV1,QuotaRejected,QuotaConflict
OUT=ROOT/"target"/"cloud-quota-v1"/"receipt.json"
q=CloudQuotaV1(shared_capacity=10,semantic_headroom=5,export_cap=4,background_cap=4)
e=q.reserve(tenant_id="t1",reservation_id="e1",work_class="export",amount=4)
b=q.reserve(tenant_id="t1",reservation_id="b1",work_class="background",amount=4)
background_blocked=False
try:q.reserve(tenant_id="t1",reservation_id="b2",work_class="background",amount=1)
except QuotaRejected as x: background_blocked=str(x)=="background_budget_paused"
i=q.reserve(tenant_id="t1",reservation_id="i1",work_class="interactive",amount=7)
semantic_blocked=False
try:q.reserve(tenant_id="t1",reservation_id="i2",work_class="interactive",amount=1)
except QuotaRejected as x: semantic_blocked=str(x)=="semantic_headroom_exhausted"
renewed=q.renew_lease(tenant_id="t1",reservation_id="e1",expected_lease_generation=e["lease_generation"])
stale_expiry_fenced=False
try:q.expire_stale(tenant_id="t1",reservation_id="e1",observed_lease_generation=e["lease_generation"])
except QuotaConflict as x: stale_expiry_fenced=str(x)=="lease_advanced"
q.expire_stale(tenant_id="t1",reservation_id="e1",observed_lease_generation=renewed["lease_generation"])
r={"receipt_kind":"chaptera.cloud-quota-v1.contract","real_service":False,"product_acceptance":False,"invariants":{
"background_degrades_before_semantic":background_blocked,
"interactive_uses_protected_headroom":q.usage("t1")["protected_interactive"]==5,
"semantic_exhaustion_rejected":semantic_blocked,
"stale_expiry_fenced":stale_expiry_fenced,
"expired_reservation_released":q.usage("t1")["export"]==0
},"guardrail":"Public atomic reservation reference only; numeric production budgets/provider primitives not measured."}
assert all(r["invariants"].values()),r
OUT.parent.mkdir(parents=True,exist_ok=True);OUT.write_text(json.dumps(r,indent=2)+"\n");print(json.dumps(r,indent=2))
