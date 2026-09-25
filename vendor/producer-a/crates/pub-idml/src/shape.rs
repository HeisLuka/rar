use crate::{IdmlPackage, IdmlPart, IdmlPartContent, IdmlPartKind};
use pub_export::{CapabilityLevel, ExportPlan};
use pub_model::{CanonicalId, EMU_PER_POINT, LengthEmu, NodeId, PageId, RectEmu, Size2D};
use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Write as _;

pub const AUTHORED_SHAPE_IDENTITY_FEATURE: &str = "node.created_identity";
pub const AUTHORED_SHAPE_GEOMETRY_FEATURE: &str = "node.geometry.bounds";
pub const AUTHORED_SHAPE_PAINT_FEATURE: &str = "shape.paint";
pub const AUTHORED_SHAPE_ORDER_FEATURE: &str = "page.object_order";

const GRAPHIC_PATH: &str = "Resources/Graphic.xml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct IdmlRgb8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdmlAuthoredRectanglePlacement {
    pub node_id: NodeId,
    pub page_id: PageId,
    pub page_size: Size2D,
    pub bounds: RectEmu,
    pub fill_visible: bool,
    pub fill_color: IdmlRgb8,
    pub stroke_visible: bool,
    pub stroke_color: IdmlRgb8,
    pub stroke_width_emu: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdmlAuthoredShapeError {
    NonIdmlPackage,
    DuplicatePlacement { node_id: NodeId },
    InvalidPageSize { page_id: PageId },
    InvalidBounds { node_id: NodeId },
    InvalidStroke { node_id: NodeId },
    MissingPreservedIdentity { node_id: NodeId },
    MissingPreservedGeometry { node_id: NodeId },
    MissingPreservedPaint { node_id: NodeId },
    MissingOrderLoss { page_id: PageId },
    MissingSpread { page_id: PageId },
    BinarySpread { page_id: PageId },
    InvalidSpreadXml { page_id: PageId },
    MissingDesignMap,
    BinaryDesignMap,
    InvalidDesignMapXml,
    BinaryGraphicResource,
    InvalidGraphicResourceXml,
}

impl fmt::Display for IdmlAuthoredShapeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonIdmlPackage => {
                formatter.write_str("authored shape projection requires an IDML package")
            }
            Self::DuplicatePlacement { node_id } => write!(
                formatter,
                "duplicate authored rectangle placement for node {}",
                node_id.as_canonical()
            ),
            Self::InvalidPageSize { page_id } => write!(
                formatter,
                "authored rectangle page {} has non-positive size",
                page_id.as_canonical()
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
            Self::MissingSpread { page_id } => write!(
                formatter,
                "IDML package is missing spread for authored rectangle page {}",
                page_id.as_canonical()
            ),
            Self::BinarySpread { page_id } => write!(
                formatter,
                "IDML spread for authored rectangle page {} is binary",
                page_id.as_canonical()
            ),
            Self::InvalidSpreadXml { page_id } => write!(
                formatter,
                "IDML spread for authored rectangle page {} has no closing Spread element",
                page_id.as_canonical()
            ),
            Self::MissingDesignMap => formatter.write_str("IDML package is missing designmap.xml"),
            Self::BinaryDesignMap => formatter.write_str("IDML designmap.xml is binary"),
            Self::InvalidDesignMapXml => {
                formatter.write_str("IDML designmap.xml has no closing Document element")
            }
            Self::BinaryGraphicResource => {
                formatter.write_str("IDML Resources/Graphic.xml is binary")
            }
            Self::InvalidGraphicResourceXml => formatter
                .write_str("IDML Resources/Graphic.xml has no closing idPkg:Graphic element"),
        }
    }
}

impl std::error::Error for IdmlAuthoredShapeError {}

pub fn add_authored_rectangles_to_idml(
    plan: &ExportPlan,
    package: &mut IdmlPackage,
    placements: &[IdmlAuthoredRectanglePlacement],
) -> Result<(), IdmlAuthoredShapeError> {
    if package.target.format != "idml" || plan.target.format != "idml" {
        return Err(IdmlAuthoredShapeError::NonIdmlPackage);
    }

    let mut ordered = placements.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|placement| (placement.page_id, placement.node_id));

    let mut seen_nodes = BTreeSet::new();
    let mut colors = BTreeSet::new();
    for placement in &ordered {
        if !seen_nodes.insert(placement.node_id) {
            return Err(IdmlAuthoredShapeError::DuplicatePlacement {
                node_id: placement.node_id,
            });
        }
        validate_placement(plan, placement)?;
        if placement.fill_visible {
            colors.insert(placement.fill_color);
        }
        if placement.stroke_visible {
            colors.insert(placement.stroke_color);
        }
    }

    if !ordered.is_empty() {
        ensure_graphic_resource(package, &colors)?;
        ensure_designmap_graphic_reference(package)?;
    }

    for placement in ordered {
        let spread_path = spread_path(placement.page_id);
        let spread = package
            .parts
            .iter_mut()
            .find(|part| part.kind == IdmlPartKind::Spread && part.path == spread_path)
            .ok_or(IdmlAuthoredShapeError::MissingSpread {
                page_id: placement.page_id,
            })?;

        let IdmlPartContent::Text(xml) = &mut spread.content else {
            return Err(IdmlAuthoredShapeError::BinarySpread {
                page_id: placement.page_id,
            });
        };
        let marker = "  </Spread>";
        let Some(insert_at) = xml.rfind(marker) else {
            return Err(IdmlAuthoredShapeError::InvalidSpreadXml {
                page_id: placement.page_id,
            });
        };
        xml.insert_str(insert_at, &rectangle_xml(placement)?);
    }

    package.parts.sort();
    Ok(())
}

fn validate_placement(
    plan: &ExportPlan,
    placement: &IdmlAuthoredRectanglePlacement,
) -> Result<(), IdmlAuthoredShapeError> {
    if !placement.page_size.is_positive() {
        return Err(IdmlAuthoredShapeError::InvalidPageSize {
            page_id: placement.page_id,
        });
    }
    if placement.bounds.width.get() <= 0
        || placement.bounds.height.get() <= 0
        || placement.bounds.right().is_none()
        || placement.bounds.bottom().is_none()
    {
        return Err(IdmlAuthoredShapeError::InvalidBounds {
            node_id: placement.node_id,
        });
    }
    if placement.stroke_width_emu <= 0 {
        return Err(IdmlAuthoredShapeError::InvalidStroke {
            node_id: placement.node_id,
        });
    }

    let origin = placement.node_id.into_canonical();
    if !has_preserved_feature(plan, origin, AUTHORED_SHAPE_IDENTITY_FEATURE) {
        return Err(IdmlAuthoredShapeError::MissingPreservedIdentity {
            node_id: placement.node_id,
        });
    }
    if !has_preserved_feature(plan, origin, AUTHORED_SHAPE_GEOMETRY_FEATURE) {
        return Err(IdmlAuthoredShapeError::MissingPreservedGeometry {
            node_id: placement.node_id,
        });
    }
    if !has_preserved_feature(plan, origin, AUTHORED_SHAPE_PAINT_FEATURE) {
        return Err(IdmlAuthoredShapeError::MissingPreservedPaint {
            node_id: placement.node_id,
        });
    }
    if !has_reported_feature_loss(
        plan,
        placement.page_id.into_canonical(),
        AUTHORED_SHAPE_ORDER_FEATURE,
    ) {
        return Err(IdmlAuthoredShapeError::MissingOrderLoss {
            page_id: placement.page_id,
        });
    }

    Ok(())
}

fn ensure_graphic_resource(
    package: &mut IdmlPackage,
    colors: &BTreeSet<IdmlRgb8>,
) -> Result<(), IdmlAuthoredShapeError> {
    if let Some(graphic) = package
        .parts
        .iter_mut()
        .find(|part| part.path == GRAPHIC_PATH)
    {
        let IdmlPartContent::Text(xml) = &mut graphic.content else {
            return Err(IdmlAuthoredShapeError::BinaryGraphicResource);
        };
        let marker = "</idPkg:Graphic>";
        let Some(insert_at) = xml.rfind(marker) else {
            return Err(IdmlAuthoredShapeError::InvalidGraphicResourceXml);
        };
        let mut fragment = String::new();
        for color in colors {
            let self_id = color_id(*color);
            if !xml.contains(&format!("Self=\"{self_id}\"")) {
                fragment.push_str(&color_xml(*color));
            }
        }
        xml.insert_str(insert_at, &fragment);
        return Ok(());
    }

    let mut xml = String::new();
    writeln!(xml, "<?xml version=\"1.0\" encoding=\"utf-8\"?>").unwrap();
    writeln!(
        xml,
        "<idPkg:Graphic xmlns:idPkg=\"{}\" DOMVersion=\"{}\">",
        crate::IDML_PACKAGING_NAMESPACE,
        "7.0"
    )
    .unwrap();
    for color in colors {
        xml.push_str(&color_xml(*color));
    }
    xml.push_str("</idPkg:Graphic>\n");

    package.parts.push(IdmlPart {
        path: GRAPHIC_PATH.into(),
        kind: IdmlPartKind::Resource,
        content: IdmlPartContent::Text(xml),
    });
    Ok(())
}

fn ensure_designmap_graphic_reference(
    package: &mut IdmlPackage,
) -> Result<(), IdmlAuthoredShapeError> {
    let designmap = package
        .parts
        .iter_mut()
        .find(|part| part.kind == IdmlPartKind::DesignMap && part.path == "designmap.xml")
        .ok_or(IdmlAuthoredShapeError::MissingDesignMap)?;
    let IdmlPartContent::Text(xml) = &mut designmap.content else {
        return Err(IdmlAuthoredShapeError::BinaryDesignMap);
    };
    if xml.contains("src=\"Resources/Graphic.xml\"") {
        return Ok(());
    }

    let marker = "</Document>";
    let Some(insert_at) = xml.rfind(marker) else {
        return Err(IdmlAuthoredShapeError::InvalidDesignMapXml);
    };
    xml.insert_str(
        insert_at,
        "  <idPkg:Graphic src=\"Resources/Graphic.xml\"/>\n",
    );
    Ok(())
}

fn rectangle_xml(
    placement: &IdmlAuthoredRectanglePlacement,
) -> Result<String, IdmlAuthoredShapeError> {
    let right = placement
        .bounds
        .right()
        .ok_or(IdmlAuthoredShapeError::InvalidBounds {
            node_id: placement.node_id,
        })?;
    let bottom = placement
        .bounds
        .bottom()
        .ok_or(IdmlAuthoredShapeError::InvalidBounds {
            node_id: placement.node_id,
        })?;

    let x = format_emu_points(placement.bounds.x);
    let y = format_emu_points(placement.bounds.y);
    let right = format_emu_points(right);
    let bottom = format_emu_points(bottom);
    let tx = format_ratio(
        -i128::from(placement.page_size.width.get()),
        i128::from(EMU_PER_POINT),
        15,
    );
    let ty = format_ratio(
        -i128::from(placement.page_size.height.get()),
        i128::from(EMU_PER_POINT) * 2,
        15,
    );
    let fill = if placement.fill_visible {
        color_id(placement.fill_color)
    } else {
        "Swatch/None".into()
    };
    let stroke = if placement.stroke_visible {
        color_id(placement.stroke_color)
    } else {
        "Swatch/None".into()
    };
    let stroke_width = if placement.stroke_visible {
        format_emu_points(LengthEmu::new(placement.stroke_width_emu))
    } else {
        "0".into()
    };

    let mut xml = String::new();
    writeln!(
        xml,
        "    <Rectangle Self=\"{}\" ContentType=\"GraphicType\" AppliedObjectStyle=\"ObjectStyle/$ID/[None]\" Visible=\"true\" Name=\"$ID/\" FillColor=\"{fill}\" StrokeColor=\"{stroke}\" StrokeWeight=\"{stroke_width}\" ItemTransform=\"1 0 0 1 {tx} {ty}\">",
        idml_self("uac", placement.node_id.into_canonical())
    )
    .unwrap();
    xml.push_str("      <Properties>\n");
    xml.push_str("        <PathGeometry>\n");
    xml.push_str("          <GeometryPathType PathOpen=\"false\">\n");
    xml.push_str("            <PathPointArray>\n");
    write_path_point(&mut xml, &x, &y);
    write_path_point(&mut xml, &x, &bottom);
    write_path_point(&mut xml, &right, &bottom);
    write_path_point(&mut xml, &right, &y);
    xml.push_str("            </PathPointArray>\n");
    xml.push_str("          </GeometryPathType>\n");
    xml.push_str("        </PathGeometry>\n");
    xml.push_str("      </Properties>\n");
    xml.push_str("    </Rectangle>\n");
    Ok(xml)
}

fn color_xml(color: IdmlRgb8) -> String {
    let self_id = color_id(color);
    let name = color_name(color);
    format!(
        "  <Color Self=\"{self_id}\" Model=\"Process\" Space=\"RGB\" ColorValue=\"{} {} {}\" ColorOverride=\"Normal\" Name=\"{name}\" ColorEditable=\"true\" ColorRemovable=\"true\" Visible=\"true\"/>\n",
        color.r, color.g, color.b
    )
}

fn color_id(color: IdmlRgb8) -> String {
    format!("Color/{}", color_name(color))
}

fn color_name(color: IdmlRgb8) -> String {
    format!("Chaptera_RGB_{}_{}_{}", color.r, color.g, color.b)
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

fn write_path_point(xml: &mut String, x: &str, y: &str) {
    writeln!(
        xml,
        "              <PathPointType Anchor=\"{x} {y}\" LeftDirection=\"{x} {y}\" RightDirection=\"{x} {y}\"/>"
    )
    .unwrap();
}

fn spread_path(page_id: PageId) -> String {
    format!(
        "Spreads/Spread_{}.xml",
        idml_self("usp", page_id.into_canonical())
    )
}

fn idml_self(prefix: &str, id: CanonicalId) -> String {
    let mut value = String::with_capacity(prefix.len() + 32);
    value.push_str(prefix);
    for byte in id.into_bytes() {
        write!(value, "{byte:02x}").unwrap();
    }
    value
}

fn format_emu_points(value: LengthEmu) -> String {
    format_ratio(i128::from(value.get()), i128::from(EMU_PER_POINT), 15)
}

fn format_ratio(numerator: i128, denominator: i128, precision: usize) -> String {
    debug_assert!(denominator > 0);
    if numerator == 0 {
        return "0".into();
    }

    let negative = numerator < 0;
    let numerator = numerator.abs();
    let whole = numerator / denominator;
    let mut remainder = numerator % denominator;
    let mut result = String::new();

    if negative {
        result.push('-');
    }
    write!(&mut result, "{whole}").unwrap();
    if remainder == 0 {
        return result;
    }

    result.push('.');
    for _ in 0..precision {
        remainder *= 10;
        let digit = remainder / denominator;
        result.push(char::from(
            b'0' + u8::try_from(digit).expect("decimal digit"),
        ));
        remainder %= denominator;
        if remainder == 0 {
            break;
        }
    }
    while result.ends_with('0') {
        result.pop();
    }
    if result.ends_with('.') {
        result.pop();
    }
    result
}
