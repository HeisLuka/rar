use crate::{IdmlPackage, IdmlPartKind};
use std::collections::BTreeSet;
use std::fmt;
use std::io::{Cursor, Read, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

pub const IDML_MIMETYPE_PATH: &str = "mimetype";
pub const IDML_MIMETYPE: &str = "application/vnd.adobe.indesign-idml-package";
pub const IDML_CONTAINER_PATH: &str = "META-INF/container.xml";
pub const IDML_ROOT_PATH: &str = "designmap.xml";

const IDML_CONTAINER_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n\
  <rootfiles>\n\
    <rootfile full-path=\"designmap.xml\" media-type=\"text/xml\">\n\
    </rootfile>\n\
  </rootfiles>\n\
</container>\n";

#[derive(Debug)]
pub enum IdmlPackageWriteError {
    MissingDesignMap,
    MultipleDesignMaps,
    ReservedPath { path: String },
    DuplicatePath { path: String },
    InvalidUnixPath { path: String },
    Zip(zip::result::ZipError),
    Io(std::io::Error),
    InvalidWrittenArchive(String),
}

impl fmt::Display for IdmlPackageWriteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingDesignMap => {
                formatter.write_str("IDML physical package requires designmap.xml")
            }
            Self::MultipleDesignMaps => {
                formatter.write_str("IDML physical package requires exactly one designmap.xml")
            }
            Self::ReservedPath { path } => {
                write!(
                    formatter,
                    "IDML package part uses reserved UCF path: {path}"
                )
            }
            Self::DuplicatePath { path } => {
                write!(
                    formatter,
                    "IDML physical package contains duplicate path: {path}"
                )
            }
            Self::InvalidUnixPath { path } => write!(
                formatter,
                "IDML physical package path must be a safe relative UNIX path: {path}"
            ),
            Self::Zip(error) => write!(formatter, "ZIP write/read error: {error}"),
            Self::Io(error) => write!(formatter, "ZIP I/O error: {error}"),
            Self::InvalidWrittenArchive(message) => {
                write!(
                    formatter,
                    "written IDML archive failed self-validation: {message}"
                )
            }
        }
    }
}

impl std::error::Error for IdmlPackageWriteError {}

impl From<zip::result::ZipError> for IdmlPackageWriteError {
    fn from(value: zip::result::ZipError) -> Self {
        Self::Zip(value)
    }
}

impl From<std::io::Error> for IdmlPackageWriteError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

/// Serialize a logical IDML package into a deterministic UCF/ZIP container.
///
/// Archive order is fixed:
/// 1. `mimetype` (Stored, exact bytes, no newline)
/// 2. `META-INF/container.xml`
/// 3. semantic package parts sorted by path.
///
/// All entries use Stored compression intentionally. UCF only requires the
/// mimetype entry to be stored, but using one deterministic method for this
/// bounded writer removes compression-library/version variance from golden
/// artifacts.
pub fn write_idml_ucf(package: &IdmlPackage) -> Result<Vec<u8>, IdmlPackageWriteError> {
    validate_logical_package(package)?;

    let cursor = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(cursor);
    let options = deterministic_stored_options();

    writer.start_file(IDML_MIMETYPE_PATH, options)?;
    writer.write_all(IDML_MIMETYPE.as_bytes())?;

    writer.start_file(IDML_CONTAINER_PATH, options)?;
    writer.write_all(IDML_CONTAINER_XML.as_bytes())?;

    let mut parts = package.parts.iter().collect::<Vec<_>>();
    parts.sort_by(|left, right| left.path.cmp(&right.path));

    for part in parts {
        writer.start_file(&part.path, options)?;
        writer.write_all(part.content.as_bytes())?;
    }

    let cursor = writer.finish()?;
    let bytes = cursor.into_inner();
    validate_written_ucf(&bytes)?;
    Ok(bytes)
}

fn deterministic_stored_options() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(DateTime::default())
        .unix_permissions(0o644)
}

fn validate_logical_package(package: &IdmlPackage) -> Result<(), IdmlPackageWriteError> {
    let designmaps = package
        .parts
        .iter()
        .filter(|part| part.kind == IdmlPartKind::DesignMap || part.path == IDML_ROOT_PATH)
        .count();

    match designmaps {
        0 => return Err(IdmlPackageWriteError::MissingDesignMap),
        1 => {}
        _ => return Err(IdmlPackageWriteError::MultipleDesignMaps),
    }

    let mut seen = BTreeSet::new();
    for part in &package.parts {
        if !seen.insert(part.path.as_str()) {
            return Err(IdmlPackageWriteError::DuplicatePath {
                path: part.path.clone(),
            });
        }
        if matches!(part.path.as_str(), IDML_MIMETYPE_PATH | IDML_CONTAINER_PATH) {
            return Err(IdmlPackageWriteError::ReservedPath {
                path: part.path.clone(),
            });
        }
        if !is_safe_unix_path(&part.path) {
            return Err(IdmlPackageWriteError::InvalidUnixPath {
                path: part.path.clone(),
            });
        }
    }

    Ok(())
}

fn is_safe_unix_path(path: &str) -> bool {
    if path.is_empty()
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains('\\')
        || path.contains("//")
    {
        return false;
    }

    !path
        .split('/')
        .any(|component| component.is_empty() || component == "." || component == "..")
}

fn validate_written_ucf(bytes: &[u8]) -> Result<(), IdmlPackageWriteError> {
    let cursor = Cursor::new(bytes);
    let mut archive = ZipArchive::new(cursor)?;

    if archive.len() < 3 {
        return Err(IdmlPackageWriteError::InvalidWrittenArchive(
            "archive has fewer than mimetype, container.xml, and designmap".into(),
        ));
    }

    {
        let mut first = archive.by_index(0)?;
        if first.name() != IDML_MIMETYPE_PATH {
            return Err(IdmlPackageWriteError::InvalidWrittenArchive(format!(
                "first entry is {}, expected mimetype",
                first.name()
            )));
        }
        if first.compression() != CompressionMethod::Stored {
            return Err(IdmlPackageWriteError::InvalidWrittenArchive(
                "mimetype is compressed".into(),
            ));
        }
        let mut content = String::new();
        first.read_to_string(&mut content)?;
        if content != IDML_MIMETYPE {
            return Err(IdmlPackageWriteError::InvalidWrittenArchive(
                "mimetype payload is not exact".into(),
            ));
        }
    }

    {
        let mut container = archive.by_name(IDML_CONTAINER_PATH)?;
        let mut xml = String::new();
        container.read_to_string(&mut xml)?;
        if !xml.contains("full-path=\"designmap.xml\"") || !xml.contains("media-type=\"text/xml\"")
        {
            return Err(IdmlPackageWriteError::InvalidWrittenArchive(
                "container.xml does not point to designmap.xml as text/xml".into(),
            ));
        }
    }

    archive.by_name(IDML_ROOT_PATH)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IdmlPart, IdmlPartContent};
    use pub_export::TargetProfile;

    fn package(parts: Vec<IdmlPart>) -> IdmlPackage {
        IdmlPackage {
            target: TargetProfile {
                format: "idml".into(),
                adapter_version: "idml-v0.1".into(),
                profile: "bounded-editable".into(),
                schema_fence: Some("legacy-spec-8.02/dom-7.0".into()),
            },
            conversion_fence: None,
            parts,
        }
    }

    fn part(path: &str, kind: IdmlPartKind, content: &str) -> IdmlPart {
        IdmlPart {
            path: path.into(),
            kind,
            content: IdmlPartContent::Text(content.into()),
        }
    }

    #[test]
    fn writes_deterministic_ucf_with_exact_mimetype_first() {
        let logical = package(vec![
            part(
                "Stories/Story_u2.xml",
                IdmlPartKind::Story,
                "<idPkg:Story/>",
            ),
            part("designmap.xml", IdmlPartKind::DesignMap, "<Document/>"),
            part(
                "Spreads/Spread_u1.xml",
                IdmlPartKind::Spread,
                "<idPkg:Spread/>",
            ),
        ]);

        let first = write_idml_ucf(&logical).expect("first IDML write");
        let second = write_idml_ucf(&logical).expect("second IDML write");
        assert_eq!(first, second, "same logical package must be byte-identical");

        let mut archive = ZipArchive::new(Cursor::new(first)).expect("read emitted IDML zip");
        assert_eq!(archive.len(), 5);

        let mut mimetype = archive.by_index(0).expect("mimetype at index zero");
        assert_eq!(mimetype.name(), IDML_MIMETYPE_PATH);
        assert_eq!(mimetype.compression(), CompressionMethod::Stored);
        let mut mime_text = String::new();
        mimetype.read_to_string(&mut mime_text).unwrap();
        assert_eq!(mime_text, IDML_MIMETYPE);
        drop(mimetype);

        let container = archive
            .by_name(IDML_CONTAINER_PATH)
            .expect("container.xml must exist");
        assert_eq!(container.compression(), CompressionMethod::Stored);
        drop(container);

        assert!(archive.by_name("designmap.xml").is_ok());
        assert!(archive.by_name("Spreads/Spread_u1.xml").is_ok());
        assert!(archive.by_name("Stories/Story_u2.xml").is_ok());
    }

    #[test]
    fn sorts_semantic_parts_after_reserved_ucf_entries() {
        let logical = package(vec![
            part("z.xml", IdmlPartKind::Xml, "z"),
            part("designmap.xml", IdmlPartKind::DesignMap, "d"),
            part("a.xml", IdmlPartKind::Xml, "a"),
        ]);
        let bytes = write_idml_ucf(&logical).expect("write sorted package");
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();

        assert_eq!(archive.by_index(0).unwrap().name(), "mimetype");
        assert_eq!(
            archive.by_index(1).unwrap().name(),
            "META-INF/container.xml"
        );
        assert_eq!(archive.by_index(2).unwrap().name(), "a.xml");
        assert_eq!(archive.by_index(3).unwrap().name(), "designmap.xml");
        assert_eq!(archive.by_index(4).unwrap().name(), "z.xml");
    }

    #[test]
    fn rejects_reserved_and_non_unix_paths() {
        for invalid in [
            "mimetype",
            "META-INF/container.xml",
            "Stories\\Story.xml",
            "../designmap.xml",
            "/designmap.xml",
        ] {
            let logical = package(vec![
                part("designmap.xml", IdmlPartKind::DesignMap, "<Document/>"),
                part(invalid, IdmlPartKind::Xml, "x"),
            ]);

            assert!(
                write_idml_ucf(&logical).is_err(),
                "path should be rejected: {invalid}"
            );
        }
    }

    #[test]
    fn writes_binary_resource_bytes_without_utf8_coercion() {
        let binary = vec![0x00, 0xff, 0x89, 0x50, 0x4e, 0x47, 0x80, 0x01];
        let logical = package(vec![
            part("designmap.xml", IdmlPartKind::DesignMap, "<Document/>"),
            IdmlPart {
                path: "Links/image.bin".into(),
                kind: IdmlPartKind::Resource,
                content: IdmlPartContent::Binary(binary.clone()),
            },
        ]);

        let bytes = write_idml_ucf(&logical).expect("write binary IDML resource");
        let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("read emitted IDML zip");
        let mut resource = archive.by_name("Links/image.bin").expect("binary resource");
        let mut roundtrip = Vec::new();
        resource
            .read_to_end(&mut roundtrip)
            .expect("read binary payload");
        assert_eq!(roundtrip, binary);
    }

    #[test]
    fn duplicate_semantic_path_is_rejected() {
        let logical = package(vec![
            part("designmap.xml", IdmlPartKind::DesignMap, "<Document/>"),
            part("Stories/Story.xml", IdmlPartKind::Story, "one"),
            part("Stories/Story.xml", IdmlPartKind::Story, "two"),
        ]);
        assert!(matches!(
            write_idml_ucf(&logical),
            Err(IdmlPackageWriteError::DuplicatePath { .. })
        ));
    }

    #[test]
    fn designmap_is_mandatory() {
        let logical = package(vec![part("Stories/Story.xml", IdmlPartKind::Story, "x")]);
        assert!(matches!(
            write_idml_ucf(&logical),
            Err(IdmlPackageWriteError::MissingDesignMap)
        ));
    }
}
