const CURRENT_DOCUMENT_V1 = "chaptera.current-document.v1";
const SCENE_V1 = "chaptera.scene.v1";

const PARTIAL_REASONS = Object.freeze([
  "paint_projection_deferred",
  "resource_projection_deferred",
  "stacking_order_unavailable",
  "text_style_projection_deferred",
]);

function object(value, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError(label + " must be an object");
  }
  return value;
}

function safeInteger(value, label) {
  if (!Number.isSafeInteger(value)) {
    throw new RangeError(label + " must be a JavaScript-safe integer");
  }
  return value;
}

function positiveSafeInteger(value, label) {
  safeInteger(value, label);
  if (!(value > 0)) throw new RangeError(label + " must be positive");
  return value;
}

function canonicalize(value) {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (value && typeof value === "object") {
    const out = {};
    for (const key of Object.keys(value).sort()) out[key] = canonicalize(value[key]);
    return out;
  }
  return value;
}

async function sha256HexBytes(bytes) {
  const digest = await globalThis.crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

async function hashId(value) {
  const canonical = JSON.stringify(canonicalize(value));
  const hex = await sha256HexBytes(new TextEncoder().encode(canonical));
  return "sha256:" + hex;
}

async function hashText(value) {
  const hex = await sha256HexBytes(new TextEncoder().encode(value));
  return "sha256:" + hex;
}

function severityRank(value) {
  return { info: 0, warning: 1, error: 2 }[value] ?? 99;
}

function normalizeScene(snapshot) {
  const value = structuredClone(snapshot);
  value.pages.sort((a, b) => a.order - b.order || a.page_id.localeCompare(b.page_id));
  const pageOrder = new Map(value.pages.map((page) => [page.page_id, page.order]));
  value.nodes.sort((a, b) => {
    const ap = pageOrder.get(a.page_id) ?? Number.MAX_SAFE_INTEGER;
    const bp = pageOrder.get(b.page_id) ?? Number.MAX_SAFE_INTEGER;
    if (ap !== bp) return ap - bp;
    const az = a.z_order === null;
    const bz = b.z_order === null;
    if (az !== bz) return az ? 1 : -1;
    if (!az && a.z_order !== b.z_order) return a.z_order - b.z_order;
    const apaint = a.paint_order === null;
    const bpaint = b.paint_order === null;
    if (apaint !== bpaint) return apaint ? 1 : -1;
    if (!apaint && a.paint_order !== b.paint_order) return a.paint_order - b.paint_order;
    return a.node_id.localeCompare(b.node_id);
  });
  value.stories.sort((a, b) => a.story_id.localeCompare(b.story_id));
  value.story_frames.sort(
    (a, b) =>
      a.story_id.localeCompare(b.story_id) ||
      a.frame_ordinal - b.frame_ordinal ||
      a.node_id.localeCompare(b.node_id),
  );
  value.paints.sort((a, b) => a.paint_id.localeCompare(b.paint_id));
  value.resources.sort((a, b) => a.resource_id.localeCompare(b.resource_id));
  value.diagnostics.sort(
    (a, b) =>
      severityRank(a.severity) - severityRank(b.severity) ||
      a.code.localeCompare(b.code) ||
      (a.origin_node_id ?? "").localeCompare(b.origin_node_id ?? "") ||
      a.message_key.localeCompare(b.message_key),
  );
  value.capabilities.sort(
    (a, b) =>
      a.key.localeCompare(b.key) ||
      a.state.localeCompare(b.state) ||
      (a.note ?? "").localeCompare(b.note ?? ""),
  );
  value.fidelity.reasons.sort();
  return value;
}

async function finalizeScene(snapshot) {
  const normalized = normalizeScene(snapshot);
  const withoutId = structuredClone(normalized);
  delete withoutId.snapshot_id;
  normalized.snapshot_id = await hashId(withoutId);
  return normalizeScene(normalized);
}

function sceneNodeKind(kind) {
  switch (kind) {
    case "shape":
      return "shape";
    case "text_frame":
      return "text_frame";
    case "image_frame":
      return "picture_frame";
    case "group":
      return "group";
    case "table":
      return "table";
    case "connector":
      return "line";
    default:
      return "unknown";
  }
}

function validateTransform(transform, label) {
  object(transform, label);
  for (const key of ["a", "b", "c", "d"]) {
    if (typeof transform[key] !== "string" || transform[key].length === 0) {
      throw new TypeError(label + "." + key + " must be an exact decimal string");
    }
  }
  return {
    a: transform.a,
    b: transform.b,
    c: transform.c,
    d: transform.d,
    tx: safeInteger(transform.tx, label + ".tx"),
    ty: safeInteger(transform.ty, label + ".ty"),
  };
}

function resolveNodePage(nodeId, nodes, pageIds, visiting = new Set()) {
  if (visiting.has(nodeId)) throw new Error("authoring graph contains a node parent cycle");
  visiting.add(nodeId);
  const node = object(nodes[nodeId], "authoring_graph.nodes[" + nodeId + "]");
  const header = object(node.header, "node.header");
  const parent = header.parent_id;
  if (pageIds.has(parent)) return parent;
  if (nodes[parent]) return resolveNodePage(parent, nodes, pageIds, visiting);
  throw new Error("authoring graph node parent does not resolve to a document page");
}

async function geometryLayoutEnvironment(graph) {
  const engineRevision =
    typeof graph.resolver_version === "string" && graph.resolver_version.length
      ? graph.resolver_version
      : "authoring-graph-resolver-unknown";
  const fontSetFingerprint = await hashText("chaptera.phase-c.geometry-only.fonts-unresolved.v1");
  const resourceFingerprint = await hashText(
    "chaptera.phase-c.geometry-only.resources-unresolved.v1",
  );
  const environmentId = await hashId({
    engine_revision: engineRevision,
    font_set_fingerprint: fontSetFingerprint,
    resource_fingerprint: resourceFingerprint,
  });
  return {
    environment_id: environmentId,
    engine_revision: engineRevision,
    font_set_fingerprint: fontSetFingerprint,
    resource_fingerprint: resourceFingerprint,
  };
}

export async function projectCurrentAuthoringGraphToScene(current) {
  object(current, "current document response");
  if (current.protocol_version !== CURRENT_DOCUMENT_V1) {
    throw new Error("chaptera.current-document.v1 is required");
  }
  if (typeof current.document_id !== "string" || current.document_id.length === 0) {
    throw new TypeError("document_id is required");
  }
  if (typeof current.source_hash !== "string" || !/^[0-9a-f]{64}$/.test(current.source_hash)) {
    throw new TypeError("source_hash must be lowercase SHA-256");
  }
  if (typeof current.revision_id !== "string" || !current.revision_id.startsWith("sha256:")) {
    throw new TypeError("revision_id must be a sha256 identity");
  }

  const graph = object(current.authoring_graph, "authoring_graph");
  const document = object(graph.document, "authoring_graph.document");
  if (document.source_hash !== current.source_hash) {
    throw new Error("authoring graph source identity differs from current document");
  }
  if (!Array.isArray(document.pages)) throw new TypeError("authoring graph document.pages is required");

  const pagesById = object(graph.pages, "authoring_graph.pages");
  const nodesById = object(graph.nodes, "authoring_graph.nodes");
  const storiesById = object(graph.stories ?? {}, "authoring_graph.stories");
  const pageIds = new Set(document.pages);
  if (pageIds.size !== document.pages.length) throw new Error("authoring graph contains duplicate document pages");
  for (const pageId of Object.keys(pagesById)) {
    if (!pageIds.has(pageId)) throw new Error("authoring graph contains a page outside document order");
  }

  const pages = document.pages.map((pageId, order) => {
    const page = object(pagesById[pageId], "authoring_graph.pages[" + pageId + "]");
    if (page.id !== pageId) throw new Error("authoring graph page key/id mismatch");
    const size = object(page.size, "page.size");
    return {
      page_id: pageId,
      order,
      width_emu: positiveSafeInteger(size.width, "page.size.width"),
      height_emu: positiveSafeInteger(size.height, "page.size.height"),
    };
  });

  const nodes = [];
  const storyFrames = [];
  for (const [nodeId, rawNode] of Object.entries(nodesById)) {
    const node = object(rawNode, "authoring_graph.nodes[" + nodeId + "]");
    const header = object(node.header, "node.header");
    if (header.id !== nodeId) throw new Error("authoring graph node key/id mismatch");
    const bounds = object(header.bounds, "node.header.bounds");
    const parentIsNode = Boolean(nodesById[header.parent_id]);
    const pageId = resolveNodePage(nodeId, nodesById, pageIds);
    nodes.push({
      node_id: nodeId,
      page_id: pageId,
      parent_node_id: parentIsNode ? header.parent_id : null,
      kind: sceneNodeKind(node.kind),
      bounds: {
        x: safeInteger(bounds.x, "node.bounds.x"),
        y: safeInteger(bounds.y, "node.bounds.y"),
        width: safeInteger(bounds.width, "node.bounds.width"),
        height: safeInteger(bounds.height, "node.bounds.height"),
      },
      z_order: null,
      paint_order: null,
      paint_id: null,
      resource_id: null,
      transform: validateTransform(header.transform, "node.header.transform"),
    });

    const frame = node.payload?.story_frame ?? null;
    if (frame?.story_id) {
      safeInteger(frame.ordinal, "story frame ordinal");
      storyFrames.push({
        story_id: frame.story_id,
        frame_ordinal: frame.ordinal,
        node_id: nodeId,
      });
    }
  }

  const stories = Object.entries(storiesById).map(([storyId, rawStory]) => {
    const story = object(rawStory, "authoring_graph.stories[" + storyId + "]");
    if (story.id !== storyId) throw new Error("authoring graph story key/id mismatch");
    if (typeof story.text !== "string") throw new TypeError("authoring graph story text must be a string");
    return {
      story_id: storyId,
      text: story.text,
      text_fidelity: "partial",
    };
  });
  for (const frame of storyFrames) {
    if (!storiesById[frame.story_id]) {
      throw new Error("authoring graph story frame refers to a missing story");
    }
  }

  const scene = {
    protocol_version: SCENE_V1,
    document_id: current.document_id,
    source_hash: current.source_hash,
    revision_id: current.revision_id,
    snapshot_id: "sha256:" + "0".repeat(64),
    layout_environment: await geometryLayoutEnvironment(graph),
    pages,
    nodes,
    stories,
    story_frames: storyFrames,
    paints: [],
    resources: [],
    diagnostics: [
      {
        severity: "warning",
        code: "SCENE.AUTHORING_GRAPH.PARTIAL",
        message_key: "scene.authoring_graph.partial_projection",
      },
    ],
    capabilities: [
      { key: "render.geometry", state: "supported", note: null },
      {
        key: "render.paint",
        state: "unsupported",
        note: "Paint projection is not yet admitted from the authoring graph.",
      },
      {
        key: "render.resources",
        state: "unsupported",
        note: "Resource projection is not yet admitted from the authoring graph.",
      },
      {
        key: "render.stacking",
        state: "unsupported",
        note: "ResolvedGraph does not currently carry authoritative stacking order.",
      },
      {
        key: "render.text",
        state: "partial",
        note: "Story text is available without full text style/layout projection.",
      },
    ],
    fidelity: {
      state: "partial",
      reasons: [...PARTIAL_REASONS],
    },
    stacking_fidelity: "unknown",
  };
  return finalizeScene(scene);
}
