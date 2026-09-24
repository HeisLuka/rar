use crate::{ESCHER_DELAY_STREAM_PATH, ESCHER_STREAM_PATH, PUB_ADAPTER_ID, PubSourceGraph};
use anyhow::{Context, Result};
use pub_core::RawSpan;
use pub_escher::{BlipKind, inspect_bstore, inspect_delayed_blips, resolve_delayed_blip};
use pub_model::{
    ImageResource, NodeId, PageId, ResourceId, Sha256Digest, SourceDerivedIdInput,
    derive_source_canonical_id,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PubAssetUse {
    pub page_id: PageId,
    pub node_id: NodeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubAssetManifestEntry {
    /// Exact one-based OfficeArt BStore identity.
    pub slot: u32,
    pub c_ref: u32,
    pub uid: [u8; 16],
    pub bstore_record_source: RawSpan,
    pub blip_kind: Option<BlipKind>,
    pub blip_record_source: Option<RawSpan>,
    pub image_payload_source: Option<RawSpan>,
    pub payload_sha256: Option<Sha256Digest>,
    pub payload_len: Option<u64>,
    pub uses: Vec<PubAssetUse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum PubAssetManifestDiagnostic {
    MissingBStoreSlot { slot: u32 },
    EmptyBStoreSlot { slot: u32 },
    DelayedBlipUnresolved { slot: u32, fo_delay: u32 },
    EmbeddedBlipNotExtracted { slot: u32 },
    StandardPayloadUnavailable { slot: u32, kind: BlipKind },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubAssetManifest {
    pub assets: Vec<PubAssetManifestEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<PubAssetManifestDiagnostic>,
}

pub fn build_pub_asset_manifest(
    graph: &PubSourceGraph,
    escher: &[u8],
    delayed: &[u8],
) -> Result<PubAssetManifest> {
    let bstore = inspect_bstore(pub_core::StreamPath(ESCHER_STREAM_PATH.into()), escher)
        .context("inspect OfficeArt BStore")?;
    let delayed_inventory = inspect_delayed_blips(
        pub_core::StreamPath(ESCHER_DELAY_STREAM_PATH.into()),
        delayed,
    )
    .context("inspect delayed OfficeArt BLIPs")?;

    let mut uses_by_slot = BTreeMap::<u32, Vec<PubAssetUse>>::new();
    for (node_id, node) in &graph.nodes {
        let Some(slot) = node.payload.image_slot else {
            continue;
        };
        let Some(page_id) = graph
            .pages
            .keys()
            .find(|page_id| page_id.into_canonical() == node.header.parent_id)
        else {
            continue;
        };
        uses_by_slot.entry(slot).or_default().push(PubAssetUse {
            page_id: *page_id,
            node_id: *node_id,
        });
    }

    let mut assets = Vec::with_capacity(uses_by_slot.len());
    let mut diagnostics = Vec::new();

    for (slot, mut uses) in uses_by_slot {
        uses.sort();

        let Some(bstore_slot) = bstore.slots.iter().find(|entry| entry.slot == slot) else {
            diagnostics.push(PubAssetManifestDiagnostic::MissingBStoreSlot { slot });
            continue;
        };

        if bstore_slot.is_empty() {
            diagnostics.push(PubAssetManifestDiagnostic::EmptyBStoreSlot { slot });
            assets.push(PubAssetManifestEntry {
                slot,
                c_ref: bstore_slot.c_ref,
                uid: bstore_slot.uid,
                bstore_record_source: bstore_slot.record_source.clone(),
                blip_kind: None,
                blip_record_source: None,
                image_payload_source: None,
                payload_sha256: None,
                payload_len: None,
                uses,
            });
            continue;
        }

        if bstore_slot.embedded_blip_source.is_some() {
            diagnostics.push(PubAssetManifestDiagnostic::EmbeddedBlipNotExtracted { slot });
            assets.push(PubAssetManifestEntry {
                slot,
                c_ref: bstore_slot.c_ref,
                uid: bstore_slot.uid,
                bstore_record_source: bstore_slot.record_source.clone(),
                blip_kind: None,
                blip_record_source: bstore_slot.embedded_blip_source.clone(),
                image_payload_source: None,
                payload_sha256: None,
                payload_len: None,
                uses,
            });
            continue;
        }

        let delayed_blip = match resolve_delayed_blip(&bstore, &delayed_inventory, slot) {
            Ok(Some(record)) => record,
            Ok(None) => {
                diagnostics.push(PubAssetManifestDiagnostic::StandardPayloadUnavailable {
                    slot,
                    kind: BlipKind::Unknown,
                });
                assets.push(PubAssetManifestEntry {
                    slot,
                    c_ref: bstore_slot.c_ref,
                    uid: bstore_slot.uid,
                    bstore_record_source: bstore_slot.record_source.clone(),
                    blip_kind: None,
                    blip_record_source: None,
                    image_payload_source: None,
                    payload_sha256: None,
                    payload_len: None,
                    uses,
                });
                continue;
            }
            Err(pub_escher::AssetReadError::DelayedRecordNotFound { fo_delay, .. }) => {
                diagnostics
                    .push(PubAssetManifestDiagnostic::DelayedBlipUnresolved { slot, fo_delay });
                assets.push(PubAssetManifestEntry {
                    slot,
                    c_ref: bstore_slot.c_ref,
                    uid: bstore_slot.uid,
                    bstore_record_source: bstore_slot.record_source.clone(),
                    blip_kind: None,
                    blip_record_source: None,
                    image_payload_source: None,
                    payload_sha256: None,
                    payload_len: None,
                    uses,
                });
                continue;
            }
            Err(error) => return Err(error).context("resolve delayed OfficeArt BLIP"),
        };

        let (payload_sha256, payload_len) =
            if let Some(payload_source) = delayed_blip.image_payload_source.as_ref() {
                let payload =
                    slice_span(delayed, payload_source).context("slice delayed image payload")?;
                let digest = Sha256::digest(payload);
                let mut bytes = [0_u8; 32];
                bytes.copy_from_slice(&digest);
                (
                    Some(Sha256Digest::from_bytes(bytes)),
                    Some(payload_source.len),
                )
            } else {
                diagnostics.push(PubAssetManifestDiagnostic::StandardPayloadUnavailable {
                    slot,
                    kind: delayed_blip.kind,
                });
                (None, None)
            };

        assets.push(PubAssetManifestEntry {
            slot,
            c_ref: bstore_slot.c_ref,
            uid: bstore_slot.uid,
            bstore_record_source: bstore_slot.record_source.clone(),
            blip_kind: Some(delayed_blip.kind),
            blip_record_source: Some(delayed_blip.record_source.clone()),
            image_payload_source: delayed_blip.image_payload_source.clone(),
            payload_sha256,
            payload_len,
            uses,
        });
    }

    Ok(PubAssetManifest {
        assets,
        diagnostics,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PubImageAlpha {
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubImageBlobRef {
    pub source: RawSpan,
    pub kind: BlipKind,
}

pub type PubImageResource = ImageResource<(), PubImageAlpha, PubImageBlobRef>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum PubImageResourceDiagnostic {
    AssetPayloadNotExact { slot: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubImageResourceCatalog {
    pub resources: BTreeMap<ResourceId, PubImageResource>,
    pub node_resources: BTreeMap<NodeId, ResourceId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<PubImageResourceDiagnostic>,
}

pub fn build_pub_image_resource_catalog(
    graph: &PubSourceGraph,
    manifest: &PubAssetManifest,
) -> Result<PubImageResourceCatalog> {
    let mut resources = BTreeMap::new();
    let mut node_resources = BTreeMap::new();
    let mut diagnostics = Vec::new();

    for asset in &manifest.assets {
        let (Some(payload_hash), Some(payload_source), Some(kind)) = (
            asset.payload_sha256,
            asset.image_payload_source.as_ref(),
            asset.blip_kind,
        ) else {
            diagnostics.push(PubImageResourceDiagnostic::AssetPayloadNotExact { slot: asset.slot });
            continue;
        };

        let resource_id = ResourceId::from_canonical(
            derive_source_canonical_id(SourceDerivedIdInput {
                source_hash: &graph.source.source_hash,
                adapter_id: PUB_ADAPTER_ID,
                source_object_key: &format!("escher/bstore/slot/{}", asset.slot),
                semantic_role: "cdm.resource.image",
            })
            .map_err(|error| anyhow::anyhow!("image resource identity error: {error:?}"))?,
        );

        let mime = match kind {
            BlipKind::Png => "image/png",
            BlipKind::Jpeg => "image/jpeg",
            BlipKind::Dib => "image/x-ms-bmp-dib",
            BlipKind::Tiff => "image/tiff",
            BlipKind::Emf => "image/x-emf",
            BlipKind::Wmf => "image/x-wmf",
            BlipKind::Pict => "image/x-pict",
            BlipKind::Unknown => "application/octet-stream",
        }
        .to_owned();

        resources.insert(
            resource_id,
            ImageResource {
                id: resource_id,
                mime,
                source_hash: payload_hash,
                intrinsic_size_px: None,
                color_profile: None,
                alpha: PubImageAlpha::Unknown,
                original_blob: PubImageBlobRef {
                    source: payload_source.clone(),
                    kind,
                },
            },
        );

        for usage in &asset.uses {
            node_resources.insert(usage.node_id, resource_id);
        }
    }

    Ok(PubImageResourceCatalog {
        resources,
        node_resources,
        diagnostics,
    })
}

fn slice_span<'a>(bytes: &'a [u8], span: &RawSpan) -> Result<&'a [u8]> {
    let start = usize::try_from(span.offset).context("span offset does not fit usize")?;
    let len = usize::try_from(span.len).context("span length does not fit usize")?;
    let end = start.checked_add(len).context("span end overflow")?;
    bytes
        .get(start..end)
        .context("span is outside source stream")
}
