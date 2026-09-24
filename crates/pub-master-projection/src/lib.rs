//! Bounded page -> master projection gate for mature Publisher sources.
//!
//! Rar does not mirror the historical Contents reader here. The caller supplies
//! the already-extracted exact OplPd field records and page identities. This
//! crate owns the semantic admission law: field 0x0D must be wire 0x68 and
//! resolve to an existing raw 0x43 PAGE without duplication or self-reference.

use pub_model::{MasterProjectionRelationV1, PubProjectionContextV1};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

pub const MASTER_FIELD_ID: u8 = 0x0d;
pub const REFERENCE_U32_WIRE: u8 = 0x68;
pub const RAW_TYPE_PAGE: u16 = 0x43;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExactReferenceFieldV1 {
    pub field_id: u8,
    pub block_type: u8,
    pub value: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageProjectionSourceV1 {
    pub page_id: String,
    pub seq_num: u32,
    pub raw_type: u16,
    #[serde(default)]
    pub master_fields: Vec<ExactReferenceFieldV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MasterProjectionInputV1 {
    pub pages: Vec<PageProjectionSourceV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MasterProjectionReceiptV1 {
    pub receipt_version: String,
    pub projection_context_version: String,
    pub relation_count: usize,
    pub relations: Vec<MasterProjectionRelationV1>,
    pub invariants: MasterProjectionInvariantsV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MasterProjectionInvariantsV1 {
    pub exact_field_id: u8,
    pub exact_wire: u8,
    pub target_raw_type: u16,
    pub source_graph_mutated: bool,
    pub semantic_node_clones_created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MasterProjectionError {
    InvalidPageId,
    DuplicatePageId(String),
    DuplicateSeqNum(u32),
    NonPageRecord { seq_num: u32, raw_type: u16 },
    DuplicateMasterField { seq_num: u32 },
    WrongField { seq_num: u32, actual: u8 },
    WrongWire { seq_num: u32, actual: u8 },
    UnresolvedMaster { source_seq_num: u32, master_seq_num: u32 },
    TargetNotPage { master_seq_num: u32, raw_type: u16 },
    SelfReference { seq_num: u32 },
    DuplicateSourceRelation(String),
}

impl fmt::Display for MasterProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPageId => write!(f, "page_id must be canonical lowercase UUID"),
            Self::DuplicatePageId(id) => write!(f, "duplicate page id {id}"),
            Self::DuplicateSeqNum(seq) => write!(f, "duplicate PAGE seqNum {seq}"),
            Self::NonPageRecord { seq_num, raw_type } => {
                write!(f, "source seqNum {seq_num} is raw type 0x{raw_type:02x}, not PAGE")
            }
            Self::DuplicateMasterField { seq_num } => {
                write!(f, "PAGE {seq_num} contains duplicate field0x0D")
            }
            Self::WrongField { seq_num, actual } => {
                write!(f, "PAGE {seq_num}: expected field0x0D, got 0x{actual:02x}")
            }
            Self::WrongWire { seq_num, actual } => {
                write!(f, "PAGE {seq_num}: field0x0D must use wire0x68, got 0x{actual:02x}")
            }
            Self::UnresolvedMaster {
                source_seq_num,
                master_seq_num,
            } => write!(
                f,
                "PAGE {source_seq_num}: unresolved master PAGE handle {master_seq_num}"
            ),
            Self::TargetNotPage {
                master_seq_num,
                raw_type,
            } => write!(
                f,
                "master handle {master_seq_num} resolves to raw type 0x{raw_type:02x}, not PAGE"
            ),
            Self::SelfReference { seq_num } => write!(f, "PAGE {seq_num} references itself as master"),
            Self::DuplicateSourceRelation(id) => {
                write!(f, "source page {id} has more than one admitted master relation")
            }
        }
    }
}

impl Error for MasterProjectionError {}

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

pub fn build_master_relations_v1(
    input: &MasterProjectionInputV1,
) -> Result<Vec<MasterProjectionRelationV1>, MasterProjectionError> {
    let mut pages_by_seq = BTreeMap::<u32, &PageProjectionSourceV1>::new();
    let mut page_ids = BTreeSet::new();

    for page in &input.pages {
        if !valid_uuid(&page.page_id) {
            return Err(MasterProjectionError::InvalidPageId);
        }
        if !page_ids.insert(page.page_id.clone()) {
            return Err(MasterProjectionError::DuplicatePageId(page.page_id.clone()));
        }
        if pages_by_seq.insert(page.seq_num, page).is_some() {
            return Err(MasterProjectionError::DuplicateSeqNum(page.seq_num));
        }
    }

    let mut relations = Vec::new();
    let mut relation_sources = BTreeSet::new();

    for page in &input.pages {
        if page.raw_type != RAW_TYPE_PAGE {
            return Err(MasterProjectionError::NonPageRecord {
                seq_num: page.seq_num,
                raw_type: page.raw_type,
            });
        }
        if page.master_fields.len() > 1 {
            return Err(MasterProjectionError::DuplicateMasterField {
                seq_num: page.seq_num,
            });
        }
        let Some(field) = page.master_fields.first() else {
            continue;
        };
        if field.field_id != MASTER_FIELD_ID {
            return Err(MasterProjectionError::WrongField {
                seq_num: page.seq_num,
                actual: field.field_id,
            });
        }
        if field.block_type != REFERENCE_U32_WIRE {
            return Err(MasterProjectionError::WrongWire {
                seq_num: page.seq_num,
                actual: field.block_type,
            });
        }
        if field.value == page.seq_num {
            return Err(MasterProjectionError::SelfReference {
                seq_num: page.seq_num,
            });
        }
        let target = pages_by_seq
            .get(&field.value)
            .copied()
            .ok_or(MasterProjectionError::UnresolvedMaster {
                source_seq_num: page.seq_num,
                master_seq_num: field.value,
            })?;
        if target.raw_type != RAW_TYPE_PAGE {
            return Err(MasterProjectionError::TargetNotPage {
                master_seq_num: target.seq_num,
                raw_type: target.raw_type,
            });
        }
        if !relation_sources.insert(page.page_id.clone()) {
            return Err(MasterProjectionError::DuplicateSourceRelation(
                page.page_id.clone(),
            ));
        }

        relations.push(MasterProjectionRelationV1 {
            source_page_id: page.page_id.clone(),
            source_page_seq_num: page.seq_num,
            master_page_id: target.page_id.clone(),
            master_page_seq_num: target.seq_num,
        });
    }

    relations.sort_by(|left, right| {
        left.source_page_id
            .cmp(&right.source_page_id)
            .then(left.master_page_id.cmp(&right.master_page_id))
    });
    Ok(relations)
}

pub fn build_projection_context_v1(
    input: &MasterProjectionInputV1,
) -> Result<PubProjectionContextV1, MasterProjectionError> {
    Ok(PubProjectionContextV1::with_master_relations(
        build_master_relations_v1(input)?,
    ))
}

pub fn build_receipt_v1(
    input: &MasterProjectionInputV1,
) -> Result<MasterProjectionReceiptV1, MasterProjectionError> {
    let context = build_projection_context_v1(input)?;
    Ok(MasterProjectionReceiptV1 {
        receipt_version: "chaptera.master-page-projection-receipt.v1".to_owned(),
        projection_context_version: context.schema_version.clone(),
        relation_count: context.master_relations.len(),
        relations: context.master_relations,
        invariants: MasterProjectionInvariantsV1 {
            exact_field_id: MASTER_FIELD_ID,
            exact_wire: REFERENCE_U32_WIRE,
            target_raw_type: RAW_TYPE_PAGE,
            source_graph_mutated: false,
            semantic_node_clones_created: false,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(id_tail: u32, seq: u32, master: Option<u32>) -> PageProjectionSourceV1 {
        PageProjectionSourceV1 {
            page_id: format!("10000000-0000-4000-8000-{id_tail:012x}"),
            seq_num: seq,
            raw_type: RAW_TYPE_PAGE,
            master_fields: master
                .map(|value| {
                    vec![ExactReferenceFieldV1 {
                        field_id: MASTER_FIELD_ID,
                        block_type: REFERENCE_U32_WIRE,
                        value,
                    }]
                })
                .unwrap_or_default(),
        }
    }

    #[test]
    fn carlton_shape_three_pages_share_one_master_relation() {
        let input = MasterProjectionInputV1 {
            pages: vec![
                page(263, 263, None),
                page(266, 266, Some(263)),
                page(361, 361, Some(263)),
                page(406, 406, Some(263)),
            ],
        };
        let context = build_projection_context_v1(&input).expect("context");
        assert_eq!(context.master_relations.len(), 3);
        assert!(context.cmo_relations.is_empty());
        assert!(context
            .master_relations
            .iter()
            .all(|relation| relation.master_page_seq_num == 263));
    }

    #[test]
    fn no_master_field_is_not_invented_as_a_relation() {
        let input = MasterProjectionInputV1 {
            pages: vec![page(263, 263, None), page(266, 266, None)],
        };
        assert!(build_master_relations_v1(&input).expect("relations").is_empty());
    }

    #[test]
    fn duplicate_wrong_wire_unresolved_and_self_ref_fail_closed() {
        let mut duplicate = page(266, 266, Some(263));
        duplicate.master_fields.push(ExactReferenceFieldV1 {
            field_id: MASTER_FIELD_ID,
            block_type: REFERENCE_U32_WIRE,
            value: 263,
        });
        let input = MasterProjectionInputV1 {
            pages: vec![page(263, 263, None), duplicate],
        };
        assert!(matches!(
            build_master_relations_v1(&input),
            Err(MasterProjectionError::DuplicateMasterField { .. })
        ));

        let mut wrong_wire = page(266, 266, Some(263));
        wrong_wire.master_fields[0].block_type = 0x20;
        let input = MasterProjectionInputV1 {
            pages: vec![page(263, 263, None), wrong_wire],
        };
        assert!(matches!(
            build_master_relations_v1(&input),
            Err(MasterProjectionError::WrongWire { .. })
        ));

        let input = MasterProjectionInputV1 {
            pages: vec![page(266, 266, Some(999))],
        };
        assert!(matches!(
            build_master_relations_v1(&input),
            Err(MasterProjectionError::UnresolvedMaster { .. })
        ));

        let input = MasterProjectionInputV1 {
            pages: vec![page(266, 266, Some(266))],
        };
        assert!(matches!(
            build_master_relations_v1(&input),
            Err(MasterProjectionError::SelfReference { .. })
        ));
    }

    #[test]
    fn wrong_field_and_non_page_source_fail_closed() {
        let mut wrong = page(266, 266, Some(263));
        wrong.master_fields[0].field_id = 0x0c;
        let input = MasterProjectionInputV1 {
            pages: vec![page(263, 263, None), wrong],
        };
        assert!(matches!(
            build_master_relations_v1(&input),
            Err(MasterProjectionError::WrongField { .. })
        ));

        let mut non_page = page(263, 263, None);
        non_page.raw_type = 0x70;
        let input = MasterProjectionInputV1 {
            pages: vec![non_page],
        };
        assert!(matches!(
            build_master_relations_v1(&input),
            Err(MasterProjectionError::NonPageRecord { .. })
        ));
    }
}
