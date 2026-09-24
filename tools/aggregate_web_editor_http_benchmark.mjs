#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";

const target = process.argv[2];
const browser = process.argv[3];
const output = process.argv[4];

if (!target || !browser || !output) {
  throw new Error("usage: aggregate_web_editor_http_benchmark.mjs <dir> <browser> <output>");
}

const files = fs.readdirSync(target)
  .filter((name) => name.startsWith(browser + "-http-receipt-") && name.endsWith(".json"))
  .sort();

if (files.length === 0) throw new Error("no receipts for " + browser);

const receipts = files.map((name) =>
  JSON.parse(fs.readFileSync(path.join(target, name), "utf8"))
);

const timingKeys = [
  "browser_scene_current_http",
  "gateway_scene_current",
  "browser_commit_http",
  "gateway_commit",
  "browser_scene_revision_http",
  "gateway_scene_revision",
];

function percentile(values, p) {
  const sorted = [...values].sort((a, b) => a - b);
  const rank = Math.max(0, Math.ceil((p / 100) * sorted.length) - 1);
  return sorted[rank];
}

function stats(values) {
  return {
    count: values.length,
    min: Math.min(...values),
    p50: percentile(values, 50),
    p95: percentile(values, 95),
    max: Math.max(...values),
    mean: values.reduce((a, b) => a + b, 0) / values.length,
  };
}

for (const receipt of receipts) {
  if (receipt.browser_engine !== browser) throw new Error("browser mismatch");
  if (receipt.real_pub !== false || receipt.product_acceptance !== false) {
    throw new Error("benchmark receipt tried to widen acceptance claim");
  }
  if (!receipt.observability.same_trace_browser_and_server) {
    throw new Error("cross-boundary trace correlation failed");
  }
  if (!receipt.observability.semantic_operation_correlated) {
    throw new Error("semantic operation correlation failed");
  }
  if (!receipt.observability.metrics_high_cardinality_labels_absent) {
    throw new Error("metric cardinality guard failed");
  }
  if (!receipt.stale_base_rejected || !receipt.exact_retry_same_revision) {
    throw new Error("revision semantics regression");
  }
}

const timings = {};
for (const key of timingKeys) {
  timings[key] = stats(
    receipts.map((r) => Number(r.observability.timings_ms[key]))
  );
}

const summary = {
  receipt_kind: "chaptera.synthetic-http-observability-benchmark.v1",
  browser_engine: browser,
  browser_versions: [...new Set(receipts.map((r) => r.browser_version))].sort(),
  sample_count: receipts.length,
  real_pub: false,
  product_acceptance: false,
  same_trace_all: receipts.every((r) => r.observability.same_trace_browser_and_server),
  semantic_operation_correlation_all: receipts.every((r) => r.observability.semantic_operation_correlated),
  metrics_high_cardinality_labels_absent_all: receipts.every(
    (r) => r.observability.metrics_high_cardinality_labels_absent
  ),
  revision_semantics_all: receipts.every(
    (r) => r.stale_base_rejected && r.exact_retry_same_revision && r.semantic_executor_calls === 1
  ),
  timings_ms: timings,
  interpretation:
    "Synthetic CI transport baseline only. Values measure local GitHub runner browser-to-localhost HTTP plumbing over the public RevisionKernel harness; they are not real-PUB product SLOs."
};

fs.writeFileSync(output, JSON.stringify(summary, null, 2) + "\n");
process.stdout.write(JSON.stringify(summary, null, 2) + "\n");
