import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  RENDERER_KINDS,
  RENDERER_POLICY_V1,
  assertSceneSourceNeutral,
  buildOverlayPlan,
  buildRenderPlan
} from "./render-v1.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const FIXTURES = path.resolve(HERE, "../../packages/protocol/scene/v1/fixtures");
const VIEW = {
  emu_per_css_px: 9525,
  zoom: 1,
  pan_x_css_px: 0,
  pan_y_css_px: 0
};

function fixture(name) {
  return JSON.parse(fs.readFileSync(path.join(FIXTURES, name), "utf8"));
}

function deepFreeze(value) {
  if (value && typeof value === "object" && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const child of Object.values(value)) deepFreeze(child);
  }
  return value;
}

test("preflight exposes all required renderer candidates", () => {
  assert.deepEqual(RENDERER_KINDS, ["svg", "canvas2d", "webgl2-hybrid"]);
});

test("V1 renderer policy keeps SVG primary without an invented auto-switch threshold", () => {
  assert.deepEqual(RENDERER_POLICY_V1, {
    primary: "svg",
    retained_scale_lane: "canvas2d",
    optional_accelerators: ["webgl2-hybrid"],
    automatic_switch_threshold: null
  });
});

test("render plan is deterministic and does not mutate a frozen scene", () => {
  const scene = deepFreeze(fixture("group-table.json"));
  const left = buildRenderPlan(scene, VIEW);
  const right = buildRenderPlan(scene, VIEW);
  assert.deepEqual(
    left.pages.map((page) => page.nodes.map((node) => node.node_id)),
    right.pages.map((page) => page.nodes.map((node) => node.node_id))
  );
  assert.equal(left.pages[0].stacking_authority, "exact");
  assert.equal(left.pages[0].nodes.length, 3);
});

test("exact stacking is back-to-front by z then paint order", () => {
  const plan = buildRenderPlan(fixture("group-table.json"), VIEW);
  assert.deepEqual(
    plan.pages[0].nodes.map((node) => node.node_id),
    [
      "23333333-3333-4333-8333-333333333330",
      "23333333-3333-4333-8333-333333333331",
      "23333333-3333-4333-8333-333333333332"
    ]
  );
});

test("negative off-page geometry survives the render plan", () => {
  const plan = buildRenderPlan(fixture("partial-unsupported.json"), VIEW);
  const node = plan.pages[0].nodes[0];
  assert.equal(node.canonical_bounds.x, -12700);
  assert.ok(node.x < plan.pages[0].x);
  assert.equal(node.diagnostics[0].code, "SCENE.NODE.UNSUPPORTED");
  assert.equal(plan.fidelity.state, "partial");
});

test("zoom and pan change display coordinates without mutating scene coordinates", () => {
  const scene = fixture("exact-image.json");
  const before = structuredClone(scene);
  const base = buildRenderPlan(scene, VIEW);
  const moved = buildRenderPlan(scene, {
    ...VIEW,
    zoom: 2,
    pan_x_css_px: 80,
    pan_y_css_px: -40
  });
  assert.notEqual(base.pages[0].nodes[0].x, moved.pages[0].nodes[0].x);
  assert.equal(
    base.pages[0].nodes[0].canonical_bounds.x,
    moved.pages[0].nodes[0].canonical_bounds.x
  );
  assert.deepEqual(scene, before);
});

test("text is explicitly browser-preview-only", () => {
  const plan = buildRenderPlan(fixture("simple-text.json"), VIEW);
  const node = plan.pages[0].nodes[0];
  assert.equal(node.story.text, "Hello, Publisher");
  assert.equal(node.story.authority, "browser_preview_only");
  assert.equal(node.story.text_fidelity, "partial");
});

test("resource availability is carried without source bytes", () => {
  const plan = buildRenderPlan(fixture("exact-image.json"), VIEW);
  const resource = plan.pages[0].nodes[0].resource;
  assert.equal(resource.kind, "image");
  assert.equal(resource.availability, "available");
  assert.equal(resource.fetch_handle, "res_img_exact_001");
  assert.ok(!("bytes" in resource));
});

test("selection and transient preview are independent overlays", () => {
  const scene = fixture("group-table.json");
  const plan = buildRenderPlan(scene, VIEW);
  const selected = plan.pages[0].nodes[1];
  const baseBounds = structuredClone(scene.nodes[1].bounds);
  const overlay = buildOverlayPlan(plan, {
    selected_node_id: selected.node_id,
    preview_bounds: {
      x: baseBounds.x + 100000,
      y: baseBounds.y + 200000,
      width: baseBounds.width,
      height: baseBounds.height
    }
  });
  assert.equal(overlay.selected_node_id, selected.node_id);
  assert.notEqual(overlay.selected_bounds.x, selected.x);
  assert.deepEqual(scene.nodes[1].bounds, baseBounds);
});

test("renderer rejects raw/private source carrier fields recursively", () => {
  const scene = fixture("simple-text.json");
  assert.doesNotThrow(() => assertSceneSourceNeutral(scene));
  const bad = structuredClone(scene);
  bad.nodes[0].parser_record = { stream_path: "Contents/7" };
  assert.throws(() => buildRenderPlan(bad, VIEW), /forbidden source field/);
});
