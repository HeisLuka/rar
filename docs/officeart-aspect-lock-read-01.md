# OFFICEART-ASPECT-LOCK-READ-01

This hosted-safe observer consumes the existing bounded `pub-escher`
`SpContainer/FOPT` inventory. It does not parse OfficeArt independently.

For MS-ODRAW Protection Boolean Properties `0x007F`, it preserves:

- the raw FOPTE scalar and source span;
- `fUsefLockAspectRatio`;
- `fLockAspectRatio`;
- a derived `aspect_lock: Option<bool>`.

The derived value is deliberately tri-state:

```text
use bit absent      -> None
use bit present + 0 -> Some(false)
use bit present + 1 -> Some(true)
```

A raw value bit without its use bit is retained in the receipt but is not
promoted to an effective value.

## Semantic firewall

This tool does not claim that Publisher `Shape.LockAspectRatio` writes or
resolves through this property. It does not decide the ratio baseline,
rebasing law, defaults by shape class, resize behavior, UI policy, or writer
semantics. Those remain with `PUB-T-846 / SHAPE-ASPECT-LOCK-AUTH-01` and its
native Save/reopen causal experiment.

The command accepts a raw OfficeArt stream file and emits only a logical
stream name plus raw observation provenance. It does not emit the local input
path or document bytes.
