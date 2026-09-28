use anyhow::{Context, Result, bail};
use pub_contents::{
    BLOCK_TYPE_HANDLE_U32, BLOCK_TYPE_U32, Contents0x2cChunk, RawContentsBlockBody,
    parse_0x2c_header, parse_confirmed_0x2c_trailer_root,
};
use pub_core::StreamPath;
use pub_model::{CanonicalId, NodeId, Sha256Digest, StoryId};
use pub_plccmob_projection::{
    CONTENTS_RAW_TYPE_PLC_CMOB, CarrierSourceIdentityV1, ExactU32FieldV1,
    OPL_DOCQ_PLC_CMOB_FIELD_ID, PlcCmobChunkInputV1, PlcCmobSourceProjectionInputV1,
    PlcCmobSourceProjectionOutputV1, ProducerV1, TargetSourceIdentityV1,
    build_source_projection_output_v1, parse_confirmed_mature_plc_cmob,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use super::{
    CONTENTS_STREAM_PATH, FIELD_STORY_ID, PubResolvedGraph, PubSourceGraph, RAW_TYPE_SHAPE, build_reference_index,
    chunk_for_reference, single_parent_seq, single_raw_type, unique_reference_by_raw_type,
};

const RAW_TYPE_OPL_DOCQ: u16 = 0x5B;
const FIELD_CMO_ID: u16 = 0x0F;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubCmoProjectionBridgeV1 {
    pub output: PlcCmobSourceProjectionOutputV1,
    pub active_graph_identity_parity: bool,
}

/// Builds the already-proven PlcCmob/Cmo projection authority from the same
/// mature-0x2C bytes and canonical graph used by the active Reader.
///
/// This is deliberately an authority bridge, not a second Cmo parser:
/// - Contents directory/locator work reuses the active Reader's bounded parser;
/// - exact PlcCmob row wire is validated by `pub-plccmob-projection`;
/// - canonical ids are produced by the shared cdm-source-id-v1 law there;
/// - every resulting NodeId/StoryId is checked against the active resolved graph.
///
/// The function is intentionally strict. Callers should invoke it only for a
/// source family whose Cmo projection is already admitted; an unavailable or
/// inconsistent relation is evidence, not a reason to guess.
pub fn build_mature_0x2c_cmo_projection_bridge_v1(
    bytes: &[u8],
    source_hash: Sha256Digest,
    source_graph: &PubSourceGraph,
    graph: &PubResolvedGraph,
) -> Result<PubCmoProjectionBridgeV1> {
    let contents = pub_cfb::read_stream_reader(Cursor::new(bytes), CONTENTS_STREAM_PATH)
        .with_context(|| format!("read {CONTENTS_STREAM_PATH} for Cmo projection"))?;
    let contents_stream = StreamPath(CONTENTS_STREAM_PATH.into());
    let header = parse_0x2c_header(contents_stream.clone(), &contents)
        .context("parse mature-0x2C Contents header for Cmo projection")?;
    let trailer = parse_confirmed_0x2c_trailer_root(&contents, &header)
        .context("parse mature-0x2C Contents trailer for Cmo projection")?;
    let references = build_reference_index(&contents, &trailer.directory)?;

    let docq = unique_reference_by_raw_type(&references, RAW_TYPE_OPL_DOCQ, "OplDocq")?;
    let docq_chunk = chunk_for_reference(contents_stream.clone(), &contents, docq)?;
    let plccmob_seq = exact_u32_field(
        &docq_chunk,
        u16::from(OPL_DOCQ_PLC_CMOB_FIELD_ID),
        BLOCK_TYPE_HANDLE_U32,
        "OplDocq.PlcCmob",
        true,
    )?
    .expect("required exact field checked");

    let plccmob_ref = references
        .get(&plccmob_seq)
        .with_context(|| format!("OplDocq PlcCmob handle {plccmob_seq} is unresolved"))?;
    if single_raw_type(plccmob_ref) != Some(CONTENTS_RAW_TYPE_PLC_CMOB) {
        bail!(
            "OplDocq PlcCmob handle {plccmob_seq} does not resolve to raw type 0x{CONTENTS_RAW_TYPE_PLC_CMOB:02X}"
        );
    }
    if single_parent_seq(plccmob_ref) != Some(u32::try_from(docq.seq_num).context("OplDocq seqNum")?) {
        bail!("PlcCmob parent does not match OplDocq");
    }

    let plccmob_chunk =
        chunk_for_reference(contents_stream.clone(), &contents, plccmob_ref)?;
    let plccmob_bytes = exact_chunk_bytes(&contents, &plccmob_chunk)?;
    let parsed =
        parse_confirmed_mature_plc_cmob(plccmob_bytes).context("validate exact PlcCmob wire")?;

    let mut carrier_rows = BTreeMap::<u32, (u32, Option<u32>, u32)>::new();
    let mut target_qsids = BTreeSet::<u32>::new();

    for entry in &parsed.entries {
        target_qsids.insert(entry.target_qsid);

        let carrier_ref = references.get(&entry.carrier_ohpo).with_context(|| {
            format!("PlcCmob carrier Ohpo {} is unresolved", entry.carrier_ohpo)
        })?;
        if single_raw_type(carrier_ref) != Some(RAW_TYPE_SHAPE) {
            bail!(
                "PlcCmob carrier Ohpo {} is not a SHAPE",
                entry.carrier_ohpo
            );
        }
        let source_parent_seq_num = single_parent_seq(carrier_ref).with_context(|| {
            format!(
                "PlcCmob carrier Ohpo {} does not have one source parent",
                entry.carrier_ohpo
            )
        })?;
        let carrier_chunk =
            chunk_for_reference(contents_stream.clone(), &contents, carrier_ref)?;
        let carrier_cmo_id = exact_u32_field(
            &carrier_chunk,
            FIELD_CMO_ID,
            BLOCK_TYPE_U32,
            "OplPo.CmoID",
            true,
        )?
        .expect("required CmoID checked");
        if carrier_cmo_id != entry.cmo_id {
            bail!(
                "PlcCmob carrier Ohpo {} CmoID {} != row {}",
                entry.carrier_ohpo,
                carrier_cmo_id,
                entry.cmo_id
            );
        }
        let carrier_story_qsid = exact_u32_field(
            &carrier_chunk,
            FIELD_STORY_ID,
            BLOCK_TYPE_U32,
            "OplPo.StoryId",
            false,
        )?;

        match carrier_rows.entry(entry.carrier_ohpo) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert((carrier_cmo_id, carrier_story_qsid, source_parent_seq_num));
            }
            std::collections::btree_map::Entry::Occupied(slot) => {
                if *slot.get() != (carrier_cmo_id, carrier_story_qsid, source_parent_seq_num) {
                    bail!(
                        "PlcCmob carrier Ohpo {} has inconsistent repeated identity",
                        entry.carrier_ohpo
                    );
                }
            }
        }
    }

    let carriers = carrier_rows
        .into_iter()
        .map(
            |(carrier_ohpo, (carrier_cmo_id, carrier_story_qsid, source_parent_seq_num))| {
                CarrierSourceIdentityV1 {
                    carrier_ohpo,
                    carrier_cmo_id,
                    carrier_story_qsid,
                    source_parent_seq_num,
                    effective_parent_seq_num: source_parent_seq_num,
                }
            },
        )
        .collect::<Vec<_>>();

    let mut targets = Vec::with_capacity(target_qsids.len());
    for target_qsid in target_qsids {
        let source_frames = source_graph
            .nodes
            .values()
            .filter_map(|node| {
                node.payload
                    .story_frame
                    .as_ref()
                    .filter(|frame| frame.text_id == target_qsid)
                    .map(|frame| (node.payload.contents_seq_num, frame.story_id))
            })
            .collect::<Vec<_>>();
        let target_frames = source_frames
            .iter()
            .map(|(seq_num, _)| *seq_num)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let target_story_ids = source_frames
            .iter()
            .filter_map(|(_, story_id)| *story_id)
            .collect::<BTreeSet<_>>();
        if target_story_ids.len() != 1 {
            bail!(
                "target Qsid {target_qsid} resolves to {} canonical Stories",
                target_story_ids.len()
            );
        }
        let target_story_id = *target_story_ids
            .first()
            .expect("one target Story checked");
        let target_story = source_graph
            .stories
            .get(&target_story_id)
            .with_context(|| format!("target Qsid {target_qsid} Story is absent from source graph"))?;
        let object_marker_count = target_story
            .text
            .chars()
            .filter(|ch| *ch == '\u{FFFC}')
            .count();

        targets.push(TargetSourceIdentityV1 {
            target_qsid,
            target_frame_seq_nums: target_frames,
            object_marker_count,
        });
    }

    let source_hash_string = source_hash.to_string();
    let input = PlcCmobSourceProjectionInputV1 {
        source_hash: source_hash_string,
        producer: ProducerV1 {
            implementation: "rar-active-reader-cmo-bridge".to_owned(),
            commit_or_build: "reader-cmo-authority-bridge-v1".to_owned(),
            core_integration: true,
        },
        opl_docq_field: ExactU32FieldV1 {
            field_id: OPL_DOCQ_PLC_CMOB_FIELD_ID,
            block_type: BLOCK_TYPE_HANDLE_U32,
            value: plccmob_seq,
        },
        plccmob_chunk: PlcCmobChunkInputV1 {
            seq_num: plccmob_seq,
            raw_type: CONTENTS_RAW_TYPE_PLC_CMOB,
            hex: encode_hex(plccmob_bytes),
        },
        carriers,
        targets,
    };

    let output = build_source_projection_output_v1(&input)
        .context("build canonical PlcCmob projection context for active Reader")?;
    verify_active_graph_identity_parity(graph, &output)?;

    Ok(PubCmoProjectionBridgeV1 {
        output,
        active_graph_identity_parity: true,
    })
}

fn exact_u32_field(
    chunk: &Contents0x2cChunk,
    field_id: u16,
    expected_wire: u8,
    label: &str,
    required: bool,
) -> Result<Option<u32>> {
    let matches = chunk
        .fields
        .iter()
        .filter(|field| field.id == field_id)
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        bail!("{label} is duplicated");
    }
    let Some(field) = matches.first() else {
        if required {
            bail!("{label} is missing");
        }
        return Ok(None);
    };
    if field.block_type != expected_wire {
        bail!(
            "{label} uses wire 0x{:02X}, expected 0x{expected_wire:02X}",
            field.block_type
        );
    }
    let RawContentsBlockBody::U32 { value, .. } = &field.body else {
        bail!("{label} does not decode as u32");
    };
    Ok(Some(*value))
}

fn exact_chunk_bytes<'a>(contents: &'a [u8], chunk: &Contents0x2cChunk) -> Result<&'a [u8]> {
    let start = usize::try_from(chunk.source.offset).context("chunk offset does not fit usize")?;
    let len = usize::try_from(chunk.source.len).context("chunk length does not fit usize")?;
    let end = start
        .checked_add(len)
        .filter(|end| *end <= contents.len())
        .context("chunk range is outside Contents")?;
    Ok(&contents[start..end])
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0F)]));
    }
    out
}

fn parse_node_id(value: &str, label: &str) -> Result<NodeId> {
    let canonical = value
        .parse::<CanonicalId>()
        .with_context(|| format!("{label} is not a canonical UUID: {value}"))?;
    Ok(NodeId::from_canonical(canonical))
}

fn parse_story_id(value: &str, label: &str) -> Result<StoryId> {
    let canonical = value
        .parse::<CanonicalId>()
        .with_context(|| format!("{label} is not a canonical UUID: {value}"))?;
    Ok(StoryId::from_canonical(canonical))
}

fn verify_active_graph_identity_parity(
    graph: &PubResolvedGraph,
    output: &PlcCmobSourceProjectionOutputV1,
) -> Result<()> {
    for relation in &output.context.cmo_relations {
        let carrier_node_id = parse_node_id(
            &relation.carrier_node_id,
            "Cmo carrier_node_id",
        )?;
        let carrier_node = graph.nodes.get(&carrier_node_id).with_context(|| {
            format!(
                "root/vendor identity mismatch for carrier Ohpo {}",
                relation.carrier_ohpo
            )
        })?;

        let target_story_id = parse_story_id(
            &relation.target_story_id,
            "Cmo target_story_id",
        )?;
        if !graph.stories.contains_key(&target_story_id) {
            bail!(
                "root/vendor identity mismatch for target Qsid {}",
                relation.target_qsid
            );
        }

        if let Some(frame_id) = relation.target_frame_node_id.as_deref() {
            let target_frame_id = parse_node_id(frame_id, "Cmo target_frame_node_id")?;
            let target_frame = graph.nodes.get(&target_frame_id).with_context(|| {
                format!(
                    "root/vendor identity mismatch for target frame Qsid {}",
                    relation.target_qsid
                )
            })?;
            let frame_story_id = target_frame
                .payload
                .story_frame
                .as_ref()
                .and_then(|frame| frame.story_id)
                .with_context(|| {
                    format!(
                        "target frame for Qsid {} has no resolved Story identity",
                        relation.target_qsid
                    )
                })?;
            if frame_story_id != target_story_id {
                bail!(
                    "target frame/story mismatch for Qsid {}",
                    relation.target_qsid
                );
            }
        }

        if let Some(story_id) = relation.carrier_story_id.as_deref() {
            let carrier_story_id = parse_story_id(story_id, "Cmo carrier_story_id")?;
            if !graph.stories.contains_key(&carrier_story_id) {
                bail!(
                    "root/vendor identity mismatch for carrier Story Ohpo {}",
                    relation.carrier_ohpo
                );
            }
            let node_story_id = carrier_node
                .payload
                .story_frame
                .as_ref()
                .and_then(|frame| frame.story_id)
                .with_context(|| {
                    format!(
                        "carrier node Ohpo {} has no resolved Story identity",
                        relation.carrier_ohpo
                    )
                })?;
            if node_story_id != carrier_story_id {
                bail!(
                    "carrier node/story mismatch for Ohpo {}",
                    relation.carrier_ohpo
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_encoding_is_exact_and_lowercase() {
        assert_eq!(encode_hex(&[0x00, 0x7F, 0xA0, 0xFF]), "007fa0ff");
    }
}
