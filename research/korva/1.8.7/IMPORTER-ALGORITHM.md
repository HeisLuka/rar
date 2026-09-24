# Korva 1.8.7 PUB importer algorithm reconstruction

Status: bounded static reconstruction from the exact pinned Linux binary. This is **not** a fidelity benchmark and **not** source-genealogy proof.

## Artifact identity

- Package: `Korva_1.8.7_amd64.deb`
- Package SHA-256: `e0a2abde284c4c714262c07afab168659387a797f033a869d0db1b1f4bfa010c`
- Main ELF SHA-256: `87e10c427ab00d3d9f5bcee78ee0e2acb11063358243975bcc442e905c04ae3d`
- Parent reproducible static receipt: [RECEIPT.md](./RECEIPT.md)
- Successful deep provenance run: [GitHub Actions 35950334562](https://github.com/HeisLuka/rar/actions/runs/35950334562)

## High-confidence pipeline

The binary supports this bounded reconstruction:

```text
input .PUB bytes
  -> cfb::CompoundFile::open(Cursor<Vec<u8>>)
  -> enumerate CFB entries
  -> case-insensitive / fallback stream-name matching
       -> Quill/QuillSub/CONTENTS
       -> EscherDelayStm
  -> open selected stream and read it fully into Vec<u8>

Quill CONTENTS bytes
  -> extract_text
  -> heuristic UTF-16LE text mining
  -> prose / CJK filtering
  -> control cleanup + whitespace normalization
  -> normalized strings
  -> new Pagewell Paragraph objects using Helvetica 12 pt

EscherDelayStm bytes
  -> extract_images
  -> OfficeArt/Escher record-guided BLIP search
  -> JPEG/PNG signature parser
  -> if structural pass finds none: whole-stream bytewise carving
  -> new Pagewell Asset objects with base64 data

normalized text + carved images
  -> Document::blank(612, 792)
  -> synthetic image grid
  -> synthetic body frames
  -> pagewell_core::layout::build_page_scene fit probes
  -> binary-search / fragment text into frames/pages
  -> reconstructed Pagewell/Korva document
```

The important boundary is that the importer reconstructs a new Pagewell document from extracted content. The analyzed functions do not show recovery of the original Publisher page/object/layout graph.

## 1. CFB stream selection and reading

### `import_pub_bytes`

Exact symbol:

- `pagewell_core::import_pub::import_pub_bytes`
- start `0x1d0eee0`
- stop `0x1d12ba0`

It directly calls:

- `cfb::CompoundFile<Cursor<Vec<u8>>>::open`

Within the exact function there are live xrefs to:

- `Quill/QuillSub/CONTENTS`
- `EscherDelayStm`

These are not merely global strings: the xrefs are inside `import_pub_bytes`.

### `closure#3`

`pagewell_core::import_pub::import_pub_bytes::{closure#3}` at `0x1d0b2f0` performs stream-name matching.

Static behavior recovered:

- lowercases the requested stream name;
- lowercases candidate CFB entry names;
- contains exact length + byte equality comparison;
- also contains a looser substring/path-style fallback.

Safe conclusion: stream lookup is tolerant/case-insensitive rather than a rigid exact-path-only lookup.

### `closure#4`

`pagewell_core::import_pub::import_pub_bytes::{closure#4}` at `0x1d0c540`:

- calls `CompoundFile::open_stream::<&str>`;
- reads the opened stream through Rust `Read` machinery into a `Vec<u8>`.

So both target streams are materially read into memory for local parsing.

## 2. Quill text extraction is heuristic mining, not a recovered Quill object model

`pagewell_core::import_pub::extract_text` starts at `0x1d0dcb0`.

### Candidate generation

The function walks the raw stream in 2-byte units and forms little-endian `u16` values.

It accumulates contiguous plausible UTF-16 code units and flushes runs on implausible values. Runs with at least three UTF-16 units can be converted with `String::from_utf16_lossy`.

This is raw text-carving behavior over the stream bytes, not evidence of Quill record-level semantic decoding.

### Prose / CJK filter

`extract_text::{closure#1}` at `0x1d147c0` filters candidate strings.

A candidate passes the alphabetic-prose branch when:

- alphabetic Unicode character count is at least **25**; and
- the string contains an ASCII space.

Alphabetic classification uses Rust Unicode tables, including `core::unicode::unicode_data::alphabetic::lookup_slow`.

If that branch does not accept the candidate, another branch counts characters in CJK/Hangul/ideograph-heavy ranges:

- U+3000..U+9FFF
- U+AC00..U+D7A3
- U+F900..U+FAFF
- U+20000..U+2FA1F

That branch accepts at **10 or more** matching characters.

### Best-candidate fallback

If the normal filter yields no useful candidate, `extract_text` selects the candidate with the largest alphabetic-character count and keeps it only when that count is at least **3**.

### Normalization

Accepted strings are split on code points in the LF/VT/FF/CR group.

`sanitize_line` maps:

- U+0000..U+001F -> ASCII space
- U+007F..U+009F -> ASCII space
- U+FFFD -> ASCII space

The resulting text is then processed by whitespace splitting and recombined with single spaces.

### Consequence

The analyzed binary strongly supports this interpretation:

> Korva 1.8.7 does not reconstruct Publisher's Quill text object/formatting graph here. It mines plausible UTF-16LE prose from the entire `CONTENTS` byte stream, cleans it, and converts it into new Pagewell paragraphs.

That conclusion is specific to the analyzed importer path and must not be generalized to private source revisions not represented by this binary.

## 3. Escher image extraction is a structural BLIP-guided scan plus carving fallback

`pagewell_core::import_pub::extract_images` starts at `0x1d0e9e0`.

### First pass: 8-byte Escher record headers

The loop reads an 8-byte record header shape:

- 16-bit record type at offset +2;
- 32-bit payload length at offset +4.

The record-type test accepts the inclusive range:

- `0xF018..0xF117`

Microsoft's MS-ODRAW specification identifies this range as `OfficeArtBlip` in an `OfficeArtBStoreContainerFileBlock`:

- https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-odraw/a7d7d967-6bff-489c-a267-3ec30448344a

For a selected record, Korva scans only an early bounded prefix of the record payload (up to roughly 80 bytes), invoking `sniff_image` at successive byte offsets.

This is consistent with BLIP records where UID/tag fields precede embedded file bytes. For example, MS-ODRAW's `OfficeArtBlipPNG` describes an 8-byte record header followed by one or two 16-byte UIDs, a one-byte tag, then PNG file data:

- https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-odraw/7af7d17e-6ae1-4c43-a3d6-691e6b3b4a45

The structural pass is bounded to 10,000 record iterations.

### `sniff_image`

`pagewell_core::import_pub::sniff_image` starts at `0x1d0d730`.

It recognizes exactly two file families in this path:

**JPEG**
- checks `FF D8 FF`;
- walks JPEG markers;
- detects dimensions from SOF markers;
- seeks EOI;
- returns MIME `image/jpeg`.

**PNG**
- checks exact PNG signature `89 50 4E 47 0D 0A 1A 0A`;
- walks PNG chunks;
- reads dimensions from IHDR;
- stops on IEND;
- returns MIME `image/png`.

No equivalent branch for WMF/EMF/TIFF was observed in this function.

### Second pass: whole-stream carving

If the structural Escher pass produces zero images, `extract_images` falls back to a bytewise scan across the complete `EscherDelayStm`.

For each offset:

- call `sniff_image`;
- on failure, advance one byte;
- on success, require a nontrivial result (observed minimum-length guard);
- append the extracted image;
- skip forward by the found image length.

The image result vector is capped at 4096 entries.

### Consequence

This is not a full reconstruction of Publisher/Escher object placement.

A better description is:

> Korva uses lightweight OfficeArt BLIP record guidance to find embedded JPEG/PNG bytes, with raw signature carving as a fallback.

## 4. Images are rebuilt as new Pagewell assets and placed in a synthetic grid

Inside `import_pub_bytes`, extracted image bytes are base64-encoded and inserted into:

- `BTreeMap<String, pagewell_core::model::Asset>`.

Importer-specific string fragments include:

- `image-`
- `img`

The document does not appear to reuse original Publisher object identifiers in this path.

### Synthetic image layout

The binary contains the following layout constants directly in the importer:

- page width: **612**
- page height: **792**
- left/top margin: **54**
- content width: **504**
- image row height: **150**
- image gap: **12**
- row step: **162**
- nominal content-bottom coordinate: **738**

The number of image columns is selected by image count:

- 1 image -> 1 column
- 2–4 images -> 2 columns
- 5+ images -> 3 columns

The cell width is computed as:

```text
(504 - (columns - 1) * 12) / columns
```

Horizontal placement follows:

```text
x = 54 + column_index * (cell_width + 12)
```

The vertical cursor starts at 54 and advances by 162 per completed row. The importer compares the next 150-high image row against the 738 content-bottom coordinate and creates/uses another page when needed.

This is synthetic gallery-style placement; it is not recovered Publisher geometry.

## 5. The target document is newly constructed as US Letter

The importer calls:

- `<pagewell_core::model::Document>::blank` at `0x1c12110`

with floating-point arguments:

- **612.0**
- **792.0**

Those are US Letter dimensions in PostScript points (8.5 x 11 inches).

Therefore the analyzed import path constructs a new US Letter document rather than preserving a source page size inferred from the Publisher file.

## 6. Text styling is recreated with fixed Pagewell formatting

The importer and its paragraph-builder contain literal `Helvetica`.

The paragraph builder used for every extracted normalized text string constructs a new character format containing:

- font family: **Helvetica**
- size: **12.0**
- color: opaque black (`0xff000000` in the observed field)
- additional Pagewell defaults

Separately, `import_pub_bytes` constructs a style named:

- `Title`

also backed by `Helvetica`, with a larger fixed size observed in its style structure.

This is direct binary evidence that source Publisher typography is not preserved by the analyzed text-import path.

## 7. Text is reflowed by Pagewell's own layout engine

Once normalized strings are converted to Paragraph objects, the importer does not simply dump all text into one frame.

It invokes:

- `pagewell_core::import_pub::imported_body_fits`
- `pagewell_core::import_pub::longest_fitting_paragraph_prefix`
- `pagewell_core::import_pub::imported_paragraph_fragment`

### `imported_body_fits`

This function creates a temporary synthetic frame whose literal id is:

- `__import-body-probe`

and invokes:

- `pagewell_core::layout::build_page_scene` at `0x1cb64c0`.

It then inspects the resulting scene/layout state and returns a boolean fit result.

So fit testing uses Pagewell's actual layout engine rather than a rough character-count estimate.

### Binary search and fragmentation

`import_pub_bytes` performs a binary-search-shaped loop over paragraph counts, repeatedly calling `imported_body_fits` to identify how much body content fits.

When a whole paragraph does not fit, it uses character-boundary candidates plus `imported_paragraph_fragment` and further fit probes to split the paragraph.

The body frame geometry uses the same synthetic Letter-page coordinate system. The observed constants show:

- x = **54**
- width = **504**
- bottom target = **738** when possible
- minimum preferred body height = **120**
- absolute page bottom = **792**
- very small remaining body areas are rejected by a **24**-point threshold.

### Consequence

Publisher's original text-frame chain and page geometry are not being preserved in this path.

Instead, Korva:

1. mines normalized text;
2. creates new Pagewell paragraphs;
3. creates synthetic body frames;
4. asks Pagewell's layout engine how much fits;
5. splits/reflows text across frames/pages.

## 8. What this proves and what it does not

### Strongly supported by the pinned binary

- direct in-process CFB parsing;
- active reads of Quill `CONTENTS` and `EscherDelayStm`;
- heuristic UTF-16LE prose extraction;
- JPEG/PNG BLIP-guided extraction plus carving fallback;
- new Pagewell assets and paragraphs;
- fixed Helvetica-based text formatting;
- new 612 x 792 Letter document;
- synthetic image-grid placement;
- Pagewell-native layout fit/reflow for body text.

### Not proved by this analysis

- source-code genealogy / clean-room independence;
- that every Korva platform/build uses exactly this algorithm;
- complete absence of another hidden PUB import/write path elsewhere;
- exact handling of every Publisher version;
- semantic fidelity on any real corpus;
- preservation or non-preservation of every possible Publisher object type outside the analyzed path.

## Bottom line

For Korva 1.8.7's pinned Linux binary, the best binary-backed model is:

> **content salvage + reconstruction**, not semantic Publisher layout import.

It salvages text and raster images from two well-known CFB streams, normalizes them into Pagewell's internal model, and lays them out again using Korva/Pagewell defaults.

That makes Korva useful as a distinct behavioral data point, but it should not be treated as a high-fidelity independent semantic oracle for Publisher object/layout semantics.
