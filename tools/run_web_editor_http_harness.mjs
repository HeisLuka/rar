#!/usr/bin/env node
import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, firefox } from "playwright";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TARGET = path.join(ROOT, "target", "web-editor-http");
const API_PORT = 8765;
const API_BASE = "http://127.0.0.1:" + API_PORT;
const BROWSER_ENGINE = process.env.BROWSER_ENGINE ?? "chromium";
const RUN_INDEX = process.env.RUN_INDEX ?? "0";
const BROWSERS = { chromium, firefox };

if (!(BROWSER_ENGINE in BROWSERS)) {
  throw new Error("unsupported BROWSER_ENGINE: " + BROWSER_ENGINE);
}

function mimeFor(filePath) {
  if (filePath.endsWith(".html")) return "text/html; charset=utf-8";
  if (filePath.endsWith(".mjs") || filePath.endsWith(".js")) return "text/javascript; charset=utf-8";
  if (filePath.endsWith(".json")) return "application/json; charset=utf-8";
  return "application/octet-stream";
}

function startStaticServer() {
  return new Promise((resolve) => {
    const server = http.createServer((request, response) => {
      try {
        const url = new URL(request.url ?? "/", "http://localhost");
        const filePath = path.resolve(ROOT, "." + decodeURIComponent(url.pathname));
        if (!filePath.startsWith(ROOT + path.sep)) {
          response.writeHead(403).end("forbidden");
          return;
        }
        const body = fs.readFileSync(filePath);
        response.writeHead(200, { "content-type": mimeFor(filePath), "cache-control": "no-store" });
        response.end(body);
      } catch {
        response.writeHead(404).end("not found");
      }
    });
    server.listen(0, "127.0.0.1", () => resolve({ server, port: server.address().port }));
  });
}

async function waitForApi(child) {
  const deadline = Date.now() + 10000;
  let lastError = null;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error("HTTP harness exited before ready with code " + child.exitCode);
    }
    try {
      const response = await fetch(API_BASE + "/health");
      if (response.ok) return;
      lastError = new Error("health returned " + response.status);
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error("HTTP harness did not become ready: " + String(lastError));
}

async function postCommit(request) {
  const response = await fetch(API_BASE + "/v1/commit", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(request),
  });
  const value = await response.json();
  if (!response.ok) throw new Error("commit request failed: " + JSON.stringify(value));
  return value;
}

async function harnessState() {
  const response = await fetch(API_BASE + "/v1/harness/state");
  if (!response.ok) throw new Error("harness state failed");
  return response.json();
}

async function traceSummary(traceId) {
  const response = await fetch(
    API_BASE + "/v1/observability/traces/" + encodeURIComponent(traceId)
  );
  if (!response.ok) throw new Error("trace summary failed for " + traceId);
  return response.json();
}

async function metricsSnapshot() {
  const response = await fetch(API_BASE + "/v1/observability/metrics");
  if (!response.ok) throw new Error("metrics snapshot failed");
  return response.json();
}

function lastSpan(spans, name) {
  const matches = spans.filter((span) => span.name === name);
  if (matches.length === 0) throw new Error("missing browser span " + name);
  return matches[matches.length - 1];
}

function serverSpan(trace, name) {
  const span = trace.spans.find((item) => item.name === name);
  if (!span) throw new Error("missing server span " + name + " in trace " + trace.trace_id);
  return span;
}

function assertMetricLabelsBounded(snapshot) {
  const forbidden = new Set([
    "document_id", "principal_id", "user_id", "story_id", "revision_id",
    "event_id", "client_operation_id", "interaction_id", "trace_id",
    "source_hash", "content_hash", "file_name", "filename"
  ]);
  for (const metric of snapshot.metrics ?? []) {
    for (const label of Object.keys(metric.labels ?? {})) {
      if (forbidden.has(label)) {
        throw new Error("high-cardinality metric label leaked: " + label);
      }
    }
  }
}

async function main() {
  fs.mkdirSync(TARGET, { recursive: true });
  const api = spawn(
    "python3",
    ["services/editor-api/web_shell_http_harness.py", "--port", String(API_PORT)],
    { cwd: ROOT, stdio: ["ignore", "pipe", "pipe"] }
  );
  let apiStdout = "";
  let apiStderr = "";
  api.stdout.on("data", (chunk) => { apiStdout += chunk.toString(); });
  api.stderr.on("data", (chunk) => { apiStderr += chunk.toString(); });

  const staticState = await startStaticServer();
  let browser = null;
  try {
    await waitForApi(api);
    browser = await BROWSERS[BROWSER_ENGINE].launch({ headless: true });
    const browserVersion = browser.version();
    const page = await browser.newPage({ viewport: { width: 1280, height: 920 } });
    const pageErrors = [];
    const consoleErrors = [];
    page.on("pageerror", (error) => pageErrors.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });

    const pageUrl =
      "http://127.0.0.1:" + staticState.port +
      "/apps/web/editor-shell-http-harness.html?api=" +
      encodeURIComponent(API_BASE);
    await page.goto(pageUrl, { waitUntil: "networkidle" });
    try {
      await page.waitForFunction(() => window.__shellReady === true, null, { timeout: 5000 });
    } catch {
      throw new Error(
        "HTTP browser shell boot failed: pageErrors=" + JSON.stringify(pageErrors) +
        " consoleErrors=" + JSON.stringify(consoleErrors)
      );
    }

    const initial = await page.evaluate(() => {
      const nodeId = window.__shell.snapshot.nodes[0].node_id;
      const hostRect = document.getElementById("host").getBoundingClientRect();
      return {
        node_id: nodeId,
        document_id: window.__shell.snapshot.document_id,
        revision_id: window.__shell.snapshot.revision_id,
        snapshot_id: window.__shell.snapshot.snapshot_id,
        source_hash: window.__shell.snapshot.source_hash,
        bounds: window.__shell.nodeScreenBounds(nodeId),
        host: { x: hostRect.x, y: hostRect.y },
        spans: structuredClone(window.__observability.spans),
      };
    });

    const startX = initial.host.x + initial.bounds.x + initial.bounds.width / 2;
    const startY = initial.host.y + initial.bounds.y + initial.bounds.height / 2;
    const endX = startX + 40;
    const endY = startY + 25;

    await page.mouse.move(startX, startY);
    await page.mouse.down();
    for (let i = 1; i <= 8; i += 1) {
      await page.mouse.move(
        startX + ((endX - startX) * i) / 8,
        startY + ((endY - startY) * i) / 8
      );
    }

    const during = await page.evaluate(() => ({
      revision_id: window.__shell.snapshot.revision_id,
      browser_commit_requests: window.__service.commitRequests,
      preview: window.__shell.gesture?.previewBounds() ?? null,
    }));
    if (during.revision_id !== initial.revision_id) throw new Error("pointermove mutated browser base revision");
    if (during.browser_commit_requests !== 0) throw new Error("pointermove crossed HTTP commit boundary");
    if (!during.preview) throw new Error("pointermove did not create transient preview");

    await page.mouse.up();
    await page.waitForFunction(
      (oldRevision) => window.__shell.snapshot.revision_id !== oldRevision,
      initial.revision_id,
      { timeout: 5000 }
    );

    const browserFinal = await page.evaluate(() => ({
      shell: window.__shell.stateReceipt(),
      last_request: structuredClone(window.__service.lastRequest),
      browser_commit_requests: window.__service.commitRequests,
      source_hash: window.__shell.snapshot.source_hash,
      commit_trace: structuredClone(window.__service.lastCommitTraceContext),
      spans: structuredClone(window.__observability.spans),
    }));

    if (browserFinal.browser_commit_requests !== 1) throw new Error("browser did not issue exactly one HTTP commit");
    if ("before" in browserFinal.last_request.command) throw new Error("browser sent canonical before-state");
    if (browserFinal.shell.selected_node_id !== initial.node_id) throw new Error("canonical NodeId changed");
    if (browserFinal.source_hash !== initial.source_hash) throw new Error("source identity changed");
    if (!browserFinal.commit_trace?.trace_id) throw new Error("commit trace context missing");

    const openBrowserSpan = lastSpan(initial.spans, "browser.scene_current");
    const commitBrowserSpan = lastSpan(browserFinal.spans, "browser.commit_http");
    const sceneBrowserSpan = lastSpan(browserFinal.spans, "browser.scene_revision");

    const openTrace = await traceSummary(openBrowserSpan.trace_id);
    const commitTrace = await traceSummary(commitBrowserSpan.trace_id);
    const sceneTrace = await traceSummary(sceneBrowserSpan.trace_id);
    const openServerSpan = serverSpan(openTrace, "gateway.scene_current");
    const commitServerSpan = serverSpan(commitTrace, "gateway.commit");
    const sceneServerSpan = serverSpan(sceneTrace, "gateway.scene_revision");

    if (commitTrace.trace_id !== browserFinal.commit_trace.trace_id) {
      throw new Error("browser/server commit trace identity diverged");
    }
    if (!commitTrace.spans.some(
      (span) => span.client_operation_id === browserFinal.last_request.client_operation_id
    )) {
      throw new Error("server trace lost semantic client-operation correlation");
    }

    const metrics = await metricsSnapshot();
    assertMetricLabelsBounded(metrics);

    const afterBrowser = await harnessState();
    if (afterBrowser.commit_requests !== 1 || afterBrowser.executor_calls !== 1) {
      throw new Error("server did not execute exactly one semantic commit");
    }
    if (afterBrowser.current_revision_id !== browserFinal.shell.revision_id) {
      throw new Error("browser/server revision identity diverged");
    }

    const exactRetry = await postCommit(browserFinal.last_request);
    if (exactRetry.revision_id !== browserFinal.shell.revision_id) {
      throw new Error("exact HTTP retry did not return same accepted revision");
    }
    const afterRetry = await harnessState();
    if (afterRetry.commit_requests !== 2 || afterRetry.executor_calls !== 1) {
      throw new Error("idempotent HTTP retry re-executed semantic mutation");
    }

    const staleRequest = structuredClone(browserFinal.last_request);
    staleRequest.client_operation_id = "stale-probe-" + RUN_INDEX;
    staleRequest.command.x_emu += 1000;
    const stale = await postCommit(staleRequest);
    if (stale.protocol_version !== "chaptera.commit-rejected.v1" || stale.code !== "stale_revision") {
      throw new Error("stale HTTP request was not explicitly rejected");
    }
    const afterStale = await harnessState();
    if (afterStale.current_revision_id !== browserFinal.shell.revision_id || afterStale.executor_calls !== 1) {
      throw new Error("stale HTTP request mutated server state");
    }

    const history = await page.evaluate(async (runIndex) => {
      const shell = window.__shell;
      const service = window.__service;
      const undoRequest = {
        protocol_version: "chaptera.history-transition-intent.v1",
        document_id: shell.snapshot.document_id,
        source_hash: shell.snapshot.source_hash,
        base_revision_id: shell.snapshot.revision_id,
        client_operation_id: "history-undo-" + runIndex,
        command: { kind: "undo" },
      };
      const undo = await service.historyTransition(structuredClone(undoRequest));
      const undoScene = await service.sceneForRevision(undo.revision_id);
      shell.loadSnapshot(undoScene, { preserveSelection: false });

      const redoRequest = {
        protocol_version: "chaptera.history-transition-intent.v1",
        document_id: shell.snapshot.document_id,
        source_hash: shell.snapshot.source_hash,
        base_revision_id: shell.snapshot.revision_id,
        client_operation_id: "history-redo-" + runIndex,
        command: { kind: "redo" },
      };
      const redo = await service.historyTransition(structuredClone(redoRequest));
      const redoScene = await service.sceneForRevision(redo.revision_id);
      shell.loadSnapshot(redoScene, { preserveSelection: false });

      return {
        undo_request: undoRequest,
        undo,
        undo_snapshot_id: undoScene.snapshot_id,
        redo_request: redoRequest,
        redo,
        redo_snapshot_id: redoScene.snapshot_id,
        final_revision_id: shell.snapshot.revision_id,
        final_snapshot_id: shell.snapshot.snapshot_id,
        history_requests: service.historyRequests,
        last_history_trace: structuredClone(service.lastHistoryTraceContext),
        spans: structuredClone(window.__observability.spans),
      };
    }, RUN_INDEX);

    if (history.undo.protocol_version !== "chaptera.history-transition-accepted.v1") {
      throw new Error("Undo did not cross the HTTP history seam");
    }
    if (history.redo.protocol_version !== "chaptera.history-transition-accepted.v1") {
      throw new Error("Redo did not cross the HTTP history seam");
    }
    if (history.history_requests !== 2) throw new Error("browser history request count mismatch");
    if (history.final_revision_id !== history.redo.revision_id) {
      throw new Error("browser did not load Redo revision scene");
    }
    if (!history.last_history_trace?.trace_id) throw new Error("history trace context missing");

    const afterHistory = await harnessState();
    if (
      afterHistory.current_revision_id !== history.redo.revision_id ||
      afterHistory.executor_calls !== 1 ||
      afterHistory.history_executor_calls !== 2 ||
      afterHistory.history_requests !== 2
    ) {
      throw new Error("HTTP history seam did not execute exactly one Undo and one Redo");
    }

    const historyRetry = await page.evaluate(
      async (undoRequest) => window.__service.historyTransition(structuredClone(undoRequest)),
      history.undo_request,
    );
    if (historyRetry.revision_id !== history.undo.revision_id) {
      throw new Error("exact history retry did not return original Undo revision");
    }
    const afterHistoryRetry = await harnessState();
    if (
      afterHistoryRetry.current_revision_id !== history.redo.revision_id ||
      afterHistoryRetry.history_executor_calls !== 2
    ) {
      throw new Error("exact history retry re-executed or moved current revision");
    }

    const historyBrowserSpan = lastSpan(history.spans, "browser.history_http");
    const historyTrace = await traceSummary(historyBrowserSpan.trace_id);
    const historyServerSpan = serverSpan(historyTrace, "gateway.commit");
    if (historyTrace.trace_id !== history.last_history_trace.trace_id) {
      throw new Error("browser/server history trace identity diverged");
    }
    if (!historyTrace.spans.some(
      (span) => span.client_operation_id === history.redo_request.client_operation_id
    )) {
      throw new Error("server trace lost history client-operation correlation");
    }

    if (RUN_INDEX === "0") {
      await page.locator("#host").screenshot({
        path: path.join(TARGET, BROWSER_ENGINE + "-http-shell.png")
      });
    }

    const receipt = {
      receipt_kind: "chaptera.synthetic-http-service-observability.v1",
      browser_engine: BROWSER_ENGINE,
      browser_version: browserVersion,
      run_index: Number.parseInt(RUN_INDEX, 10),
      real_pub: false,
      product_acceptance: false,
      api_process_boundary: true,
      server_kernel: "public RevisionKernel harness",
      initial_revision_id: initial.revision_id,
      accepted_revision_id: browserFinal.shell.revision_id,
      undo_revision_id: history.undo.revision_id,
      redo_revision_id: history.redo.revision_id,
      browser_commit_requests: browserFinal.browser_commit_requests,
      browser_history_requests: history.history_requests + 1,
      server_commit_requests_after_probes: afterHistoryRetry.commit_requests,
      semantic_executor_calls: afterHistoryRetry.executor_calls,
      history_executor_calls: afterHistoryRetry.history_executor_calls,
      exact_retry_same_revision: exactRetry.revision_id === browserFinal.shell.revision_id,
      stale_base_rejected: stale.code === "stale_revision",
      undo_redo_cross_http_commit_transport:
        history.undo.protocol_version === "chaptera.history-transition-accepted.v1" &&
        history.redo.protocol_version === "chaptera.history-transition-accepted.v1",
      history_exact_retry_no_reexecution:
        afterHistoryRetry.history_executor_calls === afterHistory.history_executor_calls,
      browser_sent_before_state: "before" in browserFinal.last_request.command,
      node_id_stable: browserFinal.shell.selected_node_id === initial.node_id,
      source_hash_stable: browserFinal.source_hash === initial.source_hash,
      observability: {
        trace_protocol_version: browserFinal.commit_trace.protocol_version,
        same_trace_browser_and_server: commitTrace.trace_id === browserFinal.commit_trace.trace_id,
        semantic_operation_correlated: commitTrace.spans.some(
          (span) => span.client_operation_id === browserFinal.last_request.client_operation_id
        ),
        metric_series_count: metrics.metrics.length,
        metrics_high_cardinality_labels_absent: true,
        document_payload_logged: false,
        timings_ms: {
          browser_scene_current_http: openBrowserSpan.duration_ms,
          gateway_scene_current: openServerSpan.duration_ms,
          browser_commit_http: commitBrowserSpan.duration_ms,
          gateway_commit: commitServerSpan.duration_ms,
          browser_scene_revision_http: sceneBrowserSpan.duration_ms,
          gateway_scene_revision: sceneServerSpan.duration_ms,
          browser_history_http: historyBrowserSpan.duration_ms,
          gateway_history_commit: historyServerSpan.duration_ms,
        }
      },
      note: "Synthetic Scene V1 plus public revision-kernel HTTP harness. Timings are CI transport/observability baselines only and must not be used as real-PUB product SLOs."
    };

    const outputPath = path.join(
      TARGET,
      BROWSER_ENGINE + "-http-receipt-" + RUN_INDEX.padStart(2, "0") + ".json"
    );
    fs.writeFileSync(outputPath, JSON.stringify(receipt, null, 2) + "\n");
    process.stdout.write(JSON.stringify(receipt, null, 2) + "\n");
  } finally {
    if (browser) await browser.close();
    await new Promise((resolve) => staticState.server.close(resolve));
    if (api.exitCode === null) {
      api.kill("SIGTERM");
      await new Promise((resolve) => {
        api.once("exit", resolve);
        setTimeout(resolve, 2000);
      });
    }
    if (api.exitCode && api.exitCode !== 0 && api.exitCode !== null) {
      console.error("API stdout:", apiStdout);
      console.error("API stderr:", apiStderr);
    }
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
