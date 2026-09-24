use crate::{
    ESCHER_DELAY_STREAM_PATH, ESCHER_STREAM_PATH, PubAssetManifest, PubAssetUse,
    PubImageResourceCatalog, PubSourceGraph, build_pub_asset_manifest,
    build_pub_image_resource_catalog,
};
use anyhow::{Context, Result, bail};
use pub_core::{RawPublication, RawSpan};
use pub_model::{ResourceId, Sha256Digest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Cursor;
use std::path::Path;

pub const PUB_ASSET_EXPORT_SCHEMA_V0_1: &str = "pub-assets-v0.1";
pub const PUB_ASSET_MANIFEST_FILENAME: &str = "manifest.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PubAssetExportFile {
    pub resource_id: ResourceId,
    pub filename: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubAssetExportManifestEntry {
    pub resource_id: ResourceId,
    pub filename: String,
    pub mime: String,
    pub sha256: Sha256Digest,
    pub byte_len: u64,
    pub source: RawSpan,
    pub uses: Vec<PubAssetUse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum PubAssetExportDiagnostic {
    AssetNotPromoted { slot: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubAssetExportManifest {
    pub schema_version: String,
    pub assets: Vec<PubAssetExportManifestEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<PubAssetExportDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PubAssetExportBundle {
    pub files: Vec<PubAssetExportFile>,
    pub manifest: PubAssetExportManifest,
}

/// Builds the exact embedded-image export bundle directly from one complete PUB file.
///
/// This keeps CFB/Escher stream handling inside the format-aware reader layer.
/// Missing EscherDelayStm is treated as an empty delayed stream; unsupported or
/// non-exact image payloads remain explicit manifest diagnostics rather than
/// being guessed from other source state.
pub fn build_mature_0x2c_asset_export_bundle_from_bytes(
    pub_bytes: &[u8],
    graph: &PubSourceGraph,
) -> Result<PubAssetExportBundle> {
    let escher = pub_cfb::read_stream_reader(Cursor::new(pub_bytes), ESCHER_STREAM_PATH)
        .context("read Escher stream for exact asset export")?;

    let inventory = pub_cfb::inspect_reader(Cursor::new(pub_bytes))
        .context("inspect CFB for delayed image stream")?;
    let has_delayed_stream = inventory
        .entries
        .iter()
        .any(|entry| entry.path == ESCHER_DELAY_STREAM_PATH);
    let delayed = if has_delayed_stream {
        pub_cfb::read_stream_reader(Cursor::new(pub_bytes), ESCHER_DELAY_STREAM_PATH)
            .context("read delayed Escher stream for exact asset export")?
    } else {
        Vec::new()
    };

    let manifest = build_pub_asset_manifest(graph, &escher, &delayed)
        .context("build exact PUB asset manifest")?;
    let catalog = build_pub_image_resource_catalog(graph, &manifest)
        .context("promote exact PUB assets to image resources")?;
    let raw = RawPublication::from_streams([
        (pub_core::StreamPath(ESCHER_STREAM_PATH.into()), escher),
        (
            pub_core::StreamPath(ESCHER_DELAY_STREAM_PATH.into()),
            delayed,
        ),
    ]);

    build_pub_asset_export_bundle(&manifest, &catalog, &raw)
        .context("materialize exact embedded PUB image bytes")
}

pub fn build_pub_asset_export_bundle(
    asset_manifest: &PubAssetManifest,
    catalog: &PubImageResourceCatalog,
    raw: &RawPublication,
) -> Result<PubAssetExportBundle> {
    let mut files_by_resource = BTreeMap::<ResourceId, PubAssetExportFile>::new();
    let mut entries_by_resource = BTreeMap::<ResourceId, PubAssetExportManifestEntry>::new();
    let mut diagnostics = Vec::new();

    for asset in &asset_manifest.assets {
        let (Some(payload_sha256), Some(payload_source)) =
            (asset.payload_sha256, asset.image_payload_source.as_ref())
        else {
            diagnostics.push(PubAssetExportDiagnostic::AssetNotPromoted { slot: asset.slot });
            continue;
        };

        let mut resource_ids = BTreeSet::new();
        for usage in &asset.uses {
            if let Some(resource_id) = catalog.node_resources.get(&usage.node_id) {
                resource_ids.insert(*resource_id);
            }
        }

        if resource_ids.is_empty() {
            diagnostics.push(PubAssetExportDiagnostic::AssetNotPromoted { slot: asset.slot });
            continue;
        }
        if resource_ids.len() != 1 {
            bail!(
                "asset slot {} maps to multiple semantic ImageResource ids: {:?}",
                asset.slot,
                resource_ids
            );
        }

        let resource_id = *resource_ids.iter().next().expect("one resource id");
        let resource = catalog
            .resources
            .get(&resource_id)
            .with_context(|| format!("missing ImageResource for asset slot {}", asset.slot))?;

        if resource.source_hash != payload_sha256 {
            bail!(
                "ImageResource {} hash differs from asset manifest slot {}",
                resource_id.as_canonical(),
                asset.slot
            );
        }
        if &resource.original_blob.source != payload_source {
            bail!(
                "ImageResource {} source span differs from asset manifest slot {}",
                resource_id.as_canonical(),
                asset.slot
            );
        }

        let bytes = raw
            .bytes(&resource.original_blob.source)
            .with_context(|| {
                format!(
                    "source bytes unavailable for ImageResource {} at {:?}",
                    resource_id.as_canonical(),
                    resource.original_blob.source
                )
            })?
            .to_vec();

        let digest = Sha256::digest(&bytes);
        let mut digest_bytes = [0_u8; 32];
        digest_bytes.copy_from_slice(&digest);
        let actual_sha256 = Sha256Digest::from_bytes(digest_bytes);
        if actual_sha256 != resource.source_hash {
            bail!(
                "source bytes hash mismatch for ImageResource {}",
                resource_id.as_canonical()
            );
        }

        let filename = asset_filename(resource_id, &resource.mime);
        let mut uses = asset.uses.clone();
        uses.sort();
        uses.dedup();

        let entry = PubAssetExportManifestEntry {
            resource_id,
            filename: filename.clone(),
            mime: resource.mime.clone(),
            sha256: resource.source_hash,
            byte_len: u64::try_from(bytes.len()).context("asset length does not fit u64")?,
            source: resource.original_blob.source.clone(),
            uses,
        };

        if let Some(existing) = entries_by_resource.get(&resource_id) {
            if existing != &entry {
                bail!(
                    "shared ImageResource {} has inconsistent export metadata",
                    resource_id.as_canonical()
                );
            }
            continue;
        }

        files_by_resource.insert(
            resource_id,
            PubAssetExportFile {
                resource_id,
                filename,
                bytes,
            },
        );
        entries_by_resource.insert(resource_id, entry);
    }

    let files = files_by_resource.into_values().collect();
    let assets = entries_by_resource.into_values().collect();

    Ok(PubAssetExportBundle {
        files,
        manifest: PubAssetExportManifest {
            schema_version: PUB_ASSET_EXPORT_SCHEMA_V0_1.to_owned(),
            assets,
            diagnostics,
        },
    })
}

pub fn pub_asset_manifest_json(manifest: &PubAssetExportManifest) -> Result<String> {
    serde_json::to_string_pretty(manifest).context("serialize PUB asset export manifest")
}

pub fn write_pub_asset_export_bundle(
    bundle: &PubAssetExportBundle,
    output_dir: &Path,
) -> Result<()> {
    fs::create_dir_all(output_dir)
        .with_context(|| format!("create asset output directory {}", output_dir.display()))?;

    for file in &bundle.files {
        let path = output_dir.join(&file.filename);
        fs::write(&path, &file.bytes)
            .with_context(|| format!("write extracted asset {}", path.display()))?;
    }

    let manifest = pub_asset_manifest_json(&bundle.manifest)?;
    let manifest_path = output_dir.join(PUB_ASSET_MANIFEST_FILENAME);
    fs::write(&manifest_path, manifest.as_bytes())
        .with_context(|| format!("write asset manifest {}", manifest_path.display()))?;

    Ok(())
}

fn asset_filename(resource_id: ResourceId, mime: &str) -> String {
    let extension = extension_for_mime(mime);
    format!("asset-{}.{}", resource_id.as_canonical(), extension)
}

fn extension_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/x-ms-bmp-dib" => "dib",
        "image/tiff" => "tif",
        "image/x-emf" => "emf",
        "image/x-wmf" => "wmf",
        "image/x-pict" => "pict",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PubAssetManifestEntry, PubImageAlpha, PubImageBlobRef, PubImageResource,
        PubImageResourceDiagnostic,
    };
    use pub_core::StreamPath;
    use pub_escher::BlipKind;
    use pub_model::{CanonicalId, NodeId, PageId};
    use std::collections::BTreeMap;

    fn canonical(byte: u8) -> CanonicalId {
        CanonicalId::from_bytes([byte; 16])
    }

    fn digest(bytes: &[u8]) -> Sha256Digest {
        let hash = Sha256::digest(bytes);
        let mut output = [0_u8; 32];
        output.copy_from_slice(&hash);
        Sha256Digest::from_bytes(output)
    }

    #[test]
    fn bundle_preserves_original_bytes_and_deduplicates_shared_resource() {
        let stream = StreamPath("/Escher/EscherDelayStm".into());
        let payload = vec![0xff, 0xd8, 0xff, 0xd9];
        let raw = RawPublication::from_streams([(stream.clone(), payload.clone())]);
        let source = RawSpan {
            stream,
            offset: 0,
            len: payload.len() as u64,
        };

        let page_id = PageId::from_canonical(canonical(1));
        let node_a = NodeId::from_canonical(canonical(2));
        let node_b = NodeId::from_canonical(canonical(3));
        let resource_id = ResourceId::from_canonical(canonical(4));
        let sha = digest(&payload);

        let manifest = PubAssetManifest {
            assets: vec![PubAssetManifestEntry {
                slot: 1,
                c_ref: 2,
                uid: [0x11; 16],
                bstore_record_source: RawSpan {
                    stream: StreamPath("/Escher/EscherStm".into()),
                    offset: 1,
                    len: 2,
                },
                blip_kind: Some(BlipKind::Jpeg),
                blip_record_source: Some(source.clone()),
                image_payload_source: Some(source.clone()),
                payload_sha256: Some(sha),
                payload_len: Some(payload.len() as u64),
                uses: vec![
                    PubAssetUse {
                        page_id,
                        node_id: node_b,
                    },
                    PubAssetUse {
                        page_id,
                        node_id: node_a,
                    },
                ],
            }],
            diagnostics: Vec::new(),
        };

        let mut resources = BTreeMap::new();
        resources.insert(
            resource_id,
            PubImageResource {
                id: resource_id,
                mime: "image/jpeg".into(),
                source_hash: sha,
                intrinsic_size_px: None,
                color_profile: None,
                alpha: PubImageAlpha::Unknown,
                original_blob: PubImageBlobRef {
                    source: source.clone(),
                    kind: BlipKind::Jpeg,
                },
            },
        );
        let catalog = PubImageResourceCatalog {
            resources,
            node_resources: BTreeMap::from([(node_a, resource_id), (node_b, resource_id)]),
            diagnostics: Vec::<PubImageResourceDiagnostic>::new(),
        };

        let bundle =
            build_pub_asset_export_bundle(&manifest, &catalog, &raw).expect("bundle should build");

        assert_eq!(bundle.files.len(), 1);
        assert_eq!(bundle.files[0].bytes, payload);
        assert!(bundle.files[0].filename.ends_with(".jpg"));
        assert_eq!(bundle.manifest.assets.len(), 1);
        assert_eq!(
            bundle.manifest.assets[0]
                .uses
                .iter()
                .map(|usage| usage.node_id)
                .collect::<Vec<_>>(),
            vec![node_a, node_b]
        );

        let json_a = pub_asset_manifest_json(&bundle.manifest).expect("manifest json");
        let json_b = pub_asset_manifest_json(&bundle.manifest).expect("manifest json");
        assert_eq!(json_a, json_b);
        assert!(json_a.contains(PUB_ASSET_EXPORT_SCHEMA_V0_1));
        assert!(json_a.contains(&bundle.files[0].filename));
    }

    #[test]
    fn unknown_mime_uses_bin_without_guessing() {
        let id = ResourceId::from_canonical(canonical(9));
        assert!(asset_filename(id, "application/octet-stream").ends_with(".bin"));
    }
}
