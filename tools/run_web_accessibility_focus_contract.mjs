#!/usr/bin/env node
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import {fileURLToPath} from "node:url";
import {chromium, firefox} from "playwright";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TARGET = path.join(ROOT, "target", "web-accessibility-focus-v1");
const BROWSER_NAME = process.argv[2] ?? "chromium";
const ENGINES = {chromium, firefox};
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
        const rel = url.pathname === "/"
          ? "/apps/web/accessibility-focus-contract.html"
          : decodeURIComponent(url.pathname);
        const filePath = path.resolve(ROOT, "." + rel);
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
    server.listen(0, "127.0.0.1", () =>
      resolve({server, port: server.address().port}));
  });
}

function stableSemantic(state) {
  return {
    active_node_id: state.active_node_id,
    selected_node_ids: state.selected_node_ids,
    logical_order: state.logical_order,
    tab_stop_node_ids: state.tab_stop_node_ids,
    names: state.names,
  };
}

async function main() {
  fs.mkdirSync(TARGET, {recursive: true});
  const serverState = await startServer();
  const args = BROWSER_NAME === "chromium"
    ? ["--enable-webgl", "--ignore-gpu-blocklist", "--use-angle=swiftshader"]
    : [];
  const browser = await ENGINES[BROWSER_NAME].launch({headless: true, args});

  try {
    const page = await browser.newPage({viewport: {width: 1200, height: 800}});
    const origin = "http://127.0.0.1:" + serverState.port + "/";
    await page.goto(origin, {waitUntil: "networkidle"});
    await page.waitForFunction(() =>
      !!window.__a11yFocusContract &&
      typeof window.__a11yFocusContract.semanticState === "function");

    const summary = await page.evaluate(() => window.__a11yFocusContract.sceneSummary());
    const initial = await page.evaluate(() => window.__a11yFocusContract.semanticState());
    if (initial.logical_order.length < 3) throw new Error("fixture must expose at least three semantic nodes");

    const focusTarget = initial.logical_order[1];
    await page.evaluate(
      (nodeId) => window.__a11yFocusContract.focusNode(nodeId),
      focusTarget,
    );
    await page.evaluate(
      (nodeId) => window.__a11yFocusContract.selectNodes([nodeId]),
      focusTarget,
    );
    const baseline = await page.evaluate(() => window.__a11yFocusContract.semanticState());
    const baselineStable = stableSemantic(baseline);
    const ariaSnapshot = await page.locator("#semantic-mirror").ariaSnapshot();

    const rendererTransitions = [];
    for (const kind of ["svg", "canvas2d", "webgl2-hybrid", "svg"]) {
      const transition = await page.evaluate(
        async (rendererKind) => window.__a11yFocusContract.setRenderer(rendererKind),
        kind,
      );
      rendererTransitions.push(transition);
      if (JSON.stringify(stableSemantic(transition.semantic_state)) !== JSON.stringify(baselineStable)) {
        throw new Error("semantic accessibility identity changed after renderer switch: " + kind);
      }
      if (transition.active_element.node_id !== focusTarget) {
        throw new Error("DOM semantic focus changed after renderer switch: " + kind);
      }
    }

    // Scene focus owns document commands.
    await page.evaluate(() => window.__a11yFocusContract.clearCommands());
    await page.locator(`[role="treeitem"][data-node-id="${focusTarget}"]`).focus();
    await page.keyboard.press("Delete");
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press("Control+z");
    let commands = await page.evaluate(() => window.__a11yFocusContract.commands());
    const firstThree = commands.map((x) => x.command);
    if (JSON.stringify(firstThree) !== JSON.stringify([
      "delete_selection",
      "nudge_left",
      "document_undo",
    ])) {
      throw new Error("scene command routing mismatch: " + JSON.stringify(firstThree));
    }

    // Roving scene navigation owns exactly one tab stop.
    await page.keyboard.press("ArrowDown");
    commands = await page.evaluate(() => window.__a11yFocusContract.commands());
    if (commands.at(-1)?.command !== "focus_next_scene_item") {
      throw new Error("scene ArrowDown did not route to semantic focus navigation");
    }
    const afterArrow = await page.evaluate(() => ({
      semantic: window.__a11yFocusContract.semanticState(),
      active: window.__a11yFocusContract.activeElement(),
      stops: window.__a11yFocusContract.tabStops(),
    }));
    if (afterArrow.semantic.tab_stop_node_ids.length !== 1) {
      throw new Error("semantic scene must expose one roving tab stop");
    }
    if (afterArrow.active.node_id !== afterArrow.semantic.active_node_id) {
      throw new Error("DOM focus and semantic scene focus diverged");
    }

    // Inspector does not leak document commands.
    const beforeInspector = commands.length;
    await page.locator("#inspector").focus();
    await page.keyboard.press("Delete");
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press("Control+z");
    const afterInspector = await page.evaluate(() => window.__a11yFocusContract.commands());
    if (afterInspector.length !== beforeInspector) {
      throw new Error("document command leaked from inspector");
    }

    // Story editor does not leak document commands.
    const beforeStory = afterInspector.length;
    const story = page.locator("#story-editor");
    await story.focus();
    await page.keyboard.press("Backspace");
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press("Control+z");
    const afterStory = await page.evaluate(() => window.__a11yFocusContract.commands());
    if (afterStory.length !== beforeStory) {
      throw new Error("document command leaked from Story editor");
    }

    // Explicit composition state fences scene-global shortcuts even after DOM focus moves.
    await story.evaluate((el) =>
      el.dispatchEvent(new CompositionEvent("compositionstart", {bubbles: true, data: "漢"})));
    if (!(await page.evaluate(() => window.__a11yFocusContract.composing()))) {
      throw new Error("explicit composition fence did not activate");
    }
    const beforeComposition = afterStory.length;
    const currentSceneNode = afterArrow.semantic.active_node_id;
    await page.locator(`[role="treeitem"][data-node-id="${currentSceneNode}"]`).focus();
    await page.keyboard.press("Delete");
    await page.keyboard.press("Escape");
    const afterComposition = await page.evaluate(() => window.__a11yFocusContract.commands());
    if (afterComposition.length !== beforeComposition) {
      throw new Error("document command leaked while explicit composition fence active");
    }
    await story.evaluate((el) =>
      el.dispatchEvent(new CompositionEvent("compositionend", {bubbles: true, data: "漢"})));

    // Escape is context-routed for transient scene work.
    await page.locator(`[role="treeitem"][data-node-id="${currentSceneNode}"]`).focus();
    await page.evaluate(() => window.__a11yFocusContract.setTransientOperation("drag"));
    await page.keyboard.press("Escape");
    const afterEscape = await page.evaluate(() => window.__a11yFocusContract.commands());
    if (afterEscape.at(-1)?.command !== "cancel_transient_operation") {
      throw new Error("scene Escape did not cancel transient operation");
    }

    // Modal context fences underlying document shortcuts.
    await page.locator("#open-dialog").click();
    const dialogFocus = await page.evaluate(() => window.__a11yFocusContract.activeElement());
    if (dialogFocus.id !== "dialog-input") throw new Error("dialog did not own focus");
    const beforeDialog = afterEscape.length;
    await page.keyboard.press("Delete");
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press("Control+z");
    const afterDialog = await page.evaluate(() => window.__a11yFocusContract.commands());
    if (afterDialog.length !== beforeDialog) {
      throw new Error("document command leaked from modal");
    }
    await page.locator("#dialog-close").click();

    // Product-level Tab order does not depend on visual renderer DOM.
    await page.locator("#page-nav").focus();
    const tabOrder = [];
    for (let i = 0; i < 5; i += 1) {
      tabOrder.push(await page.evaluate(() => window.__a11yFocusContract.activeElement()));
      await page.keyboard.press("Tab");
    }
    const tabRoles = tabOrder.map((x) => x.node_id ? "scene" : x.id);
    const expected = ["page-nav", "scene", "inspector", "story-editor", "open-dialog"];
    if (JSON.stringify(tabRoles) !== JSON.stringify(expected)) {
      throw new Error("product tab order mismatch: " + JSON.stringify(tabRoles));
    }

    const rendererAvailability = Object.fromEntries(
      rendererTransitions
        .filter((x, index) => index < 3)
        .map((x) => [x.renderer_kind, x.renderer_stats.available]),
    );
    const receipt = {
      receipt_kind: "chaptera.web-accessibility-focus-v1.browser-contract",
      browser_engine: BROWSER_NAME,
      real_pub: false,
      representative_scene: false,
      native_os_ime_measured: false,
      screen_reader_measured: false,
      wcag_acceptance: false,
      scene_summary: summary,
      renderer_availability: rendererAvailability,
      semantic_aria_snapshot: ariaSnapshot,
      invariants: {
        semantic_projection_exposes_all_fixture_node_ids:
          JSON.stringify([...baseline.logical_order].sort()) ===
          JSON.stringify([...summary.node_ids].sort()),
        exactly_one_roving_scene_tab_stop:
          baseline.tab_stop_node_ids.length === 1,
        renderer_switch_preserves_semantic_identity_order_selection_and_focus:
          rendererTransitions.every(
            (x) => JSON.stringify(stableSemantic(x.semantic_state)) === JSON.stringify(baselineStable),
          ),
        renderer_rebuild_preserves_dom_semantic_focus:
          rendererTransitions.every((x) => x.active_element.node_id === focusTarget),
        scene_focus_routes_document_commands:
          JSON.stringify(firstThree) === JSON.stringify([
            "delete_selection",
            "nudge_left",
            "document_undo",
          ]),
        arrow_navigation_moves_semantic_focus_with_one_roving_tab_stop:
          afterArrow.semantic.tab_stop_node_ids.length === 1 &&
          afterArrow.active.node_id === afterArrow.semantic.active_node_id,
        inspector_fences_document_global_commands:
          afterInspector.length === beforeInspector,
        story_editor_fences_document_global_commands:
          afterStory.length === beforeStory,
        explicit_composition_state_fences_scene_commands:
          afterComposition.length === beforeComposition,
        scene_escape_cancels_transient_operation:
          afterEscape.at(-1)?.command === "cancel_transient_operation",
        modal_fences_underlying_document_commands:
          afterDialog.length === beforeDialog,
        product_tab_order_is_renderer_independent:
          JSON.stringify(tabRoles) === JSON.stringify(expected),
      },
      guardrail:
        "Headless Chromium/Firefox product-contract acceptance on synthetic BrowserScene fixture. Does not claim native OS IME, real assistive-technology/screen-reader behavior, macOS menu conventions, high contrast/reduced motion, keyboard resize/multiselect or WCAG conformance.",
    };
    if (!Object.values(receipt.invariants).every(Boolean)) {
      throw new Error(JSON.stringify(receipt));
    }
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
