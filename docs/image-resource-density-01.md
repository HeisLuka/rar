# IMAGE-RESOURCE-DENSITY-01

This layer derives bounded physical-density metadata from exact PNG/JPEG bytes
that already pass `EDITOR-ASSET-INTRINSIC-01` admission.

It preserves source-specific evidence before resolution:

- PNG `pHYs`: X/Y pixels-per-unit and unit specifier;
- JPEG JFIF APP0: density unit and X/Y density;
- JPEG Exif/TIFF: `XResolution`, `YResolution`, `ResolutionUnit`.

No 72/96 DPI fallback exists.

Physical values use exact rational arithmetic. PNG pixels-per-meter and
centimeter-based JPEG metadata are converted to pixels-per-inch only as exact
derived rationals. Asymmetric X/Y density is preserved.

If multiple physical sources agree exactly, the descriptor reports a
consistent resolved density while retaining every source. If JFIF and Exif
disagree, the result is `conflicting_physical_sources` and no resolved DPI is
selected.

Unitless pHYs/JFIF/Exif evidence is retained but never becomes a physical DPI.

## Authority boundary

This task does not claim that Publisher uses source density as its
`RelativeToOriginalSize` baseline. `PUB-T-851 / PICTURE-ORIGINAL-SIZE-AUTH-01`
owns that native causal question. The density descriptor is only an exact
source-side input to that experiment and to the state-transition comparator.

No resampling, print-size policy, color conversion, EXIF orientation rewrite,
image mutation or transcoding occurs here.
