import test from "node:test";
import assert from "node:assert/strict";

import {PendingChainV1, PendingRejected} from "../pending-chain-v1.mjs";
import {
  isDispatchable,
  observeCanonicalAdvance,
  recordOutcome,
} from "./src/PendingState.res.mjs";

const R0 = "sha256:" + "0".repeat(64);
const R1 = "sha256:" + "1".repeat(64);
const R2 = "sha256:" + "2".repeat(64);

function chain(session = "session-pilot") {
  const value = new PendingChainV1({
    canonicalRevisionId: R0,
    sessionIncarnation: session,
  });
  value.enqueue({
    clientOperationId: "operation-A",
    intent: {kind: "MoveNodeTo", node_id: "node-1", x_emu: 10, y_emu: 20},
  });
  return value;
}

test("accepted head transition matches current PendingChainV1", () => {
  const js = chain("session-accepted");
  js.recordOutcome("operation-A", {status: "accepted", revision_id: R1});

  const res = recordOutcome("pending", "", R0, "accepted", R1, "");
  assert.equal(res.state, "accepted");
  assert.equal(res.canonicalRevisionId, js.snapshot().canonical_revision_id);
  assert.equal(res.removeHead, true);
  assert.equal(js.snapshot().pending.length, 0);
});

test("rejected head becomes blocked with the same reason", () => {
  const js = chain("session-rejected");
  js.recordOutcome("operation-A", {status: "rejected", code: "policy_denied"});

  const res = recordOutcome("pending", "", R0, "rejected", "", "policy_denied");
  const [entry] = js.snapshot().pending;
  assert.equal(res.state, entry.state);
  assert.equal(res.blockedReason, entry.blocked_reason);
  assert.equal(res.canonicalRevisionId, js.snapshot().canonical_revision_id);
  assert.equal(res.removeHead, false);
});

test("conflict head becomes blocked with the same reason", () => {
  const js = chain("session-conflict");
  js.recordOutcome("operation-A", {
    status: "conflict",
    code: "stale_base",
    current_revision_id: R2,
  });

  const res = recordOutcome("pending", "", R0, "conflict", "", "stale_base");
  const [entry] = js.snapshot().pending;
  assert.equal(res.state, entry.state);
  assert.equal(res.blockedReason, entry.blocked_reason);
  assert.equal(res.removeHead, false);
});

test("remote canonical advance produces the same reresolution state", () => {
  const js = chain("session-advance");
  js.observeCanonicalAdvance(R2);

  const res = observeCanonicalAdvance("pending", "", R0, R2);
  const [entry] = js.snapshot().pending;
  assert.equal(res.state, entry.state);
  assert.equal(res.blockedReason, entry.blocked_reason);
  assert.equal(res.canonicalRevisionId, js.snapshot().canonical_revision_id);
});

test("dispatchability agrees for pending, blocked and reresolution states", () => {
  const pending = chain("session-dispatch-pending");
  assert.doesNotThrow(() => pending.requestFor("operation-A"));
  assert.equal(isDispatchable("pending", ""), true);

  const blocked = chain("session-dispatch-blocked");
  blocked.recordOutcome("operation-A", {status: "rejected", code: "denied"});
  assert.throws(() => blocked.requestFor("operation-A"), PendingRejected);
  assert.equal(isDispatchable("blocked", "denied"), false);

  const stale = chain("session-dispatch-stale");
  stale.observeCanonicalAdvance(R2);
  assert.throws(() => stale.requestFor("operation-A"), PendingRejected);
  assert.equal(
    isDispatchable("reresolution_required", "canonical_advanced_before_predecessor"),
    false,
  );
});
