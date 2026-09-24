# FIDELITY-REFERENCE-PDF-01 — external PDF visual oracle

This adapter uses a pinned mature MuPDF renderer through PyMuPDF. It does not implement PDF parsing or rasterization.

The receipt first records artifact hashes, page count and exact page boxes. Each aligned page is rasterized under an explicit fixed environment and compared with:
- raster hashes;
- significant-pixel count/fraction after a bounded per-channel threshold;
- mean/max channel delta;
- a localized significant-difference bounding box;
- coarse connected difference regions.

There is intentionally no single opaque pass/fail fidelity score. Low-level renderer/antialiasing differences remain visible as metrics and can be separated from large placement/missing-region differences.

The contract fixture proves the reusable adapter only. A customer/sample-specific Chaptera PDF vs external reference PDF remains a consuming receipt and must retain its own artifact provenance.
