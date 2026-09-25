import fs from "node:fs";
import { performance } from "node:perf_hooks";
import {
  SpatialWindowLifecycleV2,
  spatialGenerationKey,
} from "../apps/web/spatial-lifecycle-v1.mjs";

function identity(totalPages, generation = 1) {
  return {
    document_id: "doc:bench",
    revision_id: "rev:" + generation,
    layout_environment_id: "env:1",
    window_id: "window:49-51",
    window_generation: generation,
    page_ids: ["p49", "p50", "p51"],
    shard_fingerprints: ["s49:" + generation, "s50:" + generation, "s51:" + generation],
    total_document_pages: totalPages,
  };
}

function nodes() {
  const out = [];
  for (let page = 49; page <= 51; page += 1) {
    for (let index = 0; index < 2000; index += 1) {
      out.push({
        node_id: "p" + page + ":n" + index,
        page_id: "p" + page,
        bounds: {
          x: (index % 50) * 10000 - 20000,
          y: Math.floor(index / 50) * 10000,
          width: 8000,
          height: 8000,
        },
        paint_order: index,
        z_order: index,
      });
    }
  }
  return out;
}

function percentile(values, p) {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * p) - 1))];
}

function run(totalPages) {
  const index = new SpatialWindowLifecycleV2({ cellSizeEmu: 50000, maxQueryCells: 4096 });
  const data = nodes();
  const buildStarted = performance.now();
  index.replaceWindow(identity(totalPages), data);
  const buildMs = performance.now() - buildStarted;
  const generation = spatialGenerationKey(identity(totalPages));
  const query = { x: 100000, y: 100000, width: 200000, height: 200000 };
  const querySamples = [];
  let last = null;
  for (let iteration = 0; iteration < 1000; iteration += 1) {
    const started = performance.now();
    last = index.queryBox("p50", query, generation);
    querySamples.push(performance.now() - started);
  }

  const patchNode = {
    node_id: "p50:n10",
    page_id: "p50",
    bounds: { x: 900000, y: 900000, width: 8000, height: 8000 },
    paint_order: 10,
    z_order: 10,
  };
  const patchStarted = performance.now();
  index.applyNodePatch({
    base_generation_key: generation,
    next_identity: identity(totalPages, 2),
    upsert_nodes: [patchNode],
  });
  const patchMs = performance.now() - patchStarted;

  return {
    total_document_pages: totalPages,
    indexed_window_nodes: data.length,
    build_ms: buildMs,
    query_p50_ms: percentile(querySamples, 0.50),
    query_p95_ms: percentile(querySamples, 0.95),
    query_stats: last.stats,
    patch_rebuild_ms: patchMs,
    receipt: index.receipt(),
  };
}

const pages100 = run(100);
const pages500 = run(500);
const receipt = {
  schema: "chaptera.interaction-spatial-lifecycle-benchmark.v2",
  measurement_class: "synthetic_equal_three_page_window",
  real_pub: false,
  representative_corpus: false,
  technology_decision_allowed: false,
  cases: [pages100, pages500],
  deterministic_operation_cost_equal:
    pages100.query_stats.cells_visited === pages500.query_stats.cells_visited
    && pages100.query_stats.candidate_checks === pages500.query_stats.candidate_checks,
  limitations: [
    "Wall-clock timing is Node/CI host behavior only.",
    "Uniform grid is one disposable mechanism, not a semantic architecture mandate.",
    "Current benchmark uses source-neutral window geometry, not a representative real-PUB corpus.",
  ],
};
fs.mkdirSync("target/spatial-lifecycle", { recursive: true });
fs.writeFileSync("target/spatial-lifecycle/receipt.json", JSON.stringify(receipt, null, 2) + "\n");
console.log(JSON.stringify(receipt));
