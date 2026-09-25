import { BrowserImageDecodeAdapterV1 } from "./image-decode-v1.mjs";

self.onmessage = async (event) => {
  const { identity, reference, bytes } = event.data;
  try {
    const adapter = new BrowserImageDecodeAdapterV1({ runtime_generation: "worker-v1" });
    const token = adapter.begin(identity);
    const result = await adapter.decode(token, new Uint8Array(bytes), reference);
    if (!result.accepted) {
      self.postMessage({ ok: false, result });
      return;
    }
    const receipt = result.receipt;
    result.handle.close();
    self.postMessage({ ok: true, receipt, released: result.handle.released });
  } catch (error) {
    self.postMessage({ ok: false, error: String(error) });
  }
};
