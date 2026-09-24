import test from "node:test";
import assert from "node:assert/strict";

import {
  FocusRouterV1,
  projectSceneAccessibilityV1,
} from "./accessibility-focus-v1.mjs";

const scene = {
  protocol_version: "chaptera.scene.v1",
  document_id: "doc:1",
  revision_id: "rev:1",
  pages: [
    {page_id: "page:b", order: 2},
    {page_id: "page:a", order: 1},
  ],
  nodes: [
    {node_id: "node:b1", page_id: "page:b", kind: "shape"},
    {node_id: "node:a1", page_id: "page:a", kind: "table"},
    {node_id: "node:a2", page_id: "page:a", kind: "shape"},
  ],
};

test("semantic projection uses canonical NodeId and page logical order", () => {
  const p = projectSceneAccessibilityV1(scene, {
    activeNodeId: "node:a2",
    selectedNodeIds: ["node:a2"],
  });
  assert.deepEqual(p.items.map((x) => x.node_id), ["node:a1", "node:a2", "node:b1"]);
  assert.equal(p.active_node_id, "node:a2");
  assert.equal(p.items.find((x) => x.node_id === "node:a2").selected, true);
  assert.deepEqual(p.items.filter((x) => x.tab_index === 0).map((x) => x.node_id), ["node:a2"]);
});

test("unknown active node falls back to first semantic item", () => {
  const p = projectSceneAccessibilityV1(scene, {activeNodeId: "node:missing"});
  assert.equal(p.active_node_id, "node:a1");
});

test("projection rejects duplicate canonical NodeId", () => {
  const bad = {
    ...scene,
    nodes: [...scene.nodes, {...scene.nodes[0]}],
  };
  assert.throws(() => projectSceneAccessibilityV1(bad), /duplicate canonical NodeId/);
});

test("scene owner routes document commands", () => {
  const router = new FocusRouterV1();
  router.setOwner("scene");
  assert.equal(router.route({key: "Delete"}).command, "delete_selection");
  assert.equal(router.route({key: "ArrowLeft"}).command, "nudge_left");
  assert.equal(router.route({key: "ArrowRight"}).command, "nudge_right");
  assert.equal(router.route({key: "ArrowDown"}).command, "focus_next_scene_item");
  assert.equal(router.route({key: "ArrowUp"}).command, "focus_previous_scene_item");
  assert.equal(router.route({key: "z", ctrlKey: true}).command, "document_undo");
});

test("non-scene owners fence document commands", () => {
  for (const owner of ["story_text", "inspector", "modal", "page_navigation", "none"]) {
    const router = new FocusRouterV1();
    router.setOwner(owner);
    assert.equal(router.route({key: "Delete"}), null, owner);
    assert.equal(router.route({key: "ArrowLeft"}), null, owner);
    assert.equal(router.route({key: "z", ctrlKey: true}), null, owner);
  }
});

test("explicit composition state fences global routing", () => {
  const router = new FocusRouterV1();
  router.setOwner("scene");
  router.setComposing(true);
  assert.equal(router.route({key: "Delete"}), null);
  router.setComposing(false);
  assert.equal(router.route({key: "Delete", eventIsComposing: true}), null);
});

test("Escape cancels transient scene operation only in scene context", () => {
  const router = new FocusRouterV1();
  router.setTransientOperation("drag");
  router.setOwner("scene");
  const routed = router.route({key: "Escape"});
  assert.equal(routed.command, "cancel_transient_operation");
  assert.equal(routed.transient_operation, "drag");

  router.setOwner("story_text");
  assert.equal(router.route({key: "Escape"}), null);
});

test("plain Escape is not a document command without transient operation", () => {
  const router = new FocusRouterV1();
  router.setOwner("scene");
  assert.equal(router.route({key: "Escape"}), null);
});

test("shifted undo is not silently treated as document undo", () => {
  const router = new FocusRouterV1();
  router.setOwner("scene");
  assert.equal(router.route({key: "z", ctrlKey: true, shiftKey: true}), null);
});
