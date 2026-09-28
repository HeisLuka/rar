# CHAPTERA-LOCAL-DOGFOOD-LOCK-RESTART-PREFLIGHT-01

Safe Windows restart/resume harness for the persistent local Chaptera dogfood gate.

## Purpose

Prove the physical host primitives needed by final updater dogfood without coupling the preflight to updater internals:

- owned process termination;
- exclusive Windows file lock and release;
- durable restart marker;
- Windows Task Scheduler resume registration;
- post-start resume detection;
- cleanup.

## Safety law

The default Prepare path never reboots the machine.

If either -RequestReboot or -AcknowledgeSafeReboot is absent, the harness emits REBOOT_DEFERRED_SAFETY, records the prepared proof and removes the temporary scheduled task/state.

A reboot is requested only when both switches are supplied explicitly. The reboot command does not use /f, so Windows is not instructed to force-close interactive applications.

## Modes

SelfTest:
Runs lock/process/marker/scheduler/resume/cleanup on the same boot. It must emit SELFTEST_PASS and must not claim a real reboot.

Prepare:
Creates durable state and registers an ONLOGON resume task. With no explicit reboot authorization, emits REBOOT_DEFERRED_SAFETY.

Resume:
Requires the durable marker. For a real run, LastBootUpTime must differ from the pre-reboot value before PASS is emitted.

Cleanup:
Removes task-owned scheduler entry and temporary state.

## Hosted claim boundary

GitHub Actions runs only SelfTest. This validates script syntax, scheduler registration, marker durability, same-process cleanup and lock primitives.

A real reboot receipt can only be produced on the owner's persistent Windows host and remains a local acceptance requirement.
