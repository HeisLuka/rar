#!/usr/bin/env node
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, firefox } from "playwright";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TARGET = path.join(ROOT, "target", "web-scene-window-replace");
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
        const relative = url.pathname === "/" ? "/apps/web/render-benchmark.html" : decodeURIComponent(url.pathname);
        const filePath = path.resolve(ROOT, "." + relative);
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

function uuid(prefix, n) {
  return prefix + "-0000-4000-8000-" + String(n).padStart(12, "0");
}

function makeScene(pageCount, nodesPerPage) {
  const pages = [];
  const nodes = [];
  let nodeCounter = 0;
  for (let p = 0; p < pageCount; p += 1) {
    const pageId = uuid("10000000", p + 1);
    pages.push({page_id: pageId, order: p, width_emu: 7772400, height_emu: 10058400});
    for (let n = 0; n < nodesPerPage; n += 1) {
      nodeCounter += 1;
      nodes.push({
        node_id: uuid("20000000", nodeCounter),
        page_id: pageId,
        parent_node_id: null,
        kind: "shape",
        bounds: {
          x: 400000 + (n % 5) * 1200000,
          y: 400000 + Math.floor(n / 5) * 1200000,
          width: 800000,
          height: 800000
        },
        z_order: n,
        paint_order: n,
        paint_id: null,
        resource_id: null,
        transform: {a: "1", b: "0", c: "0", d: "1", tx: 0, ty: 0}
      });
    }
  }
  return {
    protocol_version: "chaptera.scene.v1",
    document_id: "19999999-9999-4999-8999-999999999999",
    source_hash: "d".repeat(64),
    revision_id: "sha256:" + "d".repeat(64),
    snapshot_id: "sha256:" + "0".repeat(64),
    layout_environment: {
      environment_id: "sha256:" + "1".repeat(64),
      engine_revision: "synthetic-window-replace-v1",
      font_set_fingerprint: "sha256:" + "2".repeat(64),
      resource_fingerprint: "sha256:" + "3".repeat(64)
    },
    stacking_fidelity: "exact",
    pages,
    nodes,
    stories: [],
    story_frames: [],
    paints: [],
    resources: [],
    diagnostics: [],
    capabilities: [{key: "render.geometry", state: "supported", note: "synthetic window replacement only"}],
    fidelity: {state: "partial", reasons: ["synthetic_multipage_not_real_pub"]}
  };
}

function windowScene(full, center, radius = 1) {
  const lo = Math.max(0, center - radius);
  const hi = Math.min(full.pages.length - 1, center + radius);
  const pages = full.pages.filter((p) => p.order >= lo && p.order <= hi);
  const pageIds = new Set(pages.map((p) => p.page_id));
  return {...full, pages, nodes: full.nodes.filter((n) => pageIds.has(n.page_id))};
}

async function main() {
  fs.mkdirSync(TARGET, {recursive: true});
  const serverState = await startServer();
  const browserType = ENGINES[BROWSER_NAME];
  const args = BROWSER_NAME === "chromium"
    ? ["--enable-webgl", "--ignore-gpu-blocklist", "--use-angle=swiftshader"]
    : [];
  const browser = await browserType.launch({headless: true, args});
  try {
    const page = await browser.newPage({viewport: {width: 1200, height: 800}});
    await page.goto("http://127.0.0.1:" + serverState.port + "/apps/web/render-benchmark.html", {waitUntil: "networkidle"});

    const cases = [];
    for (const sourcePages of [100, 500]) {
      const full = makeScene(sourcePages, 10);
      const mid = Math.floor(sourcePages / 2);
      const windows = [];
      for (let i = -10; i < 10; i += 1) windows.push(windowScene(full, mid + i, 1));

      const result = await page.evaluate(async ({windows}) => {
        const {createRenderer, RENDERER_KINDS} = await import("/apps/web/render-v1.mjs");
        const host = document.querySelector("#host");
        const view = {
          emu_per_css_px: 9525,
          zoom: 1,
          pan_x_css_px: 0,
          pan_y_css_px: 0,
          page_gap_css_px: 32,
          page_margin_css_px: 24
        };
        const raf2 = () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
        const stats = (values) => {
          const sorted = [...values].sort((a,b) => a-b);
          const q = (p) => sorted[Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * p))];
          return {samples: values.length, p50_ms: q(0.5), p95_ms: q(0.95), max_ms: Math.max(...values)};
        };
        const out = [];
        for (const kind of RENDERER_KINDS) {
          let renderer = null;
          const sync = [];
          const settled = [];
          let available = true;
          let unavailable_reason = null;
          for (const snapshot of windows) {
            if (renderer) renderer.destroy();
            host.replaceChildren();
            const start = performance.now();
            renderer = createRenderer(kind, host, snapshot, view);
            const syncDone = performance.now();
            const st = renderer.stats();
            if (!st.available) {
              available = false;
              unavailable_reason = st.unavailable_reason;
              renderer.destroy();
              renderer = null;
              break;
            }
            await raf2();
            const settledDone = performance.now();
            sync.push(syncDone - start);
            settled.push(settledDone - start);
          }
          if (renderer) renderer.destroy();
          out.push({
            renderer: kind,
            available,
            unavailable_reason,
            replace_sync: available ? stats(sync) : null,
            replace_settled: available ? stats(settled) : null
          });
        }
        return out;
      }, {windows});

      const payloads = windows.map((w) => Buffer.byteLength(JSON.stringify(w), "utf8"));
      cases.push({
        source_pages: sourcePages,
        window_pages: 3,
        navigation_steps: windows.length,
        window_payload_bytes_min: Math.min(...payloads),
        window_payload_bytes_max: Math.max(...payloads),
        renderers: result
      });
    }

    const receipt = {
      receipt_kind: "chaptera.synthetic-window-replacement-navigation.v1",
      browser_engine: BROWSER_NAME,
      real_pub: false,
      representative_corpus: false,
      scene_patch_decision_allowed: false,
      technology_decision_allowed: false,
      window_radius_pages: 1,
      cases,
      note: "Measures repeated whole-window renderer replacement for a 3-page working set. This may justify deferring ScenePatch on synthetic geometry, but cannot settle representative real-PUB/resource behavior."
    };
    const out = path.join(TARGET, BROWSER_NAME + "-receipt.json");
    fs.writeFileSync(out, JSON.stringify(receipt, null, 2) + "\n");
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
