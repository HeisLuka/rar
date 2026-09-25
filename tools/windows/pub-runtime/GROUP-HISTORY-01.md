# GROUP-HISTORY-01

Bounded native experiment for `PUB-T-633 / U-GEO-04`.

## Question

Does Publisher's previous-group memory survive Save, process close, one fresh reopen, or two fresh reopens?

The first arm deliberately answers only the COM-visible lifetime question. A persisted carrier may be named only if Regroup still succeeds after reopen and a later structural diff localizes a causal difference between a previously grouped-then-ungrouped PUB and an independently constructed ungrouped control.

## Exact environment

- packet: `tools/research-runner/experiments/group-history-01.packet.json`
- environment: `publisher-2019`
- exact blank: `pubgen-create-20260923/minimal-blank-v1-generated.pub`
- blank SHA-256: `5bf6057b8b11c8ee4a421d93885ae6e9e7c03a7a42d541e6d0df33497c08c33b`
- Publisher EXE SHA-256: `e1ef8811b85b82045f37c4173b92726101be3a25e550b0dcb9f178df834ab20b`
- expected Publisher version prefix: `16.0.12527.`

Each arm starts from a fresh copy of the same blank and creates three tagged rectangles A/B/C.

## Arms

- `S0`: independent A/B/C, one Save + reopen, negative Regroup control.
- `S1`: Group(A,B,C)=G, Save + reopen.
- `S2`: Group → Ungroup → Regroup in memory, no Save.
- `S3`: Group → Ungroup → Save → Regroup in the same session.
- `S4`: Group → Ungroup → Save → close → one reopen → Regroup.
- `S5`: Group → Ungroup → Save → close/reopen twice → Regroup.
- `S6`: independent A/B/C, first Save/reopen, second no-op Save/reopen, negative Regroup control.

Former members are re-found by stable `PUB_ORACLE_ID` tags rather than assuming Publisher preserves their shape names after Ungroup.

## Evidence

For each relevant state the operation records:

- document dirty/saved state;
- top-level shape count;
- Shape.Name / Shape.ID / Shape.Type;
- oracle tag;
- z-order;
- bounds;
- group item count and child identity;
- Regroup success/error + HRESULT;
- returned group identity;
- whether Regroup reused the old group Shape.ID or allocated a new one;
- private PUB file size/SHA-256 receipts.

## Decision rule

- S2 and S3 succeed, S4/S5 fail, both independent controls fail → previous-group memory is bounded as session/runtime-scoped for this build.
- S4 or S5 succeeds while both independent controls fail → persisted-across-reopen candidate; next step is structural diff of the private saved ungrouped PUB against S0/S6.
- S0 or S6 succeeds → invalid control / harness or API-assumption failure; do not interpret persistence.
- A raw carrier is never inferred from COM success alone.

## Evidence boundary

Generated PUB files remain under the native runner's private evidence directory. Uploaded evidence is limited to:

- `environment.json`
- `analysis/group-history-01.json`
- `logs/group-history-01.txt`
- generic evidence manifest

If reopen Regroup is positive, carrier localization is a follow-up on those private bytes; Open XML `regrouptable` remains only a cross-format clue, not Publisher evidence.
