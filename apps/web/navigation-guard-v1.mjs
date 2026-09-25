function clone(value) { return value == null ? value : structuredClone(value); }

function normalizeFacts(facts) {
  if (!facts || typeof facts !== "object") throw new TypeError("navigation facts required");
  const pending = Number.isSafeInteger(facts.pending_intent_count) && facts.pending_intent_count >= 0
    ? facts.pending_intent_count : 0;
  return {
    editor_product_state: String(facts.editor_product_state ?? "unknown"),
    pending_intent_count: pending,
    canonical_frontier_durable: facts.canonical_frontier_durable === true,
    local_recovery_durable: facts.local_recovery_durable === true,
    recovery_storage_available: facts.recovery_storage_available !== false,
    unrecoverable_intent: facts.unrecoverable_intent === true,
  };
}

export function evaluateNavigationSafetyV1(inputFacts) {
  const facts = normalizeFacts(inputFacts);
  if (facts.pending_intent_count === 0 && !facts.unrecoverable_intent) {
    return Object.freeze({ safe_to_leave:true, warn:false, reason:"no_pending_intent" });
  }
  if (facts.canonical_frontier_durable) {
    return Object.freeze({ safe_to_leave:true, warn:false, reason:"canonical_durable" });
  }
  if (facts.unrecoverable_intent) {
    return Object.freeze({ safe_to_leave:false, warn:true, reason:"explicit_unrecoverable_intent" });
  }
  if (facts.pending_intent_count > 0 && facts.local_recovery_durable) {
    return Object.freeze({ safe_to_leave:true, warn:false, reason:"local_recovery_durable" });
  }
  if (facts.pending_intent_count > 0 && !facts.recovery_storage_available) {
    return Object.freeze({ safe_to_leave:false, warn:true, reason:"recovery_storage_unavailable" });
  }
  return Object.freeze({
    safe_to_leave:false,
    warn:true,
    reason:"pending_intent_not_durably_recoverable",
  });
}

export class WebNavigationGuardV1 {
  constructor({ statusProvider, onDecision = null }) {
    if (!statusProvider || typeof statusProvider.currentNavigationFacts !== "function") {
      throw new TypeError("statusProvider.currentNavigationFacts() is required");
    }
    this.statusProvider = statusProvider;
    this.onDecision = onDecision;
  }

  decision() {
    const facts = normalizeFacts(this.statusProvider.currentNavigationFacts());
    const decision = evaluateNavigationSafetyV1(facts);
    const out = Object.freeze({
      protocol_version:"chaptera.web-navigation-guard.v1",
      facts:Object.freeze(facts),
      ...decision,
    });
    this.onDecision?.(clone(out));
    return clone(out);
  }

  allowAppNavigation() {
    return this.decision().safe_to_leave;
  }
}

export function bindBeforeUnloadGuardV1({ guard, target = globalThis.window ?? null }) {
  if (!guard || typeof guard.decision !== "function") throw new TypeError("guard.decision() is required");
  if (!target || typeof target.addEventListener !== "function") return { destroy() {} };
  const onBeforeUnload = (event) => {
    const decision = guard.decision();
    if (!decision.warn) return;
    event.preventDefault?.();
    event.returnValue = "";
    return "";
  };
  target.addEventListener("beforeunload", onBeforeUnload);
  return {
    destroy() { target.removeEventListener("beforeunload", onBeforeUnload); },
  };
}
