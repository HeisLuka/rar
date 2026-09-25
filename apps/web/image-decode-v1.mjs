export const BROWSER_IMAGE_DECODE_SCHEMA_V1 = "chaptera.browser-image-decode.v1";

export const EXACT_CREATE_IMAGE_BITMAP_OPTIONS_V1 = Object.freeze({
  imageOrientation: "none",
  premultiplyAlpha: "none",
  colorSpaceConversion: "none",
});

const SHA256_HEX = /^[0-9a-f]{64}$/;

function assertIdentity(identity) {
  if (!identity || typeof identity !== "object") throw new Error("image decode identity is required");
  for (const key of ["resource_id", "derivative_id", "content_hash", "mime_type"]) {
    if (typeof identity[key] !== "string" || identity[key].length === 0) {
      throw new Error(`${key} is required`);
    }
  }
  if (!SHA256_HEX.test(identity.content_hash)) throw new Error("content_hash must be lowercase SHA-256");
  if (!["image/png", "image/jpeg"].includes(identity.mime_type)) {
    throw new Error("browser image decode V1 admits only PNG/JPEG");
  }
}

function identityKey(identity) {
  return [identity.resource_id, identity.derivative_id, identity.content_hash, identity.mime_type].join("\u001f");
}

function assertReference(reference, identity) {
  if (!reference || reference.contract_version !== "chaptera.image-decode-contract.v1") {
    throw new Error("reference decode receipt must be chaptera.image-decode-contract.v1");
  }
  if (reference.unsupported_or_error_code != null) {
    throw new Error("reference decode receipt is unsupported");
  }
  if (reference.resource_sha256 !== identity.content_hash) {
    throw new Error("reference resource hash mismatch");
  }
  if (reference.mime_type !== identity.mime_type) throw new Error("reference mime mismatch");
  if (reference.orientation_applied !== false) {
    throw new Error("reference decode must preserve EXIF orientation as metadata only");
  }
  for (const name of ["encoded_dimensions", "decoded_dimensions"]) {
    if (!Array.isArray(reference[name]) || reference[name].length !== 2 ||
        reference[name].some((value) => !Number.isInteger(value) || value <= 0)) {
      throw new Error(`invalid reference ${name}`);
    }
  }
}

async function sha256Hex(blob) {
  const bytes = await blob.arrayBuffer();
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function asBlob(bytes, mimeType) {
  if (bytes instanceof Blob) return bytes;
  if (bytes instanceof ArrayBuffer) return new Blob([bytes], { type: mimeType });
  if (ArrayBuffer.isView(bytes)) {
    return new Blob([bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength)], { type: mimeType });
  }
  throw new Error("decode bytes must be Blob, ArrayBuffer, or typed array");
}

export class BrowserImageDecodeAdapterV1 {
  constructor({ createImageBitmapImpl = globalThis.createImageBitmap, runtime_generation = "browser-runtime-v1" } = {}) {
    this.createImageBitmapImpl = createImageBitmapImpl;
    this.runtimeGeneration = runtime_generation;
    this.generations = new Map();
  }

  begin(identity) {
    assertIdentity(identity);
    const key = identityKey(identity);
    const generation = (this.generations.get(key) ?? 0) + 1;
    this.generations.set(key, generation);
    return Object.freeze({ key, generation, identity: Object.freeze({ ...identity }) });
  }

  supersede(identity) {
    assertIdentity(identity);
    const key = identityKey(identity);
    const generation = (this.generations.get(key) ?? 0) + 1;
    this.generations.set(key, generation);
    return generation;
  }

  isCurrent(token) {
    return this.generations.get(token.key) === token.generation;
  }

  async decode(token, bytes, reference) {
    if (!token || !token.identity) throw new Error("decode token is required");
    assertIdentity(token.identity);
    assertReference(reference, token.identity);
    if (!this.isCurrent(token)) return { accepted: false, reason: "stale_generation_before_decode" };
    if (typeof this.createImageBitmapImpl !== "function") {
      return {
        accepted: false,
        reason: "exact_browser_decode_unsupported",
        error_code: "create_image_bitmap_unavailable",
      };
    }

    const blob = asBlob(bytes, token.identity.mime_type);
    const actualHash = await sha256Hex(blob);
    if (actualHash !== token.identity.content_hash) {
      return { accepted: false, reason: "resource_hash_mismatch" };
    }

    const started = performance.now();
    let bitmap;
    try {
      bitmap = await this.createImageBitmapImpl(blob, EXACT_CREATE_IMAGE_BITMAP_OPTIONS_V1);
    } catch (error) {
      return {
        accepted: false,
        reason: "exact_browser_decode_unsupported",
        error_code: "create_image_bitmap_exact_policy_failed",
        detail: String(error),
      };
    }
    const decodeMs = performance.now() - started;

    if (!this.isCurrent(token)) {
      if (typeof bitmap.close === "function") bitmap.close();
      return { accepted: false, reason: "stale_generation_after_decode", released: true };
    }

    const expected = reference.decoded_dimensions;
    if (bitmap.width !== expected[0] || bitmap.height !== expected[1]) {
      if (typeof bitmap.close === "function") bitmap.close();
      return {
        accepted: false,
        reason: "decoded_dimensions_mismatch",
        expected_dimensions: [...expected],
        browser_dimensions: [bitmap.width, bitmap.height],
        released: true,
      };
    }

    let released = false;
    const handle = {
      bitmap,
      close() {
        if (!released) {
          if (typeof bitmap.close === "function") bitmap.close();
          released = true;
        }
      },
      get released() {
        return released;
      },
    };

    return {
      accepted: true,
      handle,
      receipt: Object.freeze({
        schema: BROWSER_IMAGE_DECODE_SCHEMA_V1,
        resource_id: token.identity.resource_id,
        derivative_id: token.identity.derivative_id,
        resource_sha256: token.identity.content_hash,
        mime_type: token.identity.mime_type,
        request_generation: token.generation,
        runtime_generation: this.runtimeGeneration,
        decoder_api: "createImageBitmap",
        orientation_policy: "preserve_metadata_do_not_apply",
        alpha_policy: "straight_unassociated_at_decode_boundary",
        color_conversion_policy: "none",
        resize_policy: "no_decode_time_resize",
        frame_policy: "single_static_frame_only",
        create_image_bitmap_options: { ...EXACT_CREATE_IMAGE_BITMAP_OPTIONS_V1 },
        reference_decoder_id: reference.decoder_id,
        reference_exif_orientation: reference.exif_orientation,
        reference_orientation_applied: reference.orientation_applied,
        reference_alpha_association: reference.alpha_association,
        encoded_dimensions: [...reference.encoded_dimensions],
        decoded_dimensions: [bitmap.width, bitmap.height],
        decode_ms: decodeMs,
        browser_default_decode_used: false,
        exact_policy_requested: true,
      }),
    };
  }
}
