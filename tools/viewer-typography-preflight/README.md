# VIEWER-TYPOGRAPHY-PROJECTION-01 — bounded Quill typography preflight

This slice is deliberately smaller than a full Reader typography implementation.
It runs on pinned public real PUB fixtures and retains only source-free JSON.

It proves and inventories:

- exact Quill `FONT` record order and ordinal -> family-name mapping;
- `FDPC` font-index observations without claiming Story/run ownership;
- the second `STSH` layer as paired default character/paragraph state;
- even `STSH1` records as default character styles;
- character property `0x0C` as text size in EMU/points;
- nested `0x24` entries as script-slot -> `FONT` ordinal -> family-name references.

It intentionally does **not** assign those observations to Viewer Story/run objects yet.
The receipt closes the bounded observation with zero proven eligible Stories, leaves coverage explicitly ambiguous/not computable, and names the exact next
join discriminator: reproduce the FDPC/BTEC aggregate-TEXT coordinate join, split it at
SYID+STRS Story boundaries, and only then project unambiguous source typography.

No host-font discovery or substitution is performed. Raw PUB and Quill bytes are workflow-temporary
and must never be uploaded as artifacts.
