# OFFICEART-INSET-PEN-READ-01

This hosted-safe observer consumes the existing bounded `pub-escher`
SpContainer/FOPT inventory. It does not create another OfficeArt parser.

For Line Style Boolean Properties `0x01FF`, it preserves the raw scalar and
FOPTE provenance and exposes:

- `fUsefInsetPen`;
- `fUsefInsetPenOK`;
- `fInsetPen`;
- `fInsetPenOK`;
- a fail-closed `inset_pen: Option<bool>`.

The effective value is admitted only when both use gates are explicit and the
raw `InsetPenOK` value allows the property. Raw bits remain visible even when
the effective value stays unresolved.

## Semantic firewall

This observer does not claim that Publisher `Line.InsetPen` writes these
bits, does not infer inside/outside ink geometry, and does not promote a new
canonical Stroke field. Native causality and geometry remain with
`PUB-T-845 / SHAPE-INSET-PEN-AUTH-01`.

No table-border, BorderArt, default synthesis, or writer/rewrite semantics are
introduced here. The CLI does not include local input paths or document bytes
in its JSON output.
