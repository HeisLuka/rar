use crate::{OdgPackage, OdgPart, OdgPartKind};
use pub_export::{CapabilityLevel, ExportPlan};
use pub_model::{CanonicalId, NodeId, PageId, RectEmu, ResourceId};
use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Write as _;

pub const IMAGE_BYTES_FEATURE: &str = "image.bytes";
pub const IMAGE_FRAME_GEOMETRY_FEATURE: &str = "image.frame_geometry";
pub const IMAGE_CONTENT_TRANSFORM_FEATURE: &str = "image.content_transform";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OdgEmbeddedImagePlacement {
    pub node_id: NodeId,
    pub page_id: PageId,
    pub resource_id: ResourceId,
    pub frame_bounds: RectEmu,
    pub z_index: usize,
    pub mime: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OdgImageError {
    NonOdgPackage,
    EmptyPayload { resource_id: ResourceId },
    UnsupportedMime { resource_id: ResourceId, mime: String },
    InvalidFrameBounds { node_id: NodeId },
    DuplicatePlacement { node_id: NodeId },
    MissingPreservedBytes { resource_id: ResourceId },
    MissingPreservedFrameGeometry { node_id: NodeId },
    MissingContentTransformLoss { node_id: NodeId },
    MissingContent,
    InvalidContentUtf8,
    MissingPage { page_id: PageId },
    DuplicateResourcePath { path: String },
}

impl fmt::Display for OdgImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonOdgPackage => formatter.write_str("image projection requires an ODG package"),
            Self::EmptyPayload { resource_id } => write!(
                formatter,
                "image resource {} has an empty exact payload",
                resource_id.as_canonical()
            ),
            Self::UnsupportedMime { resource_id, mime } => write!(
                formatter,
                "image resource {} has unsupported MIME {mime}",
                resource_id.as_canonical()
            ),
            Self::InvalidFrameBounds { node_id } => write!(
                formatter,
                "image frame {} has non-positive or overflowing bounds",
                node_id.as_canonical()
            ),
            Self::DuplicatePlacement { node_id } => write!(
                formatter,
                "duplicate image placement for node {}",
                node_id.as_canonical()
            ),
            Self::MissingPreservedBytes { resource_id } => write!(
                formatter,
                "image resource {} is not covered by preserved image.bytes",
                resource_id.as_canonical()
            ),
            Self::MissingPreservedFrameGeometry { node_id } => write!(
                formatter,
                "image frame {} is not covered by preserved image.frame_geometry",
                node_id.as_canonical()
            ),
            Self::MissingContentTransformLoss { node_id } => write!(
                formatter,
                "image frame {} lacks explicit image.content_transform loss",
                node_id.as_canonical()
            ),
            Self::MissingContent => formatter.write_str("ODG package is missing content.xml"),
            Self::InvalidContentUtf8 => formatter.write_str("ODG content.xml is not UTF-8"),
            Self::MissingPage { page_id } => write!(
                formatter,
                "ODG content.xml is missing page {}",
                page_id.as_canonical()
            ),
            Self::DuplicateResourcePath { path } => {
                write!(formatter, "ODG package already contains resource path {path}")
            }
        }
    }
}

impl std::error::Error for OdgImageError {}

pub fn add_embedded_images_to_odg(
    plan: &ExportPlan,
    package: &mut OdgPackage,
    placements: &[OdgEmbeddedImagePlacement],
) -> Result<(), OdgImageError> {
    if package.target.format != "odg" || plan.target.format != "odg" {
        return Err(OdgImageError::NonOdgPackage);
    }

    let mut ordered = placements.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|placement| {
        (
            placement.page_id,
            placement.z_index,
            placement.node_id,
            placement.resource_id,
        )
    });

    let mut seen_nodes = BTreeSet::new();
    let mut resource_paths = BTreeSet::new();
    let mut resources = Vec::new();

    let content_index = package
        .parts
        .iter()
        .position(|part| part.kind == OdgPartKind::Content && part.path == crate::ODG_CONTENT_PATH)
        .ok_or(OdgImageError::MissingContent)?;
    let mut xml = String::from_utf8(package.parts[content_index].content.clone())
        .map_err(|_| OdgImageError::InvalidContentUtf8)?;

    for placement in ordered {
        if !seen_nodes.insert(placement.node_id) {
            return Err(OdgImageError::DuplicatePlacement {
                node_id: placement.node_id,
            });
        }
        validate_placement(plan, placement)?;

        let resource_path = resource_path(placement);
        if package.parts.iter().any(|part| part.path == resource_path) {
            return Err(OdgImageError::DuplicateResourcePath {
                path: resource_path,
            });
        }
        if resource_paths.insert(resource_path.clone()) {
            resources.push(OdgPart {
                path: resource_path.clone(),
                kind: OdgPartKind::Resource,
                media_type: placement.mime.clone(),
                content: placement.bytes.clone(),
            });
        }

        let page_name = crate::semantic::page_name(placement.page_id);
        let page_marker = format!("<draw:page draw:name=\"{page_name}\"");
        let page_start = xml
            .find(&page_marker)
            .ok_or(OdgImageError::MissingPage {
                page_id: placement.page_id,
            })?;
        let relative_close = xml[page_start..]
            .find("      </draw:page>")
            .ok_or(OdgImageError::MissingPage {
                page_id: placement.page_id,
            })?;
        let insert_at = page_start + relative_close;
        xml.insert_str(insert_at, &image_frame_xml(placement, &resource_path)?);
    }

    package.parts[content_index].content = xml.into_bytes();
    package.parts.extend(resources);
    package.parts.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(())
}

fn validate_placement(
    plan: &ExportPlan,
    placement: &OdgEmbeddedImagePlacement,
) -> Result<(), OdgImageError> {
    if placement.bytes.is_empty() {
        return Err(OdgImageError::EmptyPayload {
            resource_id: placement.resource_id,
        });
    }
    resource_extension(&placement.mime).ok_or_else(|| OdgImageError::UnsupportedMime {
        resource_id: placement.resource_id,
        mime: placement.mime.clone(),
    })?;
    if placement.frame_bounds.width.get() <= 0
        || placement.frame_bounds.height.get() <= 0
        || placement.frame_bounds.right().is_none()
        || placement.frame_bounds.bottom().is_none()
    {
        return Err(OdgImageError::InvalidFrameBounds {
            node_id: placement.node_id,
        });
    }

    if !has_preserved_feature(
        plan,
        placement.resource_id.into_canonical(),
        IMAGE_BYTES_FEATURE,
    ) {
        return Err(OdgImageError::MissingPreservedBytes {
            resource_id: placement.resource_id,
        });
    }
    if !has_preserved_feature(
        plan,
        placement.node_id.into_canonical(),
        IMAGE_FRAME_GEOMETRY_FEATURE,
    ) {
        return Err(OdgImageError::MissingPreservedFrameGeometry {
            node_id: placement.node_id,
        });
    }
    if !has_reported_feature_loss(
        plan,
        placement.node_id.into_canonical(),
        IMAGE_CONTENT_TRANSFORM_FEATURE,
    ) {
        return Err(OdgImageError::MissingContentTransformLoss {
            node_id: placement.node_id,
        });
    }

    Ok(())
}

fn has_preserved_feature(plan: &ExportPlan, origin: CanonicalId, feature: &str) -> bool {
    plan.features.iter().any(|planned| {
        planned.request.origin == Some(origin)
            && planned.request.feature == feature
            && planned.disposition == CapabilityLevel::Preserved
    })
}

fn has_reported_feature_loss(plan: &ExportPlan, origin: CanonicalId, feature: &str) -> bool {
    plan.losses
        .iter()
        .any(|loss| loss.origin == Some(origin) && loss.feature == feature)
}

fn resource_path(placement: &OdgEmbeddedImagePlacement) -> String {
    let extension = resource_extension(&placement.mime)
        .expect("validated placement MIME has a bounded ODG extension");
    format!(
        "Pictures/{}.{}",
        placement.resource_id.as_canonical(),
        extension
    )
}

fn resource_extension(mime: &str) -> Option<&'static str> {
    match mime {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        _ => None,
    }
}

fn image_frame_xml(
    placement: &OdgEmbeddedImagePlacement,
    resource_path: &str,
) -> Result<String, OdgImageError> {
    let bounds = placement.frame_bounds;
    if bounds.right().is_none() || bounds.bottom().is_none() {
        return Err(OdgImageError::InvalidFrameBounds {
            node_id: placement.node_id,
        });
    }

    let mut xml = String::new();
    writeln!(
        xml,
        "        <draw:frame draw:name=\"{}\" draw:z-index=\"{}\" svg:x=\"{}pt\" svg:y=\"{}pt\" svg:width=\"{}pt\" svg:height=\"{}pt\">",
        image_name(placement.node_id),
        placement.z_index,
        crate::semantic::format_emu_points(bounds.x),
        crate::semantic::format_emu_points(bounds.y),
        crate::semantic::format_emu_points(bounds.width),
        crate::semantic::format_emu_points(bounds.height),
    )
    .unwrap();
    writeln!(
        xml,
        "          <draw:image xlink:href=\"{resource_path}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/>"
    )
    .unwrap();
    xml.push_str("        </draw:frame>\n");
    Ok(xml)
}

fn image_name(node_id: NodeId) -> String {
    let mut value = String::from("Image_");
    for byte in node_id.into_canonical().into_bytes() {
        write!(value, "{byte:02x}").unwrap();
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use pub_export::{SemanticFeatureRequest, TargetCapabilityManifest, TargetProfile, plan_export};
    use pub_model::{CanonicalId, LengthEmu};
    use std::collections::BTreeMap;

    fn id(byte: u8) -> CanonicalId {
        CanonicalId::from_bytes([byte; 16])
    }

    fn plan(node_id: NodeId, resource_id: ResourceId) -> ExportPlan {
        let target = TargetProfile {
            format: "odg".into(),
            adapter_version: crate::ODG_ADAPTER_VERSION_V0_1.into(),
            profile: "bounded-editable".into(),
            schema_fence: Some(crate::ODG_SCHEMA_FENCE_ODF_1_4.into()),
        };
        let mut features = BTreeMap::new();
        features.insert(IMAGE_BYTES_FEATURE.into(), CapabilityLevel::Preserved);
        features.insert(
            IMAGE_FRAME_GEOMETRY_FEATURE.into(),
            CapabilityLevel::Preserved,
        );
        features.insert(
            IMAGE_CONTENT_TRANSFORM_FEATURE.into(),
            CapabilityLevel::Approximated,
        );
        plan_export(
            &TargetCapabilityManifest { target, features },
            vec![
                SemanticFeatureRequest {
                    feature: IMAGE_BYTES_FEATURE.into(),
                    origin: Some(resource_id.into_canonical()),
                    property_path: Some("replacement_asset.bytes".into()),
                    require_preserved: true,
                },
                SemanticFeatureRequest {
                    feature: IMAGE_FRAME_GEOMETRY_FEATURE.into(),
                    origin: Some(node_id.into_canonical()),
                    property_path: Some("node.bounds".into()),
                    require_preserved: true,
                },
                SemanticFeatureRequest {
                    feature: IMAGE_CONTENT_TRANSFORM_FEATURE.into(),
                    origin: Some(node_id.into_canonical()),
                    property_path: Some("image.content_transform".into()),
                    require_preserved: false,
                },
            ],
        )
    }

    #[test]
    fn validates_bounded_image_plan_contract() {
        let node_id = NodeId::from_canonical(id(1));
        let resource_id = ResourceId::from_canonical(id(2));
        let plan = plan(node_id, resource_id);
        let placement = OdgEmbeddedImagePlacement {
            node_id,
            page_id: PageId::from_canonical(id(3)),
            resource_id,
            frame_bounds: RectEmu {
                x: LengthEmu::new(10),
                y: LengthEmu::new(20),
                width: LengthEmu::new(30),
                height: LengthEmu::new(40),
            },
            z_index: 2,
            mime: "image/png".into(),
            bytes: vec![1, 2, 3],
        };
        assert!(validate_placement(&plan, &placement).is_ok());
    }
}
