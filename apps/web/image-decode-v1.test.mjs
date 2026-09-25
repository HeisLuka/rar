import test from "node:test";
import assert from "node:assert/strict";
import {
  BrowserImageDecodeAdapterV1,
  EXACT_CREATE_IMAGE_BITMAP_OPTIONS_V1,
} from "./image-decode-v1.mjs";

const hash = "a".repeat(64);
const identity = Object.freeze({
  resource_id: "image:fixture",
  derivative_id: "source-exact",
  content_hash: hash,
  mime_type: "image/png",
});
const reference = Object.freeze({
  contract_version: "chaptera.image-decode-contract.v1",
  resource_sha256: hash,
  mime_type: "image/png",
  encoded_dimensions: [2, 1],
  decoded_dimensions: [2, 1],
  alpha_association: "straight_unassociated",
  exif_orientation: null,
  orientation_applied: false,
  decoder_id: "reference",
  unsupported_or_error_code: null,
});

async function withHash(bytes) {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

async function fixtureIdentity(bytes) {
  return { ...identity, content_hash: await withHash(bytes) };
}

test("exact path always requests explicit orientation/alpha/color policies and no resize", async () => {
  const bytes = new Uint8Array([1, 2, 3, 4]);
  const actualIdentity = await fixtureIdentity(bytes);
  const actualReference = { ...reference, resource_sha256: actualIdentity.content_hash };
  let observedOptions;
  const bitmap = { width: 2, height: 1, closed: false, close() { this.closed = true; } };
  const adapter = new BrowserImageDecodeAdapterV1({
    createImageBitmapImpl: async (_blob, options) => {
      observedOptions = options;
      return bitmap;
    },
  });
  const result = await adapter.decode(adapter.begin(actualIdentity), bytes, actualReference);
  assert.equal(result.accepted, true);
  assert.deepEqual(observedOptions, EXACT_CREATE_IMAGE_BITMAP_OPTIONS_V1);
  assert.equal("resizeWidth" in observedOptions, false);
  assert.equal("resizeHeight" in observedOptions, false);
  assert.equal(result.receipt.browser_default_decode_used, false);
  result.handle.close();
  assert.equal(bitmap.closed, true);
});

test("stale async completion is rejected and graphics object is released", async () => {
  const bytes = new Uint8Array([4, 3, 2, 1]);
  const actualIdentity = await fixtureIdentity(bytes);
  const actualReference = { ...reference, resource_sha256: actualIdentity.content_hash };
  let release;
  const pending = new Promise((resolve) => { release = resolve; });
  const bitmap = { width: 2, height: 1, closed: false, close() { this.closed = true; } };
  const adapter = new BrowserImageDecodeAdapterV1({
    createImageBitmapImpl: async () => {
      await pending;
      return bitmap;
    },
  });
  const oldToken = adapter.begin(actualIdentity);
  const decoding = adapter.decode(oldToken, bytes, actualReference);
  adapter.begin(actualIdentity);
  release();
  const result = await decoding;
  assert.equal(result.accepted, false);
  assert.equal(result.reason, "stale_generation_after_decode");
  assert.equal(bitmap.closed, true);
});

test("dimension mismatch fails closed and releases the bitmap", async () => {
  const bytes = new Uint8Array([9, 8, 7]);
  const actualIdentity = await fixtureIdentity(bytes);
  const actualReference = { ...reference, resource_sha256: actualIdentity.content_hash };
  const bitmap = { width: 1, height: 2, closed: false, close() { this.closed = true; } };
  const adapter = new BrowserImageDecodeAdapterV1({ createImageBitmapImpl: async () => bitmap });
  const result = await adapter.decode(adapter.begin(actualIdentity), bytes, actualReference);
  assert.equal(result.accepted, false);
  assert.equal(result.reason, "decoded_dimensions_mismatch");
  assert.equal(bitmap.closed, true);
});

test("reference receipt that applied EXIF orientation is rejected", async () => {
  const bytes = new Uint8Array([1]);
  const actualIdentity = await fixtureIdentity(bytes);
  const adapter = new BrowserImageDecodeAdapterV1({ createImageBitmapImpl: async () => ({ width: 2, height: 1 }) });
  await assert.rejects(
    adapter.decode(adapter.begin(actualIdentity), bytes, {
      ...reference,
      resource_sha256: actualIdentity.content_hash,
      orientation_applied: true,
    }),
    /preserve EXIF orientation/,
  );
});

test("missing createImageBitmap is explicit unsupported, never a default img fallback", async () => {
  const bytes = new Uint8Array([7, 7]);
  const actualIdentity = await fixtureIdentity(bytes);
  const actualReference = { ...reference, resource_sha256: actualIdentity.content_hash };
  const adapter = new BrowserImageDecodeAdapterV1({ createImageBitmapImpl: null });
  const result = await adapter.decode(adapter.begin(actualIdentity), bytes, actualReference);
  assert.equal(result.accepted, false);
  assert.equal(result.error_code, "create_image_bitmap_unavailable");
});
