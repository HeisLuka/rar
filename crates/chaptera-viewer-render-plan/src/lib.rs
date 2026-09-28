//! Source-neutral render-plan boundary for Chaptera document surfaces.
//!
//! The plan answers only what the current Viewer document intends to paint.
//! It deliberately contains no egui types, EditorSession state, parser-private
//! carrier names, source offsets, or mutable authoring commands.

#[cfg(feature = "projected-scene-instances")]
use chaptera_scene_instance::SceneInstanceV1;
#[cfg(feature = "projected-scene-instances")]
use pub_model::CanonicalId;
use pub_model::{Affine2D, NodeId, PageId, RectEmu, ResourceId, Size2D, StoryId};
use pub_viewer::ViewerGeometryDocument;
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
    /// Semantic origin identity. Projected visuals retain this origin and carry
    /// their separate canonical visual authority in `scene_instance`.
    pub node_id: NodeId,
    #[cfg(feature = "projected-scene-instances")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_instance: Option<SceneInstanceV1>,
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
    #[cfg(feature = "projected-scene-instances")]
    ProjectedInstanceIdentityInvalid { field: &'static str },
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
            #[cfg(feature = "projected-scene-instances")]
            Self::ProjectedInstanceIdentityInvalid { field } => {
                write!(formatter, "projected scene instance carries invalid {field}")
            }
        }
    }
}

impl std::error::Error for RenderPlanErrorV1 {}

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
            let paint = visual
                .paints
                .iter()
                .find(|paint| paint.node_id == node.origin);
            let image = visual
                .images
                .iter()
                .find(|image| image.node_ids.contains(&node.origin))
                .map(|image| RenderImageRefV1 {
                    resource_id: image.resource_id,
                    mime: image.mime.clone(),
                });
            let text = visual
                .text_fragments
                .iter()
                .find(|fragment| fragment.frame_id == node.origin)
                .map(|fragment| {
                    let current_story_text = visual
                        .document
                        .stories
                        .iter()
                        .find(|story| story.id == fragment.story_id)
                        .map(|story| story.text.as_str());
                    let mut typography = visual
                        .typography_runs
                        .iter()
                        .filter(|run| run.story_id == fragment.story_id)
                        .filter(|run| {
                            current_story_text
                                .is_some_and(|text| run.applies_to_story_text(text))
                        })
                        .filter_map(|run| {
                            let scalar_start = run.scalar_start.max(fragment.scalar_start);
                            let scalar_end = run.scalar_end.min(fragment.scalar_end);
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

                    RenderTextFragmentV1 {
                        story_id: fragment.story_id,
                        scalar_start: fragment.scalar_start,
                        scalar_end: fragment.scalar_end,
                        text: fragment.text.clone(),
                        line_count: fragment.line_count,
                        typography,
                    }
                });

            NodeRenderPlanV1 {
                node_id: node.origin,
                #[cfg(feature = "projected-scene-instances")]
                scene_instance: None,
                bounds: node.bounds,
                transform: node.transform.clone(),
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
        })
        .collect::<Vec<_>>();

    #[cfg(feature = "projected-scene-instances")]
    {
        for projected in &visual.projected_instances {
            let target_page = projected
                .scene_instance
                .target_page_id
                .parse::<CanonicalId>()
                .map(PageId::from_canonical)
                .map_err(|_| RenderPlanErrorV1::ProjectedInstanceIdentityInvalid {
                    field: "target_page_id",
                })?;
            if target_page != page.id {
                continue;
            }

            let origin_node_id = projected
                .scene_instance
                .origin_node_id
                .parse::<CanonicalId>()
                .map(NodeId::from_canonical)
                .map_err(|_| RenderPlanErrorV1::ProjectedInstanceIdentityInvalid {
                    field: "origin_node_id",
                })?;

            let story_authority_id = projected
                .scene_instance
                .story_authority_id
                .as_deref()
                .map(|value| {
                    value
                        .parse::<CanonicalId>()
                        .map(StoryId::from_canonical)
                        .map_err(|_| RenderPlanErrorV1::ProjectedInstanceIdentityInvalid {
                            field: "story_authority_id",
                        })
                })
                .transpose()?;

            let paint = visual
                .paints
                .iter()
                .find(|paint| paint.node_id == origin_node_id);
            let image = visual
                .images
                .iter()
                .find(|image| image.node_ids.contains(&origin_node_id))
                .map(|image| RenderImageRefV1 {
                    resource_id: image.resource_id,
                    mime: image.mime.clone(),
                });
            let text = story_authority_id.and_then(|story_id| {
                let story = visual
                    .document
                    .stories
                    .iter()
                    .find(|story| story.id == story_id)?;
                let scalar_end = u32::try_from(story.text.chars().count()).ok()?;
                let mut typography = visual
                    .typography_runs
                    .iter()
                    .filter(|run| run.story_id == story_id)
                    .filter(|run| run.applies_to_story_text(&story.text))
                    .filter_map(|run| {
                        let scalar_start = run.scalar_start;
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
                Some(RenderTextFragmentV1 {
                    story_id,
                    scalar_start: 0,
                    scalar_end,
                    text: story.text.clone(),
                    line_count: 0,
                    typography,
                })
            });

            nodes.push(NodeRenderPlanV1 {
                node_id: origin_node_id,
                scene_instance: Some(projected.scene_instance.clone()),
                bounds: projected.bounds,
                transform: projected.transform.clone(),
                solid_fill_rgb: paint.and_then(|paint| paint.solid_fill_rgb),
                solid_line: paint
                    .and_then(|paint| paint.solid_line.as_ref())
                    .map(|line| RenderSolidLineV1 {
                        rgb: line.rgb,
                        width_emu: line.width_emu,
                    }),
                image,
                text,
            });
        }
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
        ViewerDocument, ViewerEmbeddedImage, ViewerNodePaint, ViewerPage, ViewerSolidLine,
        ViewerSource, ViewerTextFragment, ViewerTypographyRun, viewer_story_text_sha256,
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
            #[cfg(feature = "projected-scene-instances")]
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
        #[cfg(feature = "projected-scene-instances")]
        assert!(node.scene_instance.is_none());
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

    #[cfg(feature = "projected-scene-instances")]
    #[test]
    fn projected_instance_preserves_canonical_scene_authority() {
        use chaptera_scene_instance::{
            SCENE_INSTANCE_SCHEMA_V1, SceneInstanceV1, SceneProjectionKindV1,
        };
        use pub_viewer::ViewerProjectedSceneInstanceV1;

        let mut visual = fixture();
        let page_id = visual.document.pages[0].id;
        let origin_node_id = visual.scene.nodes[0].origin;
        let story_id = visual.document.stories[0].id;
        let canonical_instance = SceneInstanceV1 {
            schema_version: SCENE_INSTANCE_SCHEMA_V1.to_owned(),
            instance_id: "sha256:canonical-scene-instance-test".to_owned(),
            projection_kind: SceneProjectionKindV1::CmoStorySlot,
            origin_node_id: origin_node_id.as_canonical().to_string(),
            target_page_id: page_id.as_canonical().to_string(),
            source_parent_origin: None,
            story_authority_id: Some(story_id.as_canonical().to_string()),
            cmo_slot_index: Some(0),
            cmo_scalar_index: Some(0),
        };
        let projected_bounds = RectEmu::new(
            LengthEmu::new(50),
            LengthEmu::new(60),
            LengthEmu::new(700),
            LengthEmu::new(800),
        );
        visual.projected_instances.push(ViewerProjectedSceneInstanceV1 {
            scene_instance: canonical_instance.clone(),
            bounds: projected_bounds,
            transform: Affine2D::identity(),
            target_story_id: None,
            target_frame_node_id: None,
        });

        let plan = build_page_render_plan_v1(&visual, 0).expect("render plan");
        assert_eq!(plan.nodes.len(), 2);
        let projected = plan
            .nodes
            .iter()
            .find(|node| node.scene_instance.is_some())
            .expect("projected scene instance");
        assert_eq!(projected.node_id, origin_node_id);
        assert_eq!(projected.bounds, projected_bounds);
        assert_eq!(projected.scene_instance.as_ref(), Some(&canonical_instance));
        assert_eq!(
            projected.text.as_ref().map(|text| text.text.as_str()),
            Some("hello")
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
