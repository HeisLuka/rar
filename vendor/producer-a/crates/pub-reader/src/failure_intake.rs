use pub_contents::{ContentsFamily, ContentsReadError};
use serde::{Deserialize, Serialize};
use std::cmp::min;
use std::io::{Cursor, Read};

const CFB_MAGIC: &[u8; 8] = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1";
const ZIP_LOCAL_MAGIC: &[u8; 4] = b"PK\x03\x04";
const ZIP_EMPTY_MAGIC: &[u8; 4] = b"PK\x05\x06";
const ZIP_SPANNED_MAGIC: &[u8; 4] = b"PK\x07\x08";
const MAX_SIGNATURE_SCAN: usize = 64 * 1024;
const MAX_ZIP_ENTRIES: usize = 512;
const MAX_ZIP_MEMBER_PREFIX: usize = 64 * 1024;
const MAX_ZIP_MEMBER_FULL_INSPECT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ZIP_TOTAL_FULL_INSPECT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ZIP_MEMBER_DECLARED_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureIntakeClass {
    PubHighValue,
    PubDamaged,
    PubPossible,
    ArchiveWithPub,
    NotPub,
    SuspiciousPolyglot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureIntakeConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureIntakeReason {
    Empty,
    HtmlPrefix,
    XmlPrefix,
    PdfMagic,
    PngMagic,
    JpegMagic,
    GifMagic,
    BmpMagic,
    TiffMagic,
    WebpMagic,
    PeMagic,
    PlainText,
    ZipContainer,
    ZipParseFailed,
    ZipEntryLimit,
    ZipContainsPublisherCandidate,
    ZipCandidateUnreadable,
    ZipAmbiguousPublisherCandidate,
    ZipNoPublisherCandidate,
    CfbMagic,
    CfbParseFailed,
    RawContentsDirectoryMarker,
    PublisherIdentityString,
    RawQuillMarker,
    ContentsStream,
    ContentsFamily0x22,
    ContentsFamily0x2c,
    ContentsTooShort,
    ContentsUnsupportedMagic,
    PublisherAuxiliaryStream,
    GenericCfbNoPublisherEvidence,
    EmbeddedCfbSignature,
    OpaqueBinary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureIntakeClassification {
    pub class: FailureIntakeClass,
    pub confidence: FailureIntakeConfidence,
    pub reasons: Vec<FailureIntakeReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contents_family: Option<ContentsFamily>,
}

pub fn classify_failure_candidate(bytes: &[u8]) -> FailureIntakeClassification {
    classify_inner(bytes, true)
}

fn classify_inner(bytes: &[u8], allow_zip: bool) -> FailureIntakeClassification {
    if bytes.is_empty() {
        return classification(
            FailureIntakeClass::NotPub,
            FailureIntakeConfidence::High,
            vec![FailureIntakeReason::Empty],
            None,
        );
    }

    if let Some(foreign_reason) = foreign_prefix_reason(bytes) {
        if find_subslice(
            &bytes[..min(bytes.len(), MAX_SIGNATURE_SCAN)],
            CFB_MAGIC.as_slice(),
            1,
        )
        .is_some()
        {
            return classification(
                FailureIntakeClass::SuspiciousPolyglot,
                FailureIntakeConfidence::High,
                vec![foreign_reason, FailureIntakeReason::EmbeddedCfbSignature],
                None,
            );
        }
        return classification(
            FailureIntakeClass::NotPub,
            FailureIntakeConfidence::High,
            vec![foreign_reason],
            None,
        );
    }

    if allow_zip && is_zip(bytes) {
        return classify_zip(bytes);
    }

    if bytes.starts_with(CFB_MAGIC) {
        return classify_cfb(bytes);
    }

    if looks_plain_text(bytes) {
        return classification(
            FailureIntakeClass::NotPub,
            FailureIntakeConfidence::High,
            vec![FailureIntakeReason::PlainText],
            None,
        );
    }

    classification(
        FailureIntakeClass::PubPossible,
        FailureIntakeConfidence::Low,
        vec![FailureIntakeReason::OpaqueBinary],
        None,
    )
}

fn classify_cfb(bytes: &[u8]) -> FailureIntakeClassification {
    let inventory = match pub_cfb::inspect_reader(Cursor::new(bytes)) {
        Ok(inventory) => inventory,
        Err(_) => {
            let mut reasons = vec![
                FailureIntakeReason::CfbMagic,
                FailureIntakeReason::CfbParseFailed,
            ];
            let mut publisher_evidence = false;
            if contains_raw_contents_directory_marker(bytes) {
                reasons.push(FailureIntakeReason::RawContentsDirectoryMarker);
                publisher_evidence = true;
            }
            let raw_hints = raw_publisher_identity_hints(bytes);
            if raw_hints.publisher_identity {
                reasons.push(FailureIntakeReason::PublisherIdentityString);
                publisher_evidence = true;
            }
            if raw_hints.quill {
                reasons.push(FailureIntakeReason::RawQuillMarker);
                publisher_evidence = true;
            }
            if publisher_evidence {
                return classification(
                    FailureIntakeClass::PubDamaged,
                    FailureIntakeConfidence::Medium,
                    reasons,
                    None,
                );
            }
            return classification(
                FailureIntakeClass::PubPossible,
                FailureIntakeConfidence::Low,
                reasons,
                None,
            );
        }
    };

    let has_contents = inventory
        .entries
        .iter()
        .any(|entry| entry.path == "/Contents");
    let has_auxiliary = inventory.entries.iter().any(|entry| {
        matches!(
            entry.path.as_str(),
            "/Quill/QuillSub/CONTENTS" | "/Escher/EscherStm" | "/Escher/EscherDelayStm"
        )
    });

    if has_contents {
        let mut reasons = vec![
            FailureIntakeReason::CfbMagic,
            FailureIntakeReason::ContentsStream,
        ];
        match pub_cfb::read_stream_reader(Cursor::new(bytes), "/Contents") {
            Ok(contents) => match pub_contents::detect_family(&contents) {
                Ok(ContentsFamily::Family0x22) => {
                    reasons.push(FailureIntakeReason::ContentsFamily0x22);
                    classification(
                        FailureIntakeClass::PubHighValue,
                        FailureIntakeConfidence::High,
                        reasons,
                        Some(ContentsFamily::Family0x22),
                    )
                }
                Ok(ContentsFamily::Family0x2c) => {
                    reasons.push(FailureIntakeReason::ContentsFamily0x2c);
                    classification(
                        FailureIntakeClass::PubHighValue,
                        FailureIntakeConfidence::High,
                        reasons,
                        Some(ContentsFamily::Family0x2c),
                    )
                }
                Err(ContentsReadError::TooShort { .. }) => {
                    reasons.push(FailureIntakeReason::ContentsTooShort);
                    classification(
                        FailureIntakeClass::PubDamaged,
                        FailureIntakeConfidence::High,
                        reasons,
                        None,
                    )
                }
                Err(_) => {
                    reasons.push(FailureIntakeReason::ContentsUnsupportedMagic);
                    if has_auxiliary {
                        reasons.push(FailureIntakeReason::PublisherAuxiliaryStream);
                    }
                    classification(
                        FailureIntakeClass::PubPossible,
                        FailureIntakeConfidence::Medium,
                        reasons,
                        None,
                    )
                }
            },
            Err(_) => {
                reasons.push(FailureIntakeReason::ContentsTooShort);
                classification(
                    FailureIntakeClass::PubDamaged,
                    FailureIntakeConfidence::High,
                    reasons,
                    None,
                )
            }
        }
    } else if has_auxiliary {
        classification(
            FailureIntakeClass::PubPossible,
            FailureIntakeConfidence::Medium,
            vec![
                FailureIntakeReason::CfbMagic,
                FailureIntakeReason::PublisherAuxiliaryStream,
            ],
            None,
        )
    } else {
        classification(
            FailureIntakeClass::PubPossible,
            FailureIntakeConfidence::Low,
            vec![
                FailureIntakeReason::CfbMagic,
                FailureIntakeReason::GenericCfbNoPublisherEvidence,
            ],
            None,
        )
    }
}

fn classify_zip(bytes: &[u8]) -> FailureIntakeClassification {
    let mut archive = match zip::ZipArchive::new(Cursor::new(bytes)) {
        Ok(archive) => archive,
        Err(_) => {
            return classification(
                FailureIntakeClass::NotPub,
                FailureIntakeConfidence::Medium,
                vec![
                    FailureIntakeReason::ZipContainer,
                    FailureIntakeReason::ZipParseFailed,
                ],
                None,
            );
        }
    };

    if archive.len() > MAX_ZIP_ENTRIES {
        return classification(
            FailureIntakeClass::PubPossible,
            FailureIntakeConfidence::Low,
            vec![
                FailureIntakeReason::ZipContainer,
                FailureIntakeReason::ZipEntryLimit,
            ],
            None,
        );
    }

    let mut unreadable_candidate = false;
    let mut ambiguous_candidate = false;
    let mut full_inspect_budget = MAX_ZIP_TOTAL_FULL_INSPECT_BYTES;
    for index in 0..archive.len() {
        let mut member = match archive.by_index(index) {
            Ok(member) => member,
            Err(_) => continue,
        };
        if member.is_dir() {
            continue;
        }

        let name_is_pub = member.name().to_ascii_lowercase().ends_with(".pub");
        let declared_size = member.size();
        if declared_size > MAX_ZIP_MEMBER_DECLARED_BYTES && !name_is_pub {
            continue;
        }

        let full_inspect = name_is_pub
            && declared_size <= MAX_ZIP_MEMBER_FULL_INSPECT_BYTES
            && declared_size <= full_inspect_budget;
        let read_limit = if full_inspect {
            full_inspect_budget -= declared_size;
            declared_size
        } else {
            MAX_ZIP_MEMBER_PREFIX as u64
        };

        let mut inspected = Vec::with_capacity(
            min(declared_size, read_limit)
                .try_into()
                .unwrap_or(usize::MAX),
        );
        let read_result = (&mut member).take(read_limit).read_to_end(&mut inspected);

        if read_result.is_err() {
            if name_is_pub {
                unreadable_candidate = true;
            }
            continue;
        }

        let nested = classify_inner(&inspected, false);
        let strong_publisher_evidence = matches!(
            nested.class,
            FailureIntakeClass::PubHighValue | FailureIntakeClass::PubDamaged
        ) && (nested.contents_family.is_some()
            || nested
                .reasons
                .contains(&FailureIntakeReason::RawContentsDirectoryMarker)
            || nested
                .reasons
                .contains(&FailureIntakeReason::ContentsStream)
            || nested
                .reasons
                .contains(&FailureIntakeReason::PublisherAuxiliaryStream)
            || nested
                .reasons
                .contains(&FailureIntakeReason::PublisherIdentityString)
            || nested
                .reasons
                .contains(&FailureIntakeReason::RawQuillMarker));

        if strong_publisher_evidence {
            let confidence = if nested.contents_family.is_some() {
                FailureIntakeConfidence::High
            } else {
                FailureIntakeConfidence::Medium
            };
            return classification(
                FailureIntakeClass::ArchiveWithPub,
                confidence,
                vec![
                    FailureIntakeReason::ZipContainer,
                    FailureIntakeReason::ZipContainsPublisherCandidate,
                ],
                nested.contents_family,
            );
        }

        // A .pub member name is only a hint. Generic OLE/CFB bytes are not
        // Publisher evidence, but retaining one bounded "possible" signal keeps
        // genuinely unknown Publisher variants from being silently discarded.
        if name_is_pub
            && inspected.starts_with(CFB_MAGIC)
            && nested.class == FailureIntakeClass::PubPossible
        {
            ambiguous_candidate = true;
        }
    }

    let mut reasons = vec![
        FailureIntakeReason::ZipContainer,
        FailureIntakeReason::ZipNoPublisherCandidate,
    ];
    if unreadable_candidate || ambiguous_candidate {
        if unreadable_candidate {
            reasons.push(FailureIntakeReason::ZipCandidateUnreadable);
        }
        if ambiguous_candidate {
            reasons.push(FailureIntakeReason::ZipAmbiguousPublisherCandidate);
        }
        return classification(
            FailureIntakeClass::PubPossible,
            FailureIntakeConfidence::Low,
            reasons,
            None,
        );
    }

    classification(
        FailureIntakeClass::NotPub,
        FailureIntakeConfidence::High,
        reasons,
        None,
    )
}

fn foreign_prefix_reason(bytes: &[u8]) -> Option<FailureIntakeReason> {
    if bytes.starts_with(b"%PDF-") {
        return Some(FailureIntakeReason::PdfMagic);
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1A\n") {
        return Some(FailureIntakeReason::PngMagic);
    }
    if bytes.starts_with(b"\xFF\xD8\xFF") {
        return Some(FailureIntakeReason::JpegMagic);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(FailureIntakeReason::GifMagic);
    }
    if bytes.starts_with(b"BM") {
        return Some(FailureIntakeReason::BmpMagic);
    }
    if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        return Some(FailureIntakeReason::TiffMagic);
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some(FailureIntakeReason::WebpMagic);
    }
    if bytes.starts_with(b"MZ") {
        return Some(FailureIntakeReason::PeMagic);
    }

    let text = normalized_text_prefix(bytes);
    if text.starts_with("<!doctype html")
        || text.starts_with("<html")
        || text.starts_with("<head")
        || text.starts_with("<body")
    {
        return Some(FailureIntakeReason::HtmlPrefix);
    }
    if text.starts_with("<?xml") || text.starts_with("<svg") {
        return Some(FailureIntakeReason::XmlPrefix);
    }

    None
}

fn normalized_text_prefix(bytes: &[u8]) -> String {
    let prefix = &bytes[..min(bytes.len(), 512)];
    let prefix = prefix.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(prefix);
    let start = prefix
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(prefix.len());
    String::from_utf8_lossy(&prefix[start..]).to_ascii_lowercase()
}

fn looks_plain_text(bytes: &[u8]) -> bool {
    let prefix = &bytes[..min(bytes.len(), 1024)];
    if prefix.len() < 8 || prefix.contains(&0) {
        return false;
    }
    let printable = prefix
        .iter()
        .filter(|byte| byte.is_ascii_graphic() || byte.is_ascii_whitespace())
        .count();
    printable * 100 >= prefix.len() * 95
}

fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(ZIP_LOCAL_MAGIC)
        || bytes.starts_with(ZIP_EMPTY_MAGIC)
        || bytes.starts_with(ZIP_SPANNED_MAGIC)
}

#[derive(Debug, Clone, Copy, Default)]
struct RawPublisherIdentityHints {
    publisher_identity: bool,
    quill: bool,
}

fn raw_publisher_identity_hints(bytes: &[u8]) -> RawPublisherIdentityHints {
    const MAX_IDENTITY_SCAN: usize = 4 * 1024 * 1024;
    let sample = &bytes[..min(bytes.len(), MAX_IDENTITY_SCAN)];
    let lower = sample.to_ascii_lowercase();

    let has_ascii = |needle: &[u8]| find_subslice(&lower, needle, 0).is_some();
    let has_utf16le = |needle: &str| {
        let encoded: Vec<u8> = needle
            .bytes()
            .flat_map(|byte| [byte.to_ascii_lowercase(), 0])
            .collect();
        find_subslice(&lower, &encoded, 0).is_some()
    };

    RawPublisherIdentityHints {
        publisher_identity: has_ascii(b"microsoft publisher")
            || has_ascii(b"mspublisher")
            || has_utf16le("microsoft publisher")
            || has_utf16le("mspublisher"),
        // CFB directory names are UTF-16LE. Require the Publisher-specific
        // Quill hierarchy rather than an arbitrary payload string containing
        // the English word "quill".
        quill: has_utf16le("quill") && has_utf16le("quillsub"),
    }
}

fn contains_raw_contents_directory_marker(bytes: &[u8]) -> bool {
    const CONTENTS_UTF16_LE: &[u8] = b"C\x00o\x00n\x00t\x00e\x00n\x00t\x00s\x00";
    find_subslice(
        &bytes[..min(bytes.len(), 1024 * 1024)],
        CONTENTS_UTF16_LE,
        0,
    )
    .is_some()
}

fn find_subslice(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    if needle.is_empty() || start >= haystack.len() {
        return None;
    }
    haystack[start..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|index| index + start)
}

fn classification(
    class: FailureIntakeClass,
    confidence: FailureIntakeConfidence,
    reasons: Vec<FailureIntakeReason>,
    contents_family: Option<ContentsFamily>,
) -> FailureIntakeClassification {
    FailureIntakeClassification {
        class,
        confidence,
        reasons,
        contents_family,
    }
}
