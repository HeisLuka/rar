import { createRenderer, normalizeView } from "./render-v1.mjs";
import {
  MoveGestureV1,
  TransientSelectionV1,
  hitTestSnapshot,
  reconcileMoveCommit,
  screenToDocumentPoint,
} from "./interaction-v1.mjs";

const DEFAULT_VIEW = Object.freeze({
  emu_per_css_px: 9525,
  zoom: 1,
  pan_x_css_px: 0,
  pan_y_css_px: 0,
  page_gap_css_px: 32,
  page_margin_css_px: 24,
});

function clone(value) {
  return structuredClone(value);
}

export class BrowserEditorShellV1 {
  constructor({
    host,
    service,
    rendererKind = "svg",
    view = DEFAULT_VIEW,
    operationIdFactory = null,
    onState = null,
  }) {
    if (!host) throw new TypeError("host is required");
    if (!service || typeof service.commit !== "function" || typeof service.sceneForRevision !== "function") {
      throw new TypeError("service must implement commit() and sceneForRevision()");
    }
    this.host = host;
    this.service = service;
    this.rendererKind = rendererKind;
    this.view = normalizeView(view);
    this.operationIdFactory = operationIdFactory ?? ((index) => "browser-op-" + index);
    this.onState = onState;
    this.selection = new TransientSelectionV1();
    this.snapshot = null;
    this.renderer = null;
    this.gesture = null;
    this.gesturePageId = null;
    this.operationCounter = 0;
    this.commitRequests = 0;
    this.pointerMoveCount = 0;
    this.lastCommitResult = null;
    this._bound = false;
    this._handlers = null;
  }

  loadSnapshot(snapshot, { preserveSelection = true } = {}) {
    const previousSelection = preserveSelection ? this.selection.nodeId : null;
    if (this.renderer) this.renderer.destroy();
    this.snapshot = clone(snapshot);
    this.renderer = createRenderer(this.rendererKind, this.host, this.snapshot, this.view);
    this.gesture = null;
    this.gesturePageId = null;
    if (previousSelection && this.snapshot.nodes.some((node) => node.node_id === previousSelection)) {
      this.selection.select(previousSelection);
    } else {
      this.selection.clear();
    }
    this._renderOverlay();
    this._emitState("scene_loaded");
  }

  bindPointerEvents() {
    if (this._bound) return;
    const down = (event) => {
      if (event.button !== 0) return;
      const point = this._hostPoint(event);
      const state = this.pointerDownCss(point.x, point.y);
      if (state.kind === "selected") {
        try { this.host.setPointerCapture?.(event.pointerId); } catch {}
      }
    };
    const move = (event) => {
      if (!this.gesture) return;
      const point = this._hostPoint(event);
      this.pointerMoveCss(point.x, point.y);
    };
    const up = async (event) => {
      if (!this.gesture) return;
      const point = this._hostPoint(event);
      try {
        await this.pointerUpCss(point.x, point.y);
      } finally {
        try { this.host.releasePointerCapture?.(event.pointerId); } catch {}
      }
    };
    const cancel = () => this.cancelGesture();
    this.host.addEventListener("pointerdown", down);
    this.host.addEventListener("pointermove", move);
    this.host.addEventListener("pointerup", up);
    this.host.addEventListener("pointercancel", cancel);
    this._handlers = { down, move, up, cancel };
    this._bound = true;
  }

  unbindPointerEvents() {
    if (!this._bound) return;
    const { down, move, up, cancel } = this._handlers;
    this.host.removeEventListener("pointerdown", down);
    this.host.removeEventListener("pointermove", move);
    this.host.removeEventListener("pointerup", up);
    this.host.removeEventListener("pointercancel", cancel);
    this._handlers = null;
    this._bound = false;
  }

  pointerDownCss(xCss, yCss) {
    this._requireScene();
    if (this.gesture) throw new Error("gesture already active");
    const page = this._pageAt(xCss, yCss);
    if (!page) return { kind: "none" };
    const point = this._documentPoint(page, xCss, yCss);
    const hit = hitTestSnapshot(this.snapshot, page.page_id, point);
    if (hit.kind !== "hit") {
      if (hit.kind === "none") {
        this.selection.clear();
        this._renderOverlay();
      }
      this._emitState("hit_test_" + hit.kind);
      return hit;
    }
    this.selection.select(hit.node_id);
    this.gesture = new MoveGestureV1(this.snapshot, hit.node_id, point);
    this.gesturePageId = page.page_id;
    this._renderOverlay();
    this._emitState("gesture_started");
    return { kind: "selected", node_id: hit.node_id };
  }

  pointerMoveCss(xCss, yCss) {
    if (!this.gesture) return null;
    const page = this._pageById(this.gesturePageId);
    const point = this._documentPoint(page, xCss, yCss);
    const preview = this.gesture.update(point);
    this.pointerMoveCount += 1;
    this._renderOverlay();
    this._emitState("gesture_preview");
    return preview;
  }

  async pointerUpCss(xCss, yCss) {
    if (!this.gesture) throw new Error("no active gesture");
    this.pointerMoveCss(xCss, yCss);
    const gesture = this.gesture;
    const selectedNodeId = this.selection.nodeId;
    const clientOperationId = this.operationIdFactory(++this.operationCounter);
    const request = gesture.commit(clientOperationId);
    this.commitRequests += 1;
    this._emitState("commit_sent");

    const result = await this.service.commit(clone(request));
    this.lastCommitResult = clone(result);
    const reconciliation = reconcileMoveCommit(this.snapshot, gesture, result);

    if (reconciliation.kind === "accepted_waiting_for_scene") {
      const next = await this.service.sceneForRevision(result.revision_id);
      if (!next || next.protocol_version !== "chaptera.scene.v1") {
        throw new Error("service did not return BrowserSceneSnapshotV1");
      }
      if (next.document_id !== this.snapshot.document_id) throw new Error("scene document identity changed");
      if (next.source_hash !== this.snapshot.source_hash) throw new Error("scene source identity changed");
      if (next.revision_id !== result.revision_id) throw new Error("scene revision does not match accepted commit");
      this.gesture = null;
      this.gesturePageId = null;
      this.loadSnapshot(next, { preserveSelection: false });
      if (next.nodes.some((node) => node.node_id === selectedNodeId)) this.selection.select(selectedNodeId);
      this._renderOverlay();
      this._emitState("commit_reconciled");
      return { request, result, reconciliation, scene: clone(next) };
    }

    this.gesture = null;
    this.gesturePageId = null;
    this._renderOverlay();
    this._emitState("commit_rejected");
    return { request, result, reconciliation, scene: clone(this.snapshot) };
  }

  cancelGesture() {
    if (!this.gesture) return false;
    this.gesture.cancel();
    this.gesture = null;
    this.gesturePageId = null;
    this._renderOverlay();
    this._emitState("gesture_cancelled");
    return true;
  }

  nodeScreenBounds(nodeId) {
    this._requireScene();
    const node = this.renderer.plan.nodeById.get(nodeId);
    return node ? { x: node.x, y: node.y, width: node.width, height: node.height } : null;
  }

  stateReceipt() {
    return {
      receipt_class: "synthetic_browser_plumbing",
      real_pub: false,
      product_acceptance: false,
      document_id: this.snapshot?.document_id ?? null,
      revision_id: this.snapshot?.revision_id ?? null,
      selected_node_id: this.selection.nodeId,
      pointer_move_count: this.pointerMoveCount,
      commit_request_count: this.commitRequests,
      last_commit_protocol: this.lastCommitResult?.protocol_version ?? null,
      renderer: this.rendererKind,
      browser_scene_is_durable_authority: false,
    };
  }

  destroy() {
    this.unbindPointerEvents();
    this.renderer?.destroy();
    this.renderer = null;
    this.snapshot = null;
    this.gesture = null;
  }

  _pageAt(xCss, yCss) {
    return this.renderer.plan.pages.find((page) =>
      xCss >= page.x && xCss < page.x + page.width &&
      yCss >= page.y && yCss < page.y + page.height) ?? null;
  }

  _pageById(pageId) {
    const page = this.renderer.plan.pages.find((item) => item.page_id === pageId);
    if (!page) throw new Error("gesture page is missing");
    return page;
  }

  _documentPoint(page, xCss, yCss) {
    return screenToDocumentPoint(
      { x_css_px: xCss, y_css_px: yCss },
      {
        screen_origin_x_css_px: page.x,
        screen_origin_y_css_px: page.y,
        document_origin_x_emu: 0,
        document_origin_y_emu: 0,
        emu_per_css_px: this.view.emu_per_css_px / this.view.zoom,
      },
    );
  }

  _hostPoint(event) {
    const rect = this.host.getBoundingClientRect();
    return { x: event.clientX - rect.left, y: event.clientY - rect.top };
  }

  _renderOverlay() {
    if (!this.renderer) return;
    this.renderer.updateOverlay({
      selected_node_id: this.selection.nodeId,
      preview_bounds: this.gesture?.previewBounds() ?? null,
    });
  }

  _emitState(reason) {
    this.onState?.({ reason, ...this.stateReceipt() });
  }

  _requireScene() {
    if (!this.snapshot || !this.renderer) throw new Error("scene is not loaded");
  }
}
