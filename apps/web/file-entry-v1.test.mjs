import test from "node:test";
import assert from "node:assert/strict";

import {
  FILE_ENTRY_STATE_V1,
  WebFileEntryControllerV1,
  bindWebFileEntryV1,
  classifyPublisherSelection,
  isPublisherFile,
} from "./file-entry-v1.mjs";

function pub(name = "newsletter.pub", size = 1024) {
  return { name, size, type: "application/x-mspublisher" };
}

class FakeTarget {
  constructor() {
    this.handlers = new Map();
    this.clickCount = 0;
    this.files = [];
    this.value = "";
  }
  addEventListener(type, fn) {
    const list = this.handlers.get(type) ?? [];
    list.push(fn);
    this.handlers.set(type, list);
  }
  removeEventListener(type, fn) {
    const list = this.handlers.get(type) ?? [];
    this.handlers.set(type, list.filter((item) => item !== fn));
  }
  click() { this.clickCount += 1; }
  emit(type, event = {}) {
    for (const fn of this.handlers.get(type) ?? []) fn(event);
  }
}

function ingress(overrides = {}) {
  return {
    async beginUpload({ client_request_id }) {
      return { upload_id: "upload:" + client_request_id };
    },
    async uploadBytes({ file, onProgress }) {
      onProgress?.(file.size, file.size);
    },
    async completeUpload() {},
    async waitUntilValidated() { return { status: "validated" }; },
    async createProjectFromUpload({ client_request_id }) {
      return { project_id: "project:" + client_request_id, document_id: "doc:" + client_request_id };
    },
    ...overrides,
  };
}

test("Publisher file recognition is extension-bounded and selection routing is explicit", () => {
  assert.equal(isPublisherFile(pub("A.PUB")), true);
  assert.equal(isPublisherFile({ name: "a.pdf", size: 12, type: "application/pdf" }), false);
  assert.equal(classifyPublisherSelection([pub()]).kind, "single_pub");
  assert.equal(classifyPublisherSelection([pub("a.pub"), pub("b.pub")]).kind, "multiple_pub");
  assert.equal(classifyPublisherSelection([{ name: "a.pdf", size: 1 }]).kind, "unsupported");
});

test("controller exposes honest upload to validation to project-open phases", async () => {
  const phases = [];
  let opened = null;
  const controller = new WebFileEntryControllerV1({
    sourceIngress: ingress(),
    requestIdFactory: () => "request-0001",
    onState: (state) => phases.push(state.phase),
    openProject: async (value) => { opened = value; },
  });

  const state = await controller.openFile(pub());
  assert.equal(state.phase, FILE_ENTRY_STATE_V1.READY);
  assert.equal(state.bytes_sent, 1024);
  assert.equal(state.upload_id, "upload:request-0001");
  assert.equal(state.project_id, "project:request-0001");
  assert.equal(state.document_id, "doc:request-0001");
  assert.equal(opened.project_id, state.project_id);
  assert.deepEqual(
    [...new Set(phases)],
    ["uploading", "validating", "preparing_project", "opening", "ready"],
  );
});

test("retry preserves the same logical client request id", async () => {
  let uploadCalls = 0;
  const beginIds = [];
  const createIds = [];
  const controller = new WebFileEntryControllerV1({
    sourceIngress: ingress({
      async beginUpload({ client_request_id }) {
        beginIds.push(client_request_id);
        return { upload_id: "stable-upload" };
      },
      async uploadBytes({ file, onProgress }) {
        uploadCalls += 1;
        if (uploadCalls === 1) {
          const error = new Error("temporary network failure");
          error.code = "network_error";
          error.retryable = true;
          throw error;
        }
        onProgress(file.size, file.size);
      },
      async createProjectFromUpload({ client_request_id }) {
        createIds.push(client_request_id);
        return { project_id: "p1", document_id: "d1" };
      },
    }),
    requestIdFactory: () => "request-stable-01",
    openProject: async () => {},
  });

  const failed = await controller.openFile(pub());
  assert.equal(failed.phase, FILE_ENTRY_STATE_V1.ERROR);
  assert.equal(failed.error_code, "network_error");
  assert.equal(failed.retryable, true);

  const ready = await controller.retry();
  assert.equal(ready.phase, FILE_ENTRY_STATE_V1.READY);
  assert.deepEqual(beginIds, ["request-stable-01", "request-stable-01"]);
  assert.deepEqual(createIds, ["request-stable-01"]);
});

test("cancel aborts an in-flight upload and never creates a project", async () => {
  let createCalls = 0;
  const controller = new WebFileEntryControllerV1({
    sourceIngress: ingress({
      async uploadBytes({ signal }) {
        await new Promise((resolve, reject) => {
          if (signal.aborted) {
            const error = new Error("aborted");
            error.name = "AbortError";
            reject(error);
            return;
          }
          signal.addEventListener("abort", () => {
            const error = new Error("aborted");
            error.name = "AbortError";
            reject(error);
          }, { once: true });
        });
      },
      async createProjectFromUpload() {
        createCalls += 1;
        return { project_id: "p", document_id: "d" };
      },
    }),
    requestIdFactory: () => "request-cancel-01",
    openProject: async () => {},
  });

  const running = controller.openFile(pub());
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(controller.cancel(), true);
  const state = await running;
  assert.equal(state.phase, FILE_ENTRY_STATE_V1.CANCELLED);
  assert.equal(createCalls, 0);
});

test("unsupported file fails before SourceIngress", async () => {
  let began = 0;
  const controller = new WebFileEntryControllerV1({
    sourceIngress: ingress({ async beginUpload() { began += 1; return { upload_id: "x" }; } }),
    requestIdFactory: () => "request-nope-01",
    openProject: async () => {},
  });
  const state = await controller.openFile({ name: "not-pub.pdf", size: 10, type: "application/pdf" });
  assert.equal(state.phase, FILE_ENTRY_STATE_V1.ERROR);
  assert.equal(state.error_code, "unsupported_file_type");
  assert.equal(state.retryable, false);
  assert.equal(began, 0);
});

test("drop over an open project requires explicit open-as-new-project routing", async () => {
  const dropTarget = new FakeTarget();
  const input = new FakeTarget();
  const button = new FakeTarget();
  const keyboard = new FakeTarget();
  let opened = 0;
  let prompted = null;
  const binding = bindWebFileEntryV1({
    controller: { async openFile() { opened += 1; } },
    dropTarget,
    fileInput: input,
    openButton: button,
    keyboardTarget: keyboard,
    hasOpenProject: () => true,
    onOpenAsNewProject: (value) => { prompted = value; },
  });

  const result = await binding.handleSelection([pub("other.pub")], "drop");
  assert.equal(result.kind, "open_as_new_project_required");
  assert.equal(opened, 0);
  assert.equal(prompted.file.name, "other.pub");
  binding.destroy();
});

test("Ctrl/Cmd+O and Open button route to the same browser file picker", () => {
  const dropTarget = new FakeTarget();
  const input = new FakeTarget();
  const button = new FakeTarget();
  const keyboard = new FakeTarget();
  const binding = bindWebFileEntryV1({
    controller: { async openFile() {} },
    dropTarget,
    fileInput: input,
    openButton: button,
    keyboardTarget: keyboard,
  });

  button.emit("click", {});
  let prevented = false;
  keyboard.emit("keydown", {
    key: "o",
    ctrlKey: true,
    metaKey: false,
    altKey: false,
    isComposing: false,
    defaultPrevented: false,
    preventDefault() { prevented = true; },
  });
  assert.equal(input.clickCount, 2);
  assert.equal(prevented, true);
  binding.destroy();
});
