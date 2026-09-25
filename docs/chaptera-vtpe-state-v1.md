# CHAPTERA-VTPE-STATE-01 — Rar validation V1

The active `HeisLuka/rar` desktop already contains the historical VTPE state slice. This task therefore closes by validating current Rar code, not by replaying or porting the old `yab#221` branch.

## Current production wiring

`chaptera-desktop` pins `eframe = 0.31.1` with the `persistence` feature.

The app uses the versioned key `chaptera.supporter.v1`:

- `eframe::run_native` passes `CreationContext.storage` into `ViewerApp::new_with_storage`;
- startup restores the exact key through `SupporterState::from_json_str`;
- malformed or unknown schema falls back to default state;
- `eframe::App::save` writes only the supporter JSON under that key.

The persisted model contains cadence timestamps/counters only. It contains no filename, filesystem path, source hash, document text or document identifier.

## Cadence contract

The current `supporter.rs` tests prove:

- first meaningful success is immediately eligible;
- after a shown prompt, the next prompt requires both at least 7 days and at least 3 new meaningful successes;
- high-salience prompts are capped at 2 in a true rolling 30-day window;
- the second Later/dismissal suppresses for 45 days;
- Already supported suppresses for 180 days;
- explicit Support click suppresses for 30 days without claiming payment;
- clock rollback suppresses conservatively;
- malformed/future persisted schema fails closed.

## Rar-owned persistence proof

The validation adds an in-process `eframe::Storage` test double and exercises the real desktop wiring:

1. seed `chaptera.supporter.v1` with a state carrying second-dismissal suppression;
2. construct `ViewerApp::new_with_storage`;
3. prove suppression survives restore;
4. mutate supporter state;
5. invoke the real `eframe::App::save`;
6. prove exactly one versioned key is persisted and it round-trips to the exact state;
7. prove malformed/future storage restores the conservative default.

No network, provider/payment logic or document-derived identity is introduced.
