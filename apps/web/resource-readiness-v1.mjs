export const RESOURCE_READINESS_SCHEMA = "chaptera.web-resource-readiness.v1";

const STATES = new Set(["unrequested","fetching","decoding","ready","failed","blocked","evicted"]);
const CONSUMER_KINDS = new Set(["page","segment","glyph","image","other"]);

function requireString(value, label) {
  if (typeof value !== "string" || value.length === 0) throw new TypeError(label + " must be a non-empty string");
  return value;
}

function resourceKey(identity) {
  return JSON.stringify([
    requireString(identity.resource_id, "resource_id"),
    requireString(identity.content_hash, "content_hash"),
    requireString(identity.derivative_id ?? "original", "derivative_id"),
  ]);
}

function sortedUnique(values) {
  return [...new Set(values)].sort();
}

export class ResourceReadinessRuntimeV1 {
  constructor({ document_id, revision_id, snapshot_id }) {
    this.documentId = requireString(document_id, "document_id");
    this.revisionId = requireString(revision_id, "revision_id");
    this.snapshotId = requireString(snapshot_id, "snapshot_id");
    this.backendGeneration = 1;
    this.resources = new Map();
    this.dependencies = new Map();
    this.reverse = new Map();
    this.invalidated = new Set();
    this.repaintPages = new Set();
    this.metrics = {
      requests_started: 0,
      stale_completions_rejected: 0,
      ready_transitions: 0,
      failed_transitions: 0,
      blocked_transitions: 0,
      evictions: 0,
      backend_resets: 0,
      compiled_invalidations: 0,
      repaint_schedules: 0,
      authoring_operations_emitted: 0,
      scene_patches_emitted: 0,
      canonical_revisions_emitted: 0,
    };
  }

  registerCompiledDependency({
    cache_entry_id,
    page_id,
    segment_id = null,
    resource_id,
    content_hash,
    derivative_id = "original",
    consumer_kind = "other",
  }) {
    cache_entry_id = requireString(cache_entry_id, "cache_entry_id");
    page_id = requireString(page_id, "page_id");
    if (!CONSUMER_KINDS.has(consumer_kind)) throw new TypeError("unsupported consumer_kind");
    const identity = { resource_id, content_hash, derivative_id };
    const key = resourceKey(identity);
    const dep = Object.freeze({
      cache_entry_id,
      page_id,
      segment_id: segment_id == null ? null : requireString(segment_id, "segment_id"),
      resource_id,
      content_hash,
      derivative_id,
      consumer_kind,
      revision_id: this.revisionId,
      snapshot_id: this.snapshotId,
    });
    const old = this.dependencies.get(cache_entry_id);
    if (old) {
      const oldKey = resourceKey(old);
      const set = this.reverse.get(oldKey);
      set?.delete(cache_entry_id);
      if (set?.size === 0) this.reverse.delete(oldKey);
    }
    this.dependencies.set(cache_entry_id, dep);
    if (!this.reverse.has(key)) this.reverse.set(key, new Set());
    this.reverse.get(key).add(cache_entry_id);
    return dep;
  }

  unregisterCompiledDependency(cacheEntryId) {
    const dep = this.dependencies.get(cacheEntryId);
    if (!dep) return false;
    const key = resourceKey(dep);
    this.dependencies.delete(cacheEntryId);
    const set = this.reverse.get(key);
    set?.delete(cacheEntryId);
    if (set?.size === 0) this.reverse.delete(key);
    this.invalidated.delete(cacheEntryId);
    return true;
  }

  #recordFor(identity) {
    const key = resourceKey(identity);
    return { key, record: this.resources.get(key) ?? null };
  }

  beginRequest(identity) {
    const key = resourceKey(identity);
    const previous = this.resources.get(key);
    const generation = (previous?.request_generation ?? 0) + 1;
    const record = {
      identity: Object.freeze({
        resource_id: identity.resource_id,
        content_hash: identity.content_hash,
        derivative_id: identity.derivative_id ?? "original",
      }),
      state: "fetching",
      request_generation: generation,
      material_generation: previous?.material_generation ?? 0,
      backend_generation: this.backendGeneration,
      binding_identity: null,
      temporary_visual: null,
      terminal_reason: null,
    };
    this.resources.set(key, record);
    this.metrics.requests_started += 1;
    return Object.freeze({ ...record.identity, request_generation: generation, backend_generation: this.backendGeneration });
  }

  #acceptToken(token) {
    const { key, record } = this.#recordFor(token);
    if (
      !record ||
      token.request_generation !== record.request_generation ||
      token.backend_generation !== this.backendGeneration ||
      token.backend_generation !== record.backend_generation
    ) {
      this.metrics.stale_completions_rejected += 1;
      return { accepted: false, key, record };
    }
    return { accepted: true, key, record };
  }

  markDecoding(token) {
    const check = this.#acceptToken(token);
    if (!check.accepted) return { accepted: false, reason: "stale_generation" };
    if (check.record.state !== "fetching") throw new Error("decoding transition requires fetching");
    check.record.state = "decoding";
    return { accepted: true, state: "decoding" };
  }

  setTemporaryVisual(token, kind) {
    const check = this.#acceptToken(token);
    if (!check.accepted) return { accepted: false, reason: "stale_generation" };
    if (!["fetching","decoding"].includes(check.record.state)) {
      throw new Error("temporary visual is only legal while resource is pending");
    }
    if (!["placeholder","fallback_font","preview_derivative"].includes(kind)) {
      throw new TypeError("unsupported temporary visual");
    }
    check.record.temporary_visual = kind;
    return { accepted: true, state: check.record.state, temporary_visual: kind, fulfills_exact_resource: false };
  }

  #invalidateKey(key, reason) {
    const ids = [...(this.reverse.get(key) ?? [])].sort();
    const pages = [];
    for (const id of ids) {
      const dep = this.dependencies.get(id);
      if (!dep) continue;
      if (!this.invalidated.has(id)) {
        this.invalidated.add(id);
        this.metrics.compiled_invalidations += 1;
      }
      pages.push(dep.page_id);
      if (!this.repaintPages.has(dep.page_id)) {
        this.repaintPages.add(dep.page_id);
        this.metrics.repaint_schedules += 1;
      }
    }
    return {
      reason,
      cache_entry_ids: ids,
      page_ids: sortedUnique(pages),
      revision_id: this.revisionId,
      snapshot_id: this.snapshotId,
    };
  }

  completeReady(token, { binding_identity }) {
    const check = this.#acceptToken(token);
    if (!check.accepted) return { accepted: false, reason: "stale_generation" };
    if (!["fetching","decoding"].includes(check.record.state)) {
      throw new Error("ready transition requires fetching/decoding");
    }
    requireString(binding_identity, "binding_identity");
    check.record.state = "ready";
    check.record.material_generation += 1;
    check.record.binding_identity = binding_identity;
    check.record.temporary_visual = null;
    check.record.terminal_reason = null;
    this.metrics.ready_transitions += 1;
    return {
      accepted: true,
      state: "ready",
      binding_identity,
      material_generation: check.record.material_generation,
      invalidation: this.#invalidateKey(check.key, "resource_ready"),
      canonical_revision_changed: false,
    };
  }

  completeTerminal(token, { state, reason_code }) {
    if (!["failed","blocked"].includes(state)) throw new TypeError("terminal state must be failed or blocked");
    const check = this.#acceptToken(token);
    if (!check.accepted) return { accepted: false, reason: "stale_generation" };
    if (!["fetching","decoding"].includes(check.record.state)) {
      throw new Error("terminal transition requires fetching/decoding");
    }
    check.record.state = state;
    check.record.binding_identity = null;
    check.record.temporary_visual = null;
    check.record.terminal_reason = requireString(reason_code, "reason_code");
    if (state === "failed") this.metrics.failed_transitions += 1;
    if (state === "blocked") this.metrics.blocked_transitions += 1;
    return {
      accepted: true,
      state,
      reason_code,
      invalidation: this.#invalidateKey(check.key, "resource_" + state),
      auto_retry_scheduled: false,
      canonical_revision_changed: false,
    };
  }

  evict(identity) {
    const { key, record } = this.#recordFor(identity);
    if (!record || record.state !== "ready") return { evicted: false };
    record.state = "evicted";
    record.request_generation += 1;
    record.backend_generation = this.backendGeneration;
    record.binding_identity = null;
    record.temporary_visual = null;
    this.metrics.evictions += 1;
    return {
      evicted: true,
      invalidation: this.#invalidateKey(key, "resource_evicted"),
      canonical_revision_changed: false,
    };
  }

  backendReset() {
    this.backendGeneration += 1;
    this.metrics.backend_resets += 1;
    const invalidated = new Set();
    const pages = new Set();
    for (const [key, record] of this.resources) {
      if (record.state === "ready") {
        record.state = "evicted";
        record.request_generation += 1;
        record.backend_generation = this.backendGeneration;
        record.binding_identity = null;
        const result = this.#invalidateKey(key, "backend_reset");
        result.cache_entry_ids.forEach(id => invalidated.add(id));
        result.page_ids.forEach(id => pages.add(id));
      } else {
        record.request_generation += 1;
        record.backend_generation = this.backendGeneration;
      }
    }
    return {
      backend_generation: this.backendGeneration,
      cache_entry_ids: [...invalidated].sort(),
      page_ids: [...pages].sort(),
      canonical_revision_changed: false,
    };
  }

  resourceState(identity) {
    const { record } = this.#recordFor(identity);
    if (!record) return Object.freeze({ state: "unrequested" });
    if (!STATES.has(record.state)) throw new Error("internal readiness state corrupted");
    return Object.freeze({
      state: record.state,
      request_generation: record.request_generation,
      material_generation: record.material_generation,
      backend_generation: record.backend_generation,
      binding_identity: record.binding_identity,
      temporary_visual: record.temporary_visual,
      terminal_reason: record.terminal_reason,
    });
  }

  drainRepaintQueue() {
    const pages = [...this.repaintPages].sort();
    this.repaintPages.clear();
    return pages;
  }

  acknowledgeRecompiled(cacheEntryIds) {
    for (const id of cacheEntryIds) this.invalidated.delete(id);
    return this.invalidatedCacheEntries();
  }

  invalidatedCacheEntries() {
    return [...this.invalidated].sort();
  }

  receipt() {
    const resources = [...this.resources.values()]
      .map(record => ({
        ...record.identity,
        state: record.state,
        request_generation: record.request_generation,
        material_generation: record.material_generation,
        backend_generation: record.backend_generation,
        binding_ready: record.binding_identity !== null,
        temporary_visual: record.temporary_visual,
        terminal_reason: record.terminal_reason,
      }))
      .sort((a,b) => resourceKey(a).localeCompare(resourceKey(b)));
    return {
      schema: RESOURCE_READINESS_SCHEMA,
      document_id: this.documentId,
      canonical_revision_id: this.revisionId,
      snapshot_id: this.snapshotId,
      backend_generation: this.backendGeneration,
      resources,
      dependency_count: this.dependencies.size,
      invalidated_cache_entries: this.invalidatedCacheEntries(),
      pending_repaint_pages: [...this.repaintPages].sort(),
      metrics: { ...this.metrics },
      authority: {
        runtime_only: true,
        authoring_operations_emitted: 0,
        scene_patches_emitted: 0,
        canonical_revisions_emitted: 0,
      },
    };
  }
}

export { resourceKey };
