use serde::{Deserialize, Serialize};

pub const PUB_PROJECTION_CONTEXT_SCHEMA_V1: &str = "chaptera.pub-projection-context.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CmoProjectionRelationV1 {
    pub source_order: usize,
    pub cmo_id: u32,
    pub carrier_ohpo: u32,
    pub carrier_cmo_id: u32,
    pub target_qsid: u32,
    pub carrier_node_id: String,
    pub carrier_story_id: Option<String>,
    pub target_story_id: String,
    pub target_frame_node_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubProjectionContextV1 {
    pub schema_version: String,
    #[serde(default)]
    pub cmo_relations: Vec<CmoProjectionRelationV1>,
}

impl Default for PubProjectionContextV1 {
    fn default() -> Self {
        Self {
            schema_version: PUB_PROJECTION_CONTEXT_SCHEMA_V1.to_owned(),
            cmo_relations: Vec::new(),
        }
    }
}

impl PubProjectionContextV1 {
    pub fn with_cmo_relations(cmo_relations: Vec<CmoProjectionRelationV1>) -> Self {
        Self {
            schema_version: PUB_PROJECTION_CONTEXT_SCHEMA_V1.to_owned(),
            cmo_relations,
        }
    }

    pub fn cmo_relations_for_target_qsid(
        &self,
        target_qsid: u32,
    ) -> impl Iterator<Item = &CmoProjectionRelationV1> {
        self.cmo_relations
            .iter()
            .filter(move |relation| relation.target_qsid == target_qsid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relation(order: usize, qsid: u32) -> CmoProjectionRelationV1 {
        CmoProjectionRelationV1 {
            source_order: order,
            cmo_id: u32::try_from(order + 1).expect("order"),
            carrier_ohpo: 400 + u32::try_from(order).expect("order"),
            carrier_cmo_id: u32::try_from(order + 1).expect("order"),
            target_qsid: qsid,
            carrier_node_id: format!("10000000-0000-4000-8000-{order:012x}"),
            carrier_story_id: None,
            target_story_id: "20000000-0000-4000-8000-000000000001".to_owned(),
            target_frame_node_id: Some(
                "30000000-0000-4000-8000-000000000001".to_owned(),
            ),
        }
    }

    #[test]
    fn default_context_is_versioned_and_empty() {
        let context = PubProjectionContextV1::default();
        assert_eq!(context.schema_version, PUB_PROJECTION_CONTEXT_SCHEMA_V1);
        assert!(context.cmo_relations.is_empty());
    }

    #[test]
    fn target_projection_preserves_global_source_order() {
        let context = PubProjectionContextV1::with_cmo_relations(vec![
            relation(0, 49),
            relation(1, 218),
            relation(2, 49),
        ]);

        let orders = context
            .cmo_relations_for_target_qsid(49)
            .map(|relation| relation.source_order)
            .collect::<Vec<_>>();

        assert_eq!(orders, vec![0, 2]);
    }
}
