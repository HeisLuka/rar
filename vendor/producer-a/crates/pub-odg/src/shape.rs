use crate::{OdgPackage, OdgPartKind};
use pub_export::{CapabilityLevel, ExportPlan};
use pub_model::{CanonicalId, LengthEmu, NodeId, PageId, RectEmu};
use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Write as _;

pub const AUTHORED_SHAPE_IDENTITY_FEATURE: &str = "node.created_identity";
pub const AUTHORED_SHAPE_GEOMETRY_FEATURE: &str = "node.geometry.bounds";
pub const AUTHORED_SHAPE_PAINT_FEATURE: &str = "shape.paint";
pub const AUTHORED_SHAPE_ORDER_FEATURE: &str = "page.object_order";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct OdgRgb8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OdgAuthoredRectanglePlacement {
    pub node_id: NodeId,
    pub page_id: PageId,
    pub bounds: RectEmu,
    pub fill_visible: bool,
    pub fill_color: OdgRgb8,
    pub stroke_visible: bool,
    pub stroke_color: OdgRgb8,
    pub stroke_width_emu: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OdgAuthoredShapeError {
    NonOdgPackage,
    DuplicatePlacement { node_id: NodeId },
    InvalidBounds { node_id: NodeId },
    InvalidStroke { node_id: NodeId },
    MissingPreservedIdentity { node_id: NodeId },
    MissingPreservedGeometry { node_id: NodeId },
    MissingPreservedPaint { node_id: NodeId },
    MissingOrderLoss { page_id: PageId },
    MissingContent,
    InvalidContentUtf8,
    MissingAutomaticStyles,
    MissingPage { page_id: PageId },
}

impl fmt::Display for OdgAuthoredShapeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonOdgPackage => {
                formatter.write_str("authored shape projection requires an ODG package")
            }
            Self::DuplicatePlacement { node_id } => write!(
                formatter,
                "duplicate authored rectangle placement for node {}",
                node_id.as_canonical()
            ),
            Self::InvalidBounds { node_id } => write!(
                formatter,
                "authored rectangle {} has non-positive or overflowing bounds",
                node_id.as_canonical()
            ),
            Self::InvalidStroke { node_id } => write!(
                formatter,
                "authored rectangle {} has invalid stroke width",
                node_id.as_canonical()
            ),
            Self::MissingPreservedIdentity { node_id } => write!(
                formatter,
                "authored rectangle {} lacks preserved node.created_identity",
                node_id.as_canonical()
            ),
            Self::MissingPreservedGeometry { node_id } => write!(
                formatter,
                "authored rectangle {} lacks preserved node.geometry.bounds",
                node_id.as_canonical()
            ),
            Self::MissingPreservedPaint { node_id } => write!(
                formatter,
                "authored rectangle {} lacks preserved shape.paint",
                node_id.as_canonical()
            ),
            Self::MissingOrderLoss { page_id } => write!(
                formatter,
                "authored rectangle page {} lacks explicit page.object_order loss",
                page_id.as_canonical()
            ),
            Self::MissingContent => formatter.write_str("ODG package is missing content.xml"),
            Self::InvalidContentUtf8 => formatter.write_str("ODG content.xml is not UTF-8"),
            Self::MissingAutomaticStyles => {
                formatter.write_str("ODG content.xml is missing automatic-styles marker")
            }
            Self::MissingPage { page_id } => write!(
                formatter,
                "ODG content.xml is missing page {}",
                page_id.as_canonical()
            ),
        }
    }
}

impl std::error::Error for OdgAuthoredShapeError {}

pub fn add_authored_rectangles_to_odg(
    plan: &ExportPlan,
    package: &mut OdgPackage,
    placements: &[OdgAuthoredRectanglePlacement],
) -> Result<(), OdgAuthoredShapeError> {
    if package.target.format != "odg" || plan.target.format != "odg" {
        return Err(OdgAuthoredShapeError::NonOdgPackage);
    }

    let mut ordered = placements.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|placement| (placement.page_id, placement.node_id));

    let mut seen_nodes = BTreeSet::new();
    for placement in &ordered {
        if !seen_nodes.insert(placement.node_id) {
            return Err(OdgAuthoredShapeError::DuplicatePlacement {
                node_id: placement.node_id,
            });
        }
        validate_placement(plan, placement)?;
    }

    if ordered.is_empty() {
        return Ok(());
    }

    let content_index = package
        .parts
        .iter()
        .position(|part| part.kind == OdgPartKind::Content && part.path == crate::ODG_CONTENT_PATH)
        .ok_or(OdgAuthoredShapeError::MissingContent)?;
    let mut xml = String::from_utf8(package.parts[content_index].content.clone())
        .map_err(|_| OdgAuthoredShapeError::InvalidContentUtf8)?;

    let automatic_styles = "  <office:automatic-styles/>\n";
    let Some(styles_at) = xml.find(automatic_styles) else {
        return Err(OdgAuthoredShapeError::MissingAutomaticStyles);
    };
    let mut styles = String::from("  <office:automatic-styles>\n");
    for placement in &ordered {
        styles.push_str(&graphic_style_xml(placement));
    }
    styles.push_str("  </office:automatic-styles>\n");
    xml.replace_range(styles_at..styles_at + automatic_styles.len(), &styles);

    for placement in ordered {
        let page_name = crate::semantic::page_name(placement.page_id);
        let page_marker = format!("<draw:page draw:name=\"{page_name}\"");
        let page_start = xml
            .find(&page_marker)
            .ok_or(OdgAuthoredShapeError::MissingPage {
                page_id: placement.page_id,
            })?;
        let relative_close = xml[page_start..].find("      </draw:page>").ok_or(
            OdgAuthoredShapeError::MissingPage {
                page_id: placement.page_id,
            },
        )?;
        let insert_at = page_start + relative_close;
        xml.insert_str(insert_at, &rectangle_xml(placement)?);
    }

    package.parts[content_index].content = xml.into_bytes();
    Ok(())
}

fn validate_placement(
    plan: &ExportPlan,
    placement: &OdgAuthoredRectanglePlacement,
) -> Result<(), OdgAuthoredShapeError> {
    if placement.bounds.width.get() <= 0
        || placement.bounds.height.get() <= 0
        || placement.bounds.right().is_none()
        || placement.bounds.bottom().is_none()
    {
        return Err(OdgAuthoredShapeError::InvalidBounds {
            node_id: placement.node_id,
        });
    }
    if placement.stroke_width_emu <= 0 {
        return Err(OdgAuthoredShapeError::InvalidStroke {
            node_id: placement.node_id,
        });
    }

    let origin = placement.node_id.into_canonical();
    if !has_preserved_feature(plan, origin, AUTHORED_SHAPE_IDENTITY_FEATURE) {
        return Err(OdgAuthoredShapeError::MissingPreservedIdentity {
            node_id: placement.node_id,
        });
    }
    if !has_preserved_feature(plan, origin, AUTHORED_SHAPE_GEOMETRY_FEATURE) {
        return Err(OdgAuthoredShapeError::MissingPreservedGeometry {
            node_id: placement.node_id,
        });
    }
    if !has_preserved_feature(plan, origin, AUTHORED_SHAPE_PAINT_FEATURE) {
        return Err(OdgAuthoredShapeError::MissingPreservedPaint {
            node_id: placement.node_id,
        });
    }
    if !has_reported_feature_loss(
        plan,
        placement.page_id.into_canonical(),
        AUTHORED_SHAPE_ORDER_FEATURE,
    ) {
        return Err(OdgAuthoredShapeError::MissingOrderLoss {
            page_id: placement.page_id,
        });
    }

    Ok(())
}

fn graphic_style_xml(placement: &OdgAuthoredRectanglePlacement) -> String {
    let mut properties = String::new();
    if placement.fill_visible {
        write!(
            properties,
            " draw:fill=\"solid\" draw:fill-color=\"{}\"",
            rgb_hex(placement.fill_color)
        )
        .unwrap();
    } else {
        properties.push_str(" draw:fill=\"none\"");
    }

    if placement.stroke_visible {
        write!(
            properties,
            " draw:stroke=\"solid\" svg:stroke-color=\"{}\" svg:stroke-width=\"{}pt\"",
            rgb_hex(placement.stroke_color),
            crate::semantic::format_emu_points(LengthEmu::new(placement.stroke_width_emu))
        )
        .unwrap();
    } else {
        properties.push_str(" draw:stroke=\"none\"");
    }

    format!(
        "    <style:style style:name=\"{}\" style:family=\"graphic\">\n      <style:graphic-properties{properties}/>\n    </style:style>\n",
        style_name(placement.node_id)
    )
}

fn rectangle_xml(
    placement: &OdgAuthoredRectanglePlacement,
) -> Result<String, OdgAuthoredShapeError> {
    if placement.bounds.right().is_none() || placement.bounds.bottom().is_none() {
        return Err(OdgAuthoredShapeError::InvalidBounds {
            node_id: placement.node_id,
        });
    }

    Ok(format!(
        "        <draw:rect draw:name=\"{}\" draw:style-name=\"{}\" svg:x=\"{}pt\" svg:y=\"{}pt\" svg:width=\"{}pt\" svg:height=\"{}pt\"/>\n",
        shape_name(placement.node_id),
        style_name(placement.node_id),
        crate::semantic::format_emu_points(placement.bounds.x),
        crate::semantic::format_emu_points(placement.bounds.y),
        crate::semantic::format_emu_points(placement.bounds.width),
        crate::semantic::format_emu_points(placement.bounds.height),
    ))
}

fn rgb_hex(color: OdgRgb8) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

fn shape_name(node_id: NodeId) -> String {
    stable_name("Shape", node_id)
}

fn style_name(node_id: NodeId) -> String {
    stable_name("ShapeStyle", node_id)
}

fn stable_name(prefix: &str, node_id: NodeId) -> String {
    let mut value = String::from(prefix);
    value.push('_');
    for byte in node_id.into_canonical().into_bytes() {
        write!(value, "{byte:02x}").unwrap();
    }
    value
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
