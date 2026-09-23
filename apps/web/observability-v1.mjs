const TRACE_PROTOCOL_VERSION = "chaptera.trace-context.v1";
const ID_RE = /^[A-Za-z0-9._:-]{8,160}$/;
const OPERATION_CLASSES = new Set(["commit", "scene_read", "open", "reconnect", "export", "asset", "other"]);
const BROWSER_FAMILIES = new Set(["chromium", "firefox", "webkit", "other", "unknown"]);

function boundedId(label, value) {
  if (typeof value !== "string" || !ID_RE.test(value)) {
    throw new TypeError(label + " must be a bounded opaque identifier");
  }
  return value;
}

function defaultId(prefix) {
  const uuid = globalThis.crypto?.randomUUID?.();
  if (!uuid) {
    throw new Error("crypto.randomUUID() is required unless idFactory is provided");
  }
  return prefix + ":" + uuid;
}

export function normalizeTraceContextV1(value) {
  if (!value || typeof value !== "object") throw new TypeError("trace context is required");
  if (value.protocol_version !== TRACE_PROTOCOL_VERSION) throw new TypeError("unsupported trace context version");
  if (!OPERATION_CLASSES.has(value.operation_class)) throw new TypeError("unsupported operation_class");
  const browserFamily = value.browser_family ?? "unknown";
  if (!BROWSER_FAMILIES.has(browserFamily)) throw new TypeError("unsupported browser_family");
  const normalized = {
    protocol_version: TRACE_PROTOCOL_VERSION,
    trace_id: boundedId("trace_id", value.trace_id),
    interaction_id: boundedId("interaction_id", value.interaction_id),
    session_incarnation: boundedId("session_incarnation", value.session_incarnation),
    operation_class: value.operation_class,
    browser_family: browserFamily,
  };
  if (value.client_operation_id != null) {
    normalized.client_operation_id = boundedId("client_operation_id", value.client_operation_id);
  }
  return normalized;
}

export function traceHeadersV1(context) {
  const value = normalizeTraceContextV1(context);
  return {
    "x-chaptera-trace-version": value.protocol_version,
    "x-chaptera-trace-id": value.trace_id,
    "x-chaptera-interaction-id": value.interaction_id,
    "x-chaptera-session-incarnation": value.session_incarnation,
    "x-chaptera-operation-class": value.operation_class,
    "x-chaptera-browser-family": value.browser_family,
  };
}

export class BrowserObservabilityV1 {
  constructor({
    sessionIncarnation,
    browserFamily = "unknown",
    idFactory = null,
    maxSpans = 256,
  }) {
    this.sessionIncarnation = boundedId("session_incarnation", sessionIncarnation);
    if (!BROWSER_FAMILIES.has(browserFamily)) throw new TypeError("unsupported browser_family");
    if (!Number.isSafeInteger(maxSpans) || maxSpans <= 0) throw new TypeError("maxSpans must be positive");
    this.browserFamily = browserFamily;
    this.idFactory = idFactory ?? defaultId;
    this.maxSpans = maxSpans;
    this.spans = [];
  }

  createContext({ operationClass, clientOperationId = null, interactionId = null } = {}) {
    if (!OPERATION_CLASSES.has(operationClass)) throw new TypeError("unsupported operation_class");
    const context = {
      protocol_version: TRACE_PROTOCOL_VERSION,
      trace_id: boundedId("trace_id", this.idFactory("trace")),
      interaction_id: boundedId("interaction_id", interactionId ?? this.idFactory("interaction")),
      session_incarnation: this.sessionIncarnation,
      operation_class: operationClass,
      browser_family: this.browserFamily,
    };
    if (clientOperationId != null) {
      context.client_operation_id = boundedId("client_operation_id", clientOperationId);
    }
    return Object.freeze(context);
  }

  mark(name, context, { outcome = "success", durationMs = 0 } = {}) {
    if (typeof name !== "string" || !/^[a-z][a-z0-9_.-]{1,63}$/.test(name)) {
      throw new TypeError("span name must be bounded");
    }
    const normalized = normalizeTraceContextV1(context);
    if (!["success", "error", "rejected", "unknown"].includes(outcome)) {
      throw new TypeError("outcome must be bounded");
    }
    if (typeof durationMs !== "number" || !Number.isFinite(durationMs) || durationMs < 0) {
      throw new TypeError("durationMs must be non-negative");
    }
    const span = {
      protocol_version: TRACE_PROTOCOL_VERSION,
      name,
      trace_id: normalized.trace_id,
      interaction_id: normalized.interaction_id,
      session_incarnation: normalized.session_incarnation,
      operation_class: normalized.operation_class,
      browser_family: normalized.browser_family,
      outcome,
      duration_ms: Math.round(durationMs * 1000) / 1000,
    };
    if (normalized.client_operation_id) span.client_operation_id = normalized.client_operation_id;
    this.spans.push(span);
    if (this.spans.length > this.maxSpans) {
      this.spans.splice(0, this.spans.length - this.maxSpans);
    }
    return structuredClone(span);
  }

  async measure(name, context, fn) {
    const start = performance.now();
    let outcome = "success";
    try {
      return await fn();
    } catch (error) {
      outcome = "error";
      throw error;
    } finally {
      this.mark(name, context, { outcome, durationMs: performance.now() - start });
    }
  }

  receipt(traceId) {
    boundedId("trace_id", traceId);
    const spans = this.spans.filter((span) => span.trace_id === traceId).map((span) => structuredClone(span));
    return {
      protocol_version: TRACE_PROTOCOL_VERSION,
      trace_id: traceId,
      span_count: spans.length,
      spans,
      contains_document_payload: false,
      semantic_authority: false,
    };
  }
}

export { TRACE_PROTOCOL_VERSION };
