export function assertFontEnvironmentMatchesScene(scene, fontEnvironment) {
  const checks = [
    ["document_id", scene.document_id, fontEnvironment.document_id],
    ["revision_id", scene.revision_id, fontEnvironment.revision_id],
    ["scene_snapshot_id", scene.snapshot_id, fontEnvironment.scene_snapshot_id],
    [
      "layout_environment_id",
      scene.layout_environment.environment_id,
      fontEnvironment.layout_environment_id,
    ],
    [
      "font_set_fingerprint",
      scene.layout_environment.font_set_fingerprint,
      fontEnvironment.font_set_fingerprint,
    ],
  ];
  for (const [label, expected, actual] of checks) {
    if (expected !== actual) {
      throw new Error(`${label} mismatch`);
    }
  }
  if (fontEnvironment.preview_authority === "server_positioned_glyphs") {
    const text = scene.capabilities.find((item) => item.key === "render.text");
    if (!text || text.state !== "supported") {
      throw new Error("scene does not provide authoritative positioned text");
    }
  }
}

export function resolveBrowserFont(fontDescriptor) {
  switch (fontDescriptor.delivery) {
    case "deliver_exact":
    case "deliver_subset":
      if (!fontDescriptor.resource_id || !fontDescriptor.fetch_handle) {
        throw new Error("font delivery requires explicit resource");
      }
      return {
        kind: "explicit_font_resource",
        font_fingerprint: fontDescriptor.font_fingerprint,
        content_hash: fontDescriptor.content_hash,
        face_index: fontDescriptor.face_index,
        resource_id: fontDescriptor.resource_id,
        fetch_handle: fontDescriptor.fetch_handle,
      };

    case "substitute_explicit":
      if (!fontDescriptor.fallback) {
        throw new Error("explicit substitution requires fallback");
      }
      return {
        kind: "explicit_substitute",
        source_font_fingerprint: fontDescriptor.font_fingerprint,
        ...fontDescriptor.fallback,
      };

    case "server_render_only":
      return {
        kind: "server_render_only",
        font_fingerprint: fontDescriptor.font_fingerprint,
        reason_code: fontDescriptor.reason_code,
      };

    case "blocked":
      return {
        kind: "blocked",
        font_fingerprint: fontDescriptor.font_fingerprint,
        reason_code: fontDescriptor.reason_code,
      };

    default:
      throw new Error("unknown font delivery mode");
  }
}

export function browserTextMetricAuthority(fontEnvironment) {
  return {
    browser_metrics_authoritative: false,
    preview_authority: fontEnvironment.preview_authority,
    rule:
      fontEnvironment.preview_authority === "server_positioned_glyphs"
        ? "render server positions; browser font metrics do not relayout"
        : "browser metrics are preview/composition only; server relayout wins",
  };
}
