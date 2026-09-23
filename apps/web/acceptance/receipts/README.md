# WEB-ACCEPTANCE-01 real receipt slots

This directory intentionally contains **no synthetic closure receipt**.

The final product gate requires:

- `viewer-geometry.real.json` — sanitized real `ViewerGeometryDocument` emitted by canonical private core from the pinned real PUB;
- `../../../../packages/protocol/revision/v1/producer-receipts/sample-newsletter.real.json` — sanitized canonical EditorSession revision receipt with `core_integration=true`;
- `browser-acceptance.real.json` — real headless-browser end-to-end receipt validated by `browser-acceptance-receipt.schema.json`.

Unit fixtures, synthetic Scene V1 fixtures, manually authored receipts, screenshots without semantic evidence, or browser runs over mocked Scene JSON cannot occupy these slots.
