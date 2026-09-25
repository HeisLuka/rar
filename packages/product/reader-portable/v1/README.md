# Chaptera Reader — portable technical preview

Contract: `chaptera.reader-portable-readme.v1`

This package contains the read-only Chaptera Reader application.

## Boundary

- opens supported Publisher files through the current Viewer path;
- keeps the source PUB read-only;
- exposes Reader/view/diagnostic behavior only;
- does not enable Editor controls, Editor Project save/replay, editable export, or native Save PUB;
- does not require Cargo, a repository checkout, or a Chaptera server to launch.

This is a bounded technical preview, not a claim of full Microsoft Publisher visual or format compatibility.
