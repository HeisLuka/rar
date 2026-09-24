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
        response.writeHead(200, {
          "content-type": mimeFor(filePath),
          "cache-control": "no-store"
        });
        response.end(body);
      } catch {
        response.writeHead(404).end("not found");
      }
    });
    server.listen(0, "127.0.0.1", () =>
      resolve({ server, port: server.address().port }));
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

function sameBounds(a, b) {
  return a && b &&
    a.x === b.x &&
    a.y === b.y &&
    a.width === b.width &&
    a.height === b.height;
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
    const page = await browser.newPage({ viewport: { width: 1280, height: 960 } });
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
        revision_id: window.__shell.snapshot.revision_id,
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
    if (during.revision_id !== initial.revision_id) {
      throw new Error("pointermove mutated browser base revision");
    }
    if (during.browser_commit_requests !== 0) {
      throw new Error("pointermove crossed HTTP commit boundary");
    }
    if (!during.preview) throw new Error("pointermove did not create transient preview");

    await page.mouse.up();
    await page.waitForFunction(
      (oldRevision) => window.__shell.snapshot.revision_id !== oldRevision,
      initial.revision_id,
      { timeout: 5000 }
    );

    const accepted = await page.evaluate((nodeId) => ({
      shell: window.__shell.stateReceipt(),
      last_request: structuredClone(window.__service.lastRequest),
      browser_commit_requests: window.__service.commitRequests,
      source_hash: window.__shell.snapshot.source_hash,
      bounds: window.__shell.nodeScreenBounds(nodeId),
      commit_trace: structuredClone(window.__service.lastCommitTraceContext),
      spans: structuredClone(window.__observability.spans),
    }), initial.node_id);

    if (accepted.browser_commit_requests !== 1) {
      throw new Error("browser did not issue exactly one HTTP commit");
    }
    if ("before" in accepted.last_request.command) {
      throw new Error("browser sent canonical before-state");
    }
    if (accepted.shell.selected_node_id !== initial.node_id) {
      throw new Error("canonical NodeId changed");
    }
    if (accepted.source_hash !== initial.source_hash) {
      throw new Error("source identity changed");
    }
    if (!accepted.commit_trace?.trace_id) {
      throw new Error("commit trace context missing");
    }
    if (sameBounds(accepted.bounds, initial.bounds)) {
      throw new Error("accepted move did not change rendered bounds");
    }

    const openBrowserSpan = lastSpan(initial.spans, "browser.scene_current");
    const commitBrowserSpan = lastSpan(accepted.spans, "browser.commit_http");
    const commitSceneBrowserSpan = lastSpan(accepted.spans, "browser.scene_revision");

    const openTrace = await traceSummary(openBrowserSpan.trace_id);
    const commitTrace = await traceSummary(commitBrowserSpan.trace_id);
    const commitSceneTrace = await traceSummary(commitSceneBrowserSpan.trace_id);
    const openServerSpan = serverSpan(openTrace, "gateway.scene_current");
    const commitServerSpan = serverSpan(commitTrace, "gateway.commit");
    const commitSceneServerSpan = serverSpan(commitSceneTrace, "gateway.scene_revision");

    if (commitTrace.trace_id !== accepted.commit_trace.trace_id) {
      throw new Error("browser/server commit trace identity diverged");
    }
    if (!commitTrace.spans.some(
      (span) => span.client_operation_id === accepted.last_request.client_operation_id
    )) {
      throw new Error("server trace lost semantic client-operation correlation");
    }

    let state = await harnessState();
    if (state.commit_requests !== 1 || state.executor_calls !== 1) {
      throw new Error("server did not execute exactly one semantic commit");
    }
    if (state.current_revision_id !== accepted.shell.revision_id) {
      throw new Error("browser/server revision identity diverged");
    }

    const exactRetry = await postCommit(accepted.last_request);
    if (exactRetry.revision_id !== accepted.shell.revision_id) {
      throw new Error("exact HTTP retry did not return same accepted revision");
    }
    state = await harnessState();
    if (state.commit_requests !== 2 || state.executor_calls !== 1) {
      throw new Error("idempotent HTTP retry re-executed semantic mutation");
    }

    const staleRequest = structuredClone(accepted.last_request);
    staleRequest.client_operation_id = "stale-probe-" + RUN_INDEX;
    staleRequest.command.x_emu += 1000;
    const stale = await postCommit(staleRequest);
    if (
      stale.protocol_version !== "chaptera.commit-rejected.v1" ||
      stale.code !== "stale_revision"
    ) {
      throw new Error("stale HTTP request was not explicitly rejected");
    }
    state = await harnessState();
    if (
      state.current_revision_id !== accepted.shell.revision_id ||
      state.executor_calls !== 1
    ) {
      throw new Error("stale HTTP request mutated server state");
    }

    const acceptedRevision = accepted.shell.revision_id;

    await page.locator("#undo").click();
    await page.waitForFunction(
      (prior) =>
        window.__historyError === null &&
        window.__shell.snapshot.revision_id !== prior &&
        window.__historyActions.length >= 1,
      acceptedRevision,
      { timeout: 5000 }
    );
    const undo = await page.evaluate((nodeId) => ({
      shell: window.__shell.stateReceipt(),
      bounds: window.__shell.nodeScreenBounds(nodeId),
      action: structuredClone(window.__historyActions.at(-1)),
      history_requests: window.__service.historyRequests,
      history_trace: structuredClone(window.__service.lastHistoryTraceContext),
      source_hash: window.__shell.snapshot.source_hash,
      spans: structuredClone(window.__observability.spans),
      error: window.__historyError,
    }), initial.node_id);
    if (undo.error) throw new Error("undo UI failed: " + undo.error);
    if (undo.action.kind !== "undo") throw new Error("Undo button routed wrong history kind");
    if (undo.action.request.command.kind !== "undo") throw new Error("Undo request kind mismatch");
    if (Object.keys(undo.action.request.command).length !== 1) {
      throw new Error("browser Undo request carried authoritative target state");
    }
    if (undo.action.result.protocol_version !== "chaptera.history-transition-accepted.v1") {
      throw new Error("Undo was not accepted as history transition");
    }
    if (undo.action.result.transition_kind !== "undo") {
      throw new Error("Undo result transition kind mismatch");
    }
    if (undo.shell.revision_id === acceptedRevision || undo.shell.revision_id === initial.revision_id) {
      throw new Error("Undo did not create a fresh immutable revision");
    }
    if (!sameBounds(undo.bounds, initial.bounds)) {
      throw new Error("Undo scene did not restore baseline geometry");
    }

    const undoRevision = undo.shell.revision_id;
    await page.locator("#redo").click();
    await page.waitForFunction(
      (prior) =>
        window.__historyError === null &&
        window.__shell.snapshot.revision_id !== prior &&
        window.__historyActions.length >= 2,
      undoRevision,
      { timeout: 5000 }
    );
    const redo = await page.evaluate((nodeId) => ({
      shell: window.__shell.stateReceipt(),
      bounds: window.__shell.nodeScreenBounds(nodeId),
      action: structuredClone(window.__historyActions.at(-1)),
      history_requests: window.__service.historyRequests,
      history_trace: structuredClone(window.__service.lastHistoryTraceContext),
      source_hash: window.__shell.snapshot.source_hash,
      spans: structuredClone(window.__observability.spans),
      error: window.__historyError,
    }), initial.node_id);
    if (redo.error) throw new Error("redo UI failed: " + redo.error);
    if (redo.action.kind !== "redo") throw new Error("Redo button routed wrong history kind");
    if (redo.action.request.command.kind !== "redo") throw new Error("Redo request kind mismatch");
    if (Object.keys(redo.action.request.command).length !== 1) {
      throw new Error("browser Redo request carried authoritative target state");
    }
    if (redo.action.result.protocol_version !== "chaptera.history-transition-accepted.v1") {
      throw new Error("Redo was not accepted as history transition");
    }
    if (redo.action.result.transition_kind !== "redo") {
      throw new Error("Redo result transition kind mismatch");
    }
    if (
      redo.shell.revision_id === acceptedRevision ||
      redo.shell.revision_id === undoRevision ||
      redo.shell.revision_id === initial.revision_id
    ) {
      throw new Error("Redo did not create a fresh immutable revision");
    }
    if (!sameBounds(redo.bounds, accepted.bounds)) {
      throw new Error("Redo scene did not restore accepted geometry");
    }
    if (redo.shell.selected_node_id !== initial.node_id) {
      throw new Error("history reconciliation lost canonical selection identity");
    }

    const undoBrowserSpan = lastSpan(undo.spans, "browser.history_http");
    const redoBrowserSpan = lastSpan(redo.spans, "browser.history_http");
    const undoTrace = await traceSummary(undoBrowserSpan.trace_id);
    const redoTrace = await traceSummary(redoBrowserSpan.trace_id);
    const undoServerSpan = serverSpan(undoTrace, "gateway.history");
    const redoServerSpan = serverSpan(redoTrace, "gateway.history");

    if (!undoTrace.spans.some(
      (span) => span.client_operation_id === undo.action.request.client_operation_id
    )) {
      throw new Error("Undo trace lost client-operation correlation");
    }
    if (!redoTrace.spans.some(
      (span) => span.client_operation_id === redo.action.request.client_operation_id
    )) {
      throw new Error("Redo trace lost client-operation correlation");
    }

    const afterHistory = await harnessState();
    if (afterHistory.history_requests !== 2 || afterHistory.history_executor_calls !== 2) {
      throw new Error("browser Undo/Redo did not execute exactly two authoritative history transitions");
    }
    if (afterHistory.current_revision_id !== redo.shell.revision_id) {
      throw new Error("browser/server revision identity diverged after Redo");
    }

    const metrics = await metricsSnapshot();
    assertMetricLabelsBounded(metrics);

    if (RUN_INDEX === "0") {
      await page.locator("#host").screenshot({
        path: path.join(TARGET, BROWSER_ENGINE + "-http-shell.png")
      });
    }

    const receipt = {
      receipt_kind: "chaptera.synthetic-http-service-observability.v2",
      browser_engine: BROWSER_ENGINE,
      browser_version: browserVersion,
      run_index: Number.parseInt(RUN_INDEX, 10),
      real_pub: false,
      product_acceptance: false,
      api_process_boundary: true,
      server_kernel: "public RevisionKernel harness",
      initial_revision_id: initial.revision_id,
      accepted_revision_id: acceptedRevision,
      undo_revision_id: undoRevision,
      redo_revision_id: redo.shell.revision_id,
      browser_commit_requests: accepted.browser_commit_requests,
      browser_history_requests: redo.history_requests,
      server_commit_requests_after_probes: afterHistory.commit_requests,
      semantic_executor_calls: afterHistory.executor_calls,
      server_history_requests: afterHistory.history_requests,
      history_executor_calls: afterHistory.history_executor_calls,
      exact_retry_same_revision: exactRetry.revision_id === acceptedRevision,
      stale_base_rejected: stale.code === "stale_revision",
      browser_sent_before_state: "before" in accepted.last_request.command,
      browser_history_sent_authoritative_target:
        Object.keys(undo.action.request.command).length !== 1 ||
        Object.keys(redo.action.request.command).length !== 1,
      node_id_stable: redo.shell.selected_node_id === initial.node_id,
      source_hash_stable:
        accepted.source_hash === initial.source_hash &&
        undo.source_hash === initial.source_hash &&
        redo.source_hash === initial.source_hash,
      undo_restored_baseline_geometry: sameBounds(undo.bounds, initial.bounds),
      redo_restored_accepted_geometry: sameBounds(redo.bounds, accepted.bounds),
      undo_created_fresh_revision:
        undoRevision !== initial.revision_id && undoRevision !== acceptedRevision,
      redo_created_fresh_revision:
        redo.shell.revision_id !== initial.revision_id &&
        redo.shell.revision_id !== acceptedRevision &&
        redo.shell.revision_id !== undoRevision,
      observability: {
        trace_protocol_version: accepted.commit_trace.protocol_version,
        same_trace_browser_and_server: commitTrace.trace_id === accepted.commit_trace.trace_id,
        semantic_operation_correlated: commitTrace.spans.some(
          (span) => span.client_operation_id === accepted.last_request.client_operation_id
        ),
        history_operations_correlated:
          undoTrace.spans.some(
            (span) => span.client_operation_id === undo.action.request.client_operation_id
          ) &&
          redoTrace.spans.some(
            (span) => span.client_operation_id === redo.action.request.client_operation_id
          ),
        metric_series_count: metrics.metrics.length,
        metrics_high_cardinality_labels_absent: true,
        document_payload_logged: false,
        timings_ms: {
          browser_scene_current_http: openBrowserSpan.duration_ms,
          gateway_scene_current: openServerSpan.duration_ms,
          browser_commit_http: commitBrowserSpan.duration_ms,
          gateway_commit: commitServerSpan.duration_ms,
          browser_scene_revision_http: commitSceneBrowserSpan.duration_ms,
          gateway_scene_revision: commitSceneServerSpan.duration_ms,
          browser_undo_http: undoBrowserSpan.duration_ms,
          gateway_undo: undoServerSpan.duration_ms,
          browser_redo_http: redoBrowserSpan.duration_ms,
          gateway_redo: redoServerSpan.duration_ms,
        }
      },
      note:
        "Synthetic Scene V1 plus public revision-kernel HTTP harness. Move and browser-visible Undo/Redo transport are proven, but authoritative history executor remains synthetic. Timings are CI plumbing baselines only and must not be used as real-PUB product SLOs."
    };

    if (receipt.browser_history_sent_authoritative_target) {
      throw new Error("browser history request leaked authoritative target state");
    }
    if (!receipt.source_hash_stable) {
      throw new Error("source identity changed across history loop");
    }

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
