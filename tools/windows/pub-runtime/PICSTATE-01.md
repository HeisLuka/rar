# PICSTATE-01

Bounded native experiment for `PUB-T-183 / PICSTATE-01`.

## Question

What does Publisher 16.0 build 12527 actually do for the four `Shapes.AddPicture` cells formed by:

- `LinkToFile = False|True`
- `SaveWithDocument = False|True`

Documentation is not sufficient because the Publisher 2003 help and current Microsoft Learn disagree around the same-value combinations. Every cell is therefore measured, including expected errors.

## Exact inputs

The packet uses the pinned runtime blank:

- `pubgen-create-20260923/minimal-blank-v1-generated.pub`
- SHA-256 `5bf6057b8b11c8ee4a421d93885ae6e9e7c03a7a42d541e6d0df33497c08c33b`

Publisher executable:

- version prefix `16.0.12527.`
- SHA-256 `e1ef8811b85b82045f37c4173b92726101be3a25e550b0dcb9f178df834ab20b`

The picture source is self-contained: the operation materializes the same deterministic 8x8 PNG in every arm from embedded bytes and verifies SHA-256 `54ac15d54e246a28e6a77173345b84360106257a0a6dacd8db0cfdfe59a31b9b`.

## Four cells

- `FF`: LinkToFile=False, SaveWithDocument=False
- `FT`: LinkToFile=False, SaveWithDocument=True
- `TF`: LinkToFile=True, SaveWithDocument=False
- `TT`: LinkToFile=True, SaveWithDocument=True

Each arm gets its own source and publication directories and starts from the same blank fixture.

For each cell the operation records:

- AddPicture success/error + HRESULT;
- Save success/error;
- fresh-reopen success/error;
- Shape.ID/type/geometry;
- PictureFormat.IsLinked;
- FileName;
- FileSize;
- OriginalFileSize;
- LinkedFileStatus;
- ImageFormat;
- LinkFormat.SourceFullName where legal;
- publication/source directory file names, sizes and SHA-256 before AddPicture, after AddPicture, after Save and after fresh reopen;
- whether a file with the exact sentinel hash appeared in the publication directory.

## Evidence boundary

Generated PUB files and any Publisher-created picture copies remain under the native runner's private evidence directory. The uploaded evidence is limited to:

- `environment.json`
- `analysis/picstate-01.json`
- `logs/picstate-01.txt`
- generic evidence manifest

Absolute private paths are redacted from COM filename/source fields.

A positive COM truth table does **not** identify the physical persisted carrier. BLIP/BStore/EscherDelay/link-path localization is the next private-PUB structural join and belongs downstream of this arm.

## Execution

Run `.github/workflows/pub-native-research.yml` with:

- packet: `tools/research-runner/experiments/picstate-01.packet.json`
- environment: `publisher-2019`
