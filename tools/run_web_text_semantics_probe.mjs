#!/usr/bin/env node
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, firefox } from "playwright";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TARGET = path.join(ROOT, "target", "web-text-semantics");
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
        const rel = url.pathname === "/" ? "/apps/web/text-semantics-probe.html" : decodeURIComponent(url.pathname);
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

function diffStrings(before, after) {
  let prefix = 0;
  while (prefix < before.length && prefix < after.length && before[prefix] === after[prefix]) prefix += 1;
  let suffix = 0;
  while (
    suffix < before.length - prefix &&
    suffix < after.length - prefix &&
    before[before.length - 1 - suffix] === after[after.length - 1 - suffix]
  ) suffix += 1;
  return {
    prefix_utf16: prefix,
    removed: before.slice(prefix, before.length - suffix),
    inserted: after.slice(prefix, after.length - suffix),
    suffix_utf16: suffix,
  };
}

function graphemeCount(value) {
  if (!value) return 0;
  const segmenter = new Intl.Segmenter("en", {granularity: "grapheme"});
  return Array.from(segmenter.segment(value)).length;
}

function characterize(before, after) {
  const d = diffStrings(before, after);
  return {
    ...d,
    removed_utf16_units: d.removed.length,
    removed_code_points: Array.from(d.removed).length,
    removed_graphemes: graphemeCount(d.removed),
    inserted_utf16_units: d.inserted.length,
    inserted_code_points: Array.from(d.inserted).length,
    inserted_graphemes: graphemeCount(d.inserted),
  };
}

function eventDigest(events) {
  return events.map((e) => ({
    type: e.type,
    key: e.key,
    input_type: e.input_type,
    data: e.data,
    is_composing: e.is_composing,
    target_range_count: Array.isArray(e.target_ranges) ? e.target_ranges.length : null,
    target_ranges: e.target_ranges,
    selection: e.selection,
  }));
}

async function backspaceCase(page, surface, name, cluster) {
  const before = "A" + cluster + "B";
  const caret = before.length - 1;
  if (surface === "textarea") {
    await page.evaluate(({before, caret}) => window.setTextarea(before, caret), {before, caret});
  } else {
    await page.evaluate(({before, caret}) => window.setContentEditable(before, caret), {before, caret});
  }
  await page.keyboard.press("Backspace");
  await page.waitForTimeout(10);
  const snap = await page.evaluate(() => window.probeSnapshot());
  const after = surface === "textarea" ? snap.textarea_value : snap.contenteditable_text;
  const change = characterize(before, after);
  const beforeinput = snap.events.find((e) => e.type === "beforeinput");
  const input = snap.events.find((e) => e.type === "input");
  if (!beforeinput || !input) throw new Error(surface + " " + name + " did not emit beforeinput+input");
  if (beforeinput.input_type !== "deleteContentBackward") {
    throw new Error(surface + " " + name + " unexpected beforeinput type " + beforeinput.input_type);
  }
  if (after === before) throw new Error(surface + " " + name + " backspace made no edit");
  return {
    surface,
    case: name,
    before,
    after,
    caret_before_utf16: caret,
    change,
    events: eventDigest(snap.events),
  };
}

async function main() {
  fs.mkdirSync(TARGET, {recursive: true});
  const serverState = await startServer();
  const browser = await ENGINES[BROWSER_NAME].launch({headless: true});
  try {
    const page = await browser.newPage({viewport: {width: 1000, height: 700}});
    await page.goto("http://127.0.0.1:" + serverState.port + "/", {waitUntil: "networkidle"});

    const capabilities = await page.evaluate(() => window.probeSnapshot().capabilities);

    const utf16Textarea = await page.evaluate(() => window.setTextarea("A😀B", 3));
    const utf16ContentEditable = await page.evaluate(() => window.setContentEditable("A😀B", 3));
    if (utf16Textarea.textarea.start !== 3) throw new Error("textarea caret did not preserve UTF-16 offset 3");
    if (utf16ContentEditable.dom?.anchor_offset !== 3) throw new Error("contenteditable caret did not preserve DOM offset 3");

    const clusters = [
      ["single_emoji", "😀"],
      ["zwj_family", "👨‍👩‍👧‍👦"],
      ["combining_acute", "a\u0301"],
      ["flag_us", "🇺🇸"],
    ];

    const deletionCases = [];
    for (const [name, cluster] of clusters) {
      deletionCases.push(await backspaceCase(page, "textarea", name, cluster));
    }
    for (const [name, cluster] of clusters) {
      deletionCases.push(await backspaceCase(page, "contenteditable", name, cluster));
    }

    await page.evaluate(() => window.setTextarea("AB", 1));
    await page.keyboard.insertText("漢");
    await page.waitForTimeout(10);
    const inserted = await page.evaluate(() => window.probeSnapshot());
    const insertBeforeinput = inserted.events.find((e) => e.type === "beforeinput");
    const insertInput = inserted.events.find((e) => e.type === "input");
    if (!insertBeforeinput || !insertInput) throw new Error("insertText did not emit beforeinput+input");

    const receipt = {
      receipt_kind: "chaptera.browser-text-input-semantics.v1",
      browser_engine: BROWSER_NAME,
      real_pub: false,
      canonical_story_integration: false,
      native_os_ime_measured: false,
      product_acceptance: false,
      capabilities,
      offset_probe: {
        text: "A😀B",
        js_utf16_length: "A😀B".length,
        unicode_code_points: Array.from("A😀B").length,
        textarea_caret_requested: 3,
        textarea_caret_observed: utf16Textarea.textarea.start,
        contenteditable_dom_offset_requested: 3,
        contenteditable_dom_offset_observed: utf16ContentEditable.dom?.anchor_offset ?? null,
        interpretation:
          "Browser textarea/DOM selection offsets are measured in UTF-16 code units here; canonical Story scalar/grapheme positions require explicit conversion."
      },
      deletion_cases: deletionCases,
      insert_text_case: {
        before: "AB",
        inserted_text: "漢",
        after: inserted.textarea_value,
        events: eventDigest(inserted.events),
      },
      note:
        "Real headless browser keyboard editing semantics only. This run does not drive an OS IME and therefore cannot close compositionstart/update/end behavior. It informs range normalization, grapheme UX, and DOM-offset conversion only."
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
