# CHAPTERA-WIN-UPDATE-ACCEPT-01

Fresh-main W2 acceptance over merged CHAPTERA-UPDATE-CORE-HARDEN-01.

## Goal
Prove the Windows update path converges safely across healthy update, failed health check rollback, and restart recovery without mutating user state outside the product tree.

## Scope
- Windows-only product-level acceptance.
- Reuse chaptera-update-engine/orchestrator/trust; no second updater implementation.
- Use test identities/keys only; production signing remains out of scope.
- Fail closed on ambiguous or broken state.

## Acceptance
- A healthy candidate reaches confirmed current.
- A candidate that fails health returns to the previous confirmed tree.
- A process restart after unconfirmed activation recovers the previous confirmed tree before another transaction.
- External user-state sentinel survives every transition.
- Workflow runs on windows-latest against the fresh-main updater head.
