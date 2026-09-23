import test from "node:test";
import assert from "node:assert/strict";

import {
  MAX_SAFE_EMU,
  MIN_SAFE_EMU,
  MoveGestureV1,
  TransientSelectionV1,
  assertSafeEmu,
  hitTestSnapshot,
  reconcileMoveCommit,
  roundHalfAwayFromZero,
  screenToDocumentPoint,
} from "./interaction-v1.mjs";

const DOC = "10000000-0000-4000-8000-000000000001";
const PAGE = "20000000-0000-4000-8000-000000000001";
const BACK = "30000000-0000-4000-8000-000000000001";
const FRONT = "30000000-0000-4000-8000-000000000002";
const SOURCE = "a".repeat(64);
const REV = "sha256:" + "b".repeat(64);

function snapshot(stacking_fidelity = "exact") {
  return {
    protocol_version: "chaptera.scene.v1",
    document_id: DOC,
    source_hash: SOURCE,
    revision_id: REV,
    snapshot_id: "sha256:" + "c".repeat(64),
    stacking_fidelity,
    nodes: [
      {
        node_id: BACK,
        page_id: PAGE,
        bounds: { x: 0, y: 0, width: 1000, height: 1000 },
        z_order: stacking_fidelity === "exact" ? 1 : null,
        paint_order: stacking_fidelity === "exact" ? 1 : null,
      },
      {
        node_id: FRONT,
        page_id: PAGE,
        bounds: { x: 100, y: 100, width: 500, height: 500 },
        z_order: stacking_fidelity === "exact" ? 2 : null,
        paint_order: stacking_fidelity === "exact" ? 2 : null,
      },
    ],
  };
}

test("safe EMU fence accepts JS exact integer endpoints", () => {
  assert.equal(assertSafeEmu(MAX_SAFE_EMU), MAX_SAFE_EMU);
  assert.equal(assertSafeEmu(MIN_SAFE_EMU), MIN_SAFE_EMU);
  assert.throws(() => assertSafeEmu(MAX_SAFE_EMU + 1), /safe integer/);
});

test("screen transform has explicit symmetric half-away rounding", () => {
  assert.equal(roundHalfAwayFromZero(1.5), 2);
  assert.equal(roundHalfAwayFromZero(-1.5), -2);

  const point = screenToDocumentPoint(
    { x_css_px: 10.5, y_css_px: 20.5 },
    {
      screen_origin_x_css_px: 0,
      screen_origin_y_css_px: 0,
      document_origin_x_emu: 100,
      document_origin_y_emu: -100,
      emu_per_css_px: 2,
    },
  );
  assert.deepEqual(point, { x_emu: 121, y_emu: -59 });
});

test("zoom/pan mapping never mutates canonical scene geometry", () => {
  const scene = snapshot();
  const before = structuredClone(scene.nodes);
  screenToDocumentPoint(
    { x_css_px: 50, y_css_px: 70 },
    {
      screen_origin_x_css_px: 10,
      screen_origin_y_css_px: 20,
      document_origin_x_emu: 1000,
      document_origin_y_emu: 2000,
      emu_per_css_px: 25,
    },
  );
  assert.deepEqual(scene.nodes, before);
});

test("exact stacking chooses deterministic frontmost canonical NodeId", () => {
  const hit = hitTestSnapshot(snapshot("exact"), PAGE, { x_emu: 200, y_emu: 200 });
  assert.deepEqual(hit, { kind: "hit", node_id: FRONT });
});

test("unknown stacking fails closed on overlap instead of inventing z-order", () => {
  const hit = hitTestSnapshot(snapshot("unknown"), PAGE, { x_emu: 200, y_emu: 200 });
  assert.equal(hit.kind, "ambiguous");
  assert.deepEqual(hit.node_ids, [BACK, FRONT]);
  assert.equal(hit.reason, "stacking_not_authoritative");
});

test("selection is transient and refuses durable JSON serialization", () => {
  const selection = new TransientSelectionV1();
  selection.select(FRONT);
  assert.equal(selection.nodeId, FRONT);
  assert.throws(() => JSON.stringify(selection), /not durable authoring state/);
  selection.clear();
  assert.equal(selection.nodeId, null);
});

test("pointer updates only transient preview and pointer-up emits one intent", () => {
  const scene = snapshot();
  const frozenBase = structuredClone(scene);
  const move = new MoveGestureV1(scene, FRONT, { x_emu: 150, y_emu: 150 });

  assert.deepEqual(move.update({ x_emu: 250, y_emu: 350 }), {
    x: 200,
    y: 300,
    width: 500,
    height: 500,
  });
  assert.deepEqual(scene, frozenBase);

  const request = move.commit("90000000-0000-4000-8000-000000000001");
  assert.equal(request.protocol_version, "chaptera.commit-request.v1");
  assert.equal(request.document_id, DOC);
  assert.equal(request.source_hash, SOURCE);
  assert.equal(request.base_revision_id, REV);
  assert.deepEqual(request.command, {
    kind: "move_node_to",
    node_id: FRONT,
    x_emu: 200,
    y_emu: 300,
  });
  assert.ok(!("before" in request.command));
  assert.throws(
    () => move.commit("90000000-0000-4000-8000-000000000002"),
    /must be active/,
  );
});

test("cancel creates no commit and clears preview", () => {
  const move = new MoveGestureV1(snapshot(), FRONT, { x_emu: 150, y_emu: 150 });
  move.update({ x_emu: 180, y_emu: 190 });
  move.cancel();
  assert.equal(move.previewBounds(), null);
  assert.throws(
    () => move.commit("90000000-0000-4000-8000-000000000003"),
    /must be active/,
  );
});

test("stale rejection restores base scene rather than auto-reapplying", () => {
  const scene = snapshot();
  const move = new MoveGestureV1(scene, FRONT, { x_emu: 150, y_emu: 150 });
  move.update({ x_emu: 250, y_emu: 350 });
  move.commit("90000000-0000-4000-8000-000000000004");

  const reconciled = reconcileMoveCommit(scene, move, {
    protocol_version: "chaptera.commit-rejected.v1",
    document_id: DOC,
    base_revision_id: REV,
    current_revision_id: "sha256:" + "d".repeat(64),
    client_operation_id: "90000000-0000-4000-8000-000000000004",
    code: "stale_revision",
    message_key: "revision.stale",
    retryable: true,
  });

  assert.equal(reconciled.kind, "rejected_restore_base_scene");
  assert.strictEqual(reconciled.display_snapshot, scene);
  assert.equal(reconciled.preview_bounds, null);
  assert.equal(reconciled.code, "stale_revision");
});
