#!/usr/bin/env node
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, firefox } from "playwright";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TARGET = path.join(ROOT, "target", "web-accessibility-renderer");
const FIXTURE = path.join(ROOT, "packages", "protocol", "scene", "v1", "fixtures", "group-table.json");
const BROWSER_NAME = process.argv[2] ?? "chromium";
const ENGINES = { chromium, firefox };
if (!(BROWSER_NAME in ENGINES)) throw new Error("unsupported browser " + BROWSER_NAME);

function mimeFor(filePath) {
  if (filePath.endsWith(".html")) return "text/html; charset=utf-8";
  if (filePath.endsWith(".mjs") || filePath.endsWith(".js")) return "text/javascript; charset=utf-8";
  if (filePath.endsWith(".json")) return "application/json; charset=utf-8";
  return "application/octet-stream";
}

function startServer() {
  return new Promise((resolve) => {
    const server = http.createServer((request, response) => {
      try {
        const url = new URL(request.url ?? "/", "http://localhost");
        const rel = url.pathname === "/" ? "/apps/web/render-benchmark.html" : decodeURIComponent(url.pathname);
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

async function domStats(page, selector) {
  return page.locator(selector).evaluate((root) => ({
    descendant_elements: root.querySelectorAll("*").length,
    focusable_descendants: root.querySelectorAll(
      'a[href],button,input,select,textarea,[tabindex]:not([tabindex="-1"])'
    ).length,
    explicit_roles: Array.from(root.querySelectorAll("[role]")).map((el) => ({
      role: el.getAttribute("role"),
      aria_label: el.getAttribute("aria-label"),
      tag: el.tagName.toLowerCase(),
    })),
    aria_labeled_descendants: root.querySelectorAll("[aria-label],[aria-labelledby]").length,
  }));
}

function snapshotLineCount(value) {
  return typeof value === "string" && value.trim() ? value.trim().split("\n").length : 0;
}

async function main() {
  fs.mkdirSync(TARGET, {recursive: true});
  const fixtureText = fs.readFileSync(FIXTURE, "utf8");
  const fixture = JSON.parse(fixtureText);
  const serverState = await startServer();
  const browserType = ENGINES[BROWSER_NAME];
  const args = BROWSER_NAME === "chromium"
    ? ["--enable-webgl", "--ignore-gpu-blocklist", "--use-angle=swiftshader"]
    : [];
  const browser = await browserType.launch({headless: true, args});

  try {
    const page = await browser.newPage({viewport: {width: 1200, height: 800}});
    await page.goto(
      "http://127.0.0.1:" + serverState.port + "/apps/web/render-benchmark.html",
      {waitUntil: "networkidle"}
    );

    const backends = [];
    for (const renderer of ["svg", "canvas2d", "webgl2-hybrid"]) {
      const state = await page.evaluate(
        async ({payloadText, renderer}) => window.renderForScreenshot(payloadText, renderer),
        {payloadText: fixtureText, renderer}
      );
      const host = page.locator("#host");
      const ariaSnapshot = state.available ? await host.ariaSnapshot() : "";
      const stats = await domStats(page, "#host");
      backends.push({
        renderer,
        available: state.available,
        aria_snapshot: ariaSnapshot,
        aria_snapshot_lines: snapshotLineCount(ariaSnapshot),
        dom: stats,
      });
    }

    await page.evaluate((payloadText) => {
      const scene = JSON.parse(payloadText);
      document.querySelector("#chaptera-a11y-mirror")?.remove();
      const mirror = document.createElement("div");
      mirror.id = "chaptera-a11y-mirror";
      mirror.setAttribute("role", "tree");
      mirror.setAttribute("aria-label", "Publication objects");
      mirror.style.position = "fixed";
      mirror.style.left = "-10000px";
      mirror.style.top = "0";
      mirror.style.width = "1px";
      mirror.style.height = "1px";
      mirror.style.overflow = "hidden";
      scene.nodes.forEach((node, index) => {
        const item = document.createElement("button");
        item.type = "button";
        item.setAttribute("role", "treeitem");
        item.setAttribute("aria-label", node.kind + " " + node.node_id);
        item.dataset.nodeId = node.node_id;
        item.tabIndex = index === 0 ? 0 : -1;
        mirror.appendChild(item);
      });
      document.body.appendChild(mirror);
    }, fixtureText);

    const mirror = page.locator("#chaptera-a11y-mirror");
    const mirrorSnapshot = await mirror.ariaSnapshot();
    const mirrorStats = await domStats(page, "#chaptera-a11y-mirror");
    const firstItem = page.locator("#chaptera-a11y-mirror [role=treeitem]").first();
    await firstItem.focus();
    const active = await page.evaluate(() => ({
      role: document.activeElement?.getAttribute("role"),
      node_id: document.activeElement?.dataset?.nodeId ?? null,
      aria_label: document.activeElement?.getAttribute("aria-label"),
    }));

    const receipt = {
      receipt_kind: "chaptera.renderer-accessibility-surface.v1",
      browser_engine: BROWSER_NAME,
      real_pub: false,
      representative_scene: false,
      product_acceptance: false,
      scene_nodes: fixture.nodes.length,
      renderer_backends: backends,
      semantic_mirror: {
        aria_snapshot: mirrorSnapshot,
        aria_snapshot_lines: snapshotLineCount(mirrorSnapshot),
        dom: mirrorStats,
        focused_first_item: active,
        node_count_matches_scene: mirrorStats.explicit_roles.filter((x) => x.role === "treeitem").length === fixture.nodes.length,
      },
      interpretation_guardrails: {
        accessibility_identity_must_use_semantic_node_id_not_renderer_dom_id: true,
        renderer_backend_must_not_be_accessibility_authority: true,
        mirror_is_pattern_probe_not_final_accessibility_architecture: true,
      },
      note:
        "Synthetic Scene fixture only. Measures current renderer DOM/accessibility exposure and a renderer-independent semantic mirror pattern. Does not test a screen reader, production focus routing, high contrast, reduced motion, or full WCAG acceptance."
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
