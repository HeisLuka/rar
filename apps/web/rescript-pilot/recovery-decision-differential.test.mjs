import test from "node:test";
import assert from "node:assert/strict";

import {
  BrowserRecoveryCoordinatorV1,
  MemoryRecoveryStoreV1,
} from "../recovery-v1.mjs";
import {planOne} from "./src/RecoveryDecision.res.mjs";

const REV1 = "sha256:" + "a".repeat(64);
const REV2 = "sha256:" + "b".repeat(64);

function coordinator() {
  return new BrowserRecoveryCoordinatorV1({
    store: new MemoryRecoveryStoreV1(),
    principalScopeId: "principal:1",
    documentId: "doc:1",
    clientSessionIncarnation: "tab-session:1",
    appVersion: "web-app:v1",
    commandSemanticVersion: "move-node:v1",
  });
}

async function prepared({dependsOn = null} = {}) {
  const c = coordinator();
  await c.prepareIntent({
    clientOperationId: "move-op-00000001",
    baseRevisionId: REV1,
    lifecycleGeneration: 7,
    dependsOnClientOperationId: dependsOn,
    normalizedIntent: {kind: "move_node_to", node_id: "node:1", x_emu: 10, y_emu: 20},
  });
  return c;
}

function res({
  state = "prepared",
  stateReason = "",
  principalMatches = true,
  authzAllowed = true,
  schemaMatches = true,
  appMatches = true,
  commandMatches = true,
  lifecycleMatches = true,
  currentRevisionMatches = true,
  lookupStatus = "not_found",
  lookupRevisionId = "",
  lookupCode = "",
  dependencyPresent = false,
  dependencyStatus = "not_found",
  dependencyCode = "",
} = {}) {
  return planOne(
    state,
    stateReason,
    principalMatches,
    authzAllowed,
    schemaMatches,
    appMatches,
    commandMatches,
    lifecycleMatches,
    currentRevisionMatches,
    lookupStatus,
    lookupRevisionId,
    lookupCode,
    dependencyPresent,
    dependencyStatus,
    dependencyCode,
  );
}

function bootstrap(overrides = {}) {
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

test("sent_unknown accepted resolves identically", async () => {
  const c = await prepared();
  await c.markSendStarted("move-op-00000001");
  const [js] = await c.planRecovery(bootstrap({
    current_revision_id: REV2,
    lookupOperationOutcome: async () => ({status: "accepted", revision_id: REV2}),
  }));
  const typed = res({
    state: "sent_unknown",
    currentRevisionMatches: false,
    lookupStatus: "accepted",
    lookupRevisionId: REV2,
  });
  assert.equal(typed.action, js.action);
  assert.equal(typed.revisionId, js.revision_id);
});

test("sent_unknown rejection surfaces identically", async () => {
  const c = await prepared();
  await c.markSendStarted("move-op-00000001");
  const [js] = await c.planRecovery(bootstrap({
    lookupOperationOutcome: async () => ({
      status: "rejected",
      code: "stale_revision",
      current_revision_id: REV2,
    }),
  }));
  const typed = res({
    state: "sent_unknown",
    lookupStatus: "rejected",
    lookupCode: "stale_revision",
  });
  assert.equal(typed.action, js.action);
  assert.equal(typed.reason, js.rejection.code);
});

test("stale prepared operation asks for refresh identically", async () => {
  const c = await prepared();
  const [js] = await c.planRecovery(bootstrap({current_revision_id: REV2}));
  const typed = res({currentRevisionMatches: false});
  assert.equal(typed.action, js.action);
  assert.equal(typed.reason, js.reason);
});

test("authz loss quarantines before retry", async () => {
  const c = await prepared();
  const [js] = await c.planRecovery(bootstrap({authz_allowed: false}));
  const typed = res({authzAllowed: false});
  assert.equal(typed.action, js.action);
  assert.equal(typed.reason, js.reason);
});

test("lifecycle mismatch quarantines identically", async () => {
  const c = await prepared();
  const [js] = await c.planRecovery(bootstrap({lifecycle_generation: 8}));
  const typed = res({lifecycleMatches: false});
  assert.equal(typed.action, js.action);
  assert.equal(typed.reason, js.reason);
});

test("unresolved dependency blocks identically", async () => {
  const c = await prepared({dependsOn: "move-op-00000000"});
  const [js] = await c.planRecovery(bootstrap({
    lookupOperationOutcome: async () => ({status: "not_found"}),
  }));
  const typed = res({dependencyPresent: true, dependencyStatus: "not_found"});
  assert.equal(typed.action, js.action);
});

test("rejected dependency blocks with the same dependency code", async () => {
  const c = await prepared({dependsOn: "move-op-00000000"});
  const [js] = await c.planRecovery(bootstrap({
    lookupOperationOutcome: async () => ({status: "rejected", code: "policy_denied"}),
  }));
  const typed = res({
    dependencyPresent: true,
    dependencyStatus: "rejected",
    dependencyCode: "policy_denied",
  });
  assert.equal(typed.action, js.action);
  assert.equal(typed.dependencyCode, js.dependency_code);
});

test("current prepared operation retries exact identity", async () => {
  const c = await prepared();
  const [js] = await c.planRecovery(bootstrap());
  const typed = res();
  assert.equal(typed.action, js.action);
});
