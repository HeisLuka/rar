import test from "node:test";
import assert from "node:assert/strict";

import {
  assertFontEnvironmentMatchesScene,
  browserTextMetricAuthority,
  resolveBrowserFont,
} from "./font-environment-v1.mjs";

const scene = {
  document_id: "11111111-1111-4111-8111-111111111111",
  revision_id: "sha256:" + "1".repeat(64),
  snapshot_id: "sha256:" + "2".repeat(64),
  layout_environment: {
    environment_id: "sha256:" + "3".repeat(64),
    font_set_fingerprint: "sha256:" + "4".repeat(64),
  },
  capabilities: [{ key: "render.text", state: "partial", note: null }],
};

function env() {
  return {
    protocol_version: "chaptera.font-environment.v1",
    document_id: scene.document_id,
    revision_id: scene.revision_id,
    scene_snapshot_id: scene.snapshot_id,
    layout_environment_id: scene.layout_environment.environment_id,
    font_set_fingerprint: scene.layout_environment.font_set_fingerprint,
    preview_authority: "server_frame_geometry_only",
    fonts: [],
    diagnostics: [],
  };
}

test("font environment is fenced to exact scene/layout identity", () => {
  assert.doesNotThrow(() => assertFontEnvironmentMatchesScene(scene, env()));
  const changed = env();
  changed.font_set_fingerprint = "sha256:" + "9".repeat(64);
  assert.throws(
    () => assertFontEnvironmentMatchesScene(scene, changed),
    /font_set_fingerprint mismatch/,
  );
});

test("partial text scene cannot be promoted to positioned-glyph authority", () => {
  const changed = env();
  changed.preview_authority = "server_positioned_glyphs";
  assert.throws(
    () => assertFontEnvironmentMatchesScene(scene, changed),
    /does not provide authoritative positioned text/,
  );
});

test("blocked and server-only fonts never invent host fallback", () => {
  const blocked = resolveBrowserFont({
    font_fingerprint: "sha256:" + "5".repeat(64),
    delivery: "blocked",
    reason_code: "delivery.blocked",
  });
  assert.deepEqual(blocked, {
    kind: "blocked",
    font_fingerprint: "sha256:" + "5".repeat(64),
    reason_code: "delivery.blocked",
  });

  const serverOnly = resolveBrowserFont({
    font_fingerprint: "sha256:" + "6".repeat(64),
    delivery: "server_render_only",
    reason_code: "delivery.not_authorized",
  });
  assert.equal(serverOnly.kind, "server_render_only");
  assert.ok(!("family" in serverOnly));
});

test("explicit substitution uses only the declared fingerprinted fallback", () => {
  const resolved = resolveBrowserFont({
    font_fingerprint: "sha256:" + "5".repeat(64),
    delivery: "substitute_explicit",
    reason_code: "source.missing",
    fallback: {
      font_fingerprint: "sha256:" + "7".repeat(64),
      content_hash: "8".repeat(64),
      face_index: 0,
      family: "Fallback Sans",
      style: "Regular",
      resource_id: "81111111-1111-4111-8111-111111111111",
      fetch_handle: "font_resource_fallback",
    },
  });
  assert.equal(resolved.kind, "explicit_substitute");
  assert.equal(resolved.font_fingerprint, "sha256:" + "7".repeat(64));
  assert.equal(resolved.family, "Fallback Sans");
});

test("browser text metrics are never canonical document authority", () => {
  assert.deepEqual(browserTextMetricAuthority(env()), {
    browser_metrics_authoritative: false,
    preview_authority: "server_frame_geometry_only",
    rule: "browser metrics are preview/composition only; server relayout wins",
  });
});
