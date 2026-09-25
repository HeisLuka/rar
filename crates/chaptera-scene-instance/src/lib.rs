//! Typed visual-instance identity and mutation admission for Chaptera desktop V0.
//!
//! A rendered visual instance is not automatically an authored NodeId. This
//! crate keeps hit/selection identity separate from semantic origin identity and
//! provides the single fail-closed gate that may translate a visual instance
//! back to an origin NodeId for page-owned object mutation.

use pub_model::{CmoProjectionRelationV1, MasterProjectionRelationV1};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const SCENE_INSTANCE_SCHEMA_V1: &str = "chaptera.scene-instance.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneProjectionKindV1 {
    DirectPageLocal,
    InheritedMaster,
    CmoStorySlot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectMutationKindV1 {
    MoveNode,
    ReplaceImage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometrySyncPolicyV1 {
    ApplyAuthoredOriginGeometry,
    ReprojectFromContext,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneInstanceV1 {
    pub schema_version: String,
    pub instance_id: String,
    pub projection_kind: SceneProjectionKindV1,
    pub origin_node_id: String,
    pub target_page_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_parent_origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub story_authority_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmo_slot_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmo_scalar_index: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectMutationAdmissionV1 {
    pub admitted: bool,
    pub mutation: ObjectMutationKindV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_node_id: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneInstanceError {
    InvalidIdentity { field: &'static str },
    MasterTargetMismatch,
    CmoTargetMismatch,
    CmoFrameUnresolved,
}

impl std::fmt::Display for SceneInstanceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidIdentity { field } => {
                write!(formatter, "{field} must be canonical lowercase UUID")
            }
            Self::MasterTargetMismatch => {
                formatter.write_str("master relation does not target requested customer page")
            }
            Self::CmoTargetMismatch => {
                formatter.write_str("Cmo relation does not target requested Story")
            }
            Self::CmoFrameUnresolved => {
                formatter.write_str("Cmo relation target frame is unresolved")
            }
        }
    }
}

impl std::error::Error for SceneInstanceError {}

fn canonical_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

fn require_uuid(value: &str, field: &'static str) -> Result<(), SceneInstanceError> {
    if canonical_uuid(value) {
        Ok(())
    } else {
        Err(SceneInstanceError::InvalidIdentity { field })
    }
}

fn canonical_instance_id(parts: &[(&str, String)]) -> String {
    let values = parts
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    let bytes = serde_json::to_vec(&values).expect("BTreeMap<String,String> JSON is infallible");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub fn direct_page_local_instance_v1(
    origin_node_id: &str,
    target_page_id: &str,
) -> Result<SceneInstanceV1, SceneInstanceError> {
    require_uuid(origin_node_id, "origin_node_id")?;
    require_uuid(target_page_id, "target_page_id")?;
    let instance_id = canonical_instance_id(&[
        ("origin", origin_node_id.to_owned()),
        ("projection_kind", "direct_page_local".to_owned()),
        ("target_page", target_page_id.to_owned()),
    ]);
    Ok(SceneInstanceV1 {
        schema_version: SCENE_INSTANCE_SCHEMA_V1.to_owned(),
        instance_id,
        projection_kind: SceneProjectionKindV1::DirectPageLocal,
        origin_node_id: origin_node_id.to_owned(),
        target_page_id: target_page_id.to_owned(),
        source_parent_origin: Some(target_page_id.to_owned()),
        story_authority_id: None,
        cmo_slot_index: None,
        cmo_scalar_index: None,
    })
}

pub fn inherited_master_instance_v1(
    origin_node_id: &str,
    source_master_page_id: &str,
    target_page_id: &str,
    relation: &MasterProjectionRelationV1,
) -> Result<SceneInstanceV1, SceneInstanceError> {
    require_uuid(origin_node_id, "origin_node_id")?;
    require_uuid(source_master_page_id, "source_master_page_id")?;
    require_uuid(target_page_id, "target_page_id")?;
    if relation.master_page_id != source_master_page_id || relation.source_page_id != target_page_id
    {
        return Err(SceneInstanceError::MasterTargetMismatch);
    }
    let instance_id = canonical_instance_id(&[
        ("origin", origin_node_id.to_owned()),
        ("projection_kind", "inherited_master".to_owned()),
        ("target_page", target_page_id.to_owned()),
    ]);
    Ok(SceneInstanceV1 {
        schema_version: SCENE_INSTANCE_SCHEMA_V1.to_owned(),
        instance_id,
        projection_kind: SceneProjectionKindV1::InheritedMaster,
        origin_node_id: origin_node_id.to_owned(),
        target_page_id: target_page_id.to_owned(),
        source_parent_origin: Some(source_master_page_id.to_owned()),
        story_authority_id: None,
        cmo_slot_index: None,
        cmo_scalar_index: None,
    })
}

pub fn cmo_story_slot_instance_v1(
    relation: &CmoProjectionRelationV1,
    target_page_id: &str,
    slot_index: usize,
    scalar_index: u32,
) -> Result<SceneInstanceV1, SceneInstanceError> {
    require_uuid(&relation.carrier_node_id, "carrier_node_id")?;
    require_uuid(target_page_id, "target_page_id")?;
    require_uuid(&relation.target_story_id, "target_story_id")?;
    let target_frame = relation
        .target_frame_node_id
        .as_deref()
        .ok_or(SceneInstanceError::CmoFrameUnresolved)?;
    require_uuid(target_frame, "target_frame_node_id")?;
    if relation.target_story_id.is_empty() {
        return Err(SceneInstanceError::CmoTargetMismatch);
    }
    if let Some(story_id) = relation.carrier_story_id.as_deref() {
        require_uuid(story_id, "carrier_story_id")?;
    }
    let instance_id = canonical_instance_id(&[
        ("origin", relation.carrier_node_id.clone()),
        ("projection_kind", "cmo_story_slot".to_owned()),
        ("scalar_index", scalar_index.to_string()),
        ("slot_index", slot_index.to_string()),
        ("target_frame", target_frame.to_owned()),
        ("target_page", target_page_id.to_owned()),
        ("target_story", relation.target_story_id.clone()),
    ]);
    Ok(SceneInstanceV1 {
        schema_version: SCENE_INSTANCE_SCHEMA_V1.to_owned(),
        instance_id,
        projection_kind: SceneProjectionKindV1::CmoStorySlot,
        origin_node_id: relation.carrier_node_id.clone(),
        target_page_id: target_page_id.to_owned(),
        source_parent_origin: None,
        story_authority_id: relation.carrier_story_id.clone(),
        cmo_slot_index: Some(slot_index),
        cmo_scalar_index: Some(scalar_index),
    })
}

pub fn admit_object_mutation_v1(
    instance: &SceneInstanceV1,
    mutation: ObjectMutationKindV1,
) -> ObjectMutationAdmissionV1 {
    match instance.projection_kind {
        SceneProjectionKindV1::DirectPageLocal => ObjectMutationAdmissionV1 {
            admitted: true,
            mutation,
            origin_node_id: Some(instance.origin_node_id.clone()),
            reason: "direct_page_local".to_owned(),
        },
        SceneProjectionKindV1::InheritedMaster => ObjectMutationAdmissionV1 {
            admitted: false,
            mutation,
            origin_node_id: None,
            reason: "inherited_master_read_only".to_owned(),
        },
        SceneProjectionKindV1::CmoStorySlot => ObjectMutationAdmissionV1 {
            admitted: false,
            mutation,
            origin_node_id: None,
            reason: "cmo_story_slot_object_read_only".to_owned(),
        },
    }
}

pub fn geometry_sync_policy_v1(instance: &SceneInstanceV1) -> GeometrySyncPolicyV1 {
    match instance.projection_kind {
        SceneProjectionKindV1::DirectPageLocal => GeometrySyncPolicyV1::ApplyAuthoredOriginGeometry,
        SceneProjectionKindV1::InheritedMaster | SceneProjectionKindV1::CmoStorySlot => {
            GeometrySyncPolicyV1::ReprojectFromContext
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHAPE_380: &str = "10000000-0000-4000-8000-000000000380";
    const MASTER_263: &str = "20000000-0000-4000-8000-000000000263";
    const PAGE_266: &str = "20000000-0000-4000-8000-000000000266";
    const PAGE_361: &str = "20000000-0000-4000-8000-000000000361";
    const PAGE_406: &str = "20000000-0000-4000-8000-000000000406";

    fn master_relation(page: &str, seq: u32) -> MasterProjectionRelationV1 {
        MasterProjectionRelationV1 {
            source_page_id: page.to_owned(),
            source_page_seq_num: seq,
            master_page_id: MASTER_263.to_owned(),
            master_page_seq_num: 263,
        }
    }

    #[test]
    fn direct_instance_is_the_only_object_mutation_admission() {
        let instance = direct_page_local_instance_v1(SHAPE_380, PAGE_266).expect("direct instance");
        for mutation in [
            ObjectMutationKindV1::MoveNode,
            ObjectMutationKindV1::ReplaceImage,
        ] {
            let decision = admit_object_mutation_v1(&instance, mutation);
            assert!(decision.admitted);
            assert_eq!(decision.origin_node_id.as_deref(), Some(SHAPE_380));
        }
        assert_eq!(
            geometry_sync_policy_v1(&instance),
            GeometrySyncPolicyV1::ApplyAuthoredOriginGeometry
        );
    }

    #[test]
    fn exact_carlton_footer_instances_share_origin_but_not_visual_identity() {
        let pages = [(PAGE_266, 266), (PAGE_361, 361), (PAGE_406, 406)];
        let instances = pages
            .iter()
            .map(|(page, seq)| {
                inherited_master_instance_v1(
                    SHAPE_380,
                    MASTER_263,
                    page,
                    &master_relation(page, *seq),
                )
                .expect("master instance")
            })
            .collect::<Vec<_>>();

        assert!(
            instances
                .iter()
                .all(|item| item.origin_node_id == SHAPE_380)
        );
        assert_eq!(
            instances
                .iter()
                .map(|item| item.instance_id.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3
        );
        for instance in &instances {
            assert!(!admit_object_mutation_v1(instance, ObjectMutationKindV1::MoveNode).admitted);
            assert_eq!(
                geometry_sync_policy_v1(instance),
                GeometrySyncPolicyV1::ReprojectFromContext
            );
        }
    }

    #[test]
    fn inherited_master_hash_matches_existing_resolved_scene_law() {
        let instance = inherited_master_instance_v1(
            SHAPE_380,
            MASTER_263,
            PAGE_266,
            &master_relation(PAGE_266, 266),
        )
        .expect("instance");
        let expected = canonical_instance_id(&[
            ("origin", SHAPE_380.to_owned()),
            ("projection_kind", "inherited_master".to_owned()),
            ("target_page", PAGE_266.to_owned()),
        ]);
        assert_eq!(instance.instance_id, expected);
    }

    #[test]
    fn cmo_slot_blocks_object_mutation_without_blocking_story_authority() {
        let carrier = "30000000-0000-4000-8000-000000000441";
        let story = "40000000-0000-4000-8000-000000000057";
        let target_story = "40000000-0000-4000-8000-000000000049";
        let frame = "50000000-0000-4000-8000-000000000049";
        let relation = CmoProjectionRelationV1 {
            source_order: 3,
            cmo_id: 7,
            carrier_ohpo: 441,
            carrier_cmo_id: 7,
            target_qsid: 49,
            carrier_node_id: carrier.to_owned(),
            carrier_story_id: Some(story.to_owned()),
            target_story_id: target_story.to_owned(),
            target_frame_node_id: Some(frame.to_owned()),
        };
        let instance = cmo_story_slot_instance_v1(&relation, PAGE_266, 0, 0).expect("Cmo instance");
        assert_eq!(instance.story_authority_id.as_deref(), Some(story));
        assert!(!admit_object_mutation_v1(&instance, ObjectMutationKindV1::MoveNode).admitted);
        assert!(!admit_object_mutation_v1(&instance, ObjectMutationKindV1::ReplaceImage).admitted);
        assert_eq!(
            geometry_sync_policy_v1(&instance),
            GeometrySyncPolicyV1::ReprojectFromContext
        );
    }

    #[test]
    fn exact_carlton_shape380_has_three_read_only_customer_page_instances() {
        use pub_model::{derive_pub_node_id_v1, derive_pub_page_id_v1};

        let source_hash = "bf9cda0f632b5820ab9dbdbe1b838b2a988b2f3fdd69253c22b4fc3aef9f11c3";
        let origin = derive_pub_node_id_v1(source_hash, 380).expect("shape380");
        let master = derive_pub_page_id_v1(source_hash, 263).expect("PAGE263");
        let page_specs = [(266_u32, "PAGE266"), (361, "PAGE361"), (406, "PAGE406")];
        let mut instance_ids = std::collections::BTreeSet::new();

        for (seq, _label) in page_specs {
            let target = derive_pub_page_id_v1(source_hash, seq).expect("customer page");
            let relation = MasterProjectionRelationV1 {
                source_page_id: target.clone(),
                source_page_seq_num: seq,
                master_page_id: master.clone(),
                master_page_seq_num: 263,
            };
            let instance = inherited_master_instance_v1(&origin, &master, &target, &relation)
                .expect("Carlton inherited footer instance");
            assert!(instance_ids.insert(instance.instance_id.clone()));
            assert_eq!(instance.origin_node_id, origin);
            assert_eq!(instance.target_page_id, target);
            assert!(!admit_object_mutation_v1(&instance, ObjectMutationKindV1::MoveNode).admitted);
        }

        assert_eq!(instance_ids.len(), 3);
    }

    #[test]
    fn cmo_instances_at_distinct_slots_are_distinct_even_with_same_origin() {
        let carrier = "30000000-0000-4000-8000-000000000441";
        let target_story = "40000000-0000-4000-8000-000000000049";
        let frame = "50000000-0000-4000-8000-000000000049";
        let relation = CmoProjectionRelationV1 {
            source_order: 3,
            cmo_id: 7,
            carrier_ohpo: 441,
            carrier_cmo_id: 7,
            target_qsid: 49,
            carrier_node_id: carrier.to_owned(),
            carrier_story_id: None,
            target_story_id: target_story.to_owned(),
            target_frame_node_id: Some(frame.to_owned()),
        };
        let first = cmo_story_slot_instance_v1(&relation, PAGE_266, 0, 0).expect("first instance");
        let second =
            cmo_story_slot_instance_v1(&relation, PAGE_266, 1, 3).expect("second instance");
        assert_ne!(first.instance_id, second.instance_id);
        assert_eq!(first.origin_node_id, second.origin_node_id);
    }
}
