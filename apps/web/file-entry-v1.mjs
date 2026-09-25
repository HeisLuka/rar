export const FILE_ENTRY_STATE_V1 = Object.freeze({
  IDLE: "idle",
  UPLOADING: "uploading",
  VALIDATING: "validating",
  PREPARING_PROJECT: "preparing_project",
  OPENING: "opening",
  READY: "ready",
  CANCELLED: "cancelled",
  ERROR: "error",
});

export const FILE_ENTRY_LABEL_V1 = Object.freeze({
  [FILE_ENTRY_STATE_V1.IDLE]: "Open a Publisher file",
  [FILE_ENTRY_STATE_V1.UPLOADING]: "Uploading",
  [FILE_ENTRY_STATE_V1.VALIDATING]: "Validating",
  [FILE_ENTRY_STATE_V1.PREPARING_PROJECT]: "Preparing project",
  [FILE_ENTRY_STATE_V1.OPENING]: "Opening",
  [FILE_ENTRY_STATE_V1.READY]: "Ready",
  [FILE_ENTRY_STATE_V1.CANCELLED]: "Cancelled",
  [FILE_ENTRY_STATE_V1.ERROR]: "Needs attention",
});

function cloneState(value) {
  return value == null ? value : structuredClone(value);
}

function requireMethod(object, name) {
  if (!object || typeof object[name] !== "function") {
    throw new TypeError("sourceIngress must implement " + name + "()");
  }
}

function requireSourceIngress(sourceIngress) {
  for (const name of [
    "beginUpload",
    "uploadBytes",
    "completeUpload",
    "waitUntilValidated",
    "createProjectFromUpload",
  ]) {
    requireMethod(sourceIngress, name);
  }
}

function errorCode(error) {
  return typeof error?.code === "string" && error.code ? error.code : "file_entry_failed";
}

function aborted(error) {
  return error?.name === "AbortError" || error?.code === "aborted";
}

function publisherName(name) {
  return typeof name === "string" && /\.pub$/i.test(name.trim());
}

export function isPublisherFile(file) {
  return Boolean(
    file &&
    publisherName(file.name) &&
    Number.isSafeInteger(file.size) &&
    file.size >= 0
  );
}

export function classifyPublisherSelection(fileList) {
  const files = Array.from(fileList ?? []);
  if (files.length === 0) return { kind: "empty", files: [] };
  const pubs = files.filter(isPublisherFile);
  if (files.length === 1 && pubs.length === 1) {
    return { kind: "single_pub", file: pubs[0] };
  }
  if (pubs.length > 1) return { kind: "multiple_pub", files: pubs };
  return { kind: "unsupported", files };
}

export class WebFileEntryControllerV1 {
  constructor({
    sourceIngress,
    openProject,
    requestIdFactory = null,
    onState = null,
  }) {
    requireSourceIngress(sourceIngress);
    if (typeof openProject !== "function") throw new TypeError("openProject callback is required");
    this.sourceIngress = sourceIngress;
    this.openProject = openProject;
    this.requestIdFactory = requestIdFactory ?? (() => crypto.randomUUID());
    this.onState = onState;
    this._attempt = null;
    this._abort = null;
    this._active = false;
    this._state = Object.freeze({
      protocol_version: "chaptera.web-file-entry-state.v1",
      phase: FILE_ENTRY_STATE_V1.IDLE,
      label: FILE_ENTRY_LABEL_V1[FILE_ENTRY_STATE_V1.IDLE],
      file_name: null,
      bytes_total: 0,
      bytes_sent: 0,
      upload_id: null,
      project_id: null,
      document_id: null,
      error_code: null,
      retryable: false,
    });
  }

  state() {
    return cloneState(this._state);
  }

  async openFile(file) {
    if (!isPublisherFile(file)) {
      this._set({
        phase: FILE_ENTRY_STATE_V1.ERROR,
        file_name: file?.name ?? null,
        error_code: "unsupported_file_type",
        retryable: false,
      });
      return this.state();
    }
    if (this._active) throw new Error("file entry attempt already active");
    const clientRequestId = this.requestIdFactory();
    if (typeof clientRequestId !== "string" || clientRequestId.length < 8) {
      throw new TypeError("requestIdFactory must return a bounded string");
    }
    this._attempt = { file, client_request_id: clientRequestId };
    return this._runAttempt(this._attempt);
  }

  async retry() {
    if (!this._attempt) throw new Error("no file entry attempt to retry");
    if (this._active) throw new Error("file entry attempt already active");
    if (![FILE_ENTRY_STATE_V1.ERROR, FILE_ENTRY_STATE_V1.CANCELLED].includes(this._state.phase)) {
      throw new Error("file entry attempt is not retryable in phase " + this._state.phase);
    }
    if (this._state.phase === FILE_ENTRY_STATE_V1.ERROR && !this._state.retryable) {
      throw new Error("file entry failure is not retryable");
    }
    return this._runAttempt(this._attempt);
  }

  cancel() {
    if (!this._active || !this._abort) return false;
    this._abort.abort();
    return true;
  }

  async _runAttempt(attempt) {
    this._active = true;
    this._abort = new AbortController();
    const signal = this._abort.signal;
    const file = attempt.file;
    let uploadId = null;
    try {
      this._set({
        phase: FILE_ENTRY_STATE_V1.UPLOADING,
        file_name: file.name,
        bytes_total: file.size,
        bytes_sent: 0,
        upload_id: null,
        project_id: null,
        document_id: null,
        error_code: null,
        retryable: false,
      });

      const begun = await this.sourceIngress.beginUpload({
        client_request_id: attempt.client_request_id,
        file_name: file.name,
        byte_length: file.size,
        mime: typeof file.type === "string" ? file.type : "",
        signal,
      });
      uploadId = begun?.upload_id;
      if (typeof uploadId !== "string" || !uploadId) {
        throw Object.assign(new Error("source ingress did not return upload_id"), {
          code: "invalid_ingress_response",
          retryable: false,
        });
      }
      this._set({ upload_id: uploadId });

      await this.sourceIngress.uploadBytes({
        upload_id: uploadId,
        file,
        signal,
        onProgress: (bytesSent, bytesTotal = file.size) => {
          if (!Number.isFinite(bytesSent) || !Number.isFinite(bytesTotal)) return;
          const total = Math.max(0, Math.floor(bytesTotal));
          const sent = Math.max(0, Math.min(total, Math.floor(bytesSent)));
          this._set({ bytes_sent: sent, bytes_total: total });
        },
      });

      this._set({ phase: FILE_ENTRY_STATE_V1.VALIDATING, bytes_sent: file.size, bytes_total: file.size });
      await this.sourceIngress.completeUpload({ upload_id: uploadId, signal });
      const validation = await this.sourceIngress.waitUntilValidated({ upload_id: uploadId, signal });
      if (validation?.status !== "validated") {
        const failure = new Error("source upload did not validate");
        failure.code = validation?.code ?? "source_validation_failed";
        failure.retryable = validation?.retryable === true;
        throw failure;
      }

      this._set({ phase: FILE_ENTRY_STATE_V1.PREPARING_PROJECT });
      const project = await this.sourceIngress.createProjectFromUpload({
        upload_id: uploadId,
        client_request_id: attempt.client_request_id,
        signal,
      });
      if (typeof project?.project_id !== "string" || typeof project?.document_id !== "string") {
        throw Object.assign(new Error("source ingress did not return project/document identity"), {
          code: "invalid_project_response",
          retryable: false,
        });
      }

      this._set({
        phase: FILE_ENTRY_STATE_V1.OPENING,
        project_id: project.project_id,
        document_id: project.document_id,
      });
      await this.openProject({
        project_id: project.project_id,
        document_id: project.document_id,
        upload_id: uploadId,
        signal,
      });
      this._set({ phase: FILE_ENTRY_STATE_V1.READY, retryable: false });
      return this.state();
    } catch (error) {
      if (aborted(error) || signal.aborted) {
        this._set({
          phase: FILE_ENTRY_STATE_V1.CANCELLED,
          error_code: null,
          retryable: true,
        });
      } else {
        this._set({
          phase: FILE_ENTRY_STATE_V1.ERROR,
          upload_id: uploadId ?? this._state.upload_id,
          error_code: errorCode(error),
          retryable: error?.retryable !== false,
        });
      }
      return this.state();
    } finally {
      this._active = false;
      this._abort = null;
    }
  }

  _set(patch) {
    const phase = patch.phase ?? this._state.phase;
    this._state = Object.freeze({
      ...this._state,
      ...patch,
      phase,
      label: FILE_ENTRY_LABEL_V1[phase],
    });
    this.onState?.(this.state());
  }
}

export function bindWebFileEntryV1({
  controller,
  dropTarget,
  fileInput,
  openButton,
  keyboardTarget = globalThis.window ?? null,
  hasOpenProject = () => false,
  onOpenAsNewProject = null,
  onMultiplePub = null,
  onUnsupported = null,
}) {
  if (!controller || typeof controller.openFile !== "function") {
    throw new TypeError("controller.openFile() is required");
  }
  for (const [name, target] of [["dropTarget", dropTarget], ["fileInput", fileInput], ["openButton", openButton]]) {
    if (!target || typeof target.addEventListener !== "function") {
      throw new TypeError(name + " EventTarget is required");
    }
  }

  async function handleSelection(fileList, source = "picker") {
    const selection = classifyPublisherSelection(fileList);
    if (selection.kind === "single_pub") {
      if (hasOpenProject()) {
        onOpenAsNewProject?.({ file: selection.file, source });
        return { kind: "open_as_new_project_required", file: selection.file };
      }
      await controller.openFile(selection.file);
      return selection;
    }
    if (selection.kind === "multiple_pub") onMultiplePub?.({ files: selection.files, source });
    if (selection.kind === "unsupported") onUnsupported?.({ files: selection.files, source });
    return selection;
  }

  const onOpenClick = () => fileInput.click();
  const onInputChange = () => {
    const files = fileInput.files;
    Promise.resolve(handleSelection(files, "picker")).finally(() => {
      try { fileInput.value = ""; } catch {}
    });
  };
  const onDragOver = (event) => {
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = "copy";
  };
  const onDrop = (event) => {
    event.preventDefault();
    void handleSelection(event.dataTransfer?.files ?? [], "drop");
  };
  const onKeyDown = (event) => {
    if (event.defaultPrevented || event.isComposing) return;
    if ((event.ctrlKey || event.metaKey) && !event.altKey && String(event.key).toLowerCase() === "o") {
      event.preventDefault();
      fileInput.click();
    }
  };

  openButton.addEventListener("click", onOpenClick);
  fileInput.addEventListener("change", onInputChange);
  dropTarget.addEventListener("dragover", onDragOver);
  dropTarget.addEventListener("drop", onDrop);
  keyboardTarget?.addEventListener?.("keydown", onKeyDown);

  return {
    handleSelection,
    destroy() {
      openButton.removeEventListener("click", onOpenClick);
      fileInput.removeEventListener("change", onInputChange);
      dropTarget.removeEventListener("dragover", onDragOver);
      dropTarget.removeEventListener("drop", onDrop);
      keyboardTarget?.removeEventListener?.("keydown", onKeyDown);
    },
  };
}
