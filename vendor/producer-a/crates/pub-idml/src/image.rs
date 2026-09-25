use crate::{IdmlPackage, IdmlPartContent, IdmlPartKind};
use pub_export::{CapabilityLevel, ExportPlan};
use pub_model::{CanonicalId, EMU_PER_POINT, NodeId, PageId, RectEmu, ResourceId, Size2D};
use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Write as _;

pub const IMAGE_BYTES_FEATURE: &str = "image.bytes";
pub const IMAGE_FRAME_GEOMETRY_FEATURE: &str = "image.frame_geometry";
pub const IMAGE_CONTENT_TRANSFORM_FEATURE: &str = "image.content_transform";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdmlEmbeddedImagePlacement {
    pub node_id: NodeId,
    pub page_id: PageId,
    pub page_size: Size2D,
    pub resource_id: ResourceId,
    pub frame_bounds: RectEmu,
    pub mime: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdmlImageError {
    NonIdmlPackage,
    EmptyPayload {
        resource_id: ResourceId,
    },
    UnsupportedMime {
        resource_id: ResourceId,
        mime: String,
    },
    InvalidPageSize {
        page_id: PageId,
    },
    InvalidFrameBounds {
        node_id: NodeId,
    },
    DuplicatePlacement {
        node_id: NodeId,
    },
    MissingPreservedBytes {
        resource_id: ResourceId,
    },
    MissingPreservedFrameGeometry {
        node_id: NodeId,
    },
    MissingContentTransformLoss {
        node_id: NodeId,
    },
    MissingSpread {
        page_id: PageId,
    },
    BinarySpread {
        page_id: PageId,
    },
    InvalidSpreadXml {
        page_id: PageId,
    },
}

impl fmt::Display for IdmlImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonIdmlPackage => {
                formatter.write_str("image projection requires an IDML package")
            }
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
            Self::InvalidPageSize { page_id } => write!(
                formatter,
                "image placement page {} has non-positive size",
                page_id.as_canonical()
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
            Self::MissingSpread { page_id } => write!(
                formatter,
                "IDML package is missing spread for page {}",
                page_id.as_canonical()
            ),
            Self::BinarySpread { page_id } => write!(
                formatter,
                "IDML spread for page {} is not a text part",
                page_id.as_canonical()
            ),
            Self::InvalidSpreadXml { page_id } => write!(
                formatter,
                "IDML spread for page {} has no closing Spread element",
                page_id.as_canonical()
            ),
        }
    }
}

impl std::error::Error for IdmlImageError {}

/// Adds a bounded embedded-image projection to an already-created logical IDML package.
///
/// The caller supplies only source-neutral semantics: typed identities, exact payload bytes,
/// MIME type and authored frame geometry. The bounded v0.1 projection intentionally
/// materializes the image content into the authored frame bounds because Publisher crop/fit/
/// inner-transform semantics are not grounded yet. That downgrade must already be present in
/// the ExportPlan as image.content_transform.
pub fn add_embedded_images_to_idml(
    plan: &ExportPlan,
    package: &mut IdmlPackage,
    placements: &[IdmlEmbeddedImagePlacement],
) -> Result<(), IdmlImageError> {
    if package.target.format != "idml" || plan.target.format != "idml" {
        return Err(IdmlImageError::NonIdmlPackage);
    }

    let mut ordered = placements.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|placement| (placement.page_id, placement.node_id, placement.resource_id));

    let mut seen_nodes = BTreeSet::new();
    for placement in ordered {
        if !seen_nodes.insert(placement.node_id) {
            return Err(IdmlImageError::DuplicatePlacement {
                node_id: placement.node_id,
            });
        }
        validate_placement(plan, placement)?;

        let spread_path = spread_path(placement.page_id);
        let spread = package
            .parts
            .iter_mut()
            .find(|part| part.kind == IdmlPartKind::Spread && part.path == spread_path)
            .ok_or(IdmlImageError::MissingSpread {
                page_id: placement.page_id,
            })?;

        let IdmlPartContent::Text(xml) = &mut spread.content else {
            return Err(IdmlImageError::BinarySpread {
                page_id: placement.page_id,
            });
        };

        let marker = "  </Spread>";
        let Some(insert_at) = xml.rfind(marker) else {
            return Err(IdmlImageError::InvalidSpreadXml {
                page_id: placement.page_id,
            });
        };

        let fragment = image_frame_xml(placement)?;
        xml.insert_str(insert_at, &fragment);
    }

    Ok(())
}

fn validate_placement(
    plan: &ExportPlan,
    placement: &IdmlEmbeddedImagePlacement,
) -> Result<(), IdmlImageError> {
    if placement.bytes.is_empty() {
        return Err(IdmlImageError::EmptyPayload {
            resource_id: placement.resource_id,
        });
    }
    image_type_name(&placement.mime).ok_or_else(|| IdmlImageError::UnsupportedMime {
        resource_id: placement.resource_id,
        mime: placement.mime.clone(),
    })?;

    if !placement.page_size.is_positive() {
        return Err(IdmlImageError::InvalidPageSize {
            page_id: placement.page_id,
        });
    }
    if placement.frame_bounds.width.get() <= 0
        || placement.frame_bounds.height.get() <= 0
        || placement.frame_bounds.right().is_none()
        || placement.frame_bounds.bottom().is_none()
    {
        return Err(IdmlImageError::InvalidFrameBounds {
            node_id: placement.node_id,
        });
    }

    if !has_preserved_feature(
        plan,
        placement.resource_id.into_canonical(),
        IMAGE_BYTES_FEATURE,
    ) {
        return Err(IdmlImageError::MissingPreservedBytes {
            resource_id: placement.resource_id,
        });
    }
    if !has_preserved_feature(
        plan,
        placement.node_id.into_canonical(),
        IMAGE_FRAME_GEOMETRY_FEATURE,
    ) {
        return Err(IdmlImageError::MissingPreservedFrameGeometry {
            node_id: placement.node_id,
        });
    }
    if !has_reported_feature_loss(
        plan,
        placement.node_id.into_canonical(),
        IMAGE_CONTENT_TRANSFORM_FEATURE,
    ) {
        return Err(IdmlImageError::MissingContentTransformLoss {
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

fn image_frame_xml(placement: &IdmlEmbeddedImagePlacement) -> Result<String, IdmlImageError> {
    let image_type =
        image_type_name(&placement.mime).ok_or_else(|| IdmlImageError::UnsupportedMime {
            resource_id: placement.resource_id,
            mime: placement.mime.clone(),
        })?;
    let right = placement
        .frame_bounds
        .right()
        .ok_or(IdmlImageError::InvalidFrameBounds {
            node_id: placement.node_id,
        })?;
    let bottom = placement
        .frame_bounds
        .bottom()
        .ok_or(IdmlImageError::InvalidFrameBounds {
            node_id: placement.node_id,
        })?;

    let x = format_emu_points(placement.frame_bounds.x);
    let y = format_emu_points(placement.frame_bounds.y);
    let right = format_emu_points(right);
    let bottom = format_emu_points(bottom);
    let width = format_emu_points(placement.frame_bounds.width);
    let height = format_emu_points(placement.frame_bounds.height);
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
    let encoded = encode_base64(&placement.bytes);

    let frame_self = idml_self("uif", placement.node_id.into_canonical());
    let image_self = idml_self("ui", placement.node_id.into_canonical());

    let mut xml = String::new();
    writeln!(
        xml,
        "    <Rectangle Self=\"{frame_self}\" ContentType=\"GraphicType\" ItemTransform=\"1 0 0 1 {tx} {ty}\">"
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
    writeln!(
        xml,
        "      <Image Self=\"{image_self}\" ImageTypeName=\"{image_type}\" ItemTransform=\"1 0 0 1 0 0\" Visible=\"true\">"
    )
    .unwrap();
    xml.push_str("        <Properties>\n");
    xml.push_str("          <Profile type=\"string\">$ID/Embedded</Profile>\n");
    writeln!(
        xml,
        "          <GraphicBounds Left=\"0\" Top=\"0\" Right=\"{width}\" Bottom=\"{height}\"/>"
    )
    .unwrap();
    writeln!(xml, "          <Contents><![CDATA[{encoded}]]></Contents>").unwrap();
    xml.push_str("        </Properties>\n");
    xml.push_str("      </Image>\n");
    xml.push_str("    </Rectangle>\n");
    Ok(xml)
}

fn image_type_name(mime: &str) -> Option<&'static str> {
    match mime {
        "image/png" => Some("$ID/PNG"),
        "image/jpeg" => Some("$ID/JPEG"),
        _ => None,
    }
}

fn write_path_point(xml: &mut String, x: &str, y: &str) {
    writeln!(
        xml,
        "              <PathPointType Anchor=\"{x} {y}\" LeftDirection=\"{x} {y}\" RightDirection=\"{x} {y}\"/>"
    )
    .unwrap();
}

fn spread_path(page_id: PageId) -> String {
    let self_id = idml_self("usp", page_id.into_canonical());
    format!("Spreads/Spread_{self_id}.xml")
}

fn idml_self(prefix: &str, id: CanonicalId) -> String {
    let mut value = String::with_capacity(prefix.len() + 32);
    value.push_str(prefix);
    for byte in id.into_bytes() {
        write!(value, "{byte:02x}").unwrap();
    }
    value
}

fn format_emu_points(value: pub_model::LengthEmu) -> String {
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
    write!(result, "{whole}").unwrap();

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

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);

        output.push(char::from(TABLE[(b0 >> 2) as usize]));
        output.push(char::from(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize]));
        if chunk.len() > 1 {
            output.push(char::from(TABLE[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize]));
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(char::from(TABLE[(b2 & 0x3f) as usize]));
        } else {
            output.push('=');
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IDML_ADAPTER_VERSION_V0_1, IDML_PACKAGING_NAMESPACE, IdmlPackageBuilder};
    use pub_export::{
        SemanticFeatureRequest, TargetCapabilityManifest, TargetProfile, plan_export,
    };
    use pub_model::LengthEmu;
    use std::collections::BTreeMap;

    fn id(byte: u8) -> CanonicalId {
        CanonicalId::from_bytes([byte; 16])
    }

    fn plan(node_id: NodeId, resource_id: ResourceId) -> ExportPlan {
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
        let manifest = TargetCapabilityManifest {
            target: TargetProfile {
                format: "idml".into(),
                adapter_version: IDML_ADAPTER_VERSION_V0_1.into(),
                profile: "bounded-editable".into(),
                schema_fence: Some("legacy-spec-8.02/dom-7.0".into()),
            },
            features,
        };
        plan_export(
            &manifest,
            vec![
                SemanticFeatureRequest {
                    feature: IMAGE_BYTES_FEATURE.into(),
                    origin: Some(resource_id.into_canonical()),
                    property_path: Some("resource.original_blob".into()),
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

    fn placement() -> IdmlEmbeddedImagePlacement {
        IdmlEmbeddedImagePlacement {
            node_id: NodeId::from_canonical(id(2)),
            page_id: PageId::from_canonical(id(1)),
            page_size: Size2D::new(LengthEmu::new(7_620_000), LengthEmu::new(9_906_000)),
            resource_id: ResourceId::from_canonical(id(3)),
            frame_bounds: RectEmu::new(
                LengthEmu::new(1_270_000),
                LengthEmu::new(2_540_000),
                LengthEmu::new(2_540_000),
                LengthEmu::new(1_270_000),
            ),
            mime: "image/png".into(),
            bytes: vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a],
        }
    }

    fn package_for(placement: &IdmlEmbeddedImagePlacement) -> IdmlPackage {
        let export_plan = plan(placement.node_id, placement.resource_id);
        let mut builder = IdmlPackageBuilder::from_export_plan(&export_plan).expect("IDML builder");
        let spread = spread_path(placement.page_id);
        builder
            .add_part(
                spread,
                IdmlPartKind::Spread,
                format!(
                    "<?xml version=\"1.0\"?><idPkg:Spread xmlns:idPkg=\"{}\"><Spread Self=\"x\">\n  </Spread>\n</idPkg:Spread>\n",
                    IDML_PACKAGING_NAMESPACE
                ),
            )
            .expect("spread");
        builder
            .add_part("designmap.xml", IdmlPartKind::DesignMap, "<Document/>")
            .expect("designmap");
        builder.finish().expect("package")
    }

    #[test]
    fn base64_encoder_matches_known_vectors() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(&[0xff, 0x00, 0x80]), "/wCA");
    }

    #[test]
    fn embedded_image_uses_standard_contents_and_authored_frame() {
        let placement = placement();
        let export_plan = plan(placement.node_id, placement.resource_id);
        let mut package = package_for(&placement);

        add_embedded_images_to_idml(&export_plan, &mut package, std::slice::from_ref(&placement))
            .expect("embedded image projection");

        let spread = package
            .parts
            .iter()
            .find(|part| part.kind == IdmlPartKind::Spread)
            .expect("spread");
        let xml = spread.content.as_text().expect("spread text");
        assert!(xml.contains("<Rectangle Self="));
        assert!(xml.contains("ContentType=\"GraphicType\""));
        assert!(xml.contains("<Image Self="));
        assert!(xml.contains("ImageTypeName=\"$ID/PNG\""));
        assert!(xml.contains("<Profile type=\"string\">$ID/Embedded</Profile>"));
        assert!(xml.contains("<Contents><![CDATA[iVBORw0K]]></Contents>"));
        assert!(xml.contains("Anchor=\"100 200\""));
        assert!(xml.contains("Anchor=\"300 300\""));
        assert!(xml.contains("GraphicBounds Left=\"0\" Top=\"0\" Right=\"200\" Bottom=\"100\""));
    }

    #[test]
    fn missing_transform_loss_is_rejected() {
        let placement = placement();
        let mut export_plan = plan(placement.node_id, placement.resource_id);
        export_plan
            .losses
            .retain(|loss| loss.feature != IMAGE_CONTENT_TRANSFORM_FEATURE);
        let mut package = package_for(&placement);

        assert!(matches!(
            add_embedded_images_to_idml(&export_plan, &mut package, &[placement]),
            Err(IdmlImageError::MissingContentTransformLoss { .. })
        ));
    }

    #[test]
    fn unsupported_mime_is_rejected() {
        let mut placement = placement();
        placement.mime = "image/x-ms-bmp-dib".into();
        let export_plan = plan(placement.node_id, placement.resource_id);
        let mut package = package_for(&placement);

        assert!(matches!(
            add_embedded_images_to_idml(&export_plan, &mut package, &[placement]),
            Err(IdmlImageError::UnsupportedMime { .. })
        ));
    }

    #[test]
    fn duplicate_node_placement_is_rejected() {
        let placement = placement();
        let export_plan = plan(placement.node_id, placement.resource_id);
        let mut package = package_for(&placement);

        assert!(matches!(
            add_embedded_images_to_idml(
                &export_plan,
                &mut package,
                &[placement.clone(), placement]
            ),
            Err(IdmlImageError::DuplicatePlacement { .. })
        ));
    }
}
