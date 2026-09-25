//! Deterministic ODG package boundary.
//!
//! The adapter consumes a target-neutral ExportPlan and emits a bounded
//! OpenDocument Graphics package. It intentionally has no dependency on PUB
//! parser/raw crates.

use pub_export::{ExportPlan, TargetProfile};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::io::{Cursor, Read, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

mod semantic;

pub use semantic::{OdgSemanticError, project_resolved_graph_to_odg};

pub const ODG_ADAPTER_VERSION_V0_1: &str = "odg-v0.1";
pub const ODG_FORMAT_PROFILE_ID: &str = "odg-bounded-v0.1";

pub fn format_profile()
-> Result<pub_format_registry::FormatProfileEntry, pub_format_registry::RegistryError> {
    pub_format_registry::resolve(ODG_FORMAT_PROFILE_ID)
}
pub const ODG_SCHEMA_FENCE_ODF_1_4: &str = "odf-1.4";
pub const ODG_MIMETYPE_PATH: &str = "mimetype";
pub const ODG_MIMETYPE: &str = "application/vnd.oasis.opendocument.graphics";
pub const ODG_CONTENT_PATH: &str = "content.xml";
pub const ODG_MANIFEST_PATH: &str = "META-INF/manifest.xml";

const MANIFEST_NAMESPACE: &str = "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OdgPartKind {
    Content,
    Styles,
    Meta,
    Settings,
    Resource,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OdgPart {
    pub path: String,
    pub kind: OdgPartKind,
    pub media_type: String,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OdgPackage {
    pub target: TargetProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversion_fence: Option<pub_export::ConversionFenceIdentity>,
    pub parts: Vec<OdgPart>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OdgPackageError {
    TargetFormatMismatch { found: String },
    SchemaFenceMismatch { found: Option<String> },
    ExportBlocked,
    EmptyPath,
    InvalidUnixPath { path: String },
    ReservedPath { path: String },
    DuplicatePath { path: String },
    MissingContent,
    MultipleContentParts,
    ContentPathMismatch { path: String },
}

impl fmt::Display for OdgPackageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TargetFormatMismatch { found } => {
                write!(formatter, "expected ODG target, found {found}")
            }
            Self::SchemaFenceMismatch { found } => {
                write!(formatter, "expected ODF 1.4 schema fence, found {found:?}")
            }
            Self::ExportBlocked => formatter.write_str("export plan contains blocking losses"),
            Self::EmptyPath => formatter.write_str("ODG package part path cannot be empty"),
            Self::InvalidUnixPath { path } => {
                write!(
                    formatter,
                    "ODG package path is not a safe relative UNIX path: {path}"
                )
            }
            Self::ReservedPath { path } => {
                write!(formatter, "ODG package part uses reserved path: {path}")
            }
            Self::DuplicatePath { path } => {
                write!(formatter, "ODG package contains duplicate path: {path}")
            }
            Self::MissingContent => formatter.write_str("ODG package requires content.xml"),
            Self::MultipleContentParts => {
                formatter.write_str("ODG package allows exactly one content.xml")
            }
            Self::ContentPathMismatch { path } => write!(
                formatter,
                "ODG content part must use content.xml path, found {path}"
            ),
        }
    }
}

impl std::error::Error for OdgPackageError {}

#[derive(Debug, Clone)]
pub struct OdgPackageBuilder {
    target: TargetProfile,
    conversion_fence: Option<pub_export::ConversionFenceIdentity>,
    parts: Vec<OdgPart>,
}

impl OdgPackageBuilder {
    pub fn from_export_plan(plan: &ExportPlan) -> Result<Self, OdgPackageError> {
        if plan.target.format != "odg" {
            return Err(OdgPackageError::TargetFormatMismatch {
                found: plan.target.format.clone(),
            });
        }
        if plan.target.schema_fence.as_deref() != Some(ODG_SCHEMA_FENCE_ODF_1_4) {
            return Err(OdgPackageError::SchemaFenceMismatch {
                found: plan.target.schema_fence.clone(),
            });
        }
        if !plan.can_serialize() {
            return Err(OdgPackageError::ExportBlocked);
        }

        Ok(Self {
            target: plan.target.clone(),
            conversion_fence: plan.conversion_fence.clone(),
            parts: Vec::new(),
        })
    }

    pub fn add_xml_part(
        &mut self,
        path: impl Into<String>,
        kind: OdgPartKind,
        content: impl Into<String>,
    ) -> Result<(), OdgPackageError> {
        self.add_part(path, kind, "text/xml", content.into().into_bytes())
    }

    pub fn add_part(
        &mut self,
        path: impl Into<String>,
        kind: OdgPartKind,
        media_type: impl Into<String>,
        content: Vec<u8>,
    ) -> Result<(), OdgPackageError> {
        let path = path.into();
        validate_part_path(&path)?;

        if matches!(path.as_str(), ODG_MIMETYPE_PATH | ODG_MANIFEST_PATH) {
            return Err(OdgPackageError::ReservedPath { path });
        }
        if kind == OdgPartKind::Content && path != ODG_CONTENT_PATH {
            return Err(OdgPackageError::ContentPathMismatch { path });
        }
        if self.parts.iter().any(|part| part.path == path) {
            return Err(OdgPackageError::DuplicatePath { path });
        }

        self.parts.push(OdgPart {
            path,
            kind,
            media_type: media_type.into(),
            content,
        });
        Ok(())
    }

    pub fn finish(mut self) -> Result<OdgPackage, OdgPackageError> {
        let content_parts = self
            .parts
            .iter()
            .filter(|part| part.kind == OdgPartKind::Content || part.path == ODG_CONTENT_PATH)
            .count();

        match content_parts {
            0 => return Err(OdgPackageError::MissingContent),
            1 => {}
            _ => return Err(OdgPackageError::MultipleContentParts),
        }

        self.parts.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(OdgPackage {
            target: self.target,
            conversion_fence: self.conversion_fence,
            parts: self.parts,
        })
    }
}

#[derive(Debug)]
pub enum OdgWriteError {
    Package(OdgPackageError),
    Zip(zip::result::ZipError),
    Io(std::io::Error),
    InvalidWrittenArchive(String),
}

impl fmt::Display for OdgWriteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Package(error) => write!(formatter, "{error}"),
            Self::Zip(error) => write!(formatter, "ZIP error: {error}"),
            Self::Io(error) => write!(formatter, "ODG I/O error: {error}"),
            Self::InvalidWrittenArchive(message) => {
                write!(formatter, "written ODG failed self-validation: {message}")
            }
        }
    }
}

impl std::error::Error for OdgWriteError {}

impl From<OdgPackageError> for OdgWriteError {
    fn from(value: OdgPackageError) -> Self {
        Self::Package(value)
    }
}

impl From<zip::result::ZipError> for OdgWriteError {
    fn from(value: zip::result::ZipError) -> Self {
        Self::Zip(value)
    }
}

impl From<std::io::Error> for OdgWriteError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

pub fn write_odg(package: &OdgPackage) -> Result<Vec<u8>, OdgWriteError> {
    validate_package(package)?;

    let cursor = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(cursor);
    let stored = deterministic_stored_options();

    writer.start_file(ODG_MIMETYPE_PATH, stored)?;
    writer.write_all(ODG_MIMETYPE.as_bytes())?;

    writer.start_file(ODG_MANIFEST_PATH, stored)?;
    writer.write_all(manifest_xml(package).as_bytes())?;

    for part in &package.parts {
        writer.start_file(&part.path, stored)?;
        writer.write_all(&part.content)?;
    }

    let bytes = writer.finish()?.into_inner();
    validate_written_odg(&bytes)?;
    Ok(bytes)
}

fn deterministic_stored_options() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(DateTime::default())
        .unix_permissions(0o644)
}

fn validate_package(package: &OdgPackage) -> Result<(), OdgPackageError> {
    let mut seen = BTreeSet::new();
    let mut content_parts = 0_usize;

    for part in &package.parts {
        validate_part_path(&part.path)?;
        if matches!(part.path.as_str(), ODG_MIMETYPE_PATH | ODG_MANIFEST_PATH) {
            return Err(OdgPackageError::ReservedPath {
                path: part.path.clone(),
            });
        }
        if !seen.insert(part.path.as_str()) {
            return Err(OdgPackageError::DuplicatePath {
                path: part.path.clone(),
            });
        }
        if part.kind == OdgPartKind::Content || part.path == ODG_CONTENT_PATH {
            content_parts += 1;
        }
    }

    match content_parts {
        0 => Err(OdgPackageError::MissingContent),
        1 => Ok(()),
        _ => Err(OdgPackageError::MultipleContentParts),
    }
}

fn validate_part_path(path: &str) -> Result<(), OdgPackageError> {
    if path.is_empty() {
        return Err(OdgPackageError::EmptyPath);
    }
    if path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path.contains("//")
        || path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(OdgPackageError::InvalidUnixPath {
            path: path.to_owned(),
        });
    }
    Ok(())
}

fn manifest_xml(package: &OdgPackage) -> String {
    let mut entries = package.parts.iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.path.cmp(&right.path));

    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str(&format!(
        "<manifest:manifest xmlns:manifest=\"{MANIFEST_NAMESPACE}\" manifest:version=\"1.4\">\n"
    ));
    xml.push_str(&format!(
        "  <manifest:file-entry manifest:full-path=\"/\" manifest:media-type=\"{ODG_MIMETYPE}\"/>\n"
    ));

    for part in entries {
        xml.push_str(&format!(
            "  <manifest:file-entry manifest:full-path=\"{}\" manifest:media-type=\"{}\"/>\n",
            escape_xml_attr(&part.path),
            escape_xml_attr(&part.media_type)
        ));
    }

    xml.push_str("</manifest:manifest>\n");
    xml
}

fn escape_xml_attr(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}

fn validate_written_odg(bytes: &[u8]) -> Result<(), OdgWriteError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;
    if archive.len() < 3 {
        return Err(OdgWriteError::InvalidWrittenArchive(
            "archive has fewer than mimetype, manifest and content.xml".into(),
        ));
    }

    {
        let mut mimetype = archive.by_index(0)?;
        if mimetype.name() != ODG_MIMETYPE_PATH {
            return Err(OdgWriteError::InvalidWrittenArchive(format!(
                "first entry is {}, expected mimetype",
                mimetype.name()
            )));
        }
        if mimetype.compression() != CompressionMethod::Stored {
            return Err(OdgWriteError::InvalidWrittenArchive(
                "mimetype is compressed".into(),
            ));
        }
        let mut content = String::new();
        mimetype.read_to_string(&mut content)?;
        if content != ODG_MIMETYPE {
            return Err(OdgWriteError::InvalidWrittenArchive(
                "mimetype payload is not exact".into(),
            ));
        }
    }

    {
        let mut manifest = archive.by_name(ODG_MANIFEST_PATH)?;
        let mut xml = String::new();
        manifest.read_to_string(&mut xml)?;
        if !xml.contains(ODG_MIMETYPE)
            || !xml.contains("manifest:full-path=\"content.xml\"")
            || !xml.contains("manifest:version=\"1.4\"")
        {
            return Err(OdgWriteError::InvalidWrittenArchive(
                "manifest does not declare ODG root/content/version".into(),
            ));
        }
    }

    archive.by_name(ODG_CONTENT_PATH)?;
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
            adapter_version: ODG_ADAPTER_VERSION_V0_1.into(),
            profile: "bounded-editable".into(),
            schema_fence: Some(ODG_SCHEMA_FENCE_ODF_1_4.into()),
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
    fn rejects_non_odg_or_blocked_plan() {
        assert!(matches!(
            OdgPackageBuilder::from_export_plan(&export_plan("idml", false)),
            Err(OdgPackageError::TargetFormatMismatch { .. })
        ));
        assert!(matches!(
            OdgPackageBuilder::from_export_plan(&export_plan("odg", true)),
            Err(OdgPackageError::ExportBlocked)
        ));
    }

    #[test]
    fn requires_odf_1_4_fence() {
        let mut plan = export_plan("odg", false);
        plan.target.schema_fence = Some("odf-1.3".into());
        assert!(matches!(
            OdgPackageBuilder::from_export_plan(&plan),
            Err(OdgPackageError::SchemaFenceMismatch { .. })
        ));
    }

    #[test]
    fn writes_deterministic_odg_with_exact_mimetype_first() {
        let plan = export_plan("odg", false);
        let mut builder = OdgPackageBuilder::from_export_plan(&plan).expect("valid ODG plan");
        builder
            .add_xml_part(
                ODG_CONTENT_PATH,
                OdgPartKind::Content,
                "<office:document-content/>",
            )
            .unwrap();
        builder
            .add_xml_part(
                "styles.xml",
                OdgPartKind::Styles,
                "<office:document-styles/>",
            )
            .unwrap();

        let package = builder.finish().expect("valid logical ODG package");
        let first = write_odg(&package).expect("first ODG write");
        let second = write_odg(&package).expect("second ODG write");
        assert_eq!(first, second);

        let mut archive = ZipArchive::new(Cursor::new(first)).unwrap();
        assert_eq!(archive.by_index(0).unwrap().name(), ODG_MIMETYPE_PATH);
        assert!(archive.by_name(ODG_MANIFEST_PATH).is_ok());
        assert!(archive.by_name(ODG_CONTENT_PATH).is_ok());
    }

    #[test]
    fn rejects_reserved_duplicate_and_unsafe_paths() {
        let plan = export_plan("odg", false);
        let mut builder = OdgPackageBuilder::from_export_plan(&plan).expect("valid ODG plan");

        assert!(matches!(
            builder.add_xml_part(ODG_MIMETYPE_PATH, OdgPartKind::Meta, "x"),
            Err(OdgPackageError::ReservedPath { .. })
        ));
        assert!(matches!(
            builder.add_xml_part("../content.xml", OdgPartKind::Content, "x"),
            Err(OdgPackageError::InvalidUnixPath { .. })
        ));

        builder
            .add_xml_part(ODG_CONTENT_PATH, OdgPartKind::Content, "one")
            .unwrap();
        assert!(matches!(
            builder.add_xml_part(ODG_CONTENT_PATH, OdgPartKind::Meta, "two"),
            Err(OdgPackageError::DuplicatePath { .. })
        ));
    }
}
