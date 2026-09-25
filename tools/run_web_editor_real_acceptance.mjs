#!/usr/bin/env node
import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TARGET = path.join(ROOT, "target", "web-acceptance-real");
const API_PORT = 8765;
const API_BASE = "http://127.0.0.1:" + API_PORT;

const FIXTURE = process.env.REAL_FIXTURE ?? path.join(ROOT, ".web-v0", "SampleNewsletter.pub");
const RESOLVED_GRAPH = process.env.REAL_RESOLVED_GRAPH ?? path.join(ROOT, ".web-v0", "resolved-graph.json");
const VIEWER_RECEIPT = process.env.REAL_VIEWER_RECEIPT ?? path.join(ROOT, "apps", "web", "acceptance", "receipts", "viewer-geometry.real.json");
const REVISION_RECEIPT = process.env.REAL_REVISION_RECEIPT ?? path.join(ROOT, "packages", "protocol", "revision", "v1", "producer-receipts", "sample-newsletter.real.json");
const EXPORTER = process.env.REAL_EXPORTER ?? path.join(ROOT, "vendor", "producer-a", "target", "debug", "chaptera-producer-a");
const WORK_DIR = process.env.REAL_WORK_DIR ?? path.join(ROOT, ".web-v0", "real-browser-service");
const REPOSITORY_COMMIT_SHA = process.env.REPOSITORY_COMMIT_SHA ?? process.env.GITHUB_SHA ?? "";
const TARGET_NODE_ID = "007d9898-568b-5125-b519-8d88243aabfb";
const CANONICAL_OPERATION_ID = "9c2f8d5e-3f1b-4a57-9d40-6a7e2c11b002";
const BEFORE = { x: 526710, y: 1191292, width: 4436165, height: 587274 };
const AFTER = { x: 653710, y: 1445292, width: 4436165, height: 587274 };
const EMU_PER_CSS_PX = 12700;

function sameJson(a, b) {
  return JSON.stringify(a) === JSON.stringify(b);
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
          "cache-control": "no-store",
        });
        response.end(body);
      } catch {
        response.writeHead(404).end("not found");
      }
    });
    server.listen(0, "127.0.0.1", () => resolve({ server, port: server.address().port }));
  });
}

async function waitForApi(child) {
  const deadline = Date.now() + 15000;
  let lastError = null;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error("real acceptance service exited before ready with code " + child.exitCode);
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
  throw new Error("real acceptance service did not become ready: " + String(lastError));
}

async function getJson(url) {
  const response = await fetch(API_BASE + url, {
    headers: { "x-chaptera-principal-id": "synthetic-editor" },
  });
  const value = await response.json();
  if (!response.ok) throw new Error(url + " failed: " + JSON.stringify(value));
  return value;
}

async function postJson(url, body = null) {
  const response = await fetch(API_BASE + url, {
    method: "POST",
    headers: {
      "x-chaptera-principal-id": "synthetic-editor",
      ...(body === null ? {} : { "content-type": "application/json" }),
    },
    ...(body === null ? {} : { body: JSON.stringify(body) }),
  });
  const value = await response.json();
  if (!response.ok) throw new Error(url + " failed: " + JSON.stringify(value));
  return value;
}

function pageUrl(port) {
  return (
    "http://127.0.0.1:" + port +
    "/apps/web/editor-shell-http-harness.html?api=" + encodeURIComponent(API_BASE) +
    "&operation_id=" + encodeURIComponent(CANONICAL_OPERATION_ID) +
    "&emu_per_css_px=" + encodeURIComponent(String(EMU_PER_CSS_PX))
  );
}

async function waitShell(page) {
  await page.waitForFunction(() => window.__shellReady === true, null, { timeout: 10000 });
}

async function main() {
  if (!/^[0-9a-f]{40}$/.test(REPOSITORY_COMMIT_SHA)) {
    throw new Error("REPOSITORY_COMMIT_SHA must be a 40-character lowercase commit SHA");
  }
  fs.mkdirSync(TARGET, { recursive: true });
  fs.mkdirSync(WORK_DIR, { recursive: true });

  const api = spawn(
    "python3",
    [
      "services/editor-api/web_real_acceptance_service.py",
      "--port", String(API_PORT),
      "--fixture", FIXTURE,
      "--resolved-graph", RESOLVED_GRAPH,
      "--viewer-receipt", VIEWER_RECEIPT,
      "--revision-receipt", REVISION_RECEIPT,
      "--exporter", EXPORTER,
      "--work-dir", WORK_DIR,
    ],
    { cwd: ROOT, stdio: ["ignore", "pipe", "pipe"] },
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
    const browserVersion = browser.version();

    let page = await browser.newPage({ viewport: { width: 1280, height: 920 } });
    await page.goto(pageUrl(staticState.port), { waitUntil: "networkidle" });
    await waitShell(page);

    const initial = await page.evaluate((targetNodeId) => {
      const node = window.__shell.snapshot.nodes.find((item) => item.node_id === targetNodeId);
      if (!node) throw new Error("canonical MoveNode target missing from real Scene");
      const screen = window.__shell.nodeScreenBounds(targetNodeId);
      const host = document.getElementById("host").getBoundingClientRect();
      return {
        document_id: window.__shell.snapshot.document_id,
        source_hash: window.__shell.snapshot.source_hash,
        revision_id: window.__shell.snapshot.revision_id,
        snapshot_id: window.__shell.snapshot.snapshot_id,
        node_id: targetNodeId,
        before: structuredClone(node.bounds),
        screen,
        host: { x: host.x, y: host.y },
        capability_visible: document.getElementById("capability-state").dataset.visibleState === "true",
        loss_visible: document.getElementById("loss-state").dataset.visibleState === "true",
      };
    }, TARGET_NODE_ID);

    if (!sameJson(initial.before, BEFORE)) {
      throw new Error("initial real Scene does not equal canonical before-state");
    }
    if (!initial.capability_visible || !initial.loss_visible) {
      throw new Error("initial capability/loss state is not visible");
    }

    const startX = initial.host.x + initial.screen.x + initial.screen.width / 2;
    const startY = initial.host.y + initial.screen.y + initial.screen.height / 2;
    const endX = startX + (AFTER.x - BEFORE.x) / EMU_PER_CSS_PX;
    const endY = startY + (AFTER.y - BEFORE.y) / EMU_PER_CSS_PX;

    await page.mouse.move(startX, startY);
    await page.mouse.down();
    await page.mouse.move(endX, endY);

    const during = await page.evaluate(() => ({
      revision_id: window.__shell.snapshot.revision_id,
      browser_commit_requests: window.__service.commitRequests,
      preview: window.__shell.gesture?.previewBounds() ?? null,
      selected_node_id: window.__shell.selection.nodeId,
    }));
    if (during.revision_id !== initial.revision_id) {
      throw new Error("pointermove created a durable revision");
    }
    if (during.browser_commit_requests !== 0) {
      throw new Error("pointermove crossed HTTP commit boundary");
    }
    if (during.selected_node_id !== TARGET_NODE_ID || !sameJson(during.preview, AFTER)) {
      throw new Error("real DOM drag did not produce canonical transient preview");
    }

    await page.mouse.up();
    await page.waitForFunction(
      (oldRevision) => window.__shell.snapshot.revision_id !== oldRevision,
      initial.revision_id,
      { timeout: 10000 },
    );

    const accepted = await page.evaluate((targetNodeId) => {
      const node = window.__shell.snapshot.nodes.find((item) => item.node_id === targetNodeId);
      return {
        revision_id: window.__shell.snapshot.revision_id,
        snapshot_id: window.__shell.snapshot.snapshot_id,
        selected_node_id: window.__shell.selection.nodeId,
        bounds: structuredClone(node.bounds),
        request: structuredClone(window.__service.lastRequest),
        browser_commit_requests: window.__service.commitRequests,
      };
    }, TARGET_NODE_ID);
    if (!sameJson(accepted.bounds, AFTER)) throw new Error("accepted Scene geometry mismatch");
    if (accepted.selected_node_id !== TARGET_NODE_ID) throw new Error("NodeId changed after commit");
    if (accepted.browser_commit_requests !== 1) throw new Error("release did not create exactly one commit");
    if (
      accepted.request.client_operation_id !== CANONICAL_OPERATION_ID ||
      accepted.request.command.node_id !== TARGET_NODE_ID ||
      accepted.request.command.x_emu !== AFTER.x ||
      accepted.request.command.y_emu !== AFTER.y ||
      "before" in accepted.request.command
    ) {
      throw new Error("browser request is not the canonical Producer B intent");
    }

    const afterAcceptedState = await getJson("/v1/harness/state");
    if (
      afterAcceptedState.executor_calls !== 1 ||
      !sameJson(afterAcceptedState.last_operation, {
        kind: "move_node",
        node_id: TARGET_NODE_ID,
        before: BEFORE,
        after: AFTER,
      })
    ) {
      throw new Error("server did not derive canonical before-state exactly once");
    }

    const exactRetry = await postJson("/v1/commit", accepted.request);
    if (exactRetry.revision_id !== accepted.revision_id) {
      throw new Error("exact retry did not return same accepted revision");
    }
    const afterRetry = await getJson("/v1/harness/state");
    if (afterRetry.executor_calls !== 1) throw new Error("exact retry re-executed semantic mutation");

    const staleRequest = structuredClone(accepted.request);
    staleRequest.client_operation_id = "real-stale-probe";
    staleRequest.command.x_emu += 1000;
    const stale = await postJson("/v1/commit", staleRequest);
    if (stale.protocol_version !== "chaptera.commit-rejected.v1" || stale.code !== "stale_revision") {
      throw new Error("stale base was not explicitly rejected");
    }

    const history = await page.evaluate(async () => {
      const shell = window.__shell;
      const service = window.__service;
      const undoRequest = {
        protocol_version: "chaptera.history-transition-intent.v1",
        document_id: shell.snapshot.document_id,
        source_hash: shell.snapshot.source_hash,
        base_revision_id: shell.snapshot.revision_id,
        client_operation_id: "real-history-undo",
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
        client_operation_id: "real-history-redo",
        command: { kind: "redo" },
      };
      const redo = await service.historyTransition(structuredClone(redoRequest));
      const redoScene = await service.sceneForRevision(redo.revision_id);
      shell.loadSnapshot(redoScene, { preserveSelection: false });

      return {
        undo,
        undo_snapshot_id: undoScene.snapshot_id,
        redo,
        redo_snapshot_id: redoScene.snapshot_id,
        final_revision_id: shell.snapshot.revision_id,
        final_snapshot_id: shell.snapshot.snapshot_id,
      };
    });

    const afterHistory = await getJson("/v1/harness/state");
    if (afterHistory.executor_calls !== 1 || afterHistory.history_executor_calls !== 2) {
      throw new Error("Undo/Redo did not execute exactly once each");
    }
    if (afterHistory.current_revision_id !== history.redo.revision_id) {
      throw new Error("server current revision is not Redo");
    }

    const reopenedServerScene = await postJson("/v1/harness/reopen");
    if (reopenedServerScene.snapshot_id !== history.redo_snapshot_id) {
      throw new Error("fresh server-side reopen changed final Scene");
    }

    await page.close();
    page = await browser.newPage({ viewport: { width: 1280, height: 920 } });
    await page.goto(pageUrl(staticState.port), { waitUntil: "networkidle" });
    await waitShell(page);
    const reopened = await page.evaluate((targetNodeId) => {
      const node = window.__shell.snapshot.nodes.find((item) => item.node_id === targetNodeId);
      return {
        revision_id: window.__shell.snapshot.revision_id,
        snapshot_id: window.__shell.snapshot.snapshot_id,
        bounds: structuredClone(node.bounds),
        capability_visible: document.getElementById("capability-state").dataset.visibleState === "true",
        loss_visible: document.getElementById("loss-state").dataset.visibleState === "true",
      };
    }, TARGET_NODE_ID);
    if (
      reopened.revision_id !== history.redo.revision_id ||
      reopened.snapshot_id !== history.redo_snapshot_id ||
      !sameJson(reopened.bounds, AFTER)
    ) {
      throw new Error("new browser page did not reconstruct final Redo state");
    }
    if (!reopened.capability_visible || !reopened.loss_visible) {
      throw new Error("reopened capability/loss state is not visible");
    }

    const exportProof = await getJson("/v1/harness/export-proof?target=idml");
    if (!exportProof.geometry_matches_edit || exportProof.node_id !== TARGET_NODE_ID) {
      throw new Error("final editable export does not reflect canonical edit");
    }

    const finalState = await getJson("/v1/harness/state");
    if (
      finalState.fixture_sha256 !== initial.source_hash ||
      finalState.fixture_byte_len !== 291840 ||
      finalState.reopen_count < 1
    ) {
      throw new Error("source immutability/reopen proof failed");
    }

    const receipt = {
      receipt_version: "chaptera.web-acceptance-receipt.v1",
      receipt_class: "real_pub_browser",
      repository_commit_sha: REPOSITORY_COMMIT_SHA,
      browser: {
        name: "chromium",
        version: browserVersion,
        headless: true,
      },
      fixture: {
        name: "SampleNewsletter.pub",
        sha256: initial.source_hash,
        byte_len: 291840,
        family: "mature-0x2c",
      },
      initial_revision_id: initial.revision_id,
      selected_node_id: TARGET_NODE_ID,
      before_rect: BEFORE,
      after_rect: AFTER,
      client_operation_id: CANONICAL_OPERATION_ID,
      accepted_revision_id: accepted.revision_id,
      undo_revision_id: history.undo.revision_id,
      redo_revision_id: history.redo.revision_id,
      scene_snapshot_ids: {
        initial: initial.snapshot_id,
        accepted: accepted.snapshot_id,
        undo: history.undo_snapshot_id,
        redo: history.redo_snapshot_id,
        reopen: reopened.snapshot_id,
      },
      export: {
        target: "idml",
        sha256: exportProof.artifact_sha256,
        geometry_reflects_edit: true,
      },
      source_immutability: {
        before_sha256: initial.source_hash,
        after_sha256: finalState.fixture_sha256,
        unchanged: initial.source_hash === finalState.fixture_sha256,
      },
      capability_state_visible: initial.capability_visible && reopened.capability_visible,
      loss_state_visible: initial.loss_visible && reopened.loss_visible,
      native_pub_save_enabled: false,
      semantic_assertions: {
        pointermove_created_no_revision: true,
        one_release_one_move: accepted.browser_commit_requests === 1 && afterAcceptedState.executor_calls === 1,
        node_id_stable: accepted.selected_node_id === TARGET_NODE_ID,
        server_before_state_won:
          !("before" in accepted.request.command) &&
          sameJson(afterAcceptedState.last_operation.before, BEFORE),
        reopen_independent_of_browser_memory:
          finalState.reopen_count >= 1 && reopened.snapshot_id === history.redo_snapshot_id,
        stale_base_not_silently_accepted: stale.code === "stale_revision",
      },
    };

    const output = path.join(TARGET, "browser-acceptance.real.json");
    fs.writeFileSync(output, JSON.stringify(receipt, null, 2) + "\n");
    await page.locator("#host").screenshot({ path: path.join(TARGET, "chromium-real-shell.png") });
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
    if (api.exitCode && api.exitCode !== 0) {
      console.error("API stdout:", apiStdout);
      console.error("API stderr:", apiStderr);
    }
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
