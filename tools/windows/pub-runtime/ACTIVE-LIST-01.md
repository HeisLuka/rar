# ACTIVE-LIST-01

Bounded native experiment seam for `PUB-T-202 / U-QUILL-07`.

## Question

Can Publisher's public `ParagraphFormat` list axes be mutated one at a time, survive Save + fresh reopen, and produce a byte-pinned private output family suitable for the already-known OplPap/OplQListFormat carrier join?

This first seam deliberately does **not** claim the private-field mapping from COM values alone. The exact field coordinates are already known from prior Publisher registry/XML work; this packet only creates the causal native arms needed to test value and omission laws.

## Exact environment

- packet: `tools/research-runner/experiments/active-list-01.packet.json`
- environment: `publisher-2019`
- exact blank: `pubgen-create-20260923/minimal-blank-v1-generated.pub`
- blank SHA-256: `5bf6057b8b11c8ee4a421d93885ae6e9e7c03a7a42d541e6d0df33497c08c33b`
- Publisher EXE SHA-256: `e1ef8811b85b82045f37c4173b92726101be3a25e550b0dcb9f178df834ab20b`
- expected Publisher version prefix: `16.0.12527.`

Every arm starts from a fresh copy of the same one-page blank, creates the same tagged text box at the same geometry, writes the same text, applies one bounded list mutation family, saves to a private output PUB, closes Publisher, opens a fresh process, and records the reopened COM state.

## Arms

- `C0`: plain no-list control.
- `T0`, `T1`, `T22`: list-type representatives Arabic, Uppercase Roman, Arabic Leading Zero.
- `BD`: bullet list with Publisher's default bullet.
- `BT_STAR`, `BT_HASH`: explicit one-character bullet text controls.
- `SEP0..SEP8`: all nine documented `PbListSeparator` values under Arabic numbering.
- `N1`, `N4`, `N37`: independent list-start values under Arabic numbering.
- `FN_ARIAL`, `FN_VERDANA`: bullet font-name arms.
- `FS12`, `FS24`: bullet font-size arms.

The public API values are inputs and COM observations only. Numeric equality with a private wire scalar is not assumed.

## Existing private-field targets

The experiment is designed around already-localized fields rather than rediscovering coordinates:

- `OplPap.0x257 ListFormat` → nested `OplQListFormat`
  - `0x200 Type`
  - `0x201 BulletText`
  - `0x202 BulletLcid`
- `OplPap.0x258 ListSepSimple`
- `OplPap.0x215 ParaNumber`
- `OplPap.0x202 FpsList` candidate for bullet size
- `OplPap.0x203 FtcList` and/or `0x22F StzListFont` for bullet font name

Those identities come from prior evidence. This packet does not promote API↔field value laws until the private outputs are inspected owner-scope-correctly.

## Evidence

For every arm the public source-safe receipt records:

- requested semantic arm and API value;
- mutation success/error + HRESULT;
- Shape.ID and stable oracle tag;
- ListType;
- ListNumberSeparator;
- ListNumberStart;
- ListBulletText;
- ListBulletFontName;
- ListBulletFontSize;
- state before Save, after Save, and after fresh reopen;
- whether the targeted API value round-trips;
- private output PUB size/SHA-256 receipt.

Generated PUBs remain under the native runner's private evidence directory.

## Decision / follow-up

A successful native run creates the causal output family for the second-stage structural join:

1. compare each mutated output against its matched control;
2. use the existing packed-tag / bounded Contents readers;
3. localize deltas only in the already-known OplPap/OplQListFormat owner scope;
4. test value equality and omission/default behavior;
5. separately verify that the raw 0x4A built-in preset bank stays byte-identical.

If a public API value fails to survive fresh reopen, record that behavior first; do not force a private-field interpretation.

## Evidence boundary

This seam may establish COM-visible mutation/reopen laws. It may **not** by itself claim:

- exact API numeric value = private persisted scalar;
- omission/default law;
- `0x4A` immutability;
- full list fidelity across other Publisher versions, scripts, or CJK/RTL list families.

Those require the private owner-scoped byte join after native execution.
