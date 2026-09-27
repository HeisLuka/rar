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
