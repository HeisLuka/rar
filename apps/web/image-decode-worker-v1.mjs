import {
  decodeBrowserBitmapExactV1,
} from "./image-decode-v1.mjs";

self.onmessage = async (event) => {
  const message = event.data ?? {};
  if (message.kind !== "decode") return;
  try {
    const blob = new Blob([message.bytes], { type: message.mime_type });
    const decoded = await decodeBrowserBitmapExactV1({
      blob,
      identity: message.identity,
      reference_receipt: message.reference_receipt,
      request_generation: message.request_generation,
      backend_generation: message.backend_generation,
      policy_generation: message.policy_generation,
      placement: "worker",
      backend_id: message.backend_id ?? "browser-worker",
    });
    self.postMessage(
      {
        kind: "decoded",
        request_id: message.request_id,
        receipt: decoded.receipt,
        bitmap: decoded.bitmap,
      },
      [decoded.bitmap],
    );
  } catch (error) {
    self.postMessage({
      kind: "decode_error",
      request_id: message.request_id,
      error_code: error?.code ?? "browser_decode_failed",
      detail: error?.message ?? String(error),
    });
  }
};
