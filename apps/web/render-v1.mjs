const FORBIDDEN_SOURCE_KEYS = new Set([
  "raw_pub_bytes", "raw_bytes", "source_path", "filesystem_path",
  "cfb_path", "stream_path", "stream_name", "parser_record",
  "carrier", "source_ref", "byte_range"
]);

export const RENDERER_KINDS = Object.freeze(["svg", "canvas2d", "webgl2-hybrid"]);

export const RENDERER_POLICY_V1 = Object.freeze({
  primary: "svg",
  retained_scale_lane: "canvas2d",
  optional_accelerators: Object.freeze(["webgl2-hybrid"]),
  automatic_switch_threshold: null
});

function finite(value, label) {
  if (!Number.isFinite(value)) throw new TypeError(label + " must be finite");
  return value;
}

function safeInteger(value, label) {
  if (!Number.isSafeInteger(value)) {
    throw new RangeError(label + " must be a JavaScript-safe integer");
  }
  return value;
}

export function assertSceneSourceNeutral(value, at = "$") {
  if (Array.isArray(value)) {
    value.forEach((child, index) => assertSceneSourceNeutral(child, at + "[" + index + "]"));
    return;
  }
  if (value && typeof value === "object") {
    for (const [key, child] of Object.entries(value)) {
      if (FORBIDDEN_SOURCE_KEYS.has(key)) {
        throw new Error("renderer input contains forbidden source field " + key + " at " + at);
      }
      assertSceneSourceNeutral(child, at + "." + key);
    }
  }
}

export function normalizeView(view = {}) {
  const emuPerCssPx = finite(view.emu_per_css_px ?? 9525, "emu_per_css_px");
  const zoom = finite(view.zoom ?? 1, "zoom");
  if (!(emuPerCssPx > 0)) throw new RangeError("emu_per_css_px must be > 0");
  if (!(zoom > 0)) throw new RangeError("zoom must be > 0");
  return Object.freeze({
    emu_per_css_px: emuPerCssPx,
    zoom,
    pan_x_css_px: finite(view.pan_x_css_px ?? 0, "pan_x_css_px"),
    pan_y_css_px: finite(view.pan_y_css_px ?? 0, "pan_y_css_px"),
    page_gap_css_px: finite(view.page_gap_css_px ?? 32, "page_gap_css_px"),
    page_margin_css_px: finite(view.page_margin_css_px ?? 24, "page_margin_css_px")
  });
}

function emuToCss(value, view) {
  return (safeInteger(value, "EMU") / view.emu_per_css_px) * view.zoom;
}

function compareExactStacking(a, b) {
  if (a.z_order !== b.z_order) return a.z_order < b.z_order ? -1 : 1;
  if (a.paint_order !== b.paint_order) return a.paint_order < b.paint_order ? -1 : 1;
  return a.node_id.localeCompare(b.node_id);
}

function rgbaCss(color) {
  if (!color) return null;
  return "rgba(" + color.r + "," + color.g + "," + color.b + "," + (color.a / 255) + ")";
}

export function buildRenderPlan(snapshot, rawView = {}) {
  assertSceneSourceNeutral(snapshot);
  if (snapshot?.protocol_version !== "chaptera.scene.v1") {
    throw new Error("renderer requires BrowserSceneSnapshotV1");
  }
  const view = normalizeView(rawView);
  const pages = [...snapshot.pages].sort((a, b) => a.order - b.order);
  const paints = new Map(snapshot.paints.map((paint) => [paint.paint_id, paint]));
  const stories = new Map(snapshot.stories.map((story) => [story.story_id, story]));
  const storyByNode = new Map(
    snapshot.story_frames.map((frame) => [frame.node_id, stories.get(frame.story_id) ?? null])
  );
  const resources = new Map(snapshot.resources.map((resource) => [resource.resource_id, resource]));
  const diagnosticsByNode = new Map();
  for (const diagnostic of snapshot.diagnostics) {
    if (!diagnostic.origin_node_id) continue;
    const bucket = diagnosticsByNode.get(diagnostic.origin_node_id) ?? [];
    bucket.push(diagnostic);
    diagnosticsByNode.set(diagnostic.origin_node_id, bucket);
  }

  let yCursor = view.page_margin_css_px + view.pan_y_css_px;
  const pagePlans = [];
  const nodeById = new Map();

  for (const page of pages) {
    const pageX = view.page_margin_css_px + view.pan_x_css_px;
    const pageY = yCursor;
    const pageWidth = emuToCss(page.width_emu, view);
    const pageHeight = emuToCss(page.height_emu, view);
    let nodes = snapshot.nodes.filter((node) => node.page_id === page.page_id);
    let stackingAuthority = "snapshot_order";
    if (
      snapshot.stacking_fidelity === "exact" &&
      nodes.every((node) => Number.isInteger(node.z_order) && Number.isInteger(node.paint_order))
    ) {
      nodes = [...nodes].sort(compareExactStacking);
      stackingAuthority = "exact";
    }

    const nodePlans = nodes.map((node) => {
      Object.entries(node.bounds).forEach(([key, value]) => safeInteger(value, "node.bounds." + key));
      const paint = node.paint_id ? paints.get(node.paint_id) ?? null : null;
      const story = storyByNode.get(node.node_id) ?? null;
      const resource = node.resource_id ? resources.get(node.resource_id) ?? null : null;
      const plan = Object.freeze({
        node_id: node.node_id,
        page_id: node.page_id,
        kind: node.kind,
        x: pageX + emuToCss(node.bounds.x, view),
        y: pageY + emuToCss(node.bounds.y, view),
        width: emuToCss(node.bounds.width, view),
        height: emuToCss(node.bounds.height, view),
        paint: Object.freeze({
          fill: rgbaCss(paint?.fill ?? null),
          stroke: rgbaCss(paint?.stroke?.color ?? null),
          stroke_width_css_px: emuToCss(paint?.stroke?.width_emu ?? 0, view)
        }),
        story: story ? Object.freeze({
          text: story.text,
          text_fidelity: story.text_fidelity,
          authority: "browser_preview_only"
        }) : null,
        resource: resource ? Object.freeze({
          resource_id: resource.resource_id,
          kind: resource.kind,
          availability: resource.availability,
          mime: resource.mime,
          content_hash: resource.content_hash,
          fetch_handle: resource.fetch_handle
        }) : null,
        diagnostics: Object.freeze([...(diagnosticsByNode.get(node.node_id) ?? [])]),
        canonical_bounds: Object.freeze({ ...node.bounds })
      });
      nodeById.set(node.node_id, plan);
      return plan;
    });

    pagePlans.push(Object.freeze({
      page_id: page.page_id,
      order: page.order,
      x: pageX,
      y: pageY,
      width: pageWidth,
      height: pageHeight,
      stacking_authority: stackingAuthority,
      nodes: Object.freeze(nodePlans)
    }));
    yCursor += pageHeight + view.page_gap_css_px;
  }

  return Object.freeze({
    protocol_version: "chaptera.render-plan.v1",
    document_id: snapshot.document_id,
    revision_id: snapshot.revision_id,
    snapshot_id: snapshot.snapshot_id,
    view,
    pages: Object.freeze(pagePlans),
    nodeById,
    fidelity: snapshot.fidelity,
    diagnostics: Object.freeze([...snapshot.diagnostics])
  });
}

export function buildOverlayPlan(renderPlan, overlay = {}) {
  const selectedNodeId = overlay.selected_node_id ?? null;
  const selected = selectedNodeId ? renderPlan.nodeById.get(selectedNodeId) ?? null : null;
  let selectedBounds = selected
    ? { x: selected.x, y: selected.y, width: selected.width, height: selected.height }
    : null;

  if (overlay.preview_bounds && selected) {
    const page = renderPlan.pages.find((item) => item.page_id === selected.page_id);
    if (!page) throw new Error("selected node page is missing");
    const b = overlay.preview_bounds;
    Object.entries(b).forEach(([key, value]) => safeInteger(value, "preview_bounds." + key));
    selectedBounds = {
      x: page.x + emuToCss(b.x, renderPlan.view),
      y: page.y + emuToCss(b.y, renderPlan.view),
      width: emuToCss(b.width, renderPlan.view),
      height: emuToCss(b.height, renderPlan.view)
    };
  }

  return Object.freeze({
    selected_node_id: selectedNodeId,
    selected_bounds: selectedBounds ? Object.freeze(selectedBounds) : null
  });
}

function clearHost(host) {
  while (host.firstChild) host.removeChild(host.firstChild);
}

function hostSize(host) {
  return {
    width: Math.max(1, Math.floor(host.clientWidth || 1200)),
    height: Math.max(1, Math.floor(host.clientHeight || 800))
  };
}

function svgNode(tag, attrs) {
  const node = document.createElementNS("http://www.w3.org/2000/svg", tag);
  for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, String(value));
  return node;
}

class SvgRenderer {
  constructor(host, snapshot, view) {
    this.host = host;
    this.snapshot = snapshot;
    this.view = normalizeView(view);
    this.overlay = {};
    this.renderBase();
  }
  renderBase() {
    clearHost(this.host);
    const size = hostSize(this.host);
    this.root = svgNode("svg", { width: size.width, height: size.height, "data-renderer": "svg" });
    this.host.appendChild(this.root);
    this.plan = buildRenderPlan(this.snapshot, this.view);
    for (const page of this.plan.pages) {
      this.root.appendChild(svgNode("rect", {
        x: page.x, y: page.y, width: page.width, height: page.height,
        fill: "white", stroke: "rgba(0,0,0,0.25)", "data-page-id": page.page_id
      }));
      for (const node of page.nodes) {
        this.root.appendChild(svgNode("rect", {
          x: node.x, y: node.y, width: node.width, height: node.height,
          fill: node.paint.fill ?? "rgba(120,120,120,0.08)",
          stroke: node.paint.stroke ?? (node.diagnostics.length ? "rgba(180,80,0,0.9)" : "rgba(0,0,0,0.15)"),
          "stroke-width": Math.max(0.5, node.paint.stroke_width_css_px || 0.5),
          "data-node-id": node.node_id
        }));
        if (node.story) {
          const label = svgNode("text", {
            x: node.x + 2, y: node.y + 14, "font-size": 12,
            "data-text-authority": node.story.authority
          });
          label.textContent = node.story.text.slice(0, 120);
          this.root.appendChild(label);
        } else if (node.resource) {
          const label = svgNode("text", { x: node.x + 2, y: node.y + 14, "font-size": 10 });
          label.textContent = "image:" + node.resource.availability;
          this.root.appendChild(label);
        }
      }
    }
    this.overlayGroup = svgNode("g", { "data-layer": "transient-overlay" });
    this.root.appendChild(this.overlayGroup);
    this.renderOverlay();
  }
  renderOverlay() {
    while (this.overlayGroup.firstChild) this.overlayGroup.removeChild(this.overlayGroup.firstChild);
    const overlay = buildOverlayPlan(this.plan, this.overlay);
    if (!overlay.selected_bounds) return;
    const b = overlay.selected_bounds;
    this.overlayGroup.appendChild(svgNode("rect", {
      x: b.x, y: b.y, width: b.width, height: b.height,
      fill: "none", stroke: "rgba(0,80,220,0.95)", "stroke-width": 2,
      "data-selection-node-id": overlay.selected_node_id
    }));
  }
  setView(view) { this.view = normalizeView(view); this.renderBase(); }
  updateOverlay(overlay) { this.overlay = { ...overlay }; this.renderOverlay(); }
  stats() { return { available: true, renderer: "svg", dom_elements: this.host.querySelectorAll("*").length }; }
  destroy() { clearHost(this.host); }
}

function setupCanvas(host, rendererName) {
  clearHost(host);
  const size = hostSize(host);
  host.style.position = "relative";
  const base = document.createElement("canvas");
  const overlay = document.createElement("canvas");
  base.width = overlay.width = size.width;
  base.height = overlay.height = size.height;
  base.dataset.renderer = rendererName;
  overlay.dataset.layer = "transient-overlay";
  [base, overlay].forEach((canvas) => {
    canvas.style.position = "absolute";
    canvas.style.inset = "0";
  });
  overlay.style.pointerEvents = "none";
  host.append(base, overlay);
  return { ...size, base, overlay };
}

function drawOverlay(canvas, plan, overlayState) {
  const context = canvas.getContext("2d");
  context.clearRect(0, 0, canvas.width, canvas.height);
  const overlay = buildOverlayPlan(plan, overlayState);
  if (!overlay.selected_bounds) return;
  const b = overlay.selected_bounds;
  context.strokeStyle = "rgba(0,80,220,0.95)";
  context.lineWidth = 2;
  context.strokeRect(b.x, b.y, b.width, b.height);
}

class Canvas2dRenderer {
  constructor(host, snapshot, view) {
    this.host = host;
    this.snapshot = snapshot;
    this.view = normalizeView(view);
    this.overlay = {};
    this.surface = setupCanvas(host, "canvas2d");
    this.renderBase();
  }
  renderBase() {
    this.plan = buildRenderPlan(this.snapshot, this.view);
    const context = this.surface.base.getContext("2d");
    context.clearRect(0, 0, this.surface.width, this.surface.height);
    for (const page of this.plan.pages) {
      context.fillStyle = "white";
      context.fillRect(page.x, page.y, page.width, page.height);
      context.strokeStyle = "rgba(0,0,0,0.25)";
      context.strokeRect(page.x, page.y, page.width, page.height);
      for (const node of page.nodes) {
        if (node.paint.fill) {
          context.fillStyle = node.paint.fill;
          context.fillRect(node.x, node.y, node.width, node.height);
        }
        context.strokeStyle = node.paint.stroke ?? (node.diagnostics.length ? "rgba(180,80,0,0.9)" : "rgba(0,0,0,0.15)");
        context.lineWidth = Math.max(0.5, node.paint.stroke_width_css_px || 0.5);
        context.strokeRect(node.x, node.y, node.width, node.height);
        if (node.story) {
          context.fillStyle = "rgba(0,0,0,0.9)";
          context.font = "12px sans-serif";
          context.fillText(node.story.text.slice(0, 120), node.x + 2, node.y + 14);
        } else if (node.resource) {
          context.fillStyle = "rgba(0,0,0,0.7)";
          context.font = "10px sans-serif";
          context.fillText("image:" + node.resource.availability, node.x + 2, node.y + 14);
        }
      }
    }
    drawOverlay(this.surface.overlay, this.plan, this.overlay);
  }
  setView(view) { this.view = normalizeView(view); this.renderBase(); }
  updateOverlay(overlay) { this.overlay = { ...overlay }; drawOverlay(this.surface.overlay, this.plan, this.overlay); }
  stats() { return { available: true, renderer: "canvas2d", dom_elements: this.host.querySelectorAll("*").length }; }
  destroy() { clearHost(this.host); }
}

function fillWebGlRect(gl, canvasHeight, rect, color) {
  const x = Math.floor(rect.x);
  const y = Math.floor(canvasHeight - rect.y - rect.height);
  const width = Math.max(0, Math.ceil(rect.width));
  const height = Math.max(0, Math.ceil(rect.height));
  if (!width || !height) return;
  const parts = color?.match(/[\d.]+/g)?.map(Number) ?? [120, 120, 120, 0.12];
  gl.scissor(x, y, width, height);
  gl.clearColor(parts[0] / 255, parts[1] / 255, parts[2] / 255, parts[3] ?? 1);
  gl.clear(gl.COLOR_BUFFER_BIT);
}

class WebGl2HybridRenderer {
  constructor(host, snapshot, view) {
    this.host = host;
    this.snapshot = snapshot;
    this.view = normalizeView(view);
    this.overlay = {};
    this.surface = setupCanvas(host, "webgl2-hybrid");
    this.gl = this.surface.base.getContext("webgl2", { antialias: false, preserveDrawingBuffer: true });
    if (!this.gl) {
      this.available = false;
      this.unavailableReason = "webgl2_context_unavailable";
      return;
    }
    this.available = true;
    this.textLayer = document.createElement("div");
    this.textLayer.dataset.layer = "text-diagnostics";
    this.textLayer.style.position = "absolute";
    this.textLayer.style.inset = "0";
    this.textLayer.style.pointerEvents = "none";
    this.host.insertBefore(this.textLayer, this.surface.overlay);
    this.renderBase();
  }
  renderBase() {
    if (!this.available) return;
    this.plan = buildRenderPlan(this.snapshot, this.view);
    const gl = this.gl;
    gl.enable(gl.SCISSOR_TEST);
    gl.viewport(0, 0, this.surface.width, this.surface.height);
    gl.scissor(0, 0, this.surface.width, this.surface.height);
    gl.clearColor(0.94, 0.94, 0.94, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    this.textLayer.replaceChildren();
    for (const page of this.plan.pages) {
      fillWebGlRect(gl, this.surface.height, page, "rgba(255,255,255,1)");
      for (const node of page.nodes) {
        fillWebGlRect(
          gl, this.surface.height, node,
          node.paint.fill ?? (node.diagnostics.length ? "rgba(255,190,90,0.35)" : "rgba(120,120,120,0.12)")
        );
        if (node.story || node.resource || node.diagnostics.length) {
          const label = document.createElement("div");
          label.dataset.nodeId = node.node_id;
          label.dataset.textAuthority = node.story?.authority ?? "none";
          label.style.position = "absolute";
          label.style.left = (node.x + 2) + "px";
          label.style.top = (node.y + 2) + "px";
          label.style.maxWidth = Math.max(1, node.width - 4) + "px";
          label.style.overflow = "hidden";
          label.style.whiteSpace = "nowrap";
          label.style.font = "11px sans-serif";
          label.textContent = node.story?.text.slice(0, 120) ??
            (node.resource ? "image:" + node.resource.availability : "diagnostic");
          this.textLayer.appendChild(label);
        }
      }
    }
    drawOverlay(this.surface.overlay, this.plan, this.overlay);
  }
  setView(view) { this.view = normalizeView(view); this.renderBase(); }
  updateOverlay(overlay) {
    if (!this.available) return;
    this.overlay = { ...overlay };
    drawOverlay(this.surface.overlay, this.plan, this.overlay);
  }
  stats() {
    return {
      available: this.available,
      renderer: "webgl2-hybrid",
      unavailable_reason: this.available ? null : this.unavailableReason,
      dom_elements: this.host.querySelectorAll("*").length
    };
  }
  destroy() { clearHost(this.host); }
}

export function createRenderer(kind, host, snapshot, view = {}) {
  if (!RENDERER_KINDS.includes(kind)) throw new Error("unknown renderer kind: " + kind);
  if (kind === "svg") return new SvgRenderer(host, snapshot, view);
  if (kind === "canvas2d") return new Canvas2dRenderer(host, snapshot, view);
  return new WebGl2HybridRenderer(host, snapshot, view);
}
