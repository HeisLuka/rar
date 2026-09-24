# FIDELITY-REFERENCE-PDF-01 — external PDF visual oracle

This adapter uses a pinned mature MuPDF renderer through PyMuPDF. It does not implement PDF parsing or rasterization.

The receipt first records both artifact hashes, page count, render/media/crop boxes and rotation through MuPDF. Each aligned page is rasterized under an explicit fixed environment and compared with:
- raster hashes;
- raw changed-pixel count;
- below-threshold noise-only pixel count/fraction;
- significant-pixel count/fraction after a bounded per-channel threshold;
- mean/max channel delta;
- a localized significant-difference bounding box;
- a compact row-major tile-RLE significant-difference mask;
- coarse connected significant-difference regions.

There is intentionally no single opaque pass/fail fidelity score. Raw raster disagreement and below-threshold renderer/antialiasing noise remain visible separately from material thresholded regions.

The contract tests cover identical PDFs, moved-object localization, page-box mismatch, page-count mismatch and an explicitly subthreshold raster delta. The contract fixture proves the reusable adapter only. A customer/sample-specific Chaptera PDF vs external reference PDF remains a consuming receipt and must retain its own artifact provenance.

Limitations: visual agreement does not prove authoring-semantic equivalence, the compact mask is tile-granular rather than a semantic pixel mask, and PDF/X/object-level conformance is outside this gate.
