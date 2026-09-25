const IMAGE_MIME = new Set(["image/png", "image/jpeg"]);

function clone(value) {
  return value == null ? value : structuredClone(value);
}

function requireMethod(object, name, label) {
  if (!object || typeof object[name] !== "function") {
    throw new TypeError(label + "." + name + "() is required");
  }
}

function extensionIsImage(name) {
  return typeof name === "string" && /\.(png|jpe?g)$/i.test(name.trim());
}

export function isSupportedCanvasImageFile(file) {
  if (!file || typeof file !== "object") return false;
  if (!Number.isSafeInteger(file.size) || file.size < 0) return false;
  if (IMAGE_MIME.has(String(file.type || "").toLowerCase())) return true;
  return extensionIsImage(file.name);
}

export function classifyCanvasImageFiles(fileList) {
  const files = Array.from(fileList ?? []);
  const images = files.filter(isSupportedCanvasImageFile);
  if (images.length === 0) return { kind: "none", files };
  if (images.length === 1 && files.length === 1) return { kind: "single_image", file: images[0] };
  if (images.length > 1) return { kind: "multiple_images", files: images };
  return { kind: "mixed", files, images };
}

export class WebAssetDropUxV1 {
  constructor({
    assetService,
    pictureService,
    interactionContext,
    requestIdFactory = null,
    onState = null,
  }) {
    for (const name of ["importImageAsset"]) requireMethod(assetService, name, "assetService");
    for (const name of ["planCreatePictureFrame", "createPictureFrame", "replaceImage"]) {
      requireMethod(pictureService, name, "pictureService");
    }
    for (const name of ["currentContext"]) requireMethod(interactionContext, name, "interactionContext");
    this.assetService = assetService;
    this.pictureService = pictureService;
    this.interactionContext = interactionContext;
    this.requestIdFactory = requestIdFactory ?? (() => crypto.randomUUID());
    this.onState = onState;
    this._attempt = null;
    this._state = Object.freeze({
      protocol_version: "chaptera.web-asset-drop-ux.v1",
      phase: "idle",
      route: null,
      file_name: null,
      asset_id: null,
      node_id: null,
      error_code: null,
      retryable: false,
    });
  }

  state() { return clone(this._state); }

  async handleFile(file, { document_point = null, source = "drop" } = {}) {
    if (!isSupportedCanvasImageFile(file)) {
      this._set({phase:"error",route:null,file_name:file?.name ?? null,error_code:"unsupported_image_type",retryable:false});
      return this.state();
    }
    const context = this.interactionContext.currentContext();
    this._assertSceneContext(context);
    const requestId = this._requestId();
    const attempt = {
      file,
      document_point: document_point == null ? null : clone(document_point),
      source,
      request_id: requestId,
      context: clone(context),
    };
    this._attempt = attempt;
    return this._run(attempt);
  }

  async retry() {
    if (!this._attempt) throw new Error("no image attempt to retry");
    if (this._state.phase !== "error" || this._state.retryable !== true) {
      throw new Error("image attempt is not retryable");
    }
    return this._run(this._attempt);
  }

  async _run(attempt) {
    try {
      this._set({
        phase:"importing",
        route:null,
        file_name:attempt.file.name ?? null,
        asset_id:null,
        node_id:null,
        error_code:null,
        retryable:false,
      });
      const asset = await this.assetService.importImageAsset({
        file: attempt.file,
        client_request_id: attempt.request_id,
      });
      this._validateAsset(asset);
      this._set({asset_id:asset.asset_id});

      const current = this.interactionContext.currentContext();
      this._assertSceneContext(current);
      if (current.document_id !== attempt.context.document_id) {
        throw Object.assign(new Error("document changed during image import"), {code:"document_context_changed", retryable:false});
      }

      if (current.selection?.node_id && current.capabilities?.can_replace_image === true) {
        this._set({phase:"committing",route:"replace_image",node_id:current.selection.node_id});
        const result = await this.pictureService.replaceImage({
          document_id: current.document_id,
          base_revision_id: current.base_revision_id,
          node_id: current.selection.node_id,
          asset_id: asset.asset_id,
          intrinsic_width_px: asset.intrinsic_width_px,
          intrinsic_height_px: asset.intrinsic_height_px,
          client_operation_id: attempt.request_id,
        });
        this._validateCommitResult(result, current.document_id);
        this._set({phase:"ready",route:"replace_image",node_id:current.selection.node_id,retryable:false});
        return this.state();
      }

      if (current.capabilities?.can_create_picture !== true) {
        throw Object.assign(new Error("picture creation is not admitted"), {code:"create_picture_not_admitted", retryable:false});
      }
      if (!current.page_id) {
        throw Object.assign(new Error("page target required"), {code:"page_target_required", retryable:false});
      }
      if (!attempt.document_point) {
        throw Object.assign(new Error("document point required for create"), {code:"document_point_required", retryable:false});
      }

      this._set({phase:"planning",route:"create_picture"});
      const plan = await this.pictureService.planCreatePictureFrame({
        document_id: current.document_id,
        base_revision_id: current.base_revision_id,
        page_id: current.page_id,
        asset_id: asset.asset_id,
        intrinsic_width_px: asset.intrinsic_width_px,
        intrinsic_height_px: asset.intrinsic_height_px,
        document_point: clone(attempt.document_point),
      });
      this._validatePlan(plan, current);

      this._set({phase:"committing",route:"create_picture",node_id:plan.node_id});
      const result = await this.pictureService.createPictureFrame({
        document_id: current.document_id,
        base_revision_id: current.base_revision_id,
        page_id: current.page_id,
        node_id: plan.node_id,
        asset_id: asset.asset_id,
        intrinsic_width_px: asset.intrinsic_width_px,
        intrinsic_height_px: asset.intrinsic_height_px,
        frame: clone(plan.frame),
        client_operation_id: attempt.request_id,
      });
      this._validateCommitResult(result, current.document_id);
      this._set({phase:"ready",route:"create_picture",node_id:plan.node_id,retryable:false});
      return this.state();
    } catch (error) {
      this._set({
        phase:"error",
        error_code:error?.code ?? "asset_drop_failed",
        retryable:error?.retryable === true,
      });
      return this.state();
    }
  }

  _assertSceneContext(context) {
    if (!context || typeof context !== "object") throw new TypeError("interaction context required");
    if (context.focus_owner !== "scene") {
      throw Object.assign(new Error("image gesture belongs to another focus owner"), {code:"focus_owner_not_scene", retryable:false});
    }
    for (const key of ["document_id","base_revision_id"]) {
      if (typeof context[key] !== "string" || !context[key]) throw new TypeError(key + " required");
    }
  }

  _validateAsset(asset) {
    if (!asset || typeof asset !== "object") throw new TypeError("asset import result required");
    if (typeof asset.asset_id !== "string" || !asset.asset_id) throw new TypeError("asset_id required");
    if (!Number.isSafeInteger(asset.intrinsic_width_px) || asset.intrinsic_width_px <= 0) throw new TypeError("intrinsic_width_px required");
    if (!Number.isSafeInteger(asset.intrinsic_height_px) || asset.intrinsic_height_px <= 0) throw new TypeError("intrinsic_height_px required");
  }

  _validatePlan(plan, current) {
    if (!plan || typeof plan !== "object") throw new TypeError("picture placement plan required");
    if (typeof plan.node_id !== "string" || !plan.node_id) throw new TypeError("plan node_id required");
    if (plan.page_id !== current.page_id) throw Object.assign(new Error("placement plan page mismatch"), {code:"placement_page_mismatch", retryable:false});
    const f = plan.frame;
    if (!f || !["x","y","width","height"].every((k)=>Number.isSafeInteger(f[k]))) throw new TypeError("exact frame required");
    if (!(f.width > 0 && f.height > 0)) throw new RangeError("picture frame must be positive");
  }

  _validateCommitResult(result, documentId) {
    if (!result || typeof result !== "object") throw new TypeError("commit result required");
    if (result.document_id !== documentId) throw Object.assign(new Error("commit document mismatch"), {code:"commit_document_mismatch", retryable:false});
    if (typeof result.revision_id !== "string" || !result.revision_id) throw new TypeError("revision_id required");
  }

  _requestId() {
    const id = this.requestIdFactory("image");
    if (typeof id !== "string" || id.length < 8) throw new TypeError("request id must be bounded string");
    return id;
  }

  _set(patch) {
    this._state = Object.freeze({...this._state, ...patch});
    this.onState?.(this.state());
  }
}

export function bindCanvasImageDropPasteV1({
  controller,
  canvasTarget,
  clipboardTarget = globalThis.window ?? null,
  screenToDocumentPoint,
  getFocusOwner,
  getPasteDocumentPoint = null,
  onMultipleImages = null,
  onUnsupported = null,
}) {
  if (!controller || typeof controller.handleFile !== "function") throw new TypeError("controller.handleFile() required");
  if (!canvasTarget || typeof canvasTarget.addEventListener !== "function") throw new TypeError("canvasTarget EventTarget required");
  if (typeof screenToDocumentPoint !== "function") throw new TypeError("screenToDocumentPoint() required");
  if (typeof getFocusOwner !== "function") throw new TypeError("getFocusOwner() required");

  const onDragOver = (event) => {
    const selection = classifyCanvasImageFiles(event.dataTransfer?.files ?? []);
    if (selection.kind === "single_image") {
      event.preventDefault();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "copy";
    }
  };

  const onDrop = (event) => {
    const selection = classifyCanvasImageFiles(event.dataTransfer?.files ?? []);
    if (selection.kind === "single_image") {
      event.preventDefault();
      const documentPoint = screenToDocumentPoint({x_css_px:event.clientX, y_css_px:event.clientY});
      void controller.handleFile(selection.file,{document_point:documentPoint,source:"drop"});
      return;
    }
    if (selection.kind === "multiple_images") onMultipleImages?.(selection.files);
    else if (selection.kind !== "none") onUnsupported?.(selection);
  };

  const onPaste = (event) => {
    if (getFocusOwner() !== "scene" || event.isComposing) return;
    const files = [];
    for (const item of Array.from(event.clipboardData?.items ?? [])) {
      if (item.kind === "file") {
        const file = item.getAsFile?.();
        if (file) files.push(file);
      }
    }
    const selection = classifyCanvasImageFiles(files);
    if (selection.kind === "single_image") {
      event.preventDefault();
      const documentPoint = typeof getPasteDocumentPoint === "function" ? getPasteDocumentPoint() : null;
      void controller.handleFile(selection.file,{document_point:documentPoint,source:"paste"});
      return;
    }
    if (selection.kind === "multiple_images") onMultipleImages?.(selection.files);
  };

  canvasTarget.addEventListener("dragover", onDragOver);
  canvasTarget.addEventListener("drop", onDrop);
  clipboardTarget?.addEventListener?.("paste", onPaste);
  return {
    destroy() {
      canvasTarget.removeEventListener("dragover", onDragOver);
      canvasTarget.removeEventListener("drop", onDrop);
      clipboardTarget?.removeEventListener?.("paste", onPaste);
    },
  };
}
