# Chaptera state-transition fidelity v1

Current render or geometry equality is not sufficient evidence of authoring fidelity when preserved state can change the result of the next supported edit.

The bounded validation law is:

```text
State S + exact Operation O -> Next State S'
```

Two receipts are **equivalent** only when the same versioned comparison contract, exact operation, required normalized state fields, and required invariants are all known and equal before and after the operation.

A comparison is **divergent** when those required known values differ.

A comparison is **not_comparable** when the contract/operation differs, a required field or invariant is missing, or required authority is explicitly unknown/unavailable. Missing evidence is never converted into equality.

## Ownership boundary

This protocol owns only the source-neutral pairwise comparison envelope.

It does not own:

- Publisher mutation orchestration;
- COM lifecycle or save/reopen control;
- raw PUB stream/byte blast-radius analysis;
- target-side post-import edit scripts or multi-edit sequences;
- feature-specific semantics such as aspect-ratio rebasing, inline ownership, connector routing/site normalization, image DPI/original-size law, or callout policy.

Feature-specific research/authoring owners define their normalized comparable fields and native oracle. The comparator consumes those receipts without inventing defaults.

## V1 synthetic discriminator controls

The source-free tests intentionally include four cases where the current projection can match while the next supported operation diverges:

1. same rectangle, different latent aspect-lock behavior -> next resize;
2. same object placement, fixed vs Story-owned inline behavior -> next text edit;
3. same connector pixels, relation-preserved vs flattened behavior -> next endpoint-owner move;
4. same picture frame, different original-size baseline -> relative-to-original scaling.

Callout policy is a direct semantic consumer of the same primitive but is not required as a fifth V1 synthetic domain fixture.

## Files

- `packages/protocol/state-transition/v1/receipt.schema.json`
- `tools/compare_state_transition_receipts.py`
- `tools/test_compare_state_transition_receipts.py`
- `.github/workflows/state-transition-fidelity.yml`
