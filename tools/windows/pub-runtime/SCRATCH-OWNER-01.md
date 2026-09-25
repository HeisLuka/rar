# SCRATCH-OWNER-01

Bounded native experiment for `PUB-T-129 / COM-ORACLE-04`.

## Question

Does Publisher persist `Page.Shapes` and `Document.ScratchArea.Shapes` as distinct semantic owner domains across Save -> reopen, and does COM `Shape.ID` survive that boundary?

This packet deliberately does **not** claim the raw Contents/Escher owner field. It emits only COM-side owner membership, stable tags, geometry, `Shape.ID`, `IsExcess`, and whole-file hashes. Raw Contents/Oid/seqNum/SPID attribution remains a separate parser join.

## Exact input

The packet reuses the pinned generated blank seed:

- path on the native runner: `pubgen-create-20260923/minimal-blank-v1-generated.pub`
- SHA-256: `5bf6057b8b11c8ee4a421d93885ae6e9e7c03a7a42d541e6d0df33497c08c33b`
- Publisher executable SHA-256: `e1ef8811b85b82045f37c4173b92726101be3a25e550b0dcb9f178df834ab20b`

## Arm

The operation creates two geometrically identical rectangles:

- `PUB_ORACLE_ID=SCRATCH_OWNER_PAGE` in `Page.Shapes`
- `PUB_ORACLE_ID=SCRATCH_OWNER_SCRATCH` in `Document.ScratchArea.Shapes`

It records owner membership before Save, after Save, and after fresh reopen.

## Evidence boundary

Saved PUB bytes remain under the native runner's private evidence directory and are not uploaded by the generic workflow. Public evidence is limited to:

- `environment.json`
- `analysis/scratch-owner-01.json`
- `logs/scratch-owner-01.txt`

Run through `.github/workflows/pub-native-research.yml` with:

- packet: `tools/research-runner/experiments/scratch-owner-01.packet.json`
- environment: `publisher-2019`

A positive COM result is still not enough to name a physical owner field. Follow-up must join the private saved PUB to the existing parser/Escher/Oid tooling.
