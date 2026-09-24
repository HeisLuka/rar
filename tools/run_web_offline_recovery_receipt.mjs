#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import {fileURLToPath} from "node:url";

import {
  BrowserRecoveryCoordinatorV1,
  MemoryRecoveryStoreV1,
  RecoveryStorageError,
} from "../apps/web/recovery-v1.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const OUT = path.join(ROOT, "target", "web-offline-recovery-v1", "logic-receipt.json");
const REV1 = "sha256:" + "a".repeat(64);
const REV2 = "sha256:" + "b".repeat(64);

function make(store = new MemoryRecoveryStoreV1(), overrides = {}) {
  return new BrowserRecoveryCoordinatorV1({
    store,
    principalScopeId: "principal:1",
    documentId: "doc:1",
    clientSessionIncarnation: "tab:A",
    appVersion: "web-app:v1",
    commandSemanticVersion: "move-node:v1",
    ...overrides,
  });
}

function intent(x = 10) {
  return {kind: "move_node_to", node_id: "node:1", x_emu: x, y_emu: 20};
}

function boot(overrides = {}) {
  return {
    principal_scope_id: "principal:1",
    current_revision_id: REV1,
    lifecycle_generation: 7,
    app_version: "web-app:v1",
    command_semantic_version: "move-node:v1",
    authz_allowed: true,
    lookupOperationOutcome: async () => ({status: "not_found"}),
    ...overrides,
  };
}

const svc = make();
const prepared = await svc.prepareIntent({
  clientOperationId: "move-op-00000001",
  baseRevisionId: REV1,
  lifecycleGeneration: 7,
  normalizedIntent: intent(),
});
const retry = await svc.prepareIntent({
  clientOperationId: "move-op-00000001",
  baseRevisionId: REV1,
  lifecycleGeneration: 7,
  normalizedIntent: intent(),
});
await svc.markSendStarted("move-op-00000001");
const unknownAccepted = await svc.planRecovery(boot({
  current_revision_id: REV2,
  lookupOperationOutcome: async () => ({status: "accepted", revision_id: REV2}),
}));

const staleSvc = make();
await staleSvc.prepareIntent({
  clientOperationId: "move-op-00000002",
  baseRevisionId: REV1,
  lifecycleGeneration: 7,
  normalizedIntent: intent(20),
});
const stalePlan = await staleSvc.planRecovery(boot({current_revision_id: REV2}));

const revokeSvc = make();
await revokeSvc.prepareIntent({
  clientOperationId: "move-op-00000003",
  baseRevisionId: REV1,
  lifecycleGeneration: 7,
  normalizedIntent: intent(30),
});
const revokePlan = await revokeSvc.planRecovery(boot({authz_allowed: false}));

const versionSvc = make();
await versionSvc.prepareIntent({
  clientOperationId: "move-op-00000004",
  baseRevisionId: REV1,
  lifecycleGeneration: 7,
  normalizedIntent: intent(40),
});
const versionPlan = await versionSvc.planRecovery(boot({
  command_semantic_version: "move-node:v2",
}));

let storageFailureSurfaced = false;
class FailingStore extends MemoryRecoveryStoreV1 {
  async put() { throw new Error("quota"); }
}
try {
  await make(new FailingStore()).prepareIntent({
    clientOperationId: "move-op-00000005",
    baseRevisionId: REV1,
    lifecycleGeneration: 7,
    normalizedIntent: intent(50),
  });
} catch (error) {
  storageFailureSurfaced = error instanceof RecoveryStorageError;
}

const receipt = {
  receipt_kind: "chaptera.web-local-recovery-v1.logic-contract",
  full_offline_authoring: false,
  canonical_browser_authority: false,
  invariants: {
    durable_prepare_is_idempotent_by_exact_operation_identity:
      prepared.client_operation_id === retry.client_operation_id,
    unknown_outcome_resolved_before_retry:
      unknownAccepted[0]?.action === "resolved_accepted" &&
      (await svc.listPending()).length === 0,
    stale_base_requires_refresh:
      stalePlan[0]?.action === "refresh_required",
    revoked_access_quarantines:
      revokePlan[0]?.action === "quarantined" &&
      revokePlan[0]?.reason === "authz_denied",
    semantic_version_mismatch_quarantines:
      versionPlan[0]?.action === "quarantined" &&
      versionPlan[0]?.reason === "command_semantic_version_mismatch",
    storage_failure_is_typed_and_surfaced: storageFailureSurfaced,
  },
  guardrail:
    "Logic/reference contract only. Browser IndexedDB persistence is measured separately; real canonical service reconnect acceptance remains downstream.",
};
if (!Object.values(receipt.invariants).every(Boolean)) throw new Error(JSON.stringify(receipt));
fs.mkdirSync(path.dirname(OUT), {recursive: true});
fs.writeFileSync(OUT, JSON.stringify(receipt, null, 2) + "\n");
process.stdout.write(JSON.stringify(receipt, null, 2) + "\n");
