# OFFICEART-CALLOUT-READ-01

This hosted-safe observer consumes the existing bounded `pub-escher`
SpContainer/FOPT inventory. It does not create another OfficeArt parser and it
does not promote Publisher semantics.

Current MS-ODRAW defines the bounded observable Callout family used here as:

- `0x0341 / dxyCalloutGap`;
- `0x0342 / spcoa`;
- `0x0343 / spcod`;
- `0x0344 / dxyCalloutDropSpecified`;
- `0x0345 / dxyCalloutLengthSpecified`;
- `0x037F / Callout Boolean Properties`.

The boolean observation preserves both the seven `fUse...` bits and their
seven value bits. An absent use bit remains unresolved/default; it is never
materialized as explicit false.

Known `spcoa` and `spcod` enum values may be named in the receipt, but the
raw scalar is always preserved and unknown values remain unknown.

## Important current-spec correction

Current MS-ODRAW defines `0x0340` as `unused832`, undefined and ignored.
Older Office drawing tables/tools label PID 832 as `spcot / Callout type`.
That historical name is useful archaeology, but this observer deliberately
does not decode `0x0340` as Callout.Type. The Publisher
`Callout.Type <-> persisted carrier` join remains open in
`PUB-T-855 / CALLOUT-AUTH-01`.

## Semantic firewall

This tool does not claim that Publisher `CalloutFormat.AutoAttach`,
`AutoLength`, `Drop`, `Gap`, `Length`, `Angle`, `Type`, `Accent`,
or `Border` maps to a specific raw field merely because MS-ODRAW names a
related OfficeArt property. It does not infer callout paths/routing, create a
canonical CalloutPolicy, or add writer/rewrite support.

The CLI accepts a raw OfficeArt stream file and emits only a logical stream
name plus raw observation provenance. It does not emit the local input path or
document bytes.
