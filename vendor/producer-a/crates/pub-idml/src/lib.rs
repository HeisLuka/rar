//! Deterministic IDML package boundary.
//!
//! This crate is a target adapter layer. It consumes a target-neutral
//! `pub_export::ExportPlan` and does not depend on PUB source parser crates.
//! The first gate models deterministic package parts; semantic XML mapping and
//! final ZIP emission are layered on top of this boundary.

use pub_export::{ExportPlan, TargetProfile};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

mod image;
mod package_writer;
mod semantic;
mod shape;
mod table;

pub use image::{
    IMAGE_BYTES_FEATURE, IMAGE_CONTENT_TRANSFORM_FEATURE, IMAGE_FRAME_GEOMETRY_FEATURE,
    IdmlEmbeddedImagePlacement, IdmlImageError, add_embedded_images_to_idml,
};
pub use package_writer::{
    IDML_CONTAINER_PATH, IDML_MIMETYPE, IDML_MIMETYPE_PATH, IDML_ROOT_PATH, IdmlPackageWriteError,
    write_idml_ucf,
};
pub use semantic::{
    IDML_PACKAGING_NAMESPACE, IDML_SCHEMA_FENCE_LEGACY_DOM_7, IdmlSemanticError, IdmlWireProfile,
    project_resolved_graph_to_idml, project_resolved_graph_to_idml_with_tables,
};
pub use shape::{
    AUTHORED_SHAPE_GEOMETRY_FEATURE, AUTHORED_SHAPE_IDENTITY_FEATURE, AUTHORED_SHAPE_ORDER_FEATURE,
    AUTHORED_SHAPE_PAINT_FEATURE, IdmlAuthoredRectanglePlacement, IdmlAuthoredShapeError, IdmlRgb8,
    add_authored_rectangles_to_idml,
};
pub use table::{IdmlSimpleTable, IdmlTableCell, IdmlTableError};

pub const IDML_ADAPTER_VERSION_V0_1: &str = "idml-v0.1";
pub const IDML_FORMAT_PROFILE_ID: &str = "idml-bounded-v0.1";

pub fn format_profile()
-> Result<pub_format_registry::FormatProfileEntry, pub_format_registry::RegistryError> {
    pub_format_registry::resolve(IDML_FORMAT_PROFILE_ID)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdmlPartKind {
    DesignMap,
    Resource,
    MasterSpread,
    Spread,
    Story,
    Xml,
}

/// Payload of one logical IDML package part.
///
/// XML/text parts remain UTF-8 strings while embedded resources may carry
/// arbitrary bytes without base64 or UTF-8 coercion.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "encoding", content = "data", rename_all = "snake_case")]
pub enum IdmlPartContent {
    Text(String),
    Binary(Vec<u8>),
}

impl IdmlPartContent {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Text(value) => value.as_bytes(),
            Self::Binary(value) => value.as_slice(),
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value.as_str()),
            Self::Binary(_) => None,
        }
    }

    pub fn is_binary(&self) -> bool {
        matches!(self, Self::Binary(_))
    }

    pub fn contains(&self, pattern: &str) -> bool {
        self.as_text().is_some_and(|text| text.contains(pattern))
    }

    pub fn matches<'a>(&'a self, pattern: &'a str) -> Box<dyn Iterator<Item = &'a str> + 'a> {
        match self {
            Self::Text(value) => Box::new(value.matches(pattern)),
            Self::Binary(_) => Box::new(std::iter::empty()),
        }
    }

    pub fn find(&self, pattern: &str) -> Option<usize> {
        self.as_text().and_then(|text| text.find(pattern))
    }
}

impl From<String> for IdmlPartContent {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for IdmlPartContent {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct IdmlPart {
    pub path: String,
    pub kind: IdmlPartKind,
    pub content: IdmlPartContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdmlPackage {
    pub target: TargetProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversion_fence: Option<pub_export::ConversionFenceIdentity>,
    pub parts: Vec<IdmlPart>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdmlPackageError {
    TargetFormatMismatch { found: String },
    ExportBlocked,
    EmptyPath,
    AbsolutePath { path: String },
    ParentTraversal { path: String },
    DuplicatePath { path: String },
    MissingDesignMap,
    MultipleDesignMaps,
    InvalidDesignMapPath { path: String },
    BinaryDesignMap,
}

impl fmt::Display for IdmlPackageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TargetFormatMismatch { found } => {
                write!(formatter, "ожидался IDML target, найден {found}")
            }
            Self::ExportBlocked => formatter.write_str("export plan содержит blocking losses"),
            Self::EmptyPath => formatter.write_str("IDML package part path не может быть пустым"),
            Self::AbsolutePath { path } => {
                write!(
                    formatter,
                    "IDML package part path должен быть относительным: {path}"
                )
            }
            Self::ParentTraversal { path } => {
                write!(
                    formatter,
                    "IDML package part path содержит parent traversal: {path}"
                )
            }
            Self::DuplicatePath { path } => {
                write!(
                    formatter,
                    "IDML package содержит duplicate part path: {path}"
                )
            }
            Self::MissingDesignMap => formatter.write_str("IDML package требует designmap.xml"),
            Self::MultipleDesignMaps => {
                formatter.write_str("IDML package допускает ровно один designmap.xml")
            }
            Self::InvalidDesignMapPath { path } => write!(
                formatter,
                "design map должен находиться по пути designmap.xml, найден {path}"
            ),
            Self::BinaryDesignMap => {
                formatter.write_str("designmap.xml должен быть UTF-8 text part, а не binary")
            }
        }
    }
}

impl std::error::Error for IdmlPackageError {}

#[derive(Debug, Clone)]
pub struct IdmlPackageBuilder {
    target: TargetProfile,
    conversion_fence: Option<pub_export::ConversionFenceIdentity>,
    parts: Vec<IdmlPart>,
}

impl IdmlPackageBuilder {
    pub fn from_export_plan(plan: &ExportPlan) -> Result<Self, IdmlPackageError> {
        if plan.target.format != "idml" {
            return Err(IdmlPackageError::TargetFormatMismatch {
                found: plan.target.format.clone(),
            });
        }
        if !plan.can_serialize() {
            return Err(IdmlPackageError::ExportBlocked);
        }

        Ok(Self {
            target: plan.target.clone(),
            conversion_fence: plan.conversion_fence.clone(),
            parts: Vec::new(),
        })
    }

    /// Add a UTF-8 XML/text part.
    pub fn add_part(
        &mut self,
        path: impl Into<String>,
        kind: IdmlPartKind,
        content: impl Into<String>,
    ) -> Result<(), IdmlPackageError> {
        self.add_payload(path, kind, IdmlPartContent::Text(content.into()))
    }

    /// Add an arbitrary binary resource part without text coercion.
    pub fn add_binary_part(
        &mut self,
        path: impl Into<String>,
        kind: IdmlPartKind,
        content: impl Into<Vec<u8>>,
    ) -> Result<(), IdmlPackageError> {
        self.add_payload(path, kind, IdmlPartContent::Binary(content.into()))
    }

    fn add_payload(
        &mut self,
        path: impl Into<String>,
        kind: IdmlPartKind,
        content: IdmlPartContent,
    ) -> Result<(), IdmlPackageError> {
        let path = path.into();
        validate_relative_part_path(&path)?;

        if kind == IdmlPartKind::DesignMap && path != "designmap.xml" {
            return Err(IdmlPackageError::InvalidDesignMapPath { path });
        }
        if path == "designmap.xml" && content.is_binary() {
            return Err(IdmlPackageError::BinaryDesignMap);
        }

        if self.parts.iter().any(|part| part.path == path) {
            return Err(IdmlPackageError::DuplicatePath { path });
        }

        self.parts.push(IdmlPart {
            path,
            kind,
            content,
        });
        Ok(())
    }

    pub fn finish(mut self) -> Result<IdmlPackage, IdmlPackageError> {
        let design_maps = self
            .parts
            .iter()
            .filter(|part| part.kind == IdmlPartKind::DesignMap)
            .count();

        match design_maps {
            0 => return Err(IdmlPackageError::MissingDesignMap),
            1 => {}
            _ => return Err(IdmlPackageError::MultipleDesignMaps),
        }

        self.parts.sort();
        Ok(IdmlPackage {
            target: self.target,
            conversion_fence: self.conversion_fence,
            parts: self.parts,
        })
    }
}

impl IdmlPackage {
    pub fn part_paths(&self) -> impl Iterator<Item = &str> {
        self.parts.iter().map(|part| part.path.as_str())
    }

    pub fn validate_unique_paths(&self) -> bool {
        let mut seen = BTreeSet::new();
        self.parts.iter().all(|part| seen.insert(&part.path))
    }
}

fn validate_relative_part_path(path: &str) -> Result<(), IdmlPackageError> {
    if path.is_empty() {
        return Err(IdmlPackageError::EmptyPath);
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(IdmlPackageError::AbsolutePath {
            path: path.to_owned(),
        });
    }

    if path.split(['/', '\\']).any(|component| component == "..") {
        return Err(IdmlPackageError::ParentTraversal {
            path: path.to_owned(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pub_export::{
        CapabilityLevel, SemanticFeatureRequest, TargetCapabilityManifest, plan_export,
    };
    use std::collections::BTreeMap;

    fn export_plan(format: &str, blocked: bool) -> ExportPlan {
        let target = TargetProfile {
            format: format.into(),
            adapter_version: IDML_ADAPTER_VERSION_V0_1.into(),
            profile: "bounded-editable".into(),
            schema_fence: Some("legacy-spec-8.02".into()),
        };
        let mut features = BTreeMap::new();
        if !blocked {
            features.insert("page.geometry".into(), CapabilityLevel::Preserved);
        }
        let manifest = TargetCapabilityManifest { target, features };

        plan_export(
            &manifest,
            vec![SemanticFeatureRequest {
                feature: "page.geometry".into(),
                origin: None,
                property_path: None,
                require_preserved: true,
            }],
        )
    }

    #[test]
    fn rejects_non_idml_plan() {
        let plan = export_plan("odg", false);
        assert!(matches!(
            IdmlPackageBuilder::from_export_plan(&plan),
            Err(IdmlPackageError::TargetFormatMismatch { .. })
        ));
    }

    #[test]
    fn rejects_blocked_export_plan() {
        let plan = export_plan("idml", true);
        assert!(matches!(
            IdmlPackageBuilder::from_export_plan(&plan),
            Err(IdmlPackageError::ExportBlocked)
        ));
    }

    #[test]
    fn requires_exact_designmap_path() {
        let plan = export_plan("idml", false);
        let mut builder =
            IdmlPackageBuilder::from_export_plan(&plan).expect("IDML plan should be accepted");

        assert!(matches!(
            builder.add_part(
                "Resources/designmap.xml",
                IdmlPartKind::DesignMap,
                "<Document/>"
            ),
            Err(IdmlPackageError::InvalidDesignMapPath { .. })
        ));
    }

    #[test]
    fn package_parts_are_deterministic_by_path() {
        let plan = export_plan("idml", false);
        let mut builder =
            IdmlPackageBuilder::from_export_plan(&plan).expect("IDML plan should be accepted");

        builder
            .add_part(
                "Stories/Story_u2.xml",
                IdmlPartKind::Story,
                "<Story Self=\"u2\"/>",
            )
            .expect("story should be accepted");
        builder
            .add_part("designmap.xml", IdmlPartKind::DesignMap, "<Document/>")
            .expect("designmap should be accepted");
        builder
            .add_part(
                "Spreads/Spread_u1.xml",
                IdmlPartKind::Spread,
                "<Spread Self=\"u1\"/>",
            )
            .expect("spread should be accepted");

        let package = builder.finish().expect("package should be valid");
        assert_eq!(
            package.part_paths().collect::<Vec<_>>(),
            vec![
                "Spreads/Spread_u1.xml",
                "Stories/Story_u2.xml",
                "designmap.xml",
            ]
        );
        assert!(package.validate_unique_paths());
    }

    #[test]
    fn binary_part_preserves_arbitrary_bytes() {
        let plan = export_plan("idml", false);
        let mut builder =
            IdmlPackageBuilder::from_export_plan(&plan).expect("IDML plan should be accepted");

        builder
            .add_part("designmap.xml", IdmlPartKind::DesignMap, "<Document/>")
            .expect("designmap should be accepted");
        let bytes = vec![0x00, 0xff, 0x89, 0x50, 0x4e, 0x47, 0x80];
        builder
            .add_binary_part("Links/image.bin", IdmlPartKind::Resource, bytes.clone())
            .expect("binary resource should be accepted");

        let package = builder.finish().expect("package should be valid");
        let part = package
            .parts
            .iter()
            .find(|part| part.path == "Links/image.bin")
            .expect("binary part");
        assert_eq!(part.content.as_bytes(), bytes.as_slice());
        assert!(part.content.is_binary());
        assert_eq!(part.content.as_text(), None);
    }

    #[test]
    fn designmap_cannot_be_binary() {
        let plan = export_plan("idml", false);
        let mut builder =
            IdmlPackageBuilder::from_export_plan(&plan).expect("IDML plan should be accepted");

        assert!(matches!(
            builder.add_binary_part("designmap.xml", IdmlPartKind::DesignMap, vec![0xff, 0x00]),
            Err(IdmlPackageError::BinaryDesignMap)
        ));
    }

    #[test]
    fn rejects_duplicate_or_traversing_paths() {
        let plan = export_plan("idml", false);
        let mut builder =
            IdmlPackageBuilder::from_export_plan(&plan).expect("IDML plan should be accepted");

        builder
            .add_part("designmap.xml", IdmlPartKind::DesignMap, "<Document/>")
            .expect("designmap should be accepted");
        assert!(matches!(
            builder.add_part("designmap.xml", IdmlPartKind::Xml, "<Other/>"),
            Err(IdmlPackageError::DuplicatePath { .. })
        ));
        assert!(matches!(
            builder.add_part("../evil.xml", IdmlPartKind::Xml, "<Other/>"),
            Err(IdmlPackageError::ParentTraversal { .. })
        ));
    }
}
