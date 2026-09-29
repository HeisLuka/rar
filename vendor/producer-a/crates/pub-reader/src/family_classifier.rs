use pub_contents::{ContentsFamily, ContentsReadError};
use serde::{Deserialize, Serialize};
use std::io::Cursor;

const CONTENTS_STREAM_PATH: &str = "/Contents";
const QUILL_STREAM_PATH: &str = "/Quill/QuillSub/CONTENTS";
const ESCHER_STREAM_PATH: &str = "/Escher/EscherStm";
const ESCHER_DELAY_STREAM_PATH: &str = "/Escher/EscherDelayStm";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PubFamilyProfile {
    Legacy22LowText,
    Legacy22Quill,
    Mature2cComplete,
    Mature2cIncomplete,
    PublisherUnknownFamily,
    NotStructuredPublisher,
}

impl PubFamilyProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Legacy22LowText => "legacy_0x22_low_text",
            Self::Legacy22Quill => "legacy_0x22_quill",
            Self::Mature2cComplete => "mature_0x2c_complete",
            Self::Mature2cIncomplete => "mature_0x2c_incomplete",
            Self::PublisherUnknownFamily => "publisher_unknown_family",
            Self::NotStructuredPublisher => "not_structured_publisher",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PubReaderRoute {
    Legacy22LowText,
    Legacy22Quill,
    Mature2c,
    Unsupported,
}

impl PubReaderRoute {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Legacy22LowText => "legacy_0x22_low_text",
            Self::Legacy22Quill => "legacy_0x22_quill",
            Self::Mature2c => "mature_0x2c",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PubFamilyConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PubFamilyReason {
    CfbContainer,
    CfbParseFailed,
    ContentsStream,
    ContentsMissing,
    ContentsTooShort,
    ContentsUnsupportedMagic,
    ContentsFamily0x22,
    ContentsFamily0x2c,
    QuillPresent,
    QuillAbsent,
    EscherPresent,
    EscherAbsent,
    EscherDelayPresent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PubFamilyClassification {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<ContentsFamily>,
    pub profile: PubFamilyProfile,
    pub route: PubReaderRoute,
    pub confidence: PubFamilyConfidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contents_magic: Option<[u8; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serialization_revision: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_content_version: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_secondary_fingerprint: Option<u16>,
    pub has_quill: bool,
    pub has_escher: bool,
    pub has_escher_delay: bool,
    pub reasons: Vec<PubFamilyReason>,
}

/// Classifies one candidate before any family-specific PUB parser runs.
///
/// This boundary is intentionally structural and bounded:
/// - the file extension is irrelevant;
/// - exact CFB path names are evidence;
/// - 0x22 vs 0x2C comes from /Contents bytes;
/// - Quill/Escher presence selects a grounded persistence profile;
/// - marketing Publisher versions are not inferred from one scalar.
///
/// legacy_content_version and legacy_secondary_fingerprint are exposed as
/// evidence coordinates for old-0x22 callers, not as universal version labels.
pub fn classify_pub_family(bytes: &[u8]) -> PubFamilyClassification {
    let inventory = match pub_cfb::inspect_reader(Cursor::new(bytes)) {
        Ok(inventory) => inventory,
        Err(_) => {
            return PubFamilyClassification {
                family: None,
                profile: PubFamilyProfile::NotStructuredPublisher,
                route: PubReaderRoute::Unsupported,
                confidence: PubFamilyConfidence::Low,
                contents_magic: None,
                serialization_revision: None,
                legacy_content_version: None,
                legacy_secondary_fingerprint: None,
                has_quill: false,
                has_escher: false,
                has_escher_delay: false,
                reasons: vec![PubFamilyReason::CfbParseFailed],
            };
        }
    };

    let has_path = |path: &str| inventory.entries.iter().any(|entry| entry.path == path);
    let has_contents = has_path(CONTENTS_STREAM_PATH);
    let has_quill = has_path(QUILL_STREAM_PATH);
    let has_escher = has_path(ESCHER_STREAM_PATH);
    let has_escher_delay = has_path(ESCHER_DELAY_STREAM_PATH);

    let mut reasons = vec![PubFamilyReason::CfbContainer];
    reasons.push(if has_quill {
        PubFamilyReason::QuillPresent
    } else {
        PubFamilyReason::QuillAbsent
    });
    reasons.push(if has_escher {
        PubFamilyReason::EscherPresent
    } else {
        PubFamilyReason::EscherAbsent
    });
    if has_escher_delay {
        reasons.push(PubFamilyReason::EscherDelayPresent);
    }

    if !has_contents {
        reasons.push(PubFamilyReason::ContentsMissing);
        return PubFamilyClassification {
            family: None,
            profile: PubFamilyProfile::PublisherUnknownFamily,
            route: PubReaderRoute::Unsupported,
            confidence: PubFamilyConfidence::Low,
            contents_magic: None,
            serialization_revision: None,
            legacy_content_version: None,
            legacy_secondary_fingerprint: None,
            has_quill,
            has_escher,
            has_escher_delay,
            reasons,
        };
    }
    reasons.push(PubFamilyReason::ContentsStream);

    let contents = match pub_cfb::read_stream_reader(Cursor::new(bytes), CONTENTS_STREAM_PATH) {
        Ok(contents) => contents,
        Err(_) => {
            reasons.push(PubFamilyReason::ContentsTooShort);
            return PubFamilyClassification {
                family: None,
                profile: PubFamilyProfile::PublisherUnknownFamily,
                route: PubReaderRoute::Unsupported,
                confidence: PubFamilyConfidence::Medium,
                contents_magic: None,
                serialization_revision: None,
                legacy_content_version: None,
                legacy_secondary_fingerprint: None,
                has_quill,
                has_escher,
                has_escher_delay,
                reasons,
            };
        }
    };

    let contents_magic = contents
        .get(..4)
        .map(|magic| [magic[0], magic[1], magic[2], magic[3]]);

    match pub_contents::detect_family(&contents) {
        Ok(ContentsFamily::Family0x22) => {
            reasons.push(PubFamilyReason::ContentsFamily0x22);
            let legacy_content_version = read_u16_le_at(&contents, 0x04);
            let legacy_secondary_fingerprint = read_u16_le_at(&contents, 0x0c);
            let (profile, route) = if has_quill {
                (PubFamilyProfile::Legacy22Quill, PubReaderRoute::Legacy22Quill)
            } else {
                (
                    PubFamilyProfile::Legacy22LowText,
                    PubReaderRoute::Legacy22LowText,
                )
            };
            PubFamilyClassification {
                family: Some(ContentsFamily::Family0x22),
                profile,
                route,
                confidence: PubFamilyConfidence::High,
                contents_magic,
                serialization_revision: None,
                legacy_content_version,
                legacy_secondary_fingerprint,
                has_quill,
                has_escher,
                has_escher_delay,
                reasons,
            }
        }
        Ok(ContentsFamily::Family0x2c) => {
            reasons.push(PubFamilyReason::ContentsFamily0x2c);
            let serialization_revision = read_u16_le_at(&contents, 0x0c);
            let (profile, route) = if has_quill && has_escher {
                (PubFamilyProfile::Mature2cComplete, PubReaderRoute::Mature2c)
            } else {
                (
                    PubFamilyProfile::Mature2cIncomplete,
                    PubReaderRoute::Unsupported,
                )
            };
            PubFamilyClassification {
                family: Some(ContentsFamily::Family0x2c),
                profile,
                route,
                confidence: PubFamilyConfidence::High,
                contents_magic,
                serialization_revision,
                legacy_content_version: None,
                legacy_secondary_fingerprint: None,
                has_quill,
                has_escher,
                has_escher_delay,
                reasons,
            }
        }
        Err(ContentsReadError::TooShort { .. }) => {
            reasons.push(PubFamilyReason::ContentsTooShort);
            PubFamilyClassification {
                family: None,
                profile: PubFamilyProfile::PublisherUnknownFamily,
                route: PubReaderRoute::Unsupported,
                confidence: PubFamilyConfidence::Medium,
                contents_magic,
                serialization_revision: None,
                legacy_content_version: None,
                legacy_secondary_fingerprint: None,
                has_quill,
                has_escher,
                has_escher_delay,
                reasons,
            }
        }
        Err(_) => {
            reasons.push(PubFamilyReason::ContentsUnsupportedMagic);
            PubFamilyClassification {
                family: None,
                profile: PubFamilyProfile::PublisherUnknownFamily,
                route: PubReaderRoute::Unsupported,
                confidence: PubFamilyConfidence::Medium,
                contents_magic,
                serialization_revision: None,
                legacy_content_version: None,
                legacy_secondary_fingerprint: None,
                has_quill,
                has_escher,
                has_escher_delay,
                reasons,
            }
        }
    }
}

fn read_u16_le_at(bytes: &[u8], offset: usize) -> Option<u16> {
    let slice = bytes.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    fn contents(family: [u8; 4], word_at_04: u16, word_at_0c: u16) -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x20];
        bytes[..4].copy_from_slice(&family);
        bytes[0x04..0x06].copy_from_slice(&word_at_04.to_le_bytes());
        bytes[0x0c..0x0e].copy_from_slice(&word_at_0c.to_le_bytes());
        bytes
    }

    fn synthetic_cfb(contents: Option<&[u8]>, quill: bool, escher: bool) -> Vec<u8> {
        let mut compound =
            cfb::CompoundFile::create(Cursor::new(Vec::new())).expect("create synthetic CFB");

        if let Some(contents) = contents {
            compound
                .create_stream(CONTENTS_STREAM_PATH)
                .expect("create Contents")
                .write_all(contents)
                .expect("write Contents");
        }

        if quill {
            compound.create_storage("/Quill").expect("create Quill");
            compound
                .create_storage("/Quill/QuillSub")
                .expect("create QuillSub");
            compound
                .create_stream(QUILL_STREAM_PATH)
                .expect("create Quill CONTENTS")
                .write_all(b"quill")
                .expect("write Quill CONTENTS");
        }

        if escher {
            compound.create_storage("/Escher").expect("create Escher");
            compound
                .create_stream(ESCHER_STREAM_PATH)
                .expect("create EscherStm")
                .write_all(b"escher")
                .expect("write EscherStm");
        }

        compound.flush().expect("flush synthetic CFB");
        compound.into_inner().into_inner()
    }

    #[test]
    fn classifies_publisher97_style_no_quill_as_legacy_low_text() {
        let bytes = synthetic_cfb(
            Some(&contents(
                pub_contents::CONTENTS_0X22_MAGIC,
                300,
                0x0088,
            )),
            false,
            false,
        );

        let classified = classify_pub_family(&bytes);

        assert_eq!(classified.family, Some(ContentsFamily::Family0x22));
        assert_eq!(classified.profile, PubFamilyProfile::Legacy22LowText);
        assert_eq!(classified.route, PubReaderRoute::Legacy22LowText);
        assert_eq!(classified.legacy_content_version, Some(300));
        assert_eq!(classified.legacy_secondary_fingerprint, Some(0x0088));
        assert!(!classified.has_quill);
        assert!(!classified.has_escher);
    }

    #[test]
    fn classifies_old_0x22_with_quill_separately() {
        let bytes = synthetic_cfb(
            Some(&contents(
                pub_contents::CONTENTS_0X22_MAGIC,
                300,
                0x0268,
            )),
            true,
            false,
        );

        let classified = classify_pub_family(&bytes);

        assert_eq!(classified.family, Some(ContentsFamily::Family0x22));
        assert_eq!(classified.profile, PubFamilyProfile::Legacy22Quill);
        assert_eq!(classified.route, PubReaderRoute::Legacy22Quill);
        assert!(classified.has_quill);
    }

    #[test]
    fn mature_0x2c_requires_both_quill_and_escher() {
        let complete = synthetic_cfb(
            Some(&contents(
                pub_contents::CONTENTS_0X2C_MAGIC,
                0,
                21,
            )),
            true,
            true,
        );
        let missing_quill = synthetic_cfb(
            Some(&contents(
                pub_contents::CONTENTS_0X2C_MAGIC,
                0,
                21,
            )),
            false,
            true,
        );
        let missing_escher = synthetic_cfb(
            Some(&contents(
                pub_contents::CONTENTS_0X2C_MAGIC,
                0,
                21,
            )),
            true,
            false,
        );

        let complete = classify_pub_family(&complete);
        assert_eq!(complete.profile, PubFamilyProfile::Mature2cComplete);
        assert_eq!(complete.route, PubReaderRoute::Mature2c);
        assert_eq!(complete.serialization_revision, Some(21));

        for classified in [
            classify_pub_family(&missing_quill),
            classify_pub_family(&missing_escher),
        ] {
            assert_eq!(classified.family, Some(ContentsFamily::Family0x2c));
            assert_eq!(classified.profile, PubFamilyProfile::Mature2cIncomplete);
            assert_eq!(classified.route, PubReaderRoute::Unsupported);
        }
    }

    #[test]
    fn unknown_contents_magic_does_not_guess_a_parser() {
        let bytes = synthetic_cfb(
            Some(&contents([0xE8, 0xAC, 0x23, 0x00], 0, 0)),
            true,
            true,
        );

        let classified = classify_pub_family(&bytes);

        assert_eq!(classified.family, None);
        assert_eq!(
            classified.profile,
            PubFamilyProfile::PublisherUnknownFamily
        );
        assert_eq!(classified.route, PubReaderRoute::Unsupported);
        assert!(
            classified
                .reasons
                .contains(&PubFamilyReason::ContentsUnsupportedMagic)
        );
    }

    #[test]
    #[ignore = "requires CHAPTERA_OPNHOUS_PUB exact public fixture"]
    fn exact_opnhous_publisher97_routes_to_legacy_low_text() {
        use sha2::{Digest, Sha256};
        use std::fs;
        use std::path::PathBuf;

        let path = std::env::var_os("CHAPTERA_OPNHOUS_PUB")
            .map(PathBuf::from)
            .expect("CHAPTERA_OPNHOUS_PUB");
        let bytes = fs::read(path).expect("read exact OPNHOUS fixture");
        assert_eq!(bytes.len(), 11_264);
        let actual_sha256 = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            actual_sha256,
            "0c74bed1b862f4603a77567f817ad22bf1f7c42eb5afbee0c907732953534b5c"
        );

        let classified = classify_pub_family(&bytes);
        assert_eq!(classified.family, Some(ContentsFamily::Family0x22));
        assert_eq!(classified.profile, PubFamilyProfile::Legacy22LowText);
        assert_eq!(classified.route, PubReaderRoute::Legacy22LowText);
        assert_eq!(classified.legacy_content_version, Some(300));
        assert_eq!(classified.legacy_secondary_fingerprint, Some(0x0088));
        assert!(!classified.has_quill);
        assert!(!classified.has_escher);
    }

    #[test]
    fn missing_contents_and_non_cfb_fail_closed() {
        let missing_contents = synthetic_cfb(None, true, false);
        let classified = classify_pub_family(&missing_contents);
        assert_eq!(
            classified.profile,
            PubFamilyProfile::PublisherUnknownFamily
        );
        assert_eq!(classified.route, PubReaderRoute::Unsupported);
        assert!(
            classified
                .reasons
                .contains(&PubFamilyReason::ContentsMissing)
        );

        let not_cfb = classify_pub_family(b"not a compound file");
        assert_eq!(
            not_cfb.profile,
            PubFamilyProfile::NotStructuredPublisher
        );
        assert_eq!(not_cfb.route, PubReaderRoute::Unsupported);
    }
}
