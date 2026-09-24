//! Bounded public-safe PlcCmob reader/join/projection slice.
//!
//! This crate intentionally does not mirror the historical PUB reader. It owns only
//! the exact later-0x2C PlcCmob wire admitted by PLCCMOB-PROJECTION-01 plus the
//! source-free join needed to emit the existing public receipt contract.

use pub_model::{\n    CmoProjectionRelationV1, PUB_PROJECTION_CONTEXT_SCHEMA_V1, PubProjectionContextV1,\n};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

pub const OPL_DOCQ_PLC_CMOB_FIELD_ID: u8 = 0x05;
pub const BLOCK_TYPE_U32: u8 = 0x20;
pub const BLOCK_TYPE_REFERENCE_U32: u8 = 0x68;
pub const BLOCK_TYPE_OBJECT_HANDLE_U32: u8 = 0x70;
pub const BLOCK_TYPE_ROW_CONTAINER: u8 = 0x88;
pub const BLOCK_TYPE_ARRAY_CONTAINER: u8 = 0xA0;
pub const CONTENTS_RAW_TYPE_PLC_CMOB: u16 = 0x70;

const PLC_CMOB_DECLARED_COUNT_ID: u8 = 0x01;
const PLC_CMOB_ENTRY_ARRAY_ID: u8 = 0x02;
const PLC_CMOB_ENTRY_ID: u8 = 0x00;
const CMO_ENTRY_CMO_ID: u8 = 0x01;
const CMO_ENTRY_TARGET_QSID: u8 = 0x02;
const CMO_ENTRY_CARRIER_OHPO: u8 = 0x03;
const PLC_CMOB_ROW_DECLARED_LENGTH: u32 = 22;
const PLC_CMOB_FIXED_PREFIX_SIZE: usize = 16;
const PLC_CMOB_ROW_SIZE: usize = 24;

pub const RECEIPT_VERSION_V1: &str = "chaptera.plccmob-projection-receipt.v1";
pub const PROJECTION_CONTEXT_VERSION_V1: &str = PUB_PROJECTION_CONTEXT_SCHEMA_V1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawSpanV1 {
    pub offset: usize,
    pub len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatureCmobEntryV1 {
    pub source_order: usize,
    pub source: RawSpanV1,
    pub cmo_id: u32,
    pub cmo_id_source: RawSpanV1,
    pub target_qsid: u32,
    pub target_qsid_source: RawSpanV1,
    pub carrier_ohpo: u32,
    pub carrier_ohpo_source: RawSpanV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaturePlcCmobV1 {
    pub source: RawSpanV1,
    pub declared_count: u32,
    pub declared_count_source: RawSpanV1,
    pub entry_array_source: RawSpanV1,
    pub content_source: u32,
    pub entries: Vec<MatureCmobEntryV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProducerV1 {
    pub implementation: String,
    pub commit_or_build: String,
    pub core_integration: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExactU32FieldV1 {
    pub field_id: u8,
    pub block_type: u8,
    pub value: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlcCmobChunkInputV1 {
    pub seq_num: u32,
    pub raw_type: u16,
    pub hex: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarrierSourceV1 {
    pub carrier_ohpo: u32,
    pub carrier_cmo_id: u32,
    pub carrier_node_id: String,
    pub carrier_story_id: Option<String>,
    pub source_parent_id: String,
    pub effective_parent_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetSourceV1 {
    pub target_qsid: u32,
    pub target_story_id: String,
    pub target_frame_node_id: Option<String>,
    pub object_marker_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlcCmobProjectionInputV1 {
    pub source_hash: String,
    pub producer: ProducerV1,
    pub opl_docq_field: ExactU32FieldV1,
    pub plccmob_chunk: PlcCmobChunkInputV1,
    pub carriers: Vec<CarrierSourceV1>,
    pub targets: Vec<TargetSourceV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlcCmobReceiptSummaryV1 {
    pub declared_count: usize,
    pub row_count: usize,
    pub raw_size: usize,
}

pub type PlcCmobRelationReceiptV1 = CmoProjectionRelationV1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlcCmobTargetReceiptV1 {
    pub target_qsid: u32,
    pub relation_count: usize,
    pub object_marker_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlcCmobInvariantsReceiptV1 {
    pub source_parentage_preserved: bool,
    pub carrier_reparent_count: usize,
    pub raw_text_emitted: bool,
    pub ordered_relation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailClosedProbeV1 {
    pub code: String,
    pub diagnostic_emitted: bool,
    pub projection_emitted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailClosedProbesV1 {
    pub count_mismatch: FailClosedProbeV1,
    pub malformed_row: FailClosedProbeV1,
    pub unresolved_ohpo: FailClosedProbeV1,
    pub carrier_cmo_id_mismatch: FailClosedProbeV1,
    pub unresolved_target_qsid: FailClosedProbeV1,
    pub marker_count_mismatch: FailClosedProbeV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlcCmobProjectionReceiptV1 {
    pub receipt_version: String,
    pub producer: ProducerV1,
    pub source_hash: String,
    pub projection_context_version: String,
    pub plc_cmob: PlcCmobReceiptSummaryV1,
    pub relations: Vec<PlcCmobRelationReceiptV1>,
    pub targets: Vec<PlcCmobTargetReceiptV1>,
    pub invariants: PlcCmobInvariantsReceiptV1,
    pub fail_closed_probes: FailClosedProbesV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlcCmobProjectionError {
    Truncated {
        at: usize,
        need: usize,
        available: usize,
    },
    WrongField {
        context: &'static str,
        expected: u8,
        actual: u8,
    },
    DuplicateField {
        context: &'static str,
        field_id: u8,
    },
    WrongWire {
        context: &'static str,
        expected: u8,
        actual: u8,
    },
    DeclaredLengthMismatch {
        context: &'static str,
        expected: u32,
        actual: u32,
    },
    MalformedPayloadLength {
        payload_len: usize,
    },
    EntryCountMismatch {
        declared: usize,
        actual: usize,
    },
    InvalidHexLength,
    InvalidHexByte {
        index: usize,
    },
    InvalidSourceHash,
    InvalidProducerField {
        field: &'static str,
    },
    LocatorFieldMismatch {
        expected: u8,
        actual: u8,
    },
    LocatorWireMismatch {
        expected: u8,
        actual: u8,
    },
    LocatorHandleMismatch {
        field_value: u32,
        chunk_seq_num: u32,
    },
    PlcCmobRawTypeMismatch {
        expected: u16,
        actual: u16,
    },
    DuplicateCarrierOhpo {
        carrier_ohpo: u32,
    },
    DuplicateTargetQsid {
        target_qsid: u32,
    },
    InvalidUuid {
        field: &'static str,
    },
    CarrierReparented {
        carrier_ohpo: u32,
    },
    UnresolvedOhpo {
        carrier_ohpo: u32,
    },
    CarrierCmoIdMismatch {
        carrier_ohpo: u32,
        row_cmo_id: u32,
        carrier_cmo_id: u32,
    },
    UnresolvedTargetQsid {
        target_qsid: u32,
    },
    MarkerCountMismatch {
        target_qsid: u32,
        relation_count: usize,
        object_marker_count: usize,
    },
}

impl fmt::Display for PlcCmobProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated {
                at,
                need,
                available,
            } => write!(
                f,
                "truncated PlcCmob block at {at}: need {need} bytes, have {available}"
            ),
            Self::WrongField {
                context,
                expected,
                actual,
            } => write!(
                f,
                "{context}: expected field 0x{expected:02x}, got 0x{actual:02x}"
            ),
            Self::DuplicateField { context, field_id } => {
                write!(f, "{context}: duplicate field 0x{field_id:02x}")
            }
            Self::WrongWire {
                context,
                expected,
                actual,
            } => write!(
                f,
                "{context}: expected wire 0x{expected:02x}, got 0x{actual:02x}"
            ),
            Self::DeclaredLengthMismatch {
                context,
                expected,
                actual,
            } => write!(
                f,
                "{context}: declared length {actual} != expected {expected}"
            ),
            Self::MalformedPayloadLength { payload_len } => {
                write!(
                    f,
                    "PlcCmob entry payload length {payload_len} is not a multiple of 24"
                )
            }
            Self::EntryCountMismatch { declared, actual } => {
                write!(
                    f,
                    "PlcCmob declared count {declared} != actual row count {actual}"
                )
            }
            Self::InvalidHexLength => write!(f, "PlcCmob hex input has odd length"),
            Self::InvalidHexByte { index } => write!(f, "invalid hex byte at input index {index}"),
            Self::InvalidSourceHash => write!(f, "source_hash must be 64 lowercase hex characters"),
            Self::InvalidProducerField { field } => write!(f, "invalid producer field: {field}"),
            Self::LocatorFieldMismatch { expected, actual } => write!(
                f,
                "OplDocq locator field mismatch: expected 0x{expected:02x}, got 0x{actual:02x}"
            ),
            Self::LocatorWireMismatch { expected, actual } => write!(
                f,
                "OplDocq locator wire mismatch: expected 0x{expected:02x}, got 0x{actual:02x}"
            ),
            Self::LocatorHandleMismatch {
                field_value,
                chunk_seq_num,
            } => write!(
                f,
                "OplDocq OhPlccmob handle {field_value} != supplied PlcCmob chunk seqNum {chunk_seq_num}"
            ),
            Self::PlcCmobRawTypeMismatch { expected, actual } => write!(
                f,
                "PlcCmob chunk raw type mismatch: expected 0x{expected:02x}, got 0x{actual:02x}"
            ),
            Self::DuplicateCarrierOhpo { carrier_ohpo } => {
                write!(f, "duplicate carrier Ohpo {carrier_ohpo}")
            }
            Self::DuplicateTargetQsid { target_qsid } => {
                write!(f, "duplicate target Qsid {target_qsid}")
            }
            Self::InvalidUuid { field } => write!(f, "invalid lowercase canonical UUID in {field}"),
            Self::CarrierReparented { carrier_ohpo } => {
                write!(f, "carrier {carrier_ohpo} was reparented")
            }
            Self::UnresolvedOhpo { carrier_ohpo } => {
                write!(f, "unresolved carrier Ohpo {carrier_ohpo}")
            }
            Self::CarrierCmoIdMismatch {
                carrier_ohpo,
                row_cmo_id,
                carrier_cmo_id,
            } => write!(
                f,
                "carrier {carrier_ohpo} CmoID {carrier_cmo_id} != row CmoId {row_cmo_id}"
            ),
            Self::UnresolvedTargetQsid { target_qsid } => {
                write!(f, "unresolved target Qsid {target_qsid}")
            }
            Self::MarkerCountMismatch {
                target_qsid,
                relation_count,
                object_marker_count,
            } => write!(
                f,
                "target Qsid {target_qsid}: {relation_count} PlcCmob rows != {object_marker_count} U+FFFC markers"
            ),
        }
    }
}

impl Error for PlcCmobProjectionError {}

fn read_u32_le(bytes: &[u8], offset: usize) -> Result<u32, PlcCmobProjectionError> {
    let end = offset
        .checked_add(4)
        .ok_or(PlcCmobProjectionError::Truncated {
            at: offset,
            need: 4,
            available: bytes.len().saturating_sub(offset),
        })?;
    let slice = bytes
        .get(offset..end)
        .ok_or(PlcCmobProjectionError::Truncated {
            at: offset,
            need: 4,
            available: bytes.len().saturating_sub(offset),
        })?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn expect_tag(
    bytes: &[u8],
    offset: usize,
    expected_field: u8,
    expected_wire: u8,
    context: &'static str,
) -> Result<(), PlcCmobProjectionError> {
    let pair =
        bytes
            .get(offset..offset.saturating_add(2))
            .ok_or(PlcCmobProjectionError::Truncated {
                at: offset,
                need: 2,
                available: bytes.len().saturating_sub(offset),
            })?;
    if pair[0] != expected_field {
        return Err(PlcCmobProjectionError::WrongField {
            context,
            expected: expected_field,
            actual: pair[0],
        });
    }
    if pair[1] != expected_wire {
        return Err(PlcCmobProjectionError::WrongWire {
            context,
            expected: expected_wire,
            actual: pair[1],
        });
    }
    Ok(())
}

pub fn parse_confirmed_mature_plc_cmob(
    bytes: &[u8],
) -> Result<MaturePlcCmobV1, PlcCmobProjectionError> {
    if bytes.len() < PLC_CMOB_FIXED_PREFIX_SIZE {
        return Err(PlcCmobProjectionError::Truncated {
            at: 0,
            need: PLC_CMOB_FIXED_PREFIX_SIZE,
            available: bytes.len(),
        });
    }

    expect_tag(
        bytes,
        0,
        PLC_CMOB_DECLARED_COUNT_ID,
        BLOCK_TYPE_U32,
        "PlcCmob.IcmobMax",
    )?;
    let declared_count = read_u32_le(bytes, 2)?;

    expect_tag(
        bytes,
        6,
        PLC_CMOB_ENTRY_ARRAY_ID,
        BLOCK_TYPE_ARRAY_CONTAINER,
        "PlcCmob.Rgcmob",
    )?;
    let array_declared_length = read_u32_le(bytes, 8)?;
    let expected_array_declared_length =
        u32::try_from(bytes.len().saturating_sub(8)).map_err(|_| {
            PlcCmobProjectionError::MalformedPayloadLength {
                payload_len: bytes.len(),
            }
        })?;
    if array_declared_length != expected_array_declared_length {
        return Err(PlcCmobProjectionError::DeclaredLengthMismatch {
            context: "PlcCmob.Rgcmob",
            expected: expected_array_declared_length,
            actual: array_declared_length,
        });
    }
    let content_source = read_u32_le(bytes, 12)?;

    let payload_len = bytes.len() - PLC_CMOB_FIXED_PREFIX_SIZE;
    if payload_len % PLC_CMOB_ROW_SIZE != 0 {
        return Err(PlcCmobProjectionError::MalformedPayloadLength { payload_len });
    }
    let actual_count = payload_len / PLC_CMOB_ROW_SIZE;
    if usize::try_from(declared_count).ok() != Some(actual_count) {
        return Err(PlcCmobProjectionError::EntryCountMismatch {
            declared: usize::try_from(declared_count).unwrap_or(usize::MAX),
            actual: actual_count,
        });
    }

    let mut entries = Vec::with_capacity(actual_count);
    for source_order in 0..actual_count {
        let row_offset = PLC_CMOB_FIXED_PREFIX_SIZE + source_order * PLC_CMOB_ROW_SIZE;
        expect_tag(
            bytes,
            row_offset,
            PLC_CMOB_ENTRY_ID,
            BLOCK_TYPE_ROW_CONTAINER,
            "PlcCmob.Rgcmob[]",
        )?;
        let row_declared_length = read_u32_le(bytes, row_offset + 2)?;
        if row_declared_length != PLC_CMOB_ROW_DECLARED_LENGTH {
            return Err(PlcCmobProjectionError::DeclaredLengthMismatch {
                context: "PlcCmob.Rgcmob[]",
                expected: PLC_CMOB_ROW_DECLARED_LENGTH,
                actual: row_declared_length,
            });
        }

        let field_specs = [
            (CMO_ENTRY_CMO_ID, BLOCK_TYPE_U32, "PlcCmob.CmoId"),
            (CMO_ENTRY_TARGET_QSID, BLOCK_TYPE_U32, "PlcCmob.Qsid"),
            (
                CMO_ENTRY_CARRIER_OHPO,
                BLOCK_TYPE_REFERENCE_U32,
                "PlcCmob.Ohpo",
            ),
        ];
        let field_offsets = [row_offset + 6, row_offset + 12, row_offset + 18];
        let mut seen = BTreeSet::new();
        let mut values = [0u32; 3];

        for (idx, ((expected_field, expected_wire, context), field_offset)) in
            field_specs.into_iter().zip(field_offsets).enumerate()
        {
            let pair = bytes.get(field_offset..field_offset + 2).ok_or(
                PlcCmobProjectionError::Truncated {
                    at: field_offset,
                    need: 2,
                    available: bytes.len().saturating_sub(field_offset),
                },
            )?;
            if !seen.insert(pair[0]) {
                return Err(PlcCmobProjectionError::DuplicateField {
                    context: "PlcCmob.Rgcmob[]",
                    field_id: pair[0],
                });
            }
            if pair[0] != expected_field {
                return Err(PlcCmobProjectionError::WrongField {
                    context,
                    expected: expected_field,
                    actual: pair[0],
                });
            }
            if pair[1] != expected_wire {
                return Err(PlcCmobProjectionError::WrongWire {
                    context,
                    expected: expected_wire,
                    actual: pair[1],
                });
            }
            values[idx] = read_u32_le(bytes, field_offset + 2)?;
        }

        entries.push(MatureCmobEntryV1 {
            source_order,
            source: RawSpanV1 {
                offset: row_offset,
                len: PLC_CMOB_ROW_SIZE,
            },
            cmo_id: values[0],
            cmo_id_source: RawSpanV1 {
                offset: row_offset + 6,
                len: 6,
            },
            target_qsid: values[1],
            target_qsid_source: RawSpanV1 {
                offset: row_offset + 12,
                len: 6,
            },
            carrier_ohpo: values[2],
            carrier_ohpo_source: RawSpanV1 {
                offset: row_offset + 18,
                len: 6,
            },
        });
    }

    Ok(MaturePlcCmobV1 {
        source: RawSpanV1 {
            offset: 0,
            len: bytes.len(),
        },
        declared_count,
        declared_count_source: RawSpanV1 { offset: 0, len: 6 },
        entry_array_source: RawSpanV1 {
            offset: 6,
            len: bytes.len() - 6,
        },
        content_source,
        entries,
    })
}

pub fn decode_hex(value: &str) -> Result<Vec<u8>, PlcCmobProjectionError> {
    let bytes = value.as_bytes();
    if bytes.len() % 2 != 0 {
        return Err(PlcCmobProjectionError::InvalidHexLength);
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for i in (0..bytes.len()).step_by(2) {
        let high =
            hex_nibble(bytes[i]).ok_or(PlcCmobProjectionError::InvalidHexByte { index: i })?;
        let low = hex_nibble(bytes[i + 1])
            .ok_or(PlcCmobProjectionError::InvalidHexByte { index: i + 1 })?;
        out.push((high << 4) | low);
    }
    Ok(out)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_token(value: &str, allow_colon: bool, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'_' | b'-')
                || (allow_colon && byte == b':')
        })
}

fn valid_uuid(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }
    value.bytes().enumerate().all(|(idx, byte)| {
        if matches!(idx, 8 | 13 | 18 | 23) {
            byte == b'-'
        } else {
            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
        }
    })
}

fn probe(code: &str) -> FailClosedProbeV1 {
    FailClosedProbeV1 {
        code: code.to_owned(),
        diagnostic_emitted: true,
        projection_emitted: false,
    }
}

pub fn default_fail_closed_probes_v1() -> FailClosedProbesV1 {
    FailClosedProbesV1 {
        count_mismatch: probe("plccmob.count_mismatch"),
        malformed_row: probe("plccmob.malformed_row"),
        unresolved_ohpo: probe("plccmob.unresolved_ohpo"),
        carrier_cmo_id_mismatch: probe("plccmob.carrier_cmo_id_mismatch"),
        unresolved_target_qsid: probe("plccmob.unresolved_target_qsid"),
        marker_count_mismatch: probe("plccmob.marker_count_mismatch"),
    }
}

pub fn build_projection_receipt_v1(
    input: &PlcCmobProjectionInputV1,
) -> Result<PlcCmobProjectionReceiptV1, PlcCmobProjectionError> {
    if !is_lower_hex_64(&input.source_hash) {
        return Err(PlcCmobProjectionError::InvalidSourceHash);
    }
    if !valid_token(&input.producer.implementation, false, 128) {
        return Err(PlcCmobProjectionError::InvalidProducerField {
            field: "producer.implementation",
        });
    }
    if !valid_token(&input.producer.commit_or_build, true, 160) {
        return Err(PlcCmobProjectionError::InvalidProducerField {
            field: "producer.commit_or_build",
        });
    }
    if !input.producer.core_integration {
        return Err(PlcCmobProjectionError::InvalidProducerField {
            field: "producer.core_integration",
        });
    }

    if input.opl_docq_field.field_id != OPL_DOCQ_PLC_CMOB_FIELD_ID {
        return Err(PlcCmobProjectionError::LocatorFieldMismatch {
            expected: OPL_DOCQ_PLC_CMOB_FIELD_ID,
            actual: input.opl_docq_field.field_id,
        });
    }
    if input.opl_docq_field.block_type != BLOCK_TYPE_OBJECT_HANDLE_U32 {
        return Err(PlcCmobProjectionError::LocatorWireMismatch {
            expected: BLOCK_TYPE_OBJECT_HANDLE_U32,
            actual: input.opl_docq_field.block_type,
        });
    }
    if input.opl_docq_field.value != input.plccmob_chunk.seq_num {
        return Err(PlcCmobProjectionError::LocatorHandleMismatch {
            field_value: input.opl_docq_field.value,
            chunk_seq_num: input.plccmob_chunk.seq_num,
        });
    }
    if input.plccmob_chunk.raw_type != CONTENTS_RAW_TYPE_PLC_CMOB {
        return Err(PlcCmobProjectionError::PlcCmobRawTypeMismatch {
            expected: CONTENTS_RAW_TYPE_PLC_CMOB,
            actual: input.plccmob_chunk.raw_type,
        });
    }

    let raw = decode_hex(&input.plccmob_chunk.hex)?;
    let parsed = parse_confirmed_mature_plc_cmob(&raw)?;

    let mut carriers = BTreeMap::new();
    for carrier in &input.carriers {
        if !valid_uuid(&carrier.carrier_node_id) {
            return Err(PlcCmobProjectionError::InvalidUuid {
                field: "carrier_node_id",
            });
        }
        if let Some(story_id) = &carrier.carrier_story_id {
            if !valid_uuid(story_id) {
                return Err(PlcCmobProjectionError::InvalidUuid {
                    field: "carrier_story_id",
                });
            }
        }
        if carrier.source_parent_id != carrier.effective_parent_id {
            return Err(PlcCmobProjectionError::CarrierReparented {
                carrier_ohpo: carrier.carrier_ohpo,
            });
        }
        if carriers.insert(carrier.carrier_ohpo, carrier).is_some() {
            return Err(PlcCmobProjectionError::DuplicateCarrierOhpo {
                carrier_ohpo: carrier.carrier_ohpo,
            });
        }
    }

    let mut targets = BTreeMap::new();
    for target in &input.targets {
        if !valid_uuid(&target.target_story_id) {
            return Err(PlcCmobProjectionError::InvalidUuid {
                field: "target_story_id",
            });
        }
        if let Some(frame_id) = &target.target_frame_node_id {
            if !valid_uuid(frame_id) {
                return Err(PlcCmobProjectionError::InvalidUuid {
                    field: "target_frame_node_id",
                });
            }
        }
        if targets.insert(target.target_qsid, target).is_some() {
            return Err(PlcCmobProjectionError::DuplicateTargetQsid {
                target_qsid: target.target_qsid,
            });
        }
    }

    let mut relations = Vec::with_capacity(parsed.entries.len());
    let mut relation_counts = BTreeMap::<u32, usize>::new();

    for entry in &parsed.entries {
        let carrier =
            carriers
                .get(&entry.carrier_ohpo)
                .ok_or(PlcCmobProjectionError::UnresolvedOhpo {
                    carrier_ohpo: entry.carrier_ohpo,
                })?;
        if carrier.carrier_cmo_id != entry.cmo_id {
            return Err(PlcCmobProjectionError::CarrierCmoIdMismatch {
                carrier_ohpo: entry.carrier_ohpo,
                row_cmo_id: entry.cmo_id,
                carrier_cmo_id: carrier.carrier_cmo_id,
            });
        }

        let target = targets.get(&entry.target_qsid).ok_or(
            PlcCmobProjectionError::UnresolvedTargetQsid {
                target_qsid: entry.target_qsid,
            },
        )?;

        relations.push(CmoProjectionRelationV1 {
            source_order: entry.source_order,
            cmo_id: entry.cmo_id,
            carrier_ohpo: entry.carrier_ohpo,
            carrier_cmo_id: carrier.carrier_cmo_id,
            target_qsid: entry.target_qsid,
            carrier_node_id: carrier.carrier_node_id.clone(),
            carrier_story_id: carrier.carrier_story_id.clone(),
            target_story_id: target.target_story_id.clone(),
            target_frame_node_id: target.target_frame_node_id.clone(),
        });
        *relation_counts.entry(entry.target_qsid).or_insert(0) += 1;
    }

    let mut target_receipts = Vec::with_capacity(relation_counts.len());
    for (target_qsid, relation_count) in relation_counts {
        let target = targets
            .get(&target_qsid)
            .expect("target was validated before relation emission");
        if target.object_marker_count != relation_count {
            return Err(PlcCmobProjectionError::MarkerCountMismatch {
                target_qsid,
                relation_count,
                object_marker_count: target.object_marker_count,
            });
        }
        target_receipts.push(PlcCmobTargetReceiptV1 {
            target_qsid,
            relation_count,
            object_marker_count: target.object_marker_count,
        });
    }

    Ok(PlcCmobProjectionReceiptV1 {
        receipt_version: RECEIPT_VERSION_V1.to_owned(),
        producer: input.producer.clone(),
        source_hash: input.source_hash.clone(),
        projection_context_version: PROJECTION_CONTEXT_VERSION_V1.to_owned(),
        plc_cmob: PlcCmobReceiptSummaryV1 {
            declared_count: usize::try_from(parsed.declared_count).unwrap_or(usize::MAX),
            row_count: parsed.entries.len(),
            raw_size: raw.len(),
        },
        relations,
        targets: target_receipts,
        invariants: PlcCmobInvariantsReceiptV1 {
            source_parentage_preserved: true,
            carrier_reparent_count: 0,
            raw_text_emitted: false,
            ordered_relation: true,
        },
        fail_closed_probes: default_fail_closed_probes_v1(),
    })
}


pub fn build_pub_projection_context_v1(
    input: &PlcCmobProjectionInputV1,
) -> Result<PubProjectionContextV1, PlcCmobProjectionError> {
    let receipt = build_projection_receipt_v1(input)?;
    Ok(PubProjectionContextV1::with_cmo_relations(\n        receipt.relations,\n    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_u32_field(out: &mut Vec<u8>, field_id: u8, wire: u8, value: u32) {
        out.push(field_id);
        out.push(wire);
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn fixture(entries: &[(u32, u32, u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        push_u32_field(
            &mut out,
            PLC_CMOB_DECLARED_COUNT_ID,
            BLOCK_TYPE_U32,
            u32::try_from(entries.len()).expect("fixture count"),
        );
        out.push(PLC_CMOB_ENTRY_ARRAY_ID);
        out.push(BLOCK_TYPE_ARRAY_CONTAINER);
        let declared = 8 + 24 * u32::try_from(entries.len()).expect("fixture count");
        out.extend_from_slice(&declared.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());

        for (cmo_id, target_qsid, carrier_ohpo) in entries {
            out.push(PLC_CMOB_ENTRY_ID);
            out.push(BLOCK_TYPE_ROW_CONTAINER);
            out.extend_from_slice(&PLC_CMOB_ROW_DECLARED_LENGTH.to_le_bytes());
            push_u32_field(&mut out, CMO_ENTRY_CMO_ID, BLOCK_TYPE_U32, *cmo_id);
            push_u32_field(
                &mut out,
                CMO_ENTRY_TARGET_QSID,
                BLOCK_TYPE_U32,
                *target_qsid,
            );
            push_u32_field(
                &mut out,
                CMO_ENTRY_CARRIER_OHPO,
                BLOCK_TYPE_REFERENCE_U32,
                *carrier_ohpo,
            );
        }
        out
    }

    fn encode_hex(bytes: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        out
    }

    fn valid_input() -> PlcCmobProjectionInputV1 {
        PlcCmobProjectionInputV1 {
            source_hash: "a".repeat(64),
            producer: ProducerV1 {
                implementation: "rar-pub-plccmob-projection".to_owned(),
                commit_or_build: "synthetic-test".to_owned(),
                core_integration: true,
            },
            opl_docq_field: ExactU32FieldV1 {
                field_id: 0x05,
                block_type: 0x70,
                value: 900,
            },
            plccmob_chunk: PlcCmobChunkInputV1 {
                seq_num: 900,
                raw_type: 0x70,
                hex: encode_hex(&fixture(&[(7, 49, 441), (9, 49, 446)])),
            },
            carriers: vec![
                CarrierSourceV1 {
                    carrier_ohpo: 441,
                    carrier_cmo_id: 7,
                    carrier_node_id: "10000000-0000-4000-8000-000000000001".to_owned(),
                    carrier_story_id: Some("20000000-0000-4000-8000-000000000001".to_owned()),
                    source_parent_id: "page:279".to_owned(),
                    effective_parent_id: "page:279".to_owned(),
                },
                CarrierSourceV1 {
                    carrier_ohpo: 446,
                    carrier_cmo_id: 9,
                    carrier_node_id: "10000000-0000-4000-8000-000000000002".to_owned(),
                    carrier_story_id: None,
                    source_parent_id: "page:279".to_owned(),
                    effective_parent_id: "page:279".to_owned(),
                },
            ],
            targets: vec![TargetSourceV1 {
                target_qsid: 49,
                target_story_id: "30000000-0000-4000-8000-000000000001".to_owned(),
                target_frame_node_id: Some("40000000-0000-4000-8000-000000000001".to_owned()),
                object_marker_count: 2,
            }],
        }
    }

    #[test]
    fn parses_exact_n1_and_n2_wire_and_preserves_order() {
        let one = parse_confirmed_mature_plc_cmob(&fixture(&[(1, 218, 319)])).expect("N=1");
        assert_eq!(one.declared_count, 1);
        assert_eq!(one.entries.len(), 1);
        assert_eq!(one.entries[0].source_order, 0);
        assert_eq!(one.entries[0].cmo_id, 1);
        assert_eq!(one.entries[0].target_qsid, 218);
        assert_eq!(one.entries[0].carrier_ohpo, 319);

        let two =
            parse_confirmed_mature_plc_cmob(&fixture(&[(7, 49, 441), (9, 49, 446)])).expect("N=2");
        assert_eq!(two.declared_count, 2);
        assert_eq!(two.entries.len(), 2);
        assert_eq!(two.entries[1].source_order, 1);
        assert_eq!(two.entries[1].carrier_ohpo, 446);
    }

    #[test]
    fn synthetic_march_and_december_sizes_match_grounded_law() {
        let march_entries = (0..9)
            .map(|idx| (idx + 1, 49, 400 + idx))
            .collect::<Vec<_>>();
        let december_entries = (0..21)
            .map(|idx| (idx + 1, 170, 500 + idx))
            .collect::<Vec<_>>();
        let march = fixture(&march_entries);
        let december = fixture(&december_entries);

        assert_eq!(march.len(), 232);
        assert_eq!(december.len(), 520);
        assert_eq!(
            parse_confirmed_mature_plc_cmob(&march)
                .expect("march")
                .entries
                .len(),
            9
        );
        assert_eq!(
            parse_confirmed_mature_plc_cmob(&december)
                .expect("december")
                .entries
                .len(),
            21
        );
    }

    #[test]
    fn count_mismatch_fails_closed() {
        let mut raw = fixture(&[(1, 218, 319)]);
        raw[2..6].copy_from_slice(&2u32.to_le_bytes());
        assert!(matches!(
            parse_confirmed_mature_plc_cmob(&raw),
            Err(PlcCmobProjectionError::EntryCountMismatch { .. })
        ));
    }

    #[test]
    fn exact_wire_mismatches_fail_closed() {
        let mut count_wire = fixture(&[(1, 218, 319)]);
        count_wire[1] = BLOCK_TYPE_REFERENCE_U32;
        assert!(matches!(
            parse_confirmed_mature_plc_cmob(&count_wire),
            Err(PlcCmobProjectionError::WrongWire { .. })
        ));

        let mut array_wire = fixture(&[(1, 218, 319)]);
        array_wire[7] = BLOCK_TYPE_ROW_CONTAINER;
        assert!(matches!(
            parse_confirmed_mature_plc_cmob(&array_wire),
            Err(PlcCmobProjectionError::WrongWire { .. })
        ));

        let mut row_wire = fixture(&[(1, 218, 319)]);
        row_wire[17] = BLOCK_TYPE_ARRAY_CONTAINER;
        assert!(matches!(
            parse_confirmed_mature_plc_cmob(&row_wire),
            Err(PlcCmobProjectionError::WrongWire { .. })
        ));

        let mut ohpo_wire = fixture(&[(1, 218, 319)]);
        ohpo_wire[35] = BLOCK_TYPE_U32;
        assert!(matches!(
            parse_confirmed_mature_plc_cmob(&ohpo_wire),
            Err(PlcCmobProjectionError::WrongWire { .. })
        ));
    }

    #[test]
    fn duplicate_and_truncated_rows_fail_closed() {
        let mut duplicate = fixture(&[(1, 218, 319)]);
        duplicate[28] = CMO_ENTRY_CMO_ID;
        assert!(matches!(
            parse_confirmed_mature_plc_cmob(&duplicate),
            Err(PlcCmobProjectionError::DuplicateField { .. })
        ));

        let mut truncated = fixture(&[(1, 218, 319)]);
        truncated.pop();
        assert!(parse_confirmed_mature_plc_cmob(&truncated).is_err());
    }

    #[test]
    fn successful_join_materializes_shared_projection_context() {
        let context = build_pub_projection_context_v1(&valid_input()).expect("context");
        assert_eq!(context.schema_version, PROJECTION_CONTEXT_VERSION_V1);
        assert_eq!(context.cmo_relations.len(), 2);
        assert_eq!(context.cmo_relations[0].source_order, 0);
        assert_eq!(context.cmo_relations[1].source_order, 1);
        assert_eq!(
            context
                .cmo_relations_for_target_qsid(49)
                .map(|relation| relation.carrier_ohpo)
                .collect::<Vec<_>>(),
            vec![441, 446]
        );
    }

    #[test]
    fn successful_join_emits_existing_source_free_receipt_shape() {
        let receipt = build_projection_receipt_v1(&valid_input()).expect("receipt");
        assert_eq!(receipt.receipt_version, RECEIPT_VERSION_V1);
        assert_eq!(
            receipt.projection_context_version,
            PROJECTION_CONTEXT_VERSION_V1
        );
        assert_eq!(receipt.plc_cmob.declared_count, 2);
        assert_eq!(receipt.plc_cmob.row_count, 2);
        assert_eq!(receipt.plc_cmob.raw_size, 64);
        assert_eq!(receipt.relations.len(), 2);
        assert_eq!(receipt.relations[0].source_order, 0);
        assert_eq!(receipt.relations[1].source_order, 1);
        assert_eq!(receipt.targets.len(), 1);
        assert_eq!(receipt.targets[0].target_qsid, 49);
        assert_eq!(receipt.targets[0].relation_count, 2);
        assert_eq!(receipt.targets[0].object_marker_count, 2);
        assert!(receipt.invariants.source_parentage_preserved);
        assert_eq!(receipt.invariants.carrier_reparent_count, 0);
        assert!(!receipt.invariants.raw_text_emitted);
    }

    #[test]
    fn authoritative_opldocq_locator_is_exact_and_not_raw_type_guessing() {
        let mut input = valid_input();
        input.opl_docq_field.block_type = BLOCK_TYPE_REFERENCE_U32;
        assert!(matches!(
            build_projection_receipt_v1(&input),
            Err(PlcCmobProjectionError::LocatorWireMismatch { .. })
        ));

        let mut input = valid_input();
        input.opl_docq_field.value += 1;
        assert!(matches!(
            build_projection_receipt_v1(&input),
            Err(PlcCmobProjectionError::LocatorHandleMismatch { .. })
        ));

        let mut input = valid_input();
        input.plccmob_chunk.raw_type = 0x6f;
        assert!(matches!(
            build_projection_receipt_v1(&input),
            Err(PlcCmobProjectionError::PlcCmobRawTypeMismatch { .. })
        ));
    }

    #[test]
    fn unresolved_join_states_and_cardinality_fail_closed() {
        let mut input = valid_input();
        input.carriers.remove(0);
        assert!(matches!(
            build_projection_receipt_v1(&input),
            Err(PlcCmobProjectionError::UnresolvedOhpo { .. })
        ));

        let mut input = valid_input();
        input.carriers[0].carrier_cmo_id = 99;
        assert!(matches!(
            build_projection_receipt_v1(&input),
            Err(PlcCmobProjectionError::CarrierCmoIdMismatch { .. })
        ));

        let mut input = valid_input();
        input.targets.clear();
        assert!(matches!(
            build_projection_receipt_v1(&input),
            Err(PlcCmobProjectionError::UnresolvedTargetQsid { .. })
        ));

        let mut input = valid_input();
        input.targets[0].object_marker_count = 1;
        assert!(matches!(
            build_projection_receipt_v1(&input),
            Err(PlcCmobProjectionError::MarkerCountMismatch { .. })
        ));
    }

    #[test]
    fn carrier_parentage_is_preservation_state_not_target_reparenting() {
        let mut input = valid_input();
        input.carriers[0].effective_parent_id = "page:266".to_owned();
        assert!(matches!(
            build_projection_receipt_v1(&input),
            Err(PlcCmobProjectionError::CarrierReparented { .. })
        ));
    }

    #[test]
    fn input_surface_has_no_customer_text_field_and_json_receipt_matches_public_names() {
        let receipt = build_projection_receipt_v1(&valid_input()).expect("receipt");
        let json = serde_json::to_value(receipt).expect("serialize");
        assert!(json.get("relations").is_some());
        assert!(json.get("targets").is_some());
        assert!(json.get("carrier_text").is_none());
        assert!(json.get("private_checkout_path").is_none());
        assert_eq!(
            json["fail_closed_probes"]["unresolved_ohpo"]["projection_emitted"],
            false
        );
    }
}
