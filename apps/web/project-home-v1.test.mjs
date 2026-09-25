import test from "node:test";
import assert from "node:assert/strict";
import { WebProjectHomeControllerV1, normalizeProjectCardV1 } from "./project-home-v1.mjs";

function project(overrides = {}) {
  return {
    project_id: "project:1",
    document_id: "document:1",
    name: "Newsletter",
    lifecycle_state: "active",
    lifecycle_generation: 2,
    metadata_version: 5,
    current_revision_id: "sha256:" + "a".repeat(64),
    workspace_id: "workspace:a",
    ...overrides,
  };
}

function services() {
  const current = new Map([["project:1", project()]]);
  return {
    current,
    projections: {
      async listRecent() { return [{ project: current.get("project:1"), activity_order: 99, kind: "open" }]; },
      async search(query) { return query === "news" ? [{ project: current.get("project:1") }] : []; },
      async thumbnailForProject(p) {
        return { freshness: "fresh", artifact_id: "thumb:1", key: { revision_id: p.current_revision_id } };
      },
    },
    lifecycle: {
      async getProject(id) { return structuredClone(current.get(id)); },
      async renameProject(args) {
        const p = current.get(args.project_id);
        assert.equal(args.expected_lifecycle_generation, p.lifecycle_generation);
        assert.equal(args.expected_metadata_version, p.metadata_version);
        const next = { ...p, name: args.name, metadata_version: p.metadata_version + 1 };
        current.set(args.project_id, next);
        return structuredClone(next);
      },
      async forkProject(args) {
        const source = current.get(args.source_project_id);
        assert.equal(args.selected_revision_id, source.current_revision_id);
        return project({
          project_id: "project:fork",
          document_id: "document:fork",
          name: args.name ?? "Newsletter copy",
          lifecycle_generation: 0,
          metadata_version: 0,
          current_revision_id: "revision:genesis:fork",
        });
      },
      async trashProject(args) {
        const p = current.get(args.project_id);
        assert.equal(args.expected_lifecycle_generation, p.lifecycle_generation);
        const next = { ...p, lifecycle_state: "trashed", lifecycle_generation: p.lifecycle_generation + 1 };
        current.set(args.project_id, next);
        return structuredClone(next);
      },
      async restoreProject(args) {
        const p = current.get(args.project_id);
        assert.equal(args.expected_lifecycle_generation, p.lifecycle_generation);
        const next = { ...p, lifecycle_state: "active", lifecycle_generation: p.lifecycle_generation + 1 };
        current.set(args.project_id, next);
        return structuredClone(next);
      },
    },
  };
}

test("card preserves explicit thumbnail freshness and recent activity", () => {
  const card = normalizeProjectCardV1({
    project: project(),
    thumbnail: { freshness: "stale", artifact_id: "thumb:old", key: { revision_id: "rev:old" } },
    recent: { activity_order: 10, kind: "edit" },
  });
  assert.equal(card.thumbnail.freshness, "stale");
  assert.equal(card.thumbnail.revision_id, "rev:old");
  assert.equal(card.recent_activity_order, 10);
});

test("recent comes from projection order, not revision chronology", async () => {
  const { projections, lifecycle } = services();
  const controller = new WebProjectHomeControllerV1({ projections, lifecycle, requestIdFactory: () => "request-0001" });
  const state = await controller.loadRecent();
  assert.equal(state.mode, "recent");
  assert.equal(state.cards[0].recent_activity_order, 99);
  assert.equal(state.cards[0].thumbnail.freshness, "fresh");
});

test("blank search returns Recent instead of inventing a local search index", async () => {
  const { projections, lifecycle } = services();
  const controller = new WebProjectHomeControllerV1({ projections, lifecycle, requestIdFactory: () => "request-0001" });
  const state = await controller.search("   ");
  assert.equal(state.mode, "recent");
});

test("rename re-reads current project and uses split lifecycle/metadata fences", async () => {
  const { projections, lifecycle, current } = services();
  let getCalls = 0;
  const original = lifecycle.getProject;
  lifecycle.getProject = async (id) => { getCalls += 1; return original(id); };
  const controller = new WebProjectHomeControllerV1({ projections, lifecycle, requestIdFactory: () => "rename-request-01" });
  const renamed = await controller.rename("project:1", "Fall Newsletter");
  assert.equal(getCalls, 1);
  assert.equal(renamed.name, "Fall Newsletter");
  assert.equal(renamed.lifecycle_generation, 2);
  assert.equal(renamed.metadata_version, 6);
  assert.equal(current.get("project:1").document_id, "document:1");
});

test("duplicate forks current revision into new project and document identities", async () => {
  const { projections, lifecycle } = services();
  const controller = new WebProjectHomeControllerV1({ projections, lifecycle, requestIdFactory: () => "fork-request-0001" });
  const copy = await controller.duplicate("project:1");
  assert.equal(copy.project_id, "project:fork");
  assert.equal(copy.document_id, "document:fork");
  assert.notEqual(copy.current_revision_id, project().current_revision_id);
});

test("duplicate fails closed if backend reuses source identity", async () => {
  const { projections, lifecycle } = services();
  lifecycle.forkProject = async () => project();
  const controller = new WebProjectHomeControllerV1({ projections, lifecycle, requestIdFactory: () => "fork-request-0002" });
  await assert.rejects(() => controller.duplicate("project:1"), /reused source identity/);
});

test("trash and restore re-read current lifecycle generation for each command", async () => {
  const { projections, lifecycle } = services();
  const controller = new WebProjectHomeControllerV1({ projections, lifecycle, requestIdFactory: () => "life-request-0001" });
  const trashed = await controller.trash("project:1");
  assert.equal(trashed.lifecycle_state, "trashed");
  assert.equal(trashed.lifecycle_generation, 3);
  const restored = await controller.restore("project:1");
  assert.equal(restored.lifecycle_state, "active");
  assert.equal(restored.lifecycle_generation, 4);
});
