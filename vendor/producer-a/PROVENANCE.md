# Producer A bounded donor runtime slice

Primary source provenance: `HeisLuka/yab@2416bf1af401caa997e588b41285816422a7d59b`.

This directory started as the minimum reverse/viewer runtime cut needed by
`open_mature_0x2c_geometry`. `EDITOR-DESKTOP-APP-01` extends that bounded donor
slice with only the historical editor/interaction/export crates required to
build the first current Chaptera desktop product in `HeisLuka/rar`:

- `pub-editor`
- `pub-interaction`
- `pub-export`
- `pub-idml`
- `pub-odg`
- `pub-writer`

The historical repository is not a build or runtime dependency. These sources
are vendored implementation material only. Current product authority remains in
Rar, including scene-instance identity/mutation admission and the desktop
acceptance contracts.

`vendor/producer-a` intentionally remains a nested Cargo workspace and is
excluded from the root Rar workspace package namespace. Its historical
`pub-model` package uses donor-only version `0.1.0-donor` solely to avoid a
Cargo.lock identity collision with the current root `pub-model`; the Rust crate
API name remains `pub_model`.

Raw real-PUB fixture bytes are not part of this donor extension. Exact document
acceptance remains a separate controlled input/receipt boundary.

This is a transitional donor slice, not a second canonical repository.
