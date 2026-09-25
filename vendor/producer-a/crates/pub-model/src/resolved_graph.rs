use crate::{
    Document, ExtensionId, Node, NodeId, OpaqueExtension, Page, PageId, Paragraph, ParagraphId,
    ResourceId, SourceDescriptor, Story, StoryId, Style, StyleId, TextRun, TextRunId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Effective authoring semantics derived deterministically from SourceGraph.
///
/// This is not a physical layout scene. It may retain provenance and
/// adapter-bounded payload while materializing only resolver rules that are
/// explicitly supported by evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedGraph<
    NodePayload,
    Resource,
    StyleKind,
    StyleProperties,
    ExtensionStorage,
    ParagraphProperties = (),
    RunProperties = (),
> {
    pub cdm_version: String,
    pub resolver_version: String,
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
