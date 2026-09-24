import { RENDERER_KINDS } from "./render-v1.mjs";

export const RENDER_FALLBACK_VERSION_V1 = "chaptera.web-render-fallback.v1";

export const FALLBACK_REASON_CODES_V1 = Object.freeze([
  "api_unavailable",
  "initialization_failed",
  "device_or_context_lost",
  "circuit_open",
  "workload_capability_missing",
  "surface_incompatible",
  "safe_mode_disabled",
  "same_backend_recovery",
  "fallback_after_repeated_loss",
  "no_compatible_backend"
]);

function clone(value) {
  return structuredClone(value);
}

function assertFiniteInteger(value, label) {
  if (!Number.isSafeInteger(value)) throw new TypeError(label + " must be a safe integer");
  return value;
}

function normalizeCandidate(candidate) {
  if (!candidate || typeof candidate !== "object") throw new TypeError("candidate must be object");
  if (!candidate.backend_id || typeof candidate.backend_id !== "string") {
    throw new TypeError("candidate.backend_id is required");
  }
  if (!RENDERER_KINDS.includes(candidate.renderer_kind)) {
    throw new Error("candidate renderer_kind is not an admitted render-v1 backend");
  }
  const preferenceRank = assertFiniteInteger(candidate.preference_rank, "preference_rank");
  if (preferenceRank < 0) throw new RangeError("preference_rank must be >= 0");
  return Object.freeze({
    backend_id: candidate.backend_id,
    renderer_kind: candidate.renderer_kind,
    preference_rank: preferenceRank
  });
}

export function normalizeFallbackPolicyV1(policy = {}) {
  const sameBackendRetryLimit = assertFiniteInteger(
    policy.same_backend_retry_limit ?? 1,
    "same_backend_retry_limit"
  );
  const circuitFailureThreshold = assertFiniteInteger(
    policy.circuit_failure_threshold ?? 2,
    "circuit_failure_threshold"
  );
  const cooldownMs = assertFiniteInteger(policy.cooldown_ms ?? 30_000, "cooldown_ms");
  if (sameBackendRetryLimit < 0 || circuitFailureThreshold < 1 || cooldownMs < 1) {
    throw new RangeError("fallback policy values are outside V1 bounds");
  }
  return Object.freeze({
    same_backend_retry_limit: sameBackendRetryLimit,
    circuit_failure_threshold: circuitFailureThreshold,
    cooldown_ms: cooldownMs
  });
}

function requirementList(requirements) {
  if (!requirements || typeof requirements !== "object") {
    throw new TypeError("workload requirements are required");
  }
  const mandatory = [...(requirements.mandatory_capabilities ?? [])];
  mandatory.sort();
  return mandatory;
}

function probeCompatibility(probe, requirements) {
  if (!probe || probe.available !== true) {
    return { compatible: false, reason: probe?.reason_code ?? "api_unavailable" };
  }
  const mandatory = requirementList(requirements);
  const supported = new Set(probe.capabilities ?? []);
  const missing = mandatory.filter((item) => !supported.has(item));
  if (missing.length) {
    return {
      compatible: false,
      reason: "workload_capability_missing",
      missing
    };
  }
  if (probe.surface_compatible === false) {
    return { compatible: false, reason: "surface_incompatible" };
  }
  return { compatible: true, reason: null, missing: [] };
}

export function selectBackendV1({
  candidates,
  probes,
  requirements,
  circuits = {},
  now_ms = 0
}) {
  const normalized = candidates.map(normalizeCandidate).sort(
    (a, b) => a.preference_rank - b.preference_rank ||
      a.backend_id.localeCompare(b.backend_id)
  );
  const rejected = [];
  for (const candidate of normalized) {
    const circuit = circuits[candidate.backend_id];
    if (circuit?.open_until_ms > now_ms) {
      rejected.push({
        backend_id: candidate.backend_id,
        reason_code: "circuit_open"
      });
      continue;
    }
    const verdict = probeCompatibility(probes[candidate.backend_id], requirements);
    if (!verdict.compatible) {
      rejected.push({
        backend_id: candidate.backend_id,
        reason_code: verdict.reason,
        missing_capabilities: verdict.missing ?? []
      });
      continue;
    }
    return Object.freeze({
      selected_backend_id: candidate.backend_id,
      renderer_kind: candidate.renderer_kind,
      selected_preference_rank: candidate.preference_rank,
      selection_reason: rejected.length ? "fallback_from_rejected_preferred" : "preferred_compatible",
      rejected: Object.freeze(rejected)
    });
  }
  return Object.freeze({
    selected_backend_id: null,
    renderer_kind: null,
    selected_preference_rank: null,
    selection_reason: "no_compatible_backend",
    rejected: Object.freeze(rejected)
  });
}

export class RenderFallbackControllerV1 {
  constructor({
    candidates,
    probes,
    requirements,
    semantic_state,
    policy = {},
    now_ms = 0
  }) {
    this.version = RENDER_FALLBACK_VERSION_V1;
    this.candidates = candidates.map(normalizeCandidate);
    this.probes = clone(probes);
    this.requirements = clone(requirements);
    this.semanticState = clone(semantic_state);
    this.policy = normalizeFallbackPolicyV1(policy);
    this.circuits = {};
    this.failures = {};
    this.sameBackendRecoveryAttempts = {};
    this.state = "unselected";
    this.activeBackendId = null;
    this.pendingBackendId = null;
    this.generation = 0;
    this.pendingGeneration = null;
    this.lastSelection = null;
    this.switchCount = 0;
    this.canonicalOperationsEmitted = 0;
    this.authoringRevisionsEmitted = 0;
    this.layoutRevisionsEmitted = 0;
    this.backendLocalState = {};
    this.startedAtMs = now_ms;
  }

  invariantSnapshot() {
    return clone(this.semanticState);
  }

  initialSelect(now_ms = 0) {
    const selection = selectBackendV1({
      candidates: this.candidates,
      probes: this.probes,
      requirements: this.requirements,
      circuits: this.circuits,
      now_ms
    });
    this.lastSelection = selection;
    if (!selection.selected_backend_id) {
      this.state = "no_compatible_backend";
      this.pendingBackendId = null;
      this.pendingGeneration = null;
      return selection;
    }
    this.state = "initializing";
    this.pendingBackendId = selection.selected_backend_id;
    this.pendingGeneration = this.generation + 1;
    return selection;
  }

  publishCoherentFrame({ backend_id, generation, coherent }) {
    if (coherent !== true) {
      return Object.freeze({ published: false, reason: "frame_not_coherent" });
    }
    if (
      backend_id !== this.pendingBackendId ||
      generation !== this.pendingGeneration
    ) {
      return Object.freeze({ published: false, reason: "stale_generation_or_backend" });
    }
    this.activeBackendId = backend_id;
    this.generation = generation;
    this.pendingBackendId = null;
    this.pendingGeneration = null;
    this.backendLocalState = { generation, backend_id };
    this.state = "active";
    return Object.freeze({ published: true, backend_id, generation });
  }

  completionIsCurrent(generation) {
    return generation === this.generation ||
      (this.pendingGeneration !== null && generation === this.pendingGeneration);
  }

  reportLoss({ backend_id, now_ms = 0 }) {
    if (this.state !== "active" || backend_id !== this.activeBackendId) {
      return Object.freeze({ action: "ignored_stale_loss" });
    }
    const before = this.invariantSnapshot();
    const failures = (this.failures[backend_id] ?? 0) + 1;
    this.failures[backend_id] = failures;
    this.state = "recovering_same_backend";

    const attempts = this.sameBackendRecoveryAttempts[backend_id] ?? 0;
    if (
      attempts < this.policy.same_backend_retry_limit &&
      failures < this.policy.circuit_failure_threshold
    ) {
      this.sameBackendRecoveryAttempts[backend_id] = attempts + 1;
      this.pendingBackendId = backend_id;
      this.pendingGeneration = this.generation + 1;
      this.backendLocalState = {};
      return Object.freeze({
        action: "recover_same_backend",
        backend_id,
        generation: this.pendingGeneration,
        reason_code: "same_backend_recovery",
        semantic_state_preserved: JSON.stringify(before) === JSON.stringify(this.semanticState)
      });
    }

    this.circuits[backend_id] = {
      opened_at_ms: now_ms,
      open_until_ms: now_ms + this.policy.cooldown_ms,
      failure_count: failures
    };
    this.state = "switching_backend";
    this.backendLocalState = {};
    const selection = selectBackendV1({
      candidates: this.candidates,
      probes: this.probes,
      requirements: this.requirements,
      circuits: this.circuits,
      now_ms
    });
    this.lastSelection = selection;
    if (!selection.selected_backend_id) {
      this.activeBackendId = null;
      this.pendingBackendId = null;
      this.pendingGeneration = null;
      this.state = "no_compatible_backend";
      return Object.freeze({
        action: "no_compatible_backend",
        reason_code: "no_compatible_backend",
        semantic_state_preserved: JSON.stringify(before) === JSON.stringify(this.semanticState)
      });
    }
    this.pendingBackendId = selection.selected_backend_id;
    this.pendingGeneration = this.generation + 1;
    this.switchCount += 1;
    return Object.freeze({
      action: "switch_backend",
      backend_id: selection.selected_backend_id,
      generation: this.pendingGeneration,
      reason_code: "fallback_after_repeated_loss",
      semantic_state_preserved: JSON.stringify(before) === JSON.stringify(this.semanticState)
    });
  }

  reportInitializationFailure({ backend_id, now_ms = 0 }) {
    if (backend_id !== this.pendingBackendId) {
      return Object.freeze({ action: "ignored_stale_initialization_failure" });
    }
    this.failures[backend_id] = (this.failures[backend_id] ?? 0) + 1;
    this.circuits[backend_id] = {
      opened_at_ms: now_ms,
      open_until_ms: now_ms + this.policy.cooldown_ms,
      failure_count: this.failures[backend_id]
    };
    this.backendLocalState = {};
    const selection = selectBackendV1({
      candidates: this.candidates,
      probes: this.probes,
      requirements: this.requirements,
      circuits: this.circuits,
      now_ms
    });
    this.lastSelection = selection;
    if (!selection.selected_backend_id) {
      this.activeBackendId = null;
      this.pendingBackendId = null;
      this.pendingGeneration = null;
      this.state = "no_compatible_backend";
      return Object.freeze({ action: "no_compatible_backend" });
    }
    this.pendingBackendId = selection.selected_backend_id;
    this.pendingGeneration = this.generation + 1;
    this.state = "switching_backend";
    this.switchCount += 1;
    return Object.freeze({
      action: "switch_backend",
      backend_id: selection.selected_backend_id,
      generation: this.pendingGeneration
    });
  }

  canRepromote(backend_id, now_ms = 0) {
    const circuit = this.circuits[backend_id];
    if (!circuit) return true;
    return now_ms >= circuit.open_until_ms;
  }

  receipt() {
    return Object.freeze({
      protocol_version: this.version,
      state: this.state,
      active_backend_id: this.activeBackendId,
      pending_backend_id: this.pendingBackendId,
      generation: this.generation,
      pending_generation: this.pendingGeneration,
      selected_reason: this.lastSelection?.selection_reason ?? null,
      rejected_candidates: clone(this.lastSelection?.rejected ?? []),
      switch_count: this.switchCount,
      circuit_breakers: clone(this.circuits),
      canonical_operations_emitted: this.canonicalOperationsEmitted,
      authoring_revisions_emitted: this.authoringRevisionsEmitted,
      layout_revisions_emitted: this.layoutRevisionsEmitted,
      semantic_state: this.invariantSnapshot()
    });
  }
}
