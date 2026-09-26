mod build_identity;
mod trusted_time;
#[cfg(target_os = "windows")]
mod windows_platform;
pub use build_identity::{
    BUILD_IDENTITY_CONTENT_TYPE, BuildIdentityPayloadV1, TrustedBuildIdentity,
};
pub use trusted_time::{
    LEASE_COMMITMENT_LEN, LeaseTimeInputV1, TimeAcceptance, TimePolicy, TrustedTimeError,
    TrustedTimeStateV1, evaluate_time_bound_right,
};
#[cfg(target_os = "windows")]
pub use windows_platform::{
    DeviceKeyBacking, WindowsDeviceKey, WindowsPlatformError, load_trusted_time_state,
    save_trusted_time_state,
};

use coset::{
    iana, ContentType, CoseSign1, RegisteredLabelWithPrivate, TaggedCborSerializable,
};
use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use thiserror::Error;

pub const ENTITLEMENT_CONTENT_TYPE: &str = "application/vnd.chaptera.entitlement+cbor";
pub const MAX_ARTIFACT_SIZE: usize = 16 * 1024;
pub const MAX_KID_LEN: usize = 64;
pub const DEVICE_KEY_ID_LEN: usize = 32;
pub const MAX_ID_LEN: usize = 128;
pub const MAX_PRODUCT_ID_LEN: usize = 64;
pub const MAX_GRANTS: usize = 64;
pub const MAX_GRANT_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationPayloadV1 {
    pub schema_version: u16,
    pub activation_id: String,
    pub entitlement_id: String,
    pub product_id: String,
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
pub struct VerifyContext<'a> {
    pub build_identity_artifact: Vec<u8>,
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
    pub(crate) fn resolve(&self, kid: &[u8]) -> Result<&TrustedSigner, EntitlementError> {
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
    #[error("invalid local trust bundle")]
    InvalidTrustBundle,
    #[error("signed payload violates Chaptera V1 policy")]
    PayloadPolicyViolation,
    #[error("signed build identity violates Chaptera V1 policy")]
    BuildIdentityPolicyViolation,
    #[error("product mismatch")]
    ProductMismatch,
    #[error("device mismatch")]
    DeviceMismatch,
    #[error("version not covered")]
    VersionNotCovered,
    #[error("time-bound right not implemented in slice A")]
    TimeBoundRightUnsupported,
}

pub struct EntitlementVerifier {
    trust: TrustBundle,
    build_trust: TrustBundle,
}

pub(crate) fn bounded_nonempty(value: &str, max_len: usize) -> bool {
    !value.is_empty() && value.len() <= max_len
}

fn validate_payload_shape(payload: &ActivationPayloadV1) -> Result<(), EntitlementError> {
    if !bounded_nonempty(&payload.activation_id, MAX_ID_LEN)
        || !bounded_nonempty(&payload.entitlement_id, MAX_ID_LEN)
        || !bounded_nonempty(&payload.product_id, MAX_PRODUCT_ID_LEN)
        || payload.device_key_id.len() != DEVICE_KEY_ID_LEN
        || payload.grants.is_empty()
        || payload.grants.len() > MAX_GRANTS
        || payload
            .grants
            .iter()
            .any(|grant| !bounded_nonempty(grant, MAX_GRANT_LEN))
    {
        return Err(EntitlementError::PayloadPolicyViolation);
    }

    for (index, grant) in payload.grants.iter().enumerate() {
        if payload.grants[..index].iter().any(|seen| seen == grant) {
            return Err(EntitlementError::PayloadPolicyViolation);
        }
    }

    if let RightV1::Perpetual {
        min_major,
        max_major,
        ..
    } = &payload.right
        && min_major > max_major
    {
        return Err(EntitlementError::PayloadPolicyViolation);
    }

    Ok(())
}

impl EntitlementVerifier {
    pub fn new(trust: TrustBundle, build_trust: TrustBundle) -> Self {
        Self { trust, build_trust }
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
        match protected.alg.as_ref() {
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
        if protected.key_id.len() > MAX_KID_LEN {
            return Err(EntitlementError::MalformedArtifact);
        }

        let signer = self.trust.resolve(&protected.key_id)?;
        if signer.public_key_sec1.len() != 65 || signer.public_key_sec1.first() != Some(&0x04) {
            return Err(EntitlementError::InvalidTrustBundle);
        }

        let verifying_key = VerifyingKey::from_sec1_bytes(&signer.public_key_sec1)
            .map_err(|_| EntitlementError::InvalidTrustBundle)?;

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
        validate_payload_shape(&payload)?;

        if ctx.expected_device_key_id.len() != DEVICE_KEY_ID_LEN {
            return Err(EntitlementError::PayloadPolicyViolation);
        }

        let build =
            build_identity::verify_build_identity(&ctx.build_identity_artifact, &self.build_trust)?;

        if payload.product_id != build.product_id() {
            return Err(EntitlementError::ProductMismatch);
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
                if build.major() < min_major || build.major() > max_major {
                    return Err(EntitlementError::VersionNotCovered);
                }
                if let Some(until) = updates_until
                    && build.released_at() > until
                {
                    return Err(EntitlementError::VersionNotCovered);
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
            grants: payload.grants,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ciborium::value::Value;
    use coset::{CoseSign1Builder, HeaderBuilder};
    use p256::ecdsa::{SigningKey, signature::Signer};

    const TEST_KID: &[u8] = b"test-k1";
    const BUILD_TEST_KID: &[u8] = b"build-test-k1";
    static DEVICE_ID: [u8; DEVICE_KEY_ID_LEN] = [0xA5; DEVICE_KEY_ID_LEN];

    fn signing_key() -> SigningKey {
        SigningKey::from_slice(&[7u8; 32]).expect("fixed test key")
    }

    fn build_signing_key() -> SigningKey {
        SigningKey::from_slice(&[8u8; 32]).expect("fixed build test key")
    }

    fn trust_bundle() -> TrustBundle {
        let signing = signing_key();
        let verifying = signing.verifying_key();
        TrustBundle {
            keys: vec![TrustedSigner {
                kid: TEST_KID.to_vec(),
                public_key_sec1: verifying.to_sec1_point(false).as_bytes().to_vec(),
            }],
        }
    }

    fn build_trust_bundle() -> TrustBundle {
        let signing = build_signing_key();
        let verifying = signing.verifying_key();
        TrustBundle {
            keys: vec![TrustedSigner {
                kid: BUILD_TEST_KID.to_vec(),
                public_key_sec1: verifying.to_sec1_point(false).as_bytes().to_vec(),
            }],
        }
    }

    fn verifier() -> EntitlementVerifier {
        EntitlementVerifier::new(trust_bundle(), build_trust_bundle())
    }

    fn verifier_with_entitlement_trust(trust: TrustBundle) -> EntitlementVerifier {
        EntitlementVerifier::new(trust, build_trust_bundle())
    }

    fn payload() -> ActivationPayloadV1 {
        ActivationPayloadV1 {
            schema_version: 1,
            activation_id: "act-1".into(),
            entitlement_id: "ent-1".into(),
            product_id: "chaptera.editor".into(),
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

    fn sign_payload_bytes(payload_bytes: Vec<u8>, kid: &[u8]) -> Vec<u8> {
        let signing = signing_key();
        let protected = HeaderBuilder::new()
            .algorithm(iana::Algorithm::ESP256)
            .key_id(kid.to_vec())
            .content_type(ENTITLEMENT_CONTENT_TYPE.to_owned())
            .build();

        CoseSign1Builder::new()
            .protected(protected)
            .payload(payload_bytes)
            .create_signature(&[], |tbs| {
                let sig: Signature = signing.sign(tbs);
                sig.to_bytes().to_vec()
            })
            .build()
            .to_tagged_vec()
            .expect("COSE")
    }

    fn sign_artifact(payload: &ActivationPayloadV1, kid: &[u8]) -> Vec<u8> {
        sign_payload_bytes(encode_payload(payload), kid)
    }

    fn build_payload() -> BuildIdentityPayloadV1 {
        BuildIdentityPayloadV1 {
            schema_version: 1,
            product_id: "chaptera.editor".into(),
            major: 2,
            minor: 4,
            patch: 1,
            released_at: 1_850_000_000,
            release_sequence: 42,
        }
    }

    fn encode_build_payload(payload: &BuildIdentityPayloadV1) -> Vec<u8> {
        let mut out = Vec::new();
        ciborium::ser::into_writer(payload, &mut out).expect("build CBOR payload");
        out
    }

    fn sign_build_payload_with(
        payload: &BuildIdentityPayloadV1,
        signing: &SigningKey,
        kid: &[u8],
    ) -> Vec<u8> {
        let protected = HeaderBuilder::new()
            .algorithm(iana::Algorithm::ESP256)
            .key_id(kid.to_vec())
            .content_type(BUILD_IDENTITY_CONTENT_TYPE.to_owned())
            .build();

        CoseSign1Builder::new()
            .protected(protected)
            .payload(encode_build_payload(payload))
            .create_signature(&[], |tbs| {
                let sig: Signature = signing.sign(tbs);
                sig.to_bytes().to_vec()
            })
            .build()
            .to_tagged_vec()
            .expect("build COSE")
    }

    fn sign_build_identity(payload: &BuildIdentityPayloadV1) -> Vec<u8> {
        sign_build_payload_with(payload, &build_signing_key(), BUILD_TEST_KID)
    }

    fn ctx() -> VerifyContext<'static> {
        VerifyContext {
            build_identity_artifact: sign_build_identity(&build_payload()),
            expected_device_key_id: &DEVICE_ID,
        }
    }

    #[test]
    fn build_release_timestamp_is_signed_authority_not_caller_input() {
        let verifier = verifier();
        let mut build = build_payload();
        build.released_at = 1_950_000_000;
        let context = VerifyContext {
            build_identity_artifact: sign_build_identity(&build),
            expected_device_key_id: &DEVICE_ID,
        };

        let err = verifier
            .verify(&sign_artifact(&payload(), TEST_KID), &context)
            .unwrap_err();
        assert_eq!(err, EntitlementError::VersionNotCovered);
    }

    #[test]
    fn entitlement_issuer_key_cannot_sign_build_identity() {
        let verifier = verifier();
        let context = VerifyContext {
            build_identity_artifact: sign_build_payload_with(
                &build_payload(),
                &signing_key(),
                TEST_KID,
            ),
            expected_device_key_id: &DEVICE_ID,
        };

        let err = verifier
            .verify(&sign_artifact(&payload(), TEST_KID), &context)
            .unwrap_err();
        assert_eq!(err, EntitlementError::UnknownSigner);
    }

    #[test]
    fn tampered_build_identity_signature_is_rejected() {
        let verifier = verifier();
        let artifact = sign_build_identity(&build_payload());
        let mut sign1 = CoseSign1::from_tagged_slice(&artifact).unwrap();
        let mut changed = build_payload();
        changed.released_at += 1;
        sign1.payload = Some(encode_build_payload(&changed));
        let context = VerifyContext {
            build_identity_artifact: sign1.to_tagged_vec().unwrap(),
            expected_device_key_id: &DEVICE_ID,
        };

        let err = verifier
            .verify(&sign_artifact(&payload(), TEST_KID), &context)
            .unwrap_err();
        assert_eq!(err, EntitlementError::SignatureInvalid);
    }

    #[test]
    fn malformed_build_identity_policy_is_rejected() {
        let verifier = verifier();
        let mut build = build_payload();
        build.release_sequence = 0;
        let context = VerifyContext {
            build_identity_artifact: sign_build_identity(&build),
            expected_device_key_id: &DEVICE_ID,
        };

        let err = verifier
            .verify(&sign_artifact(&payload(), TEST_KID), &context)
            .unwrap_err();
        assert_eq!(err, EntitlementError::BuildIdentityPolicyViolation);
    }

    #[test]
    fn build_identity_product_must_match_entitlement_product() {
        let verifier = verifier();
        let mut build = build_payload();
        build.product_id = "chaptera.reader".into();
        let context = VerifyContext {
            build_identity_artifact: sign_build_identity(&build),
            expected_device_key_id: &DEVICE_ID,
        };

        let err = verifier
            .verify(&sign_artifact(&payload(), TEST_KID), &context)
            .unwrap_err();
        assert_eq!(err, EntitlementError::ProductMismatch);
    }

    #[test]
    fn valid_perpetual_entitlement_verifies() {
        let verifier = verifier();
        let result = verifier.verify(&sign_artifact(&payload(), TEST_KID), &ctx());
        assert!(result.is_ok());
    }

    #[test]
    fn ambiguous_signer_key_id_is_rejected() {
        let signing = signing_key();
        let public = signing
            .verifying_key()
            .to_sec1_point(false)
            .as_bytes()
            .to_vec();
        let verifier = verifier_with_entitlement_trust(TrustBundle {
            keys: vec![
                TrustedSigner {
                    kid: TEST_KID.to_vec(),
                    public_key_sec1: public.clone(),
                },
                TrustedSigner {
                    kid: TEST_KID.to_vec(),
                    public_key_sec1: public,
                },
            ],
        });

        let err = verifier
            .verify(&sign_artifact(&payload(), TEST_KID), &ctx())
            .unwrap_err();
        assert_eq!(err, EntitlementError::AmbiguousSigner);
    }

    #[test]
    fn unprotected_headers_are_rejected_even_if_signature_would_still_verify() {
        let verifier = verifier();
        let artifact = sign_artifact(&payload(), TEST_KID);
        let mut sign1 = CoseSign1::from_tagged_slice(&artifact).unwrap();
        sign1.unprotected.content_type =
            Some(ContentType::Text("text/plain".to_owned()));
        let changed = sign1.to_tagged_vec().unwrap();

        let err = verifier.verify(&changed, &ctx()).unwrap_err();
        assert_eq!(err, EntitlementError::UnexpectedUnprotectedHeaders);
    }

    #[test]
    fn unknown_signer_is_rejected() {
        let verifier = verifier();
        let err = verifier
            .verify(&sign_artifact(&payload(), b"unknown"), &ctx())
            .unwrap_err();
        assert_eq!(err, EntitlementError::UnknownSigner);
    }

    #[test]
    fn modified_payload_breaks_signature() {
        let verifier = verifier();
        let artifact = sign_artifact(&payload(), TEST_KID);
        let mut sign1 = CoseSign1::from_tagged_slice(&artifact).unwrap();
        let mut changed = payload();
        changed.entitlement_id = "ent-tampered".into();
        sign1.payload = Some(encode_payload(&changed));
        let tampered = sign1.to_tagged_vec().unwrap();

        let err = verifier.verify(&tampered, &ctx()).unwrap_err();
        assert_eq!(err, EntitlementError::SignatureInvalid);
    }

    #[test]
    fn wrong_product_is_rejected() {
        let verifier = verifier();
        let mut c = ctx();
        let mut build = build_payload();
        build.product_id = "chaptera.reader".into();
        c.build_identity_artifact = sign_build_identity(&build);
        let err = verifier
            .verify(&sign_artifact(&payload(), TEST_KID), &c)
            .unwrap_err();
        assert_eq!(err, EntitlementError::ProductMismatch);
    }

    #[test]
    fn wrong_device_is_rejected() {
        let verifier = verifier();
        let c = VerifyContext {
            expected_device_key_id: &[0x5A; DEVICE_KEY_ID_LEN],
            ..ctx()
        };
        let err = verifier
            .verify(&sign_artifact(&payload(), TEST_KID), &c)
            .unwrap_err();
        assert_eq!(err, EntitlementError::DeviceMismatch);
    }

    #[test]
    fn unsupported_schema_is_rejected() {
        let verifier = verifier();
        let mut p = payload();
        p.schema_version = 2;
        let err = verifier
            .verify(&sign_artifact(&p, TEST_KID), &ctx())
            .unwrap_err();
        assert_eq!(err, EntitlementError::UnsupportedSchema);
    }

    #[test]
    fn uncovered_major_is_rejected() {
        let verifier = verifier();
        let mut c = ctx();
        let mut build = build_payload();
        build.major = 3;
        c.build_identity_artifact = sign_build_identity(&build);
        let err = verifier
            .verify(&sign_artifact(&payload(), TEST_KID), &c)
            .unwrap_err();
        assert_eq!(err, EntitlementError::VersionNotCovered);
    }

    #[test]
    fn build_after_updates_until_is_rejected_without_using_current_time() {
        let verifier = verifier();
        let mut c = ctx();
        let mut build = build_payload();
        build.released_at = 1_950_000_000;
        c.build_identity_artifact = sign_build_identity(&build);
        let err = verifier
            .verify(&sign_artifact(&payload(), TEST_KID), &c)
            .unwrap_err();
        assert_eq!(err, EntitlementError::VersionNotCovered);
    }

    #[test]
    fn time_bound_rights_are_explicitly_not_implemented_in_slice_a() {
        let verifier = verifier();
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
    fn wrong_algorithm_is_rejected_before_signature_verification() {
        let verifier = verifier();
        let artifact = sign_artifact(&payload(), TEST_KID);
        let mut sign1 = CoseSign1::from_tagged_slice(&artifact).unwrap();
        sign1.protected.header.alg =
            Some(RegisteredLabelWithPrivate::Assigned(iana::Algorithm::EdDSA));
        sign1.protected.original_data = None;
        let changed = sign1.to_tagged_vec().unwrap();

        let err = verifier.verify(&changed, &ctx()).unwrap_err();
        assert_eq!(err, EntitlementError::UnsupportedAlgorithm);
    }

    #[test]
    fn wrong_content_type_is_rejected_before_signature_verification() {
        let verifier = verifier();
        let artifact = sign_artifact(&payload(), TEST_KID);
        let mut sign1 = CoseSign1::from_tagged_slice(&artifact).unwrap();
        sign1.protected.header.content_type =
            Some(ContentType::Text("application/octet-stream".to_owned()));
        sign1.protected.original_data = None;
        let changed = sign1.to_tagged_vec().unwrap();

        let err = verifier.verify(&changed, &ctx()).unwrap_err();
        assert_eq!(err, EntitlementError::UnexpectedContentType);
    }

    #[test]
    fn overlong_kid_is_rejected_before_trust_lookup() {
        let verifier = verifier();
        let artifact = sign_artifact(&payload(), TEST_KID);
        let mut sign1 = CoseSign1::from_tagged_slice(&artifact).unwrap();
        sign1.protected.header.key_id = vec![b'k'; MAX_KID_LEN + 1];
        sign1.protected.original_data = None;
        let changed = sign1.to_tagged_vec().unwrap();

        let err = verifier.verify(&changed, &ctx()).unwrap_err();
        assert_eq!(err, EntitlementError::MalformedArtifact);
    }

    #[test]
    fn wrong_device_key_id_width_is_rejected_as_payload_policy() {
        let verifier = verifier();
        let mut p = payload();
        p.device_key_id = vec![0xA5; DEVICE_KEY_ID_LEN - 1];

        let err = verifier
            .verify(&sign_artifact(&p, TEST_KID), &ctx())
            .unwrap_err();
        assert_eq!(err, EntitlementError::PayloadPolicyViolation);
    }

    #[test]
    fn duplicate_grants_are_rejected() {
        let verifier = verifier();
        let mut p = payload();
        p.grants = vec!["edit".into(), "edit".into()];

        let err = verifier
            .verify(&sign_artifact(&p, TEST_KID), &ctx())
            .unwrap_err();
        assert_eq!(err, EntitlementError::PayloadPolicyViolation);
    }

    #[test]
    fn excessive_grant_count_is_rejected() {
        let verifier = verifier();
        let mut p = payload();
        p.grants = (0..=MAX_GRANTS).map(|i| format!("g{i}")).collect();

        let err = verifier
            .verify(&sign_artifact(&p, TEST_KID), &ctx())
            .unwrap_err();
        assert_eq!(err, EntitlementError::PayloadPolicyViolation);
    }

    #[test]
    fn inverted_perpetual_major_range_is_rejected() {
        let verifier = verifier();
        let mut p = payload();
        p.right = RightV1::Perpetual {
            min_major: 3,
            max_major: 2,
            updates_until: None,
        };

        let err = verifier
            .verify(&sign_artifact(&p, TEST_KID), &ctx())
            .unwrap_err();
        assert_eq!(err, EntitlementError::PayloadPolicyViolation);
    }

    #[test]
    fn signed_payload_with_unknown_field_is_rejected() {
        let verifier = verifier();
        let mut value = Value::serialized(&payload()).expect("payload value");
        let Value::Map(entries) = &mut value else {
            panic!("payload must serialize as a CBOR map");
        };
        entries.push((
            Value::Text("unexpected_authority".to_owned()),
            Value::Text("must-not-be-ignored".to_owned()),
        ));

        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&value, &mut bytes).expect("CBOR value");
        let err = verifier
            .verify(&sign_payload_bytes(bytes, TEST_KID), &ctx())
            .unwrap_err();
        assert_eq!(err, EntitlementError::MalformedArtifact);
    }

    #[test]
    fn local_artifact_rejects_subject_and_edition_authority_fields() {
        let verifier = verifier();
        let mut value = Value::serialized(&payload()).expect("payload value");
        let Value::Map(entries) = &mut value else {
            panic!("payload must serialize as a CBOR map");
        };
        entries.push((
            Value::Text("subject_id".to_owned()),
            Value::Text("global-user-123".to_owned()),
        ));
        entries.push((
            Value::Text("edition".to_owned()),
            Value::Text("enterprise".to_owned()),
        ));

        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&value, &mut bytes).expect("CBOR value");
        let err = verifier
            .verify(&sign_payload_bytes(bytes, TEST_KID), &ctx())
            .unwrap_err();
        assert_eq!(err, EntitlementError::MalformedArtifact);
    }

    #[test]
    fn signed_payload_with_duplicate_cbor_key_is_rejected() {
        let verifier = verifier();
        let mut value = Value::serialized(&payload()).expect("payload value");
        let Value::Map(entries) = &mut value else {
            panic!("payload must serialize as a CBOR map");
        };
        entries.push((
            Value::Text("product_id".to_owned()),
            Value::Text("chaptera.editor".to_owned()),
        ));

        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&value, &mut bytes).expect("CBOR value");
        let err = verifier
            .verify(&sign_payload_bytes(bytes, TEST_KID), &ctx())
            .unwrap_err();
        assert_eq!(err, EntitlementError::MalformedArtifact);
    }

    #[test]
    fn non_64_byte_signature_is_rejected_before_crypto_verification() {
        let verifier = verifier();
        let artifact = sign_artifact(&payload(), TEST_KID);
        let mut sign1 = CoseSign1::from_tagged_slice(&artifact).unwrap();
        sign1.signature.truncate(63);
        let changed = sign1.to_tagged_vec().unwrap();

        let err = verifier.verify(&changed, &ctx()).unwrap_err();
        assert_eq!(err, EntitlementError::SignatureInvalid);
    }

    #[test]
    fn oversized_grant_string_is_rejected() {
        let verifier = verifier();
        let mut p = payload();
        p.grants = vec!["g".repeat(MAX_GRANT_LEN + 1)];

        let err = verifier
            .verify(&sign_artifact(&p, TEST_KID), &ctx())
            .unwrap_err();
        assert_eq!(err, EntitlementError::PayloadPolicyViolation);
    }

    #[test]
    fn compressed_or_noncanonical_local_trust_key_is_rejected() {
        let signing = signing_key();
        let compressed = signing
            .verifying_key()
            .to_sec1_point(true)
            .as_bytes()
            .to_vec();
        let verifier = verifier_with_entitlement_trust(TrustBundle {
            keys: vec![TrustedSigner {
                kid: TEST_KID.to_vec(),
                public_key_sec1: compressed,
            }],
        });

        let err = verifier
            .verify(&sign_artifact(&payload(), TEST_KID), &ctx())
            .unwrap_err();
        assert_eq!(err, EntitlementError::InvalidTrustBundle);
    }

    #[test]
    fn all_single_byte_mutations_of_valid_artifact_fail_closed_without_panicking() {
        let verifier = verifier();
        let artifact = sign_artifact(&payload(), TEST_KID);

        for index in 0..artifact.len() {
            let mut mutated = artifact.clone();
            mutated[index] ^= 0x01;
            assert!(
                verifier.verify(&mutated, &ctx()).is_err(),
                "single-byte mutation at index {index} unexpectedly verified"
            );
        }
    }

    #[test]
    fn every_truncation_of_valid_artifact_fails_closed() {
        let verifier = verifier();
        let artifact = sign_artifact(&payload(), TEST_KID);

        for len in 0..artifact.len() {
            assert!(
                verifier.verify(&artifact[..len], &ctx()).is_err(),
                "truncation at length {len} unexpectedly verified"
            );
        }
    }

    #[test]
    fn deterministic_garbage_corpus_never_panics_or_verifies() {
        let verifier = verifier();
        let mut state = 0xD1CE_BA5E_F00D_CAFEu64;

        for len in 0..=1024usize {
            let mut bytes = vec![0u8; len];
            for byte in &mut bytes {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            assert!(verifier.verify(&bytes, &ctx()).is_err());
        }
    }

    #[test]
    fn oversized_artifact_is_rejected_before_parsing() {
        let verifier = verifier();
        let err = verifier
            .verify(&vec![0u8; MAX_ARTIFACT_SIZE + 1], &ctx())
            .unwrap_err();
        assert_eq!(err, EntitlementError::ArtifactTooLarge);
    }
}
