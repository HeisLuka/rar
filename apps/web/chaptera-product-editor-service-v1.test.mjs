import test from "node:test";
import assert from "node:assert/strict";

import { ChapteraProductEditorServiceV1 } from "./chaptera-product-editor-service-v1.mjs";

const DOC = "10000000-0000-4000-8000-000000000001";
const PAGE = "20000000-0000-4000-8000-000000000001";
const NODE = "30000000-0000-4000-8000-000000000001";
const SOURCE = "a".repeat(64);
const BASE = "sha256:" + "b".repeat(64);
const CHILD = "sha256:" + "c".repeat(64);
const LAYOUT = "sha256:" + "d".repeat(64);

function json(value, status = 200) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function session(csrf = "csrf-token-1") {
  return {
    principal_id: "principal:test",
    csrf_token: csrf,
    idle_expires_at_ms: Date.now() + 60_000,
    absolute_expires_at_ms: Date.now() + 600_000,
  };
}

function current(revisionId = BASE, x = 1000) {
  return {
    protocol_version: "chaptera.current-document.v1",
    document_id: DOC,
    source_hash: SOURCE,
    revision_id: revisionId,
    revision_cursor: revisionId === BASE ? 0 : 1,
    canonical_revision_schema_version: "chaptera.cdm.authoring-revision.v1",
    canonical_authoring_revision_id: "e".repeat(64),
    project: {
      schema_version: "pub-editor-v0.11",
      source_hash: SOURCE,
      operations: [],
    },
    authoring_graph: {
      cdm_version: "chaptera-cdm-v0.1",
      resolver_version: "pub-resolver-v1",
      source: { source_hash: SOURCE },
      document: {
        id: DOC,
        format_origin: "pub",
        source_hash: SOURCE,
        pages: [PAGE],
        resources: [],
        styles: [],
      },
      pages: {
        [PAGE]: {
          id: PAGE,
          size: { width: 10_058_400, height: 7_772_400 },
          bleed: null,
          margins: null,
          children: [NODE],
        },
      },
      nodes: {
        [NODE]: {
          kind: "shape",
          header: {
            id: NODE,
            parent_id: PAGE,
            bounds: { x, y: 2000, width: 3000, height: 4000 },
            transform: { a: "1", b: "0", c: "0", d: "1", tx: 0, ty: 0 },
          },
          payload: {
            contents_seq_num: 1,
            officeart_shape_type: null,
            officeart_spid: null,
            image_slot: null,
            explicit_paint: {},
            story_frame: null,
            table: null,
          },
        },
      },
      stories: {},
      paragraphs: {},
      text_runs: {},
      resources: {},
      styles: {},
      extensions: {},
    },
  };
}

function accepted() {
  return {
    protocol_version: "chaptera.commit-accepted.v1",
    document_id: DOC,
    source_hash: SOURCE,
    base_revision_id: BASE,
    revision_id: CHILD,
    state_id: "sha256:" + "f".repeat(64),
    client_operation_id: "move-1",
    canonical_operation: {
      kind: "move_node",
      node_id: NODE,
      before: { x: 1000, y: 2000, width: 3000, height: 4000 },
      after: { x: 9525, y: 10525, width: 3000, height: 4000 },
    },
    project_schema_version: "pub-editor-v0.11",
    consequences: [],
    scene_refresh: "full_snapshot",
    replayed: false,
  };
}

function commitRequest() {
  return {
    protocol_version: "chaptera.commit-request.v1",
    document_id: DOC,
    source_hash: SOURCE,
    base_revision_id: BASE,
    client_operation_id: "move-1",
    command: {
      kind: "move_node_to",
      node_id: NODE,
      x_emu: 9525,
      y_emu: 10525,
    },
  };
}

function recorder(handler) {
  const calls = [];
  const fetchImpl = async (url, options = {}) => {
    calls.push({
      url,
      path: new URL(url).pathname,
      options: structuredClone({
        ...options,
        body: options.body ?? null,
        headers: { ...(options.headers ?? {}) },
      }),
    });
    return handler(url, options, calls.length - 1);
  };
  return { calls, fetchImpl };
}

test("current Scene uses cookie credentials and canonical current authoring graph", async () => {
  const { calls, fetchImpl } = recorder((url) => {
    assert.equal(new URL(url).pathname, "/v1/documents/" + DOC + "/current");
    return json(current());
  });
  const service = new ChapteraProductEditorServiceV1("https://chaptera.test", {
    documentId: DOC,
    fetchImpl,
  });

  const scene = await service.currentScene();
  assert.equal(scene.protocol_version, "chaptera.scene.v1");
  assert.equal(scene.revision_id, BASE);
  assert.equal(scene.nodes[0].bounds.x, 1000);
  assert.equal(calls[0].options.credentials, "include");
  assert.equal(calls[0].options.cache, "no-store");
  assert.ok(!("x-chaptera-principal-id" in calls[0].options.headers));
});

test("commit obtains server CSRF and uses document-scoped canonical route", async () => {
  const { calls, fetchImpl } = recorder((url) => {
    const path = new URL(url).pathname;
    if (path === "/v1/session") return json(session());
    if (path === "/v1/documents/" + DOC + "/commit") return json(accepted());
    throw new Error("unexpected path " + path);
  });
  const service = new ChapteraProductEditorServiceV1("https://chaptera.test", {
    documentId: DOC,
    fetchImpl,
  });

  const result = await service.commit(commitRequest());
  assert.equal(result.protocol_version, "chaptera.commit-accepted.v1");
  assert.equal(result.revision_id, CHILD);
  assert.deepEqual(
    calls.map((call) => call.path),
    ["/v1/session", "/v1/documents/" + DOC + "/commit"],
  );
  assert.equal(calls[1].options.credentials, "include");
  assert.equal(calls[1].options.headers["x-csrf-token"], "csrf-token-1");
  assert.equal(calls[1].options.headers["content-type"], "application/json");
  assert.ok(!("x-chaptera-principal-id" in calls[1].options.headers));
});

test("stale commit re-reads canonical current head before shell rejection", async () => {
  const { calls, fetchImpl } = recorder((url) => {
    const path = new URL(url).pathname;
    if (path === "/v1/session") return json(session());
    if (path === "/v1/documents/" + DOC + "/commit") {
      return json(
        {
          error: {
            code: "stale_revision",
            message: "base revision is no longer current",
          },
        },
        409,
      );
    }
    if (path === "/v1/documents/" + DOC + "/current") return json(current(CHILD, 9525));
    throw new Error("unexpected path " + path);
  });
  const service = new ChapteraProductEditorServiceV1("https://chaptera.test", {
    documentId: DOC,
    fetchImpl,
  });

  const result = await service.commit(commitRequest());
  assert.deepEqual(result, {
    protocol_version: "chaptera.commit-rejected.v1",
    document_id: DOC,
    base_revision_id: BASE,
    current_revision_id: CHILD,
    client_operation_id: "move-1",
    code: "stale_revision",
    message_key: "revision.stale_revision",
    retryable: true,
  });
  assert.equal(calls.at(-1).path, "/v1/documents/" + DOC + "/current");
});

test("exact scene reconciliation refuses a silently advanced current head", async () => {
  const { fetchImpl } = recorder(() => json(current(CHILD, 9525)));
  const service = new ChapteraProductEditorServiceV1("https://chaptera.test", {
    documentId: DOC,
    fetchImpl,
  });

  await assert.rejects(
    service.sceneForRevision(BASE),
    /advanced before exact Scene reconciliation/,
  );
  const scene = await service.sceneForRevision(CHILD);
  assert.equal(scene.revision_id, CHILD);
  assert.equal(scene.nodes[0].bounds.x, 9525);
});

test("csrf_invalid refreshes /v1/session once and retries the same mutation", async () => {
  let sessionCount = 0;
  let commitCount = 0;
  const { calls, fetchImpl } = recorder((url, options) => {
    const path = new URL(url).pathname;
    if (path === "/v1/session") {
      sessionCount += 1;
      return json(session("csrf-token-" + sessionCount));
    }
    if (path === "/v1/documents/" + DOC + "/commit") {
      commitCount += 1;
      if (commitCount === 1) {
        assert.equal(options.headers["x-csrf-token"], "csrf-token-1");
        return json({ error: "csrf_invalid" }, 403);
      }
      assert.equal(options.headers["x-csrf-token"], "csrf-token-2");
      return json(accepted());
    }
    throw new Error("unexpected path " + path);
  });
  const service = new ChapteraProductEditorServiceV1("https://chaptera.test", {
    documentId: DOC,
    fetchImpl,
  });

  const result = await service.commit(commitRequest());
  assert.equal(result.revision_id, CHILD);
  assert.equal(sessionCount, 2);
  assert.equal(commitCount, 2);
  assert.deepEqual(
    calls.map((call) => call.path),
    [
      "/v1/session",
      "/v1/documents/" + DOC + "/commit",
      "/v1/session",
      "/v1/documents/" + DOC + "/commit",
    ],
  );
});

test("direct export API uses canonical create/status/download/cancel surfaces without preview", async () => {
  const { calls, fetchImpl } = recorder((url) => {
    const path = new URL(url).pathname;
    if (path === "/v1/session") return json(session());
    if (path === "/v1/exports") {
      return json({
        protocol_version: "chaptera.export-job-http.v1",
        job_id: "job:1",
        document_id: DOC,
        revision_id: CHILD,
        target_profile: "idml",
        layout_environment_id: LAYOUT,
        status: "queued",
        artifact_id: null,
        loss_report_id: null,
        error_code: null,
        progress_percent: null,
      });
    }
    if (path === "/v1/exports/job%3A1") {
      return json({
        protocol_version: "chaptera.export-job-http.v1",
        job_id: "job:1",
        document_id: DOC,
        revision_id: CHILD,
        target_profile: "idml",
        layout_environment_id: LAYOUT,
        status: "ready",
        artifact_id: "binding:artifact",
        loss_report_id: "binding:loss",
        error_code: null,
        progress_percent: null,
      });
    }
    if (path === "/v1/exports/job%3A1/download") {
      return json({
        protocol_version: "chaptera.export-download.v1",
        job_id: "job:1",
        artifact_id: "binding:artifact",
        download_handle: "https://download.invalid/opaque",
        expires_at_ms: Date.now() + 60_000,
      });
    }
    if (path === "/v1/exports/job%3A1/cancel") {
      return json({
        protocol_version: "chaptera.export-job-http.v1",
        job_id: "job:1",
        document_id: DOC,
        revision_id: CHILD,
        target_profile: "idml",
        layout_environment_id: LAYOUT,
        status: "cancelled",
        artifact_id: null,
        loss_report_id: null,
        error_code: null,
        progress_percent: null,
      });
    }
    throw new Error("unexpected path " + path);
  });
  const service = new ChapteraProductEditorServiceV1("https://chaptera.test", {
    documentId: DOC,
    fetchImpl,
  });

  const created = await service.createExport({
    revisionId: CHILD,
    targetProfile: "idml",
    layoutEnvironmentId: LAYOUT,
    clientRequestId: "export-request-1",
  });
  assert.equal(created.status, "queued");

  const status = await service.exportStatus("job:1");
  assert.equal(status.status, "ready");
  assert.equal(status.artifact_id, "binding:artifact");

  const download = await service.authorizeExportDownload("job:1", "binding:artifact");
  assert.equal(download.protocol_version, "chaptera.export-download.v1");

  const cancelled = await service.cancelExport("job:1");
  assert.equal(cancelled.status, "cancelled");

  assert.equal(
    calls.filter((call) => call.path.includes("preview")).length,
    0,
  );
  for (const call of calls) {
    assert.equal(call.options.credentials, "include");
    assert.ok(!("x-chaptera-principal-id" in call.options.headers));
  }
  for (const call of calls.filter((call) => call.options.method === "POST")) {
    assert.equal(call.options.headers["x-csrf-token"], "csrf-token-1");
  }
});
