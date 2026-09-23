import test from "node:test";
import assert from "node:assert/strict";

import {
  BrowserObservabilityV1,
  normalizeTraceContextV1,
  traceHeadersV1,
} from "./observability-v1.mjs";

function factory() {
  let n = 0;
  return (prefix) => prefix + ":unit-" + String(++n).padStart(4, "0");
}

test("trace context separates trace identity from semantic operation identity", () => {
  const obs = new BrowserObservabilityV1({
    sessionIncarnation: "session:unit-test",
    browserFamily: "chromium",
    idFactory: factory(),
  });
  const context = obs.createContext({
    operationClass: "commit",
    clientOperationId: "browser-op-12345678",
  });
  assert.notEqual(context.trace_id, context.client_operation_id);
  assert.equal(context.operation_class, "commit");
  const headers = traceHeadersV1(context);
  assert.equal(headers["x-chaptera-trace-id"], context.trace_id);
  assert.equal(headers["x-chaptera-operation-class"], "commit");
  assert.equal("document_id" in headers, false);
});

test("browser receipt contains bounded trace metadata but no document payload", () => {
  const obs = new BrowserObservabilityV1({
    sessionIncarnation: "session:unit-test",
    browserFamily: "firefox",
    idFactory: factory(),
  });
  const context = obs.createContext({ operationClass: "scene_read" });
  obs.mark("browser.scene_revision", context, { durationMs: 1.25 });
  const receipt = obs.receipt(context.trace_id);
  assert.equal(receipt.span_count, 1);
  assert.equal(receipt.contains_document_payload, false);
  assert.equal(receipt.semantic_authority, false);
  assert.equal("document_id" in receipt.spans[0], false);
  assert.equal("revision_id" in receipt.spans[0], false);
});

test("invalid unbounded context is rejected", () => {
  assert.throws(() => normalizeTraceContextV1({
    protocol_version: "chaptera.trace-context.v1",
    trace_id: "tiny",
    interaction_id: "interaction:12345678",
    session_incarnation: "session:12345678",
    operation_class: "commit",
    browser_family: "chromium",
  }));
});
