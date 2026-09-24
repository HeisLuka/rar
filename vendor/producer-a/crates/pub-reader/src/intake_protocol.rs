use crate::{FailureEnvelope, FailureIntakeClass};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const CHAPTERA_INTAKE_PROTOCOL_SCHEMA_V1: &str = "chaptera-intake-protocol/v1";
pub const CHAPTERA_EXACT_FILE_CONSENT_V1: &str = "chaptera-exact-file-consent/v1";
pub const CHAPTERA_INTAKE_RETENTION_POLICY_V1: &str = "chaptera-intake-retention-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntakeCapabilityRequest {
    pub schema_version: String,
    pub consent_contract_version: String,
    pub failure: FailureEnvelope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntakeDedupeDisposition {
    NewBlob,
    ExactDuplicate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntakeClusterDisposition {
    NewCluster,
    ExistingClusterEvidence,
    ResearchCandidate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntakeReceipt {
    pub schema_version: String,
    pub submission_id: String,
    pub server_sha256: String,
    pub dedupe_disposition: IntakeDedupeDisposition,
    pub cluster_disposition: IntakeClusterDisposition,
    pub retention_policy_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntakeProtocolError {
    WrongSchema,
    WrongConsentContract,
    IneligibleClass(FailureIntakeClass),
    MissingSubmissionId,
    InvalidServerSha256,
    WrongRetentionPolicyVersion,
}

impl fmt::Display for IntakeProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongSchema => write!(f, "unsupported Chaptera intake protocol schema"),
            Self::WrongConsentContract => {
                write!(f, "unsupported or missing exact-file consent contract")
            }
            Self::IneligibleClass(class) => {
                write!(
                    f,
                    "failure class {class:?} is not eligible for exact-file intake"
                )
            }
            Self::MissingSubmissionId => write!(f, "intake receipt submission_id is empty"),
            Self::InvalidServerSha256 => {
                write!(
                    f,
                    "intake receipt server_sha256 is not lowercase SHA-256 hex"
                )
            }
            Self::WrongRetentionPolicyVersion => {
                write!(f, "unsupported Chaptera intake retention policy version")
            }
        }
    }
}

impl std::error::Error for IntakeProtocolError {}

pub fn exact_file_intake_eligible(class: FailureIntakeClass) -> bool {
    matches!(
        class,
        FailureIntakeClass::PubHighValue | FailureIntakeClass::PubDamaged
    )
}

pub fn build_intake_capability_request(
    failure: FailureEnvelope,
) -> Result<IntakeCapabilityRequest, IntakeProtocolError> {
    if !exact_file_intake_eligible(failure.intake_class) {
        return Err(IntakeProtocolError::IneligibleClass(failure.intake_class));
    }

    Ok(IntakeCapabilityRequest {
        schema_version: CHAPTERA_INTAKE_PROTOCOL_SCHEMA_V1.to_owned(),
        consent_contract_version: CHAPTERA_EXACT_FILE_CONSENT_V1.to_owned(),
        failure,
    })
}

pub fn validate_intake_capability_request(
    request: &IntakeCapabilityRequest,
) -> Result<(), IntakeProtocolError> {
    if request.schema_version != CHAPTERA_INTAKE_PROTOCOL_SCHEMA_V1 {
        return Err(IntakeProtocolError::WrongSchema);
    }
    if request.consent_contract_version != CHAPTERA_EXACT_FILE_CONSENT_V1 {
        return Err(IntakeProtocolError::WrongConsentContract);
    }
    if !exact_file_intake_eligible(request.failure.intake_class) {
        return Err(IntakeProtocolError::IneligibleClass(
            request.failure.intake_class,
        ));
    }
    Ok(())
}

pub fn validate_intake_receipt(receipt: &IntakeReceipt) -> Result<(), IntakeProtocolError> {
    if receipt.schema_version != CHAPTERA_INTAKE_PROTOCOL_SCHEMA_V1 {
        return Err(IntakeProtocolError::WrongSchema);
    }
    if receipt.submission_id.trim().is_empty() {
        return Err(IntakeProtocolError::MissingSubmissionId);
    }
    if !is_lowercase_sha256(&receipt.server_sha256) {
        return Err(IntakeProtocolError::InvalidServerSha256);
    }
    if receipt.retention_policy_version != CHAPTERA_INTAKE_RETENTION_POLICY_V1 {
        return Err(IntakeProtocolError::WrongRetentionPolicyVersion);
    }
    Ok(())
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        FailureCode, FailureContainerFamily, FailureIntakeConfidence, FailureIntakeReason,
        FailureParserStage, FailureSizeBucket,
    };

    fn envelope(class: FailureIntakeClass) -> FailureEnvelope {
        FailureEnvelope {
            schema_version: crate::CHAPTERA_FAILURE_ENVELOPE_SCHEMA_V1.to_owned(),
            app_build: "chaptera-reader-test".to_owned(),
            engine_build: "pub-reader-test".to_owned(),
            parser_stage: FailureParserStage::PubReaderOpen,
            failure_code: FailureCode::OpenFailed,
            timeout: false,
            resource_limit: false,
            size_bucket: FailureSizeBucket::Under64KiB,
            intake_class: class,
            intake_confidence: FailureIntakeConfidence::High,
            intake_reasons: vec![FailureIntakeReason::ContentsStream],
            container_family: FailureContainerFamily::Cfb,
            contents_family: None,
            os_family: None,
            architecture: None,
            coarse_locale: None,
        }
    }

    #[test]
    fn exact_file_eligibility_is_fail_closed() {
        assert!(exact_file_intake_eligible(FailureIntakeClass::PubHighValue));
        assert!(exact_file_intake_eligible(FailureIntakeClass::PubDamaged));

        for class in [
            FailureIntakeClass::PubPossible,
            FailureIntakeClass::ArchiveWithPub,
            FailureIntakeClass::NotPub,
            FailureIntakeClass::SuspiciousPolyglot,
        ] {
            assert!(!exact_file_intake_eligible(class), "{class:?}");
            assert!(matches!(
                build_intake_capability_request(envelope(class)),
                Err(IntakeProtocolError::IneligibleClass(found)) if found == class
            ));
        }
    }

    #[test]
    fn request_contains_failure_context_but_no_client_file_identity_fields() {
        let request =
            build_intake_capability_request(envelope(FailureIntakeClass::PubHighValue)).unwrap();
        validate_intake_capability_request(&request).unwrap();

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains(CHAPTERA_EXACT_FILE_CONSENT_V1));
        for forbidden in [
            "filename",
            "file_name",
            "path",
            "client_sha256",
            "source_hash",
            "document_text",
            "raw_bytes",
        ] {
            assert!(
                !json.contains(forbidden),
                "unexpected field {forbidden}: {json}"
            );
        }
    }

    #[test]
    fn consent_contract_is_versioned_and_mandatory() {
        let mut request =
            build_intake_capability_request(envelope(FailureIntakeClass::PubDamaged)).unwrap();
        request.consent_contract_version.clear();
        assert_eq!(
            validate_intake_capability_request(&request),
            Err(IntakeProtocolError::WrongConsentContract)
        );
    }

    #[test]
    fn receipt_requires_server_identity_and_retention_policy() {
        let receipt = IntakeReceipt {
            schema_version: CHAPTERA_INTAKE_PROTOCOL_SCHEMA_V1.to_owned(),
            submission_id: "sub_01JCHAPTERAEXAMPLE".to_owned(),
            server_sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_owned(),
            dedupe_disposition: IntakeDedupeDisposition::NewBlob,
            cluster_disposition: IntakeClusterDisposition::ResearchCandidate,
            retention_policy_version: CHAPTERA_INTAKE_RETENTION_POLICY_V1.to_owned(),
        };
        validate_intake_receipt(&receipt).unwrap();

        let mut bad_hash = receipt.clone();
        bad_hash.server_sha256 =
            "0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef".to_owned();
        assert_eq!(
            validate_intake_receipt(&bad_hash),
            Err(IntakeProtocolError::InvalidServerSha256)
        );

        let mut wrong_retention = receipt;
        wrong_retention.retention_policy_version = "chaptera-intake-retention-v2".to_owned();
        assert_eq!(
            validate_intake_receipt(&wrong_retention),
            Err(IntakeProtocolError::WrongRetentionPolicyVersion)
        );
    }

    #[test]
    fn request_and_receipt_round_trip_as_typed_json() {
        let request =
            build_intake_capability_request(envelope(FailureIntakeClass::PubHighValue)).unwrap();
        let json = serde_json::to_string(&request).unwrap();
        let decoded: IntakeCapabilityRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, request);
        validate_intake_capability_request(&decoded).unwrap();
    }
}
