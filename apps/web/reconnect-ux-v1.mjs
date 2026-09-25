const BANNER_KIND = Object.freeze({
  HIDDEN: "hidden",
  RECONNECTING: "reconnecting",
  CHECKING_OUTCOME: "checking_outcome",
  NOT_SYNCED: "not_synced",
  RECOVERY_UNAVAILABLE: "recovery_unavailable",
  NEEDS_ATTENTION: "needs_attention",
  READ_ONLY: "read_only",
});

function clone(value) { return value == null ? value : structuredClone(value); }

function requireString(value, label) {
  if (typeof value !== "string" || !value) throw new TypeError(label + " required");
  return value;
}

function normalizeFacts(facts) {
  if (!facts || typeof facts !== "object") throw new TypeError("reconnect facts required");
  const transport = requireString(facts.transport_state, "transport_state");
  const recovery = requireString(facts.recovery_state, "recovery_state");
  const editor = requireString(facts.editor_product_state, "editor_product_state");
  const allowedTransport = new Set(["connected","connecting","disconnected","degraded"]);
  const allowedRecovery = new Set(["clean","prepared","sent_unknown","locally_durable","storage_failed","stale","quarantined"]);
  if (!allowedTransport.has(transport)) throw new TypeError("unsupported transport_state");
  if (!allowedRecovery.has(recovery)) throw new TypeError("unsupported recovery_state");
  return {
    transport_state: transport,
    recovery_state: recovery,
    editor_product_state: editor,
    attention_code: facts.attention_code ?? null,
    read_only_reason: facts.read_only_reason ?? null,
    can_edit: facts.can_edit === true,
    pending_count: Number.isSafeInteger(facts.pending_count) && facts.pending_count >= 0 ? facts.pending_count : 0,
  };
}

export function deriveReconnectBannerV1(inputFacts) {
  const facts = normalizeFacts(inputFacts);
  if (facts.editor_product_state === "read_only" || facts.read_only_reason) {
    return Object.freeze({
      kind: BANNER_KIND.READ_ONLY,
      visible: true,
      severity: "warning",
      title: "Read only",
      message: facts.read_only_reason ?? "Editing is not currently available.",
      can_retry_connection: false,
      editing_fenced: true,
    });
  }
  if (facts.recovery_state === "storage_failed" || facts.editor_product_state === "recovery_unavailable") {
    return Object.freeze({
      kind: BANNER_KIND.RECOVERY_UNAVAILABLE,
      visible: true,
      severity: "error",
      title: "Recovery unavailable",
      message: "Connection was interrupted and local recovery storage is unavailable. Editing is paused.",
      can_retry_connection: facts.transport_state !== "connected",
      editing_fenced: true,
    });
  }
  if (facts.recovery_state === "sent_unknown" || facts.editor_product_state === "checking_save_status") {
    return Object.freeze({
      kind: BANNER_KIND.CHECKING_OUTCOME,
      visible: true,
      severity: "info",
      title: "Checking your last change",
      message: "Checking whether your last change was accepted before anything is retried.",
      can_retry_connection: false,
      editing_fenced: true,
    });
  }
  if (facts.recovery_state === "stale" || facts.recovery_state === "quarantined" || facts.editor_product_state === "needs_attention") {
    return Object.freeze({
      kind: BANNER_KIND.NEEDS_ATTENTION,
      visible: true,
      severity: "warning",
      title: "Needs attention",
      message: facts.attention_code ?? "The document changed while you were disconnected. Review before continuing.",
      can_retry_connection: false,
      editing_fenced: true,
    });
  }
  if (facts.editor_product_state === "not_synced" || facts.recovery_state === "locally_durable") {
    return Object.freeze({
      kind: BANNER_KIND.NOT_SYNCED,
      visible: true,
      severity: "warning",
      title: "Not synced",
      message: "Your pending work is stored locally. It will be reconciled when the canonical session is available.",
      can_retry_connection: facts.transport_state !== "connected",
      editing_fenced: false,
    });
  }
  if (facts.transport_state === "connecting" || facts.transport_state === "disconnected" || facts.transport_state === "degraded") {
    return Object.freeze({
      kind: BANNER_KIND.RECONNECTING,
      visible: true,
      severity: "info",
      title: "Reconnecting",
      message: "Re-establishing the canonical editing session.",
      can_retry_connection: facts.transport_state !== "connecting",
      editing_fenced: !facts.can_edit,
    });
  }
  return Object.freeze({
    kind: BANNER_KIND.HIDDEN,
    visible: false,
    severity: "none",
    title: "",
    message: "",
    can_retry_connection: false,
    editing_fenced: !facts.can_edit,
  });
}

export class WebReconnectUxV1 {
  constructor({ statusProvider, reconnectCommand, onState = null }) {
    if (!statusProvider || typeof statusProvider.currentReconnectFacts !== "function") {
      throw new TypeError("statusProvider.currentReconnectFacts() is required");
    }
    if (typeof reconnectCommand !== "function") throw new TypeError("reconnectCommand() is required");
    this.statusProvider = statusProvider;
    this.reconnectCommand = reconnectCommand;
    this.onState = onState;
    this.browser_network_hint = "unknown";
    this._state = this._derive();
  }

  state() { return clone(this._state); }

  refresh() {
    this._state = this._derive();
    this.onState?.(this.state());
    return this.state();
  }

  setBrowserNetworkHint(value) {
    if (!["online","offline","unknown"].includes(value)) throw new TypeError("invalid browser network hint");
    this.browser_network_hint = value;
    return this.refresh();
  }

  async retryConnection() {
    const state = this.refresh();
    if (!state.banner.can_retry_connection) throw new Error("connection retry is not currently admitted");
    await this.reconnectCommand();
    return this.refresh();
  }

  _derive() {
    const facts = normalizeFacts(this.statusProvider.currentReconnectFacts());
    return Object.freeze({
      protocol_version: "chaptera.web-reconnect-ux.v1",
      browser_network_hint: this.browser_network_hint,
      authoritative: facts,
      banner: deriveReconnectBannerV1(facts),
    });
  }
}

export function bindBrowserNetworkHintsV1({ controller, target = globalThis.window ?? null }) {
  if (!controller || typeof controller.setBrowserNetworkHint !== "function") {
    throw new TypeError("controller.setBrowserNetworkHint() is required");
  }
  if (!target || typeof target.addEventListener !== "function") {
    return { destroy() {} };
  }
  const onOnline = () => controller.setBrowserNetworkHint("online");
  const onOffline = () => controller.setBrowserNetworkHint("offline");
  target.addEventListener("online", onOnline);
  target.addEventListener("offline", onOffline);
  return {
    destroy() {
      target.removeEventListener("online", onOnline);
      target.removeEventListener("offline", onOffline);
    },
  };
}
