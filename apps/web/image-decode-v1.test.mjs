import test from "node:test";
import assert from "node:assert/strict";
import {
  BrowserImageDecodeCoordinatorV1,
  EXACT_CREATE_IMAGE_BITMAP_OPTIONS,
  decodeBrowserBitmapExactV1,
  normalizedBrowserDecodeReceiptV1,
} from "./image-decode-v1.mjs";
import { ResourceReadinessRuntimeV1 } from "./resource-readiness-v1.mjs";

const BYTES = new Uint8Array([1,2,3,4]);
const HASH = "9f64a747e1b97f131fabb6b447296c9b6f0201e79fb3c5356e6c77e89b6a806a";
const IDENTITY = Object.freeze({
  resource_id: "resource:1",
  content_hash: HASH,
  derivative_id: "original",
});

function reference({ width = 3, height = 2, alpha = false, color = "color-disposition:synthetic-srgb" } = {}) {
  return {
    contract_version: "chaptera.image-decode-contract.v1",
    resource_sha256: HASH,
    mime_type: "image/png",
    encoded_dimensions: [width,height],
    decoded_dimensions: [width,height],
    source_sample_model: alpha ? "png_rgba" : "png_rgb",
    decoded_sample_model: alpha ? "rgba" : "rgb",
    source_precision_bits: 8,
    decoded_precision_bits: 8,
    alpha_present: alpha,
    alpha_association: alpha ? "straight_unassociated" : "none",
    orientation_class: "normal",
    exif_orientation: null,
    orientation_applied: false,
    frame_disposition: "single_static",
    coding_process: "lossless_png",
    decoder_id: "test-reference",
    color_disposition_ref: color,
    decoded_byte_count: width * height * (alpha ? 4 : 3),
    sample_digest_sha256: "a".repeat(64),
    cache_identity_sha256: "b".repeat(64),
    unsupported_or_error_code: null,
  };
}

function runtime() {
  return new ResourceReadinessRuntimeV1({
    document_id: "doc:1",
    revision_id: "rev:1",
    snapshot_id: "snap:1",
  });
}

function fakeBitmap(width = 3, height = 2) {
  return {
    width,
    height,
    closed: false,
    close() { this.closed = true; },
  };
}

test("exact adapter passes only explicit source-exact createImageBitmap options", async () => {
  let observed = null;
  const image = fakeBitmap();
  const coordinator = new BrowserImageDecodeCoordinatorV1({
    readiness: runtime(),
    backend_id: "unit-browser",
    createImageBitmapFn: async (_blob, options) => {
      observed = { ...options };
      return image;
    },
  });
  const result = await coordinator.decodeExact({
    identity: IDENTITY,
    blob: new Blob([BYTES], { type: "image/png" }),
    reference_receipt: reference(),
  });
  assert.equal(result.status, "ready");
  assert.deepEqual(observed, {
    imageOrientation: "none",
    premultiplyAlpha: "none",
    colorSpaceConversion: "none",
  });
  assert.deepEqual(observed, EXACT_CREATE_IMAGE_BITMAP_OPTIONS);
  assert.equal("resizeWidth" in observed, false);
  assert.equal("resizeHeight" in observed, false);
  assert.equal(result.receipt.browser_is_semantic_authority, false);
  assert.equal(result.receipt.orientation_policy, "preserve_metadata_do_not_apply");
});

test("readiness generation rejects stale completion and closes bitmap", async () => {
  let resolveDecode;
  let markDecodeStarted;
  const decodeStarted = new Promise((resolve) => { markDecodeStarted = resolve; });
  const image = fakeBitmap();
  const ready = runtime();
  const coordinator = new BrowserImageDecodeCoordinatorV1({
    readiness: ready,
    createImageBitmapFn: () => {
      markDecodeStarted();
      return new Promise((resolve) => { resolveDecode = () => resolve(image); });
    },
  });
  const pending = coordinator.decodeExact({
    identity: IDENTITY,
    blob: new Blob([BYTES], { type: "image/png" }),
    reference_receipt: reference(),
  });
  await decodeStarted;
  ready.beginRequest(IDENTITY);
  resolveDecode();
  const result = await pending;
  assert.equal(result.status, "stale");
  assert.equal(result.error_code, "readiness_generation_stale");
  assert.equal(image.closed, true);
  assert.equal(ready.receipt().metrics.stale_completions_rejected, 1);
});

test("policy generation rejects old decode and releases bitmap", async () => {
  let resolveDecode;
  let markDecodeStarted;
  const decodeStarted = new Promise((resolve) => { markDecodeStarted = resolve; });
  const image = fakeBitmap();
  const ready = runtime();
  const coordinator = new BrowserImageDecodeCoordinatorV1({
    readiness: ready,
    createImageBitmapFn: () => {
      markDecodeStarted();
      return new Promise((resolve) => { resolveDecode = () => resolve(image); });
    },
  });
  const pending = coordinator.decodeExact({
    identity: IDENTITY,
    blob: new Blob([BYTES], { type: "image/png" }),
    reference_receipt: reference(),
  });
  await decodeStarted;
  coordinator.bumpPolicyGeneration();
  resolveDecode();
  const result = await pending;
  assert.equal(result.status, "stale");
  assert.equal(result.error_code, "decode_policy_generation_stale");
  assert.equal(image.closed, true);
  assert.equal(ready.resourceState(IDENTITY).state, "blocked");
});

test("unsupported browser API produces explicit fallback state", async () => {
  const ready = runtime();
  const coordinator = new BrowserImageDecodeCoordinatorV1({
    readiness: ready,
    createImageBitmapFn: null,
  });
  const result = await coordinator.decodeExact({
    identity: IDENTITY,
    blob: new Blob([BYTES], { type: "image/png" }),
    reference_receipt: reference(),
  });
  assert.equal(result.status, "reference_fallback_required");
  assert.equal(result.error_code, "create_image_bitmap_unavailable");
  assert.equal(ready.resourceState(IDENTITY).state, "blocked");
});

test("dimension mismatch cannot publish as exact and closes object", async () => {
  const image = fakeBitmap(2,3);
  const ready = runtime();
  const coordinator = new BrowserImageDecodeCoordinatorV1({
    readiness: ready,
    createImageBitmapFn: async () => image,
  });
  const result = await coordinator.decodeExact({
    identity: IDENTITY,
    blob: new Blob([BYTES], { type: "image/png" }),
    reference_receipt: reference({width:3,height:2}),
  });
  assert.equal(result.status, "reference_fallback_required");
  assert.equal(result.error_code, "browser_dimensions_mismatch");
  assert.equal(image.closed, true);
  assert.equal(ready.resourceState(IDENTITY).state, "blocked");
});

test("non-sRGB reference is explicit unsupported rather than browser-default conversion", async () => {
  const ready = runtime();
  const coordinator = new BrowserImageDecodeCoordinatorV1({
    readiness: ready,
    createImageBitmapFn: async () => fakeBitmap(),
  });
  const result = await coordinator.decodeExact({
    identity: IDENTITY,
    blob: new Blob([BYTES], { type: "image/png" }),
    reference_receipt: reference({color:"color-disposition:icc-non-srgb"}),
  });
  assert.equal(result.status, "reference_fallback_required");
  assert.equal(result.error_code, "non_srgb_exact_unsupported");
});

test("main and worker receipts normalize to one decode identity", async () => {
  const args = {
    blob: new Blob([BYTES], { type: "image/png" }),
    identity: IDENTITY,
    reference_receipt: reference(),
    request_generation: 1,
    backend_generation: 2,
    policy_generation: 3,
    createImageBitmapFn: async () => fakeBitmap(),
  };
  const main = await decodeBrowserBitmapExactV1({
    ...args,
    placement: "main-thread",
    backend_id: "chromium-main",
  });
  const worker = await decodeBrowserBitmapExactV1({
    ...args,
    placement: "worker",
    backend_id: "chromium-worker",
  });
  assert.deepEqual(
    normalizedBrowserDecodeReceiptV1(main.receipt),
    normalizedBrowserDecodeReceiptV1(worker.receipt),
  );
  main.bitmap.close();
  worker.bitmap.close();
});
