import test from "node:test";
import assert from "node:assert/strict";

import { projectCurrentAuthoringGraphToScene } from "./current-authoring-graph-scene-v1.mjs";
import { buildRenderPlan } from "./render-v1.mjs";

const DOC = "10000000-0000-4000-8000-000000000001";
const PAGE = "20000000-0000-4000-8000-000000000001";
const GROUP = "30000000-0000-4000-8000-000000000001";
const FRAME = "30000000-0000-4000-8000-000000000002";
const STORY = "40000000-0000-4000-8000-000000000001";
const SOURCE = "a".repeat(64);
const REVISION = "sha256:" + "b".repeat(64);

function identityTransform() {
  return { a: "1", b: "0", c: "0", d: "1", tx: 0, ty: 0 };
}

function currentFixture() {
  return {
    protocol_version: "chaptera.current-document.v1",
    document_id: DOC,
    source_hash: SOURCE,
    revision_id: REVISION,
    revision_cursor: 0,
    canonical_revision_schema_version: "chaptera.cdm.authoring-revision.v1",
    canonical_authoring_revision_id: "c".repeat(64),
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
          children: [GROUP],
        },
      },
      nodes: {
        [GROUP]: {
          kind: "group",
          header: {
            id: GROUP,
            parent_id: PAGE,
            bounds: { x: 100_000, y: 200_000, width: 4_000_000, height: 3_000_000 },
            transform: identityTransform(),
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
        [FRAME]: {
          kind: "text_frame",
          header: {
            id: FRAME,
            parent_id: GROUP,
            bounds: { x: 300_000, y: 400_000, width: 2_000_000, height: 600_000 },
            transform: identityTransform(),
          },
          payload: {
            contents_seq_num: 2,
            officeart_shape_type: null,
            officeart_spid: null,
            image_slot: null,
            explicit_paint: {},
            story_frame: {
              story_id: STORY,
              ordinal: 0,
              previous_frame: null,
              next_frame: null,
            },
            table: null,
          },
        },
      },
      stories: {
        [STORY]: {
          id: STORY,
          text: "Hello, canonical graph",
          paragraphs: [],
          runs: [],
        },
      },
      paragraphs: {},
      text_runs: {},
      resources: {},
      styles: {},
      extensions: {},
    },
  };
}

test("projects authoritative graph geometry into a renderer-safe partial SceneV1", async () => {
  const scene = await projectCurrentAuthoringGraphToScene(currentFixture());

  assert.equal(scene.protocol_version, "chaptera.scene.v1");
  assert.equal(scene.document_id, DOC);
  assert.equal(scene.source_hash, SOURCE);
  assert.equal(scene.revision_id, REVISION);
  assert.match(scene.snapshot_id, /^sha256:[0-9a-f]{64}$/);
  assert.equal(scene.stacking_fidelity, "unknown");
  assert.deepEqual(scene.fidelity, {
    state: "partial",
    reasons: [
      "paint_projection_deferred",
      "resource_projection_deferred",
      "stacking_order_unavailable",
      "text_style_projection_deferred",
    ],
  });

  assert.deepEqual(scene.pages, [
    {
      page_id: PAGE,
      order: 0,
      width_emu: 10_058_400,
      height_emu: 7_772_400,
    },
  ]);
  assert.equal(scene.nodes.length, 2);
  const group = scene.nodes.find((node) => node.node_id === GROUP);
  const frame = scene.nodes.find((node) => node.node_id === FRAME);
  assert.equal(group.page_id, PAGE);
  assert.equal(group.parent_node_id, null);
  assert.equal(group.kind, "group");
  assert.equal(group.z_order, null);
  assert.equal(group.paint_order, null);
  assert.equal(frame.page_id, PAGE);
  assert.equal(frame.parent_node_id, GROUP);
  assert.equal(frame.kind, "text_frame");
  assert.deepEqual(scene.stories, [
    { story_id: STORY, text: "Hello, canonical graph", text_fidelity: "partial" },
  ]);
  assert.deepEqual(scene.story_frames, [
    { story_id: STORY, frame_ordinal: 0, node_id: FRAME },
  ]);
  assert.deepEqual(scene.paints, []);
  assert.deepEqual(scene.resources, []);

  const plan = buildRenderPlan(scene, {
    emu_per_css_px: 9525,
    zoom: 1,
    pan_x_css_px: 0,
    pan_y_css_px: 0,
  });
  assert.equal(plan.pages.length, 1);
  assert.equal(plan.pages[0].stacking_authority, "snapshot_order");
  assert.deepEqual(
    new Set(plan.pages[0].nodes.map((node) => node.node_id)),
    new Set([GROUP, FRAME]),
  );
});

test("snapshot identity is deterministic and revision geometry sensitive", async () => {
  const left = await projectCurrentAuthoringGraphToScene(currentFixture());
  const right = await projectCurrentAuthoringGraphToScene(currentFixture());
  assert.equal(left.snapshot_id, right.snapshot_id);

  const moved = currentFixture();
  moved.authoring_graph.nodes[FRAME].header.bounds.x += 9_525;
  const changed = await projectCurrentAuthoringGraphToScene(moved);
  assert.notEqual(changed.snapshot_id, left.snapshot_id);
});

test("projection fails closed on orphaned or cyclic parent topology", async () => {
  const orphan = currentFixture();
  orphan.authoring_graph.nodes[FRAME].header.parent_id =
    "50000000-0000-4000-8000-000000000001";
  await assert.rejects(
    projectCurrentAuthoringGraphToScene(orphan),
    /parent does not resolve to a document page/,
  );

  const cyclic = currentFixture();
  cyclic.authoring_graph.nodes[GROUP].header.parent_id = FRAME;
  cyclic.authoring_graph.nodes[FRAME].header.parent_id = GROUP;
  await assert.rejects(
    projectCurrentAuthoringGraphToScene(cyclic),
    /node parent cycle/,
  );
});

test("projection refuses JavaScript-inexact EMU instead of rounding canonical geometry", async () => {
  const unsafe = currentFixture();
  unsafe.authoring_graph.nodes[FRAME].header.bounds.x = Number.MAX_SAFE_INTEGER + 1;
  await assert.rejects(
    projectCurrentAuthoringGraphToScene(unsafe),
    /JavaScript-safe integer/,
  );
});
