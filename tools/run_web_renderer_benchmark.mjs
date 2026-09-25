#!/usr/bin/env node
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, firefox } from "playwright";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TARGET = path.join(ROOT, "target", "web-render");
const FIXTURE_DIR = path.join(ROOT, "packages", "protocol", "scene", "v1", "fixtures");
const BROWSER_NAME = process.argv[2] ?? "chromium";
const REAL_SCENE_PATH = process.argv[3] ? path.resolve(process.argv[3]) : null;
const ENGINES = { chromium, firefox };

if (!(BROWSER_NAME in ENGINES)) {
  throw new Error("unsupported browser " + BROWSER_NAME);
}

function mimeFor(filePath) {
  if (filePath.endsWith(".html")) return "text/html; charset=utf-8";
  if (filePath.endsWith(".mjs") || filePath.endsWith(".js")) {
    return "text/javascript; charset=utf-8";
  }
  if (filePath.endsWith(".json")) return "application/json; charset=utf-8";
  return "application/octet-stream";
}

function startServer() {
  return new Promise((resolve) => {
    const server = http.createServer((request, response) => {
      try {
        const url = new URL(request.url ?? "/", "http://localhost");
        const pathname = decodeURIComponent(url.pathname);
        const relative = pathname === "/" ? "/apps/web/render-benchmark.html" : pathname;
        const filePath = path.resolve(ROOT, "." + relative);
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
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      resolve({ server, port: address.port });
    });
  });
}

function readFixture(name) {
  return fs.readFileSync(path.join(FIXTURE_DIR, name), "utf8");
}

function stressSnapshot() {
  const base = JSON.parse(readFixture("group-table.json"));
  base.document_id = "19999999-9999-4999-8999-999999999999";
  base.source_hash = "e".repeat(64);
  base.revision_id = "sha256:" + "e".repeat(64);
  base.snapshot_id = "sha256:" + "0".repeat(64);
  base.pages[0].page_id = "19999999-9999-4999-8999-999999999998";
  base.pages[0].width_emu = 20000000;
  base.pages[0].height_emu = 30000000;
  base.nodes = [];
  base.stories = [];
  base.story_frames = [];
  base.resources = [];
  base.diagnostics = [];
  base.capabilities = [
    { key: "render.geometry", state: "supported", note: "synthetic stress only" }
  ];
  base.fidelity = { state: "partial", reasons: ["synthetic_stress_not_real_pub"] };
  base.stacking_fidelity = "exact";

  const pageId = base.pages[0].page_id;
  for (let i = 0; i < 5000; i += 1) {
    const tail = String(i + 1).padStart(12, "0");
    base.nodes.push({
      node_id: "29999999-9999-4999-8999-" + tail,
      page_id: pageId,
      parent_node_id: null,
      kind: "shape",
      bounds: {
        x: (i % 80) * 220000 - (i % 97 === 0 ? 300000 : 0),
        y: Math.floor(i / 80) * 220000,
        width: 180000,
        height: 180000
      },
      z_order: i,
      paint_order: i,
      paint_id: "paint.blue",
      resource_id: null,
      transform: { a: "1", b: "0", c: "0", d: "1", tx: 0, ty: 0 }
    });
  }
  return JSON.stringify(base);
}

async function main() {
  fs.mkdirSync(TARGET, { recursive: true });
  const serverState = await startServer();
  const browserType = ENGINES[BROWSER_NAME];
  const args = BROWSER_NAME === "chromium"
    ? ["--enable-webgl", "--ignore-gpu-blocklist", "--use-angle=swiftshader"]
    : [];
  const browser = await browserType.launch({ headless: true, args });

  try {
    const page = await browser.newPage({ viewport: { width: 1200, height: 800 } });
    await page.goto(
      "http://127.0.0.1:" + serverState.port + "/apps/web/render-benchmark.html",
      { waitUntil: "networkidle" }
    );

    const inputs = REAL_SCENE_PATH
      ? [[
          path.basename(REAL_SCENE_PATH),
          "real_pub_scene",
          fs.readFileSync(REAL_SCENE_PATH, "utf8")
        ]]
      : [
          ["simple-text.json", "protocol_fixture", readFixture("simple-text.json")],
          ["exact-image.json", "protocol_fixture", readFixture("exact-image.json")],
          ["group-table.json", "protocol_fixture", readFixture("group-table.json")],
          ["partial-unsupported.json", "protocol_fixture", readFixture("partial-unsupported.json")],
          ["synthetic-stress-5000", "synthetic_stress", stressSnapshot()]
        ];

    const cases = [];
    for (const [fixture, inputClass, payloadText] of inputs) {
      const result = await page.evaluate(
        async ({ payloadText, fixture, inputClass, realPub }) =>
          window.runRenderBenchmark(payloadText, {
            fixture,
            input_class: inputClass,
            real_pub: realPub,
            representative_corpus: realPub,
            technology_decision_allowed: false,
            focus_first_populated_page: realPub
          }),
        { payloadText, fixture, inputClass, realPub: Boolean(REAL_SCENE_PATH) }
      );
      cases.push(result);
    }

    const screenshotPayload = REAL_SCENE_PATH
      ? fs.readFileSync(REAL_SCENE_PATH, "utf8")
      : readFixture("group-table.json");
    const screenshotLabel = REAL_SCENE_PATH ? "real-sample-newsletter" : "group-table";
    for (const renderer of ["svg", "canvas2d", "webgl2-hybrid"]) {
      const state = await page.evaluate(
        async ({ payloadText, renderer, realPub }) =>
          window.renderForScreenshot(payloadText, renderer, {
            focus_first_populated_page: realPub,
            include_overlay: !realPub
          }),
        { payloadText: screenshotPayload, renderer, realPub: Boolean(REAL_SCENE_PATH) }
      );
      if (state.available) {
        await page.locator("#host").screenshot({
          path: path.join(
            TARGET,
            BROWSER_NAME + "-" + screenshotLabel + "-" + renderer + ".png"
          )
        });
      }
    }

    const realPub = Boolean(REAL_SCENE_PATH);
    const receipt = {
      receipt_kind: realPub
        ? "chaptera.real-pub-renderer-benchmark.v2"
        : "synthetic_renderer_benchmark_preflight",
      browser_engine: BROWSER_NAME,
      real_pub: realPub,
      representative_corpus: realPub,
      technology_decision_allowed: false,
      note: realPub
        ? "Pinned real SampleNewsletter BrowserSceneSnapshotV1 measurement focused on the first populated page. V2 corrects the V1 blank-page targeting bug; final WEB-RENDER-01 selection remains gated on WEB-COLOR-SURFACE-01."
        : "Protocol fixtures and synthetic stress only. Do not use this receipt as the final WEB-RENDER-01 technology decision.",
      cases
    };
    fs.writeFileSync(
      path.join(TARGET, BROWSER_NAME + "-receipt.json"),
      JSON.stringify(receipt, null, 2) + "\n"
    );
    process.stdout.write(JSON.stringify(receipt, null, 2) + "\n");
  } finally {
    await browser.close();
    await new Promise((resolve) => serverState.server.close(resolve));
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
