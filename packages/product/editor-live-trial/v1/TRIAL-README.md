# Chaptera Editor — 5-minute portable trial

Contract: `chaptera.editor-live-trial-readme.v1`

This ZIP is a bounded Chaptera Editor trial build. It is not the public Reader installer and it does not claim full Microsoft Publisher compatibility.

## Before you start

- Keep your original `.pub` file as the source. Chaptera does not overwrite it in this trial.
- Work is saved as a Chaptera Editor Project beside the source or at a location you choose.
- Unsupported operations should remain disabled or produce an explicit capability/loss message.
- Native **Save PUB** is not part of this trial.

## 5-minute path

1. Unzip the package to a normal writable folder.
2. Launch `Chaptera-Editor.exe` directly; no Rust/Cargo/dev toolchain should be required.
3. Open a copy of a Publisher file you are comfortable testing.
4. Edit one ordinary supported text Story.
5. Select one supported page-owned object and drag it to a new position.
6. Use Undo, then Redo.
7. Save the Chaptera Editor Project.
8. Close Chaptera.
9. Reopen the same source PUB plus the saved Editor Project.
10. Confirm the text edit and moved geometry are still present.

## What to report

If something fails, note the visible capability/error state and the exact step number. Do not send confidential files through public issue trackers.

## Not in this trial

- generic native Save PUB;
- full Publisher parity;
- code signing / installer / auto-update;
- rotate/group/z-order parity;
- broad linked-text reflow, master-page authoring, print/prepress parity.
