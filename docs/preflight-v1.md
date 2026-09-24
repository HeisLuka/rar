# PREFLIGHT-01 — continuous fidelity/loss diagnostics V1

This gate is a source-neutral diagnostics aggregator. It does not parse PUB and it does not decide export semantics.

Inputs are already-grounded Scene V1 state plus explicit upstream target-output risks and optional expected resource hashes. The aggregator preserves upstream diagnostics and adds deterministic derived diagnostics for resource availability, modified resources, opaque/unsupported Story semantics and non-supported capability state.

Diagnostics are object-scoped whenever Scene identity provides a canonical NodeId. Story diagnostics resolve through `story_frames`; resource diagnostics resolve through node resource bindings. Stable machine codes are paired with message keys and a deterministic human-readable count/blocking summary.

The contract test covers text overflow supplied by the grounded layout diagnostic stream, missing and modified resources, opaque Story semantics, capability loss, and target-output risk. A bounded resource fix must remove only the resolved diagnostic while preserving unrelated warnings.
