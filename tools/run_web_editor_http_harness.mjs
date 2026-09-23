#!/usr/bin/env node
import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TARGET = path.join(ROOT, "target", "web-editor-http");
const API_PORT = 8765;
const API_BASE = "http://127.0.0.1:" + API_PORT;

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
    browser = await chromium.launch({ headless: true });
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
        revision_id: window.__shell.snapshot.revision_id,
        source_hash: window.__shell.snapshot.source_hash,
        bounds: window.__shell.nodeScreenBounds(nodeId),
        host: { x: hostRect.x, y: hostRect.y },
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
    }));

    if (browserFinal.browser_commit_requests !== 1) throw new Error("browser did not issue exactly one HTTP commit");
    if ("before" in browserFinal.last_request.command) throw new Error("browser sent canonical before-state");
    if (browserFinal.shell.selected_node_id !== initial.node_id) throw new Error("canonical NodeId changed");
    if (browserFinal.source_hash !== initial.source_hash) throw new Error("source identity changed");

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
    staleRequest.client_operation_id = "stale-probe-1";
    staleRequest.command.x_emu += 1000;
    const stale = await postCommit(staleRequest);
    if (stale.protocol_version !== "chaptera.commit-rejected.v1" || stale.code !== "stale_revision") {
      throw new Error("stale HTTP request was not explicitly rejected");
    }
    const afterStale = await harnessState();
    if (afterStale.current_revision_id !== browserFinal.shell.revision_id || afterStale.executor_calls !== 1) {
      throw new Error("stale HTTP request mutated server state");
    }

    await page.locator("#host").screenshot({
      path: path.join(TARGET, "chromium-http-shell.png")
    });

    const receipt = {
      receipt_kind: "chaptera.synthetic-http-service-plumbing.v1",
      browser_engine: "chromium",
      real_pub: false,
      product_acceptance: false,
      api_process_boundary: true,
      server_kernel: "public RevisionKernel harness",
      initial_revision_id: initial.revision_id,
      accepted_revision_id: browserFinal.shell.revision_id,
      browser_commit_requests: browserFinal.browser_commit_requests,
      server_commit_requests_after_probes: afterStale.commit_requests,
      semantic_executor_calls: afterStale.executor_calls,
      exact_retry_same_revision: exactRetry.revision_id === browserFinal.shell.revision_id,
      stale_base_rejected: stale.code === "stale_revision",
      browser_sent_before_state: "before" in browserFinal.last_request.command,
      node_id_stable: browserFinal.shell.selected_node_id === initial.node_id,
      source_hash_stable: browserFinal.source_hash === initial.source_hash,
      note: "Synthetic Scene V1 plus public revision-kernel HTTP harness. This proves process/network plumbing and must not close WEB-ACCEPTANCE-01."
    };

    fs.writeFileSync(
      path.join(TARGET, "chromium-http-receipt.json"),
      JSON.stringify(receipt, null, 2) + "\n"
    );
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
