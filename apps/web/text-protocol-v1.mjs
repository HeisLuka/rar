const PROTOCOL_VERSION = "chaptera.story-range-intent.v1";
const HASH_ID_RE = /^sha256:[0-9a-f]{64}$/;
const SOURCE_HASH_RE = /^[0-9a-f]{64}$/;

function scalarLength(text) {
  return Array.from(text).length;
}

export function utf16OffsetToScalarIndex(text, utf16Offset) {
  if (typeof text !== "string") throw new TypeError("text must be string");
  if (!Number.isSafeInteger(utf16Offset) || utf16Offset < 0 || utf16Offset > text.length) {
    throw new RangeError("utf16Offset outside text");
  }
  let utf16 = 0;
  let scalar = 0;
  for (const ch of text) {
    if (utf16 === utf16Offset) return scalar;
    const width = ch.length;
    if (utf16Offset > utf16 && utf16Offset < utf16 + width) {
      throw new RangeError("utf16Offset splits a surrogate pair");
    }
    utf16 += width;
    scalar += 1;
  }
  if (utf16 === utf16Offset) return scalar;
  throw new RangeError("utf16Offset outside text");
}

export function scalarIndexToUtf16Offset(text, scalarIndex) {
  if (typeof text !== "string") throw new TypeError("text must be string");
  if (!Number.isSafeInteger(scalarIndex) || scalarIndex < 0) {
    throw new RangeError("scalarIndex must be non-negative integer");
  }
  let scalar = 0;
  let utf16 = 0;
  for (const ch of text) {
    if (scalar === scalarIndex) return utf16;
    scalar += 1;
    utf16 += ch.length;
  }
  if (scalar === scalarIndex) return utf16;
  throw new RangeError("scalarIndex outside text");
}

export function diffObservedTextToScalarReplacement(before, after) {
  if (typeof before !== "string" || typeof after !== "string") {
    throw new TypeError("before/after must be strings");
  }
  let prefix = 0;
  while (
    prefix < before.length &&
    prefix < after.length &&
    before[prefix] === after[prefix]
  ) prefix += 1;

  while (
    prefix > 0 &&
    prefix < before.length &&
    /[\uDC00-\uDFFF]/.test(before[prefix]) &&
    /[\uD800-\uDBFF]/.test(before[prefix - 1])
  ) prefix -= 1;

  let beforeEnd = before.length;
  let afterEnd = after.length;
  while (
    beforeEnd > prefix &&
    afterEnd > prefix &&
    before[beforeEnd - 1] === after[afterEnd - 1]
  ) {
    beforeEnd -= 1;
    afterEnd -= 1;
  }

  if (
    beforeEnd > prefix &&
    beforeEnd < before.length &&
    /[\uDC00-\uDFFF]/.test(before[beforeEnd]) &&
    /[\uD800-\uDBFF]/.test(before[beforeEnd - 1])
  ) beforeEnd -= 1;
  if (
    afterEnd > prefix &&
    afterEnd < after.length &&
    /[\uDC00-\uDFFF]/.test(after[afterEnd]) &&
    /[\uD800-\uDBFF]/.test(after[afterEnd - 1])
  ) afterEnd -= 1;

  return {
    start_scalar: utf16OffsetToScalarIndex(before, prefix),
    end_scalar: utf16OffsetToScalarIndex(before, beforeEnd),
    replacement_text: after.slice(prefix, afterEnd),
  };
}

export function validateStoryRangeIntentV1(value) {
  if (!value || typeof value !== "object") throw new TypeError("intent object required");
  if (value.protocol_version !== PROTOCOL_VERSION) throw new TypeError("unsupported protocol_version");
  if (typeof value.document_id !== "string" || !value.document_id) throw new TypeError("document_id required");
  if (!SOURCE_HASH_RE.test(value.source_hash ?? "")) throw new TypeError("source_hash must be lowercase SHA-256");
  if (!HASH_ID_RE.test(value.base_revision_id ?? "")) throw new TypeError("base_revision_id required");
  if (typeof value.client_operation_id !== "string" || value.client_operation_id.length < 8 || value.client_operation_id.length > 160) {
    throw new TypeError("client_operation_id must be bounded");
  }
  if (value.depends_on_client_operation_id != null &&
      (typeof value.depends_on_client_operation_id !== "string" ||
       value.depends_on_client_operation_id.length < 8 ||
       value.depends_on_client_operation_id.length > 160)) {
    throw new TypeError("depends_on_client_operation_id must be bounded or null");
  }
  const command = value.command;
  if (!command || command.kind !== "replace_story_range") throw new TypeError("replace_story_range command required");
  if (typeof command.story_id !== "string" || !command.story_id) throw new TypeError("story_id required");
  if (!Number.isSafeInteger(command.start_scalar) || command.start_scalar < 0) throw new TypeError("start_scalar invalid");
  if (!Number.isSafeInteger(command.end_scalar) || command.end_scalar < command.start_scalar) throw new TypeError("end_scalar invalid");
  if (typeof command.replacement_text !== "string") throw new TypeError("replacement_text required");
  return structuredClone(value);
}

export class BrowserTextSessionV1 {
  constructor({
    documentId,
    sourceHash,
    storyId,
    baseRevisionId,
    text,
    operationIdFactory,
  }) {
    if (typeof operationIdFactory !== "function") throw new TypeError("operationIdFactory required");
    this.documentId = documentId;
    this.sourceHash = sourceHash;
    this.storyId = storyId;
    this.baseRevisionId = baseRevisionId;
    this.text = text;
    this.operationIdFactory = operationIdFactory;
    this.pending = [];
    this.composition = null;
  }

  beginComposition() {
    if (this.composition) throw new Error("composition already active");
    this.composition = {before: this.text};
  }

  updateComposition(provisionalText) {
    if (!this.composition) throw new Error("composition not active");
    if (typeof provisionalText !== "string") throw new TypeError("provisionalText must be string");
    this.composition.provisional = provisionalText;
    return null;
  }

  endComposition(finalText) {
    if (!this.composition) throw new Error("composition not active");
    const before = this.composition.before;
    this.composition = null;
    if (finalText === before) return null;
    return this.intentFromObservedReplacement(before, finalText);
  }

  intentFromObservedReplacement(before, after) {
    if (before !== this.text) {
      throw new Error("observed before-text does not match BrowserTextSession state");
    }
    const replacement = diffObservedTextToScalarReplacement(before, after);
    return this.createIntent(replacement, after);
  }

  createIntent({start_scalar, end_scalar, replacement_text}, optimisticText = null) {
    const scalarCount = scalarLength(this.text);
    if (!Number.isSafeInteger(start_scalar) || !Number.isSafeInteger(end_scalar) ||
        start_scalar < 0 || end_scalar < start_scalar || end_scalar > scalarCount) {
      throw new RangeError("canonical scalar range outside current Story");
    }
    if (typeof replacement_text !== "string") throw new TypeError("replacement_text must be string");
    const clientOperationId = this.operationIdFactory();
    const depends = this.pending.length ? this.pending[this.pending.length - 1].client_operation_id : null;
    const request = {
      protocol_version: PROTOCOL_VERSION,
      document_id: this.documentId,
      source_hash: this.sourceHash,
      base_revision_id: this.baseRevisionId,
      client_operation_id: clientOperationId,
      depends_on_client_operation_id: depends,
      command: {
        kind: "replace_story_range",
        story_id: this.storyId,
        start_scalar,
        end_scalar,
        replacement_text,
      },
    };
    validateStoryRangeIntentV1(request);
    this.pending.push(request);
    if (optimisticText != null) this.text = optimisticText;
    return structuredClone(request);
  }

  accept(clientOperationId, {revision_id, canonical_story_text}) {
    const index = this.pending.findIndex((item) => item.client_operation_id === clientOperationId);
    if (index < 0) throw new Error("unknown pending operation");
    if (index !== 0) throw new Error("cannot accept out of causal order");
    if (!HASH_ID_RE.test(revision_id ?? "")) throw new TypeError("accepted revision_id required");
    if (typeof canonical_story_text !== "string") throw new TypeError("canonical_story_text required");
    this.pending.shift();
    this.baseRevisionId = revision_id;
    this.text = canonical_story_text;
    return {status: "accepted", remaining_pending: this.pending.length};
  }

  reject(clientOperationId, {code, current_revision_id = null}) {
    const index = this.pending.findIndex((item) => item.client_operation_id === clientOperationId);
    if (index < 0) throw new Error("unknown pending operation");
    const rejected = this.pending[index];
    const invalidated = this.pending.splice(index);
    if (current_revision_id != null && !HASH_ID_RE.test(current_revision_id)) {
      throw new TypeError("current_revision_id invalid");
    }
    if (current_revision_id) this.baseRevisionId = current_revision_id;
    return {
      status: "rejected",
      code,
      rejected_client_operation_id: rejected.client_operation_id,
      invalidated_client_operation_ids: invalidated.map((x) => x.client_operation_id),
      requires_refresh: true,
    };
  }
}

export { PROTOCOL_VERSION as STORY_RANGE_INTENT_PROTOCOL_V1 };
