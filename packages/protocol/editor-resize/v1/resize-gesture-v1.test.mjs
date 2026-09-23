import test from "node:test";
import assert from "node:assert/strict";

import {
  RESIZE_HANDLES,
  ResizeTransactionV1,
  classifyResizePointerDown,
  hitTestResizeHandle,
  resizeHandleCenters,
} from "./resize-gesture-v1.mjs";

const BASE = { x: 100, y: 200, width: 400, height: 300 };
const START = { x_emu: 0, y_emu: 0 };
const DELTA = { x_emu: 20, y_emu: 30 };

const EXPECTED = {
  "top-left": { x: 120, y: 230, width: 380, height: 270 },
  top: { x: 100, y: 230, width: 400, height: 270 },
  "top-right": { x: 100, y: 230, width: 420, height: 270 },
  left: { x: 120, y: 200, width: 380, height: 300 },
  right: { x: 100, y: 200, width: 420, height: 300 },
  "bottom-left": { x: 120, y: 200, width: 380, height: 330 },
  bottom: { x: 100, y: 200, width: 400, height: 330 },
  "bottom-right": { x: 100, y: 200, width: 420, height: 330 },
};

test("all eight handles keep the opposite edge or corner fixed", () => {
  for (const handle of RESIZE_HANDLES) {
    const tx = new ResizeTransactionV1("node:test", BASE, handle, START);
    const result = tx.update(DELTA);
    assert.equal(result.kind, "preview", handle);
    assert.deepEqual(result.bounds, EXPECTED[handle], handle);
  }
});

test("pointer-down prioritizes resize handles before body move", () => {
  const screen = { x: 10, y: 20, width: 200, height: 100 };
  const centers = resizeHandleCenters(screen);

  for (const handle of RESIZE_HANDLES) {
    const center = centers[handle];
    assert.equal(
      hitTestResizeHandle(
        screen,
        { x_css_px: center.x, y_css_px: center.y },
        6,
      ),
      handle,
    );
    assert.deepEqual(
      classifyResizePointerDown(
        screen,
        { x_css_px: center.x, y_css_px: center.y },
        6,
      ),
      { kind: "resize_handle", handle },
    );
  }

  assert.deepEqual(
    classifyResizePointerDown(screen, { x_css_px: 110, y_css_px: 70 }, 6),
    { kind: "move_body" },
  );
  assert.deepEqual(
    classifyResizePointerDown(screen, { x_css_px: 500, y_css_px: 500 }, 6),
    { kind: "none" },
  );
});

test("pointer motion updates transient preview but never base bounds", () => {
  const tx = new ResizeTransactionV1("node:test", BASE, "bottom-right", START);
  const frozen = structuredClone(BASE);
  const first = tx.update({ x_emu: 10, y_emu: 20 });
  assert.equal(first.kind, "preview");
  assert.deepEqual(first.bounds, { x: 100, y: 200, width: 410, height: 320 });
  assert.deepEqual(tx.baseBounds(), frozen);

  const second = tx.update({ x_emu: -30, y_emu: 40 });
  assert.equal(second.kind, "preview");
  assert.deepEqual(second.bounds, { x: 100, y: 200, width: 370, height: 340 });
  assert.deepEqual(tx.baseBounds(), frozen);
});

test("invalid crossing preserves the last valid preview and emits no intent", () => {
  const tx = new ResizeTransactionV1("node:test", BASE, "left", START);
  assert.deepEqual(tx.update({ x_emu: 100, y_emu: 0 }), {
    kind: "preview",
    bounds: { x: 200, y: 200, width: 300, height: 300 },
  });

  const invalid = tx.update({ x_emu: 500, y_emu: 0 });
  assert.equal(invalid.kind, "invalid");
  assert.equal(invalid.reason, "non_positive_size");
  assert.deepEqual(invalid.bounds, { x: 200, y: 200, width: 300, height: 300 });
  assert.deepEqual(tx.previewBounds(), invalid.bounds);
});

test("overflow candidate fails closed and keeps the previous preview", () => {
  const nearMax = {
    x: Number.MAX_SAFE_INTEGER - 50,
    y: 0,
    width: 40,
    height: 40,
  };
  const tx = new ResizeTransactionV1("node:test", nearMax, "right", START);
  const invalid = tx.update({ x_emu: 20, y_emu: 0 });
  assert.equal(invalid.kind, "invalid");
  assert.equal(invalid.reason, "overflow");
  assert.deepEqual(invalid.bounds, nearMax);
});

test("release emits exactly one source-neutral resize intent", () => {
  const tx = new ResizeTransactionV1("node:test", BASE, "bottom-right", START);
  tx.update({ x_emu: 20, y_emu: 30 });
  const intent = tx.commit("op-1");

  assert.deepEqual(intent, {
    protocol_version: "chaptera.resize-intent.v1",
    client_operation_id: "op-1",
    node_id: "node:test",
    after_bounds: { x: 100, y: 200, width: 420, height: 330 },
  });
  assert.ok(!("before_bounds" in intent));
  assert.throws(() => tx.commit("op-2"), /must be active/);
});

test("cancel emits no commit and clears transient preview", () => {
  const tx = new ResizeTransactionV1("node:test", BASE, "bottom", START);
  tx.update({ x_emu: 0, y_emu: 30 });
  tx.cancel();
  assert.equal(tx.previewBounds(), null);
  assert.throws(() => tx.commit("op-cancelled"), /must be active/);
});

test("no-op or orthogonal-only pointer motion cannot commit a fake resize", () => {
  const noOp = new ResizeTransactionV1("node:test", BASE, "right", START);
  assert.throws(() => noOp.commit("op-noop"), /no size change/);

  const orthogonal = new ResizeTransactionV1("node:test", BASE, "right", START);
  orthogonal.update({ x_emu: 0, y_emu: 100 });
  assert.deepEqual(orthogonal.previewBounds(), BASE);
  assert.throws(() => orthogonal.commit("op-orthogonal"), /no size change/);
});


test("invalid terminal candidate cannot commit stale last-valid preview", () => {
  const tx = new ResizeTransactionV1("node:test", BASE, "left", START);
  const first = tx.update({ x_emu: 100, y_emu: 0 });
  assert.equal(first.kind, "preview");
  const invalid = tx.update({ x_emu: 500, y_emu: 0 });
  assert.equal(invalid.kind, "invalid");
  assert.throws(() => tx.commit("op-stale-preview"), /latest resize candidate is invalid/);
});
