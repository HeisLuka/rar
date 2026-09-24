import { traceHeadersV1 } from "./observability-v1.mjs";

export class HttpEditorServiceV1 {
  constructor(baseUrl, { observability = null } = {}) {
    if (typeof baseUrl !== "string" || !/^https?:\/\//.test(baseUrl)) {
      throw new TypeError("absolute HTTP(S) base URL is required");
    }
    this.baseUrl = baseUrl.replace(/\/$/, "");
    this.commitRequests = 0;
    this.historyRequests = 0;
    this.lastRequest = null;
    this.lastHistoryRequest = null;
    this.observability = observability;
    this.lastTraceContext = null;
    this.lastCommitTraceContext = null;
    this.lastHistoryTraceContext = null;
    this.lastExportPreview = null;
    this.lastExportPreviewTraceContext = null;
  }

  async currentScene() {
    const context = this.#context("open");
    return this.#json("/v1/scenes/current", {}, context, "browser.scene_current");
  }

  async commit(request) {
    this.commitRequests += 1;
    this.lastRequest = structuredClone(request);
    const context = this.#context("commit", request.client_operation_id ?? null);
    this.lastCommitTraceContext = context;
    return this.#json("/v1/commit", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(request),
    }, context, "browser.commit_http");
  }

  async historyTransition(request) {
    this.historyRequests += 1;
    this.lastHistoryRequest = structuredClone(request);
    const context = this.#context("commit", request.client_operation_id ?? null);
    this.lastHistoryTraceContext = context;
    return this.#json("/v1/commit", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(request),
    }, context, "browser.history_http");
  }

  async exportPreview(target = "idml") {
    if (!["idml", "odg"].includes(target)) {
      throw new TypeError("export preview target must be idml or odg");
    }
    const context = this.#context("export");
    this.lastExportPreviewTraceContext = context;
    const preview = await this.#json(
      "/v1/export/preview?target=" + encodeURIComponent(target),
      {},
      context,
      "browser.export_preview_http",
    );
    this.lastExportPreview = structuredClone(preview);
    return preview;
  }

  async sceneForRevision(revisionId) {
    if (typeof revisionId !== "string" || !revisionId.startsWith("sha256:")) {
      throw new TypeError("revision id is required");
    }
    const context = this.#context("scene_read");
    return this.#json(
      "/v1/scenes/" + encodeURIComponent(revisionId),
      {},
      context,
      "browser.scene_revision"
    );
  }

  async harnessState() {
    return this.#json("/v1/harness/state");
  }

  async traceSummary(traceId) {
    if (typeof traceId !== "string" || traceId.length < 8) {
      throw new TypeError("trace id is required");
    }
    return this.#json("/v1/observability/traces/" + encodeURIComponent(traceId));
  }

  async metricsSnapshot() {
    return this.#json("/v1/observability/metrics");
  }

  #context(operationClass, clientOperationId = null) {
    if (!this.observability) return null;
    const context = this.observability.createContext({
      operationClass,
      clientOperationId,
    });
    this.lastTraceContext = context;
    return context;
  }

  async #json(path, options = {}, context = null, browserSpanName = null) {
    const headers = {
      ...(options.headers ?? {}),
      ...(context ? traceHeadersV1(context) : {}),
    };
    const execute = async () => {
      const response = await fetch(this.baseUrl + path, {
        cache: "no-store",
        credentials: "omit",
        ...options,
        headers,
      });
      const value = await response.json();
      if (!response.ok) {
        throw new Error("editor service request failed: " + response.status + " " + JSON.stringify(value));
      }
      return value;
    };

    if (this.observability && context && browserSpanName) {
      return this.observability.measure(browserSpanName, context, execute);
    }
    return execute();
  }
}
