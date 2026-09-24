use serde::{Deserialize, Serialize};

pub const PUB_PROJECTION_CONTEXT_SCHEMA_V1: &str = "chaptera.pub-projection-context.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MasterProjectionRelationV1 {
    pub source_page_id: String,
    pub source_page_seq_num: u32,
    pub master_page_id: String,
    pub master_page_seq_num: u32,
}

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
    pub master_relations: Vec<MasterProjectionRelationV1>,
    #[serde(default)]
    pub cmo_relations: Vec<CmoProjectionRelationV1>,
}

impl Default for PubProjectionContextV1 {
    fn default() -> Self {
        Self {
            schema_version: PUB_PROJECTION_CONTEXT_SCHEMA_V1.to_owned(),
            master_relations: Vec::new(),
            cmo_relations: Vec::new(),
        }
    }
}

impl PubProjectionContextV1 {
    pub fn with_cmo_relations(cmo_relations: Vec<CmoProjectionRelationV1>) -> Self {
        Self {
            schema_version: PUB_PROJECTION_CONTEXT_SCHEMA_V1.to_owned(),
            master_relations: Vec::new(),
            cmo_relations,
        }
    }

    pub fn with_master_relations(master_relations: Vec<MasterProjectionRelationV1>) -> Self {
        Self {
            schema_version: PUB_PROJECTION_CONTEXT_SCHEMA_V1.to_owned(),
            master_relations,
            cmo_relations: Vec::new(),
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

    pub fn master_for_page(&self, source_page_id: &str) -> Option<&MasterProjectionRelationV1> {
        self.master_relations
            .iter()
            .find(|relation| relation.source_page_id == source_page_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmo_relation(order: usize, qsid: u32) -> CmoProjectionRelationV1 {
        CmoProjectionRelationV1 {
            source_order: order,
            cmo_id: u32::try_from(order + 1).expect("order"),
            carrier_ohpo: 400 + u32::try_from(order).expect("order"),
            carrier_cmo_id: u32::try_from(order + 1).expect("order"),
            target_qsid: qsid,
            carrier_node_id: format!("10000000-0000-4000-8000-{order:012x}"),
            carrier_story_id: None,
            target_story_id: "20000000-0000-4000-8000-000000000001".to_owned(),
            target_frame_node_id: Some("30000000-0000-4000-8000-000000000001".to_owned()),
        }
    }

    fn master_relation(source: &str, master: &str) -> MasterProjectionRelationV1 {
        MasterProjectionRelationV1 {
            source_page_id: source.to_owned(),
            source_page_seq_num: 266,
            master_page_id: master.to_owned(),
            master_page_seq_num: 263,
        }
    }

    #[test]
    fn default_context_is_versioned_and_empty() {
        let context = PubProjectionContextV1::default();
        assert_eq!(context.schema_version, PUB_PROJECTION_CONTEXT_SCHEMA_V1);
        assert!(context.master_relations.is_empty());
        assert!(context.cmo_relations.is_empty());
    }

    #[test]
    fn target_projection_preserves_global_source_order() {
        let context = PubProjectionContextV1::with_cmo_relations(vec![
            cmo_relation(0, 49),
            cmo_relation(1, 218),
            cmo_relation(2, 49),
        ]);

        let orders = context
            .cmo_relations_for_target_qsid(49)
            .map(|relation| relation.source_order)
            .collect::<Vec<_>>();

        assert_eq!(orders, vec![0, 2]);
        assert!(context.master_relations.is_empty());
    }

    #[test]
    fn master_lookup_is_page_specific() {
        let page_a = "40000000-0000-4000-8000-000000000001";
        let page_b = "40000000-0000-4000-8000-000000000002";
        let master = "50000000-0000-4000-8000-000000000001";
        let context = PubProjectionContextV1::with_master_relations(vec![
            master_relation(page_a, master),
            master_relation(page_b, master),
        ]);

        assert_eq!(
            context
                .master_for_page(page_a)
                .map(|relation| relation.master_page_id.as_str()),
            Some(master)
        );
        assert!(context.cmo_relations.is_empty());
    }
}
