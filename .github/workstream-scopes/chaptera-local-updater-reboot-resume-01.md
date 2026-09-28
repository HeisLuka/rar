# CHAPTERA-LOCAL-UPDATER-REBOOT-RESUME-01

Journal-aware reboot/resume harness for the final persistent Windows updater dogfood.

## Goal

Bind the safe restart/resume preflight to the real Chaptera update journal so a post-reboot continuation proves recovery of a specific interrupted transaction, not merely that Windows restarted.

## Architecture

- `chaptera-update-recovery-probe` is a small diagnostic CLI over the existing `UpdateEngine`.
- It owns no independent recovery logic.
- `prepare-fault` creates a verified candidate transaction and stops at one explicit journal phase.
- `inspect` reports active transaction/phase and current-tree fingerprint.
- `recover` acquires the real install lock, calls `UpdateEngine::recover()`, cleans terminal transaction leftovers and reports the outcome.
- PowerShell owns durable reboot state, Task Scheduler registration and safety gating.

## Safety gates

Real installed-product mutation requires `-AcknowledgeProductMutation`.

A Windows restart additionally requires both `-RequestReboot` and `-AcknowledgeSafeReboot`.

Without product-mutation acknowledgement, the harness returns `MUTATION_DEFERRED_SAFETY` without changing the product tree.

If mutation is acknowledged but reboot is not, the harness recovers the interrupted transaction immediately through `UpdateEngine::recover()` and returns `REBOOT_DEFERRED_SAFETY`; it must not strand the Reader in a fault state.

The reboot request never uses `/f`.

## Resume acceptance

For a real reboot run:

1. durable state names the exact transaction id and fault phase;
2. Windows boot identity must change;
3. the same active journal transaction/phase must still be present before recovery;
4. `recover()` must return the phase-appropriate outcome;
5. no active journal remains;
6. recovered `current/` fingerprint must equal the pre-fault predecessor fingerprint;
7. the receipt records the transaction, phase, recovery outcome and both tree fingerprints.

## Hosted self-test

GitHub Windows runs same-boot self-tests for `previous-retained` and `candidate-activated` using an isolated temporary install tree.

Hosted CI also proves that the default Prepare path cannot mutate a real install without the explicit mutation acknowledgement.

A hosted self-test is never evidence of a real OS reboot.
