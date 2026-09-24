//! Versioned source/target format profile registry.
//!
//! This crate is deliberately descriptive, not authoritative format truth.
//! It centralizes the machine-readable identity/fence metadata that product
//! and conversion code must resolve instead of inferring support from scattered
//! code paths.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const FORMAT_REGISTRY_SCHEMA_V0_1: &str = "pub-format-registry-v0.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormatDirection {
    Source,
    Target,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatProfileEntry {
    pub profile_id: String,
    pub direction: FormatDirection,
    pub format: String,
    pub profile_version: String,
    pub specification: SpecificationReference,
    pub adapter: AdapterReference,
    pub capabilities: Vec<String>,
    pub validator: ValidatorReference,
    pub corpus: Vec<CorpusReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecificationReference {
    pub authority: String,
    pub title: String,
    pub version_fence: String,
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterReference {
    pub crate_name: String,
    pub implementation_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatorReference {
    pub profile_id: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusReference {
    pub fixture_id: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    UnknownProfile(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::UnknownProfile(id) => write!(f, "unknown format profile: {id}"),
        }
    }
}

impl std::error::Error for RegistryError {}

pub fn registry() -> BTreeMap<String, FormatProfileEntry> {
    let entries = [
        pub_source_profile(),
        idml_target_profile(),
        odg_target_profile(),
        pdf_target_profile(),
    ];
    entries
        .into_iter()
        .map(|entry| (entry.profile_id.clone(), entry))
        .collect()
}

pub fn resolve(profile_id: &str) -> Result<FormatProfileEntry, RegistryError> {
    registry()
        .remove(profile_id)
        .ok_or_else(|| RegistryError::UnknownProfile(profile_id.to_owned()))
}

pub fn snapshot() -> RegistrySnapshot {
    RegistrySnapshot {
        schema_version: FORMAT_REGISTRY_SCHEMA_V0_1.to_owned(),
        profiles: registry().into_values().collect(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrySnapshot {
    pub schema_version: String,
    pub profiles: Vec<FormatProfileEntry>,
}

fn pub_source_profile() -> FormatProfileEntry {
    FormatProfileEntry {
        profile_id: "pub-mature-0x2c-v0.1".into(),
        direction: FormatDirection::Source,
        format: "microsoft-publisher-pub".into(),
        profile_version: "0.1".into(),
        specification: SpecificationReference {
            authority: "PUB project canonical research".into(),
            title: "Bounded Publisher mature-0x2C source profile".into(),
            version_fence: "mature-0x2c-bounded".into(),
            reference: "project://pub/canonical/mature-0x2c".into(),
        },
        adapter: AdapterReference {
            crate_name: "pub-reader".into(),
            implementation_version: "pub-reader-v0.1".into(),
        },
        capabilities: vec![
            "inspect".into(),
            "viewer_scene".into(),
            "extract_assets".into(),
            "convert".into(),
        ],
        validator: ValidatorReference {
            profile_id: "pub-reader-mature-0x2c".into(),
            version: "v0.1".into(),
        },
        corpus: vec![CorpusReference {
            fixture_id: "SampleNewsletter.pub".into(),
            sha256: "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf".into(),
        }],
    }
}

fn idml_target_profile() -> FormatProfileEntry {
    FormatProfileEntry {
        profile_id: "idml-bounded-v0.1".into(),
        direction: FormatDirection::Target,
        format: "idml".into(),
        profile_version: "0.1".into(),
        specification: SpecificationReference {
            authority: "Adobe".into(),
            title: "InDesign Markup Language (IDML)".into(),
            version_fence: "bounded-package-profile".into(),
            reference: "https://developer.adobe.com/indesign/idml/".into(),
        },
        adapter: AdapterReference {
            crate_name: "pub-idml".into(),
            implementation_version: "idml-v0.1".into(),
        },
        capabilities: vec![
            "page.geometry".into(),
            "story.text".into(),
            "embedded.images".into(),
        ],
        validator: ValidatorReference {
            profile_id: "idml-package-structural".into(),
            version: "v0.1".into(),
        },
        corpus: vec![CorpusReference {
            fixture_id: "SampleNewsletter.pub".into(),
            sha256: "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf".into(),
        }],
    }
}

fn odg_target_profile() -> FormatProfileEntry {
    FormatProfileEntry {
        profile_id: "odg-bounded-v0.1".into(),
        direction: FormatDirection::Target,
        format: "odg".into(),
        profile_version: "0.1".into(),
        specification: SpecificationReference {
            authority: "OASIS".into(),
            title: "OpenDocument Format for Office Applications".into(),
            version_fence: "ODF-1.4-bounded".into(),
            reference: "https://docs.oasis-open.org/office/OpenDocument/v1.4/".into(),
        },
        adapter: AdapterReference {
            crate_name: "pub-odg".into(),
            implementation_version: "odg-v0.1".into(),
        },
        capabilities: vec![
            "page.geometry".into(),
            "story.text".into(),
            "embedded.images".into(),
        ],
        validator: ValidatorReference {
            profile_id: "odg-package-structural".into(),
            version: "v0.1".into(),
        },
        corpus: vec![CorpusReference {
            fixture_id: "SampleNewsletter.pub".into(),
            sha256: "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf".into(),
        }],
    }
}

fn pdf_target_profile() -> FormatProfileEntry {
    FormatProfileEntry {
        profile_id: "pdf-basic-fixed-v0.1".into(),
        direction: FormatDirection::Target,
        format: "pdf".into(),
        profile_version: "0.1".into(),
        specification: SpecificationReference {
            authority: "PUB project fixed-output contract".into(),
            title: "Bounded deterministic PDF 1.7 fixed-output profile".into(),
            version_fence: "PDF-1.7-bounded".into(),
            reference: "project://pub/pdf/basic-fixed-v0.1".into(),
        },
        adapter: AdapterReference {
            crate_name: "pub-pdf".into(),
            implementation_version: "fixed-pdf-v0.1".into(),
        },
        capabilities: vec![
            "page.geometry".into(),
            "story.text".into(),
            "embedded.images".into(),
            "shape.explicit_solid_paint".into(),
        ],
        validator: ValidatorReference {
            profile_id: "pdf-qpdf-pdfinfo".into(),
            version: "v0.1".into(),
        },
        corpus: vec![CorpusReference {
            fixture_id: "SampleNewsletter.pub".into(),
            sha256: "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf".into(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_current_pub_idml_odg_pdf_profiles() {
        for id in [
            "pub-mature-0x2c-v0.1",
            "idml-bounded-v0.1",
            "odg-bounded-v0.1",
            "pdf-basic-fixed-v0.1",
        ] {
            let entry = resolve(id).expect("known profile must resolve");
            assert_eq!(entry.profile_id, id);
            assert!(!entry.specification.reference.is_empty());
            assert!(!entry.adapter.crate_name.is_empty());
            assert!(!entry.validator.profile_id.is_empty());
            assert!(!entry.corpus.is_empty());
        }
    }

    #[test]
    fn unknown_profile_fails_closed() {
        let error = resolve("unknown-profile").expect_err("unknown profile must fail closed");
        assert_eq!(error.to_string(), "unknown format profile: unknown-profile");
    }

    #[test]
    fn snapshot_is_deterministic_and_sorted() {
        let one = serde_json::to_string(&snapshot()).expect("snapshot serializes");
        let two = serde_json::to_string(&snapshot()).expect("snapshot serializes");
        assert_eq!(one, two);

        let snapshot = snapshot();
        let ids: Vec<_> = snapshot
            .profiles
            .iter()
            .map(|entry| entry.profile_id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec![
                "idml-bounded-v0.1",
                "odg-bounded-v0.1",
                "pdf-basic-fixed-v0.1",
                "pub-mature-0x2c-v0.1"
            ]
        );
    }

    #[test]
    fn entries_do_not_hide_target_specific_defaults() {
        for entry in snapshot().profiles {
            assert!(!entry.profile_version.is_empty());
            assert!(!entry.specification.version_fence.is_empty());
            assert!(!entry.adapter.implementation_version.is_empty());
            assert!(!entry.validator.version.is_empty());
        }
    }
}
