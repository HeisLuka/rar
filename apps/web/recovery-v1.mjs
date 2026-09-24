const RECOVERY_RECORD_V1 = "chaptera.browser-recovery-record.v1";
const HASH_ID_RE = /^sha256:[0-9a-f]{64}$/;

export class RecoveryConflict extends Error {}
export class RecoveryRejected extends Error {}
export class RecoveryStorageError extends Error {}

function clone(value) {
  return structuredClone(value);
}

function canonicalize(value) {
  if (value === null || typeof value === "string" || typeof value === "boolean") return value;
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new TypeError("non-finite number in recovery payload");
    return value;
  }
  if (Array.isArray(value)) return value.map(canonicalize);
  if (value && typeof value === "object") {
    const out = {};
    for (const key of Object.keys(value).sort()) {
      if (value[key] === undefined) throw new TypeError("undefined in recovery payload");
      out[key] = canonicalize(value[key]);
    }
    return out;
  }
  throw new TypeError("recovery payload must be JSON-safe");
}

function stableJson(value) {
  return JSON.stringify(canonicalize(value));
}

function requireString(value, label, min = 1, max = 256) {
  if (typeof value !== "string" || value.length < min || value.length > max) {
    throw new TypeError(`${label} must be a bounded string`);
  }
}

function requireRevision(value, label = "revision_id") {
  if (!HASH_ID_RE.test(value ?? "")) throw new TypeError(`${label} must be sha256 id`);
}

function requireLifecycleGeneration(value) {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new TypeError("lifecycle_generation must be a non-negative safe integer");
  }
}

function storageKey(scopeId, documentId, clientOperationId) {
  return stableJson([scopeId, documentId, clientOperationId]);
}

function scopeDocumentKey(scopeId, documentId) {
  return stableJson([scopeId, documentId]);
}

function validateOutcome(outcome) {
  if (!outcome || typeof outcome !== "object") throw new TypeError("operation outcome required");
  if (outcome.status === "not_found") return;
  if (outcome.status === "accepted") {
    requireRevision(outcome.revision_id, "accepted revision_id");
    return;
  }
  if (outcome.status === "rejected") {
    requireString(outcome.code, "rejection code");
    if (outcome.current_revision_id != null) {
      requireRevision(outcome.current_revision_id, "current_revision_id");
    }
    return;
  }
  throw new TypeError("unsupported operation outcome");
}

export class MemoryRecoveryStoreV1 {
  constructor() {
    this.records = new Map();
  }

  async put(record) {
    this.records.set(record.storage_key, clone(record));
  }

  async get(scopeId, documentId, clientOperationId) {
    const value = this.records.get(storageKey(scopeId, documentId, clientOperationId));
    return value ? clone(value) : null;
  }

  async delete(scopeId, documentId, clientOperationId) {
    this.records.delete(storageKey(scopeId, documentId, clientOperationId));
  }

  async listDocument(scopeId, documentId) {
    return [...this.records.values()]
      .filter((record) =>
        record.principal_scope_id === scopeId &&
        record.document_id === documentId)
      .sort((a, b) => a.sequence - b.sequence)
      .map(clone);
  }

  async clearDocument(scopeId, documentId) {
    for (const [key, record] of this.records.entries()) {
      if (record.principal_scope_id === scopeId && record.document_id === documentId) {
        this.records.delete(key);
      }
    }
  }
}

function requestResult(request) {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error("IndexedDB request failed"));
  });
}

function transactionDone(tx) {
  return new Promise((resolve, reject) => {
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error ?? new Error("IndexedDB transaction failed"));
    tx.onabort = () => reject(tx.error ?? new Error("IndexedDB transaction aborted"));
  });
}

export class IndexedDbRecoveryStoreV1 {
  constructor({
    dbName = "chaptera-local-recovery-v1",
    storeName = "pending",
  } = {}) {
    this.dbName = dbName;
    this.storeName = storeName;
  }

  async _open() {
    if (typeof indexedDB === "undefined") {
      throw new RecoveryStorageError("indexeddb_unavailable");
    }
    const request = indexedDB.open(this.dbName, 1);
    request.onupgradeneeded = () => {
      const db = request.result;
      let store;
      if (!db.objectStoreNames.contains(this.storeName)) {
        store = db.createObjectStore(this.storeName, {keyPath: "storage_key"});
      } else {
        store = request.transaction.objectStore(this.storeName);
      }
      if (!store.indexNames.contains("scope_document")) {
        store.createIndex(
          "scope_document",
          ["principal_scope_id", "document_id"],
          {unique: false},
        );
      }
    };
    return requestResult(request);
  }

  async put(record) {
    const db = await this._open();
    try {
      const tx = db.transaction(this.storeName, "readwrite");
      tx.objectStore(this.storeName).put(clone(record));
      await transactionDone(tx);
    } finally {
      db.close();
    }
  }

  async get(scopeId, documentId, clientOperationId) {
    const db = await this._open();
    try {
      const tx = db.transaction(this.storeName, "readonly");
      const result = await requestResult(
        tx.objectStore(this.storeName).get(storageKey(scopeId, documentId, clientOperationId)),
      );
      await transactionDone(tx);
      return result ? clone(result) : null;
    } finally {
      db.close();
    }
  }

  async delete(scopeId, documentId, clientOperationId) {
    const db = await this._open();
    try {
      const tx = db.transaction(this.storeName, "readwrite");
      tx.objectStore(this.storeName).delete(storageKey(scopeId, documentId, clientOperationId));
      await transactionDone(tx);
    } finally {
      db.close();
    }
  }

  async listDocument(scopeId, documentId) {
    const db = await this._open();
    try {
      const tx = db.transaction(this.storeName, "readonly");
      const store = tx.objectStore(this.storeName);
      const rows = await requestResult(
        store.index("scope_document").getAll([scopeId, documentId]),
      );
      await transactionDone(tx);
      return rows.sort((a, b) => a.sequence - b.sequence).map(clone);
    } finally {
      db.close();
    }
  }

  async clearDocument(scopeId, documentId) {
    const rows = await this.listDocument(scopeId, documentId);
    const db = await this._open();
    try {
      const tx = db.transaction(this.storeName, "readwrite");
      const store = tx.objectStore(this.storeName);
      for (const row of rows) store.delete(row.storage_key);
      await transactionDone(tx);
    } finally {
      db.close();
    }
  }
}

export class BrowserRecoveryCoordinatorV1 {
  constructor({
    store,
    principalScopeId,
    documentId,
    clientSessionIncarnation,
    appVersion,
    commandSemanticVersion,
  }) {
    if (!store || typeof store.put !== "function") throw new TypeError("recovery store required");
    requireString(principalScopeId, "principalScopeId");
    requireString(documentId, "documentId");
    requireString(clientSessionIncarnation, "clientSessionIncarnation");
    requireString(appVersion, "appVersion");
    requireString(commandSemanticVersion, "commandSemanticVersion");
    this.store = store;
    this.principalScopeId = principalScopeId;
    this.documentId = documentId;
    this.clientSessionIncarnation = clientSessionIncarnation;
    this.appVersion = appVersion;
    this.commandSemanticVersion = commandSemanticVersion;
  }

  async _storage(call) {
    try {
      return await call();
    } catch (error) {
      if (error instanceof RecoveryStorageError) throw error;
      throw new RecoveryStorageError("recovery_storage_failure", {cause: error});
    }
  }

  async listPending() {
    return this._storage(() =>
      this.store.listDocument(this.principalScopeId, this.documentId));
  }

  async prepareIntent({
    clientOperationId,
    baseRevisionId,
    lifecycleGeneration,
    dependsOnClientOperationId = null,
    normalizedIntent,
  }) {
    requireString(clientOperationId, "clientOperationId", 8, 160);
    requireRevision(baseRevisionId, "baseRevisionId");
    requireLifecycleGeneration(lifecycleGeneration);
    if (dependsOnClientOperationId != null) {
      requireString(dependsOnClientOperationId, "dependsOnClientOperationId", 8, 160);
      if (dependsOnClientOperationId === clientOperationId) {
        throw new TypeError("operation cannot depend on itself");
      }
    }
    if (!normalizedIntent || typeof normalizedIntent !== "object") {
      throw new TypeError("normalizedIntent object required");
    }

    const semantic = {
      schema_version: RECOVERY_RECORD_V1,
      principal_scope_id: this.principalScopeId,
      app_version: this.appVersion,
      command_semantic_version: this.commandSemanticVersion,
      document_id: this.documentId,
      client_session_incarnation: this.clientSessionIncarnation,
      client_operation_id: clientOperationId,
      depends_on_client_operation_id: dependsOnClientOperationId,
      base_revision_id: baseRevisionId,
      lifecycle_generation: lifecycleGeneration,
      normalized_intent: canonicalize(normalizedIntent),
    };
    const fingerprint = stableJson(semantic);
    const prior = await this._storage(() =>
      this.store.get(this.principalScopeId, this.documentId, clientOperationId));
    if (prior) {
      if (prior.intent_fingerprint !== fingerprint) {
        throw new RecoveryConflict("idempotency_conflict");
      }
      return clone(prior);
    }

    const existing = await this.listPending();
    const sequence = existing.reduce((max, row) => Math.max(max, row.sequence ?? 0), 0) + 1;
    const record = {
      ...semantic,
      storage_key: storageKey(this.principalScopeId, this.documentId, clientOperationId),
      scope_document_key: scopeDocumentKey(this.principalScopeId, this.documentId),
      intent_fingerprint: fingerprint,
      sequence,
      state: "prepared",
      rejection: null,
      quarantine_reason: null,
    };
    await this._storage(() => this.store.put(record));
    return clone(record);
  }

  async markSendStarted(clientOperationId) {
    const record = await this._required(clientOperationId);
    if (record.state === "sent_unknown") return record;
    if (record.state !== "prepared") {
      throw new RecoveryRejected("operation_not_sendable");
    }
    record.state = "sent_unknown";
    await this._storage(() => this.store.put(record));
    return clone(record);
  }

  async ackAccepted(clientOperationId, {revision_id}) {
    requireRevision(revision_id);
    await this._required(clientOperationId);
    await this._storage(() =>
      this.store.delete(this.principalScopeId, this.documentId, clientOperationId));
    return {
      status: "accepted",
      client_operation_id: clientOperationId,
      revision_id,
      pending_removed: true,
    };
  }

  async ackRejected(clientOperationId, {
    code,
    current_revision_id = null,
  }) {
    requireString(code, "code");
    if (current_revision_id != null) requireRevision(current_revision_id, "current_revision_id");
    const record = await this._required(clientOperationId);
    record.state = "rejected";
    record.rejection = {code, current_revision_id};
    await this._storage(() => this.store.put(record));
    return clone(record);
  }

  async _required(clientOperationId) {
    requireString(clientOperationId, "clientOperationId", 8, 160);
    const record = await this._storage(() =>
      this.store.get(this.principalScopeId, this.documentId, clientOperationId));
    if (!record) throw new RecoveryRejected("unknown_recovery_operation");
    return record;
  }

  async _quarantine(record, reason) {
    record.state = "quarantined";
    record.quarantine_reason = reason;
    await this._storage(() => this.store.put(record));
    return {
      client_operation_id: record.client_operation_id,
      action: "quarantined",
      reason,
    };
  }

  async planRecovery({
    principal_scope_id,
    current_revision_id,
    lifecycle_generation,
    app_version,
    command_semantic_version,
    authz_allowed,
    lookupOperationOutcome,
  }) {
    requireString(principal_scope_id, "principal_scope_id");
    requireRevision(current_revision_id, "current_revision_id");
    requireLifecycleGeneration(lifecycle_generation);
    requireString(app_version, "app_version");
    requireString(command_semantic_version, "command_semantic_version");
    if (typeof authz_allowed !== "boolean") throw new TypeError("authz_allowed must be boolean");
    if (typeof lookupOperationOutcome !== "function") {
      throw new TypeError("lookupOperationOutcome required");
    }

    const records = await this.listPending();
    const actions = [];
    for (const record of records) {
      if (record.state === "quarantined") {
        actions.push({
          client_operation_id: record.client_operation_id,
          action: "quarantined",
          reason: record.quarantine_reason,
        });
        continue;
      }
      if (record.principal_scope_id !== principal_scope_id ||
          principal_scope_id !== this.principalScopeId) {
        actions.push(await this._quarantine(record, "auth_scope_changed"));
        continue;
      }
      if (!authz_allowed) {
        actions.push(await this._quarantine(record, "authz_denied"));
        continue;
      }
      if (record.schema_version !== RECOVERY_RECORD_V1) {
        actions.push(await this._quarantine(record, "schema_version_mismatch"));
        continue;
      }
      if (record.app_version !== app_version) {
        actions.push(await this._quarantine(record, "app_version_mismatch"));
        continue;
      }
      if (record.command_semantic_version !== command_semantic_version) {
        actions.push(await this._quarantine(record, "command_semantic_version_mismatch"));
        continue;
      }
      if (record.lifecycle_generation !== lifecycle_generation) {
        actions.push(await this._quarantine(record, "lifecycle_generation_mismatch"));
        continue;
      }
      if (record.state === "rejected") {
        actions.push({
          client_operation_id: record.client_operation_id,
          action: "surface_rejection",
          rejection: clone(record.rejection),
        });
        continue;
      }

      if (record.state === "sent_unknown") {
        const outcome = await lookupOperationOutcome(record.client_operation_id);
        validateOutcome(outcome);
        if (outcome.status === "accepted") {
          await this._storage(() =>
            this.store.delete(
              this.principalScopeId,
              this.documentId,
              record.client_operation_id,
            ));
          actions.push({
            client_operation_id: record.client_operation_id,
            action: "resolved_accepted",
            revision_id: outcome.revision_id,
          });
          continue;
        }
        if (outcome.status === "rejected") {
          record.state = "rejected";
          record.rejection = {
            code: outcome.code,
            current_revision_id: outcome.current_revision_id ?? null,
          };
          await this._storage(() => this.store.put(record));
          actions.push({
            client_operation_id: record.client_operation_id,
            action: "surface_rejection",
            rejection: clone(record.rejection),
          });
          continue;
        }
      }

      if (record.base_revision_id !== current_revision_id) {
        actions.push({
          client_operation_id: record.client_operation_id,
          action: "refresh_required",
          reason: "stale_base_revision",
          base_revision_id: record.base_revision_id,
          current_revision_id,
        });
        continue;
      }

      if (record.depends_on_client_operation_id != null) {
        const dependency = await lookupOperationOutcome(record.depends_on_client_operation_id);
        validateOutcome(dependency);
        if (dependency.status === "not_found") {
          actions.push({
            client_operation_id: record.client_operation_id,
            action: "blocked_dependency",
            dependency_client_operation_id: record.depends_on_client_operation_id,
          });
          continue;
        }
        if (dependency.status === "rejected") {
          actions.push({
            client_operation_id: record.client_operation_id,
            action: "blocked_dependency_rejected",
            dependency_client_operation_id: record.depends_on_client_operation_id,
            dependency_code: dependency.code,
          });
          continue;
        }
      }

      actions.push({
        client_operation_id: record.client_operation_id,
        action: "retry_exact_identity",
        request: {
          document_id: record.document_id,
          base_revision_id: record.base_revision_id,
          client_operation_id: record.client_operation_id,
          depends_on_client_operation_id: record.depends_on_client_operation_id,
          normalized_intent: clone(record.normalized_intent),
        },
      });
    }
    return actions;
  }
}

export {
  RECOVERY_RECORD_V1,
  stableJson as canonicalRecoveryJson,
};
