//! Source-neutral render-plan boundary for Chaptera document surfaces.
//!
//! The plan answers only what the current Viewer document intends to paint.
//! It deliberately contains no egui types, EditorSession state, parser-private
//! carrier names, source offsets, or mutable authoring commands.

use pub_layout::{
    BoundedLayoutEnvironment, BoundedLayoutProjection, BoundedShapedFlowRuntime,
    BoundedShapingRuntime, ProjectedNodeGeometry, ProjectedPage, ProjectedStory,
    ProjectedStoryFrame, font_fingerprint_sha256, resolve_bounded_shaped_flow,
};
use pub_model::{Affine2D, LengthEmu, NodeId, PageId, RectEmu, ResourceId, Size2D, StoryId};
use pub_viewer::ViewerGeometryDocument;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const PAGE_RENDER_PLAN_SCHEMA_V1: &str = "chaptera.page-render-plan.v1";
pub const SHARED_TEXT_LAYOUT_REVISION_V1: &str =
    "chaptera.viewer.shared-text-layout.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRenderPlanV1 {
    pub schema_version: String,
    pub page_id: PageId,
    pub page_size: Size2D,
    pub nodes: Vec<NodeRenderPlanV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRenderPlanV1 {
    pub node_id: NodeId,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<RenderTextLayoutV1>,
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


#[derive(Debug, Clone, Copy)]
pub struct ExplicitRenderTextFontResourceV1<'a> {
    pub resource_id: &'a str,
    pub expected_sha256: &'a str,
    pub face_index: u32,
    pub default_font_size_emu: i64,
    pub default_line_height_emu: i64,
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderTextLayoutV1 {
    pub disposition: RenderTextLayoutDispositionV1,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<RenderResolvedTextLineV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RenderTextLayoutDispositionV1 {
    SharedResolved {
        font_resource_id: String,
        font_fingerprint_sha256: String,
        font_size_emu: i64,
        line_height_emu: i64,
    },
    BackendFallback {
        reason: RenderTextLayoutFallbackReasonV1,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderTextLayoutFallbackReasonV1 {
    StoryMissing,
    StoryExtentMismatch,
    SingleFrameRequired,
    FrameGeometryInvalid,
    TypographyCoverageGap,
    MixedTypographySize,
    FontResourceInvalid,
    FontFingerprintMismatch,
    SharedLayoutFailed,
    SharedLayoutIncomplete,
}

impl RenderTextLayoutFallbackReasonV1 {
    pub const fn code(self) -> &'static str {
        match self {
            Self::StoryMissing => "story_missing",
            Self::StoryExtentMismatch => "story_extent_mismatch",
            Self::SingleFrameRequired => "single_frame_required",
            Self::FrameGeometryInvalid => "frame_geometry_invalid",
            Self::TypographyCoverageGap => "typography_coverage_gap",
            Self::MixedTypographySize => "mixed_typography_size",
            Self::FontResourceInvalid => "font_resource_invalid",
            Self::FontFingerprintMismatch => "font_fingerprint_mismatch",
            Self::SharedLayoutFailed => "shared_layout_failed",
            Self::SharedLayoutIncomplete => "shared_layout_incomplete",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderResolvedTextLineV1 {
    pub line_index: u32,
    pub scalar_start: u32,
    pub scalar_end: u32,
    pub consumed_scalar_end: u32,
    pub text: String,
    pub measured_width_emu: i64,
    pub line_height_emu: i64,
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
    let nodes = visual
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
                        layout: None,
                    }
                });

            NodeRenderPlanV1 {
                node_id: node.origin,
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
        .collect();

    Ok(PageRenderPlanV1 {
        schema_version: PAGE_RENDER_PLAN_SCHEMA_V1.to_owned(),
        page_id: page.id,
        page_size: surface.size,
        nodes,
    })
}


pub fn build_page_render_plan_with_text_layout_v1(
    visual: &ViewerGeometryDocument,
    page_index: usize,
    font: &ExplicitRenderTextFontResourceV1<'_>,
) -> Result<PageRenderPlanV1, RenderPlanErrorV1> {
    let mut plan = build_page_render_plan_v1(visual, page_index)?;
    let page_id = plan.page_id;
    let page_size = plan.page_size.clone();

    for node in &mut plan.nodes {
        let Some(fragment) = node.text.as_mut() else {
            continue;
        };
        fragment.layout = Some(resolve_text_layout_v1(
            visual,
            page_id,
            page_size.clone(),
            node.node_id,
            node.bounds.clone(),
            node.transform.clone(),
            fragment,
            font,
        ));
    }

    Ok(plan)
}

fn fallback_layout(reason: RenderTextLayoutFallbackReasonV1) -> RenderTextLayoutV1 {
    RenderTextLayoutV1 {
        disposition: RenderTextLayoutDispositionV1::BackendFallback { reason },
        lines: Vec::new(),
    }
}

fn resolve_text_layout_v1(
    visual: &ViewerGeometryDocument,
    page_id: PageId,
    page_size: Size2D,
    node_id: NodeId,
    bounds: RectEmu,
    transform: Affine2D,
    fragment: &RenderTextFragmentV1,
    font: &ExplicitRenderTextFontResourceV1<'_>,
) -> RenderTextLayoutV1 {
    let Some(story) = visual
        .document
        .stories
        .iter()
        .find(|story| story.id == fragment.story_id)
    else {
        return fallback_layout(RenderTextLayoutFallbackReasonV1::StoryMissing);
    };

    let Ok(story_scalar_len) = u32::try_from(story.text.chars().count()) else {
        return fallback_layout(RenderTextLayoutFallbackReasonV1::StoryExtentMismatch);
    };
    if fragment.scalar_start != 0
        || fragment.scalar_end != story_scalar_len
        || fragment.text != story.text
    {
        return fallback_layout(RenderTextLayoutFallbackReasonV1::StoryExtentMismatch);
    }

    let mut frames = visual
        .story_frames
        .iter()
        .filter(|frame| frame.story_id == fragment.story_id);
    let Some(frame) = frames.next() else {
        return fallback_layout(RenderTextLayoutFallbackReasonV1::SingleFrameRequired);
    };
    if frames.next().is_some() || frame.frame_id != node_id {
        return fallback_layout(RenderTextLayoutFallbackReasonV1::SingleFrameRequired);
    }

    if bounds.width.get() <= 0 || bounds.height.get() <= 0 {
        return fallback_layout(RenderTextLayoutFallbackReasonV1::FrameGeometryInvalid);
    }

    if font.resource_id.is_empty()
        || font.bytes.is_empty()
        || font.default_font_size_emu <= 0
        || font.default_line_height_emu <= 0
    {
        return fallback_layout(RenderTextLayoutFallbackReasonV1::FontResourceInvalid);
    }

    let fingerprint = font_fingerprint_sha256(font.bytes);
    if font.expected_sha256.is_empty() || fingerprint != font.expected_sha256 {
        return fallback_layout(RenderTextLayoutFallbackReasonV1::FontFingerprintMismatch);
    }

    let font_size_emu = match admitted_font_size_emu(fragment, font.default_font_size_emu) {
        Ok(size) => size,
        Err(reason) => return fallback_layout(reason),
    };
    let Some(line_height_emu) = scaled_line_height_emu(
        font_size_emu,
        font.default_font_size_emu,
        font.default_line_height_emu,
    ) else {
        return fallback_layout(RenderTextLayoutFallbackReasonV1::FontResourceInvalid);
    };

    let projection = BoundedLayoutProjection {
        pages: vec![ProjectedPage {
            origin: page_id,
            size: page_size,
            bleed: None,
            margins: None,
        }],
        node_geometry: vec![ProjectedNodeGeometry {
            origin: node_id,
            parent_origin: page_id.into_canonical(),
            bounds,
            transform,
        }],
        stories: vec![ProjectedStory {
            origin: story.id,
            text: story.text.clone(),
            paragraph_origins: Vec::new(),
            run_origins: Vec::new(),
        }],
        story_frames: vec![ProjectedStoryFrame {
            story_origin: story.id,
            frame_origin: node_id,
            ordinal: frame.ordinal,
            previous_frame_origin: None,
            next_frame_origin: None,
        }],
        tables: Vec::new(),
        guides: Vec::new(),
        diagnostics: Vec::new(),
    };

    let runtime = BoundedShapedFlowRuntime {
        shaping: BoundedShapingRuntime {
            layout: BoundedLayoutEnvironment {
                engine_revision: SHARED_TEXT_LAYOUT_REVISION_V1.to_owned(),
                font_set_fingerprint: fingerprint.clone(),
                resource_fingerprint: font.resource_id.to_owned(),
            },
            face_index: font.face_index,
            font_size_emu: LengthEmu::new(font_size_emu),
            font_bytes: font.bytes,
        },
        line_height: LengthEmu::new(line_height_emu),
    };

    let Ok(scene) = resolve_bounded_shaped_flow(&projection, &runtime) else {
        return fallback_layout(RenderTextLayoutFallbackReasonV1::SharedLayoutFailed);
    };

    let mut source_lines = scene
        .lines
        .into_iter()
        .filter(|line| line.story_origin == story.id && line.frame_origin == node_id)
        .collect::<Vec<_>>();
    source_lines.sort_by_key(|line| line.frame_line_index);

    if story_scalar_len > 0
        && source_lines
            .last()
            .map(|line| line.consumed_scalar_end)
            != Some(story_scalar_len)
    {
        return fallback_layout(RenderTextLayoutFallbackReasonV1::SharedLayoutIncomplete);
    }

    let lines = source_lines
        .into_iter()
        .map(|line| RenderResolvedTextLineV1 {
            line_index: line.frame_line_index,
            scalar_start: line.scalar_start,
            scalar_end: line.scalar_end,
            consumed_scalar_end: line.consumed_scalar_end,
            text: line.text,
            measured_width_emu: line.measured_width.get(),
            line_height_emu,
        })
        .collect();

    RenderTextLayoutV1 {
        disposition: RenderTextLayoutDispositionV1::SharedResolved {
            font_resource_id: font.resource_id.to_owned(),
            font_fingerprint_sha256: fingerprint,
            font_size_emu,
            line_height_emu,
        },
        lines,
    }
}

fn admitted_font_size_emu(
    fragment: &RenderTextFragmentV1,
    default_font_size_emu: i64,
) -> Result<i64, RenderTextLayoutFallbackReasonV1> {
    if fragment.typography.is_empty() {
        return (default_font_size_emu > 0)
            .then_some(default_font_size_emu)
            .ok_or(RenderTextLayoutFallbackReasonV1::FontResourceInvalid);
    }

    let mut cursor = fragment.scalar_start;
    let mut admitted_size = None;
    for run in &fragment.typography {
        if run.scalar_start != cursor
            || run.scalar_end <= run.scalar_start
            || run.scalar_end > fragment.scalar_end
            || run.text_size_emu == 0
        {
            return Err(RenderTextLayoutFallbackReasonV1::TypographyCoverageGap);
        }

        let size = i64::from(run.text_size_emu);
        match admitted_size {
            None => admitted_size = Some(size),
            Some(existing) if existing == size => {}
            Some(_) => return Err(RenderTextLayoutFallbackReasonV1::MixedTypographySize),
        }
        cursor = run.scalar_end;
    }

    if cursor != fragment.scalar_end {
        return Err(RenderTextLayoutFallbackReasonV1::TypographyCoverageGap);
    }
    admitted_size.ok_or(RenderTextLayoutFallbackReasonV1::TypographyCoverageGap)
}

fn scaled_line_height_emu(
    font_size_emu: i64,
    default_font_size_emu: i64,
    default_line_height_emu: i64,
) -> Option<i64> {
    if font_size_emu <= 0 || default_font_size_emu <= 0 || default_line_height_emu <= 0 {
        return None;
    }
    let numerator = i128::from(font_size_emu).checked_mul(i128::from(default_line_height_emu))?;
    let denominator = i128::from(default_font_size_emu);
    let rounded = numerator.checked_add(denominator / 2)?.checked_div(denominator)?;
    let value = i64::try_from(rounded).ok()?;
    (value > 0).then_some(value)
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
        ViewerSource, ViewerStoryFrame, ViewerTextFragment, ViewerTypographyRun,
        viewer_story_text_sha256,
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
            "LayoutJob",
            "egui",
        ] {
            assert!(!json.contains(forbidden), "render plan leaked {forbidden}");
        }
        assert!(json.contains("image/png"));
        assert!(json.contains("hello"));
    }

    fn layout_font<'a>(
        bytes: &'a [u8],
        expected_sha256: &'a str,
    ) -> ExplicitRenderTextFontResourceV1<'a> {
        ExplicitRenderTextFontResourceV1 {
            resource_id: "test.explicit.font.v1",
            expected_sha256,
            face_index: 0,
            default_font_size_emu: 10 * 12_700,
            default_line_height_emu: 12 * 12_700,
            bytes,
        }
    }

    fn single_frame_layout_fixture() -> ViewerGeometryDocument {
        let mut visual = fixture();
        let story_id = visual.document.stories[0].id;
        let node_id = visual.scene.nodes[0].origin;
        visual.scene.nodes[0].bounds = RectEmu::new(
            LengthEmu::new(10),
            LengthEmu::new(20),
            LengthEmu::new(1_200_000),
            LengthEmu::new(800_000),
        );
        visual.story_frames = vec![ViewerStoryFrame {
            story_id,
            frame_id: node_id,
            ordinal: 0,
        }];
        visual.typography_runs = vec![ViewerTypographyRun {
            story_id,
            scalar_start: 0,
            scalar_end: 5,
            source_font_name: "Source Font".into(),
            text_size_emu: 10 * 12_700,
            font_inherited: false,
            size_inherited: false,
            source_story_text_sha256: viewer_story_text_sha256("hello"),
        }];
        visual
    }

    #[test]
    fn shared_layout_is_deterministic_for_single_frame_homogeneous_story() {
        let visual = single_frame_layout_fixture();
        let bytes = font_test_data::NOTOSERIF_AUTOHINT_SHAPING;
        let fingerprint = font_fingerprint_sha256(bytes);
        let font = layout_font(bytes, &fingerprint);

        let first = build_page_render_plan_with_text_layout_v1(&visual, 0, &font)
            .expect("first shared layout plan");
        let second = build_page_render_plan_with_text_layout_v1(&visual, 0, &font)
            .expect("second shared layout plan");
        assert_eq!(first, second);

        let layout = first.nodes[0]
            .text
            .as_ref()
            .and_then(|text| text.layout.as_ref())
            .expect("text layout disposition");
        assert!(matches!(
            layout.disposition,
            RenderTextLayoutDispositionV1::SharedResolved { .. }
        ));
        assert!(!layout.lines.is_empty());
        assert_eq!(
            layout.lines.last().map(|line| line.consumed_scalar_end),
            Some(5)
        );
    }

    #[test]
    fn mixed_typography_fails_closed_to_backend_layout() {
        let mut visual = single_frame_layout_fixture();
        let story_id = visual.document.stories[0].id;
        visual.typography_runs = vec![
            ViewerTypographyRun {
                story_id,
                scalar_start: 0,
                scalar_end: 2,
                source_font_name: "A".into(),
                text_size_emu: 10 * 12_700,
                font_inherited: false,
                size_inherited: false,
                source_story_text_sha256: viewer_story_text_sha256("hello"),
            },
            ViewerTypographyRun {
                story_id,
                scalar_start: 2,
                scalar_end: 5,
                source_font_name: "B".into(),
                text_size_emu: 12 * 12_700,
                font_inherited: false,
                size_inherited: false,
                source_story_text_sha256: viewer_story_text_sha256("hello"),
            },
        ];
        let bytes = font_test_data::NOTOSERIF_AUTOHINT_SHAPING;
        let fingerprint = font_fingerprint_sha256(bytes);
        let font = layout_font(bytes, &fingerprint);

        let plan = build_page_render_plan_with_text_layout_v1(&visual, 0, &font)
            .expect("mixed typography plan");
        let layout = plan.nodes[0]
            .text
            .as_ref()
            .and_then(|text| text.layout.as_ref())
            .expect("fallback disposition");
        assert_eq!(
            layout.disposition,
            RenderTextLayoutDispositionV1::BackendFallback {
                reason: RenderTextLayoutFallbackReasonV1::MixedTypographySize
            }
        );
        assert!(layout.lines.is_empty());
    }

    #[test]
    fn font_fingerprint_mismatch_fails_closed() {
        let visual = single_frame_layout_fixture();
        let font = layout_font(font_test_data::NOTOSERIF_AUTOHINT_SHAPING, "00");

        let plan = build_page_render_plan_with_text_layout_v1(&visual, 0, &font)
            .expect("fingerprint mismatch remains a render-plan fallback");
        let layout = plan.nodes[0]
            .text
            .as_ref()
            .and_then(|text| text.layout.as_ref())
            .expect("fallback disposition");
        assert_eq!(
            layout.disposition,
            RenderTextLayoutDispositionV1::BackendFallback {
                reason: RenderTextLayoutFallbackReasonV1::FontFingerprintMismatch
            }
        );
    }


    #[test]
    fn missing_page_is_typed_failure() {
        assert!(matches!(
            build_page_render_plan_v1(&fixture(), 1),
            Err(RenderPlanErrorV1::PageIndexOutOfBounds { page_index: 1 })
        ));
    }
}
