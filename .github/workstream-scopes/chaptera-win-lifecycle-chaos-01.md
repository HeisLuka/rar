# CHAPTERA-WIN-LIFECYCLE-CHAOS-01

Hosted Windows fault-recovery profile layered on the merged deterministic lifecycle soak.

## Goal

Exercise installed Reader recovery across process-boundary failures and a real Windows exclusive file lock, while keeping the observer outside the product and preserving the final real-machine reboot gate.

## V1 scenarios

1. Install the real Reader A.
2. Stage a verified candidate and stop after PreviousRetained.
3. Drop the updater engine without recovery.
4. Construct a fresh engine instance and require rollback to byte-identical A.
5. Repeat with process death after CandidateActivated but before confirmation.
6. Hold current/chaptera-reader.exe with Windows share mode 0.
7. Require candidate preparation to fail closed.
8. Release the file handle, construct a fresh engine instance and require PreparedTransactionAborted recovery.
9. Require real PUB smoke to pass and source PUB/external state to remain byte-identical.
10. Emit fault-recovery.json beside the ordinary lifecycle soak evidence.

## Claim boundary

A fresh updater engine instance on the same Windows runner is a process-restart proof, not an operating-system reboot proof.

Actual Task Scheduler/startup continuation across a real Windows reboot remains owned by CHAPTERA-LOCAL-DOGFOOD-LOCK-RESTART-PREFLIGHT-01 and final CHAPTERA-WIN-UPDATE-DOGFOOD-01.

## Failure law

No test-side copy of predecessor bytes into current, registry repair, journal rewriting, or other hidden surgery may turn a failed product recovery into PASS.
