# CHAPTERA-WIN-LIFECYCLE-SOAK-01

Repeated external diagnostic runner for the installed Chaptera Reader lifecycle.

## Goal

Exercise the installed-product lifecycle repeatedly and collect phase-level evidence instead of treating one green update acceptance as sufficient evidence for public distribution.

The diagnostic process is deliberately outside Reader/updater ownership. A broken Reader must not destroy the observer or hide the state that led to failure.

## Reused authorities

- CHAPTERA-WIN-UPDATE-ACCEPT-01 owns the real installed Reader A -> B -> rejected C -> byte-identical rollback-to-B primitive.
- The existing Windows installer owns current/.staging/.rollback layout, Open With registration and uninstall cleanup.
- Reader --smoke-check owns the bounded real-PUB executable health probe.
- Update engine/orchestrator/trust own transaction and authenticated payload-swap semantics.
- CHAPTERA-WIN-REPAIR-01 remains the product repair-contract authority; this soak slice uses a pinned local B installer to exercise actual reinstall/repair bytes without inventing a second repair protocol.

## V1 cycle

Each cycle runs:

1. preclean any prior Chaptera installation and require a clean product boundary;
2. install A through the real Inno installer;
3. run real Reader smoke on pinned SampleNewsletter.pub;
4. invoke the existing installed update acceptance A -> B -> rejected C -> rollback B;
5. verify real Reader smoke, Open With, foreign .pub default, source PUB hash and external-state sentinel;
6. inject deterministic corruption by replacing current/chaptera-reader.exe;
7. require the corrupted Reader health probe to fail;
8. repair/reinstall from a pinned B installer;
9. require the repaired Reader binary to match the pre-corruption rollback-B binary byte-for-byte and pass real-PUB smoke;
10. uninstall;
11. require Chaptera Open With/updater-owned install content to be gone while the foreign default, source PUB and external state remain unchanged;
12. repeat.

Pull-request acceptance defaults to 3 cycles. Manual workflow dispatch defaults to 10 and accepts a higher bounded cycle count from the operator.

## Diagnostics

The runner emits:

- state.json: durable last phase/cycle/status for crash diagnosis and later resume work;
- events.jsonl: one event per phase with timestamp, duration, installed-tree fingerprint, registry snapshot, updater-journal presence/hash, relevant process snapshot, fixture hash, external-state hash and phase-specific evidence;
- summary.json: requested/completed cycles, failure location, PASS/FAIL counts and aggregate phase-duration p50/p95.

The runner never records recovered document text or private document bytes. The pinned public PUB is represented by identity/hash.

## Failure law

An unexpected state is a failed cycle even if the diagnostic script could manually repair it.

The observer may invoke only the product operation explicitly under test. It must not copy a predecessor tree or mutate registry/product state behind the updater/installer to manufacture a PASS.

## V1 non-goals

- real reboot/resume through Task Scheduler;
- random fault scheduling / chaos seeds;
- abrupt power-loss simulation;
- production signing or public repository;
- network fault injection;
- claiming the current pinned reinstall path closes CHAPTERA-WIN-REPAIR-01 product wiring.

Those are follow-on soak profiles after the deterministic repeated baseline is green.
