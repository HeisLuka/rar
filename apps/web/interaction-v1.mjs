export const MAX_SAFE_EMU = Number.MAX_SAFE_INTEGER;
export const MIN_SAFE_EMU = Number.MIN_SAFE_INTEGER;

export function assertSafeEmu(value, label = "EMU") {
  if (!Number.isSafeInteger(value)) {
    throw new RangeError(`${label} must be a JavaScript-safe integer`);
  }
  return value;
}

function assertFiniteNumber(value, label) {
  if (!Number.isFinite(value)) {
    throw new TypeError(`${label} must be finite`);
  }
  return value;
}

export function roundHalfAwayFromZero(value) {
  assertFiniteNumber(value, "round input");
  if (value === 0) return 0;
  return Math.sign(value) * Math.floor(Math.abs(value) + 0.5);
}

export function screenToDocumentPoint(screen, view) {
  const sx = assertFiniteNumber(screen.x_css_px, "screen.x_css_px");
  const sy = assertFiniteNumber(screen.y_css_px, "screen.y_css_px");
  const ox = assertFiniteNumber(view.screen_origin_x_css_px, "view.screen_origin_x_css_px");
  const oy = assertFiniteNumber(view.screen_origin_y_css_px, "view.screen_origin_y_css_px");
  const scale = assertFiniteNumber(view.emu_per_css_px, "view.emu_per_css_px");
  if (!(scale > 0)) throw new RangeError("view.emu_per_css_px must be > 0");

  const docOriginX = assertSafeEmu(view.document_origin_x_emu, "view.document_origin_x_emu");
  const docOriginY = assertSafeEmu(view.document_origin_y_emu, "view.document_origin_y_emu");

  const x = docOriginX + roundHalfAwayFromZero((sx - ox) * scale);
  const y = docOriginY + roundHalfAwayFromZero((sy - oy) * scale);
  return {
    x_emu: assertSafeEmu(x, "document x"),
    y_emu: assertSafeEmu(y, "document y"),
  };
}

function safeAdd(a, b, label) {
  return assertSafeEmu(assertSafeEmu(a, label) + assertSafeEmu(b, label), label);
}

function containsPoint(bounds, point) {
  const right = safeAdd(bounds.x, bounds.width, "bounds right");
  const bottom = safeAdd(bounds.y, bounds.height, "bounds bottom");
  return (
    point.x_emu >= bounds.x &&
    point.x_emu < right &&
    point.y_emu >= bounds.y &&
    point.y_emu < bottom
  );
}

function compareFrontmost(a, b) {
  // Higher z/paint order is treated as visually frontmost when stacking fidelity is exact.
  if (a.z_order !== b.z_order) return a.z_order < b.z_order ? 1 : -1;
  if (a.paint_order !== b.paint_order) return a.paint_order < b.paint_order ? 1 : -1;
  return a.node_id.localeCompare(b.node_id);
}

export function hitTestSnapshot(snapshot, pageId, point) {
  assertSafeEmu(point.x_emu, "hit point x");
  assertSafeEmu(point.y_emu, "hit point y");

  const candidates = snapshot.nodes.filter(
    (node) => node.page_id === pageId && containsPoint(node.bounds, point),
  );

  if (candidates.length === 0) return { kind: "none" };
  if (candidates.length === 1) {
    return { kind: "hit", node_id: candidates[0].node_id };
  }

  if (
    snapshot.stacking_fidelity !== "exact" ||
    candidates.some(
      (node) => !Number.isInteger(node.z_order) || !Number.isInteger(node.paint_order),
    )
  ) {
    return {
      kind: "ambiguous",
      node_ids: candidates.map((node) => node.node_id).sort(),
      reason: "stacking_not_authoritative",
    };
  }

  const ordered = [...candidates].sort(compareFrontmost);
  return { kind: "hit", node_id: ordered[0].node_id };
}

export class TransientSelectionV1 {
  #nodeId = null;

  get nodeId() {
    return this.#nodeId;
  }

  select(nodeId) {
    this.#nodeId = nodeId;
  }

  clear() {
    this.#nodeId = null;
  }

  toJSON() {
    throw new Error("transient selection is not durable authoring state");
  }
}

export class MoveGestureV1 {
  #snapshot;
  #nodeId;
  #baseBounds;
  #offsetX;
  #offsetY;
  #preview;
  #state = "active";

  constructor(snapshot, nodeId, pointerDocumentPoint) {
    const node = snapshot.nodes.find((item) => item.node_id === nodeId);
    if (!node) throw new Error("cannot begin move for unknown canonical NodeId");

    assertSafeEmu(pointerDocumentPoint.x_emu, "pointer x");
    assertSafeEmu(pointerDocumentPoint.y_emu, "pointer y");

    this.#snapshot = snapshot;
    this.#nodeId = nodeId;
    this.#baseBounds = { ...node.bounds };
    this.#offsetX = assertSafeEmu(
      pointerDocumentPoint.x_emu - node.bounds.x,
      "move pointer offset x",
    );
    this.#offsetY = assertSafeEmu(
      pointerDocumentPoint.y_emu - node.bounds.y,
      "move pointer offset y",
    );
    this.#preview = { ...node.bounds };
  }

  get state() {
    return this.#state;
  }

  get nodeId() {
    return this.#nodeId;
  }

  previewBounds() {
    return this.#state === "cancelled" ? null : { ...this.#preview };
  }

  update(pointerDocumentPoint) {
    if (this.#state !== "active") {
      throw new Error("move gesture is no longer active");
    }
    const x = assertSafeEmu(
      assertSafeEmu(pointerDocumentPoint.x_emu, "pointer x") - this.#offsetX,
      "preview x",
    );
    const y = assertSafeEmu(
      assertSafeEmu(pointerDocumentPoint.y_emu, "pointer y") - this.#offsetY,
      "preview y",
    );
    this.#preview = {
      x,
      y,
      width: this.#baseBounds.width,
      height: this.#baseBounds.height,
    };
    return this.previewBounds();
  }

  cancel() {
    if (this.#state === "committed") {
      throw new Error("cannot cancel a committed move");
    }
    this.#state = "cancelled";
  }

  commit(clientOperationId) {
    if (this.#state !== "active") {
      throw new Error("move gesture must be active to commit");
    }
    if (typeof clientOperationId !== "string" || clientOperationId.length === 0) {
      throw new TypeError("client_operation_id is required");
    }
    this.#state = "committed";
    return {
      protocol_version: "chaptera.commit-request.v1",
      document_id: this.#snapshot.document_id,
      source_hash: this.#snapshot.source_hash,
      base_revision_id: this.#snapshot.revision_id,
      client_operation_id: clientOperationId,
      command: {
        kind: "move_node_to",
        node_id: this.#nodeId,
        x_emu: this.#preview.x,
        y_emu: this.#preview.y,
      },
    };
  }
}

export function reconcileMoveCommit(snapshot, gesture, result) {
  if (result.protocol_version === "chaptera.commit-accepted.v1") {
    if (result.document_id !== snapshot.document_id) {
      throw new Error("accepted commit document identity mismatch");
    }
    return {
      kind: "accepted_waiting_for_scene",
      display_snapshot: snapshot,
      preview_bounds: gesture.previewBounds(),
      target_revision_id: result.revision_id,
    };
  }

  if (result.protocol_version === "chaptera.commit-rejected.v1") {
    return {
      kind: "rejected_restore_base_scene",
      display_snapshot: snapshot,
      preview_bounds: null,
      code: result.code,
      current_revision_id: result.current_revision_id ?? null,
    };
  }

  throw new Error("unknown commit result protocol");
}
