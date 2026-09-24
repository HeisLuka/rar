#!/usr/bin/env node
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, firefox } from "playwright";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TARGET = path.join(ROOT, "target", "web-multitab-recovery");
const BROWSER_NAME = process.argv[2] ?? "chromium";
const ENGINES = { chromium, firefox };
if (!(BROWSER_NAME in ENGINES)) throw new Error("unsupported browser " + BROWSER_NAME);

function mimeFor(filePath) {
  if (filePath.endsWith(".html")) return "text/html; charset=utf-8";
  if (filePath.endsWith(".mjs") || filePath.endsWith(".js")) return "text/javascript; charset=utf-8";
  return "application/octet-stream";
}

function startServer() {
  return new Promise((resolve) => {
    const server = http.createServer((request, response) => {
      try {
        const url = new URL(request.url ?? "/", "http://localhost");
        const rel = url.pathname === "/" ? "/apps/web/multitab-recovery-probe.html" : decodeURIComponent(url.pathname);
        const filePath = path.resolve(ROOT, "." + rel);
        if (!filePath.startsWith(ROOT + path.sep)) {
          response.writeHead(403).end("forbidden");
          return;
        }
        const body = fs.readFileSync(filePath);
        response.writeHead(200, {"content-type": mimeFor(filePath), "cache-control": "no-store"});
        response.end(body);
      } catch {
        response.writeHead(404).end("not found");
      }
    });
    server.listen(0, "127.0.0.1", () => resolve({server, port: server.address().port}));
  });
}

async function openPersistent(browserType, userDataDir) {
  return browserType.launchPersistentContext(userDataDir, {
    headless: true,
    viewport: {width: 1000, height: 700},
  });
}

async function main() {
  fs.mkdirSync(TARGET, {recursive: true});
  const serverState = await startServer();
  const origin = "http://127.0.0.1:" + serverState.port + "/";
  const browserType = ENGINES[BROWSER_NAME];
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), "chaptera-recovery-" + BROWSER_NAME + "-"));

  let firstContext = await openPersistent(browserType, profile);
  let pageA = firstContext.pages()[0] ?? await firstContext.newPage();
  let pageB = await firstContext.newPage();

  try {
    await Promise.all([
      pageA.goto(origin, {waitUntil: "networkidle"}),
      pageB.goto(origin, {waitUntil: "networkidle"}),
    ]);

    const capsA = await pageA.evaluate(() => window.probeCapabilities());
    const capsB = await pageB.evaluate(() => window.probeCapabilities());

    // BroadcastChannel cross-tab coordination.
    let broadcast = { supported: capsA.broadcast_channel && capsB.broadcast_channel, delivered: null, messages: [] };
    if (broadcast.supported) {
      await pageA.evaluate(() => window.startChannel("chaptera:doc:probe"));
      await pageB.evaluate(() => window.startChannel("chaptera:doc:probe"));
      await pageA.evaluate(() => window.sendChannel({type: "hello", from: "A", seq: 1}));
      await pageB.waitForFunction(() => window.channelMessages().length >= 1, null, {timeout: 2000});
      const messages = await pageB.evaluate(() => window.channelMessages());
      broadcast = {
        supported: true,
        delivered: messages.some((m) => m?.type === "hello" && m?.from === "A"),
        messages,
      };
      if (!broadcast.delivered) throw new Error("BroadcastChannel message not delivered cross-tab");
    }

    // Web Locks ownership and release-on-tab-close behavior.
    let lock = { supported: capsA.web_locks && capsB.web_locks, a_acquired: null, b_while_a: null, b_after_a_close: null };
    if (lock.supported) {
      const held = await pageA.evaluate(() => window.holdLock("chaptera:doc:probe"));
      if (!held.acquired) throw new Error("tab A failed to acquire Web Lock");
      const during = await pageB.evaluate(() => window.tryLock("chaptera:doc:probe"));
      if (during.acquired) throw new Error("tab B acquired supposedly exclusive Web Lock");
      await pageA.close();
      pageA = null;
      await pageB.waitForTimeout(50);
      const after = await pageB.evaluate(() => window.tryLock("chaptera:doc:probe"));
      if (!after.acquired) throw new Error("tab B failed to acquire Web Lock after tab A close");
      lock = { supported: true, a_acquired: true, b_while_a: during.acquired, b_after_a_close: after.acquired };
    }

    // Shared IndexedDB pending-intent queue and bounded performance baseline.
    await pageB.evaluate(() => window.clearPending());
    const idbWrite100 = await pageB.evaluate(() => window.writePendingBatch(100, "b100"));
    const idbWrite1000 = await pageB.evaluate(() => window.writePendingBatch(1000, "b1000"));
    const idbCountBeforeRestart = await pageB.evaluate(() => window.countPending());
    if (idbCountBeforeRestart.count !== 1100) {
      throw new Error("unexpected IndexedDB count before restart: " + idbCountBeforeRestart.count);
    }

    // A new tab sees the same origin-local queue.
    const pageC = await firstContext.newPage();
    await pageC.goto(origin, {waitUntil: "networkidle"});
    const idbCountCrossTab = await pageC.evaluate(() => window.countPending());
    if (idbCountCrossTab.count !== 1100) throw new Error("IndexedDB cross-tab visibility failed");

    // OPFS cross-tab if supported.
    const marker = { document_id: "doc:probe", pending_generation: 7, marker: "chaptera-opfs-probe" };
    const opfsWrite = await pageB.evaluate((marker) => window.writeOpfs(marker), marker);
    const opfsCrossTab = await pageC.evaluate(() => window.readOpfs());
    if (opfsWrite.supported && JSON.stringify(opfsCrossTab.value) !== JSON.stringify(marker)) {
      throw new Error("OPFS cross-tab read mismatch");
    }

    const persistenceRequest = await pageB.evaluate(() => window.requestPersistence());
    const capsAfterPersistRequest = await pageB.evaluate(() => window.probeCapabilities());

    await firstContext.close();
    firstContext = null;

    // Browser restart with same profile directory.
    const secondContext = await openPersistent(browserType, profile);
    const pageRestart = secondContext.pages()[0] ?? await secondContext.newPage();
    await pageRestart.goto(origin, {waitUntil: "networkidle"});
    const capsAfterRestart = await pageRestart.evaluate(() => window.probeCapabilities());
    const idbAfterRestart = await pageRestart.evaluate(() => window.countPending());
    const opfsAfterRestart = await pageRestart.evaluate(() => window.readOpfs());

    if (idbAfterRestart.count !== 1100) {
      throw new Error("IndexedDB did not survive persistent browser-context restart");
    }
    if (opfsWrite.supported && JSON.stringify(opfsAfterRestart.value) !== JSON.stringify(marker)) {
      throw new Error("OPFS did not survive persistent browser-context restart");
    }

    const receipt = {
      receipt_kind: "chaptera.browser-multitab-local-recovery.v1",
      browser_engine: BROWSER_NAME,
      real_pub: false,
      product_acceptance: false,
      capabilities_initial: capsA,
      capabilities_second_tab: capsB,
      broadcast_channel: broadcast,
      web_locks: lock,
      indexeddb: {
        cross_tab_visible: idbCountCrossTab.count === 1100,
        survives_browser_context_restart: idbAfterRestart.count === 1100,
        write_100: idbWrite100,
        write_1000: idbWrite1000,
        count_before_restart: idbCountBeforeRestart,
        count_cross_tab: idbCountCrossTab,
        count_after_restart: idbAfterRestart,
      },
      opfs: {
        supported: opfsWrite.supported,
        cross_tab_read_equal: opfsWrite.supported ? JSON.stringify(opfsCrossTab.value) === JSON.stringify(marker) : null,
        survives_browser_context_restart: opfsWrite.supported ? JSON.stringify(opfsAfterRestart.value) === JSON.stringify(marker) : null,
      },
      storage_persistence: {
        request: persistenceRequest,
        after_request: capsAfterPersistRequest.storage_persisted,
        after_restart: capsAfterRestart.storage_persisted,
        estimate_after_restart: capsAfterRestart.storage_estimate,
      },
      interpretation_guardrails: {
        web_locks_are_browser_local_coordination_not_document_authority: true,
        indexeddb_and_opfs_are_recovery_cache_not_canonical_state: true,
        server_conflict_authority_still_required: true,
      },
      note:
        "Headless GitHub-runner browser behavior on one localhost origin. Useful for multi-tab ownership/recovery architecture, not for eviction guarantees, quota policy, offline product acceptance, or server authority."
    };

    fs.writeFileSync(
      path.join(TARGET, BROWSER_NAME + "-receipt.json"),
      JSON.stringify(receipt, null, 2) + "\n"
    );
    process.stdout.write(JSON.stringify(receipt, null, 2) + "\n");
    await secondContext.close();
  } finally {
    if (firstContext) await firstContext.close().catch(() => {});
    await new Promise((resolve) => serverState.server.close(resolve));
    fs.rmSync(profile, {recursive: true, force: true});
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
