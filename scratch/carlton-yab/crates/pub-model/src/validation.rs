use crate::{
    CanonicalId, NodeId, NodeKind, PageValidationError, ParagraphId, ResourceId, SourceGraph,
    SourceGraphRegistryError, SourceRef, SourceRefValidationError, StoryId, StyleId, TextRange,
    TextRunId, validate_source_graph_registries,
};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticGraphError {
    Registry(SourceGraphRegistryError),
    DocumentSourceHashMismatch,
    DuplicateDocumentPage {
        page_id: crate::PageId,
    },
    DuplicateDocumentResource {
        resource_id: ResourceId,
    },
    MissingDocumentResource {
        resource_id: ResourceId,
    },
    DuplicateDocumentStyle {
        style_id: StyleId,
    },
    MissingDocumentStyle {
        style_id: StyleId,
    },
    PageValidation {
        page_id: crate::PageId,
        error: PageValidationError,
    },
    PageChildParentMismatch {
        page_id: crate::PageId,
        node_id: NodeId,
        actual_parent: CanonicalId,
    },
    MissingPageExtension {
        page_id: crate::PageId,
        extension_id: crate::ExtensionId,
    },
    MissingNodeParent {
        node_id: NodeId,
        parent_id: CanonicalId,
    },
    NodeParentNotGroup {
        node_id: NodeId,
        parent_node_id: NodeId,
    },
    NodeParentCycle {
        node_id: NodeId,
    },
    MissingNodeExtension {
        node_id: NodeId,
        extension_id: crate::ExtensionId,
    },
    ParagraphOwnedByDifferentStory {
        paragraph_id: ParagraphId,
        expected_story_id: StoryId,
        actual_story_id: StoryId,
    },
    TextRunOwnedByDifferentStory {
        run_id: TextRunId,
        expected_story_id: StoryId,
        actual_story_id: StoryId,
    },
    ParagraphRangeOutsideStory {
        paragraph_id: ParagraphId,
        range: TextRange,
    },
    TextRunRangeOutsideStory {
        run_id: TextRunId,
        range: TextRange,
    },
    ParagraphOrderOrOverlap {
        previous_id: ParagraphId,
        previous_range: TextRange,
        current_id: ParagraphId,
        current_range: TextRange,
    },
    MissingParagraphStyle {
        paragraph_id: ParagraphId,
        style_id: StyleId,
    },
    MissingTextRunStyle {
        run_id: TextRunId,
        style_id: StyleId,
    },
    MissingParentStyle {
        style_id: StyleId,
        parent_style_id: StyleId,
    },
    MissingExtensionOwner {
        extension_id: crate::ExtensionId,
        owner_id: CanonicalId,
    },
    InvalidPrimarySourceRef {
        owner_id: CanonicalId,
        error: SourceRefValidationError,
    },
}

/// Validates semantic invariants expressible by the current typed SourceGraph.
///
/// The validator never invents reverse relations. In particular, a Node may
/// name a Page parent while Page.children remains empty when source z-order is
/// unknown. If Page.children explicitly contains a Node, however, the Node
/// parent must agree.
pub fn validate_source_graph_semantics<
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
) -> Vec<SemanticGraphError> {
    let mut errors = validate_source_graph_registries(graph)
        .into_iter()
        .map(SemanticGraphError::Registry)
        .collect::<Vec<_>>();

    if graph.document.source_hash != graph.source.source_hash {
        errors.push(SemanticGraphError::DocumentSourceHashMismatch);
    }

    validate_document_refs(graph, &mut errors);
    validate_pages_and_parents(graph, &mut errors);
    validate_text(graph, &mut errors);
    validate_styles(graph, &mut errors);
    validate_extensions(graph, &mut errors);
    validate_source_refs(graph, &mut errors);

    errors
}

fn validate_document_refs<
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
    errors: &mut Vec<SemanticGraphError>,
) {
    let mut pages = BTreeSet::new();
    for page_id in &graph.document.pages {
        if !pages.insert(*page_id) {
            errors.push(SemanticGraphError::DuplicateDocumentPage { page_id: *page_id });
        }
    }

    let mut resources = BTreeSet::new();
    for resource_id in &graph.document.resources {
        if !resources.insert(*resource_id) {
            errors.push(SemanticGraphError::DuplicateDocumentResource {
                resource_id: *resource_id,
            });
        }
        if !graph.resources.contains_key(resource_id) {
            errors.push(SemanticGraphError::MissingDocumentResource {
                resource_id: *resource_id,
            });
        }
    }

    let mut styles = BTreeSet::new();
    for style_id in &graph.document.styles {
        if !styles.insert(*style_id) {
            errors.push(SemanticGraphError::DuplicateDocumentStyle {
                style_id: *style_id,
            });
        }
        if !graph.styles.contains_key(style_id) {
            errors.push(SemanticGraphError::MissingDocumentStyle {
                style_id: *style_id,
            });
        }
    }
}

fn validate_pages_and_parents<
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
    errors: &mut Vec<SemanticGraphError>,
) {
    let page_ids = graph
        .pages
        .keys()
        .map(|id| id.into_canonical())
        .collect::<BTreeSet<_>>();

    for page in graph.pages.values() {
        if let Err(error) = page.validate() {
            errors.push(SemanticGraphError::PageValidation {
                page_id: page.id,
                error,
            });
        }
        for child_id in &page.children {
            if let Some(node) = graph.nodes.get(child_id) {
                let expected = page.id.into_canonical();
                if node.header.parent_id != expected {
                    errors.push(SemanticGraphError::PageChildParentMismatch {
                        page_id: page.id,
                        node_id: *child_id,
                        actual_parent: node.header.parent_id,
                    });
                }
            }
        }
        for extension_id in &page.extensions {
            if !graph.extensions.contains_key(extension_id) {
                errors.push(SemanticGraphError::MissingPageExtension {
                    page_id: page.id,
                    extension_id: *extension_id,
                });
            }
        }
    }

    for (node_id, node) in &graph.nodes {
        let parent = node.header.parent_id;
        if !page_ids.contains(&parent) {
            let parent_node_id = NodeId::from_canonical(parent);
            match graph.nodes.get(&parent_node_id) {
                Some(parent_node) if parent_node.kind != NodeKind::Group => {
                    errors.push(SemanticGraphError::NodeParentNotGroup {
                        node_id: *node_id,
                        parent_node_id,
                    });
                }
                Some(_) => {}
                None => errors.push(SemanticGraphError::MissingNodeParent {
                    node_id: *node_id,
                    parent_id: parent,
                }),
            }
        }

        for extension_id in &node.header.extensions {
            if !graph.extensions.contains_key(extension_id) {
                errors.push(SemanticGraphError::MissingNodeExtension {
                    node_id: *node_id,
                    extension_id: *extension_id,
                });
            }
        }

        if has_parent_cycle(graph, *node_id) {
            errors.push(SemanticGraphError::NodeParentCycle { node_id: *node_id });
        }
    }
}

fn has_parent_cycle<
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
    start: NodeId,
) -> bool {
    let mut seen = BTreeSet::new();
    let mut current = start.into_canonical();
    seen.insert(current);

    loop {
        let node_id = NodeId::from_canonical(current);
        let Some(node) = graph.nodes.get(&node_id) else {
            return false;
        };
        let parent = node.header.parent_id;
        let parent_node_id = NodeId::from_canonical(parent);
        if !graph.nodes.contains_key(&parent_node_id) {
            return false;
        }
        if !seen.insert(parent) {
            return true;
        }
        current = parent;
    }
}

fn validate_text<
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
    errors: &mut Vec<SemanticGraphError>,
) {
    for story in graph.stories.values() {
        let mut previous: Option<(ParagraphId, TextRange)> = None;

        for paragraph_id in &story.paragraphs {
            let Some(paragraph) = graph.paragraphs.get(paragraph_id) else {
                continue;
            };
            if paragraph.story_id != story.id {
                errors.push(SemanticGraphError::ParagraphOwnedByDifferentStory {
                    paragraph_id: paragraph.id,
                    expected_story_id: story.id,
                    actual_story_id: paragraph.story_id,
                });
            }
            if !paragraph.range.fits_text(&story.text) {
                errors.push(SemanticGraphError::ParagraphRangeOutsideStory {
                    paragraph_id: paragraph.id,
                    range: paragraph.range,
                });
            }
            if let Some((previous_id, previous_range)) = previous {
                if paragraph.range.start < previous_range.end {
                    errors.push(SemanticGraphError::ParagraphOrderOrOverlap {
                        previous_id,
                        previous_range,
                        current_id: paragraph.id,
                        current_range: paragraph.range,
                    });
                }
            }
            if let Some(style_id) = paragraph.style_ref {
                if !graph.styles.contains_key(&style_id) {
                    errors.push(SemanticGraphError::MissingParagraphStyle {
                        paragraph_id: paragraph.id,
                        style_id,
                    });
                }
            }
            previous = Some((paragraph.id, paragraph.range));
        }

        for run_id in &story.runs {
            let Some(run) = graph.text_runs.get(run_id) else {
                continue;
            };
            if run.story_id != story.id {
                errors.push(SemanticGraphError::TextRunOwnedByDifferentStory {
                    run_id: run.id,
                    expected_story_id: story.id,
                    actual_story_id: run.story_id,
                });
            }
            if !run.range.fits_text(&story.text) {
                errors.push(SemanticGraphError::TextRunRangeOutsideStory {
                    run_id: run.id,
                    range: run.range,
                });
            }
            if let Some(style_id) = run.style_ref {
                if !graph.styles.contains_key(&style_id) {
                    errors.push(SemanticGraphError::MissingTextRunStyle {
                        run_id: run.id,
                        style_id,
                    });
                }
            }
        }
    }
}

fn validate_styles<
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
    errors: &mut Vec<SemanticGraphError>,
) {
    for style in graph.styles.values() {
        if let Some(parent_style_id) = style.parent_style {
            if !graph.styles.contains_key(&parent_style_id) {
                errors.push(SemanticGraphError::MissingParentStyle {
                    style_id: style.id,
                    parent_style_id,
                });
            }
        }
    }
}

fn validate_extensions<
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
    errors: &mut Vec<SemanticGraphError>,
) {
    let mut entities = BTreeSet::new();
    entities.insert(graph.document.id.into_canonical());
    entities.extend(graph.pages.keys().map(|id| id.into_canonical()));
    entities.extend(graph.nodes.keys().map(|id| id.into_canonical()));
    entities.extend(graph.stories.keys().map(|id| id.into_canonical()));
    entities.extend(graph.paragraphs.keys().map(|id| id.into_canonical()));
    entities.extend(graph.text_runs.keys().map(|id| id.into_canonical()));
    entities.extend(graph.resources.keys().map(|id| id.into_canonical()));
    entities.extend(graph.styles.keys().map(|id| id.into_canonical()));
    entities.extend(graph.extensions.values().map(|extension| extension.id));

    for (extension_id, extension) in &graph.extensions {
        if let Some(owner_id) = extension.owner_id {
            if !entities.contains(&owner_id) {
                errors.push(SemanticGraphError::MissingExtensionOwner {
                    extension_id: *extension_id,
                    owner_id,
                });
            }
        }
    }
}

fn validate_source_refs<
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
    errors: &mut Vec<SemanticGraphError>,
) {
    for node in graph.nodes.values() {
        validate_refs(
            node.header.id.into_canonical(),
            &node.header.source_refs,
            &graph.source,
            errors,
        );
    }
    for story in graph.stories.values() {
        validate_refs(
            story.id.into_canonical(),
            &story.source_refs,
            &graph.source,
            errors,
        );
    }
    for style in graph.styles.values() {
        validate_refs(
            style.id.into_canonical(),
            &style.source_refs,
            &graph.source,
            errors,
        );
    }
    for extension in graph.extensions.values() {
        validate_refs(extension.id, &extension.source_refs, &graph.source, errors);
    }
}

fn validate_refs(
    owner_id: CanonicalId,
    refs: &[SourceRef],
    source: &crate::SourceDescriptor,
    errors: &mut Vec<SemanticGraphError>,
) {
    for reference in refs {
        if let Err(error) = reference.validate_primary_source(source) {
            errors.push(SemanticGraphError::InvalidPrimarySourceRef { owner_id, error });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Affine2D, CanonicalId, Document, DocumentId, LengthEmu, Node, NodeHeader, Page, PageId,
        Paragraph, Sha256Digest, Size2D, SourceDescriptor, Story, TextRun,
    };

    fn id(byte: u8) -> CanonicalId {
        CanonicalId::from_bytes([byte; 16])
    }

    fn source() -> SourceDescriptor {
        SourceDescriptor {
            format: "pub".into(),
            format_version: Some("0x2c".into()),
            adapter_version: "pub-rs/0.1".into(),
            source_hash: Sha256Digest::from_bytes([0x11; 32]),
        }
    }

    fn graph() -> SourceGraph<(), (), (), (), (), (), ()> {
        let page_id = PageId::from_canonical(id(2));
        let mut graph = SourceGraph::empty(
            source(),
            Document {
                id: DocumentId::from_canonical(id(1)),
                format_origin: "pub".into(),
                source_hash: Sha256Digest::from_bytes([0x11; 32]),
                pages: vec![page_id],
                resources: Vec::new(),
                styles: Vec::new(),
            },
        );
        graph.pages.insert(
            page_id,
            Page {
                id: page_id,
                size: Size2D::new(LengthEmu::new(100), LengthEmu::new(100)),
                bleed: None,
                margins: None,
                children: Vec::new(),
                extensions: Vec::new(),
            },
        );
        graph
    }

    #[test]
    fn valid_minimal_graph_passes() {
        assert!(validate_source_graph_semantics(&graph()).is_empty());
    }

    #[test]
    fn parent_cycle_is_rejected() {
        let mut graph = graph();
        let a = NodeId::from_canonical(id(10));
        let b = NodeId::from_canonical(id(11));

        for (node_id, parent_id) in [(a, b.into_canonical()), (b, a.into_canonical())] {
            graph.nodes.insert(
                node_id,
                Node {
                    kind: NodeKind::Group,
                    header: NodeHeader {
                        id: node_id,
                        parent_id,
                        bounds: crate::RectEmu::new(
                            LengthEmu::ZERO,
                            LengthEmu::ZERO,
                            LengthEmu::new(1),
                            LengthEmu::new(1),
                        ),
                        transform: Affine2D::identity(),
                        source_refs: Vec::new(),
                        extensions: Vec::new(),
                    },
                    payload: (),
                },
            );
        }

        let errors = validate_source_graph_semantics(&graph);
        assert!(errors.contains(&SemanticGraphError::NodeParentCycle { node_id: a }));
        assert!(errors.contains(&SemanticGraphError::NodeParentCycle { node_id: b }));
    }

    #[test]
    fn story_ranges_and_ownership_are_checked() {
        let mut graph = graph();
        let story_id = StoryId::from_canonical(id(20));
        let other_story_id = StoryId::from_canonical(id(21));
        let paragraph_id = ParagraphId::from_canonical(id(22));
        let run_id = TextRunId::from_canonical(id(23));

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
        graph.paragraphs.insert(
            paragraph_id,
            Paragraph {
                id: paragraph_id,
                story_id,
                range: TextRange::new(0, 4).unwrap(),
                style_ref: None,
                properties: (),
            },
        );
        graph.text_runs.insert(
            run_id,
            TextRun {
                id: run_id,
                story_id: other_story_id,
                range: TextRange::new(0, 3).unwrap(),
                style_ref: None,
                properties: (),
            },
        );

        let errors = validate_source_graph_semantics(&graph);
        assert!(
            errors.contains(&SemanticGraphError::ParagraphRangeOutsideStory {
                paragraph_id,
                range: TextRange::new(0, 4).unwrap(),
            })
        );
        assert!(
            errors.contains(&SemanticGraphError::TextRunOwnedByDifferentStory {
                run_id,
                expected_story_id: story_id,
                actual_story_id: other_story_id,
            })
        );
    }
}
