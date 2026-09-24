import { assertSafeEmu } from "./interaction-v1.mjs";

export const SPATIAL_SCHEMA = "chaptera.interaction-spatial-lifecycle.v1";

function checkedPositive(value, label) {
  assertSafeEmu(value, label);
  if (value <= 0) throw new RangeError(`${label} must be > 0`);
  return value;
}

function checkedBounds(bounds, label = "bounds") {
  if (!bounds || typeof bounds !== "object") throw new TypeError(`${label} required`);
  const x = assertSafeEmu(bounds.x, `${label}.x`);
  const y = assertSafeEmu(bounds.y, `${label}.y`);
  const width = checkedPositive(bounds.width, `${label}.width`);
  const height = checkedPositive(bounds.height, `${label}.height`);
  assertSafeEmu(x + width, `${label}.right`);
  assertSafeEmu(y + height, `${label}.bottom`);
  return Object.freeze({ x, y, width, height });
}

function identityKey(identity) {
  if (!identity || typeof identity !== "object") throw new TypeError("generation identity required");
  const required = ["document_id", "revision_id", "layout_environment_id", "window_id", "window_generation"];
  for (const key of required) {
    if (identity[key] === undefined || identity[key] === null || identity[key] === "") {
      throw new TypeError(`generation identity missing ${key}`);
    }
  }
  if (!Number.isSafeInteger(identity.window_generation) || identity.window_generation < 0) {
    throw new TypeError("window_generation must be a non-negative safe integer");
  }
  const pages = [...(identity.page_ids ?? [])].sort();
  const shards = [...(identity.shard_fingerprints ?? [])].sort();
  return JSON.stringify([
    SPATIAL_SCHEMA,
    identity.document_id,
    identity.revision_id,
    identity.layout_environment_id,
    identity.window_id,
    identity.window_generation,
    pages,
    shards,
  ]);
}

function contains(bounds, point) {
  return point.x_emu >= bounds.x && point.x_emu < bounds.x + bounds.width &&
    point.y_emu >= bounds.y && point.y_emu < bounds.y + bounds.height;
}

function intersects(a, b) {
  return !(a.x + a.width <= b.x || b.x + b.width <= a.x || a.y + a.height <= b.y || b.y + b.height <= a.y);
}

function cellKey(x, y) { return `${x}:${y}`; }

function cellRange(bounds, cellSize, maxCells) {
  const minX = Math.floor(bounds.x / cellSize);
  const minY = Math.floor(bounds.y / cellSize);
  const maxX = Math.floor((bounds.x + bounds.width - 1) / cellSize);
  const maxY = Math.floor((bounds.y + bounds.height - 1) / cellSize);
  const count = (maxX - minX + 1) * (maxY - minY + 1);
  if (!Number.isSafeInteger(count) || count > maxCells) throw new RangeError("spatial query exceeds max cell budget");
  const out = [];
  for (let y = minY; y <= maxY; y++) for (let x = minX; x <= maxX; x++) out.push(cellKey(x, y));
  return out;
}

function normalizeNode(node) {
  if (!node || typeof node !== "object") return { row: null, reason: "invalid_node" };
  if (node.bounds_fidelity === "unknown" || node.bounds_fidelity === "unsupported") {
    return { row: null, reason: "unsupported_bounds" };
  }
  if (node.transformed_bounds_fidelity === "unknown" || node.transformed_bounds_fidelity === "unsupported") {
    return { row: null, reason: "unsupported_transformed_bounds" };
  }
  try {
    const bounds = checkedBounds(node.bounds, "node.bounds");
    if (typeof node.node_id !== "string" || !node.node_id || typeof node.page_id !== "string" || !node.page_id) {
      return { row: null, reason: "invalid_identity" };
    }
    return {
      row: Object.freeze({
        node_id: node.node_id,
        page_id: node.page_id,
        bounds,
        paint_order: Number.isSafeInteger(node.paint_order) ? node.paint_order : null,
        z_order: Number.isSafeInteger(node.z_order) ? node.z_order : null,
      }),
      reason: null,
    };
  } catch {
    return { row: null, reason: "invalid_bounds" };
  }
}

export class SpatialWindowV1 {
  constructor({ cellSizeEmu = 250000, maxQueryCells = 4096 } = {}) {
    this.cellSizeEmu = checkedPositive(cellSizeEmu, "cellSizeEmu");
    if (!Number.isSafeInteger(maxQueryCells) || maxQueryCells <= 0) throw new RangeError("maxQueryCells must be positive");
    this.maxQueryCells = maxQueryCells;
    this.generationKey = null;
    this.identity = null;
    this.rows = new Map();
    this.pages = new Map();
    this.grids = new Map();
    this.metrics = { rebuilds: 0, patches: 0, evictions: 0, revisits: 0, stale_rejections: 0, skipped_invalid: 0, authoring_mutations: 0, revisions_emitted: 0 };
  }

  replaceWindow(identity, nodes) {
    if (!Array.isArray(nodes)) throw new TypeError("nodes must be an array");
    const nextKey = identityKey(identity);
    const rows = new Map();
    const pages = new Map();
    const grids = new Map();
    let skipped = 0;
    for (const node of nodes) {
      const { row } = normalizeNode(node);
      if (!row) { skipped += 1; continue; }
      rows.set(row.node_id, row);
      if (!pages.has(row.page_id)) pages.set(row.page_id, []);
      pages.get(row.page_id).push(row.node_id);
      if (!grids.has(row.page_id)) grids.set(row.page_id, new Map());
      const grid = grids.get(row.page_id);
      for (const key of cellRange(row.bounds, this.cellSizeEmu, this.maxQueryCells)) {
        if (!grid.has(key)) grid.set(key, new Set());
        grid.get(key).add(row.node_id);
      }
    }
    for (const ids of pages.values()) ids.sort();
    this.identity = structuredClone(identity);
    this.generationKey = nextKey;
    this.rows = rows; this.pages = pages; this.grids = grids;
    this.metrics.rebuilds += 1; this.metrics.skipped_invalid += skipped;
    return this.receipt();
  }

  #assertGeneration(generationKey) {
    if (!this.generationKey || generationKey !== this.generationKey) {
      this.metrics.stale_rejections += 1;
      throw new Error("stale spatial generation");
    }
  }

  #candidateIds(pageId, queryBounds) {
    const grid = this.grids.get(pageId);
    const cells = cellRange(queryBounds, this.cellSizeEmu, this.maxQueryCells);
    if (!grid) return { ids: [], cellsVisited: cells.length, candidateChecks: 0 };
    const ids = new Set();
    for (const cell of cells) for (const id of grid.get(cell) ?? []) ids.add(id);
    return { ids: [...ids].sort(), cellsVisited: cells.length, candidateChecks: ids.size };
  }

  queryPoint(pageId, point, generationKey) {
    this.#assertGeneration(generationKey);
    assertSafeEmu(point.x_emu, "point.x_emu"); assertSafeEmu(point.y_emu, "point.y_emu");
    const q = { x: point.x_emu, y: point.y_emu, width: 1, height: 1 };
    const stats = this.#candidateIds(pageId, q);
    const candidates = stats.ids.map(id => this.rows.get(id)).filter(row => contains(row.bounds, point));
    return { generation_key: this.generationKey, candidates, stats: { cells_visited: stats.cellsVisited, candidate_checks: stats.candidateChecks } };
  }

  queryBox(pageId, bounds, generationKey) {
    this.#assertGeneration(generationKey);
    const q = checkedBounds(bounds, "query.bounds");
    const stats = this.#candidateIds(pageId, q);
    const candidates = stats.ids.map(id => this.rows.get(id)).filter(row => intersects(row.bounds, q));
    return { generation_key: this.generationKey, candidates, stats: { cells_visited: stats.cellsVisited, candidate_checks: stats.candidateChecks } };
  }

  snapAnchors(pageId, bounds, generationKey) {
    const result = this.queryBox(pageId, bounds, generationKey);
    const anchors = [];
    for (const row of result.candidates) {
      const b = row.bounds;
      anchors.push(
        { node_id: row.node_id, axis: "x", anchor: "min", position_emu: b.x },
        { node_id: row.node_id, axis: "x", anchor: "center", position_emu: b.x + Math.trunc(b.width / 2) },
        { node_id: row.node_id, axis: "x", anchor: "max", position_emu: b.x + b.width },
        { node_id: row.node_id, axis: "y", anchor: "min", position_emu: b.y },
        { node_id: row.node_id, axis: "y", anchor: "center", position_emu: b.y + Math.trunc(b.height / 2) },
        { node_id: row.node_id, axis: "y", anchor: "max", position_emu: b.y + b.height },
      );
    }
    return { generation_key: result.generation_key, anchors, stats: result.stats };
  }

  cullCandidates(pageId, bounds, generationKey) {
    return this.queryBox(pageId, bounds, generationKey);
  }

  applyNodePatch({ base_generation_key, next_identity, remove_node_ids = [], upsert_nodes = [] }) {
    this.#assertGeneration(base_generation_key);
    const next = [...this.rows.values()].map(row => ({
      node_id: row.node_id, page_id: row.page_id, bounds: { ...row.bounds }, paint_order: row.paint_order, z_order: row.z_order,
    }));
    const remove = new Set(remove_node_ids);
    const kept = next.filter(row => !remove.has(row.node_id) && !upsert_nodes.some(u => u.node_id === row.node_id));
    this.metrics.patches += 1;
    return this.replaceWindow(next_identity, [...kept, ...upsert_nodes]);
  }

  evictPage({ base_generation_key, next_identity, page_id }) {
    this.#assertGeneration(base_generation_key);
    const kept = [...this.rows.values()].filter(row => row.page_id !== page_id).map(row => ({ ...row, bounds: { ...row.bounds } }));
    this.metrics.evictions += 1;
    return this.replaceWindow(next_identity, kept);
  }

  revisitPage({ base_generation_key, next_identity, nodes }) {
    this.#assertGeneration(base_generation_key);
    this.metrics.revisits += 1;
    return this.replaceWindow(next_identity, [...this.rows.values()].map(row => ({ ...row, bounds: { ...row.bounds } })), ...[]);
  }

  reconcileSelection(nodeId, generationKey) {
    this.#assertGeneration(generationKey);
    return nodeId && this.rows.has(nodeId) ? { node_id: nodeId, state: "present" } : { node_id: null, state: "cleared_missing" };
  }

  receipt() {
    return {
      schema: SPATIAL_SCHEMA,
      generation_key: this.generationKey,
      page_count: this.pages.size,
      indexed_nodes: this.rows.size,
      metrics: { ...this.metrics },
      authority: { canonical_document_state: false, selection_authority: false, snap_policy_authority: false, authoring_mutations: 0, revisions_emitted: 0 },
    };
  }
}

export { identityKey, normalizeNode };
