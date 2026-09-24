import test from "node:test";
import assert from "node:assert/strict";

import {
  BrowserTextSessionV1,
  diffObservedTextToScalarReplacement,
  scalarIndexToUtf16Offset,
  utf16OffsetToScalarIndex,
  validateStoryRangeIntentV1,
} from "./text-protocol-v1.mjs";

const sourceHash = "a".repeat(64);
const revision = "sha256:" + "b".repeat(64);

function ids() {
  let n = 0;
  return () => "text-op-" + String(++n).padStart(8, "0");
}

test("UTF-16 offsets convert explicitly to Unicode scalar indices", () => {
  const text = "A😀B";
  assert.equal(text.length, 4);
  assert.equal(Array.from(text).length, 3);
  assert.equal(utf16OffsetToScalarIndex(text, 0), 0);
  assert.equal(utf16OffsetToScalarIndex(text, 1), 1);
  assert.equal(utf16OffsetToScalarIndex(text, 3), 2);
  assert.equal(utf16OffsetToScalarIndex(text, 4), 3);
  assert.throws(() => utf16OffsetToScalarIndex(text, 2), /surrogate pair/);
  assert.equal(scalarIndexToUtf16Offset(text, 2), 3);
});

test("observed browser replacement normalizes to canonical scalar range", () => {
  const before = "A👨‍👩‍👧‍👦B";
  const after = "AXB";
  const replacement = diffObservedTextToScalarReplacement(before, after);
  assert.deepEqual(replacement, {
    start_scalar: 1,
    end_scalar: 8,
    replacement_text: "X",
  });
});

test("composition updates remain transient and composition end emits one intent", () => {
  const session = new BrowserTextSessionV1({
    documentId: "doc:1",
    sourceHash,
    storyId: "story:1",
    baseRevisionId: revision,
    text: "AB",
    operationIdFactory: ids(),
  });
  session.beginComposition();
  assert.equal(session.updateComposition("A漢B"), null);
  assert.equal(session.pending.length, 0);
  const request = session.endComposition("A漢B");
  assert.equal(session.pending.length, 1);
  assert.equal(request.command.start_scalar, 1);
  assert.equal(request.command.end_scalar, 1);
  assert.equal(request.command.replacement_text, "漢");
});

test("rapid local edits form explicit causal dependency chain", () => {
  const session = new BrowserTextSessionV1({
    documentId: "doc:1",
    sourceHash,
    storyId: "story:1",
    baseRevisionId: revision,
    text: "abc",
    operationIdFactory: ids(),
  });
  const a = session.createIntent({start_scalar: 3, end_scalar: 3, replacement_text: "d"});
  const b = session.createIntent({start_scalar: 0, end_scalar: 1, replacement_text: "A"});
  assert.equal(a.depends_on_client_operation_id, null);
  assert.equal(b.depends_on_client_operation_id, a.client_operation_id);
  assert.equal(a.base_revision_id, revision);
  assert.equal(b.base_revision_id, revision);
});

test("acceptance advances revision only in causal order", () => {
  const session = new BrowserTextSessionV1({
    documentId: "doc:1",
    sourceHash,
    storyId: "story:1",
    baseRevisionId: revision,
    text: "abc",
    operationIdFactory: ids(),
  });
  const a = session.createIntent({start_scalar: 3, end_scalar: 3, replacement_text: "d"});
  const b = session.createIntent({start_scalar: 0, end_scalar: 0, replacement_text: "X"});
  assert.throws(() => session.accept(b.client_operation_id, {
    revision_id: "sha256:" + "c".repeat(64),
    canonical_story_text: "Xabcd",
  }), /causal order/);
  const result = session.accept(a.client_operation_id, {
    revision_id: "sha256:" + "c".repeat(64),
    canonical_story_text: "abcd",
  });
  assert.equal(result.remaining_pending, 1);
  assert.equal(session.baseRevisionId, "sha256:" + "c".repeat(64));
});

test("rejection invalidates dependent pending chain and requires refresh", () => {
  const session = new BrowserTextSessionV1({
    documentId: "doc:1",
    sourceHash,
    storyId: "story:1",
    baseRevisionId: revision,
    text: "abc",
    operationIdFactory: ids(),
  });
  const a = session.createIntent({start_scalar: 1, end_scalar: 1, replacement_text: "X"});
  const b = session.createIntent({start_scalar: 2, end_scalar: 2, replacement_text: "Y"});
  const result = session.reject(a.client_operation_id, {
    code: "stale_revision",
    current_revision_id: "sha256:" + "d".repeat(64),
  });
  assert.equal(result.requires_refresh, true);
  assert.deepEqual(result.invalidated_client_operation_ids, [a.client_operation_id, b.client_operation_id]);
  assert.equal(session.pending.length, 0);
});

test("wire intent carries no DOM ranges or browser layout authority", () => {
  const session = new BrowserTextSessionV1({
    documentId: "doc:1",
    sourceHash,
    storyId: "story:1",
    baseRevisionId: revision,
    text: "hello",
    operationIdFactory: ids(),
  });
  const request = session.createIntent({start_scalar: 0, end_scalar: 5, replacement_text: "hi"});
  const json = JSON.stringify(request);
  for (const forbidden of ["dom", "selectionStart", "clientWidth", "line_break", "overflow", "glyph"]) {
    assert.equal(json.includes(forbidden), false, forbidden);
  }
  assert.doesNotThrow(() => validateStoryRangeIntentV1(request));
});
