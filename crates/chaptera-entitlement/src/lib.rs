use coset::{
    iana, ContentType, CoseSign1, RegisteredLabelWithPrivate, TaggedCborSerializable,
};
use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use thiserror::Error;

pub const ENTITLEMENT_CONTENT_TYPE: &str = "application/vnd.chaptera.entitlement+cbor";
pub const MAX_ARTIFACT_SIZE: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationPayloadV1 {
    pub schema_version: u16,
    pub activation_id: String,
    pub entitlement_id: String,
    pub subject_id: String,
    pub product_id: String,
    pub edition: String,
    pub grants: Vec<String>,
    pub device_key_id: Vec<u8>,
    pub issued_at: i64,
    pub right: RightV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RightV1 {
    Perpetual {
        min_major: u32,
        max_major: u32,
        updates_until: Option<i64>,
    },
    Subscription {
        lease_id: String,
        lease_sequence: u64,
        not_before: i64,
        valid_until: i64,
        offline_grace_until: Option<i64>,
    },
    Trial {
        lease_id: String,
        lease_sequence: u64,
        not_before: i64,
        valid_until: i64,
        offline_grace_until: Option<i64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildIdentity<'a> {
    pub product_id: &'a str,
    pub major: u32,
    pub released_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyContext<'a> {
    pub build: BuildIdentity<'a>,
    pub expected_subject: Option<&'a str>,
    pub expected_device_key_id: &'a [u8],
}

#[derive(Debug, Clone)]
pub struct TrustedSigner {
    pub kid: Vec<u8>,
    pub public_key_sec1: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct TrustBundle {
    pub keys: Vec<TrustedSigner>,
}

impl TrustBundle {
    fn resolve(&self, kid: &[u8]) -> Result<&TrustedSigner, EntitlementError> {
        let mut matches = self.keys.iter().filter(|k| k.kid.as_slice() == kid);
        let first = matches.next().ok_or(EntitlementError::UnknownSigner)?;
        if matches.next().is_some() {
            return Err(EntitlementError::AmbiguousSigner);
        }
        Ok(first)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedGrant {
    pub activation_id: String,
    pub entitlement_id: String,
    pub product_id: String,
    pub edition: String,
    pub grants: Vec<String>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EntitlementError {
    #[error("artifact too large")]
    ArtifactTooLarge,
    #[error("malformed artifact")]
    MalformedArtifact,
    #[error("unsupported schema")]
    UnsupportedSchema,
    #[error("unsupported algorithm")]
    UnsupportedAlgorithm,
    #[error("unexpected unprotected headers")]
    UnexpectedUnprotectedHeaders,
    #[error("unexpected content type")]
    UnexpectedContentType,
    #[error("unknown signer")]
    UnknownSigner,
    #[error("ambiguous signer key id")]
    AmbiguousSigner,
    #[error("signature invalid")]
    SignatureInvalid,
    #[error("product mismatch")]
    ProductMismatch,
    #[error("subject mismatch")]
    SubjectMismatch,
    #[error("device mismatch")]
    DeviceMismatch,
    #[error("version not covered")]
    VersionNotCovered,
    #[error("time-bound right not implemented in slice A")]
    TimeBoundRightUnsupported,
}

pub struct EntitlementVerifier {
    trust: TrustBundle,
}

impl EntitlementVerifier {
    pub fn new(trust: TrustBundle) -> Self {
        Self { trust }
    }

    pub fn verify(
        &self,
        artifact: &[u8],
        ctx: &VerifyContext<'_>,
    ) -> Result<VerifiedGrant, EntitlementError> {
        if artifact.len() > MAX_ARTIFACT_SIZE {
            return Err(EntitlementError::ArtifactTooLarge);
        }

        let sign1 =
            CoseSign1::from_tagged_slice(artifact).map_err(|_| EntitlementError::MalformedArtifact)?;

        if !sign1.unprotected.is_empty() {
            return Err(EntitlementError::UnexpectedUnprotectedHeaders);
        }

        let protected = &sign1.protected.header;
        if !protected.crit.is_empty()
            || !protected.iv.is_empty()
            || !protected.partial_iv.is_empty()
            || !protected.counter_signatures.is_empty()
            || !protected.rest.is_empty()
        {
            return Err(EntitlementError::MalformedArtifact);
        }
        match protected.alg {
            Some(RegisteredLabelWithPrivate::Assigned(iana::Algorithm::ESP256)) => {}
            _ => return Err(EntitlementError::UnsupportedAlgorithm),
        }

        match &protected.content_type {
            Some(ContentType::Text(value)) if value == ENTITLEMENT_CONTENT_TYPE => {}
            _ => return Err(EntitlementError::UnexpectedContentType),
        }

        if protected.key_id.is_empty() {
            return Err(EntitlementError::UnknownSigner);
        }

        let signer = self.trust.resolve(&protected.key_id)?;

        let verifying_key = VerifyingKey::from_sec1_bytes(&signer.public_key_sec1)
            .map_err(|_| EntitlementError::UnknownSigner)?;

        if sign1.signature.len() != 64 {
            return Err(EntitlementError::SignatureInvalid);
        }

        sign1
            .verify_signature(&[], |sig_bytes, tbs| {
                let sig = Signature::from_slice(sig_bytes).map_err(|_| ())?;
                verifying_key.verify(tbs, &sig).map_err(|_| ())
            })
            .map_err(|_| EntitlementError::SignatureInvalid)?;

        let payload_bytes = sign1
            .payload
            .as_deref()
            .ok_or(EntitlementError::MalformedArtifact)?;

        let payload: ActivationPayloadV1 =
            ciborium::de::from_reader(Cursor::new(payload_bytes))
                .map_err(|_| EntitlementError::MalformedArtifact)?;

        if payload.schema_version != 1 {
            return Err(EntitlementError::UnsupportedSchema);
        }

        if payload.product_id != ctx.build.product_id {
            return Err(EntitlementError::ProductMismatch);
        }

        if let Some(expected_subject) = ctx.expected_subject {
            if payload.subject_id != expected_subject {
                return Err(EntitlementError::SubjectMismatch);
            }
        }

        if payload.device_key_id.as_slice() != ctx.expected_device_key_id {
            return Err(EntitlementError::DeviceMismatch);
        }

        match payload.right {
            RightV1::Perpetual {
                min_major,
                max_major,
                updates_until,
            } => {
                if ctx.build.major < min_major || ctx.build.major > max_major {
                    return Err(EntitlementError::VersionNotCovered);
                }
                if let Some(until) = updates_until {
                    if ctx.build.released_at > until {
                        return Err(EntitlementError::VersionNotCovered);
                    }
                }
            }
            RightV1::Subscription { .. } | RightV1::Trial { .. } => {
                return Err(EntitlementError::TimeBoundRightUnsupported);
            }
        }

        Ok(VerifiedGrant {
            activation_id: payload.activation_id,
            entitlement_id: payload.entitlement_id,
            product_id: payload.product_id,
            edition: payload.edition,
            grants: payload.grants,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coset::{CoseSign1Builder, HeaderBuilder};
    use p256::ecdsa::{SigningKey, signature::Signer};

    const TEST_KID: &[u8] = b"test-k1";
    const DEVICE_ID: &[u8] = b"device-a";

    fn signing_key() -> SigningKey {
        SigningKey::from_slice(&[7u8; 32]).expect("fixed test key")
    }

    fn trust_bundle() -> TrustBundle {
        let signing = signing_key();
        let verifying = signing.verifying_key();
        TrustBundle {
            keys: vec![TrustedSigner {
                kid: TEST_KID.to_vec(),
                public_key_sec1: verifying.to_encoded_point(false).as_bytes().to_vec(),
            }],
        }
    }

    fn payload() -> ActivationPayloadV1 {
        ActivationPayloadV1 {
            schema_version: 1,
            activation_id: "act-1".into(),
            entitlement_id: "ent-1".into(),
            subject_id: "user-1".into(),
            product_id: "chaptera.editor".into(),
            edition: "personal".into(),
            grants: vec!["edit".into(), "export".into()],
            device_key_id: DEVICE_ID.to_vec(),
            issued_at: 1_800_000_000,
            right: RightV1::Perpetual {
                min_major: 2,
                max_major: 2,
                updates_until: Some(1_900_000_000),
            },
        }
    }

    fn encode_payload(payload: &ActivationPayloadV1) -> Vec<u8> {
        let mut out = Vec::new();
        ciborium::ser::into_writer(payload, &mut out).expect("CBOR payload");
        out
    }

    fn sign_artifact(payload: &ActivationPayloadV1, kid: &[u8]) -> Vec<u8> {
        let signing = signing_key();
        let protected = HeaderBuilder::new()
            .algorithm(iana::Algorithm::ESP256)
            .key_id(kid.to_vec())
            .content_type(ENTITLEMENT_CONTENT_TYPE.to_owned())
            .build();

        CoseSign1Builder::new()
            .protected(protected)
            .payload(encode_payload(payload))
            .create_signature(&[], |tbs| {
                let sig: Signature = signing.sign(tbs);
                sig.to_bytes().to_vec()
            })
            .build()
            .to_tagged_vec()
            .expect("COSE")
    }

    fn ctx() -> VerifyContext<'static> {
        VerifyContext {
            build: BuildIdentity {
                product_id: "chaptera.editor",
                major: 2,
                released_at: 1_850_000_000,
            },
            expected_subject: Some("user-1"),
            expected_device_key_id: DEVICE_ID,
        }
    }

    #[test]
    fn valid_perpetual_entitlement_verifies() {
        let verifier = EntitlementVerifier::new(trust_bundle());
        let result = verifier.verify(&sign_artifact(&payload(), TEST_KID), &ctx());
        assert!(result.is_ok());
    }

    #[test]
    fn unknown_signer_is_rejected() {
        let verifier = EntitlementVerifier::new(trust_bundle());
        let err = verifier
            .verify(&sign_artifact(&payload(), b"unknown"), &ctx())
            .unwrap_err();
        assert_eq!(err, EntitlementError::UnknownSigner);
    }

    #[test]
    fn modified_payload_breaks_signature() {
        let verifier = EntitlementVerifier::new(trust_bundle());
        let artifact = sign_artifact(&payload(), TEST_KID);
        let mut sign1 = CoseSign1::from_tagged_slice(&artifact).unwrap();
        let mut changed = payload();
        changed.edition = "enterprise".into();
        sign1.payload = Some(encode_payload(&changed));
        let tampered = sign1.to_tagged_vec().unwrap();

        let err = verifier.verify(&tampered, &ctx()).unwrap_err();
        assert_eq!(err, EntitlementError::SignatureInvalid);
    }

    #[test]
    fn wrong_product_is_rejected() {
        let verifier = EntitlementVerifier::new(trust_bundle());
        let mut c = ctx();
        c.build.product_id = "chaptera.reader";
        let err = verifier
            .verify(&sign_artifact(&payload(), TEST_KID), &c)
            .unwrap_err();
        assert_eq!(err, EntitlementError::ProductMismatch);
    }

    #[test]
    fn wrong_device_is_rejected() {
        let verifier = EntitlementVerifier::new(trust_bundle());
        let c = VerifyContext {
            expected_device_key_id: b"device-b",
            ..ctx()
        };
        let err = verifier
            .verify(&sign_artifact(&payload(), TEST_KID), &c)
            .unwrap_err();
        assert_eq!(err, EntitlementError::DeviceMismatch);
    }

    #[test]
    fn unsupported_schema_is_rejected() {
        let verifier = EntitlementVerifier::new(trust_bundle());
        let mut p = payload();
        p.schema_version = 2;
        let err = verifier
            .verify(&sign_artifact(&p, TEST_KID), &ctx())
            .unwrap_err();
        assert_eq!(err, EntitlementError::UnsupportedSchema);
    }

    #[test]
    fn uncovered_major_is_rejected() {
        let verifier = EntitlementVerifier::new(trust_bundle());
        let mut c = ctx();
        c.build.major = 3;
        let err = verifier
            .verify(&sign_artifact(&payload(), TEST_KID), &c)
            .unwrap_err();
        assert_eq!(err, EntitlementError::VersionNotCovered);
    }

    #[test]
    fn build_after_updates_until_is_rejected_without_using_current_time() {
        let verifier = EntitlementVerifier::new(trust_bundle());
        let mut c = ctx();
        c.build.released_at = 1_950_000_000;
        let err = verifier
            .verify(&sign_artifact(&payload(), TEST_KID), &c)
            .unwrap_err();
        assert_eq!(err, EntitlementError::VersionNotCovered);
    }

    #[test]
    fn time_bound_rights_are_explicitly_not_implemented_in_slice_a() {
        let verifier = EntitlementVerifier::new(trust_bundle());
        let mut p = payload();
        p.right = RightV1::Subscription {
            lease_id: "lease-1".into(),
            lease_sequence: 1,
            not_before: 1_800_000_000,
            valid_until: 1_900_000_000,
            offline_grace_until: None,
        };

        let err = verifier
            .verify(&sign_artifact(&p, TEST_KID), &ctx())
            .unwrap_err();
        assert_eq!(err, EntitlementError::TimeBoundRightUnsupported);
    }

    #[test]
    fn oversized_artifact_is_rejected_before_parsing() {
        let verifier = EntitlementVerifier::new(trust_bundle());
        let err = verifier
            .verify(&vec![0u8; MAX_ARTIFACT_SIZE + 1], &ctx())
            .unwrap_err();
        assert_eq!(err, EntitlementError::ArtifactTooLarge);
    }
}
