import test from "node:test";
import assert from "node:assert/strict";

import {
  BrowserRecoveryCoordinatorV1,
  MemoryRecoveryStoreV1,
  RecoveryConflict,
  RecoveryStorageError,
} from "./recovery-v1.mjs";

const REV1 = "sha256:" + "a".repeat(64);
const REV2 = "sha256:" + "b".repeat(64);

function coordinator(store = new MemoryRecoveryStoreV1(), overrides = {}) {
  return new BrowserRecoveryCoordinatorV1({
    store,
    principalScopeId: "principal:1",
    documentId: "doc:1",
    clientSessionIncarnation: "tab-session:1",
    appVersion: "web-app:v1",
    commandSemanticVersion: "move-node:v1",
    ...overrides,
  });
}

function intent(x = 10) {
  return {kind: "move_node_to", node_id: "node:1", x_emu: x, y_emu: 20};
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

test("prepare is durable-before-send and idempotent by exact operation identity", async () => {
  const store = new MemoryRecoveryStoreV1();
  const c = coordinator(store);
  const first = await c.prepareIntent({
    clientOperationId: "move-op-00000001",
    baseRevisionId: REV1,
    lifecycleGeneration: 7,
    normalizedIntent: intent(),
  });
  const retry = await c.prepareIntent({
    clientOperationId: "move-op-00000001",
    baseRevisionId: REV1,
    lifecycleGeneration: 7,
    normalizedIntent: intent(),
  });
  assert.deepEqual(first, retry);
  assert.equal((await c.listPending()).length, 1);
  assert.equal(first.state, "prepared");
});

test("same operation id with changed semantic request fails closed", async () => {
  const c = coordinator();
  await c.prepareIntent({
    clientOperationId: "move-op-00000001",
    baseRevisionId: REV1,
    lifecycleGeneration: 7,
    normalizedIntent: intent(10),
  });
  await assert.rejects(
    c.prepareIntent({
      clientOperationId: "move-op-00000001",
      baseRevisionId: REV1,
      lifecycleGeneration: 7,
      normalizedIntent: intent(11),
    }),
    RecoveryConflict,
  );
});

test("local storage write failure is surfaced and recovery is not claimed", async () => {
  class FailingStore extends MemoryRecoveryStoreV1 {
    async put() {
      throw new Error("quota");
    }
  }
  const c = coordinator(new FailingStore());
  await assert.rejects(
    c.prepareIntent({
      clientOperationId: "move-op-00000001",
      baseRevisionId: REV1,
      lifecycleGeneration: 7,
      normalizedIntent: intent(),
    }),
    RecoveryStorageError,
  );
});

test("unknown send outcome is resolved by operation identity before retry", async () => {
  const c = coordinator();
  await c.prepareIntent({
    clientOperationId: "move-op-00000001",
    baseRevisionId: REV1,
    lifecycleGeneration: 7,
    normalizedIntent: intent(),
  });
  await c.markSendStarted("move-op-00000001");
  const actions = await c.planRecovery(bootstrap({
    current_revision_id: REV2,
    lookupOperationOutcome: async (opId) => {
      assert.equal(opId, "move-op-00000001");
      return {status: "accepted", revision_id: REV2};
    },
  }));
  assert.deepEqual(actions, [{
    client_operation_id: "move-op-00000001",
    action: "resolved_accepted",
    revision_id: REV2,
  }]);
  assert.equal((await c.listPending()).length, 0);
});

test("unknown outcome not found may retry only exact identity on current base", async () => {
  const c = coordinator();
  await c.prepareIntent({
    clientOperationId: "move-op-00000001",
    baseRevisionId: REV1,
    lifecycleGeneration: 7,
    normalizedIntent: intent(),
  });
  await c.markSendStarted("move-op-00000001");
  const [action] = await c.planRecovery(bootstrap());
  assert.equal(action.action, "retry_exact_identity");
  assert.equal(action.request.client_operation_id, "move-op-00000001");
  assert.equal(action.request.base_revision_id, REV1);
  assert.deepEqual(action.request.normalized_intent, intent());
});

test("stale base never auto-replays or auto-merges", async () => {
  const c = coordinator();
  await c.prepareIntent({
    clientOperationId: "move-op-00000001",
    baseRevisionId: REV1,
    lifecycleGeneration: 7,
    normalizedIntent: intent(),
  });
  const [action] = await c.planRecovery(bootstrap({current_revision_id: REV2}));
  assert.equal(action.action, "refresh_required");
  assert.equal(action.reason, "stale_base_revision");
  assert.equal((await c.listPending()).length, 1);
});

test("revoked access quarantines pending work instead of replaying it", async () => {
  const c = coordinator();
  await c.prepareIntent({
    clientOperationId: "move-op-00000001",
    baseRevisionId: REV1,
    lifecycleGeneration: 7,
    normalizedIntent: intent(),
  });
  const [action] = await c.planRecovery(bootstrap({authz_allowed: false}));
  assert.equal(action.action, "quarantined");
  assert.equal(action.reason, "authz_denied");
  const [record] = await c.listPending();
  assert.equal(record.state, "quarantined");
});

test("semantic-version and lifecycle boundaries quarantine old records", async () => {
  const c1 = coordinator();
  await c1.prepareIntent({
    clientOperationId: "move-op-00000001",
    baseRevisionId: REV1,
    lifecycleGeneration: 7,
    normalizedIntent: intent(),
  });
  const [versionAction] = await c1.planRecovery(bootstrap({
    command_semantic_version: "move-node:v2",
  }));
  assert.equal(versionAction.reason, "command_semantic_version_mismatch");

  const c2 = coordinator();
  await c2.prepareIntent({
    clientOperationId: "move-op-00000002",
    baseRevisionId: REV1,
    lifecycleGeneration: 7,
    normalizedIntent: intent(),
  });
  const [lifecycleAction] = await c2.planRecovery(bootstrap({
    lifecycle_generation: 8,
  }));
  assert.equal(lifecycleAction.reason, "lifecycle_generation_mismatch");
});

test("server rejection is retained for explicit user-visible recovery", async () => {
  const c = coordinator();
  await c.prepareIntent({
    clientOperationId: "move-op-00000001",
    baseRevisionId: REV1,
    lifecycleGeneration: 7,
    normalizedIntent: intent(),
  });
  await c.markSendStarted("move-op-00000001");
  const [action] = await c.planRecovery(bootstrap({
    lookupOperationOutcome: async () => ({
      status: "rejected",
      code: "stale_revision",
      current_revision_id: REV2,
    }),
  }));
  assert.equal(action.action, "surface_rejection");
  assert.equal(action.rejection.code, "stale_revision");
  const [record] = await c.listPending();
  assert.equal(record.state, "rejected");
});

test("dependent operation does not leapfrog an unresolved predecessor", async () => {
  const c = coordinator();
  await c.prepareIntent({
    clientOperationId: "move-op-00000002",
    baseRevisionId: REV1,
    lifecycleGeneration: 7,
    dependsOnClientOperationId: "move-op-00000001",
    normalizedIntent: intent(30),
  });
  const [action] = await c.planRecovery(bootstrap({
    lookupOperationOutcome: async (opId) => {
      assert.equal(opId, "move-op-00000001");
      return {status: "not_found"};
    },
  }));
  assert.equal(action.action, "blocked_dependency");
});

test("accepted ACK removes local pending state", async () => {
  const c = coordinator();
  await c.prepareIntent({
    clientOperationId: "move-op-00000001",
    baseRevisionId: REV1,
    lifecycleGeneration: 7,
    normalizedIntent: intent(),
  });
  const accepted = await c.ackAccepted("move-op-00000001", {revision_id: REV2});
  assert.equal(accepted.pending_removed, true);
  assert.equal((await c.listPending()).length, 0);
});

test("tabs stay independent sessions while sharing one browser-local recovery store", async () => {
  const store = new MemoryRecoveryStoreV1();
  const a = coordinator(store, {clientSessionIncarnation: "tab-session:A"});
  const b = coordinator(store, {clientSessionIncarnation: "tab-session:B"});
  await a.prepareIntent({
    clientOperationId: "move-op-00000001",
    baseRevisionId: REV1,
    lifecycleGeneration: 7,
    normalizedIntent: intent(10),
  });
  await b.prepareIntent({
    clientOperationId: "move-op-00000002",
    baseRevisionId: REV1,
    lifecycleGeneration: 7,
    normalizedIntent: intent(20),
  });
  const rows = await a.listPending();
  assert.equal(rows.length, 2);
  assert.deepEqual(
    rows.map((row) => row.client_session_incarnation),
    ["tab-session:A", "tab-session:B"],
  );
});
