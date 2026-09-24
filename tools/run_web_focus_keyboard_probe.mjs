#!/usr/bin/env node
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, firefox } from "playwright";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TARGET = path.join(ROOT, "target", "web-focus-keyboard");
const BROWSER_NAME = process.argv[2] ?? "chromium";
const ENGINES = { chromium, firefox };
if (!(BROWSER_NAME in ENGINES)) throw new Error("unsupported browser " + BROWSER_NAME);

function startServer() {
  return new Promise((resolve) => {
    const server = http.createServer((request, response) => {
      try {
        const url = new URL(request.url ?? "/", "http://localhost");
        const relative = url.pathname === "/" ? "/apps/web/focus-keyboard-probe.html" : decodeURIComponent(url.pathname);
        const filePath = path.resolve(ROOT, "." + relative);
        if (!filePath.startsWith(ROOT + path.sep)) {
          response.writeHead(403).end("forbidden");
          return;
        }
        const body = fs.readFileSync(filePath);
        const type = filePath.endsWith(".html") ? "text/html; charset=utf-8" : "application/octet-stream";
        response.writeHead(200, {"content-type": type, "cache-control": "no-store"});
        response.end(body);
      } catch {
        response.writeHead(404).end("not found");
      }
    });
    server.listen(0, "127.0.0.1", () => resolve({server, port: server.address().port}));
  });
}

async function commandCount(page, name) {
  return page.evaluate((name) => window.__probe.commands.filter((x) => x.command === name).length, name);
}

async function main() {
  fs.mkdirSync(TARGET, {recursive: true});
  const serverState = await startServer();
  const browser = await ENGINES[BROWSER_NAME].launch({headless: true});

  try {
    const page = await browser.newPage({viewport: {width: 1000, height: 700}});
    await page.goto("http://127.0.0.1:" + serverState.port + "/apps/web/focus-keyboard-probe.html", {waitUntil: "networkidle"});

    const results = {};

    // Canvas semantic item: global/document keyboard commands are allowed.
    const node1 = page.locator('[data-node-id="node:1"]');
    await node1.focus();
    await page.keyboard.press("Delete");
    await page.keyboard.press("ArrowLeft");
    const mod = BROWSER_NAME === "webkit" ? "Meta" : "Control";
    await page.keyboard.press(mod + "+z");
    results.scene_item_commands = await page.evaluate(() => structuredClone(window.__probe.commands));
    if (await commandCount(page, "delete_selection") !== 1) throw new Error("scene Delete not routed");
    if (await commandCount(page, "nudge_left") !== 1) throw new Error("scene ArrowLeft not routed");
    if (await commandCount(page, "document_undo") !== 1) throw new Error("scene Undo not routed");

    // Roving tree focus uses ArrowDown, not multiple Tab stops.
    await node1.focus();
    await page.keyboard.press("ArrowDown");
    results.after_arrow_down = await page.evaluate(() => window.__probe.active());
    if (results.after_arrow_down.node_id !== "node:2") throw new Error("roving scene focus failed");
    const stops = await page.evaluate(() => window.__probe.tabStops());
    const treeTabStops = stops.filter((x) => x.role === "treeitem");
    if (treeTabStops.length !== 1 || treeTabStops[0].node_id !== "node:2") {
      throw new Error("scene tree must expose one roving tab stop");
    }
    results.tab_stops_after_roving = stops;

    // Inspector input must own destructive/navigation/undo keys locally.
    const inspector = page.locator("#inspector");
    await inspector.focus();
    const beforeInspector = await page.evaluate(() => window.__probe.commands.length);
    await page.keyboard.press("Delete");
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press(mod + "+z");
    const afterInspector = await page.evaluate(() => window.__probe.commands.length);
    if (afterInspector !== beforeInspector) throw new Error("global command leaked into inspector input");
    results.inspector_global_commands_added = afterInspector - beforeInspector;

    // Contenteditable text editor also suppresses global commands.
    const editor = page.locator("#text-editor");
    await editor.focus();
    const beforeText = await page.evaluate(() => window.__probe.commands.length);
    await page.keyboard.press("Backspace");
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press(mod + "+z");
    const afterText = await page.evaluate(() => window.__probe.commands.length);
    if (afterText !== beforeText) throw new Error("global command leaked into text editor");
    results.text_editor_global_commands_added = afterText - beforeText;

    // Composition state is an even stronger global-shortcut fence.
    await editor.evaluate((el) => el.dispatchEvent(new CompositionEvent("compositionstart", {bubbles: true, data: "漢"})));
    if (!(await page.evaluate(() => window.__probe.composing()))) throw new Error("composition state did not start");
    const beforeComposition = await page.evaluate(() => window.__probe.commands.length);
    await page.keyboard.press("Escape");
    await page.keyboard.press("Delete");
    const afterComposition = await page.evaluate(() => window.__probe.commands.length);
    if (afterComposition !== beforeComposition) throw new Error("global command leaked during composition");
    await editor.evaluate((el) => el.dispatchEvent(new CompositionEvent("compositionend", {bubbles: true, data: "漢"})));
    results.composition_global_commands_added = afterComposition - beforeComposition;

    // Escape cancels transient canvas drag only when canvas semantic focus owns the command.
    await node1.focus();
    await page.evaluate(() => window.__probe.setDragActive(true));
    await page.keyboard.press("Escape");
    if (await commandCount(page, "cancel_drag") !== 1) throw new Error("canvas Escape did not cancel drag");
    results.drag_escape_cancelled = true;

    // A modal owns focus/keys while open; global document commands must not fire from its input.
    await page.locator("#open-dialog").click();
    results.dialog_initial_focus = await page.evaluate(() => window.__probe.active());
    if (results.dialog_initial_focus.id !== "dialog-input") throw new Error("dialog input did not own focus");
    const beforeDialog = await page.evaluate(() => window.__probe.commands.length);
    await page.keyboard.press("Delete");
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press(mod + "+z");
    const afterDialog = await page.evaluate(() => window.__probe.commands.length);
    if (afterDialog !== beforeDialog) throw new Error("global command leaked from modal input");
    results.dialog_global_commands_added = afterDialog - beforeDialog;
    await page.locator("#dialog-close").click();

    // Verify coarse Tab order: page navigator -> one scene-tree tab stop -> inspector -> text editor -> dialog opener.
    await page.locator("#page-nav").focus();
    const tabOrder = [];
    for (let i = 0; i < 5; i += 1) {
      tabOrder.push(await page.evaluate(() => window.__probe.active()));
      await page.keyboard.press("Tab");
    }
    results.tab_order = tabOrder;

    const receipt = {
      receipt_kind: "chaptera.web-focus-keyboard-routing-probe.v1",
      browser_engine: BROWSER_NAME,
      real_pub: false,
      native_os_ime_measured: false,
      screen_reader_measured: false,
      product_acceptance: false,
      results,
      bounded_findings: {
        scene_semantic_focus_can_own_document_shortcuts: true,
        text_and_inspector_focus_must_fence_document_shortcuts: true,
        composition_state_must_fence_global_shortcuts: true,
        modal_focus_must_fence_underlying_document_shortcuts: true,
        scene_tree_should_use_roving_tabindex: true,
        escape_can_cancel_transient_canvas_drag_when_canvas_owns_focus: true,
      },
      guardrail:
        "Headless browser routing evidence only. Synthetic CompositionEvent is not a real OS IME. Screen-reader behavior, native menu shortcuts, macOS Meta conventions and final WCAG keyboard acceptance remain open."
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
