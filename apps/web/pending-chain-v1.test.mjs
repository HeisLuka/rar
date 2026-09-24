import test from "node:test";
import assert from "node:assert/strict";

import {
  PendingBackpressure,
  PendingChainV1,
  PendingIdentityConflict,
  PendingRejected,
} from "./pending-chain-v1.mjs";

const R0 = "sha256:" + "0".repeat(64);
const R1 = "sha256:" + "1".repeat(64);
const R2 = "sha256:" + "2".repeat(64);

function move(node, x) {
  return {kind: "MoveNodeTo", node_id: node, x_emu: x, y_emu: 200};
}

function story(start, end, text) {
  return {kind: "ReplaceStoryRange", story_id: "story-1", start_scalar: start, end_scalar: end, text};
}

test("A→B→C can be enqueued without waiting one RTT", () => {
  for (const rtt of [20, 100, 300]) {
    let now = 1000;
    const chain = new PendingChainV1({
      canonicalRevisionId: R0,
      sessionIncarnation: "session-1",
      now: () => now,
    });
    const a = chain.enqueue({clientOperationId: "operation-A", intent: move("node-1", 100)});
    now += rtt / 10;
    const b = chain.enqueue({clientOperationId: "operation-B", intent: move("node-1", 110)});
    const c = chain.enqueue({clientOperationId: "operation-C", intent: story(3, 3, "x")});
    assert.deepEqual(a.causal_base, {kind: "canonical_revision", revision_id: R0});
    assert.deepEqual(b.causal_base, {kind: "pending_after", client_operation_id: "operation-A"});
    assert.deepEqual(c.causal_base, {kind: "pending_after", client_operation_id: "operation-B"});
    assert.deepEqual(chain.snapshot().pending.map((x) => x.client_sequence), [1, 2, 3]);
  }
});

test("1/5/20 pending operations preserve exact causal depth and sequence", () => {
  for (const count of [1, 5, 20]) {
    const chain = new PendingChainV1({
      canonicalRevisionId: R0,
      sessionIncarnation: "session-" + count,
      limits: {max_count: 20, max_causal_depth: 20},
    });
    for (let i = 0; i < count; i += 1) {
      chain.enqueue({clientOperationId: "operation-" + String(i).padStart(3, "0"), intent: move("node-1", i)});
    }
    const snap = chain.snapshot();
    assert.equal(snap.totals.count, count);
    assert.equal(snap.totals.causal_depth, count);
    assert.deepEqual(snap.pending.map((x) => x.client_sequence), Array.from({length: count}, (_, i) => i + 1));
  }
});

test("accepted A with lost/reordered ACK preserves queued B/C identities", () => {
  const chain = new PendingChainV1({canonicalRevisionId: R0, sessionIncarnation: "session-ack"});
  chain.enqueue({clientOperationId: "operation-A", intent: move("node-1", 1)});
  chain.enqueue({clientOperationId: "operation-B", intent: move("node-1", 2)});
  chain.enqueue({clientOperationId: "operation-C", intent: move("node-1", 3)});
  const bBefore = chain.requestFor("operation-B");

  chain.recordOutcome("operation-A", {status: "accepted", revision_id: R1});
  assert.deepEqual(chain.requestFor("operation-B"), bBefore);
  assert.equal(chain.snapshot().canonical_revision_id, R1);
  assert.deepEqual(chain.snapshot().pending.map((x) => x.client_operation_id), ["operation-B", "operation-C"]);
});

test("duplicate enqueue and duplicate ACK are exact-idempotent; mismatches fail closed", () => {
  const chain = new PendingChainV1({canonicalRevisionId: R0, sessionIncarnation: "session-idem"});
  const first = chain.enqueue({clientOperationId: "operation-A", intent: move("node-1", 1)});
  assert.deepEqual(chain.enqueue({clientOperationId: "operation-A", intent: move("node-1", 1)}), first);
  assert.throws(
    () => chain.enqueue({clientOperationId: "operation-A", intent: move("node-1", 9)}),
    PendingIdentityConflict
  );

  chain.recordOutcome("operation-A", {status: "accepted", revision_id: R1});
  assert.throws(
    () => chain.recordOutcome("operation-A", {status: "accepted", revision_id: R1}),
    PendingRejected
  );
});

test("remote canonical advancement never silently rebases pending commands", () => {
  const chain = new PendingChainV1({canonicalRevisionId: R0, sessionIncarnation: "session-remote"});
  chain.enqueue({clientOperationId: "operation-A", intent: move("node-1", 1)});
  chain.enqueue({clientOperationId: "operation-B", intent: move("node-1", 2)});
  chain.observeCanonicalAdvance(R2);
  const [a, b] = chain.snapshot().pending;
  assert.equal(a.state, "reresolution_required");
  assert.equal(a.blocked_reason, "canonical_advanced_before_predecessor");
  assert.equal(b.state, "blocked");
  assert.throws(() => chain.requestFor("operation-A"), PendingRejected);
  assert.throws(() => chain.requestFor("operation-B"), PendingRejected);
});

test("predecessor conflict blocks dependents rather than auto-applying them", () => {
  const chain = new PendingChainV1({canonicalRevisionId: R0, sessionIncarnation: "session-conflict"});
  chain.enqueue({clientOperationId: "operation-A", intent: move("node-1", 1)});
  chain.enqueue({clientOperationId: "operation-B", intent: story(4, 4, "Q")});
  chain.enqueue({clientOperationId: "operation-C", intent: story(5, 5, "R")});
  chain.recordOutcome("operation-A", {status: "conflict", code: "stale_base", current_revision_id: R2});
  assert.deepEqual(chain.snapshot().pending.map((x) => x.state), ["blocked", "blocked", "blocked"]);
  assert.equal(chain.snapshot().pending[1].blocked_reason, "predecessor_conflict");
});

test("reconnect restore reconstructs exact pending causal chain", () => {
  const chain = new PendingChainV1({canonicalRevisionId: R0, sessionIncarnation: "session-reconnect"});
  chain.enqueue({clientOperationId: "operation-A", intent: move("node-1", 1)});
  chain.enqueue({clientOperationId: "operation-B", intent: story(10, 10, "hello")});
  const snapshot = chain.snapshot();
  const restored = PendingChainV1.restore(snapshot);
  assert.deepEqual(restored.snapshot(), snapshot);
  assert.deepEqual(restored.requestFor("operation-B").intent, story(10, 10, "hello"));
});

test("count, bytes, age and causal depth enforce explicit backpressure", () => {
  const count = new PendingChainV1({
    canonicalRevisionId: R0,
    sessionIncarnation: "session-count",
    limits: {max_count: 1},
  });
  count.enqueue({clientOperationId: "operation-A", intent: move("n", 1)});
  assert.throws(() => count.enqueue({clientOperationId: "operation-B", intent: move("n", 2)}), PendingBackpressure);

  const bytes = new PendingChainV1({
    canonicalRevisionId: R0,
    sessionIncarnation: "session-bytes",
    limits: {max_bytes: 300},
  });
  assert.throws(
    () => bytes.enqueue({clientOperationId: "operation-A", intent: {kind: "Text", text: "x".repeat(1000)}}),
    PendingBackpressure
  );

  const depth = new PendingChainV1({
    canonicalRevisionId: R0,
    sessionIncarnation: "session-depth",
    limits: {max_count: 20, max_causal_depth: 1},
  });
  depth.enqueue({clientOperationId: "operation-A", intent: move("n", 1)});
  assert.throws(() => depth.enqueue({clientOperationId: "operation-B", intent: move("n", 2)}), PendingBackpressure);

  let now = 0;
  const age = new PendingChainV1({
    canonicalRevisionId: R0,
    sessionIncarnation: "session-age",
    limits: {max_age_ms: 10},
    now: () => now,
  });
  age.enqueue({clientOperationId: "operation-A", intent: move("n", 1)});
  now = 11;
  assert.throws(() => age.enqueue({clientOperationId: "operation-B", intent: move("n", 2)}), PendingBackpressure);
});

test("predicted Story scalar intent is preserved verbatim behind predecessor", () => {
  const chain = new PendingChainV1({canonicalRevisionId: R0, sessionIncarnation: "session-story"});
  chain.enqueue({clientOperationId: "operation-A", intent: story(4, 4, "AB")});
  const b = chain.enqueue({clientOperationId: "operation-B", intent: story(6, 6, "C")});
  assert.deepEqual(b.causal_base, {kind: "pending_after", client_operation_id: "operation-A"});
  assert.deepEqual(b.intent, story(6, 6, "C"));
});

test("causal model does not invent transport batching, IME or Undo semantics", () => {
  const chain = new PendingChainV1({canonicalRevisionId: R0, sessionIncarnation: "session-boundary"});
  const request = chain.enqueue({clientOperationId: "operation-A", intent: move("node-1", 10)});
  assert.ok(!("batch_window_ms" in request));
  assert.ok(!("undo_group" in request));
  assert.ok(!("composition_session" in request));
});
