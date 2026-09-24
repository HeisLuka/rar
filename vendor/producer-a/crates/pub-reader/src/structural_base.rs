use super::{
    CONTENTS_STREAM_PATH, ESCHER_STREAM_PATH, PubBridgeDiagnostic, PubNodePayload,
    build_mature_0x2c_source_graph, build_reference_index, chunk_for_reference, seq_u32,
};
use anyhow::{Context, Result, bail};
use pub_contents::{Contents0x2cChunk, parse_0x2c_header, parse_confirmed_0x2c_trailer_root};
use pub_core::StreamPath;
use pub_escher::{PUBLISHER_FIELD_SHAPE_ID, SpContainerObservation, inspect_sp_containers};
use pub_model::{NodeId, NodeKind, PageId, RectEmu, Sha256Digest};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Cursor;

pub const PUB_STRUCTURAL_BASE_SCHEMA_V1: &str = "pub-structural-base/v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PubStructuralBaseStreamDigest {
    pub path: String,
    pub len: u64,
    pub sha256: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PubStructuralBaseCandidate {
    pub page_id: PageId,
    pub node_id: NodeId,
    pub contents_seq_num: u32,
    pub bounds_emu: RectEmu,
    pub officeart_spid: u32,
    pub officeart_shape_type: u16,
    pub contents_chunk: Contents0x2cChunk,
    pub escher_shape: SpContainerObservation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PubStructuralBaseManifest {
    pub schema: &'static str,
    pub source_sha256: Sha256Digest,
    pub streams: Vec<PubStructuralBaseStreamDigest>,
    pub candidates: Vec<PubStructuralBaseCandidate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<PubBridgeDiagnostic>,
}

pub fn build_mature_0x2c_structural_base_manifest(
    pub_bytes: &[u8],
) -> Result<PubStructuralBaseManifest> {
    let source_sha256 = sha256_digest(pub_bytes);

    let inventory = pub_cfb::inspect_reader(Cursor::new(pub_bytes))
        .context("inspect CFB for structural-base manifest")?;
    let mut streams = Vec::new();
    for entry in inventory
        .entries
        .iter()
        .filter(|entry| entry.kind == pub_cfb::EntryKind::Stream)
    {
        let bytes = pub_cfb::read_stream_reader(Cursor::new(pub_bytes), &entry.path)
            .with_context(|| format!("read CFB stream {}", entry.path))?;
        let len = u64::try_from(bytes.len()).context("stream length does not fit u64")?;
        if len != entry.len {
            bail!(
                "CFB inventory length mismatch for {}: inventory={}, actual={}",
                entry.path,
                entry.len,
                len
            );
        }
        streams.push(PubStructuralBaseStreamDigest {
            path: entry.path.clone(),
            len,
            sha256: sha256_digest(&bytes),
        });
    }

    let build = build_mature_0x2c_source_graph(Cursor::new(pub_bytes), source_sha256)
        .context("build mature-0x2C SourceGraph for structural-base manifest")?;

    let contents = pub_cfb::read_stream_reader(Cursor::new(pub_bytes), CONTENTS_STREAM_PATH)
        .with_context(|| format!("read {CONTENTS_STREAM_PATH}"))?;
    let contents_stream = StreamPath(CONTENTS_STREAM_PATH.into());
    let header = parse_0x2c_header(contents_stream.clone(), &contents)
        .context("parse mature-0x2C Contents header")?;
    let trailer = parse_confirmed_0x2c_trailer_root(&contents, &header)
        .context("parse mature-0x2C Contents trailer")?;
    let references = build_reference_index(&contents, &trailer.directory)
        .context("build Contents reference index")?;

    let escher = pub_cfb::read_stream_reader(Cursor::new(pub_bytes), ESCHER_STREAM_PATH)
        .with_context(|| format!("read {ESCHER_STREAM_PATH}"))?;
    let escher_inventory = inspect_sp_containers(StreamPath(ESCHER_STREAM_PATH.into()), &escher)
        .context("inspect Escher SpContainers")?;

    let mut candidates = Vec::new();
    for (page_id, page) in &build.graph.pages {
        for node_id in &page.children {
            let Some(node) = build.graph.nodes.get(node_id) else {
                continue;
            };
            if node.kind != NodeKind::Shape {
                continue;
            }

            let payload: &PubNodePayload = &node.payload;
            if payload.story_frame.is_some()
                || payload.table.is_some()
                || payload.table_story.is_some()
                || payload.image_slot.is_some()
            {
                continue;
            }

            let (Some(officeart_spid), Some(officeart_shape_type)) =
                (payload.officeart_spid, payload.officeart_shape_type)
            else {
                continue;
            };

            let seq_num = payload.contents_seq_num;
            let Some(reference) = references.get(&seq_num) else {
                continue;
            };
            if seq_u32(reference.seq_num)? != seq_num {
                bail!("Contents reference key/seq mismatch for seq {seq_num}");
            }
            let contents_chunk = chunk_for_reference(contents_stream.clone(), &contents, reference)
                .with_context(|| format!("parse Contents chunk seq {seq_num}"))?;

            let mut escher_matches = escher_inventory.shapes.iter().filter(|shape| {
                shape.fsp.as_ref().is_some_and(|fsp| {
                    fsp.spid == officeart_spid && fsp.shape_type == officeart_shape_type
                }) && escher_contents_identity_matches(shape, seq_num)
            });
            let Some(escher_shape) = escher_matches.next() else {
                continue;
            };
            if escher_matches.next().is_some() {
                continue;
            }

            candidates.push(PubStructuralBaseCandidate {
                page_id: *page_id,
                node_id: *node_id,
                contents_seq_num: seq_num,
                bounds_emu: node.header.bounds,
                officeart_spid,
                officeart_shape_type,
                contents_chunk,
                escher_shape: escher_shape.clone(),
            });
        }
    }

    candidates.sort_by_key(|candidate| candidate.contents_seq_num);

    Ok(PubStructuralBaseManifest {
        schema: PUB_STRUCTURAL_BASE_SCHEMA_V1,
        source_sha256,
        streams,
        candidates,
        diagnostics: build.diagnostics,
    })
}

pub fn structural_base_manifest_json(
    manifest: &PubStructuralBaseManifest,
) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(manifest)
}

fn escher_contents_identity_matches(shape: &SpContainerObservation, seq_num: u32) -> bool {
    let Some(client_data) = shape.client_data.as_ref() else {
        return false;
    };
    let values = client_data
        .values(PUBLISHER_FIELD_SHAPE_ID)
        .collect::<Vec<_>>();
    values.as_slice() == [seq_num]
}

fn sha256_digest(bytes: &[u8]) -> Sha256Digest {
    let digest = Sha256::digest(bytes);
    let mut value = [0_u8; 32];
    value.copy_from_slice(&digest);
    Sha256Digest::from_bytes(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample3_pub() -> Vec<u8> {
        decode_base64(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../pub-quill/tests/fixtures/Sample3.pub.b64"
        )))
    }

    #[test]
    fn structural_base_manifest_hashes_every_stream_and_emits_bounded_candidates() {
        let source = sample3_pub();
        let manifest =
            build_mature_0x2c_structural_base_manifest(&source).expect("structural base manifest");

        assert_eq!(manifest.schema, PUB_STRUCTURAL_BASE_SCHEMA_V1);
        assert_eq!(manifest.source_sha256, sha256_digest(&source));

        let inventory = pub_cfb::inspect_reader(Cursor::new(&source)).expect("CFB inventory");
        let expected_streams = inventory
            .entries
            .iter()
            .filter(|entry| entry.kind == pub_cfb::EntryKind::Stream)
            .count();
        assert_eq!(manifest.streams.len(), expected_streams);
        assert!(
            manifest
                .streams
                .iter()
                .any(|stream| stream.path == CONTENTS_STREAM_PATH)
        );
        assert!(
            manifest
                .streams
                .iter()
                .any(|stream| stream.path == ESCHER_STREAM_PATH)
        );

        for candidate in &manifest.candidates {
            assert_eq!(
                candidate.escher_shape.fsp.as_ref().map(|fsp| fsp.spid),
                Some(candidate.officeart_spid)
            );
            assert_eq!(
                candidate
                    .escher_shape
                    .fsp
                    .as_ref()
                    .map(|fsp| fsp.shape_type),
                Some(candidate.officeart_shape_type)
            );
        }

        let json = structural_base_manifest_json(&manifest).expect("manifest JSON");
        assert!(json.contains(PUB_STRUCTURAL_BASE_SCHEMA_V1));
        assert!(json.contains(CONTENTS_STREAM_PATH));
        assert!(json.contains(ESCHER_STREAM_PATH));
    }

    fn decode_base64(text: &str) -> Vec<u8> {
        let cleaned = text
            .bytes()
            .filter(|byte| !byte.is_ascii_whitespace())
            .collect::<Vec<_>>();
        assert_eq!(cleaned.len() % 4, 0, "base64 fixture length");

        let mut output = Vec::with_capacity(cleaned.len() / 4 * 3);
        for quartet in cleaned.chunks_exact(4) {
            let a = base64_value(quartet[0]);
            let b = base64_value(quartet[1]);
            let c = if quartet[2] == b'=' {
                0
            } else {
                base64_value(quartet[2])
            };
            let d = if quartet[3] == b'=' {
                0
            } else {
                base64_value(quartet[3])
            };
            output.push((a << 2) | (b >> 4));
            if quartet[2] != b'=' {
                output.push((b << 4) | (c >> 2));
            }
            if quartet[3] != b'=' {
                output.push((c << 6) | d);
            }
        }
        output
    }

    fn base64_value(byte: u8) -> u8 {
        match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            other => panic!("invalid base64 byte {other:#x}"),
        }
    }
}
