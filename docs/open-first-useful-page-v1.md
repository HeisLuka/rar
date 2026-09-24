# OPEN-FIRST-USEFUL-PAGE-01 — source-neutral Reader open receipt

This receipt measures the user outcome **open → first useful page** separately from **open → fully ready**.

“First useful page” is not an empty window or parsed manifest. The receipt requires:

- real page geometry;
- current fidelity diagnostics;
- required visible resources;
- current text layout;
- non-empty visible content.

Each run records phase timing and scope for source/container open, profile classification, stream reads, parse/model projection, first-page dependency resolution, first-page layout/Scene, visible resource decode, first paint, remaining-document background work and search readiness.

Every phase also records available bytes/materialization and page/story/resource touch counts. A phase may be classified as first-page-only, bounded dependency work, document-global work or background work. The validator derives which document-global phases started before the first useful page. That fact is a **discriminator**, not automatic permission to introduce lazy parsing.

Final canonical document, Scene and search projection equivalence with the complete path is mandatory.

The checked-in fixture is explicitly synthetic and has `architecture_decision_allowed=false`. A lazy/current-page-first parser/model/layout architecture may be justified only by a sanitized `real_pub_source_free` receipt from the authorized Reader/Viewer runtime.
