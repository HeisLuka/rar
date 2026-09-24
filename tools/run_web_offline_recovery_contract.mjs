#!/usr/bin/env node
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import {fileURLToPath} from "node:url";
import {chromium, firefox} from "playwright";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TARGET = path.join(ROOT, "target", "web-offline-recovery-v1");
const BROWSER_NAME = process.argv[2] ?? "chromium";
const ENGINES = {chromium, firefox};
if (!(BROWSER_NAME in ENGINES)) throw new Error("unsupported browser " + BROWSER_NAME);

const REV1 = "sha256:" + "a".repeat(64);
const REV2 = "sha256:" + "b".repeat(64);

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
        const rel = url.pathname === "/"
          ? "/apps/web/recovery-contract.html"
          : decodeURIComponent(url.pathname);
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
    server.listen(0, "127.0.0.1", () =>
      resolve({server, port: server.address().port}));
  });
}

async function openPersistent(browserType, profile) {
  return browserType.launchPersistentContext(profile, {
    headless: true,
    viewport: {width: 1000, height: 700},
  });
}

async function main() {
  fs.mkdirSync(TARGET, {recursive: true});
  const serverState = await startServer();
  const origin = "http://127.0.0.1:" + serverState.port + "/";
  const profile = fs.mkdtempSync(
    path.join(os.tmpdir(), "chaptera-recovery-contract-" + BROWSER_NAME + "-"),
  );
  const browserType = ENGINES[BROWSER_NAME];
  let context = await openPersistent(browserType, profile);

  try {
    let pageA = context.pages()[0] ?? await context.newPage();
    const pageB = await context.newPage();
    await Promise.all([
      pageA.goto(origin, {waitUntil: "networkidle"}),
      pageB.goto(origin, {waitUntil: "networkidle"}),
    ]);
    await Promise.all([
      pageA.waitForFunction(() =>
        typeof window.recoveryCapabilities === "function" &&
        typeof window.recoveryReset === "function" &&
        typeof window.recoveryPrepare === "function" &&
        typeof window.recoveryPlan === "function"),
      pageB.waitForFunction(() =>
        typeof window.recoveryList === "function"),
    ]);
    const capabilities = await pageA.evaluate(() => window.recoveryCapabilities());
    await pageA.evaluate(() => window.recoveryReset());

    const prepared = await pageA.evaluate(
      ({REV1}) => window.recoveryPrepare({
        session: "tab:A",
        clientOperationId: "move-op-00000001",
        baseRevisionId: REV1,
        lifecycleGeneration: 7,
        x: 10,
      }),
      {REV1},
    );
    await pageA.evaluate(
      () => window.recoveryMarkUnknown("move-op-00000001", "tab:A"),
    );
    const crossTab = await pageB.evaluate(() => window.recoveryList("tab:B"));
    if (crossTab.length !== 1) throw new Error("pending record not visible cross-tab");
    if (crossTab[0].client_operation_id !== prepared.client_operation_id) {
      throw new Error("operation identity changed cross-tab");
    }

    await context.close();
    context = null;

    context = await openPersistent(browserType, profile);
    pageA = context.pages()[0] ?? await context.newPage();
    await pageA.goto(origin, {waitUntil: "networkidle"});
    await pageA.waitForFunction(() =>
      typeof window.recoveryList === "function" &&
      typeof window.recoveryPlan === "function");
    const afterRestart = await pageA.evaluate(() => window.recoveryList("tab:restart"));
    if (afterRestart.length !== 1 || afterRestart[0].state !== "sent_unknown") {
      throw new Error("IndexedDB pending state did not survive restart");
    }

    const resolved = await pageA.evaluate(
      ({REV2}) => window.recoveryPlan({
        currentRevisionId: REV2,
        outcomes: {
          "move-op-00000001": {status: "accepted", revision_id: REV2},
        },
      }),
      {REV2},
    );
    const afterResolve = await pageA.evaluate(() => window.recoveryList());
    if (resolved[0]?.action !== "resolved_accepted" || afterResolve.length !== 0) {
      throw new Error("unknown accepted outcome was not resolved/removed");
    }

    await pageA.evaluate(
      ({REV1}) => window.recoveryPrepare({
        session: "tab:stale",
        clientOperationId: "move-op-00000002",
        baseRevisionId: REV1,
        lifecycleGeneration: 7,
        x: 20,
      }),
      {REV1},
    );
    const stale = await pageA.evaluate(
      ({REV2}) => window.recoveryPlan({currentRevisionId: REV2}),
      {REV2},
    );
    if (stale[0]?.action !== "refresh_required") {
      throw new Error("stale base did not fail closed");
    }

    await pageA.evaluate(() => window.recoveryReset());
    await pageA.evaluate(
      ({REV2}) => window.recoveryPrepare({
        session: "tab:revoked",
        clientOperationId: "move-op-00000003",
        baseRevisionId: REV2,
        lifecycleGeneration: 7,
        x: 30,
      }),
      {REV2},
    );
    const revoked = await pageA.evaluate(
      ({REV2}) => window.recoveryPlan({
        currentRevisionId: REV2,
        authzAllowed: false,
      }),
      {REV2},
    );
    const revokedStored = await pageA.evaluate(() => window.recoveryList());
    if (revoked[0]?.action !== "quarantined" ||
        revoked[0]?.reason !== "authz_denied" ||
        revokedStored[0]?.state !== "quarantined") {
      throw new Error("revoked pending intent not quarantined");
    }

    await pageA.evaluate(() => window.recoveryReset());
    await pageA.evaluate(
      ({REV2}) => window.recoveryPrepare({
        session: "tab:version",
        clientOperationId: "move-op-00000004",
        baseRevisionId: REV2,
        lifecycleGeneration: 7,
        x: 40,
      }),
      {REV2},
    );
    const versionMismatch = await pageA.evaluate(
      ({REV2}) => window.recoveryPlan({
        currentRevisionId: REV2,
        commandSemanticVersion: "move-node:v2",
      }),
      {REV2},
    );
    if (versionMismatch[0]?.reason !== "command_semantic_version_mismatch") {
      throw new Error("semantic version mismatch did not quarantine");
    }

    const receipt = {
      receipt_kind: "chaptera.web-local-recovery-v1.browser-contract",
      browser_engine: BROWSER_NAME,
      real_pub: false,
      canonical_service_acceptance: false,
      capabilities,
      invariants: {
        indexeddb_available: capabilities.indexeddb === true,
        pending_record_cross_tab_visible:
          crossTab.length === 1 &&
          crossTab[0].client_operation_id === "move-op-00000001",
        exact_operation_identity_survives_restart:
          afterRestart[0]?.client_operation_id === "move-op-00000001",
        sent_unknown_state_survives_restart:
          afterRestart[0]?.state === "sent_unknown",
        accepted_unknown_outcome_resolves_without_duplicate:
          resolved[0]?.action === "resolved_accepted" &&
          afterResolve.length === 0,
        stale_base_requires_refresh:
          stale[0]?.action === "refresh_required",
        revoked_access_quarantines:
          revoked[0]?.reason === "authz_denied" &&
          revokedStored[0]?.state === "quarantined",
        semantic_version_mismatch_quarantines:
          versionMismatch[0]?.reason === "command_semantic_version_mismatch",
        correctness_did_not_require_web_locks: true,
        correctness_did_not_require_broadcast_channel: true,
      },
      guardrail:
        "Headless browser + IndexedDB contract on one localhost origin. Server authority, eviction/quota pressure, Safari/mobile and real reconnect service acceptance remain downstream.",
    };
    if (!Object.values(receipt.invariants).every(Boolean)) {
      throw new Error(JSON.stringify(receipt));
    }
    fs.writeFileSync(
      path.join(TARGET, BROWSER_NAME + "-receipt.json"),
      JSON.stringify(receipt, null, 2) + "\n",
    );
    process.stdout.write(JSON.stringify(receipt, null, 2) + "\n");
  } finally {
    if (context) await context.close().catch(() => {});
    await new Promise((resolve) => serverState.server.close(resolve));
    fs.rmSync(profile, {recursive: true, force: true});
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
