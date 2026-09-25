use crate::{BoundedLayoutProjection, ProjectionDiagnostic, ProjectionSeverity};
use pub_model::{Affine2D, BoxEdges, CanonicalId, NodeId, PageId, RectEmu, Size2D, StoryId};
use serde::{Deserialize, Serialize};

/// Explicit environment fence for a bounded layout run.
///
/// This is intentionally small. It proves that derived physical output is
/// parameterized by declared environment state without freezing a universal
/// layout-environment registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedLayoutEnvironment {
    pub engine_revision: String,
    pub font_set_fingerprint: String,
    pub resource_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedResolvedScene {
    pub environment: BoundedLayoutEnvironment,
    pub surfaces: Vec<ResolvedSurface>,
    pub nodes: Vec<ResolvedPhysicalNode>,
    pub origin_mapping: Vec<SceneOriginMapping>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<ResolveDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedSurface {
    pub origin: PageId,
    pub size: Size2D,
    pub bleed: Option<BoxEdges>,
    pub margins: Option<BoxEdges>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedPhysicalNode {
    pub origin: NodeId,
    pub parent_origin: CanonicalId,
    pub bounds: RectEmu,
    pub transform: Affine2D,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneOriginMapping {
    pub authoring_origin: CanonicalId,
    pub resolved_node_origin: NodeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolveSeverity {
    FidelityWarning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveDiagnostic {
    pub code: String,
    pub severity: ResolveSeverity,
    pub origin: CanonicalId,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveBlocked {
    pub projection_errors: Vec<ProjectionDiagnostic>,
}

/// Resolves the geometry-only subset of the bounded projection.
///
/// This is LAYOUT-RESOLVE-01A, not a full text/layout runtime. Authored geometry
/// becomes a source-free physical scene; story text is explicitly reported as
/// unresolved rather than silently treated as laid out.
pub fn resolve_bounded_geometry(
    projection: &BoundedLayoutProjection,
    environment: BoundedLayoutEnvironment,
) -> Result<BoundedResolvedScene, ResolveBlocked> {
    let projection_errors: Vec<_> = projection
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == ProjectionSeverity::Error)
        .cloned()
        .collect();

    if !projection_errors.is_empty() {
        return Err(ResolveBlocked { projection_errors });
    }

    let mut surfaces: Vec<_> = projection
        .pages
        .iter()
        .map(|page| ResolvedSurface {
            origin: page.origin,
            size: page.size,
            bleed: page.bleed,
            margins: page.margins,
        })
        .collect();
    surfaces.sort_by_key(|surface| surface.origin);

    let mut nodes: Vec<_> = projection
        .node_geometry
        .iter()
        .map(|node| ResolvedPhysicalNode {
            origin: node.origin,
            parent_origin: node.parent_origin,
            bounds: node.bounds,
            transform: node.transform.clone(),
        })
        .collect();
    nodes.sort_by_key(|node| node.origin);

    let origin_mapping = nodes
        .iter()
        .map(|node| SceneOriginMapping {
            authoring_origin: node.origin.into_canonical(),
            resolved_node_origin: node.origin,
        })
        .collect();

    let diagnostics = unresolved_story_diagnostics(projection);

    Ok(BoundedResolvedScene {
        environment,
        surfaces,
        nodes,
        origin_mapping,
        diagnostics,
    })
}

fn unresolved_story_diagnostics(projection: &BoundedLayoutProjection) -> Vec<ResolveDiagnostic> {
    let mut stories: Vec<StoryId> = projection
        .stories
        .iter()
        .map(|story| story.origin)
        .collect();
    stories.sort();
    stories.dedup();

    stories
        .into_iter()
        .map(|story| ResolveDiagnostic {
            code: "story_text_layout_not_implemented".into(),
            severity: ResolveSeverity::FidelityWarning,
            origin: story.into_canonical(),
            message: "geometry-only resolver does not shape or flow story text".into(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BoundedAuthoringSlice, BoundedGuideInput, BoundedNodeGeometryInput, UnknownLayoutState,
        project_bounded,
    };
    use pub_model::{
        CanonicalId, EMU_PER_MILLIMETER, LengthEmu, Page, RulerGuide, RulerGuideAxis, Size2D,
        Story, StoryFrame,
    };

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

    fn environment() -> BoundedLayoutEnvironment {
        BoundedLayoutEnvironment {
            engine_revision: "layout-resolve-01a".into(),
            font_set_fingerprint: "fonts:none".into(),
            resource_fingerprint: "resources:none".into(),
        }
    }

    fn projection() -> BoundedLayoutProjection {
        project_bounded(BoundedAuthoringSlice {
            pages: vec![Page {
                id: page_id(1),
                size: Size2D::new(
                    LengthEmu::new(90 * EMU_PER_MILLIMETER),
                    LengthEmu::new(120 * EMU_PER_MILLIMETER),
                ),
                bleed: None,
                margins: None,
                children: vec![node_id(10)],
                extensions: Vec::new(),
            }],
            node_geometry: vec![BoundedNodeGeometryInput {
                node_id: node_id(10),
                parent_origin: page_id(1).into_canonical(),
                bounds: RectEmu::new(
                    LengthEmu::new(10 * EMU_PER_MILLIMETER),
                    LengthEmu::new(20 * EMU_PER_MILLIMETER),
                    LengthEmu::new(30 * EMU_PER_MILLIMETER),
                    LengthEmu::new(40 * EMU_PER_MILLIMETER),
                ),
                transform: Affine2D::identity(),
            }],
            stories: vec![Story {
                id: story_id(7),
                text: "Alpha beta gamma".into(),
                paragraphs: Vec::new(),
                runs: Vec::new(),
                fields: Vec::new(),
                hyperlinks: Vec::new(),
                source_refs: Vec::new(),
            }],
            story_frames: vec![StoryFrame {
                story_id: story_id(7),
                frame_id: node_id(10),
                ordinal: 0,
                previous: None,
                next: None,
            }],
            tables: Vec::new(),
            guides: vec![BoundedGuideInput {
                page_id: page_id(1),
                guide: RulerGuide {
                    axis: RulerGuideAxis::Vertical,
                    position: LengthEmu::new(12 * EMU_PER_MILLIMETER),
                },
                affects_layout: false,
            }],
            unknown_layout_state: Vec::new(),
        })
    }

    #[test]
    fn fixed_projection_and_environment_resolve_deterministically() {
        let projection = projection();

        let left = resolve_bounded_geometry(&projection, environment()).expect("valid projection");
        let right = resolve_bounded_geometry(&projection, environment()).expect("valid projection");

        assert_eq!(left, right);
    }

    #[test]
    fn resolved_scene_keeps_physical_geometry_and_origin_mapping() {
        let scene =
            resolve_bounded_geometry(&projection(), environment()).expect("valid projection");

        assert_eq!(scene.surfaces.len(), 1);
        assert_eq!(scene.nodes.len(), 1);
        assert_eq!(scene.nodes[0].origin, node_id(10));
        assert_eq!(
            scene.origin_mapping[0],
            SceneOriginMapping {
                authoring_origin: node_id(10).into_canonical(),
                resolved_node_origin: node_id(10),
            }
        );
    }

    #[test]
    fn unresolved_story_layout_is_explicit_not_silent() {
        let scene =
            resolve_bounded_geometry(&projection(), environment()).expect("valid projection");

        assert_eq!(scene.diagnostics.len(), 1);
        assert_eq!(
            scene.diagnostics[0].code,
            "story_text_layout_not_implemented"
        );
        assert_eq!(scene.diagnostics[0].origin, story_id(7).into_canonical());
    }

    #[test]
    fn projection_errors_block_resolution() {
        let mut projection = projection();
        projection.diagnostics.push(ProjectionDiagnostic {
            code: "synthetic_error".into(),
            severity: ProjectionSeverity::Error,
            origin: id(99),
            message: "do not resolve invalid projection".into(),
        });

        let error = resolve_bounded_geometry(&projection, environment())
            .expect_err("projection errors must block resolve");

        assert_eq!(error.projection_errors.len(), 1);
        assert_eq!(error.projection_errors[0].code, "synthetic_error");
    }

    #[test]
    fn resolved_scene_serialization_has_no_source_format_state() {
        let scene =
            resolve_bounded_geometry(&projection(), environment()).expect("valid projection");
        let json = serde_json::to_string(&scene).expect("scene should serialize");

        assert!(json.contains("origin_mapping"));
        assert!(!json.contains("source_refs"));
        assert!(!json.contains("byte_range"));
        assert!(!json.contains("private_state_ref"));
        assert!(!json.contains("quill"));
        assert!(!json.contains("contents"));
        assert!(!json.contains("escher"));
    }

    #[test]
    fn fidelity_warnings_do_not_block_geometry_resolution() {
        let projection = project_bounded(BoundedAuthoringSlice {
            pages: Vec::new(),
            node_geometry: Vec::new(),
            stories: Vec::new(),
            story_frames: Vec::new(),
            tables: Vec::new(),
            guides: Vec::new(),
            unknown_layout_state: vec![UnknownLayoutState {
                origin: id(90),
                description: "potentially visual unknown".into(),
            }],
        });

        assert!(resolve_bounded_geometry(&projection, environment()).is_ok());
    }
}
