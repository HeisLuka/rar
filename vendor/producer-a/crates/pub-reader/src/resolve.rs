use crate::{
    PubExplicitImageCropSource, PubExplicitShapePaintSource, PubNodePayload, PubSourceGraph,
    PubTableSource, PubTableStoryOwnershipSource,
};
use anyhow::{Result, bail};
use pub_model::{Node, NodeId, ResolvedGraph, StoryId, validate_source_graph_registries};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PUB_RESOLVER_VERSION_V1: &str = "pub-resolver-v1";

pub type PubResolvedGraph = ResolvedGraph<PubResolvedNodePayload, (), (), (), String>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubResolvedNodePayload {
    pub contents_seq_num: u32,
    pub officeart_shape_type: Option<u16>,
    pub officeart_spid: Option<u32>,
    pub image_slot: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explicit_image_crop: Option<PubExplicitImageCropSource>,
    pub explicit_paint: PubExplicitShapePaintSource,
    pub story_frame: Option<PubResolvedStoryFrame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table_story: Option<PubTableStoryOwnershipSource>,
    pub table: Option<PubTableSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubResolvedStoryFrame {
    pub story_id: Option<StoryId>,
    pub ordinal: u32,
    pub previous_frame: Option<NodeId>,
    pub next_frame: Option<NodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum PubResolveDiagnostic {
    MissingStoryIdentity { node_id: NodeId, text_id: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubResolvedGraphBuild {
    pub graph: PubResolvedGraph,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<PubResolveDiagnostic>,
}

/// Resolves only evidence-backed effective authoring semantics.
///
/// Current v1 rule:
/// - absent Publisher frame ordinal means effective ordinal 0;
/// - explicit previous/next relations are preserved exactly;
/// - ordinal never synthesizes a flow edge;
/// - unresolved Story identity remains unresolved and is diagnosed.
pub fn resolve_pub_source_graph(source: &PubSourceGraph) -> Result<PubResolvedGraphBuild> {
    let registry_errors = validate_source_graph_registries(source);
    if !registry_errors.is_empty() {
        bail!("SourceGraph registry validation failed: {registry_errors:?}");
    }

    let mut diagnostics = Vec::new();
    let mut nodes = BTreeMap::new();

    for (node_id, node) in &source.nodes {
        let payload = resolve_node_payload(*node_id, &node.payload, &mut diagnostics);
        nodes.insert(
            *node_id,
            Node {
                kind: node.kind,
                header: node.header.clone(),
                payload,
            },
        );
    }

    let graph = ResolvedGraph {
        cdm_version: source.cdm_version.clone(),
        resolver_version: PUB_RESOLVER_VERSION_V1.into(),
        source: source.source.clone(),
        document: source.document.clone(),
        pages: source.pages.clone(),
        nodes,
        stories: source.stories.clone(),
        paragraphs: source.paragraphs.clone(),
        text_runs: source.text_runs.clone(),
        resources: source.resources.clone(),
        styles: source.styles.clone(),
        extensions: source.extensions.clone(),
    };

    Ok(PubResolvedGraphBuild { graph, diagnostics })
}

fn resolve_node_payload(
    node_id: NodeId,
    payload: &PubNodePayload,
    diagnostics: &mut Vec<PubResolveDiagnostic>,
) -> PubResolvedNodePayload {
    let story_frame = payload.story_frame.as_ref().map(|frame| {
        if frame.story_id.is_none() {
            diagnostics.push(PubResolveDiagnostic::MissingStoryIdentity {
                node_id,
                text_id: frame.text_id,
            });
        }

        PubResolvedStoryFrame {
            story_id: frame.story_id,
            ordinal: frame.explicit_ordinal.unwrap_or(0),
            previous_frame: frame.previous_frame,
            next_frame: frame.next_frame,
        }
    });

    PubResolvedNodePayload {
        contents_seq_num: payload.contents_seq_num,
        officeart_shape_type: payload.officeart_shape_type,
        officeart_spid: payload.officeart_spid,
        image_slot: payload.image_slot,
        explicit_image_crop: payload.explicit_image_crop.clone(),
        explicit_paint: payload.explicit_paint.clone(),
        story_frame,
        table_story: payload.table_story.clone(),
        table: payload.table.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pub_model::{CanonicalId, NodeId};

    #[test]
    fn resolver_preserves_bounded_image_crop_without_reinterpretation() {
        let node_id = NodeId::from_canonical(CanonicalId::from_bytes([0x2a; 16]));
        let source_crop = PubExplicitImageCropSource {
            top_raw: Some(28_954),
            bottom_raw: Some(21_446),
            left_raw: None,
            right_raw: Some(0),
            ambiguous: false,
        };
        let payload = PubNodePayload {
            contents_seq_num: 315,
            officeart_shape_type: Some(75),
            officeart_spid: Some(315),
            image_slot: Some(1),
            explicit_image_crop: Some(source_crop.clone()),
            explicit_paint: PubExplicitShapePaintSource::default(),
            story_frame: None,
            table_story: None,
            table: None,
        };

        let mut diagnostics = Vec::new();
        let resolved = resolve_node_payload(node_id, &payload, &mut diagnostics);

        assert_eq!(resolved.image_slot, Some(1));
        assert_eq!(resolved.explicit_image_crop, Some(source_crop));
        assert!(diagnostics.is_empty());
    }
}
