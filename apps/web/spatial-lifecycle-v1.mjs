import { assertSafeEmu } from "./interaction-v1.mjs";

export const SPATIAL_LIFECYCLE_SCHEMA = "chaptera.interaction-spatial-lifecycle.v2";

function checkedPositiveEmu(value, label) {
  assertSafeEmu(value, label);
  if (value <= 0) throw new RangeError(label + " must be > 0");
  return value;
}

function checkedBounds(bounds, label = "bounds") {
  if (!bounds || typeof bounds !== "object") throw new TypeError(label + " required");
  const x = assertSafeEmu(bounds.x, label + ".x");
  const y = assertSafeEmu(bounds.y, label + ".y");
  const width = checkedPositiveEmu(bounds.width, label + ".width");
  const height = checkedPositiveEmu(bounds.height, label + ".height");
  assertSafeEmu(x + width, label + ".right");
  assertSafeEmu(y + height, label + ".bottom");
  return Object.freeze({ x, y, width, height });
}

function checkedString(value, label) {
  if (typeof value !== "string" || value.length === 0) throw new TypeError(label + " required");
  return value;
}

function checkedStringList(values, label, { unique = false } = {}) {
  if (!Array.isArray(values)) throw new TypeError(label + " must be an array");
  const out = values.map((value, index) => checkedString(value, label + "[" + index + "]"));
  if (unique && new Set(out).size !== out.length) throw new TypeError(label + " must be unique");
  return Object.freeze([...out]);
}

export function normalizeSpatialIdentity(identity) {
  if (!identity || typeof identity !== "object") throw new TypeError("spatial identity required");
  const windowGeneration = identity.window_generation;
  if (!Number.isSafeInteger(windowGeneration) || windowGeneration < 0) {
    throw new TypeError("window_generation must be a non-negative safe integer");
  }
  const pageIds = checkedStringList(identity.page_ids ?? [], "page_ids", { unique: true });
  const shardFingerprints = checkedStringList(identity.shard_fingerprints ?? [], "shard_fingerprints");
  if (pageIds.length !== shardFingerprints.length) {
    throw new TypeError("page_ids and shard_fingerprints must have the same length");
  }
  return Object.freeze({
    document_id: checkedString(identity.document_id, "document_id"),
    revision_id: checkedString(identity.revision_id, "revision_id"),
    layout_environment_id: checkedString(identity.layout_environment_id, "layout_environment_id"),
    window_id: checkedString(identity.window_id, "window_id"),
    window_generation: windowGeneration,
    page_ids: pageIds,
    shard_fingerprints: shardFingerprints,
  });
}

export function spatialGenerationKey(identity) {
  const value = normalizeSpatialIdentity(identity);
  return JSON.stringify([
    SPATIAL_LIFECYCLE_SCHEMA,
    value.document_id,
    value.revision_id,
    value.layout_environment_id,
    value.window_id,
    value.window_generation,
    [...value.page_ids],
    [...value.shard_fingerprints],
  ]);
}

function containsPoint(bounds, point) {
  return point.x_emu >= bounds.x
    && point.x_emu < bounds.x + bounds.width
    && point.y_emu >= bounds.y
    && point.y_emu < bounds.y + bounds.height;
}

function intersects(a, b) {
  return !(
    a.x + a.width <= b.x
    || b.x + b.width <= a.x
    || a.y + a.height <= b.y
    || b.y + b.height <= a.y
  );
}

function cellKey(x, y) {
  return x + ":" + y;
}

function cellRange(bounds, cellSize, maxCells) {
  const minX = Math.floor(bounds.x / cellSize);
  const minY = Math.floor(bounds.y / cellSize);
  const maxX = Math.floor((bounds.x + bounds.width - 1) / cellSize);
  const maxY = Math.floor((bounds.y + bounds.height - 1) / cellSize);
  const width = maxX - minX + 1;
  const height = maxY - minY + 1;
  const count = width * height;
  if (!Number.isSafeInteger(count) || count > maxCells) {
    throw new RangeError("spatial query exceeds max cell budget");
  }
  const keys = [];
  for (let y = minY; y <= maxY; y += 1) {
    for (let x = minX; x <= maxX; x += 1) keys.push(cellKey(x, y));
  }
  return keys;
}

export function normalizeSpatialGeometry(node) {
  if (!node || typeof node !== "object") return { row: null, reason: "invalid_node" };
  if (node.bounds_fidelity === "unknown" || node.bounds_fidelity === "unsupported") {
    return { row: null, reason: "unsupported_bounds" };
  }
  if (
    node.transformed_bounds_fidelity === "unknown"
    || node.transformed_bounds_fidelity === "unsupported"
  ) {
    return { row: null, reason: "unsupported_transformed_bounds" };
  }
  try {
    const row = Object.freeze({
      node_id: checkedString(node.node_id, "node.node_id"),
      page_id: checkedString(node.page_id, "node.page_id"),
      bounds: checkedBounds(node.bounds, "node.bounds"),
      paint_order: Number.isSafeInteger(node.paint_order) ? node.paint_order : null,
      z_order: Number.isSafeInteger(node.z_order) ? node.z_order : null,
    });
    return { row, reason: null };
  } catch {
    return { row: null, reason: "invalid_bounds_or_identity" };
  }
}

export class SpatialWindowLifecycleV2 {
  constructor({ cellSizeEmu = 250000, maxQueryCells = 4096 } = {}) {
    this.cellSizeEmu = checkedPositiveEmu(cellSizeEmu, "cellSizeEmu");
    if (!Number.isSafeInteger(maxQueryCells) || maxQueryCells <= 0) {
      throw new RangeError("maxQueryCells must be a positive safe integer");
    }
    this.maxQueryCells = maxQueryCells;
    this.identity = null;
    this.generationKey = null;
    this.rows = new Map();
    this.pages = new Map();
    this.grids = new Map();
    this.metrics = {
      replacements: 0,
      patch_rebuilds: 0,
      page_evictions: 0,
      page_revisits: 0,
      stale_rejections: 0,
      skipped_invalid: 0,
      queries: 0,
      authoring_mutations: 0,
      revisions_emitted: 0,
    };
  }

  #build(identity, nodes) {
    if (!Array.isArray(nodes)) throw new TypeError("nodes must be an array");
    const normalizedIdentity = normalizeSpatialIdentity(identity);
    const allowedPages = new Set(normalizedIdentity.page_ids);
    const rows = new Map();
    const pages = new Map();
    const grids = new Map();
    let skipped = 0;

    for (const node of nodes) {
      const { row } = normalizeSpatialGeometry(node);
      if (!row) {
        skipped += 1;
        continue;
      }
      if (!allowedPages.has(row.page_id)) {
        skipped += 1;
        continue;
      }
      if (rows.has(row.node_id)) throw new Error("duplicate node_id in current spatial window");
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
    return {
      identity: normalizedIdentity,
      generationKey: spatialGenerationKey(normalizedIdentity),
      rows,
      pages,
      grids,
      skipped,
    };
  }

  replaceWindow(identity, nodes) {
    const built = this.#build(identity, nodes);
    this.identity = built.identity;
    this.generationKey = built.generationKey;
    this.rows = built.rows;
    this.pages = built.pages;
    this.grids = built.grids;
    this.metrics.replacements += 1;
    this.metrics.skipped_invalid += built.skipped;
    return this.receipt();
  }

  #assertCurrent(generationKey) {
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
    for (const cell of cells) {
      for (const nodeId of grid.get(cell) ?? []) ids.add(nodeId);
    }
    return {
      ids: [...ids].sort(),
      cellsVisited: cells.length,
      candidateChecks: ids.size,
    };
  }

  queryPoint(pageId, point, generationKey) {
    this.#assertCurrent(generationKey);
    checkedString(pageId, "pageId");
    assertSafeEmu(point.x_emu, "point.x_emu");
    assertSafeEmu(point.y_emu, "point.y_emu");
    const query = { x: point.x_emu, y: point.y_emu, width: 1, height: 1 };
    const stats = this.#candidateIds(pageId, query);
    const candidates = stats.ids
      .map((nodeId) => this.rows.get(nodeId))
      .filter((row) => containsPoint(row.bounds, point));
    this.metrics.queries += 1;
    return Object.freeze({
      generation_key: this.generationKey,
      candidates: Object.freeze(candidates),
      stats: Object.freeze({
        cells_visited: stats.cellsVisited,
        candidate_checks: stats.candidateChecks,
      }),
    });
  }

  queryBox(pageId, bounds, generationKey) {
    this.#assertCurrent(generationKey);
    checkedString(pageId, "pageId");
    const query = checkedBounds(bounds, "query.bounds");
    const stats = this.#candidateIds(pageId, query);
    const candidates = stats.ids
      .map((nodeId) => this.rows.get(nodeId))
      .filter((row) => intersects(row.bounds, query));
    this.metrics.queries += 1;
    return Object.freeze({
      generation_key: this.generationKey,
      candidates: Object.freeze(candidates),
      stats: Object.freeze({
        cells_visited: stats.cellsVisited,
        candidate_checks: stats.candidateChecks,
      }),
    });
  }

  hitTestCandidates(pageId, point, generationKey) {
    return this.queryPoint(pageId, point, generationKey);
  }

  marqueeCandidates(pageId, bounds, generationKey) {
    return this.queryBox(pageId, bounds, generationKey);
  }

  snapGeometryCandidates(pageId, bounds, generationKey) {
    return this.queryBox(pageId, bounds, generationKey);
  }

  cullCandidates(pageId, bounds, generationKey) {
    return this.queryBox(pageId, bounds, generationKey);
  }

  applyNodePatch({
    base_generation_key,
    next_identity,
    remove_node_ids = [],
    upsert_nodes = [],
  }) {
    this.#assertCurrent(base_generation_key);
    if (!Array.isArray(remove_node_ids) || !Array.isArray(upsert_nodes)) {
      throw new TypeError("patch node arrays required");
    }
    const remove = new Set(remove_node_ids);
    const upsertIds = new Set(upsert_nodes.map((node) => node.node_id));
    const current = [...this.rows.values()]
      .filter((row) => !remove.has(row.node_id) && !upsertIds.has(row.node_id))
      .map((row) => ({
        node_id: row.node_id,
        page_id: row.page_id,
        bounds: { ...row.bounds },
        paint_order: row.paint_order,
        z_order: row.z_order,
      }));
    this.metrics.patch_rebuilds += 1;
    return this.replaceWindow(next_identity, [...current, ...upsert_nodes]);
  }

  evictPage({ base_generation_key, next_identity, page_id }) {
    this.#assertCurrent(base_generation_key);
    const kept = [...this.rows.values()]
      .filter((row) => row.page_id !== page_id)
      .map((row) => ({
        node_id: row.node_id,
        page_id: row.page_id,
        bounds: { ...row.bounds },
        paint_order: row.paint_order,
        z_order: row.z_order,
      }));
    this.metrics.page_evictions += 1;
    return this.replaceWindow(next_identity, kept);
  }

  revisitPage({ base_generation_key, next_identity, page_id, nodes }) {
    this.#assertCurrent(base_generation_key);
    checkedString(page_id, "page_id");
    if (!Array.isArray(nodes)) throw new TypeError("revisit nodes must be an array");
    const kept = [...this.rows.values()]
      .filter((row) => row.page_id !== page_id)
      .map((row) => ({
        node_id: row.node_id,
        page_id: row.page_id,
        bounds: { ...row.bounds },
        paint_order: row.paint_order,
        z_order: row.z_order,
      }));
    this.metrics.page_revisits += 1;
    return this.replaceWindow(next_identity, [...kept, ...nodes]);
  }

  reconcileSelection(nodeIds, generationKey) {
    this.#assertCurrent(generationKey);
    if (!Array.isArray(nodeIds)) throw new TypeError("nodeIds must be an array");
    const present = [];
    const removed = [];
    for (const nodeId of nodeIds) {
      checkedString(nodeId, "selection node_id");
      (this.rows.has(nodeId) ? present : removed).push(nodeId);
    }
    return Object.freeze({
      generation_key: this.generationKey,
      present: Object.freeze(present),
      removed: Object.freeze(removed),
    });
  }

  receipt() {
    return Object.freeze({
      schema: SPATIAL_LIFECYCLE_SCHEMA,
      generation_key: this.generationKey,
      identity: this.identity,
      page_count: this.pages.size,
      indexed_nodes: this.rows.size,
      metrics: Object.freeze({ ...this.metrics }),
      mechanism: Object.freeze({
        implementation: "uniform_grid_v1",
        semantic_contract: false,
        consumer_specific_indexes_allowed: true,
      }),
      authority: Object.freeze({
        canonical_document_state: false,
        hit_winner_authority: false,
        selection_authority: false,
        snap_policy_authority: false,
        authoring_mutations: 0,
        revisions_emitted: 0,
      }),
    });
  }
}
