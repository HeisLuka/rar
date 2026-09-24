use crate::{
    FailureIntakeClass, FailureIntakeClassification, FailureIntakeConfidence, FailureIntakeReason,
};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const CHAPTERA_FAILURE_ENVELOPE_SCHEMA_V1: &str = "chaptera-failure-envelope/v1";
pub const CHAPTERA_READER_BUILD_ID: &str = concat!("chaptera-reader-", env!("CARGO_PKG_VERSION"));
pub const PUB_READER_ENGINE_BUILD_ID: &str = concat!("pub-reader-", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureTelemetryChoice {
    Disabled,
    MinimalStructural,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureParserStage {
    PubReaderOpen,
    ContainerInspect,
    ContentsDetect,
    SemanticBridge,
}

impl TryFrom<&str> for FailureParserStage {
    type Error = FailureEnvelopeBuildError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "pub_reader_open" => Ok(Self::PubReaderOpen),
            "container_inspect" => Ok(Self::ContainerInspect),
            "contents_detect" => Ok(Self::ContentsDetect),
            "semantic_bridge" => Ok(Self::SemanticBridge),
            _ => Err(FailureEnvelopeBuildError::UnapprovedValue {
                field: "parser_stage",
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    OpenFailed,
    CfbInvalid,
    ContentsUnsupported,
    ContentsMalformed,
    Timeout,
    ResourceLimit,
}

impl TryFrom<&str> for FailureCode {
    type Error = FailureEnvelopeBuildError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "open_failed" => Ok(Self::OpenFailed),
            "cfb_invalid" => Ok(Self::CfbInvalid),
            "contents_unsupported" => Ok(Self::ContentsUnsupported),
            "contents_malformed" => Ok(Self::ContentsMalformed),
            "timeout" => Ok(Self::Timeout),
            "resource_limit" => Ok(Self::ResourceLimit),
            _ => Err(FailureEnvelopeBuildError::UnapprovedValue {
                field: "failure_code",
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureOsFamily {
    Windows,
    Linux,
    Macos,
}

impl TryFrom<&str> for FailureOsFamily {
    type Error = FailureEnvelopeBuildError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "windows" => Ok(Self::Windows),
            "linux" => Ok(Self::Linux),
            "macos" => Ok(Self::Macos),
            _ => Err(FailureEnvelopeBuildError::UnapprovedValue { field: "os_family" }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureArchitecture {
    X86_64,
    Aarch64,
    X86_32,
}

impl TryFrom<&str> for FailureArchitecture {
    type Error = FailureEnvelopeBuildError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "x86_64" => Ok(Self::X86_64),
            "aarch64" => Ok(Self::Aarch64),
            "x86_32" => Ok(Self::X86_32),
            _ => Err(FailureEnvelopeBuildError::UnapprovedValue {
                field: "architecture",
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCoarseLocale {
    EnUs,
    EnGb,
    RuRu,
    Other,
}

impl TryFrom<&str> for FailureCoarseLocale {
    type Error = FailureEnvelopeBuildError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "en-US" => Ok(Self::EnUs),
            "en-GB" => Ok(Self::EnGb),
            "ru-RU" => Ok(Self::RuRu),
            _ => Err(FailureEnvelopeBuildError::UnapprovedValue {
                field: "coarse_locale",
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureSizeBucket {
    Empty,
    Under4KiB,
    Under64KiB,
    Under1MiB,
    Under16MiB,
    Under128MiB,
    AtLeast128MiB,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureContainerFamily {
    Cfb,
    Zip,
    Foreign,
    Opaque,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureEnvelopeContext {
    pub parser_stage: FailureParserStage,
    pub failure_code: FailureCode,
    pub timeout: bool,
    pub resource_limit: bool,
    pub byte_len: usize,
    pub os_family: Option<FailureOsFamily>,
    pub architecture: Option<FailureArchitecture>,
    pub coarse_locale: Option<FailureCoarseLocale>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureEnvelope {
    pub schema_version: String,
    pub app_build: String,
    pub engine_build: String,
    pub parser_stage: FailureParserStage,
    pub failure_code: FailureCode,
    pub timeout: bool,
    pub resource_limit: bool,
    pub size_bucket: FailureSizeBucket,
    pub intake_class: FailureIntakeClass,
    pub intake_confidence: FailureIntakeConfidence,
    pub intake_reasons: Vec<FailureIntakeReason>,
    pub container_family: FailureContainerFamily,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contents_family: Option<pub_contents::ContentsFamily>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_family: Option<FailureOsFamily>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture: Option<FailureArchitecture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coarse_locale: Option<FailureCoarseLocale>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureEnvelopeBuildError {
    UnapprovedValue { field: &'static str },
}

impl fmt::Display for FailureEnvelopeBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnapprovedValue { field } => {
                write!(f, "failure-envelope field {field} is not allowlisted")
            }
        }
    }
}

impl std::error::Error for FailureEnvelopeBuildError {}

pub fn build_failure_envelope(
    choice: FailureTelemetryChoice,
    classification: &FailureIntakeClassification,
    context: FailureEnvelopeContext,
) -> Option<FailureEnvelope> {
    if choice == FailureTelemetryChoice::Disabled {
        return None;
    }

    Some(FailureEnvelope {
        schema_version: CHAPTERA_FAILURE_ENVELOPE_SCHEMA_V1.to_owned(),
        app_build: CHAPTERA_READER_BUILD_ID.to_owned(),
        engine_build: PUB_READER_ENGINE_BUILD_ID.to_owned(),
        parser_stage: context.parser_stage,
        failure_code: context.failure_code,
        timeout: context.timeout,
        resource_limit: context.resource_limit,
        size_bucket: size_bucket(context.byte_len),
        intake_class: classification.class,
        intake_confidence: classification.confidence,
        intake_reasons: classification.reasons.clone(),
        container_family: container_family(classification),
        contents_family: classification.contents_family,
        os_family: context.os_family,
        architecture: context.architecture,
        coarse_locale: context.coarse_locale,
    })
}

fn size_bucket(byte_len: usize) -> FailureSizeBucket {
    match byte_len {
        0 => FailureSizeBucket::Empty,
        1..=4_095 => FailureSizeBucket::Under4KiB,
        4_096..=65_535 => FailureSizeBucket::Under64KiB,
        65_536..=1_048_575 => FailureSizeBucket::Under1MiB,
        1_048_576..=16_777_215 => FailureSizeBucket::Under16MiB,
        16_777_216..=134_217_727 => FailureSizeBucket::Under128MiB,
        _ => FailureSizeBucket::AtLeast128MiB,
    }
}

fn container_family(classification: &FailureIntakeClassification) -> FailureContainerFamily {
    if classification
        .reasons
        .contains(&FailureIntakeReason::ZipContainer)
    {
        return FailureContainerFamily::Zip;
    }
    if classification
        .reasons
        .contains(&FailureIntakeReason::CfbMagic)
    {
        return FailureContainerFamily::Cfb;
    }
    if classification.reasons.iter().any(|reason| {
        matches!(
            reason,
            FailureIntakeReason::HtmlPrefix
                | FailureIntakeReason::XmlPrefix
                | FailureIntakeReason::PdfMagic
                | FailureIntakeReason::PngMagic
                | FailureIntakeReason::JpegMagic
                | FailureIntakeReason::GifMagic
                | FailureIntakeReason::BmpMagic
                | FailureIntakeReason::TiffMagic
                | FailureIntakeReason::WebpMagic
                | FailureIntakeReason::PeMagic
                | FailureIntakeReason::PlainText
        )
    }) {
        return FailureContainerFamily::Foreign;
    }
    FailureContainerFamily::Opaque
}
