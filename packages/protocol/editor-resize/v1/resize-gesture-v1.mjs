export const RESIZE_HANDLES = Object.freeze([
  "top-left",
  "top",
  "top-right",
  "left",
  "right",
  "bottom-left",
  "bottom",
  "bottom-right",
]);

const HANDLE_SET = new Set(RESIZE_HANDLES);
const HANDLE_HIT_ORDER = Object.freeze([
  "top-left",
  "top-right",
  "bottom-left",
  "bottom-right",
  "top",
  "left",
  "right",
  "bottom",
]);

function assertFinite(value, label) {
  if (!Number.isFinite(value)) {
    throw new TypeError(`${label} must be finite`);
  }
  return value;
}

function assertSafeInteger(value, label) {
  if (!Number.isSafeInteger(value)) {
    throw new RangeError(`${label} must be a JavaScript-safe integer`);
  }
  return value;
}

function safeAdd(a, b, label) {
  return assertSafeInteger(
    assertSafeInteger(a, label) + assertSafeInteger(b, label),
    label,
  );
}

function safeSub(a, b, label) {
  return assertSafeInteger(
    assertSafeInteger(a, label) - assertSafeInteger(b, label),
    label,
  );
}

function assertDocumentPoint(point, label = "pointer") {
  if (!point || typeof point !== "object") {
    throw new TypeError(`${label} must be an object`);
  }
  return {
    x_emu: assertSafeInteger(point.x_emu, `${label}.x_emu`),
    y_emu: assertSafeInteger(point.y_emu, `${label}.y_emu`),
  };
}

export function assertResizeRect(rect, label = "bounds") {
  if (!rect || typeof rect !== "object") {
    throw new TypeError(`${label} must be an object`);
  }
  const normalized = {
    x: assertSafeInteger(rect.x, `${label}.x`),
    y: assertSafeInteger(rect.y, `${label}.y`),
    width: assertSafeInteger(rect.width, `${label}.width`),
    height: assertSafeInteger(rect.height, `${label}.height`),
  };
  if (normalized.width <= 0 || normalized.height <= 0) {
    throw new RangeError(`${label} width/height must be > 0`);
  }
  safeAdd(normalized.x, normalized.width, `${label}.right`);
  safeAdd(normalized.y, normalized.height, `${label}.bottom`);
  return normalized;
}

function cloneRect(rect) {
  return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
}

function sameRect(a, b) {
  return (
    a.x === b.x &&
    a.y === b.y &&
    a.width === b.width &&
    a.height === b.height
  );
}

function handleUses(handle, edge) {
  if (handle === edge) return true;
  return handle.split("-").includes(edge);
}

function screenRectEdges(bounds) {
  if (!bounds || typeof bounds !== "object") {
    throw new TypeError("screen bounds must be an object");
  }
  const x = assertFinite(bounds.x, "screen bounds.x");
  const y = assertFinite(bounds.y, "screen bounds.y");
  const width = assertFinite(bounds.width, "screen bounds.width");
  const height = assertFinite(bounds.height, "screen bounds.height");
  if (!(width > 0) || !(height > 0)) {
    throw new RangeError("screen bounds width/height must be > 0");
  }
  return {
    left: x,
    top: y,
    right: assertFinite(x + width, "screen bounds.right"),
    bottom: assertFinite(y + height, "screen bounds.bottom"),
  };
}

export function resizeHandleCenters(screenBounds) {
  const edges = screenRectEdges(screenBounds);
  const midX = (edges.left + edges.right) / 2;
  const midY = (edges.top + edges.bottom) / 2;
  return {
    "top-left": { x: edges.left, y: edges.top },
    top: { x: midX, y: edges.top },
    "top-right": { x: edges.right, y: edges.top },
    left: { x: edges.left, y: midY },
    right: { x: edges.right, y: midY },
    "bottom-left": { x: edges.left, y: edges.bottom },
    bottom: { x: midX, y: edges.bottom },
    "bottom-right": { x: edges.right, y: edges.bottom },
  };
}

export function hitTestResizeHandle(screenBounds, pointer, radiusCssPx = 6) {
  const px = assertFinite(pointer?.x_css_px, "pointer.x_css_px");
  const py = assertFinite(pointer?.y_css_px, "pointer.y_css_px");
  const radius = assertFinite(radiusCssPx, "radiusCssPx");
  if (!(radius > 0)) throw new RangeError("radiusCssPx must be > 0");

  const centers = resizeHandleCenters(screenBounds);
  const radiusSquared = radius * radius;
  for (const handle of HANDLE_HIT_ORDER) {
    const center = centers[handle];
    const dx = px - center.x;
    const dy = py - center.y;
    if (dx * dx + dy * dy <= radiusSquared) {
      return handle;
    }
  }
  return null;
}

export function classifyResizePointerDown(screenBounds, pointer, radiusCssPx = 6) {
  const handle = hitTestResizeHandle(screenBounds, pointer, radiusCssPx);
  if (handle) {
    return { kind: "resize_handle", handle };
  }

  const edges = screenRectEdges(screenBounds);
  const px = assertFinite(pointer?.x_css_px, "pointer.x_css_px");
  const py = assertFinite(pointer?.y_css_px, "pointer.y_css_px");
  if (px >= edges.left && px <= edges.right && py >= edges.top && py <= edges.bottom) {
    return { kind: "move_body" };
  }
  return { kind: "none" };
}

function resizeCandidate(base, handle, dx, dy) {
  let left = base.x;
  let top = base.y;
  let right = safeAdd(base.x, base.width, "base.right");
  let bottom = safeAdd(base.y, base.height, "base.bottom");

  if (handleUses(handle, "left")) {
    left = safeAdd(base.x, dx, "candidate.left");
  }
  if (handleUses(handle, "right")) {
    right = safeAdd(right, dx, "candidate.right");
  }
  if (handleUses(handle, "top")) {
    top = safeAdd(base.y, dy, "candidate.top");
  }
  if (handleUses(handle, "bottom")) {
    bottom = safeAdd(bottom, dy, "candidate.bottom");
  }

  const width = safeSub(right, left, "candidate.width");
  const height = safeSub(bottom, top, "candidate.height");
  if (width <= 0 || height <= 0) {
    throw new RangeError("candidate width/height must be > 0");
  }

  return assertResizeRect({ x: left, y: top, width, height }, "candidate");
}

export class ResizeTransactionV1 {
  #nodeId;
  #handle;
  #base;
  #startPointer;
  #preview;
  #lastUpdateValid = true;
  #state = "active";

  constructor(nodeId, beforeBounds, handle, pointerDocumentPoint) {
    if (typeof nodeId !== "string" || nodeId.length === 0) {
      throw new TypeError("nodeId is required");
    }
    if (!HANDLE_SET.has(handle)) {
      throw new RangeError("unknown resize handle");
    }

    this.#nodeId = nodeId;
    this.#handle = handle;
    this.#base = assertResizeRect(beforeBounds, "beforeBounds");
    this.#startPointer = assertDocumentPoint(pointerDocumentPoint, "startPointer");
    this.#preview = cloneRect(this.#base);
  }

  get state() {
    return this.#state;
  }

  get nodeId() {
    return this.#nodeId;
  }

  get handle() {
    return this.#handle;
  }

  baseBounds() {
    return cloneRect(this.#base);
  }

  previewBounds() {
    return this.#state === "cancelled" ? null : cloneRect(this.#preview);
  }

  update(pointerDocumentPoint) {
    if (this.#state !== "active") {
      throw new Error("resize transaction is no longer active");
    }
    const pointer = assertDocumentPoint(pointerDocumentPoint);
    let dx;
    let dy;
    try {
      dx = safeSub(pointer.x_emu, this.#startPointer.x_emu, "pointer.dx");
      dy = safeSub(pointer.y_emu, this.#startPointer.y_emu, "pointer.dy");
      const candidate = resizeCandidate(this.#base, this.#handle, dx, dy);
      this.#preview = candidate;
      this.#lastUpdateValid = true;
      return { kind: "preview", bounds: cloneRect(candidate) };
    } catch (error) {
      if (!(error instanceof RangeError)) throw error;
      this.#lastUpdateValid = false;
      return {
        kind: "invalid",
        reason: error.message.includes("> 0") ? "non_positive_size" : "overflow",
        bounds: cloneRect(this.#preview),
      };
    }
  }

  cancel() {
    if (this.#state === "committed") {
      throw new Error("cannot cancel a committed resize transaction");
    }
    this.#state = "cancelled";
  }

  commit(clientOperationId) {
    if (this.#state !== "active") {
      throw new Error("resize transaction must be active to commit");
    }
    if (!this.#lastUpdateValid) {
      throw new Error("latest resize candidate is invalid");
    }
    if (typeof clientOperationId !== "string" || clientOperationId.length === 0) {
      throw new TypeError("clientOperationId is required");
    }
    if (
      sameRect(this.#preview, this.#base) ||
      (this.#preview.width === this.#base.width &&
        this.#preview.height === this.#base.height)
    ) {
      throw new Error("resize transaction has no size change to commit");
    }

    this.#state = "committed";
    return {
      protocol_version: "chaptera.resize-intent.v1",
      client_operation_id: clientOperationId,
      node_id: this.#nodeId,
      after_bounds: cloneRect(this.#preview),
    };
  }
}
