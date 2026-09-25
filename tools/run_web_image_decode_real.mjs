#!/usr/bin/env node
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, firefox } from "playwright";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TARGET = path.join(ROOT, "target", "web-image-decode");
const BROWSER_NAME = process.argv[2] ?? "chromium";
const ENGINES = { chromium, firefox };
if (!(BROWSER_NAME in ENGINES)) throw new Error("unsupported browser " + BROWSER_NAME);

function mimeFor(filePath) {
  if (filePath.endsWith(".html")) return "text/html; charset=utf-8";
  if (filePath.endsWith(".mjs") || filePath.endsWith(".js")) return "text/javascript; charset=utf-8";
  if (filePath.endsWith(".json")) return "application/json; charset=utf-8";
  if (filePath.endsWith(".png")) return "image/png";
  if (filePath.endsWith(".jpg") || filePath.endsWith(".jpeg")) return "image/jpeg";
  return "application/octet-stream";
}

function startServer() {
  return new Promise((resolve) => {
    const server = http.createServer((request, response) => {
      try {
        const url = new URL(request.url ?? "/", "http://localhost");
        const pathname = decodeURIComponent(url.pathname);
        const relative = pathname === "/" ? "/apps/web/image-decode-real.html" : pathname;
        const filePath = path.resolve(ROOT, "." + relative);
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

async function main() {
  fs.mkdirSync(TARGET, { recursive: true });
  const state = await startServer();
  const browser = await ENGINES[BROWSER_NAME].launch({ headless: true });
  try {
    const page = await browser.newPage();
    await page.goto("http://127.0.0.1:" + state.port + "/apps/web/image-decode-real.html", { waitUntil: "networkidle" });
    const receipt = await page.evaluate(() => window.runRealImageDecodeProbe());
    receipt.browser = { engine: BROWSER_NAME, version: browser.version(), headless: true };
    const output = path.join(TARGET, BROWSER_NAME + "-receipt.json");
    fs.writeFileSync(output, JSON.stringify(receipt, null, 2) + "\n");
    process.stdout.write(JSON.stringify(receipt, null, 2) + "\n");
  } finally {
    await browser.close();
    await new Promise((resolve) => state.server.close(resolve));
  }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
