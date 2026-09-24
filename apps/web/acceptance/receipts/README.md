# WEB-ACCEPTANCE-01 real receipt slots

This directory intentionally contains **no synthetic closure receipt**.

The final product gate requires:

- `viewer-geometry.real.json` — sanitized real `ViewerGeometryDocument` emitted by canonical private core from the pinned real PUB;
- `../../../../packages/protocol/revision/v1/producer-receipts/sample-newsletter.real.json` — sanitized canonical EditorSession revision receipt with `core_integration=true`;
- `browser-acceptance.real.json` — real headless-browser end-to-end receipt validated by `browser-acceptance-receipt.schema.json`.

Unit fixtures, synthetic Scene V1 fixtures, manually authored receipts, screenshots without semantic evidence, or browser runs over mocked Scene JSON cannot occupy these slots.


## Viewer receipt privacy allowlist

Before a real Viewer receipt can participate in preflight it must validate against
`../viewer-geometry-receipt.schema.json` via:

```bash
python tools/validate_viewer_geometry_receipt.py apps/web/acceptance/receipts/viewer-geometry.real.json
```

The v0.1 receipt schema is an explicit public allowlist for the current source-free
`ViewerGeometryDocument` shape. Unknown fields fail closed. In particular, raw image
`bytes`, local/private paths, unreviewed resource fetch handles, parser carriers, and
other producer-private additions cannot silently pass merely because the Scene adapter
ignores them. A future richer producer must deliberately revise the public receipt
contract rather than widening this slot implicitly.


## Browser receipt canonical binding

The browser receipt schema is intentionally strict, but schema-valid booleans are not
sufficient product evidence. `validate_browser_acceptance_receipt.py` binds the final
receipt to the already-validated canonical Producer B receipt and the real initial Scene:

- selected NodeId, client operation id and before/after RectEmu must equal canonical MoveNode;
- MoveNode must preserve width/height, matching canonical EditorSession semantics;
- Undo and Redo RevisionIds are recomputed from the public immutable-history hash law;
- accepted/undo/redo Scene snapshot ids are recomputed from the real initial Scene,
  canonical MoveNode geometry and those exact revision ids;
- reopen must reproduce the final persisted Redo Scene and therefore the same snapshot id.

This keeps Producer C from closing the gate with plausible but unbound hash-shaped ids.
The export artifact itself remains private/local by the current privacy fence; its hash
and local geometry verification stay producer evidence rather than a public artifact.
