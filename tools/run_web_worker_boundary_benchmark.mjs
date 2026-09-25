#!/usr/bin/env node
import fs from "node:fs";
import { chromium, firefox } from "playwright";

const name = process.argv[2] || "chromium";
const browserType = ({ chromium, firefox })[name];
if (!browserType) throw new Error("browser must be chromium or firefox");
const browser = await browserType.launch({ headless: true });

function percentile(values, p) {
  const sorted = [...values].sort((a, b) => a - b);
  if (!sorted.length) return null;
  return sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * p) - 1))];
}

try {
  const page = await browser.newPage();
  const result = await page.evaluate(async () => {
    const source = `
      onmessage = (event) => {
        const message = event.data;
        if (message.kind === "work") {
          const started = performance.now();
          let checksum = 0;
          const view = new Uint8Array(message.buffer);
          for (let i = 0; i < view.length; i += 4096) checksum = (checksum + view[i]) & 0xffff;
          postMessage({ id: message.id, bytes: view.byteLength, checksum, worker_ms: performance.now() - started });
        }
      };
    `;
    const url = URL.createObjectURL(new Blob([source], { type: "text/javascript" }));
    const worker = new Worker(url);
    let seq = 0;

    const one = (size, transfer) => new Promise((resolve, reject) => {
      const buffer = new ArrayBuffer(size);
      new Uint8Array(buffer)[0] = 7;
      const id = ++seq;
      const before = buffer.byteLength;
      const started = performance.now();
      const handler = (event) => {
        if (event.data.id !== id) return;
        worker.removeEventListener("message", handler);
        requestAnimationFrame(() => {
          resolve({
            roundtrip_to_visible_ms: performance.now() - started,
            worker_ms: event.data.worker_ms,
            producer_bytes_before: before,
            producer_bytes_after_post: buffer.byteLength,
            reported_bytes: event.data.bytes,
          });
        });
      };
      worker.addEventListener("message", handler);
      worker.addEventListener("error", reject, { once: true });
      worker.postMessage({ kind: "work", id, buffer }, transfer ? [buffer] : []);
    });

    const runs = [];
    for (const size of [64 * 1024, 1024 * 1024, 8 * 1024 * 1024]) {
      for (const transport of ["clone", "transfer"]) {
        const samples = [];
        for (let i = 0; i < 8; i += 1) samples.push(await one(size, transport === "transfer"));
        runs.push({ size_bytes: size, transport, samples });
      }
    }
    worker.terminate();
    URL.revokeObjectURL(url);
    return {
      runs,
      shared_array_buffer_available: typeof SharedArrayBuffer !== "undefined",
      cross_origin_isolated: self.crossOriginIsolated === true,
    };
  });

  const rows = result.runs.map((run) => {
    const visible = run.samples.map((sample) => sample.roundtrip_to_visible_ms);
    const worker = run.samples.map((sample) => sample.worker_ms);
    const last = run.samples.at(-1);
    const transferDetached = run.transport === "transfer"
      ? run.samples.every((sample) => sample.producer_bytes_after_post === 0)
      : false;
    const simultaneousPayloadCopiesProxy = run.transport === "transfer" && transferDetached ? 1 : 2;
    return {
      size_bytes: run.size_bytes,
      transport: run.transport,
      sample_count: run.samples.length,
      input_to_visible_p50_ms: percentile(visible, 0.50),
      input_to_visible_p95_ms: percentile(visible, 0.95),
      worker_compute_p50_ms: percentile(worker, 0.50),
      producer_bytes_after_post: last.producer_bytes_after_post,
      transfer_detached: transferDetached,
      clone_bytes: run.transport === "clone" ? run.size_bytes * run.samples.length : 0,
      transferred_bytes: run.transport === "transfer" ? run.size_bytes * run.samples.length : 0,
      peak_queue_depth: 1,
      simultaneous_payload_copies_proxy: simultaneousPayloadCopiesProxy,
      memory_amplification_proxy: simultaneousPayloadCopiesProxy,
    };
  });

  const receipt = {
    schema: "chaptera.web-worker-boundary-benchmark.v2",
    browser: name,
    real_pub: false,
    representative_corpus: false,
    technology_decision_allowed: false,
    canonical_authority: "server_editor_session",
    worker_authority: "browser_derived_disposable_only",
    raw_pub_crosses_worker_boundary: false,
    shared_array_buffer_baseline: false,
    shared_array_buffer_available: result.shared_array_buffer_available,
    cross_origin_isolated: result.cross_origin_isolated,
    rows,
  };
  fs.mkdirSync("target/web-worker-boundary", { recursive: true });
  fs.writeFileSync(
    `target/web-worker-boundary/${name}.json`,
    JSON.stringify(receipt, null, 2) + "\n",
  );
  console.log(JSON.stringify(receipt));
} finally {
  await browser.close();
}
