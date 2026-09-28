# CHAPTERA-UPDATE-CORE-HARDEN-01

Coordination bootstrap for the draft PR. Replace or remove this file before the PR is marked ready if the implementation no longer needs it.

Source of truth: Notion task CHAPTERA-UPDATE-CORE-HARDEN-01.

## Goal

Harden the landed chaptera-update-engine/orchestrator/handoff split. Do not revive a parallel chaptera-update-core authority.

## Required acceptance

- Journal current/next/prev candidates carry a monotonic install-root generation and checksum.
- Recovery chooses the highest unambiguous valid generation and fails closed on competing active attempts.
- Journal binds product, architecture, channel, from/to package version, install_layout_epoch, update_protocol_version, update_mode, state-schema / rollback-compatibility identity, previous-tree digest, and candidate-tree digest.
- Recovery recomputes exact tree identity and classifies current/staging/rollback as Previous, Candidate, Missing, or Unknown.
- Ambiguous/corrupt topology becomes typed RepairRequired.
- Port the useful laws from donor PR #904: authenticated tree manifest, path/size/hash/file-count/total-byte verification, traversal and Windows-path rejection, collision checks, symlink/reparse rejection, extra/missing/tampered file rejection, bounded staging, and conservative same-volume disk-space preflight.
- Compose existing chaptera-update-trust semantics; do not duplicate trust authority.
- payload_swap only proceeds after policy validation.
- installer_required is typed and is not treated as retained-tree payload swap.
- Automatic rollback is allowed only for an authenticated rollback-compatible release edge.
- Preserve public behavior and acceptance established by #910, #911, and #912 on Windows and Ubuntu.

## Non-goals

- Reader UI or Reader-specific process discovery.
- Public signing/distribution.
- A second TUF client.
- A second updater architecture.
- Native PUB/document semantics.

## Proven prerequisites

- #903 metadata trust: landed.
- #910 update transaction engine: landed.
- #911 updater orchestration: landed.
- #912 control handoff: landed.
- #904: donor/provenance only, not merge authority.


## Fresh-main replay receipt — 2026-09-27

- Replayed from current `main` base `426dee38bea50b7c011f3a6f693a0ea0ebbaa4bc` after updater-engine overlap was detected.
- Preserves landed copied-U1 terminal lifetime and next-lock-owner `cleanup_orphaned_transactions()` semantics.
- Reapplies journal generation/checksum/high-water recovery and authenticated bounded staging on top of that base.
- PR #919 was automatically closed when the branch temporarily equaled `main` during force-reset, then reopened after replay; authoritative replay head before this receipt was `1bee323d76ff475b8e1cc3edeb51adaf29a33817`.


## Clean fresh-main successor — 2026-09-28

- Reconstructed the task-owned delta from the first-parent diff `1c4bdaf4eed6811565a549f03b85cbc8e3b0f133..9f9904efa31698abc10f206c91895c3dfc3ed1f3`, excluding unrelated AUTONOMUS/research merge-history noise from stale PR #919.
- Fresh base: `c81f3fe895322a5cf3bb9407887ac8c11b226b88`.
- Eleven of the twelve task-owned paths were byte-identical to the old clean base and replayed directly.
- The only current-main overlap was `crates/chaptera-update-trust/src/lib.rs`: current main already owns verified TUF payload receipt/read/verification logic. The successor preserves that newer authority and composes the #919 `ReleaseDecision::PayloadSwap/InstallerRequired` classification plus authenticated rollback-edge tests on top.
- Do not reintroduce the 20 unrelated research/boundary files shown by the stale PR three-dot diff.
