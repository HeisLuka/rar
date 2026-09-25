use crate::{
    Affine2D, CanonicalId, ExtensionId, LengthEmu, NodeId, PageId, ParagraphId, RectEmu,
    ResourceId, Sha256Digest, SourceRef, StoryId, StyleId, TextRange, TextRunId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size2D {
    pub width: LengthEmu,
    pub height: LengthEmu,
}

impl Size2D {
    pub const fn new(width: LengthEmu, height: LengthEmu) -> Self {
        Self { width, height }
    }

    pub const fn is_positive(self) -> bool {
        self.width.0 > 0 && self.height.0 > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoxEdges {
    pub top: LengthEmu,
    pub right: LengthEmu,
    pub bottom: LengthEmu,
    pub left: LengthEmu,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    pub id: crate::DocumentId,
    pub format_origin: String,
    pub source_hash: Sha256Digest,
    pub pages: Vec<PageId>,
    pub resources: Vec<ResourceId>,
    pub styles: Vec<StyleId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page {
    pub id: PageId,
    pub size: Size2D,
    pub bleed: Option<BoxEdges>,
    pub margins: Option<BoxEdges>,
    pub children: Vec<NodeId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<ExtensionId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageValidationError {
    NonPositiveSize { width: LengthEmu, height: LengthEmu },
    DuplicateChild { child_id: NodeId },
}

impl Page {
    pub fn validate(&self) -> Result<(), PageValidationError> {
        if !self.size.is_positive() {
            return Err(PageValidationError::NonPositiveSize {
                width: self.size.width,
                height: self.size.height,
            });
        }

        let mut seen = BTreeSet::new();
        for child_id in &self.children {
            if !seen.insert(*child_id) {
                return Err(PageValidationError::DuplicateChild {
                    child_id: *child_id,
                });
            }
        }

        Ok(())
    }
}

/// Общий header visual node.
///
/// parent_id остаётся CanonicalId, потому что machine schema допускает parent
/// разных semantic classes (Page или Group). Сужать его до одного typed ID было
/// бы сильнее текущего contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeHeader {
    pub id: NodeId,
    pub parent_id: CanonicalId,
    pub bounds: RectEmu,
    pub transform: Affine2D,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<SourceRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<ExtensionId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Shape,
    TextFrame,
    ImageFrame,
    VectorPath,
    Group,
    Connector,
    Table,
    PlacedArtifact,
    Unsupported,
}

/// Structural node envelope.
///
/// Payload остаётся generic: machine schema v0.1 намеренно оставляет rich
/// payload property sets открытыми до отдельных registries/spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node<Payload> {
    pub kind: NodeKind,
    pub header: NodeHeader,
    pub payload: Payload,
}

/// Минимальный typed envelope TextFrame.
///
/// FrameFlow и дополнительные frame properties пока не стандартизованы в
/// implementation core, поэтому caller предоставляет их отдельными типами.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextFrame<FrameFlow, Properties> {
    pub story_id: StoryId,
    pub frame_flow: FrameFlow,
    pub properties: Properties,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PixelSize {
    pub width: u32,
    pub height: u32,
}

/// Shared image resource отделён от placement.
///
/// Color-profile ref, alpha semantics и blob handle остаются generic, потому
/// что их concrete registries ещё не закрыты. Identity/hash/mime/intrinsic
/// dimensions уже являются частью formal CDM contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageResource<ColorProfileRef, Alpha, BlobRef> {
    pub id: ResourceId,
    pub mime: String,
    pub source_hash: Sha256Digest,
    pub intrinsic_size_px: Option<PixelSize>,
    pub color_profile: Option<ColorProfileRef>,
    pub alpha: Alpha,
    pub original_blob: BlobRef,
}

/// Минимальный typed envelope ImageFrame.
///
/// Crop/Fit/Clipping property registries пока не закрыты, поэтому rich image
/// placement state остаётся типом caller'а.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageFrame<Properties> {
    pub image_id: ResourceId,
    pub properties: Properties,
}

/// Style остаётся graph entity, а не materialized effective property bag.
///
/// Kind и Properties generic, потому что concrete style/property registries
/// закрываются отдельными задачами. parent_style уже typed как StyleId.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Style<Kind, Properties> {
    pub id: StyleId,
    pub kind: Kind,
    pub parent_style: Option<StyleId>,
    pub properties: Properties,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<SourceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Story {
    pub id: StoryId,
    pub text: String,
    pub paragraphs: Vec<ParagraphId>,
    pub runs: Vec<TextRunId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<CanonicalId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hyperlinks: Vec<CanonicalId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<SourceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Paragraph<Properties> {
    pub id: ParagraphId,
    pub story_id: StoryId,
    pub range: TextRange,
    pub style_ref: Option<StyleId>,
    pub properties: Properties,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextRun<Properties> {
    pub id: TextRunId,
    pub story_id: StoryId,
    pub range: TextRange,
    pub style_ref: Option<StyleId>,
    pub properties: Properties,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoryStructureError {
    MissingParagraph {
        paragraph_id: ParagraphId,
    },
    MissingRun {
        run_id: TextRunId,
    },
    ParagraphOwnedByDifferentStory {
        paragraph_id: ParagraphId,
        actual_story_id: StoryId,
    },
    RunOwnedByDifferentStory {
        run_id: TextRunId,
        actual_story_id: StoryId,
    },
    ParagraphRangeOutsideStory {
        paragraph_id: ParagraphId,
        range: TextRange,
    },
    RunRangeOutsideStory {
        run_id: TextRunId,
        range: TextRange,
    },
    ParagraphOrderOrOverlap {
        previous_id: ParagraphId,
        previous_range: TextRange,
        current_id: ParagraphId,
        current_range: TextRange,
    },
}

pub fn validate_story_structure<ParagraphProperties, RunProperties>(
    story: &Story,
    paragraphs: &[Paragraph<ParagraphProperties>],
    runs: &[TextRun<RunProperties>],
) -> Vec<StoryStructureError> {
    let mut errors = Vec::new();
    let mut previous: Option<(&ParagraphId, TextRange)> = None;

    for paragraph_id in &story.paragraphs {
        let Some(paragraph) = paragraphs.iter().find(|item| item.id == *paragraph_id) else {
            errors.push(StoryStructureError::MissingParagraph {
                paragraph_id: *paragraph_id,
            });
            continue;
        };

        if paragraph.story_id != story.id {
            errors.push(StoryStructureError::ParagraphOwnedByDifferentStory {
                paragraph_id: paragraph.id,
                actual_story_id: paragraph.story_id,
            });
        }

        if !paragraph.range.fits_text(&story.text) {
            errors.push(StoryStructureError::ParagraphRangeOutsideStory {
                paragraph_id: paragraph.id,
                range: paragraph.range,
            });
        }

        if let Some((previous_id, previous_range)) = previous {
            if paragraph.range.start < previous_range.end {
                errors.push(StoryStructureError::ParagraphOrderOrOverlap {
                    previous_id: *previous_id,
                    previous_range,
                    current_id: paragraph.id,
                    current_range: paragraph.range,
                });
            }
        }

        previous = Some((&paragraph.id, paragraph.range));
    }

    for run_id in &story.runs {
        let Some(run) = runs.iter().find(|item| item.id == *run_id) else {
            errors.push(StoryStructureError::MissingRun { run_id: *run_id });
            continue;
        };

        if run.story_id != story.id {
            errors.push(StoryStructureError::RunOwnedByDifferentStory {
                run_id: run.id,
                actual_story_id: run.story_id,
            });
        }

        if !run.range.fits_text(&story.text) {
            errors.push(StoryStructureError::RunRangeOutsideStory {
                run_id: run.id,
                range: run.range,
            });
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CanonicalId, Decimal, DocumentId};

    fn id(byte: u8) -> CanonicalId {
        CanonicalId::from_bytes([byte; 16])
    }

    fn page_id(byte: u8) -> PageId {
        PageId::from_canonical(id(byte))
    }

    fn node_id(byte: u8) -> NodeId {
        NodeId::from_canonical(id(byte))
    }

    fn story_id(byte: u8) -> StoryId {
        StoryId::from_canonical(id(byte))
    }

    fn paragraph_id(byte: u8) -> ParagraphId {
        ParagraphId::from_canonical(id(byte))
    }

    fn run_id(byte: u8) -> TextRunId {
        TextRunId::from_canonical(id(byte))
    }

    fn source_hash() -> Sha256Digest {
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .parse()
            .expect("валидный SHA-256")
    }

    #[test]
    fn document_keeps_ordered_page_identity_separate_from_source_hash() {
        let document = Document {
            id: DocumentId::from_canonical(id(1)),
            format_origin: "pub".into(),
            source_hash: source_hash(),
            pages: vec![page_id(10), page_id(11)],
            resources: Vec::new(),
            styles: Vec::new(),
        };

        assert_eq!(document.pages, vec![page_id(10), page_id(11)]);
        assert_eq!(document.source_hash, source_hash());
    }

    #[test]
    fn page_rejects_non_positive_size_and_duplicate_children() {
        let invalid_size = Page {
            id: page_id(1),
            size: Size2D::new(LengthEmu::new(0), LengthEmu::new(914_400)),
            bleed: None,
            margins: None,
            children: Vec::new(),
            extensions: Vec::new(),
        };
        assert!(matches!(
            invalid_size.validate(),
            Err(PageValidationError::NonPositiveSize { .. })
        ));

        let duplicate = Page {
            id: page_id(2),
            size: Size2D::new(LengthEmu::new(914_400), LengthEmu::new(914_400)),
            bleed: None,
            margins: None,
            children: vec![node_id(3), node_id(3)],
            extensions: Vec::new(),
        };
        assert!(matches!(
            duplicate.validate(),
            Err(PageValidationError::DuplicateChild { .. })
        ));
    }

    #[test]
    fn node_payload_stays_typed_without_freezing_rich_property_registry() {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        struct TestShape {
            name: String,
        }

        let node = Node {
            kind: NodeKind::Shape,
            header: NodeHeader {
                id: node_id(4),
                parent_id: page_id(1).into_canonical(),
                bounds: RectEmu::new(
                    LengthEmu::ZERO,
                    LengthEmu::ZERO,
                    LengthEmu::new(100),
                    LengthEmu::new(100),
                ),
                transform: Affine2D::identity(),
                source_refs: Vec::new(),
                extensions: Vec::new(),
            },
            payload: TestShape {
                name: "grounded-shape".into(),
            },
        };

        assert_eq!(node.kind, NodeKind::Shape);
        assert_eq!(node.payload.name, "grounded-shape");
        assert_eq!(node.header.transform.a, Decimal::one());
    }

    #[test]
    fn style_keeps_inheritance_relation_instead_of_flattening_it() {
        let parent = StyleId::from_canonical(id(30));
        let style = Style {
            id: StyleId::from_canonical(id(31)),
            kind: "paragraph",
            parent_style: Some(parent),
            properties: (),
            source_refs: Vec::new(),
        };

        assert_eq!(style.parent_style, Some(parent));
    }

    #[test]
    fn story_structure_accepts_unicode_scalar_ranges() {
        let story = Story {
            id: story_id(1),
            text: "A😀Б".into(),
            paragraphs: vec![paragraph_id(2)],
            runs: vec![run_id(3)],
            fields: Vec::new(),
            hyperlinks: Vec::new(),
            source_refs: Vec::new(),
        };

        let paragraphs = vec![Paragraph {
            id: paragraph_id(2),
            story_id: story.id,
            range: TextRange::new(0, 3).expect("валидный paragraph range"),
            style_ref: None,
            properties: (),
        }];
        let runs = vec![TextRun {
            id: run_id(3),
            story_id: story.id,
            range: TextRange::new(1, 2).expect("валидный run range"),
            style_ref: None,
            properties: (),
        }];

        assert!(validate_story_structure(&story, &paragraphs, &runs).is_empty());
    }

    #[test]
    fn story_structure_rejects_overlapping_paragraphs() {
        let story = Story {
            id: story_id(1),
            text: "abcdef".into(),
            paragraphs: vec![paragraph_id(2), paragraph_id(3)],
            runs: Vec::new(),
            fields: Vec::new(),
            hyperlinks: Vec::new(),
            source_refs: Vec::new(),
        };

        let paragraphs = vec![
            Paragraph {
                id: paragraph_id(2),
                story_id: story.id,
                range: TextRange::new(0, 4).expect("валидный range"),
                style_ref: None,
                properties: (),
            },
            Paragraph {
                id: paragraph_id(3),
                story_id: story.id,
                range: TextRange::new(3, 6).expect("валидный range"),
                style_ref: None,
                properties: (),
            },
        ];

        let errors = validate_story_structure(&story, &paragraphs, &[] as &[TextRun<()>]);
        assert!(matches!(
            errors.as_slice(),
            [StoryStructureError::ParagraphOrderOrOverlap { .. }]
        ));
    }

    #[test]
    fn text_frame_and_story_remain_separate_types() {
        let frame = TextFrame {
            story_id: story_id(7),
            frame_flow: (),
            properties: (),
        };

        assert_eq!(frame.story_id, story_id(7));
    }

    #[test]
    fn image_resource_and_frame_keep_resource_identity_separate_from_placement() {
        let resource_id = ResourceId::from_canonical(id(8));
        let resource = ImageResource {
            id: resource_id,
            mime: "image/png".into(),
            source_hash: source_hash(),
            intrinsic_size_px: Some(PixelSize {
                width: 640,
                height: 480,
            }),
            color_profile: None::<ResourceId>,
            alpha: (),
            original_blob: "blob:sha256".to_string(),
        };
        let frame = ImageFrame {
            image_id: resource_id,
            properties: (),
        };

        assert_eq!(resource.id, frame.image_id);
        assert_eq!(
            resource.intrinsic_size_px,
            Some(PixelSize {
                width: 640,
                height: 480,
            })
        );
    }
}
