# PREFLIGHT-01 — continuous fidelity/loss diagnostics V1

This gate is a source-neutral diagnostics aggregator. It does not parse PUB and it does not decide export semantics.

Inputs are already-grounded Scene V1 state plus explicit upstream target-output risks. The aggregator preserves upstream diagnostics and adds deterministic derived diagnostics for resource availability, opaque/unsupported Story semantics and non-supported capability state.

The receipt is object-scoped where a canonical NodeId is available. Machine codes and human-readable message keys are both retained.

The contract test proves a bounded fix removes the resolved resource diagnostic without suppressing unrelated overset, opaque-story, capability or target-output warnings.
