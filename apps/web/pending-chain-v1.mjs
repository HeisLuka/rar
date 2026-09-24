const HASH_ID_RE = /^sha256:[0-9a-f]{64}$/;
const CHAIN_SCHEMA = "chaptera.pending-chain.v1";

export class PendingBackpressure extends Error {}
export class PendingIdentityConflict extends Error {}
export class PendingRejected extends Error {}

function clone(value) {
  return structuredClone(value);
}

function canonicalize(value) {
  if (value === null || typeof value === "string" || typeof value === "boolean") return value;
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new TypeError("non-finite number");
    return value;
  }
  if (Array.isArray(value)) return value.map(canonicalize);
  if (value && typeof value === "object") {
    const out = {};
    for (const key of Object.keys(value).sort()) {
      if (value[key] === undefined) throw new TypeError("undefined is not JSON-safe");
      out[key] = canonicalize(value[key]);
    }
    return out;
  }
  throw new TypeError("value must be JSON-safe");
}

function stableJson(value) {
  return JSON.stringify(canonicalize(value));
}

function requireString(value, label, min = 1, max = 256) {
  if (typeof value !== "string" || value.length < min || value.length > max) {
    throw new TypeError(label + " must be a bounded string");
  }
}

function requireRevision(value, label = "revision_id") {
  if (!HASH_ID_RE.test(value ?? "")) throw new TypeError(label + " must be sha256 id");
}

function byteLength(value) {
  return new TextEncoder().encode(stableJson(value)).length;
}

function outcomeKey(outcome) {
  return stableJson(outcome);
}

function assertOutcome(outcome) {
  if (!outcome || typeof outcome !== "object") throw new TypeError("outcome required");
  if (outcome.status === "accepted") {
    requireRevision(outcome.revision_id);
    return;
  }
  if (outcome.status === "rejected" || outcome.status === "conflict") {
    requireString(outcome.code, "outcome code");
    if (outcome.current_revision_id != null) {
      requireRevision(outcome.current_revision_id, "current_revision_id");
    }
    return;
  }
  throw new TypeError("unsupported outcome status");
}

export class PendingChainV1 {
  constructor({
    canonicalRevisionId,
    sessionIncarnation,
    limits = {},
    now = () => Date.now(),
  }) {
    requireRevision(canonicalRevisionId, "canonicalRevisionId");
    requireString(sessionIncarnation, "sessionIncarnation");
    if (typeof now !== "function") throw new TypeError("now function required");

    this.schema = CHAIN_SCHEMA;
    this.canonicalRevisionId = canonicalRevisionId;
    this.sessionIncarnation = sessionIncarnation;
    this.now = now;
    this.limits = Object.freeze({
      max_count: limits.max_count ?? 20,
      max_bytes: limits.max_bytes ?? 256 * 1024,
      max_age_ms: limits.max_age_ms ?? 30_000,
      max_causal_depth: limits.max_causal_depth ?? 20,
    });
    for (const [name, value] of Object.entries(this.limits)) {
      if (!Number.isSafeInteger(value) || value <= 0) {
        throw new TypeError(name + " must be a positive safe integer");
      }
    }

    this.entries = [];
    this.byId = new Map();
    this.nextSequence = 1;
  }

  enqueue({clientOperationId, intent}) {
    requireString(clientOperationId, "clientOperationId", 8, 160);
    if (!intent || typeof intent !== "object") throw new TypeError("intent object required");

    const normalizedIntent = canonicalize(intent);
    const fingerprint = stableJson({
      session_incarnation: this.sessionIncarnation,
      client_operation_id: clientOperationId,
      intent: normalizedIntent,
    });
    const prior = this.byId.get(clientOperationId);
    if (prior) {
      if (prior.intent_fingerprint !== fingerprint) {
        throw new PendingIdentityConflict("idempotency_conflict");
      }
      return clone(prior);
    }

    this.#assertExistingAge();

    const predecessor = this.entries.at(-1) ?? null;
    const causalBase = predecessor
      ? {kind: "pending_after", client_operation_id: predecessor.client_operation_id}
      : {kind: "canonical_revision", revision_id: this.canonicalRevisionId};

    const createdAtMs = this.now();
    if (!Number.isFinite(createdAtMs)) throw new TypeError("clock must return finite milliseconds");

    const entry = {
      schema: CHAIN_SCHEMA,
      client_operation_id: clientOperationId,
      session_incarnation: this.sessionIncarnation,
      client_sequence: this.nextSequence,
      causal_base: causalBase,
      intent: normalizedIntent,
      intent_fingerprint: fingerprint,
      created_at_ms: createdAtMs,
      state: "pending",
      outcome: null,
      blocked_reason: null,
    };
    const encodedBytes = byteLength(entry);
    entry.encoded_bytes = encodedBytes;

    this.#assertAdmission(entry);

    this.nextSequence += 1;
    this.entries.push(entry);
    this.byId.set(clientOperationId, entry);
    return clone(entry);
  }

  requestFor(clientOperationId) {
    const entry = this.#required(clientOperationId);
    if (entry.state === "blocked" || entry.state === "reresolution_required") {
      throw new PendingRejected("operation_not_dispatchable");
    }
    return {
      schema: CHAIN_SCHEMA,
      client_operation_id: entry.client_operation_id,
      session_incarnation: entry.session_incarnation,
      client_sequence: entry.client_sequence,
      causal_base: clone(entry.causal_base),
      intent: clone(entry.intent),
    };
  }

  recordOutcome(clientOperationId, outcome) {
    assertOutcome(outcome);
    const entry = this.#required(clientOperationId);
    const normalized = canonicalize(outcome);

    if (entry.outcome != null) {
      if (outcomeKey(entry.outcome) !== outcomeKey(normalized)) {
        throw new PendingIdentityConflict("outcome_conflict");
      }
      return this.snapshot();
    }

    entry.outcome = normalized;
    entry.state = normalized.status === "accepted" ? "accepted" : "blocked";

    if (normalized.status === "accepted") {
      if (this.entries[0] !== entry) {
        throw new PendingIdentityConflict("predecessor_outcome_missing");
      }
      this.canonicalRevisionId = normalized.revision_id;
      this.#removeResolvedHead();
    } else {
      entry.blocked_reason = normalized.code;
      this.#blockDependents(entry.client_operation_id, normalized.status);
    }
    return this.snapshot();
  }

  observeCanonicalAdvance(revisionId) {
    requireRevision(revisionId);
    if (revisionId === this.canonicalRevisionId) return this.snapshot();

    this.canonicalRevisionId = revisionId;
    if (this.entries.length > 0) {
      const head = this.entries[0];
      if (head.outcome == null) {
        head.state = "reresolution_required";
        head.blocked_reason = "canonical_advanced_before_predecessor";
        this.#blockDependents(head.client_operation_id, "reresolution_required");
      }
    }
    return this.snapshot();
  }

  snapshot() {
    return {
      schema: CHAIN_SCHEMA,
      canonical_revision_id: this.canonicalRevisionId,
      session_incarnation: this.sessionIncarnation,
      limits: clone(this.limits),
      next_sequence: this.nextSequence,
      pending: this.entries.map(clone),
      totals: {
        count: this.entries.length,
        bytes: this.entries.reduce((sum, entry) => sum + entry.encoded_bytes, 0),
        causal_depth: this.entries.length,
      },
    };
  }

  static restore(snapshot, {now = () => Date.now()} = {}) {
    if (!snapshot || snapshot.schema !== CHAIN_SCHEMA) throw new TypeError("pending chain snapshot required");
    const chain = new PendingChainV1({
      canonicalRevisionId: snapshot.canonical_revision_id,
      sessionIncarnation: snapshot.session_incarnation,
      limits: snapshot.limits,
      now,
    });
    chain.nextSequence = snapshot.next_sequence;
    chain.entries = snapshot.pending.map(clone);
    chain.byId = new Map(chain.entries.map((entry) => [entry.client_operation_id, entry]));
    chain.#validateRestored();
    return chain;
  }

  #removeResolvedHead() {
    while (this.entries.length > 0 && this.entries[0].state === "accepted") {
      const resolved = this.entries.shift();
      this.byId.delete(resolved.client_operation_id);
    }
  }

  #blockDependents(predecessorId, reason) {
    let blocking = false;
    for (const entry of this.entries) {
      if (entry.client_operation_id === predecessorId) {
        blocking = true;
        continue;
      }
      if (blocking && entry.state === "pending") {
        entry.state = "blocked";
        entry.blocked_reason = "predecessor_" + reason;
      }
    }
  }

  #assertExistingAge() {
    const oldest = this.entries[0];
    if (!oldest) return;
    if (this.now() - oldest.created_at_ms > this.limits.max_age_ms) {
      throw new PendingBackpressure("pending_age_limit");
    }
  }

  #assertAdmission(entry) {
    if (this.entries.length + 1 > this.limits.max_count) {
      throw new PendingBackpressure("pending_count_limit");
    }
    const bytes = this.entries.reduce((sum, current) => sum + current.encoded_bytes, 0) + entry.encoded_bytes;
    if (bytes > this.limits.max_bytes) {
      throw new PendingBackpressure("pending_bytes_limit");
    }
    if (this.entries.length + 1 > this.limits.max_causal_depth) {
      throw new PendingBackpressure("pending_causal_depth_limit");
    }
  }

  #required(clientOperationId) {
    requireString(clientOperationId, "clientOperationId", 8, 160);
    const entry = this.byId.get(clientOperationId);
    if (!entry) throw new PendingRejected("unknown_pending_operation");
    return entry;
  }

  #validateRestored() {
    let expectedSequence = null;
    for (let i = 0; i < this.entries.length; i += 1) {
      const entry = this.entries[i];
      if (entry.schema !== CHAIN_SCHEMA) throw new TypeError("snapshot entry schema mismatch");
      if (entry.session_incarnation !== this.sessionIncarnation) {
        throw new TypeError("snapshot session incarnation mismatch");
      }
      if (expectedSequence == null) expectedSequence = entry.client_sequence;
      if (entry.client_sequence !== expectedSequence) {
        throw new TypeError("snapshot client sequence is not contiguous");
      }
      expectedSequence += 1;
      if (i === 0) {
        if (entry.causal_base.kind !== "canonical_revision") {
          throw new TypeError("snapshot head must have canonical causal base");
        }
      } else {
        if (
          entry.causal_base.kind !== "pending_after" ||
          entry.causal_base.client_operation_id !== this.entries[i - 1].client_operation_id
        ) {
          throw new TypeError("snapshot pending chain is not contiguous");
        }
      }
    }
    if (this.entries.length > this.limits.max_count ||
        this.entries.length > this.limits.max_causal_depth ||
        this.entries.reduce((sum, entry) => sum + entry.encoded_bytes, 0) > this.limits.max_bytes) {
      throw new TypeError("snapshot violates configured bounds");
    }
  }
}

export {CHAIN_SCHEMA as PENDING_CHAIN_V1, stableJson as canonicalPendingJson};
