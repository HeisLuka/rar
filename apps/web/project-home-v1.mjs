function requireMethod(object, name, label) {
  if (!object || typeof object[name] !== "function") {
    throw new TypeError((label ?? "service") + " must implement " + name + "()");
  }
}

function clone(value) {
  return value == null ? value : structuredClone(value);
}

function requireProjectRow(row) {
  if (!row || typeof row !== "object") throw new TypeError("project row required");
  if (typeof row.project_id !== "string" || !row.project_id) throw new TypeError("project_id required");
  if (typeof row.document_id !== "string" || !row.document_id) throw new TypeError("document_id required");
  if (typeof row.name !== "string") throw new TypeError("name required");
  if (!Number.isSafeInteger(row.lifecycle_generation) || row.lifecycle_generation < 0) {
    throw new TypeError("lifecycle_generation required");
  }
  if (!Number.isSafeInteger(row.metadata_version) || row.metadata_version < 0) {
    throw new TypeError("metadata_version required");
  }
  return row;
}

export function normalizeProjectCardV1({ project, thumbnail = null, recent = null }) {
  const p = requireProjectRow(project);
  const freshness = thumbnail?.freshness ?? "missing";
  if (!["fresh", "stale", "missing"].includes(freshness)) {
    throw new TypeError("thumbnail freshness must be fresh/stale/missing");
  }
  return Object.freeze({
    project_id: p.project_id,
    document_id: p.document_id,
    name: p.name,
    lifecycle_state: p.lifecycle_state ?? "active",
    lifecycle_generation: p.lifecycle_generation,
    metadata_version: p.metadata_version,
    current_revision_id: p.current_revision_id ?? null,
    workspace_id: p.workspace_id ?? null,
    recent_activity_order: recent?.activity_order ?? null,
    recent_activity_kind: recent?.kind ?? null,
    thumbnail: Object.freeze({
      freshness,
      artifact_id: thumbnail?.artifact_id ?? null,
      revision_id: thumbnail?.key?.revision_id ?? null,
    }),
  });
}

export class WebProjectHomeControllerV1 {
  constructor({ projections, lifecycle, requestIdFactory = null, onState = null }) {
    for (const name of ["listRecent", "search", "thumbnailForProject"]) {
      requireMethod(projections, name, "projections");
    }
    for (const name of ["getProject", "renameProject", "forkProject", "trashProject", "restoreProject"]) {
      requireMethod(lifecycle, name, "lifecycle");
    }
    this.projections = projections;
    this.lifecycle = lifecycle;
    this.requestIdFactory = requestIdFactory ?? (() => crypto.randomUUID());
    this.onState = onState;
    this._state = Object.freeze({
      protocol_version: "chaptera.web-project-home-state.v1",
      mode: "idle",
      query: "",
      cards: [],
      busy_project_id: null,
      error_code: null,
    });
  }

  state() { return clone(this._state); }

  async loadRecent() {
    this._set({ mode: "loading_recent", query: "", error_code: null });
    try {
      const rows = await this.projections.listRecent();
      const cards = await this._cards(rows);
      this._set({ mode: "recent", cards });
      return this.state();
    } catch (error) {
      return this._fail(error);
    }
  }

  async search(query) {
    const q = String(query ?? "").trim();
    if (!q) return this.loadRecent();
    this._set({ mode: "searching", query: q, error_code: null });
    try {
      const rows = await this.projections.search(q);
      const cards = await this._cards(rows);
      this._set({ mode: "search", query: q, cards });
      return this.state();
    } catch (error) {
      return this._fail(error);
    }
  }

  async rename(projectId, name) {
    const nextName = String(name ?? "").trim();
    if (!nextName) throw new TypeError("project name must be non-empty");
    return this._mutate(projectId, async (current, requestId) =>
      this.lifecycle.renameProject({
        project_id: current.project_id,
        expected_lifecycle_generation: current.lifecycle_generation,
        expected_metadata_version: current.metadata_version,
        name: nextName,
        request_id: requestId,
      }),
    );
  }

  async duplicate(projectId, { name = null } = {}) {
    this._set({ busy_project_id: projectId, error_code: null });
    try {
      const current = requireProjectRow(await this.lifecycle.getProject(projectId));
      if (typeof current.current_revision_id !== "string" || !current.current_revision_id) {
        throw Object.assign(new Error("current revision required for fork"), { code: "missing_current_revision" });
      }
      const result = requireProjectRow(await this.lifecycle.forkProject({
        source_project_id: current.project_id,
        selected_revision_id: current.current_revision_id,
        target_workspace_id: current.workspace_id,
        name: name == null ? null : String(name),
        request_id: this._requestId("fork"),
      }));
      if (result.project_id === current.project_id || result.document_id === current.document_id) {
        throw Object.assign(new Error("fork reused source identity"), { code: "fork_identity_reuse" });
      }
      this._set({ busy_project_id: null });
      return clone(result);
    } catch (error) {
      this._set({ busy_project_id: null, error_code: error?.code ?? "project_home_failed" });
      throw error;
    }
  }

  async trash(projectId) {
    return this._mutate(projectId, async (current, requestId) =>
      this.lifecycle.trashProject({
        project_id: current.project_id,
        expected_lifecycle_generation: current.lifecycle_generation,
        request_id: requestId,
      }),
    );
  }

  async restore(projectId) {
    return this._mutate(projectId, async (current, requestId) =>
      this.lifecycle.restoreProject({
        project_id: current.project_id,
        expected_lifecycle_generation: current.lifecycle_generation,
        request_id: requestId,
      }),
    );
  }

  async _mutate(projectId, action) {
    this._set({ busy_project_id: projectId, error_code: null });
    try {
      const current = requireProjectRow(await this.lifecycle.getProject(projectId));
      const result = requireProjectRow(await action(current, this._requestId("project")));
      this._set({ busy_project_id: null });
      return clone(result);
    } catch (error) {
      this._set({ busy_project_id: null, error_code: error?.code ?? "project_home_failed" });
      throw error;
    }
  }

  async _cards(rows) {
    if (!Array.isArray(rows)) throw new TypeError("projection rows must be an array");
    const cards = [];
    for (const row of rows) {
      const project = requireProjectRow(row.project ?? row);
      const thumbnail = await this.projections.thumbnailForProject(project);
      cards.push(normalizeProjectCardV1({ project, thumbnail, recent: row.recent ?? row }));
    }
    return cards;
  }

  _requestId(kind) {
    const id = this.requestIdFactory(kind);
    if (typeof id !== "string" || id.length < 8) throw new TypeError("request id must be a bounded string");
    return id;
  }

  _fail(error) {
    this._set({ mode: "error", error_code: error?.code ?? "project_home_failed" });
    return this.state();
  }

  _set(patch) {
    this._state = Object.freeze({ ...this._state, ...patch });
    this.onState?.(this.state());
  }
}
