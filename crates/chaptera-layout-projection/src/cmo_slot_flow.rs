use pub_model::{
    CmoProjectionRelationV1, PUB_PROJECTION_CONTEXT_SCHEMA_V1, PubProjectionContextV1,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const CMO_SLOT_FLOW_SCHEMA_V1: &str = "chaptera.cmo-slot-flow.native.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedTextLineV1 {
    pub scalar_start: u32,
    pub scalar_end: u32,
    pub consumed_scalar_end: u32,
    pub height_emu: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarrierExtentV1 {
    pub carrier_node_id: String,
    pub width_emu: i64,
    pub height_emu: i64,
    #[serde(default)]
    pub nested_cmo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CmoStorySlotFlowInputV1 {
    pub target_qsid: u32,
    pub target_story_id: String,
    pub target_frame_node_id: String,
    pub frame_count: u32,
    pub host_width_emu: i64,
    pub host_height_emu: i64,
    /// Story-global scalar positions of U+FFFC in canonical order.
    pub object_marker_scalars: Vec<u32>,
    /// Already-resolved text-line heights. These lines must not cover U+FFFC.
    #[serde(default)]
    pub text_lines: Vec<ResolvedTextLineV1>,
    pub carrier_extents: Vec<CarrierExtentV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleCmoSlotV1 {
    pub instance_id: String,
    pub slot_index: usize,
    pub scalar_index: u32,
    pub source_order: usize,
    pub cmo_id: u32,
    pub carrier_node_id: String,
    pub carrier_story_id: Option<String>,
    pub target_story_id: String,
    pub target_frame_node_id: String,
    pub preceding_text_height_emu: i64,
    pub used_height_before_emu: i64,
    pub used_height_after_emu: i64,
    pub resolved_x_emu: i64,
    pub resolved_y_emu: i64,
    pub resolved_width_emu: i64,
    pub resolved_height_emu: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CmoNonFitReasonV1 {
    Width,
    Height,
    WidthAndHeight,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CmoSlotOversetV1 {
    pub story_overset: bool,
    pub first_nonfitting_kind: Option<String>,
    pub first_nonfitting_slot_index: Option<usize>,
    pub first_nonfitting_scalar_index: Option<u32>,
    pub failure_reason: Option<CmoNonFitReasonV1>,
    pub remaining_item_count: usize,
    pub remaining_slot_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CmoSlotFlowOutputV1 {
    pub schema_version: String,
    pub target_qsid: u32,
    pub target_story_id: String,
    pub target_frame_node_id: String,
    pub host_width_emu: i64,
    pub host_height_emu: i64,
    pub visible_slots: Vec<VisibleCmoSlotV1>,
    pub overset: CmoSlotOversetV1,
    pub carrier_reparent_count: u32,
    pub scaling_applied: bool,
    pub skip_to_fit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CmoSlotFlowError {
    ProjectionContextVersion,
    TargetStoryMissing,
    TargetFrameMissing,
    MultiFrameUnsupported,
    NonPositiveHostExtent,
    MarkerCardinalityMismatch { markers: usize, relations: usize },
    DuplicateMarkerScalar { scalar: u32 },
    RelationTargetMismatch { source_order: usize },
    RelationFrameUnresolved { source_order: usize },
    RelationFrameMismatch { source_order: usize },
    RelationOrderNotStrict,
    DuplicateCarrierMetric { carrier_node_id: String },
    MissingCarrierMetric { carrier_node_id: String },
    NonPositiveCarrierExtent { carrier_node_id: String },
    NestedCmoUnsupported { carrier_node_id: String },
    InvalidTextLine { index: usize },
    TextLineCoversObjectMarker { index: usize, scalar: u32 },
    FlowItemOrderAmbiguous { scalar: u32 },
    ArithmeticOverflow,
}

impl std::fmt::Display for CmoSlotFlowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProjectionContextVersion => f.write_str("projection context version mismatch"),
            Self::TargetStoryMissing => f.write_str("target_story_id is required"),
            Self::TargetFrameMissing => f.write_str("target_frame_node_id is required"),
            Self::MultiFrameUnsupported => {
                f.write_str("bounded Cmo slot-flow V1 admits exactly one target frame")
            }
            Self::NonPositiveHostExtent => f.write_str("host extent must be positive"),
            Self::MarkerCardinalityMismatch { markers, relations } => write!(
                f,
                "U+FFFC marker count {markers} differs from Cmo relation count {relations}"
            ),
            Self::DuplicateMarkerScalar { scalar } => {
                write!(f, "duplicate U+FFFC marker scalar {scalar}")
            }
            Self::RelationTargetMismatch { source_order } => write!(
                f,
                "Cmo relation source_order {source_order} targets a different Story/Qsid"
            ),
            Self::RelationFrameUnresolved { source_order } => write!(
                f,
                "Cmo relation source_order {source_order} has no unique target frame"
            ),
            Self::RelationFrameMismatch { source_order } => write!(
                f,
                "Cmo relation source_order {source_order} targets a different frame"
            ),
            Self::RelationOrderNotStrict => {
                f.write_str("Cmo relation source_order must be strictly increasing")
            }
            Self::DuplicateCarrierMetric { carrier_node_id } => {
                write!(f, "duplicate carrier extent for {carrier_node_id}")
            }
            Self::MissingCarrierMetric { carrier_node_id } => {
                write!(f, "missing carrier extent for {carrier_node_id}")
            }
            Self::NonPositiveCarrierExtent { carrier_node_id } => {
                write!(f, "carrier {carrier_node_id} extent must be positive")
            }
            Self::NestedCmoUnsupported { carrier_node_id } => {
                write!(
                    f,
                    "carrier {carrier_node_id} requires nested Cmo projection"
                )
            }
            Self::InvalidTextLine { index } => write!(f, "text line {index} is invalid"),
            Self::TextLineCoversObjectMarker { index, scalar } => write!(
                f,
                "text line {index} covers semantic U+FFFC marker at scalar {scalar}"
            ),
            Self::FlowItemOrderAmbiguous { scalar } => {
                write!(f, "multiple bounded flow items start at scalar {scalar}")
            }
            Self::ArithmeticOverflow => f.write_str("bounded Cmo slot-flow arithmetic overflow"),
        }
    }
}

impl std::error::Error for CmoSlotFlowError {}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FlowItemV1 {
    Text {
        scalar_start: u32,
        scalar_end: u32,
        consumed_scalar_end: u32,
        height_emu: i64,
    },
    Slot {
        slot_index: usize,
        scalar_index: u32,
        relation: CmoProjectionRelationV1,
        extent: CarrierExtentV1,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FirstNonfitV1 {
    item_index: usize,
    kind: String,
    slot_index: Option<usize>,
    scalar_index: u32,
    reason: Option<CmoNonFitReasonV1>,
}

impl FlowItemV1 {
    fn scalar_key(&self) -> u32 {
        match self {
            Self::Text { scalar_start, .. } => *scalar_start,
            Self::Slot { scalar_index, .. } => *scalar_index,
        }
    }

    fn is_slot(&self) -> bool {
        matches!(self, Self::Slot { .. })
    }
}

pub fn resolve_cmo_slot_flow_v1(
    context: &PubProjectionContextV1,
    input: &CmoStorySlotFlowInputV1,
) -> Result<CmoSlotFlowOutputV1, CmoSlotFlowError> {
    if context.schema_version != PUB_PROJECTION_CONTEXT_SCHEMA_V1 {
        return Err(CmoSlotFlowError::ProjectionContextVersion);
    }
    if input.target_story_id.is_empty() {
        return Err(CmoSlotFlowError::TargetStoryMissing);
    }
    if input.target_frame_node_id.is_empty() {
        return Err(CmoSlotFlowError::TargetFrameMissing);
    }
    if input.frame_count != 1 {
        return Err(CmoSlotFlowError::MultiFrameUnsupported);
    }
    if input.host_width_emu <= 0 || input.host_height_emu <= 0 {
        return Err(CmoSlotFlowError::NonPositiveHostExtent);
    }

    let mut marker_seen = BTreeSet::new();
    for &scalar in &input.object_marker_scalars {
        if !marker_seen.insert(scalar) {
            return Err(CmoSlotFlowError::DuplicateMarkerScalar { scalar });
        }
    }
    if input
        .object_marker_scalars
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    {
        return Err(CmoSlotFlowError::RelationOrderNotStrict);
    }

    let mut relations = context
        .cmo_relations_for_target_qsid(input.target_qsid)
        .cloned()
        .collect::<Vec<_>>();
    relations.sort_by_key(|relation| relation.source_order);

    if relations.len() != input.object_marker_scalars.len() {
        return Err(CmoSlotFlowError::MarkerCardinalityMismatch {
            markers: input.object_marker_scalars.len(),
            relations: relations.len(),
        });
    }
    if relations
        .windows(2)
        .any(|pair| pair[0].source_order >= pair[1].source_order)
    {
        return Err(CmoSlotFlowError::RelationOrderNotStrict);
    }

    for relation in &relations {
        if relation.target_qsid != input.target_qsid
            || relation.target_story_id != input.target_story_id
        {
            return Err(CmoSlotFlowError::RelationTargetMismatch {
                source_order: relation.source_order,
            });
        }
        let Some(frame) = relation.target_frame_node_id.as_deref() else {
            return Err(CmoSlotFlowError::RelationFrameUnresolved {
                source_order: relation.source_order,
            });
        };
        if frame != input.target_frame_node_id {
            return Err(CmoSlotFlowError::RelationFrameMismatch {
                source_order: relation.source_order,
            });
        }
    }

    let mut extents = BTreeMap::<String, CarrierExtentV1>::new();
    for extent in &input.carrier_extents {
        if extent.width_emu <= 0 || extent.height_emu <= 0 {
            return Err(CmoSlotFlowError::NonPositiveCarrierExtent {
                carrier_node_id: extent.carrier_node_id.clone(),
            });
        }
        if extent.nested_cmo {
            return Err(CmoSlotFlowError::NestedCmoUnsupported {
                carrier_node_id: extent.carrier_node_id.clone(),
            });
        }
        if extents
            .insert(extent.carrier_node_id.clone(), extent.clone())
            .is_some()
        {
            return Err(CmoSlotFlowError::DuplicateCarrierMetric {
                carrier_node_id: extent.carrier_node_id.clone(),
            });
        }
    }

    let marker_set = input
        .object_marker_scalars
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut items = Vec::<FlowItemV1>::new();
    for (index, line) in input.text_lines.iter().enumerate() {
        if line.height_emu <= 0
            || line.scalar_end < line.scalar_start
            || line.consumed_scalar_end < line.scalar_end
        {
            return Err(CmoSlotFlowError::InvalidTextLine { index });
        }
        if let Some(&scalar) = marker_set.range(line.scalar_start..line.scalar_end).next() {
            return Err(CmoSlotFlowError::TextLineCoversObjectMarker { index, scalar });
        }
        items.push(FlowItemV1::Text {
            scalar_start: line.scalar_start,
            scalar_end: line.scalar_end,
            consumed_scalar_end: line.consumed_scalar_end,
            height_emu: line.height_emu,
        });
    }

    for (slot_index, (relation, &scalar_index)) in relations
        .into_iter()
        .zip(input.object_marker_scalars.iter())
        .enumerate()
    {
        let extent = extents
            .get(&relation.carrier_node_id)
            .cloned()
            .ok_or_else(|| CmoSlotFlowError::MissingCarrierMetric {
                carrier_node_id: relation.carrier_node_id.clone(),
            })?;
        items.push(FlowItemV1::Slot {
            slot_index,
            scalar_index,
            relation,
            extent,
        });
    }

    items.sort_by(|left, right| {
        left.scalar_key()
            .cmp(&right.scalar_key())
            .then_with(|| left.is_slot().cmp(&right.is_slot()))
    });
    if items
        .windows(2)
        .any(|pair| pair[0].scalar_key() == pair[1].scalar_key())
    {
        return Err(CmoSlotFlowError::FlowItemOrderAmbiguous {
            scalar: items
                .windows(2)
                .find(|pair| pair[0].scalar_key() == pair[1].scalar_key())
                .expect("checked pair")[0]
                .scalar_key(),
        });
    }

    let mut used_height_emu = 0_i64;
    let mut pending_text_height_emu = 0_i64;
    let mut visible_slots = Vec::new();
    let mut first_nonfit: Option<FirstNonfitV1> = None;

    for (item_index, item) in items.iter().enumerate() {
        match item {
            FlowItemV1::Text {
                scalar_start,
                scalar_end,
                consumed_scalar_end,
                height_emu,
            } => {
                let _ = (scalar_end, consumed_scalar_end);
                let next = used_height_emu
                    .checked_add(*height_emu)
                    .ok_or(CmoSlotFlowError::ArithmeticOverflow)?;
                if next > input.host_height_emu {
                    first_nonfit = Some(FirstNonfitV1 {
                        item_index,
                        kind: "shaped_line".to_owned(),
                        slot_index: None,
                        scalar_index: *scalar_start,
                        reason: Some(CmoNonFitReasonV1::Height),
                    });
                    break;
                }
                used_height_emu = next;
                pending_text_height_emu = pending_text_height_emu
                    .checked_add(*height_emu)
                    .ok_or(CmoSlotFlowError::ArithmeticOverflow)?;
            }
            FlowItemV1::Slot {
                slot_index,
                scalar_index,
                relation,
                extent,
            } => {
                let width_fits = extent.width_emu <= input.host_width_emu;
                let next = used_height_emu
                    .checked_add(extent.height_emu)
                    .ok_or(CmoSlotFlowError::ArithmeticOverflow)?;
                let height_fits = next <= input.host_height_emu;
                if !width_fits || !height_fits {
                    let reason = match (width_fits, height_fits) {
                        (false, false) => CmoNonFitReasonV1::WidthAndHeight,
                        (false, true) => CmoNonFitReasonV1::Width,
                        (true, false) => CmoNonFitReasonV1::Height,
                        (true, true) => unreachable!("nonfit branch"),
                    };
                    first_nonfit = Some(FirstNonfitV1 {
                        item_index,
                        kind: "object_slot".to_owned(),
                        slot_index: Some(*slot_index),
                        scalar_index: *scalar_index,
                        reason: Some(reason),
                    });
                    break;
                }

                visible_slots.push(VisibleCmoSlotV1 {
                    instance_id: slot_instance_id_v1(
                        &input.target_story_id,
                        &input.target_frame_node_id,
                        *scalar_index,
                        relation,
                    ),
                    slot_index: *slot_index,
                    scalar_index: *scalar_index,
                    source_order: relation.source_order,
                    cmo_id: relation.cmo_id,
                    carrier_node_id: relation.carrier_node_id.clone(),
                    carrier_story_id: relation.carrier_story_id.clone(),
                    target_story_id: input.target_story_id.clone(),
                    target_frame_node_id: input.target_frame_node_id.clone(),
                    preceding_text_height_emu: pending_text_height_emu,
                    used_height_before_emu: used_height_emu,
                    used_height_after_emu: next,
                    resolved_x_emu: 0,
                    resolved_y_emu: used_height_emu,
                    resolved_width_emu: extent.width_emu,
                    resolved_height_emu: extent.height_emu,
                });
                used_height_emu = next;
                pending_text_height_emu = 0;
            }
        }
    }

    let (overset, remaining_item_count, remaining_slot_count) = if let Some(first) = first_nonfit {
        let tail = &items[first.item_index..];
        (
            CmoSlotOversetV1 {
                story_overset: true,
                first_nonfitting_kind: Some(first.kind),
                first_nonfitting_slot_index: first.slot_index,
                first_nonfitting_scalar_index: Some(first.scalar_index),
                failure_reason: first.reason,
                remaining_item_count: tail.len(),
                remaining_slot_count: tail.iter().filter(|item| item.is_slot()).count(),
            },
            tail.len(),
            tail.iter().filter(|item| item.is_slot()).count(),
        )
    } else {
        (
            CmoSlotOversetV1 {
                story_overset: false,
                first_nonfitting_kind: None,
                first_nonfitting_slot_index: None,
                first_nonfitting_scalar_index: None,
                failure_reason: None,
                remaining_item_count: 0,
                remaining_slot_count: 0,
            },
            0,
            0,
        )
    };
    debug_assert_eq!(overset.remaining_item_count, remaining_item_count);
    debug_assert_eq!(overset.remaining_slot_count, remaining_slot_count);

    Ok(CmoSlotFlowOutputV1 {
        schema_version: CMO_SLOT_FLOW_SCHEMA_V1.to_owned(),
        target_qsid: input.target_qsid,
        target_story_id: input.target_story_id.clone(),
        target_frame_node_id: input.target_frame_node_id.clone(),
        host_width_emu: input.host_width_emu,
        host_height_emu: input.host_height_emu,
        visible_slots,
        overset,
        carrier_reparent_count: 0,
        scaling_applied: false,
        skip_to_fit: false,
    })
}

#[derive(Serialize)]
struct SlotInstanceIdentityV1<'a> {
    // Field order is deliberately alphabetical to match the already-admitted
    // Python public contract's canonical JSON (sort_keys=True).
    carrier_node_id: &'a str,
    projection_kind: &'static str,
    scalar_index: u32,
    source_order: usize,
    target_frame_node_id: &'a str,
    target_story_id: &'a str,
}

fn slot_instance_id_v1(
    target_story_id: &str,
    target_frame_node_id: &str,
    scalar_index: u32,
    relation: &CmoProjectionRelationV1,
) -> String {
    let identity = SlotInstanceIdentityV1 {
        carrier_node_id: &relation.carrier_node_id,
        projection_kind: "cmo_story_slot",
        scalar_index,
        source_order: relation.source_order,
        target_frame_node_id,
        target_story_id,
    };
    let canonical = serde_json::to_vec(&identity)
        .expect("fixed Cmo slot identity fields must serialize to canonical JSON");
    format!("sha256:{:x}", Sha256::digest(canonical))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STORY: &str = "30000000-0000-4000-8000-000000000001";
    const FRAME: &str = "40000000-0000-4000-8000-000000000001";

    fn relation(order: usize, cmo_id: u32, node: &str) -> CmoProjectionRelationV1 {
        CmoProjectionRelationV1 {
            source_order: order,
            cmo_id,
            carrier_ohpo: 400 + cmo_id,
            carrier_cmo_id: cmo_id,
            target_qsid: 49,
            carrier_node_id: node.to_owned(),
            carrier_story_id: Some(format!("50000000-0000-4000-8000-{cmo_id:012x}")),
            target_story_id: STORY.to_owned(),
            target_frame_node_id: Some(FRAME.to_owned()),
        }
    }

    fn context(relations: Vec<CmoProjectionRelationV1>) -> PubProjectionContextV1 {
        PubProjectionContextV1::with_cmo_relations(relations)
    }

    fn extent(node: &str, width: i64, height: i64) -> CarrierExtentV1 {
        CarrierExtentV1 {
            carrier_node_id: node.to_owned(),
            width_emu: width,
            height_emu: height,
            nested_cmo: false,
        }
    }

    fn input(markers: Vec<u32>, extents: Vec<CarrierExtentV1>) -> CmoStorySlotFlowInputV1 {
        CmoStorySlotFlowInputV1 {
            target_qsid: 49,
            target_story_id: STORY.to_owned(),
            target_frame_node_id: FRAME.to_owned(),
            frame_count: 1,
            host_width_emu: 100,
            host_height_emu: 100,
            object_marker_scalars: markers,
            text_lines: Vec::new(),
            carrier_extents: extents,
        }
    }

    #[test]
    fn exact_fit_is_visible_but_plus_one_height_is_overset() {
        let node = "10000000-0000-4000-8000-000000000001";
        let ctx = context(vec![relation(3, 7, node)]);
        let exact = resolve_cmo_slot_flow_v1(&ctx, &input(vec![0], vec![extent(node, 100, 100)]))
            .expect("exact fit");
        assert_eq!(exact.visible_slots.len(), 1);
        assert!(!exact.overset.story_overset);

        let too_tall =
            resolve_cmo_slot_flow_v1(&ctx, &input(vec![0], vec![extent(node, 100, 101)]))
                .expect("bounded overset");
        assert!(too_tall.visible_slots.is_empty());
        assert_eq!(
            too_tall.overset.failure_reason,
            Some(CmoNonFitReasonV1::Height)
        );
    }

    #[test]
    fn shaped_line_and_slot_share_one_vertical_cursor() {
        let node = "10000000-0000-4000-8000-000000000001";
        let ctx = context(vec![relation(3, 7, node)]);
        let mut value = input(vec![2], vec![extent(node, 100, 61)]);
        value.text_lines.push(ResolvedTextLineV1 {
            scalar_start: 0,
            scalar_end: 2,
            consumed_scalar_end: 2,
            height_emu: 40,
        });
        let output = resolve_cmo_slot_flow_v1(&ctx, &value).expect("flow");
        assert!(output.visible_slots.is_empty());
        assert_eq!(
            output.overset.failure_reason,
            Some(CmoNonFitReasonV1::Height)
        );
    }

    #[test]
    fn failing_middle_slot_hides_later_smaller_slot() {
        let a = "10000000-0000-4000-8000-000000000001";
        let b = "10000000-0000-4000-8000-000000000002";
        let c = "10000000-0000-4000-8000-000000000003";
        let ctx = context(vec![
            relation(3, 7, a),
            relation(4, 9, b),
            relation(5, 12, c),
        ]);
        let value = input(
            vec![0, 3, 5],
            vec![extent(a, 80, 60), extent(b, 101, 10), extent(c, 10, 10)],
        );
        let output = resolve_cmo_slot_flow_v1(&ctx, &value).expect("flow");
        assert_eq!(
            output
                .visible_slots
                .iter()
                .map(|slot| slot.cmo_id)
                .collect::<Vec<_>>(),
            vec![7]
        );
        assert_eq!(output.overset.first_nonfitting_slot_index, Some(1));
        assert_eq!(output.overset.remaining_slot_count, 2);
        assert!(!output.skip_to_fit);
    }

    #[test]
    fn marker_cardinality_is_not_guessed() {
        let node = "10000000-0000-4000-8000-000000000001";
        let ctx = context(vec![relation(3, 7, node)]);
        let error = resolve_cmo_slot_flow_v1(&ctx, &input(vec![0, 3], vec![extent(node, 50, 50)]))
            .expect_err("must reject mismatch");
        assert_eq!(
            error,
            CmoSlotFlowError::MarkerCardinalityMismatch {
                markers: 2,
                relations: 1
            }
        );
    }

    #[test]
    fn text_line_cannot_shape_through_object_marker() {
        let node = "10000000-0000-4000-8000-000000000001";
        let ctx = context(vec![relation(3, 7, node)]);
        let mut value = input(vec![1], vec![extent(node, 50, 50)]);
        value.text_lines.push(ResolvedTextLineV1 {
            scalar_start: 0,
            scalar_end: 2,
            consumed_scalar_end: 2,
            height_emu: 20,
        });
        assert_eq!(
            resolve_cmo_slot_flow_v1(&ctx, &value),
            Err(CmoSlotFlowError::TextLineCoversObjectMarker {
                index: 0,
                scalar: 1
            })
        );
    }

    #[test]
    fn slot_instance_id_matches_admitted_public_contract_law() {
        let relation = CmoProjectionRelationV1 {
            source_order: 3,
            cmo_id: 7,
            carrier_ohpo: 441,
            carrier_cmo_id: 7,
            target_qsid: 49,
            carrier_node_id: "b98be77a-5dc8-5134-9998-0d2738c1b077".to_owned(),
            carrier_story_id: Some("86f4e1e1-2d6e-54c4-b732-4ff49dde3737".to_owned()),
            target_story_id: "9ec067e2-2962-5923-85b9-7470503a2181".to_owned(),
            target_frame_node_id: Some("4e554b11-a364-5e15-ba52-9d1ecda18b5f".to_owned()),
        };
        assert_eq!(
            slot_instance_id_v1(
                "9ec067e2-2962-5923-85b9-7470503a2181",
                "4e554b11-a364-5e15-ba52-9d1ecda18b5f",
                0,
                &relation,
            ),
            "sha256:c2b4d34997a6f85ded10439f3ce7ff0fd1a71e8b2f829004185ee23d55911f47"
        );
    }

    fn exact_march_context() -> PubProjectionContextV1 {
        serde_json::from_str(include_str!(
            "../../../packages/protocol/pub-projection/v1/cmo-slot-flow-carlton-march-projection-context.json"
        ))
        .expect("exact retained March PubProjectionContextV1")
    }

    #[test]
    fn exact_carlton_single_slot_witnesses_fit_with_native_relations() {
        let context = exact_march_context();
        let cases = [
            (
                218,
                "3e10fd6c-52ab-518e-bc83-6cdc0e288988",
                "87cee6b6-c769-5649-b929-107fe110ff89",
                "ac83b8f2-951d-5c76-a407-f3a17fa25698",
                4_411_989,
                2_170_073,
                1_947_077,
                1_710_156,
                1,
            ),
            (
                216,
                "8b9de215-423f-54a7-93b8-89e5ae97e2ef",
                "2528df06-d811-5bd2-9ec9-77f3fb5ce9ce",
                "60d2817b-ef22-5e6f-af74-4b99b0bd90fd",
                2_518_890,
                1_099_091,
                2_473_777,
                987_695,
                5,
            ),
            (
                120,
                "060e02ec-c5e3-521d-a86a-55bbfdf16e86",
                "67d5c123-7ab5-51ed-bcf2-9e9ef6902722",
                "f11f251e-8fae-5f04-877e-d9efbf7ebb17",
                3_199_116,
                1_430_873,
                3_041_690,
                1_281_563,
                6,
            ),
        ];

        for (
            target_qsid,
            target_story_id,
            target_frame_node_id,
            carrier_node_id,
            host_width_emu,
            host_height_emu,
            carrier_width_emu,
            carrier_height_emu,
            cmo_id,
        ) in cases
        {
            let output = resolve_cmo_slot_flow_v1(
                &context,
                &CmoStorySlotFlowInputV1 {
                    target_qsid,
                    target_story_id: target_story_id.to_owned(),
                    target_frame_node_id: target_frame_node_id.to_owned(),
                    frame_count: 1,
                    host_width_emu,
                    host_height_emu,
                    object_marker_scalars: vec![0],
                    text_lines: Vec::new(),
                    carrier_extents: vec![extent(
                        carrier_node_id,
                        carrier_width_emu,
                        carrier_height_emu,
                    )],
                },
            )
            .expect("exact single-slot Carlton witness");

            assert_eq!(output.visible_slots.len(), 1);
            assert_eq!(output.visible_slots[0].cmo_id, cmo_id);
            assert_eq!(output.visible_slots[0].carrier_node_id, carrier_node_id);
            assert!(!output.overset.story_overset);
        }
    }

    #[test]
    fn carlton_q49_lower_bound_uses_exact_retained_context_and_witness_ids() {
        let context = exact_march_context();
        assert_eq!(context.cmo_relations.len(), 9);

        let carriers = [
            ("b98be77a-5dc8-5134-9998-0d2738c1b077", 2_002_380, 1_469_908),
            ("a6672853-7405-5712-8fc0-b08e5c0481c3", 7_626_802, 331_221),
            ("68dc2777-89fd-532d-995b-f0b78b3e5424", 7_626_802, 331_221),
            ("85110268-e6b9-5512-aed8-8b8f654024e5", 7_626_802, 331_221),
            ("e31c5b86-ecd2-51e5-8cc0-ef34a86281c2", 7_626_802, 331_221),
            ("84606b44-693d-5c23-bff1-84239779d9ab", 7_626_802, 331_221),
        ];

        let value = CmoStorySlotFlowInputV1 {
            target_qsid: 49,
            target_story_id: "9ec067e2-2962-5923-85b9-7470503a2181".to_owned(),
            target_frame_node_id: "4e554b11-a364-5e15-ba52-9d1ecda18b5f".to_owned(),
            frame_count: 1,
            host_width_emu: 2_145_323,
            host_height_emu: 1_793_030,
            object_marker_scalars: vec![0, 3, 5, 7, 9, 11],
            text_lines: Vec::new(),
            carrier_extents: carriers
                .into_iter()
                .map(|(node, width, height)| extent(node, width, height))
                .collect(),
        };

        let output = resolve_cmo_slot_flow_v1(&context, &value).expect("Carlton lower bound");
        assert_eq!(
            output
                .visible_slots
                .iter()
                .map(|slot| slot.cmo_id)
                .collect::<Vec<_>>(),
            vec![7]
        );
        assert_eq!(
            output.visible_slots[0].carrier_node_id,
            "b98be77a-5dc8-5134-9998-0d2738c1b077"
        );
        assert_eq!(
            output.visible_slots[0].carrier_story_id.as_deref(),
            Some("86f4e1e1-2d6e-54c4-b732-4ff49dde3737")
        );
        assert_eq!(
            output.visible_slots[0].instance_id,
            "sha256:c2b4d34997a6f85ded10439f3ce7ff0fd1a71e8b2f829004185ee23d55911f47"
        );
        assert_eq!(output.visible_slots[0].used_height_after_emu, 1_469_908);
        assert_eq!(value.host_height_emu - 1_469_908, 323_122);
        assert_eq!(
            output.overset.failure_reason,
            Some(CmoNonFitReasonV1::WidthAndHeight)
        );
        assert_eq!(output.overset.first_nonfitting_slot_index, Some(1));
        assert_eq!(output.overset.first_nonfitting_scalar_index, Some(3));
        assert_eq!(output.overset.remaining_slot_count, 5);
    }
}
