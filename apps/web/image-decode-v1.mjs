export const BROWSER_IMAGE_DECODE_SCHEMA = "chaptera.browser-image-decode.v1";
export const BROWSER_IMAGE_DECODE_POLICY = "chaptera.browser-image-decode-policy.v1";

export const EXACT_CREATE_IMAGE_BITMAP_OPTIONS = Object.freeze({
  imageOrientation: "none",
  premultiplyAlpha: "none",
  colorSpaceConversion: "none",
});

const SUPPORTED_MIME = new Set(["image/png", "image/jpeg"]);
const BLOCKING_CODES = new Set([
  "create_image_bitmap_unavailable",
  "crypto_subtle_unavailable",
  "resource_hash_mismatch",
  "reference_receipt_invalid",
  "reference_decode_unsupported",
  "non_srgb_exact_unsupported",
  "explicit_decode_options_unsupported",
  "explicit_orientation_policy_ignored",
  "browser_dimensions_mismatch",
  "decode_policy_generation_stale",
]);

function requireString(value, label) {
  if (typeof value !== "string" || value.length === 0) {
    throw new BrowserImageDecodeError("invalid_argument", label + " must be a non-empty string");
  }
  return value;
}

function normalizeHash(value) {
  const raw = requireString(value, "content_hash").toLowerCase();
  const hash = raw.startsWith("sha256:") ? raw.slice(7) : raw;
  if (!/^[0-9a-f]{64}$/.test(hash)) {
    throw new BrowserImageDecodeError("invalid_argument", "content hash must be SHA-256 lowercase hex");
  }
  return hash;
}

function normalizeIdentity(identity) {
  if (!identity || typeof identity !== "object") {
    throw new BrowserImageDecodeError("invalid_argument", "resource identity is required");
  }
  return Object.freeze({
    resource_id: requireString(identity.resource_id, "resource_id"),
    content_hash: requireString(identity.content_hash, "content_hash"),
    derivative_id: requireString(identity.derivative_id ?? "original", "derivative_id"),
  });
}

function closeQuietly(value) {
  if (value && typeof value.close === "function") {
    try {
      value.close();
      return true;
    } catch {
      return false;
    }
  }
  return false;
}

function referenceReceiptV1(receipt, identity, mimeType) {
  if (!receipt || typeof receipt !== "object") {
    throw new BrowserImageDecodeError("reference_receipt_invalid", "DecodeReceiptV1 is required");
  }
  if (receipt.contract_version !== "chaptera.image-decode-contract.v1") {
    throw new BrowserImageDecodeError("reference_receipt_invalid", "reference decoder contract version mismatch");
  }
  if (receipt.unsupported_or_error_code !== null) {
    throw new BrowserImageDecodeError("reference_decode_unsupported", String(receipt.unsupported_or_error_code));
  }
  if (receipt.mime_type !== mimeType || !SUPPORTED_MIME.has(mimeType)) {
    throw new BrowserImageDecodeError("reference_receipt_invalid", "reference MIME is outside browser V1");
  }
  if (normalizeHash(identity.content_hash) !== normalizeHash(receipt.resource_sha256)) {
    throw new BrowserImageDecodeError("reference_receipt_invalid", "resource identity differs from reference SHA");
  }
  if (
    !Array.isArray(receipt.decoded_dimensions) ||
    receipt.decoded_dimensions.length !== 2 ||
    !receipt.decoded_dimensions.every((v) => Number.isInteger(v) && v > 0)
  ) {
    throw new BrowserImageDecodeError("reference_receipt_invalid", "reference dimensions are invalid");
  }
  if (receipt.orientation_applied !== false || receipt.frame_disposition !== "single_static") {
    throw new BrowserImageDecodeError(
      "reference_receipt_invalid",
      "browser V1 requires non-oriented single-static reference decode",
    );
  }
  const colorRef = requireString(receipt.color_disposition_ref, "color_disposition_ref");
  const normalizedColorRef = colorRef.toLowerCase();
  const explicitlyNonSrgb =
    normalizedColorRef.includes("non-srgb") ||
    normalizedColorRef.includes("non_srgb") ||
    normalizedColorRef.includes("display-p3") ||
    normalizedColorRef.includes("display_p3") ||
    normalizedColorRef.includes("cmyk") ||
    normalizedColorRef.includes("unknown");
  const explicitlySrgb = /(^|[:/_-])srgb($|[:/_-])/.test(normalizedColorRef);
  if (explicitlyNonSrgb || !explicitlySrgb) {
    throw new BrowserImageDecodeError(
      "non_srgb_exact_unsupported",
      "browser exact V1 admits only explicit sRGB reference disposition",
    );
  }
  return receipt;
}

async function sha256HexBlob(blob) {
  const subtle = globalThis.crypto?.subtle;
  if (!subtle) {
    throw new BrowserImageDecodeError(
      "crypto_subtle_unavailable",
      "exact browser resource verification requires Web Crypto",
    );
  }
  const digest = new Uint8Array(await subtle.digest("SHA-256", await blob.arrayBuffer()));
  return [...digest].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function decodeReceipt({
  identity,
  reference,
  requestGeneration,
  backendGeneration,
  policyGeneration,
  placement,
  backendId,
  bitmap,
}) {
  return Object.freeze({
    schema: BROWSER_IMAGE_DECODE_SCHEMA,
    policy_version: BROWSER_IMAGE_DECODE_POLICY,
    resource_id: identity.resource_id,
    content_hash: identity.content_hash,
    derivative_id: identity.derivative_id,
    request_generation: requestGeneration,
    backend_generation: backendGeneration,
    decode_policy_generation: policyGeneration,
    decoder_adapter: "createImageBitmap",
    placement,
    backend_id: backendId,
    effective_options: Object.freeze({ ...EXACT_CREATE_IMAGE_BITMAP_OPTIONS }),
    resize_policy: "none",
    orientation_policy: "preserve_metadata_do_not_apply",
    alpha_association: reference.alpha_present ? "straight_unassociated" : "none",
    color_conversion_policy: "none",
    frame_disposition: "single_static",
    decoded_dimensions: Object.freeze([bitmap.width, bitmap.height]),
    reference_decoded_dimensions: Object.freeze([...reference.decoded_dimensions]),
    reference_decoder_id: reference.decoder_id,
    reference_cache_identity_sha256: reference.cache_identity_sha256,
    reference_sample_digest_sha256: reference.sample_digest_sha256,
    compatibility_state: "explicit_policy_and_dimensions_match_reference",
    browser_is_semantic_authority: false,
    canvas_readback_is_semantic_authority: false,
  });
}

export class BrowserImageDecodeError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "BrowserImageDecodeError";
    this.code = code;
  }
}

export function closeBrowserGraphicsObjectV1(value) {
  return closeQuietly(value);
}

export function normalizedBrowserDecodeReceiptV1(receipt) {
  if (!receipt || receipt.schema !== BROWSER_IMAGE_DECODE_SCHEMA) {
    throw new TypeError("BrowserDecodedImageV1 receipt required");
  }
  return Object.freeze({
    schema: receipt.schema,
    policy_version: receipt.policy_version,
    resource_id: receipt.resource_id,
    content_hash: receipt.content_hash,
    derivative_id: receipt.derivative_id,
    request_generation: receipt.request_generation,
    backend_generation: receipt.backend_generation,
    decode_policy_generation: receipt.decode_policy_generation,
    decoder_adapter: receipt.decoder_adapter,
    effective_options: receipt.effective_options,
    resize_policy: receipt.resize_policy,
    orientation_policy: receipt.orientation_policy,
    alpha_association: receipt.alpha_association,
    color_conversion_policy: receipt.color_conversion_policy,
    frame_disposition: receipt.frame_disposition,
    decoded_dimensions: receipt.decoded_dimensions,
    reference_decoded_dimensions: receipt.reference_decoded_dimensions,
    reference_decoder_id: receipt.reference_decoder_id,
    reference_cache_identity_sha256: receipt.reference_cache_identity_sha256,
    reference_sample_digest_sha256: receipt.reference_sample_digest_sha256,
    compatibility_state: receipt.compatibility_state,
    browser_is_semantic_authority: false,
    canvas_readback_is_semantic_authority: false,
  });
}

export async function decodeBrowserBitmapExactV1({
  blob,
  identity,
  reference_receipt,
  request_generation,
  backend_generation,
  policy_generation,
  placement = "main-thread",
  backend_id = "browser",
  createImageBitmapFn = globalThis.createImageBitmap?.bind(globalThis),
}) {
  identity = normalizeIdentity(identity);
  if (!(blob instanceof Blob)) {
    throw new BrowserImageDecodeError("invalid_argument", "Blob input is required");
  }
  const mimeType = requireString(blob.type, "blob.type");
  const reference = referenceReceiptV1(reference_receipt, identity, mimeType);
  if (!Number.isInteger(request_generation) || request_generation < 1) {
    throw new BrowserImageDecodeError("invalid_argument", "request_generation must be positive");
  }
  if (!Number.isInteger(backend_generation) || backend_generation < 1) {
    throw new BrowserImageDecodeError("invalid_argument", "backend_generation must be positive");
  }
  if (!Number.isInteger(policy_generation) || policy_generation < 1) {
    throw new BrowserImageDecodeError("invalid_argument", "policy_generation must be positive");
  }
  if (!["main-thread", "worker"].includes(placement)) {
    throw new BrowserImageDecodeError("invalid_argument", "placement must be main-thread or worker");
  }
  requireString(backend_id, "backend_id");
  if (typeof createImageBitmapFn !== "function") {
    throw new BrowserImageDecodeError(
      "create_image_bitmap_unavailable",
      "exact createImageBitmap adapter is unavailable",
    );
  }

  const actualHash = await sha256HexBlob(blob);
  if (actualHash !== normalizeHash(identity.content_hash)) {
    throw new BrowserImageDecodeError("resource_hash_mismatch", "Blob bytes differ from ResourceId content hash");
  }

  let bitmap = null;
  try {
    bitmap = await createImageBitmapFn(blob, EXACT_CREATE_IMAGE_BITMAP_OPTIONS);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    const code = error instanceof TypeError
      ? "explicit_decode_options_unsupported"
      : "browser_decode_failed";
    throw new BrowserImageDecodeError(code, message);
  }

  const [expectedWidth, expectedHeight] = reference.decoded_dimensions;
  if (
    !Number.isInteger(bitmap?.width) ||
    !Number.isInteger(bitmap?.height) ||
    bitmap.width !== expectedWidth ||
    bitmap.height !== expectedHeight
  ) {
    const observedDimensions = [bitmap?.width ?? null, bitmap?.height ?? null];
    closeQuietly(bitmap);
    const orientationWasNonNormal =
      Number.isInteger(reference.exif_orientation) && reference.exif_orientation !== 1;
    throw new BrowserImageDecodeError(
      orientationWasNonNormal
        ? "explicit_orientation_policy_ignored"
        : "browser_dimensions_mismatch",
      "browser exact decode dimensions differ from reference non-oriented raster; " +
        "expected=" + JSON.stringify(reference.decoded_dimensions) +
        " observed=" + JSON.stringify(observedDimensions),
    );
  }

  return Object.freeze({
    bitmap,
    receipt: decodeReceipt({
      identity,
      reference,
      requestGeneration: request_generation,
      backendGeneration: backend_generation,
      policyGeneration: policy_generation,
      placement,
      backendId: backend_id,
      bitmap,
    }),
  });
}

export class BrowserImageDecodeCoordinatorV1 {
  constructor({
    readiness,
    createImageBitmapFn = globalThis.createImageBitmap?.bind(globalThis),
    placement = "main-thread",
    backend_id = "browser",
  }) {
    if (
      !readiness ||
      typeof readiness.beginRequest !== "function" ||
      typeof readiness.markDecoding !== "function" ||
      typeof readiness.completeReady !== "function" ||
      typeof readiness.completeTerminal !== "function"
    ) {
      throw new TypeError("ResourceReadinessRuntimeV1-compatible runtime required");
    }
    this.readiness = readiness;
    this.createImageBitmapFn = createImageBitmapFn;
    this.placement = placement;
    this.backendId = requireString(backend_id, "backend_id");
    this.policyGeneration = 1;
  }

  bumpPolicyGeneration() {
    this.policyGeneration += 1;
    return this.policyGeneration;
  }

  begin(identity) {
    identity = normalizeIdentity(identity);
    const token = this.readiness.beginRequest(identity);
    const decoding = this.readiness.markDecoding(token);
    if (!decoding.accepted) {
      throw new BrowserImageDecodeError("readiness_generation_stale", "decode request was stale before start");
    }
    return Object.freeze({
      identity,
      token,
      policy_generation: this.policyGeneration,
    });
  }

  #finishError(handle, error) {
    const code = error instanceof BrowserImageDecodeError ? error.code : "browser_decode_failed";
    const message = error instanceof Error ? error.message : String(error);
    const state = BLOCKING_CODES.has(code) ? "blocked" : "failed";
    const completion = this.readiness.completeTerminal(handle.token, {
      state,
      reason_code: code,
    });
    return Object.freeze({
      status: state === "blocked" ? "reference_fallback_required" : "failed",
      error_code: code,
      detail: message,
      terminal_transition: completion,
      bitmap: null,
      receipt: null,
    });
  }

  publish(handle, decoded) {
    if (handle.policy_generation !== this.policyGeneration) {
      closeQuietly(decoded.bitmap);
      const completion = this.readiness.completeTerminal(handle.token, {
        state: "blocked",
        reason_code: "decode_policy_generation_stale",
      });
      return Object.freeze({
        status: "stale",
        error_code: "decode_policy_generation_stale",
        terminal_transition: completion,
        bitmap: null,
        receipt: decoded.receipt,
      });
    }
    const ready = this.readiness.completeReady(handle.token, {
      binding_identity:
        "browser-image:" +
        [
          handle.identity.resource_id,
          handle.identity.content_hash,
          handle.identity.derivative_id,
          handle.token.request_generation,
          handle.token.backend_generation,
          handle.policy_generation,
        ].join(":"),
    });
    if (!ready.accepted) {
      closeQuietly(decoded.bitmap);
      return Object.freeze({
        status: "stale",
        error_code: "readiness_generation_stale",
        terminal_transition: ready,
        bitmap: null,
        receipt: decoded.receipt,
      });
    }
    return Object.freeze({
      status: "ready",
      bitmap: decoded.bitmap,
      receipt: decoded.receipt,
      readiness_transition: ready,
    });
  }

  async decodeExact({ identity, blob, reference_receipt }) {
    const handle = this.begin(identity);
    try {
      const decoded = await decodeBrowserBitmapExactV1({
        blob,
        identity: handle.identity,
        reference_receipt,
        request_generation: handle.token.request_generation,
        backend_generation: handle.token.backend_generation,
        policy_generation: handle.policy_generation,
        placement: this.placement,
        backend_id: this.backendId,
        createImageBitmapFn: this.createImageBitmapFn,
      });
      return this.publish(handle, decoded);
    } catch (error) {
      return this.#finishError(handle, error);
    }
  }
}
