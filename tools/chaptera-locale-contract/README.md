# CHAPTERA-VTPE-LOCALE-01 — public Windows validation slice

This directory validates the source-neutral locale contract used by the Chaptera Reader
market/profile boundary. It intentionally does **not** contain or import the private Chaptera
desktop implementation.

The Windows Actions job proves:

- `GetUserDefaultLocaleName` is callable on the current `windows-latest` runner and records the
  returned user locale;
- source precedence is override → OS locale → `LANG`;
- locale normalization is trim → `_` to `-` → lowercase → remove the first `.encoding` or
  `@modifier`;
- only normalized `en-us`, `en-gb`, and `ru-ru` select US, UK, and Russia;
- every other locale fails closed to `NeutralEnglish`;
- no network/IP/geolocation/timezone/account input is used by this public validation probe.

A green receipt is evidence that the public contract and real Windows API seam are healthy.
It is **not** a claim that a private Chaptera desktop binary has incorporated the implementation.
That product-integration claim requires a separate authorized private/local producer receipt.
