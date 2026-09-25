use crate::{
    Document, ExtensionId, Node, NodeId, OpaqueExtension, Page, PageId, Paragraph, ParagraphId,
    ResourceId, SourceDescriptor, Story, StoryId, Style, StyleId, TextRun, TextRunId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const CDM_VERSION_V0_1: &str = "0.1";

/// Parsed source semantics + provenance before resolver materialization.
///
/// This is the source-layer graph from the CDM v0.1 contract. It stores only
/// values and relations that the source adapter can support explicitly. It must
/// not synthesize effective defaults merely to make the graph look complete.
///
/// SourceGraph deliberately owns registries rather than raw parser trees.
/// Format-private bytes stay behind SourceCapsule/OpaqueExtension references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceGraph<
    NodePayload,
    Resource,
    StyleKind,
    StyleProperties,
    ExtensionStorage,
    ParagraphProperties = (),
    RunProperties = (),
> {
    pub cdm_version: String,
    pub source: SourceDescriptor,
    pub document: Document,
    pub pages: BTreeMap<PageId, Page>,
    pub nodes: BTreeMap<NodeId, Node<NodePayload>>,
    pub stories: BTreeMap<StoryId, Story>,
    pub paragraphs: BTreeMap<ParagraphId, Paragraph<ParagraphProperties>>,
    pub text_runs: BTreeMap<TextRunId, TextRun<RunProperties>>,
    pub resources: BTreeMap<ResourceId, Resource>,
    pub styles: BTreeMap<StyleId, Style<StyleKind, StyleProperties>>,
    pub extensions: BTreeMap<ExtensionId, OpaqueExtension<ExtensionStorage>>,
}

impl<
    NodePayload,
    Resource,
    StyleKind,
    StyleProperties,
    ExtensionStorage,
    ParagraphProperties,
    RunProperties,
>
    SourceGraph<
        NodePayload,
        Resource,
        StyleKind,
        StyleProperties,
        ExtensionStorage,
        ParagraphProperties,
        RunProperties,
    >
{
    pub fn empty(source: SourceDescriptor, document: Document) -> Self {
        Self {
            cdm_version: CDM_VERSION_V0_1.to_owned(),
            source,
            document,
            pages: BTreeMap::new(),
            nodes: BTreeMap::new(),
            stories: BTreeMap::new(),
            paragraphs: BTreeMap::new(),
            text_runs: BTreeMap::new(),
            resources: BTreeMap::new(),
            styles: BTreeMap::new(),
            extensions: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceGraphRegistryError {
    DocumentPageMissing {
        page_id: PageId,
    },
    PageKeyMismatch {
        key: PageId,
        entity_id: PageId,
    },
    PageChildMissing {
        page_id: PageId,
        node_id: NodeId,
    },
    NodeKeyMismatch {
        key: NodeId,
        entity_id: NodeId,
    },
    StoryKeyMismatch {
        key: StoryId,
        entity_id: StoryId,
    },
    ParagraphKeyMismatch {
        key: ParagraphId,
        entity_id: ParagraphId,
    },
    TextRunKeyMismatch {
        key: TextRunId,
        entity_id: TextRunId,
    },
    StoryParagraphMissing {
        story_id: StoryId,
        paragraph_id: ParagraphId,
    },
    StoryRunMissing {
        story_id: StoryId,
        run_id: TextRunId,
    },
    StyleKeyMismatch {
        key: StyleId,
        entity_id: StyleId,
    },
    ExtensionKeyMismatch {
        key: ExtensionId,
        entity_id: crate::CanonicalId,
    },
}

pub fn validate_source_graph_registries<
    NodePayload,
    Resource,
    StyleKind,
    StyleProperties,
    ExtensionStorage,
    ParagraphProperties,
    RunProperties,
>(
    graph: &SourceGraph<
        NodePayload,
        Resource,
        StyleKind,
        StyleProperties,
        ExtensionStorage,
        ParagraphProperties,
        RunProperties,
    >,
) -> Vec<SourceGraphRegistryError> {
    let mut errors = Vec::new();

    for page_id in &graph.document.pages {
        if !graph.pages.contains_key(page_id) {
            errors.push(SourceGraphRegistryError::DocumentPageMissing { page_id: *page_id });
        }
    }

    for (key, page) in &graph.pages {
        if *key != page.id {
            errors.push(SourceGraphRegistryError::PageKeyMismatch {
                key: *key,
                entity_id: page.id,
            });
        }

        for node_id in &page.children {
            if !graph.nodes.contains_key(node_id) {
                errors.push(SourceGraphRegistryError::PageChildMissing {
                    page_id: page.id,
                    node_id: *node_id,
                });
            }
        }
    }

    for (key, node) in &graph.nodes {
        if *key != node.header.id {
            errors.push(SourceGraphRegistryError::NodeKeyMismatch {
                key: *key,
                entity_id: node.header.id,
            });
        }
    }

    for (key, story) in &graph.stories {
        if *key != story.id {
            errors.push(SourceGraphRegistryError::StoryKeyMismatch {
                key: *key,
                entity_id: story.id,
            });
        }
        for paragraph_id in &story.paragraphs {
            if !graph.paragraphs.contains_key(paragraph_id) {
                errors.push(SourceGraphRegistryError::StoryParagraphMissing {
                    story_id: story.id,
                    paragraph_id: *paragraph_id,
                });
            }
        }
        for run_id in &story.runs {
            if !graph.text_runs.contains_key(run_id) {
                errors.push(SourceGraphRegistryError::StoryRunMissing {
                    story_id: story.id,
                    run_id: *run_id,
                });
            }
        }
    }

    for (key, paragraph) in &graph.paragraphs {
        if *key != paragraph.id {
            errors.push(SourceGraphRegistryError::ParagraphKeyMismatch {
                key: *key,
                entity_id: paragraph.id,
            });
        }
    }

    for (key, run) in &graph.text_runs {
        if *key != run.id {
            errors.push(SourceGraphRegistryError::TextRunKeyMismatch {
                key: *key,
                entity_id: run.id,
            });
        }
    }

    for (key, style) in &graph.styles {
        if *key != style.id {
            errors.push(SourceGraphRegistryError::StyleKeyMismatch {
                key: *key,
                entity_id: style.id,
            });
        }
    }

    for (key, extension) in &graph.extensions {
        if key.into_canonical() != extension.id {
            errors.push(SourceGraphRegistryError::ExtensionKeyMismatch {
                key: *key,
                entity_id: extension.id,
            });
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CanonicalId, DocumentId, LengthEmu, ParagraphId, Sha256Digest, Size2D, SourceDescriptor,
        TextRange, TextRunId,
    };

    fn id(byte: u8) -> CanonicalId {
        CanonicalId::from_bytes([byte; 16])
    }

    fn source_hash() -> Sha256Digest {
        Sha256Digest::from_bytes([0x11; 32])
    }

    fn source() -> SourceDescriptor {
        SourceDescriptor {
            format: "pub".into(),
            format_version: Some("0x2c".into()),
            adapter_version: "pub-rs/0.1".into(),
            source_hash: source_hash(),
        }
    }

    fn document(page_id: PageId) -> Document {
        Document {
            id: DocumentId::from_canonical(id(1)),
            format_origin: "pub".into(),
            source_hash: source_hash(),
            pages: vec![page_id],
            resources: Vec::new(),
            styles: Vec::new(),
        }
    }

    #[test]
    fn empty_graph_keeps_version_and_reports_missing_document_page() {
        let page_id = PageId::from_canonical(id(2));
        let graph = SourceGraph::<(), (), (), (), ()>::empty(source(), document(page_id));

        assert_eq!(graph.cdm_version, CDM_VERSION_V0_1);
        assert_eq!(
            validate_source_graph_registries(&graph),
            vec![SourceGraphRegistryError::DocumentPageMissing { page_id }]
        );
    }

    #[test]
    fn story_text_entities_require_resolvable_registries() {
        let page_id = PageId::from_canonical(id(2));
        let story_id = StoryId::from_canonical(id(4));
        let paragraph_id = ParagraphId::from_canonical(id(5));
        let run_id = TextRunId::from_canonical(id(6));
        let mut graph = SourceGraph::<(), (), (), (), ()>::empty(source(), document(page_id));

        graph.stories.insert(
            story_id,
            Story {
                id: story_id,
                text: "abc".into(),
                paragraphs: vec![paragraph_id],
                runs: vec![run_id],
                fields: Vec::new(),
                hyperlinks: Vec::new(),
                source_refs: Vec::new(),
            },
        );

        let errors = validate_source_graph_registries(&graph);
        assert!(
            errors.contains(&SourceGraphRegistryError::StoryParagraphMissing {
                story_id,
                paragraph_id,
            })
        );
        assert!(errors.contains(&SourceGraphRegistryError::StoryRunMissing { story_id, run_id }));

        graph.paragraphs.insert(
            paragraph_id,
            crate::Paragraph {
                id: paragraph_id,
                story_id,
                range: TextRange::new(0, 3).unwrap(),
                style_ref: None,
                properties: (),
            },
        );
        graph.text_runs.insert(
            run_id,
            crate::TextRun {
                id: run_id,
                story_id,
                range: TextRange::new(0, 3).unwrap(),
                style_ref: None,
                properties: (),
            },
        );

        let errors = validate_source_graph_registries(&graph);
        assert!(!errors.iter().any(|error| matches!(
            error,
            SourceGraphRegistryError::StoryParagraphMissing { .. }
                | SourceGraphRegistryError::StoryRunMissing { .. }
        )));
    }

    #[test]
    fn registry_validator_accepts_document_page_and_child_node() {
        let page_id = PageId::from_canonical(id(2));
        let node_id = NodeId::from_canonical(id(3));
        let mut graph = SourceGraph::<(), (), (), (), ()>::empty(source(), document(page_id));

        graph.pages.insert(
            page_id,
            Page {
                id: page_id,
                size: Size2D::new(LengthEmu::new(1), LengthEmu::new(1)),
                bleed: None,
                margins: None,
                children: vec![node_id],
                extensions: Vec::new(),
            },
        );
        graph.nodes.insert(
            node_id,
            Node {
                kind: crate::NodeKind::Shape,
                header: crate::NodeHeader {
                    id: node_id,
                    parent_id: page_id.into_canonical(),
                    bounds: crate::RectEmu::new(
                        LengthEmu::ZERO,
                        LengthEmu::ZERO,
                        LengthEmu::new(1),
                        LengthEmu::new(1),
                    ),
                    transform: crate::Affine2D::identity(),
                    source_refs: Vec::new(),
                    extensions: Vec::new(),
                },
                payload: (),
            },
        );

        assert!(validate_source_graph_registries(&graph).is_empty());
    }
}
