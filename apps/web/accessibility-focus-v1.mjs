export const FOCUS_OWNERS_V1 = Object.freeze([
  "page_navigation",
  "scene",
  "story_text",
  "inspector",
  "modal",
  "none",
]);

function requireScene(snapshot) {
  if (!snapshot || snapshot.protocol_version !== "chaptera.scene.v1") {
    throw new TypeError("BrowserSceneSnapshotV1 required");
  }
  if (!Array.isArray(snapshot.pages) || !Array.isArray(snapshot.nodes)) {
    throw new TypeError("scene pages/nodes required");
  }
}

function requireNodeId(value, label = "node_id") {
  if (typeof value !== "string" || value.length === 0 || value.length > 256) {
    throw new TypeError(label + " must be a bounded string");
  }
}

function semanticLabel(node) {
  const explicit =
    node.accessibility_name ??
    node.semantic_name ??
    node.name ??
    null;
  if (typeof explicit === "string" && explicit.trim()) return explicit.trim();
  return String(node.kind ?? "Object") + " " + node.node_id;
}

export function projectSceneAccessibilityV1(
  snapshot,
  {
    activeNodeId = null,
    selectedNodeIds = [],
  } = {},
) {
  requireScene(snapshot);
  const pageOrder = new Map(
    [...snapshot.pages]
      .sort((a, b) => a.order - b.order)
      .map((page, index) => [page.page_id, index]),
  );
  const sourceIndex = new Map(snapshot.nodes.map((node, index) => [node.node_id, index]));
  const seen = new Set();
  const nodes = snapshot.nodes.map((node) => {
    requireNodeId(node.node_id);
    if (seen.has(node.node_id)) throw new Error("duplicate canonical NodeId");
    seen.add(node.node_id);
    return node;
  });
  nodes.sort((a, b) => {
    const ap = pageOrder.get(a.page_id) ?? Number.MAX_SAFE_INTEGER;
    const bp = pageOrder.get(b.page_id) ?? Number.MAX_SAFE_INTEGER;
    if (ap !== bp) return ap - bp;
    return sourceIndex.get(a.node_id) - sourceIndex.get(b.node_id);
  });

  const selected = new Set(selectedNodeIds);
  const admittedActive =
    activeNodeId && seen.has(activeNodeId)
      ? activeNodeId
      : (nodes[0]?.node_id ?? null);

  return Object.freeze({
    protocol_version: "chaptera.accessibility-focus-projection.v1",
    document_id: snapshot.document_id,
    revision_id: snapshot.revision_id,
    active_node_id: admittedActive,
    items: Object.freeze(nodes.map((node, index) => Object.freeze({
      node_id: node.node_id,
      page_id: node.page_id,
      role: "treeitem",
      name: semanticLabel(node),
      logical_index: index,
      selected: selected.has(node.node_id),
      tab_index: node.node_id === admittedActive ? 0 : -1,
    }))),
  });
}

function styleMirrorHost(host) {
  host.setAttribute("role", "tree");
  host.setAttribute("aria-label", "Publication objects");
  host.dataset.semanticAuthority = "canonical-node-id";
  host.style.position = "fixed";
  host.style.left = "-10000px";
  host.style.top = "0";
  host.style.width = "1px";
  host.style.height = "1px";
  host.style.overflow = "hidden";
}

export class SemanticAccessibilityControllerV1 {
  constructor({
    host = null,
    snapshot,
    activeNodeId = null,
    selectedNodeIds = [],
  }) {
    requireScene(snapshot);
    this.host = host;
    this.snapshot = snapshot;
    this.activeNodeId = activeNodeId;
    this.selectedNodeIds = new Set(selectedNodeIds);
    this.projection = null;
    this.render();
  }

  render({restoreDomFocus = false} = {}) {
    const focusedBefore =
      this.host &&
      this.host.contains(globalThis.document?.activeElement)
        ? globalThis.document.activeElement?.dataset?.nodeId ?? null
        : null;
    this.projection = projectSceneAccessibilityV1(this.snapshot, {
      activeNodeId: this.activeNodeId,
      selectedNodeIds: [...this.selectedNodeIds],
    });
    this.activeNodeId = this.projection.active_node_id;

    if (this.host) {
      styleMirrorHost(this.host);
      const fragment = document.createDocumentFragment();
      for (const item of this.projection.items) {
        const button = document.createElement("button");
        button.type = "button";
        button.setAttribute("role", item.role);
        button.setAttribute("aria-label", item.name);
        button.setAttribute("aria-selected", item.selected ? "true" : "false");
        button.dataset.nodeId = item.node_id;
        button.dataset.pageId = item.page_id ?? "";
        button.dataset.focusOwner = "scene";
        button.tabIndex = item.tab_index;
        fragment.appendChild(button);
      }
      this.host.replaceChildren(fragment);
      if (restoreDomFocus && focusedBefore && focusedBefore === this.activeNodeId) {
        this.focusSceneNode(this.activeNodeId);
      }
    }
    return this.snapshotState();
  }

  reproject(snapshot = this.snapshot) {
    requireScene(snapshot);
    const focusedInside =
      !!this.host &&
      this.host.contains(globalThis.document?.activeElement);
    this.snapshot = snapshot;
    return this.render({restoreDomFocus: focusedInside});
  }

  setSelectedNodeIds(nodeIds) {
    this.selectedNodeIds = new Set(nodeIds);
    return this.render({
      restoreDomFocus:
        !!this.host && this.host.contains(globalThis.document?.activeElement),
    });
  }

  focusSceneNode(nodeId, {domFocus = true} = {}) {
    requireNodeId(nodeId);
    if (!this.projection.items.some((item) => item.node_id === nodeId)) {
      throw new RangeError("unknown semantic NodeId");
    }
    this.activeNodeId = nodeId;
    this.render();
    if (domFocus && this.host) {
      const target = [...this.host.querySelectorAll('[role="treeitem"]')]
        .find((item) => item.dataset.nodeId === nodeId);
      target?.focus();
    }
    return this.snapshotState();
  }

  moveSceneFocus(delta) {
    if (!Number.isInteger(delta) || delta === 0) {
      throw new TypeError("focus delta must be a non-zero integer");
    }
    const ids = this.projection.items.map((item) => item.node_id);
    if (!ids.length) return this.snapshotState();
    const current = Math.max(0, ids.indexOf(this.activeNodeId));
    const next = (current + delta + ids.length) % ids.length;
    return this.focusSceneNode(ids[next]);
  }

  snapshotState() {
    const items = this.projection?.items ?? [];
    return {
      protocol_version: this.projection?.protocol_version ?? null,
      document_id: this.projection?.document_id ?? null,
      revision_id: this.projection?.revision_id ?? null,
      active_node_id: this.activeNodeId ?? null,
      selected_node_ids: items.filter((item) => item.selected).map((item) => item.node_id),
      logical_order: items.map((item) => item.node_id),
      tab_stop_node_ids: items
        .filter((item) => item.tab_index === 0)
        .map((item) => item.node_id),
      names: Object.fromEntries(items.map((item) => [item.node_id, item.name])),
    };
  }
}

export class FocusRouterV1 {
  constructor() {
    this.owner = "none";
    this.composing = false;
    this.transientOperation = null;
  }

  setOwner(owner) {
    if (!FOCUS_OWNERS_V1.includes(owner)) {
      throw new TypeError("unsupported focus owner");
    }
    this.owner = owner;
  }

  setComposing(value) {
    this.composing = !!value;
  }

  setTransientOperation(value) {
    if (value != null && (typeof value !== "string" || !value)) {
      throw new TypeError("transient operation must be string or null");
    }
    this.transientOperation = value;
  }

  route({
    key,
    ctrlKey = false,
    metaKey = false,
    shiftKey = false,
    eventIsComposing = false,
  }) {
    if (typeof key !== "string") throw new TypeError("key required");
    if (this.composing || eventIsComposing) return null;
    if (this.owner !== "scene") return null;

    if (key === "Delete" || key === "Backspace") {
      return Object.freeze({command: "delete_selection", prevent_default: true});
    }
    if (key === "ArrowLeft") {
      return Object.freeze({command: "nudge_left", prevent_default: true});
    }
    if (key === "ArrowRight") {
      return Object.freeze({command: "nudge_right", prevent_default: true});
    }
    if (key === "ArrowDown") {
      return Object.freeze({command: "focus_next_scene_item", prevent_default: true});
    }
    if (key === "ArrowUp") {
      return Object.freeze({command: "focus_previous_scene_item", prevent_default: true});
    }
    if ((ctrlKey || metaKey) && !shiftKey && key.toLowerCase() === "z") {
      return Object.freeze({command: "document_undo", prevent_default: true});
    }
    if (key === "Escape" && this.transientOperation) {
      return Object.freeze({
        command: "cancel_transient_operation",
        transient_operation: this.transientOperation,
        prevent_default: true,
      });
    }
    return null;
  }
}

export function inferFocusOwnerV1(target) {
  if (!target || typeof target.closest !== "function") return "none";
  if (target.closest("dialog[open]")) return "modal";
  if (target.closest('[data-focus-owner="story"],[contenteditable="true"],[role="textbox"]')) {
    return "story_text";
  }
  if (
    target.closest(
      '[data-focus-owner="inspector"],input:not([data-focus-owner="page_navigation"]),textarea,select',
    )
  ) {
    return "inspector";
  }
  if (target.closest('[role="treeitem"][data-node-id]')) return "scene";
  if (target.closest('[data-focus-owner="page_navigation"]')) return "page_navigation";
  return "none";
}

export function attachBrowserFocusRoutingV1({
  root = document,
  semanticController,
  commandSink = () => {},
} = {}) {
  if (!(semanticController instanceof SemanticAccessibilityControllerV1)) {
    throw new TypeError("semanticController required");
  }
  const router = new FocusRouterV1();
  const commands = [];

  function record(result, event, owner) {
    const entry = Object.freeze({
      command: result.command,
      owner,
      key: event.key,
      node_id: event.target?.dataset?.nodeId ?? null,
      transient_operation: result.transient_operation ?? null,
    });
    commands.push(entry);
    commandSink(entry);
  }

  function onCompositionStart() {
    router.setComposing(true);
  }
  function onCompositionEnd() {
    router.setComposing(false);
  }
  function onKeyDown(event) {
    const openDialog = root.querySelector?.("dialog[open]") ?? null;
    if (openDialog && !openDialog.contains(event.target)) return;
    const owner = inferFocusOwnerV1(event.target);
    router.setOwner(owner);
    const result = router.route({
      key: event.key,
      ctrlKey: !!event.ctrlKey,
      metaKey: !!event.metaKey,
      shiftKey: !!event.shiftKey,
      eventIsComposing: !!event.isComposing,
    });
    if (!result) return;
    if (result.prevent_default) event.preventDefault();

    if (result.command === "focus_next_scene_item") {
      semanticController.moveSceneFocus(1);
    } else if (result.command === "focus_previous_scene_item") {
      semanticController.moveSceneFocus(-1);
    } else if (result.command === "cancel_transient_operation") {
      router.setTransientOperation(null);
    }
    record(result, event, owner);
  }

  root.addEventListener("compositionstart", onCompositionStart, true);
  root.addEventListener("compositionend", onCompositionEnd, true);
  root.addEventListener("keydown", onKeyDown, true);

  return {
    commands,
    router,
    setTransientOperation(value) {
      router.setTransientOperation(value);
    },
    isComposing() {
      return router.composing;
    },
    destroy() {
      root.removeEventListener("compositionstart", onCompositionStart, true);
      root.removeEventListener("compositionend", onCompositionEnd, true);
      root.removeEventListener("keydown", onKeyDown, true);
    },
  };
}
