import { normalizeView } from "./render-v1.mjs";

const SAVE_SURFACE_STATES = new Set([
  "first_useful_canvas",
  "checking_latest",
  "preparing_editor",
  "edit_ready",
  "saved",
  "saving",
  "checking_save_status",
  "not_synced",
  "recovery_unavailable",
  "needs_attention",
  "read_only",
]);

const GLOBAL_DOCUMENT_FOCUS = new Set(["scene", "page_navigator"]);

function clone(value) {
  return value == null ? value : structuredClone(value);
}

function finite(value, label) {
  if (!Number.isFinite(value)) throw new TypeError(label + " must be finite");
  return value;
}

function positive(value, label) {
  const v = finite(value, label);
  if (!(v > 0)) throw new RangeError(label + " must be > 0");
  return v;
}

function clamp(value, min, max) {
  return Math.max(min, Math.min(max, value));
}

function isTextualOwner(owner) {
  return owner === "story" || owner === "inspector" || owner === "modal";
}

export function normalizeAuthoritativeSaveSurfaceV1(state) {
  if (!state || typeof state !== "object") throw new TypeError("authoritative editor state required");
  const productState = state.product_state;
  if (!SAVE_SURFACE_STATES.has(productState)) {
    throw new TypeError("unsupported authoritative product_state");
  }
  return Object.freeze({
    product_state: productState,
    label: state.label ?? productState,
    can_edit: state.can_edit === true,
    durable: state.durable === true,
    local_recovery: state.local_recovery ?? null,
    attention_code: state.attention_code ?? null,
  });
}

export function capabilityContextMenuV1({ focusOwner, composing = false, selection = null, capabilities = {} }) {
  if (composing || focusOwner !== "scene" || !selection?.node_id) return [];
  const out = [];
  if (capabilities.can_cut === true) out.push("cut");
  if (capabilities.can_copy === true) out.push("copy");
  if (capabilities.can_duplicate === true) out.push("duplicate");
  if (capabilities.can_delete === true) out.push("delete");
  if (capabilities.can_replace_image === true) out.push("replace_image");
  if (capabilities.can_edit_text === true) out.push("edit_text");
  return out;
}

export class WebEditorComfortV1 {
  constructor({
    view = {},
    stateProvider,
    commands,
    onView = null,
    minZoom = 0.1,
    maxZoom = 8,
  }) {
    if (!stateProvider || typeof stateProvider.currentProductState !== "function") {
      throw new TypeError("stateProvider.currentProductState() is required");
    }
    const required = [
      "undo", "redo", "flushSaveFrontier", "cancelTransient", "deleteSelection", "showShortcutHelp"
    ];
    for (const name of required) {
      if (!commands || typeof commands[name] !== "function") {
        throw new TypeError("commands." + name + "() is required");
      }
    }
    this.commands = commands;
    this.stateProvider = stateProvider;
    this.onView = onView;
    this.minZoom = positive(minZoom, "minZoom");
    this.maxZoom = positive(maxZoom, "maxZoom");
    if (this.maxZoom < this.minZoom) throw new RangeError("maxZoom must be >= minZoom");
    this.view = normalizeView(view);
    this.spaceHeld = false;
  }

  saveSurface() {
    return normalizeAuthoritativeSaveSurfaceV1(this.stateProvider.currentProductState());
  }

  viewState() {
    return clone(this.view);
  }

  zoomPercent() {
    return Math.round(this.view.zoom * 100);
  }

  panBy(dxCssPx, dyCssPx) {
    const next = normalizeView({
      ...this.view,
      pan_x_css_px: this.view.pan_x_css_px + finite(dxCssPx, "dxCssPx"),
      pan_y_css_px: this.view.pan_y_css_px + finite(dyCssPx, "dyCssPx"),
    });
    return this._setView(next, "pan");
  }

  zoomByFactor(factor) {
    const nextZoom = clamp(this.view.zoom * positive(factor, "zoom factor"), this.minZoom, this.maxZoom);
    return this._setView(normalizeView({ ...this.view, zoom: nextZoom }), "zoom");
  }

  zoomIn() { return this.zoomByFactor(1.1); }
  zoomOut() { return this.zoomByFactor(1 / 1.1); }
  zoom100() { return this._setView(normalizeView({ ...this.view, zoom: 1 }), "zoom_100"); }

  fitPage({ viewport_width_css_px, viewport_height_css_px, page_width_emu, page_height_emu, padding_css_px = 24 }) {
    const vw = positive(viewport_width_css_px, "viewport width");
    const vh = positive(viewport_height_css_px, "viewport height");
    const pw = positive(page_width_emu, "page width");
    const ph = positive(page_height_emu, "page height");
    const pad = Math.max(0, finite(padding_css_px, "padding"));
    const baseW = pw / this.view.emu_per_css_px;
    const baseH = ph / this.view.emu_per_css_px;
    const zoom = clamp(Math.min((vw - pad * 2) / baseW, (vh - pad * 2) / baseH), this.minZoom, this.maxZoom);
    return this._setView(normalizeView({
      ...this.view,
      zoom,
      pan_x_css_px: 0,
      pan_y_css_px: 0,
    }), "fit_page");
  }

  fitSelection({ viewport_width_css_px, viewport_height_css_px, bounds, padding_css_px = 32 }) {
    if (!bounds) throw new TypeError("selection bounds required");
    const vw = positive(viewport_width_css_px, "viewport width");
    const vh = positive(viewport_height_css_px, "viewport height");
    const bw = positive(bounds.width, "selection width");
    const bh = positive(bounds.height, "selection height");
    const pad = Math.max(0, finite(padding_css_px, "padding"));
    const baseW = bw / this.view.emu_per_css_px;
    const baseH = bh / this.view.emu_per_css_px;
    const zoom = clamp(Math.min((vw - pad * 2) / baseW, (vh - pad * 2) / baseH), this.minZoom, this.maxZoom);
    const selectionX = finite(bounds.x, "selection x") / this.view.emu_per_css_px * zoom;
    const selectionY = finite(bounds.y, "selection y") / this.view.emu_per_css_px * zoom;
    const selectionW = baseW * zoom;
    const selectionH = baseH * zoom;
    const panX = (vw - selectionW) / 2 - this.view.page_margin_css_px - selectionX;
    const panY = (vh - selectionH) / 2 - this.view.page_margin_css_px - selectionY;
    return this._setView(normalizeView({
      ...this.view,
      zoom,
      pan_x_css_px: panX,
      pan_y_css_px: panY,
    }), "fit_selection");
  }

  handleWheel(event, context = {}) {
    if (context.composing || isTextualOwner(context.focusOwner)) return { handled: false };
    if ((event.ctrlKey || event.metaKey) && Number.isFinite(event.deltaY)) {
      event.preventDefault?.();
      this.zoomByFactor(event.deltaY < 0 ? 1.1 : 1 / 1.1);
      return { handled: true, command: "zoom" };
    }
    if (Number.isFinite(event.deltaX) || Number.isFinite(event.deltaY)) {
      this.panBy(-(event.deltaX || 0), -(event.deltaY || 0));
      return { handled: true, command: "pan" };
    }
    return { handled: false };
  }

  handleKeyDown(event, context = {}) {
    const key = String(event.key ?? "");
    const lower = key.toLowerCase();
    const primary = event.ctrlKey || event.metaKey;
    const composing = context.composing === true || event.isComposing === true;
    const focusOwner = context.focusOwner ?? "scene";
    if (key === " " && !composing && focusOwner === "scene") {
      this.spaceHeld = true;
      event.preventDefault?.();
      return { handled: true, command: "pan_mode" };
    }
    if (composing) return { handled: false };

    if (primary && lower === "s" && !event.altKey) {
      event.preventDefault?.();
      this.commands.flushSaveFrontier();
      return { handled: true, command: "save_frontier" };
    }
    if (isTextualOwner(focusOwner)) return { handled: false };
    if (!GLOBAL_DOCUMENT_FOCUS.has(focusOwner) && focusOwner !== "transient") return { handled: false };

    if (primary && lower === "z" && !event.altKey) {
      event.preventDefault?.();
      if (event.shiftKey) this.commands.redo();
      else this.commands.undo();
      return { handled: true, command: event.shiftKey ? "redo" : "undo" };
    }
    if (primary && !event.altKey && (key === "+" || key === "=")) {
      event.preventDefault?.();
      this.zoomIn();
      return { handled: true, command: "zoom_in" };
    }
    if (primary && !event.altKey && key === "-") {
      event.preventDefault?.();
      this.zoomOut();
      return { handled: true, command: "zoom_out" };
    }
    if (primary && !event.altKey && key === "0") {
      event.preventDefault?.();
      this.zoom100();
      return { handled: true, command: "zoom_100" };
    }
    if (key === "Escape" && focusOwner === "transient") {
      event.preventDefault?.();
      this.commands.cancelTransient();
      return { handled: true, command: "cancel_transient" };
    }
    if ((key === "Delete" || key === "Backspace") && focusOwner === "scene" && context.capabilities?.can_delete === true && context.selection?.node_id) {
      event.preventDefault?.();
      this.commands.deleteSelection(context.selection.node_id);
      return { handled: true, command: "delete" };
    }
    if (key === "?" && !primary && !event.altKey) {
      event.preventDefault?.();
      this.commands.showShortcutHelp();
      return { handled: true, command: "help" };
    }
    return { handled: false };
  }

  handleKeyUp(event) {
    if (String(event.key ?? "") === " ") {
      this.spaceHeld = false;
      return { handled: true, command: "pan_mode_end" };
    }
    return { handled: false };
  }

  contextMenu(context = {}) {
    return capabilityContextMenuV1(context);
  }

  localUiState() {
    return Object.freeze({
      schema_version: "chaptera.web-local-ui.v1",
      view: clone(this.view),
    });
  }

  _setView(view, reason) {
    this.view = view;
    this.onView?.({ reason, view: this.viewState(), zoom_percent: this.zoomPercent() });
    return this.viewState();
  }
}
