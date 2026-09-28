//! Source-neutral render-plan boundary for Chaptera document surfaces.
//!
//! The plan answers only what the current Viewer document intends to paint.
//! It deliberately contains no egui types, EditorSession state, parser-private
//! carrier names, source offsets, or mutable authoring commands.

use pub_model::{Affine2D, NodeId, PageId, RectEmu, ResourceId, Size2D, StoryId};
use pub_viewer::{ViewerGeometryDocument, ViewerTextFragment};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const PAGE_RENDER_PLAN_SCHEMA_V1: &str = "chaptera.page-render-plan.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRenderPlanV1 {
    pub schema_version: String,
    pub page_id: PageId,
    pub page_size: Size2D,
    pub nodes: Vec<NodeRenderPlanV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRenderPlanV1 {
    /// Semantic origin identity. Projected visuals must not clone a new NodeId.
    pub node_id: NodeId,
    /// Distinct visual identity for projected instances. Direct page-local nodes
    /// keep this empty and retain the existing NodeId-based path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual_instance_id: Option<String>,
    pub bounds: RectEmu,
    pub transform: Affine2D,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solid_fill_rgb: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solid_line: Option<RenderSolidLineV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<RenderImageRefV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<RenderTextFragmentV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderSolidLineV1 {
    pub rgb: [u8; 3],
    pub width_emu: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderImageRefV1 {
    pub resource_id: ResourceId,
    pub mime: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderTextFragmentV1 {
    pub story_id: StoryId,
    pub scalar_start: u32,
    pub scalar_end: u32,
    pub text: String,
    pub line_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub typography: Vec<RenderTypographyRunV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderTypographyRunV1 {
    pub scalar_start: u32,
    pub scalar_end: u32,
    pub source_font_name: String,
    pub text_size_emu: u32,
    pub font_inherited: bool,
    pub size_inherited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderPlanErrorV1 {
    PageIndexOutOfBounds { page_index: usize },
    PageSurfaceMissing { page_id: PageId },
}

impl fmt::Display for RenderPlanErrorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PageIndexOutOfBounds { page_index } => {
                write!(formatter, "viewer page index {page_index} is unavailable")
            }
            Self::PageSurfaceMissing { page_id } => {
                write!(formatter, "viewer page {page_id:?} has no resolved surface")
            }
        }
    }
}

impl std::error::Error for RenderPlanErrorV1 {}

fn suppress_projected_object_marker_glyphs(text: &str) -> String {
    // U+FFFC remains canonical Story authority. Once a projected visual owns
    // that slot, paint a zero-width scalar instead of a missing-glyph box.
    // Cardinality is preserved so scalar-aligned typography stays valid.
    text.chars()
        .map(|ch| if ch == '\u{FFFC}' { '\u{200B}' } else { ch })
        .collect()
}

fn render_typography_for_span(
    visual: &ViewerGeometryDocument,
    story_id: StoryId,
    scalar_start: u32,
    scalar_end: u32,
    current_story_text: &str,
) -> Vec<RenderTypographyRunV1> {
    let mut typography = visual
        .typography_runs
        .iter()
        .filter(|run| run.story_id == story_id)
        .filter(|run| run.applies_to_story_text(current_story_text))
        .filter_map(|run| {
            let scalar_start = run.scalar_start.max(scalar_start);
            let scalar_end = run.scalar_end.min(scalar_end);
            (scalar_start < scalar_end).then(|| RenderTypographyRunV1 {
                scalar_start,
                scalar_end,
                source_font_name: run.source_font_name.clone(),
                text_size_emu: run.text_size_emu,
                font_inherited: run.font_inherited,
                size_inherited: run.size_inherited,
            })
        })
        .collect::<Vec<_>>();
    typography.sort_by_key(|run| (run.scalar_start, run.scalar_end, run.text_size_emu));
    typography
}

fn render_text_fragment_v1(
    visual: &ViewerGeometryDocument,
    fragment: &ViewerTextFragment,
) -> RenderTextFragmentV1 {
    let current_story_text = visual
        .document
        .stories
        .iter()
        .find(|story| story.id == fragment.story_id)
        .map(|story| story.text.as_str());

    let typography = current_story_text
        .map(|text| {
            render_typography_for_span(
                visual,
                fragment.story_id,
                fragment.scalar_start,
                fragment.scalar_end,
                text,
            )
        })
        .unwrap_or_default();

    RenderTextFragmentV1 {
        story_id: fragment.story_id,
        scalar_start: fragment.scalar_start,
        scalar_end: fragment.scalar_end,
        text: fragment.text.clone(),
        line_count: fragment.line_count,
        typography,
    }
}

fn render_projected_story_v1(
    visual: &ViewerGeometryDocument,
    story_id: StoryId,
) -> Option<RenderTextFragmentV1> {
    let story = visual
        .document
        .stories
        .iter()
        .find(|story| story.id == story_id)?;
    let scalar_end = u32::try_from(story.text.chars().count()).ok()?;
    let typography = render_typography_for_span(visual, story_id, 0, scalar_end, &story.text);

    Some(RenderTextFragmentV1 {
        story_id,
        scalar_start: 0,
        scalar_end,
        text: story.text.clone(),
        // The desktop backend reflows source-neutral text from text+bounds.
        // This count is evidence-only and is intentionally recomputed at paint.
        line_count: 0,
        typography,
    })
}

fn render_node_plan_v1(
    visual: &ViewerGeometryDocument,
    node_id: NodeId,
    bounds: RectEmu,
    transform: Affine2D,
    visual_instance_id: Option<String>,
    text: Option<RenderTextFragmentV1>,
) -> NodeRenderPlanV1 {
    let paint = visual
        .paints
        .iter()
        .find(|paint| paint.node_id == node_id);
    let image = visual
        .images
        .iter()
        .find(|image| image.node_ids.contains(&node_id))
        .map(|image| RenderImageRefV1 {
            resource_id: image.resource_id,
            mime: image.mime.clone(),
        });

    NodeRenderPlanV1 {
        node_id,
        visual_instance_id,
        bounds,
        transform,
        solid_fill_rgb: paint.and_then(|paint| paint.solid_fill_rgb),
        solid_line: paint
            .and_then(|paint| paint.solid_line.as_ref())
            .map(|line| RenderSolidLineV1 {
                rgb: line.rgb,
                width_emu: line.width_emu,
            }),
        image,
        text,
    }
}

pub fn build_page_render_plan_v1(
    visual: &ViewerGeometryDocument,
    page_index: usize,
) -> Result<PageRenderPlanV1, RenderPlanErrorV1> {
    let page = visual
        .document
        .pages
        .get(page_index)
        .ok_or(RenderPlanErrorV1::PageIndexOutOfBounds { page_index })?;
    let surface = visual
        .scene
        .surfaces
        .iter()
        .find(|surface| surface.origin == page.id)
        .ok_or(RenderPlanErrorV1::PageSurfaceMissing { page_id: page.id })?;

    let parent_origin = page.id.into_canonical();
    let mut nodes = visual
        .scene
        .nodes
        .iter()
        .filter(|node| node.parent_origin == parent_origin)
        .map(|node| {
            let target_frame_has_projected_slot = visual.projected_instances.iter().any(|instance| {
                instance.target_page_id == page.id
                    && instance.target_frame_node_id == Some(node.origin)
            });
            let text = visual
                .text_fragments
                .iter()
                .find(|fragment| fragment.frame_id == node.origin)
                .map(|fragment| {
                    let mut rendered = render_text_fragment_v1(visual, fragment);
                    if target_frame_has_projected_slot {
                        rendered.text = suppress_projected_object_marker_glyphs(&rendered.text);
                    }
                    rendered
                });

            render_node_plan_v1(
                visual,
                node.origin,
                node.bounds,
                node.transform.clone(),
                None,
                text,
            )
        })
        .collect::<Vec<_>>();

    // Projected visuals retain semantic origin NodeId but receive distinct
    // visual identity and target geometry. Insert immediately after the target
    // frame when known so paint order follows slot ownership deterministically.
    for instance in visual
        .projected_instances
        .iter()
        .filter(|instance| instance.target_page_id == page.id)
    {
        let text = instance
            .story_authority_id
            .and_then(|story_id| render_projected_story_v1(visual, story_id));
        let projected = render_node_plan_v1(
            visual,
            instance.origin_node_id,
            instance.bounds,
            instance.transform.clone(),
            Some(instance.instance_id.clone()),
            text,
        );

        let insert_at = instance
            .target_frame_node_id
            .and_then(|target_frame_node_id| {
                nodes.iter().position(|node| {
                    node.visual_instance_id.is_none() && node.node_id == target_frame_node_id
                })
            })
            .map(|index| index + 1)
            .unwrap_or(nodes.len());
        nodes.insert(insert_at, projected);
    }

    Ok(PageRenderPlanV1 {
        schema_version: PAGE_RENDER_PLAN_SCHEMA_V1.to_owned(),
        page_id: page.id,
        page_size: surface.size,
        nodes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pub_layout::{
        BoundedLayoutEnvironment, BoundedResolvedScene, ResolvedPhysicalNode, ResolvedSurface,
    };
    use pub_model::{Affine2D, CanonicalId, LengthEmu, RectEmu, Sha256Digest, Size2D};
    use pub_viewer::{
        ViewerDocument, ViewerEmbeddedImage, ViewerNodePaint, ViewerPage,
        ViewerProjectedNodeInstanceV1, ViewerProjectionKindV1, ViewerSolidLine, ViewerSource,
        ViewerTextFragment, ViewerTypographyRun, viewer_story_text_sha256,
    };

    fn canonical(byte: u8) -> CanonicalId {
        CanonicalId::from_bytes([byte; 16])
    }

    fn fixture() -> ViewerGeometryDocument {
        let page_id = PageId::from_canonical(canonical(1));
        let node_id = NodeId::from_canonical(canonical(2));
        let story_id = StoryId::from_canonical(canonical(3));
        let resource_id = ResourceId::from_canonical(canonical(4));
        let page_size = Size2D::new(LengthEmu::new(1000), LengthEmu::new(2000));

        ViewerGeometryDocument {
            schema_version: "viewer.v1".into(),
            document: ViewerDocument {
                schema_version: "viewer.document.v1".into(),
                source: ViewerSource {
                    format: "pub".into(),
                    format_version: Some("0x2c".into()),
                    source_hash: Sha256Digest::from_bytes([0x11; 32]),
                    byte_len: 12,
                },
                pages: vec![ViewerPage {
                    index: 1,
                    id: page_id,
                    width_emu: 1000,
                    height_emu: 2000,
                }],
                stories: vec![pub_viewer::ViewerStory {
                    id: story_id,
                    text: "hello".into(),
                }],
                diagnostics: Vec::new(),
            },
            scene: BoundedResolvedScene {
                environment: BoundedLayoutEnvironment {
                    engine_revision: "test".into(),
                    font_set_fingerprint: "fonts:test".into(),
                    resource_fingerprint: "resources:test".into(),
                },
                surfaces: vec![ResolvedSurface {
                    origin: page_id,
                    size: page_size,
                    bleed: None,
                    margins: None,
                }],
                nodes: vec![ResolvedPhysicalNode {
                    origin: node_id,
                    parent_origin: page_id.into_canonical(),
                    bounds: RectEmu::new(
                        LengthEmu::new(10),
                        LengthEmu::new(20),
                        LengthEmu::new(300),
                        LengthEmu::new(400),
                    ),
                    transform: Affine2D::identity(),
                }],
                origin_mapping: Vec::new(),
                diagnostics: Vec::new(),
            },
            paints: vec![ViewerNodePaint {
                node_id,
                solid_fill_rgb: Some([1, 2, 3]),
                solid_line: Some(ViewerSolidLine {
                    rgb: [4, 5, 6],
                    width_emu: 12700,
                }),
            }],
            story_frames: Vec::new(),
            text_fragments: vec![ViewerTextFragment {
                story_id,
                frame_id: node_id,
                scalar_start: 0,
                scalar_end: 5,
                text: "hello".into(),
                line_count: 1,
            }],
            projected_instances: Vec::new(),
            typography_runs: vec![ViewerTypographyRun {
                story_id,
                scalar_start: 0,
                scalar_end: 2,
                source_font_name: "Source Font".into(),
                text_size_emu: 24 * 12_700,
                font_inherited: false,
                size_inherited: true,
                source_story_text_sha256: viewer_story_text_sha256("hello"),
            }],
            images: vec![ViewerEmbeddedImage {
                resource_id,
                mime: "image/png".into(),
                node_ids: vec![node_id],
                bytes: vec![0x89, b'P', b'N', b'G'],
            }],
        }
    }

    #[test]
    fn plan_collects_document_paint_facts_without_backend_state() {
        let visual = fixture();
        let plan = build_page_render_plan_v1(&visual, 0).expect("render plan");

        assert_eq!(plan.schema_version, PAGE_RENDER_PLAN_SCHEMA_V1);
        assert_eq!(plan.nodes.len(), 1);
        let node = &plan.nodes[0];
        assert!(node.visual_instance_id.is_none());
        assert_eq!(node.solid_fill_rgb, Some([1, 2, 3]));
        assert_eq!(
            node.solid_line.as_ref().map(|line| line.rgb),
            Some([4, 5, 6])
        );
        assert_eq!(
            node.image.as_ref().map(|image| image.mime.as_str()),
            Some("image/png")
        );
        assert_eq!(
            node.text.as_ref().map(|text| text.text.as_str()),
            Some("hello")
        );
        let typography = &node.text.as_ref().expect("text").typography;
        assert_eq!(typography.len(), 1);
        assert_eq!(typography[0].scalar_start, 0);
        assert_eq!(typography[0].scalar_end, 2);
        assert_eq!(typography[0].text_size_emu, 24 * 12_700);
        assert!(typography[0].size_inherited);
    }

    #[test]
    fn projected_instance_keeps_visual_identity_separate_from_semantic_origin() {
        let mut visual = fixture();
        let page_id = visual.document.pages[0].id;
        let origin_node_id = visual.scene.nodes[0].origin;
        let story_id = visual.document.stories[0].id;
        let projected_bounds = RectEmu::new(
            LengthEmu::new(50),
            LengthEmu::new(60),
            LengthEmu::new(700),
            LengthEmu::new(800),
        );
        visual.projected_instances.push(ViewerProjectedNodeInstanceV1 {
            instance_id: "sha256:projected-cmo-slot-test".to_owned(),
            projection_kind: ViewerProjectionKindV1::CmoStorySlot,
            origin_node_id,
            target_page_id: page_id,
            target_story_id: None,
            target_frame_node_id: None,
            scalar_index: None,
            source_order: None,
            cmo_id: None,
            bounds: projected_bounds,
            transform: Affine2D::identity(),
            story_authority_id: Some(story_id),
        });

        let plan = build_page_render_plan_v1(&visual, 0).expect("render plan");
        assert_eq!(plan.nodes.len(), 2);

        let projected = plan
            .nodes
            .iter()
            .find(|node| node.visual_instance_id.as_deref() == Some("sha256:projected-cmo-slot-test"))
            .expect("projected visual instance");

        assert_eq!(projected.node_id, origin_node_id);
        assert_eq!(projected.bounds, projected_bounds);
        assert_eq!(projected.solid_fill_rgb, Some([1, 2, 3]));
        assert_eq!(
            projected.text.as_ref().map(|text| text.text.as_str()),
            Some("hello")
        );
    }

    #[test]
    fn projected_slot_suppresses_only_object_marker_glyphs_and_keeps_target_text() {
        let mut visual = fixture();
        let page_id = visual.document.pages[0].id;
        let target_node_id = visual.scene.nodes[0].origin;
        let target_story_id = visual.document.stories[0].id;
        let source = "\u{FFFC}\r\u{FFFC}\r\u{FFFC}am.";
        visual.document.stories[0].text = source.to_owned();
        visual.text_fragments[0].text = source.to_owned();
        visual.text_fragments[0].scalar_end =
            u32::try_from(source.chars().count()).expect("bounded fixture");
        visual.projected_instances.push(ViewerProjectedNodeInstanceV1 {
            instance_id: "scene:cmo-story-slot:test".to_owned(),
            projection_kind: ViewerProjectionKindV1::CmoStorySlot,
            origin_node_id: target_node_id,
            target_page_id: page_id,
            target_story_id: Some(target_story_id),
            target_frame_node_id: Some(target_node_id),
            scalar_index: Some(0),
            source_order: Some(0),
            cmo_id: Some(7),
            bounds: RectEmu::new(
                LengthEmu::new(50),
                LengthEmu::new(60),
                LengthEmu::new(70),
                LengthEmu::new(80),
            ),
            transform: Affine2D::identity(),
            story_authority_id: None,
        });

        let plan = build_page_render_plan_v1(&visual, 0).expect("render plan");
        let target = plan
            .nodes
            .iter()
            .find(|node| node.visual_instance_id.is_none() && node.node_id == target_node_id)
            .expect("direct target frame");
        let rendered = &target.text.as_ref().expect("target text preserved").text;

        assert!(!rendered.contains('\u{FFFC}'));
        assert_eq!(rendered.chars().count(), source.chars().count());
        assert!(rendered.ends_with("am."));
        assert_eq!(rendered, &suppress_projected_object_marker_glyphs(source));
    }

    #[test]
    fn projected_instance_is_painted_immediately_after_its_target_frame() {
        let mut visual = fixture();
        let page_id = visual.document.pages[0].id;
        let target_node_id = visual.scene.nodes[0].origin;
        let target_story_id = visual.document.stories[0].id;
        visual.projected_instances.push(ViewerProjectedNodeInstanceV1 {
            instance_id: "scene:cmo-story-slot:order-test".to_owned(),
            projection_kind: ViewerProjectionKindV1::CmoStorySlot,
            origin_node_id: target_node_id,
            target_page_id: page_id,
            target_story_id: Some(target_story_id),
            target_frame_node_id: Some(target_node_id),
            scalar_index: Some(0),
            source_order: Some(0),
            cmo_id: Some(7),
            bounds: RectEmu::new(
                LengthEmu::new(50),
                LengthEmu::new(60),
                LengthEmu::new(70),
                LengthEmu::new(80),
            ),
            transform: Affine2D::identity(),
            story_authority_id: None,
        });

        let plan = build_page_render_plan_v1(&visual, 0).expect("render plan");
        assert_eq!(plan.nodes.len(), 2);
        assert!(plan.nodes[0].visual_instance_id.is_none());
        assert_eq!(
            plan.nodes[1].visual_instance_id.as_deref(),
            Some("scene:cmo-story-slot:order-test")
        );
    }

    #[test]
    fn serialized_plan_is_source_neutral_and_does_not_retain_image_bytes() {
        let plan = build_page_render_plan_v1(&fixture(), 0).expect("render plan");
        let json = serde_json::to_string(&plan).expect("serialize plan");

        for forbidden in [
            "Escher",
            "Quill",
            "Contents",
            "byte_range",
            "offset",
            "89504e47",
        ] {
            assert!(!json.contains(forbidden), "render plan leaked {forbidden}");
        }
        assert!(json.contains("image/png"));
        assert!(json.contains("hello"));
    }

    #[test]
    fn missing_page_is_typed_failure() {
        assert!(matches!(
            build_page_render_plan_v1(&fixture(), 1),
            Err(RenderPlanErrorV1::PageIndexOutOfBounds { page_index: 1 })
        ));
    }
}
