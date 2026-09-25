# FALSE-OMISSION-01 packet provenance

This packet is a bounded restoration for PUB-T-646. It prepares the native
FALSE-OMISSION-01 experiment but does not itself produce any writer or format
claim.

Historical donor provenance:

- HeisLuka/pub-rs PR #140, head
  `66dfb4f57bcbdf14f8e8dd17f4bfc07e2a1fb278`: fixture semantics and
  `Invoke-FalseOmission01Adapter.ps1`.
- HeisLuka/pub-rs PR #141, head
  `3620c3cb8bffaca95f490b58cd862ff2ab6aae7f`: bounded materialization audit
  and COM/Escher identity audit.
- HeisLuka/pub-rs PR #161, head
  `f90da3e1b92eb4031168b01bdb682595271eea80`: negative namespace evidence
  that keeps ClientData.ShapeId and FSP.spid separate.

Restoration boundary:

- `New-FalseOmissionShapeFixture.ps1` creates exactly one rectangle with
  `PUB_ORACLE_ID=FALSE_OMISSION_TARGET`, geometry
  `Left=73 Top=91 Width=181 Height=103`, and both
  `Fill.Visible=False` / `Line.Visible=False`.
- `Invoke-FalseOmission01Adapter.ps1` performs only the control, fill-on, and
  line-on semantic arms and reopens the output to validate semantics.
- `false_omission_materialization_audit.py` is a bounded raw candidate audit.
  Its positive result is still not COM-to-wire semantic proof.
- `false_omission_com_escher_identity.py` tests only a file-scoped
  COM Shape.ID ↔ ClientData.ShapeId candidate and explicitly keeps FSP.spid in a
  separate namespace.
- `false_omission_packed.py` contains only the proven 11-bit packed-header and
  block-bound helpers required by the raw audit.

No generated PUB bytes are committed by this packet. Native Publisher execution
belongs to the parent FALSE-OMISSION-01 task.
