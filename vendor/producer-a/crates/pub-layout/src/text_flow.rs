use crate::{
    BoundedLayoutEnvironment, BoundedLayoutProjection, ProjectionSeverity, ResolveBlocked,
    ResolveDiagnostic, ResolveSeverity, ResolvedPhysicalNode, ResolvedSurface, SceneOriginMapping,
    resolve_bounded_geometry,
};
use pub_model::{LengthEmu, NodeId, StoryId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedTextMetrics {
    pub font_fingerprint: String,
    pub scalar_advance: LengthEmu,
    pub line_height: LengthEmu,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedTextFlowEnvironment {
    pub layout: BoundedLayoutEnvironment,
    pub text_metrics: Option<BoundedTextMetrics>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedTextFragment {
    pub story_origin: StoryId,
    pub frame_origin: NodeId,
    pub scalar_start: u32,
    pub scalar_end: u32,
    pub text: String,
    pub line_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextFragmentOriginMapping {
    pub story_origin: StoryId,
    pub frame_origin: NodeId,
    pub scalar_start: u32,
    pub scalar_end: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedTextFlowScene {
    pub environment: BoundedTextFlowEnvironment,
    pub surfaces: Vec<ResolvedSurface>,
    pub nodes: Vec<ResolvedPhysicalNode>,
    pub text_fragments: Vec<ResolvedTextFragment>,
    pub origin_mapping: Vec<SceneOriginMapping>,
    pub text_origin_mapping: Vec<TextFragmentOriginMapping>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<ResolveDiagnostic>,
}

/// LAYOUT-RESOLVE-01B0: deterministic linked-story flow using explicit,
/// environment-fenced scalar metrics.
///
/// This deliberately does not claim real font shaping. It proves source-free
/// flow semantics, explicit-chain ownership, overset reporting and fragment
/// origin mapping before a shaping engine is introduced.
pub fn resolve_bounded_text_flow(
    projection: &BoundedLayoutProjection,
    environment: BoundedTextFlowEnvironment,
) -> Result<BoundedTextFlowScene, ResolveBlocked> {
    let projection_errors: Vec<_> = projection
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == ProjectionSeverity::Error)
        .cloned()
        .collect();

    if !projection_errors.is_empty() {
        return Err(ResolveBlocked { projection_errors });
    }

    let base = resolve_bounded_geometry(projection, environment.layout.clone())?;
    let mut diagnostics: Vec<_> = base
        .diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.code != "story_text_layout_not_implemented")
        .collect();

    let Some(metrics) = environment.text_metrics.as_ref() else {
        for story in &projection.stories {
            diagnostics.push(ResolveDiagnostic {
                code: "text_metrics_missing".into(),
                severity: ResolveSeverity::FidelityWarning,
                origin: story.origin.into_canonical(),
                message: "text flow requires explicit environment-fenced text metrics".into(),
            });
        }

        return Ok(BoundedTextFlowScene {
            environment,
            surfaces: base.surfaces,
            nodes: base.nodes,
            text_fragments: Vec::new(),
            origin_mapping: base.origin_mapping,
            text_origin_mapping: Vec::new(),
            diagnostics,
        });
    };

    if metrics.font_fingerprint != environment.layout.font_set_fingerprint {
        for story in &projection.stories {
            diagnostics.push(ResolveDiagnostic {
                code: "text_metrics_font_mismatch".into(),
                severity: ResolveSeverity::FidelityWarning,
                origin: story.origin.into_canonical(),
                message: "text metrics fingerprint does not match the declared font set".into(),
            });
        }

        return Ok(BoundedTextFlowScene {
            environment,
            surfaces: base.surfaces,
            nodes: base.nodes,
            text_fragments: Vec::new(),
            origin_mapping: base.origin_mapping,
            text_origin_mapping: Vec::new(),
            diagnostics,
        });
    }

    if metrics.scalar_advance.get() <= 0 || metrics.line_height.get() <= 0 {
        for story in &projection.stories {
            diagnostics.push(ResolveDiagnostic {
                code: "invalid_text_metrics".into(),
                severity: ResolveSeverity::FidelityWarning,
                origin: story.origin.into_canonical(),
                message: "text metrics must use positive scalar advance and line height".into(),
            });
        }

        return Ok(BoundedTextFlowScene {
            environment,
            surfaces: base.surfaces,
            nodes: base.nodes,
            text_fragments: Vec::new(),
            origin_mapping: base.origin_mapping,
            text_origin_mapping: Vec::new(),
            diagnostics,
        });
    }

    let geometry: BTreeMap<_, _> = projection
        .node_geometry
        .iter()
        .map(|node| (node.origin, node.bounds))
        .collect();

    let mut text_fragments = Vec::new();
    let mut text_origin_mapping = Vec::new();

    for story in &projection.stories {
        let frames: Vec<_> = projection
            .story_frames
            .iter()
            .filter(|frame| frame.story_origin == story.origin)
            .collect();

        if frames.is_empty() {
            diagnostics.push(ResolveDiagnostic {
                code: "story_has_no_frames".into(),
                severity: ResolveSeverity::FidelityWarning,
                origin: story.origin.into_canonical(),
                message: "story content has no projected text frame".into(),
            });
            continue;
        }

        let chain = match explicit_chain(&frames) {
            Ok(chain) => chain,
            Err(code) => {
                diagnostics.push(ResolveDiagnostic {
                    code: code.into(),
                    severity: ResolveSeverity::FidelityWarning,
                    origin: story.origin.into_canonical(),
                    message: "story flow is not a single explicit linked-frame chain".into(),
                });
                continue;
            }
        };

        let scalars: Vec<char> = story.text.chars().collect();
        let mut cursor = 0usize;

        for frame_origin in chain {
            let Some(bounds) = geometry.get(&frame_origin) else {
                diagnostics.push(ResolveDiagnostic {
                    code: "text_frame_geometry_missing".into(),
                    severity: ResolveSeverity::FidelityWarning,
                    origin: frame_origin.into_canonical(),
                    message: "text frame has no geometry available to the resolver".into(),
                });
                continue;
            };

            let columns = bounds.width.get() / metrics.scalar_advance.get();
            let rows = bounds.height.get() / metrics.line_height.get();
            if columns <= 0 || rows <= 0 {
                diagnostics.push(ResolveDiagnostic {
                    code: "text_frame_has_no_capacity".into(),
                    severity: ResolveSeverity::FidelityWarning,
                    origin: frame_origin.into_canonical(),
                    message: "text frame geometry cannot fit one measured scalar".into(),
                });
                continue;
            }

            let capacity = usize::try_from(columns)
                .ok()
                .and_then(|columns| {
                    usize::try_from(rows)
                        .ok()
                        .and_then(|rows| columns.checked_mul(rows))
                })
                .unwrap_or(usize::MAX);

            let end = cursor.saturating_add(capacity).min(scalars.len());
            if end == cursor {
                continue;
            }

            let text: String = scalars[cursor..end].iter().collect();
            let scalar_start = u32::try_from(cursor).unwrap_or(u32::MAX);
            let scalar_end = u32::try_from(end).unwrap_or(u32::MAX);
            let line_count = u32::try_from(
                (end - cursor).div_ceil(usize::try_from(columns).unwrap_or(usize::MAX)),
            )
            .unwrap_or(u32::MAX);

            text_fragments.push(ResolvedTextFragment {
                story_origin: story.origin,
                frame_origin,
                scalar_start,
                scalar_end,
                text,
                line_count,
            });
            text_origin_mapping.push(TextFragmentOriginMapping {
                story_origin: story.origin,
                frame_origin,
                scalar_start,
                scalar_end,
            });

            cursor = end;
            if cursor == scalars.len() {
                break;
            }
        }

        if cursor < scalars.len() {
            diagnostics.push(ResolveDiagnostic {
                code: "story_overset".into(),
                severity: ResolveSeverity::FidelityWarning,
                origin: story.origin.into_canonical(),
                message: format!(
                    "{} measured scalars remain after the explicit frame chain",
                    scalars.len() - cursor
                ),
            });
        }
    }

    text_fragments.sort_by_key(|fragment| {
        (
            fragment.story_origin,
            fragment.scalar_start,
            fragment.frame_origin,
        )
    });
    text_origin_mapping.sort_by_key(|mapping| {
        (
            mapping.story_origin,
            mapping.scalar_start,
            mapping.frame_origin,
        )
    });

    Ok(BoundedTextFlowScene {
        environment,
        surfaces: base.surfaces,
        nodes: base.nodes,
        text_fragments,
        origin_mapping: base.origin_mapping,
        text_origin_mapping,
        diagnostics,
    })
}

pub(crate) fn explicit_chain(
    frames: &[&crate::ProjectedStoryFrame],
) -> Result<Vec<NodeId>, &'static str> {
    if frames.len() == 1 {
        return Ok(vec![frames[0].frame_origin]);
    }

    if frames
        .iter()
        .all(|frame| frame.previous_frame_origin.is_none() && frame.next_frame_origin.is_none())
    {
        return Err("shared_story_without_explicit_flow");
    }

    let by_id: BTreeMap<_, _> = frames
        .iter()
        .map(|frame| (frame.frame_origin, *frame))
        .collect();
    let heads: Vec<_> = frames
        .iter()
        .filter(|frame| frame.previous_frame_origin.is_none())
        .collect();

    if heads.len() != 1 {
        return Err("ambiguous_story_flow");
    }

    let mut chain = Vec::with_capacity(frames.len());
    let mut seen = BTreeSet::new();
    let mut current = heads[0].frame_origin;

    loop {
        if !seen.insert(current) {
            return Err("cyclic_story_flow");
        }
        chain.push(current);

        let Some(frame) = by_id.get(&current) else {
            return Err("broken_story_flow");
        };
        let Some(next) = frame.next_frame_origin else {
            break;
        };

        let Some(next_frame) = by_id.get(&next) else {
            return Err("broken_story_flow");
        };
        if next_frame.previous_frame_origin != Some(current) {
            return Err("non_reciprocal_story_flow");
        }

        current = next;
    }

    if chain.len() != frames.len() {
        return Err("disconnected_story_flow");
    }

    Ok(chain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BoundedAuthoringSlice, BoundedNodeGeometryInput, ProjectedStoryFrame, project_bounded,
    };
    use pub_model::{
        Affine2D, CanonicalId, LengthEmu, NodeId, Page, PageId, RectEmu, Size2D, Story, StoryFrame,
        StoryId,
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

    fn environment() -> BoundedTextFlowEnvironment {
        BoundedTextFlowEnvironment {
            layout: BoundedLayoutEnvironment {
                engine_revision: "layout-resolve-01b0".into(),
                font_set_fingerprint: "font-fixture-v1".into(),
                resource_fingerprint: "resources:none".into(),
            },
            text_metrics: Some(BoundedTextMetrics {
                font_fingerprint: "font-fixture-v1".into(),
                scalar_advance: LengthEmu::new(10),
                line_height: LengthEmu::new(10),
            }),
        }
    }

    fn projection(linked: bool, text: &str) -> BoundedLayoutProjection {
        let first = StoryFrame {
            story_id: story_id(7),
            frame_id: node_id(10),
            ordinal: 0,
            previous: None,
            next: linked.then_some(node_id(11)),
        };
        let second = StoryFrame {
            story_id: story_id(7),
            frame_id: node_id(11),
            ordinal: 1,
            previous: linked.then_some(node_id(10)),
            next: None,
        };

        project_bounded(BoundedAuthoringSlice {
            pages: vec![Page {
                id: page_id(1),
                size: Size2D::new(LengthEmu::new(500), LengthEmu::new(500)),
                bleed: None,
                margins: None,
                children: vec![node_id(10), node_id(11)],
                extensions: Vec::new(),
            }],
            node_geometry: vec![
                BoundedNodeGeometryInput {
                    node_id: node_id(10),
                    parent_origin: page_id(1).into_canonical(),
                    bounds: RectEmu::new(
                        LengthEmu::ZERO,
                        LengthEmu::ZERO,
                        LengthEmu::new(30),
                        LengthEmu::new(20),
                    ),
                    transform: Affine2D::identity(),
                },
                BoundedNodeGeometryInput {
                    node_id: node_id(11),
                    parent_origin: page_id(1).into_canonical(),
                    bounds: RectEmu::new(
                        LengthEmu::new(40),
                        LengthEmu::ZERO,
                        LengthEmu::new(30),
                        LengthEmu::new(20),
                    ),
                    transform: Affine2D::identity(),
                },
            ],
            stories: vec![Story {
                id: story_id(7),
                text: text.into(),
                paragraphs: Vec::new(),
                runs: Vec::new(),
                fields: Vec::new(),
                hyperlinks: Vec::new(),
                source_refs: Vec::new(),
            }],
            story_frames: vec![second, first],
            tables: Vec::new(),
            guides: Vec::new(),
            unknown_layout_state: Vec::new(),
        })
    }

    #[test]
    fn explicit_links_control_deterministic_story_flow() {
        let scene = resolve_bounded_text_flow(&projection(true, "ABCDEFGHIJ"), environment())
            .expect("projection should resolve");

        assert_eq!(scene.text_fragments.len(), 2);
        assert_eq!(scene.text_fragments[0].frame_origin, node_id(10));
        assert_eq!(scene.text_fragments[0].text, "ABCDEF");
        assert_eq!(scene.text_fragments[1].frame_origin, node_id(11));
        assert_eq!(scene.text_fragments[1].text, "GHIJ");
        assert!(
            !scene
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == "story_overset" })
        );
    }

    #[test]
    fn shared_story_without_links_is_not_silently_chained() {
        let scene = resolve_bounded_text_flow(&projection(false, "ABCDEFG"), environment())
            .expect("projection should resolve");

        assert!(scene.text_fragments.is_empty());
        assert!(
            scene
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == "shared_story_without_explicit_flow" })
        );
    }

    #[test]
    fn overset_is_explicit() {
        let scene = resolve_bounded_text_flow(&projection(true, "ABCDEFGHIJKLM"), environment())
            .expect("projection should resolve");

        assert_eq!(scene.text_fragments.len(), 2);
        assert!(
            scene
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == "story_overset" })
        );
    }

    #[test]
    fn missing_metrics_is_explicit() {
        let mut environment = environment();
        environment.text_metrics = None;

        let scene = resolve_bounded_text_flow(&projection(true, "ABC"), environment)
            .expect("projection should resolve");

        assert!(scene.text_fragments.is_empty());
        assert!(
            scene
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == "text_metrics_missing" })
        );
    }

    #[test]
    fn fixed_input_and_environment_are_deterministic() {
        let projection = projection(true, "ABCDEFGHIJ");
        let left = resolve_bounded_text_flow(&projection, environment()).unwrap();
        let right = resolve_bounded_text_flow(&projection, environment()).unwrap();

        assert_eq!(left, right);
    }

    #[test]
    fn explicit_chain_helper_rejects_ordinal_only_flow() {
        let first = ProjectedStoryFrame {
            story_origin: story_id(1),
            frame_origin: node_id(1),
            ordinal: 0,
            previous_frame_origin: None,
            next_frame_origin: None,
        };
        let second = ProjectedStoryFrame {
            story_origin: story_id(1),
            frame_origin: node_id(2),
            ordinal: 1,
            previous_frame_origin: None,
            next_frame_origin: None,
        };

        assert_eq!(
            explicit_chain(&[&first, &second]),
            Err("shared_story_without_explicit_flow")
        );
    }
}
