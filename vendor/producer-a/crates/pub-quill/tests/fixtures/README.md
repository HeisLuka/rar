# PUB-QUILL-TOKN-01 fixture provenance

These fixtures are pinned public Apache POI Publisher regression files used only
for preservation-grade Quill TOKN reader acceptance.

Upstream repository: `apache/poi`
Pinned commit: `942d95d85b15d0dfdb3bc9ba1b4f273f277757c8`

| Local fixture | Upstream path | Git blob SHA |
| --- | --- | --- |
| `60685.pub.b64` | `test-data/publisher/60685.pub` | `37d3dce70e1786e948edd5592527f90be1fae2ff` |
| `LinkAt10.pub.b64` | `test-data/publisher/LinkAt10.pub` | `89c4a4427197129d30e085e2cc88ef3fdcb9562f` |

The files are stored as base64 text to match the existing vendored test-fixture
pattern. Tests decode them in memory and read only
`/Quill/QuillSub/CONTENTS`.

Evidence boundary:
- `60685.pub` is the historical non-URL Type12 counterexample; it must parse
  through the generic TOKN carrier, not a hyperlink-specific offset heuristic.
- `LinkAt10.pub` is a controlled URL-bearing Type12 fixture.
- Fixture presence does not authorize a TOKN writer, FDPC clickable-range
  semantics, Hyperlink entity construction, or COM PageID interpretation.
