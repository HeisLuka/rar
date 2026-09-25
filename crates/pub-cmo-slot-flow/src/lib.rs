//! Native bounded Cmo/U+FFFC object-slot flow.
//!
//! This crate is the Rust authority for the already-landed source-free
//! CMO-SLOT-FLOW-01 contract. It deliberately accepts no raw Story text.
//! Upstream code must prove the U+FFFC marker positions and PlcCmob relation
//! ordering before constructing this input.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::{error::Error, fmt};

pub const INPUT_VERSION_V1: &str = "chaptera.cmo-slot-flow-input.v1";
pub const RECEIPT_VERSION_V1: &str = "chaptera.cmo-slot-flow-receipt.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProducerV1 {
    pub implementation: String,
    pub commit_or_build: String,
    pub core_integration: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostV1 {
    pub width_emu: i64,
    pub height_emu: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FlowItemV1 {
    ShapedLine {
        scalar_start: u32,
        scalar_end: u32,
        consumed_scalar_end: u32,
        height_emu: i64,
    },
    ObjectSlot {
        slot_index: usize,
        scalar_index: u32,
        source_order: usize,
        cmo_id: u32,
        carrier_node_id: String,
        carrier_story_id: Option<String>,
        intrinsic_width_emu: i64,
        intrinsic_height_emu: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CmoSlotFlowInputV1 {
    pub schema_version: String,
    pub producer: ProducerV1,
    pub source_hash: String,
    pub target_story_id: String,
    pub target_frame_node_id: String,
    pub target_frame_count: u32,
    pub story_marker_count: usize,
    pub host: HostV1,
    pub items: Vec<FlowItemV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleSlotV1 {
    pub slot_index: usize,
    pub scalar_index: u32,
    pub source_order: usize,
    pub cmo_id: u32,
    pub carrier_node_id: String,
    pub carrier_story_id: Option<String>,
    pub instance_id: String,
    pub preceding_text_height_emu: i64,
    pub used_height_before_emu: i64,
    pub used_height_after_emu: i64,
    pub intrinsic_width_emu: i64,
    pub intrinsic_height_emu: i64,
    pub resolved_x_emu: i64,
    pub resolved_y_emu: i64,
    pub resolved_width_emu: i64,
    pub resolved_height_emu: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OversetV1 {
    pub story_overset: bool,
    pub first_nonfitting_kind: Option<String>,
    pub first_nonfitting_slot_index: Option<usize>,
    pub first_nonfitting_scalar_index: Option<u32>,
    pub failure_reason: Option<String>,
    pub remaining_item_count: usize,
    pub remaining_slot_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantsV1 {
    pub u_fffc_marker_count_preserved: bool,
    pub slot_order_preserved: bool,
    pub carrier_reparent_count: u32,
    pub scaling_applied: bool,
    pub skip_to_fit: bool,
    pub raw_text_emitted: bool,
    pub nested_cmo_admitted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CmoSlotFlowReceiptV1 {
    pub receipt_version: String,
    pub producer: ProducerV1,
    pub source_hash: String,
    pub target_story_id: String,
    pub target_frame_node_id: String,
    pub host: HostV1,
    pub story_marker_count: usize,
    pub slot_count: usize,
    pub visible_slots: Vec<VisibleSlotV1>,
    pub overset: OversetV1,
    pub invariants: InvariantsV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CmoSlotFlowError {
    SchemaVersion,
    ProducerField(&'static str),
    SourceHash,
    TargetFrameCount(u32),
    TargetStoryId,
    TargetFrameNodeId,
    HostWidth(i64),
    HostHeight(i64),
    ShapedLineRange {
        index: usize,
    },
    NonPositiveShapedLineHeight {
        index: usize,
        value: i64,
    },
    SlotOrdinal {
        index: usize,
        expected: usize,
        actual: usize,
    },
    SlotScalarOrder {
        index: usize,
    },
    SlotSourceOrder {
        index: usize,
    },
    ZeroCmoId {
        index: usize,
    },
    CarrierNodeId {
        index: usize,
    },
    CarrierStoryId {
        index: usize,
    },
    NonPositiveSlotWidth {
        index: usize,
        value: i64,
    },
    NonPositiveSlotHeight {
        index: usize,
        value: i64,
    },
    MarkerCount {
        markers: usize,
        slots: usize,
    },
    MetricOverflow,
    Serialization,
}

impl fmt::Display for CmoSlotFlowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SchemaVersion => write!(f, "input schema_version mismatch"),
            Self::ProducerField(field) => write!(f, "invalid producer field {field}"),
            Self::SourceHash => write!(f, "source_hash must be lowercase SHA-256"),
            Self::TargetFrameCount(count) => write!(
                f,
                "bounded Cmo slot-flow V1 admits exactly one target frame, got {count}"
            ),
            Self::TargetStoryId => write!(f, "target_story_id must be canonical lowercase UUID"),
            Self::TargetFrameNodeId => {
                write!(f, "target_frame_node_id must be canonical lowercase UUID")
            }
            Self::HostWidth(value) => write!(f, "host.width_emu must be positive, got {value}"),
            Self::HostHeight(value) => write!(f, "host.height_emu must be positive, got {value}"),
            Self::ShapedLineRange { index } => {
                write!(f, "items[{index}] has invalid shaped-line scalar range")
            }
            Self::NonPositiveShapedLineHeight { index, value } => {
                write!(f, "items[{index}].height_emu must be positive, got {value}")
            }
            Self::SlotOrdinal {
                index,
                expected,
                actual,
            } => write!(
                f,
                "items[{index}].slot_index {actual} is not canonical ordinal {expected}"
            ),
            Self::SlotScalarOrder { index } => {
                write!(
                    f,
                    "items[{index}] object-slot scalar index is not increasing"
                )
            }
            Self::SlotSourceOrder { index } => {
                write!(f, "items[{index}] Cmo source_order is not increasing")
            }
            Self::ZeroCmoId { index } => write!(f, "items[{index}].cmo_id must be positive"),
            Self::CarrierNodeId { index } => {
                write!(
                    f,
                    "items[{index}].carrier_node_id must be canonical lowercase UUID"
                )
            }
            Self::CarrierStoryId { index } => {
                write!(
                    f,
                    "items[{index}].carrier_story_id must be canonical lowercase UUID"
                )
            }
            Self::NonPositiveSlotWidth { index, value } => write!(
                f,
                "items[{index}].intrinsic_width_emu must be positive, got {value}"
            ),
            Self::NonPositiveSlotHeight { index, value } => write!(
                f,
                "items[{index}].intrinsic_height_emu must be positive, got {value}"
            ),
            Self::MarkerCount { markers, slots } => {
                write!(
                    f,
                    "story_marker_count {markers} != object slot count {slots}"
                )
            }
            Self::MetricOverflow => write!(f, "slot-flow EMU cursor overflowed"),
            Self::Serialization => write!(f, "slot-flow identity serialization failed"),
        }
    }
}

impl Error for CmoSlotFlowError {}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
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

fn validate_input(input: &CmoSlotFlowInputV1) -> Result<usize, CmoSlotFlowError> {
    if input.schema_version != INPUT_VERSION_V1 {
        return Err(CmoSlotFlowError::SchemaVersion);
    }
    if !valid_token(&input.producer.implementation, false, 128) {
        return Err(CmoSlotFlowError::ProducerField("producer.implementation"));
    }
    if !valid_token(&input.producer.commit_or_build, true, 160) {
        return Err(CmoSlotFlowError::ProducerField("producer.commit_or_build"));
    }
    if !input.producer.core_integration {
        return Err(CmoSlotFlowError::ProducerField("producer.core_integration"));
    }
    if !is_lower_sha256(&input.source_hash) {
        return Err(CmoSlotFlowError::SourceHash);
    }
    if !valid_uuid(&input.target_story_id) {
        return Err(CmoSlotFlowError::TargetStoryId);
    }
    if !valid_uuid(&input.target_frame_node_id) {
        return Err(CmoSlotFlowError::TargetFrameNodeId);
    }
    if input.target_frame_count != 1 {
        return Err(CmoSlotFlowError::TargetFrameCount(input.target_frame_count));
    }
    if input.host.width_emu <= 0 {
        return Err(CmoSlotFlowError::HostWidth(input.host.width_emu));
    }
    if input.host.height_emu <= 0 {
        return Err(CmoSlotFlowError::HostHeight(input.host.height_emu));
    }

    let mut slot_count = 0usize;
    let mut last_scalar: Option<u32> = None;
    let mut last_source_order: Option<usize> = None;

    for (index, item) in input.items.iter().enumerate() {
        match item {
            FlowItemV1::ShapedLine {
                scalar_start,
                scalar_end,
                consumed_scalar_end,
                height_emu,
            } => {
                if scalar_end < scalar_start || consumed_scalar_end < scalar_end {
                    return Err(CmoSlotFlowError::ShapedLineRange { index });
                }
                if *height_emu <= 0 {
                    return Err(CmoSlotFlowError::NonPositiveShapedLineHeight {
                        index,
                        value: *height_emu,
                    });
                }
            }
            FlowItemV1::ObjectSlot {
                slot_index,
                scalar_index,
                source_order,
                cmo_id,
                carrier_node_id,
                carrier_story_id,
                intrinsic_width_emu,
                intrinsic_height_emu,
            } => {
                if *slot_index != slot_count {
                    return Err(CmoSlotFlowError::SlotOrdinal {
                        index,
                        expected: slot_count,
                        actual: *slot_index,
                    });
                }
                if last_scalar.is_some_and(|value| *scalar_index <= value) {
                    return Err(CmoSlotFlowError::SlotScalarOrder { index });
                }
                if last_source_order.is_some_and(|value| *source_order <= value) {
                    return Err(CmoSlotFlowError::SlotSourceOrder { index });
                }
                if *cmo_id == 0 {
                    return Err(CmoSlotFlowError::ZeroCmoId { index });
                }
                if !valid_uuid(carrier_node_id) {
                    return Err(CmoSlotFlowError::CarrierNodeId { index });
                }
                if carrier_story_id
                    .as_deref()
                    .is_some_and(|value| !valid_uuid(value))
                {
                    return Err(CmoSlotFlowError::CarrierStoryId { index });
                }
                if *intrinsic_width_emu <= 0 {
                    return Err(CmoSlotFlowError::NonPositiveSlotWidth {
                        index,
                        value: *intrinsic_width_emu,
                    });
                }
                if *intrinsic_height_emu <= 0 {
                    return Err(CmoSlotFlowError::NonPositiveSlotHeight {
                        index,
                        value: *intrinsic_height_emu,
                    });
                }

                slot_count += 1;
                last_scalar = Some(*scalar_index);
                last_source_order = Some(*source_order);
            }
        }
    }

    if input.story_marker_count != slot_count {
        return Err(CmoSlotFlowError::MarkerCount {
            markers: input.story_marker_count,
            slots: slot_count,
        });
    }
    Ok(slot_count)
}

fn instance_id(
    target_story_id: &str,
    target_frame_node_id: &str,
    scalar_index: u32,
    carrier_node_id: &str,
    source_order: usize,
) -> Result<String, CmoSlotFlowError> {
    let mut payload = BTreeMap::<String, Value>::new();
    payload.insert(
        "carrier_node_id".into(),
        Value::String(carrier_node_id.to_owned()),
    );
    payload.insert(
        "projection_kind".into(),
        Value::String("cmo_story_slot".into()),
    );
    payload.insert(
        "scalar_index".into(),
        Value::Number(serde_json::Number::from(scalar_index)),
    );
    payload.insert(
        "source_order".into(),
        Value::Number(serde_json::Number::from(
            u64::try_from(source_order).map_err(|_| CmoSlotFlowError::Serialization)?,
        )),
    );
    payload.insert(
        "target_frame_node_id".into(),
        Value::String(target_frame_node_id.to_owned()),
    );
    payload.insert(
        "target_story_id".into(),
        Value::String(target_story_id.to_owned()),
    );

    let bytes = serde_json::to_vec(&payload).map_err(|_| CmoSlotFlowError::Serialization)?;
    let digest = Sha256::digest(bytes);
    Ok(format!("sha256:{digest:x}"))
}

pub fn build_receipt_v1(
    input: &CmoSlotFlowInputV1,
) -> Result<CmoSlotFlowReceiptV1, CmoSlotFlowError> {
    let slot_count = validate_input(input)?;

    let mut used_height = 0i64;
    let mut pending_text_height = 0i64;
    let mut visible_slots = Vec::new();

    let mut first_nonfitting_index: Option<usize> = None;
    let mut first_nonfitting_kind: Option<String> = None;
    let mut first_nonfitting_slot_index: Option<usize> = None;
    let mut first_nonfitting_scalar_index: Option<u32> = None;
    let mut failure_reason: Option<String> = None;

    for (item_index, item) in input.items.iter().enumerate() {
        match item {
            FlowItemV1::ShapedLine {
                scalar_start,
                height_emu,
                ..
            } => {
                let next = used_height
                    .checked_add(*height_emu)
                    .ok_or(CmoSlotFlowError::MetricOverflow)?;
                if next > input.host.height_emu {
                    first_nonfitting_index = Some(item_index);
                    first_nonfitting_kind = Some("shaped_line".into());
                    first_nonfitting_scalar_index = Some(*scalar_start);
                    failure_reason = Some("height".into());
                    break;
                }
                used_height = next;
                pending_text_height = pending_text_height
                    .checked_add(*height_emu)
                    .ok_or(CmoSlotFlowError::MetricOverflow)?;
            }
            FlowItemV1::ObjectSlot {
                slot_index,
                scalar_index,
                source_order,
                cmo_id,
                carrier_node_id,
                carrier_story_id,
                intrinsic_width_emu,
                intrinsic_height_emu,
            } => {
                let width_fits = *intrinsic_width_emu <= input.host.width_emu;
                let next = used_height
                    .checked_add(*intrinsic_height_emu)
                    .ok_or(CmoSlotFlowError::MetricOverflow)?;
                let height_fits = next <= input.host.height_emu;
                if !width_fits || !height_fits {
                    first_nonfitting_index = Some(item_index);
                    first_nonfitting_kind = Some("object_slot".into());
                    first_nonfitting_slot_index = Some(*slot_index);
                    first_nonfitting_scalar_index = Some(*scalar_index);
                    failure_reason = Some(
                        match (width_fits, height_fits) {
                            (false, false) => "width_and_height",
                            (false, true) => "width",
                            (true, false) => "height",
                            (true, true) => unreachable!(),
                        }
                        .into(),
                    );
                    break;
                }

                visible_slots.push(VisibleSlotV1 {
                    slot_index: *slot_index,
                    scalar_index: *scalar_index,
                    source_order: *source_order,
                    cmo_id: *cmo_id,
                    carrier_node_id: carrier_node_id.clone(),
                    carrier_story_id: carrier_story_id.clone(),
                    instance_id: instance_id(
                        &input.target_story_id,
                        &input.target_frame_node_id,
                        *scalar_index,
                        carrier_node_id,
                        *source_order,
                    )?,
                    preceding_text_height_emu: pending_text_height,
                    used_height_before_emu: used_height,
                    used_height_after_emu: next,
                    intrinsic_width_emu: *intrinsic_width_emu,
                    intrinsic_height_emu: *intrinsic_height_emu,
                    resolved_x_emu: 0,
                    resolved_y_emu: used_height,
                    resolved_width_emu: *intrinsic_width_emu,
                    resolved_height_emu: *intrinsic_height_emu,
                });
                used_height = next;
                pending_text_height = 0;
            }
        }
    }

    let (remaining_item_count, remaining_slot_count) = if let Some(index) = first_nonfitting_index {
        let tail = &input.items[index..];
        (
            tail.len(),
            tail.iter()
                .filter(|item| matches!(item, FlowItemV1::ObjectSlot { .. }))
                .count(),
        )
    } else {
        (0, 0)
    };

    Ok(CmoSlotFlowReceiptV1 {
        receipt_version: RECEIPT_VERSION_V1.into(),
        producer: input.producer.clone(),
        source_hash: input.source_hash.clone(),
        target_story_id: input.target_story_id.clone(),
        target_frame_node_id: input.target_frame_node_id.clone(),
        host: input.host.clone(),
        story_marker_count: input.story_marker_count,
        slot_count,
        visible_slots,
        overset: OversetV1 {
            story_overset: first_nonfitting_index.is_some(),
            first_nonfitting_kind,
            first_nonfitting_slot_index,
            first_nonfitting_scalar_index,
            failure_reason,
            remaining_item_count,
            remaining_slot_count,
        },
        invariants: InvariantsV1 {
            u_fffc_marker_count_preserved: true,
            slot_order_preserved: true,
            carrier_reparent_count: 0,
            scaling_applied: false,
            skip_to_fit: false,
            raw_text_emitted: false,
            nested_cmo_admitted: false,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STORY: &str = "30000000-0000-4000-8000-000000000001";
    const FRAME: &str = "40000000-0000-4000-8000-000000000001";
    const CARRIER_A: &str = "10000000-0000-4000-8000-000000000001";
    const CARRIER_B: &str = "10000000-0000-4000-8000-000000000002";

    fn producer() -> ProducerV1 {
        ProducerV1 {
            implementation: "rar-cmo-slot-flow-rust".into(),
            commit_or_build: "unit-test".into(),
            core_integration: true,
        }
    }

    fn slot(
        slot_index: usize,
        scalar_index: u32,
        source_order: usize,
        carrier: &str,
        width: i64,
        height: i64,
    ) -> FlowItemV1 {
        FlowItemV1::ObjectSlot {
            slot_index,
            scalar_index,
            source_order,
            cmo_id: u32::try_from(slot_index + 1).expect("slot"),
            carrier_node_id: carrier.into(),
            carrier_story_id: None,
            intrinsic_width_emu: width,
            intrinsic_height_emu: height,
        }
    }

    fn input(items: Vec<FlowItemV1>, markers: usize) -> CmoSlotFlowInputV1 {
        CmoSlotFlowInputV1 {
            schema_version: INPUT_VERSION_V1.into(),
            producer: producer(),
            source_hash: "a".repeat(64),
            target_story_id: STORY.into(),
            target_frame_node_id: FRAME.into(),
            target_frame_count: 1,
            story_marker_count: markers,
            host: HostV1 {
                width_emu: 100,
                height_emu: 100,
            },
            items,
        }
    }

    #[test]
    fn exact_fit_is_visible_without_scaling() {
        let receipt =
            build_receipt_v1(&input(vec![slot(0, 0, 1, CARRIER_A, 100, 100)], 1)).expect("receipt");
        assert_eq!(receipt.visible_slots.len(), 1);
        assert_eq!(receipt.visible_slots[0].resolved_width_emu, 100);
        assert_eq!(receipt.visible_slots[0].resolved_height_emu, 100);
        assert!(!receipt.overset.story_overset);
        assert!(!receipt.invariants.scaling_applied);
    }

    #[test]
    fn plus_one_width_and_height_fail_closed() {
        let width = build_receipt_v1(&input(vec![slot(0, 0, 1, CARRIER_A, 101, 10)], 1)).unwrap();
        assert!(width.overset.story_overset);
        assert_eq!(width.overset.failure_reason.as_deref(), Some("width"));
        assert!(width.visible_slots.is_empty());

        let height = build_receipt_v1(&input(vec![slot(0, 0, 1, CARRIER_A, 10, 101)], 1)).unwrap();
        assert!(height.overset.story_overset);
        assert_eq!(height.overset.failure_reason.as_deref(), Some("height"));
        assert!(height.visible_slots.is_empty());
    }

    #[test]
    fn shaped_lines_and_slots_share_one_vertical_cursor() {
        let items = vec![
            FlowItemV1::ShapedLine {
                scalar_start: 0,
                scalar_end: 1,
                consumed_scalar_end: 1,
                height_emu: 30,
            },
            slot(0, 2, 3, CARRIER_A, 80, 60),
        ];
        let receipt = build_receipt_v1(&input(items, 1)).unwrap();
        assert_eq!(receipt.visible_slots[0].preceding_text_height_emu, 30);
        assert_eq!(receipt.visible_slots[0].used_height_before_emu, 30);
        assert_eq!(receipt.visible_slots[0].used_height_after_emu, 90);
    }

    #[test]
    fn first_nonfit_hides_later_smaller_slot() {
        let items = vec![
            slot(0, 0, 3, CARRIER_A, 80, 80),
            slot(1, 2, 4, CARRIER_B, 80, 30),
            slot(2, 4, 5, CARRIER_A, 10, 10),
        ];
        let receipt = build_receipt_v1(&input(items, 3)).unwrap();
        assert_eq!(receipt.visible_slots.len(), 1);
        assert_eq!(receipt.overset.first_nonfitting_slot_index, Some(1));
        assert_eq!(receipt.overset.remaining_slot_count, 2);
        assert!(!receipt.invariants.skip_to_fit);
    }

    #[test]
    fn marker_count_and_order_are_strict() {
        let mismatch = input(vec![slot(0, 0, 3, CARRIER_A, 10, 10)], 2);
        assert!(matches!(
            build_receipt_v1(&mismatch),
            Err(CmoSlotFlowError::MarkerCount { .. })
        ));

        let out_of_order = input(
            vec![
                slot(0, 4, 3, CARRIER_A, 10, 10),
                slot(1, 2, 4, CARRIER_B, 10, 10),
            ],
            2,
        );
        assert!(matches!(
            build_receipt_v1(&out_of_order),
            Err(CmoSlotFlowError::SlotScalarOrder { .. })
        ));
    }
}
