# PUB-T-163 — Quill FONT / FDPC read-only discriminator

This slice is intentionally narrower than full FONTEMB-01.

It reproduces the current libmspub Quill grammar at the byte level:

- linked Quill chunk descriptor lists;
- FONT stored count, index area, every record start/end, UTF-16LE name and trailing u32;
- exact FONT terminal-boundary accounting;
- FDPC character-style entries;
- font-index container 0x24 and its libmspub-compatible nested GENERAL_CONTAINER extraction;
- FDPC font index -> Quill FONT vector ordinal -> decoded name join.

The GitHub Actions job uses only public inputs: the Mass.gov Rights Review publication
and the pinned Apache POI SampleNewsletter fixture. Original PUB bytes are temporary
workflow inputs and are never uploaded as artifacts. Only source-free JSON receipts are retained.

A green run closes only the immediate read-only grammar/index discriminator. It does not
claim the full FONTEMB-01 DoD (none/full/subset A/AB/ABC, embedded payload identity,
or subset glyph-set behavior).


## Rights Review acquisition fence

The preferred route remains the exact-title link discovered from the public Mass.gov DDS landing page.
If that landing page rejects the GitHub-hosted runner, the workflow may try the title-derived
Mass.gov direct-document route only as a transport fallback. The fallback is accepted only when the
download is OLE/CFB and its SHA-256 is exactly
`d31129823c0b27aeabe8132fabc4038ce7bf59be68785b7306d1bda53928c223`, the already registered
Rights Review fixture identity from PUB-EV-74. A different file, redirect payload, HTML error page, or
same-title substitute fails closed.
