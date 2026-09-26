use crate::{
    EntitlementError, MAX_ARTIFACT_SIZE, MAX_KID_LEN, MAX_PRODUCT_ID_LEN, TrustBundle,
    bounded_nonempty,
};
use coset::{
    ContentType, CoseSign1, RegisteredLabelWithPrivate, TaggedCborSerializable, iana,
};
use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
use serde::{Deserialize, Serialize};
use std::io::Cursor;

pub const BUILD_IDENTITY_CONTENT_TYPE: &str =
    "application/vnd.chaptera.build-identity+cbor";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildIdentityPayloadV1 {
    pub schema_version: u16,
    pub product_id: String,
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub released_at: i64,
    pub release_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedBuildIdentity {
    product_id: String,
    major: u32,
    minor: u32,
    patch: u32,
    released_at: i64,
    release_sequence: u64,
}

impl TrustedBuildIdentity {
    pub fn product_id(&self) -> &str {
        &self.product_id
    }

    pub fn major(&self) -> u32 {
        self.major
    }

    pub fn minor(&self) -> u32 {
        self.minor
    }

    pub fn patch(&self) -> u32 {
        self.patch
    }

    pub fn released_at(&self) -> i64 {
        self.released_at
    }

    pub fn release_sequence(&self) -> u64 {
        self.release_sequence
    }
}

pub(crate) fn verify_build_identity(
    artifact: &[u8],
    trust: &TrustBundle,
) -> Result<TrustedBuildIdentity, EntitlementError> {
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
        Some(ContentType::Text(value)) if value == BUILD_IDENTITY_CONTENT_TYPE => {}
        _ => return Err(EntitlementError::UnexpectedContentType),
    }

    if protected.key_id.is_empty() {
        return Err(EntitlementError::UnknownSigner);
    }
    if protected.key_id.len() > MAX_KID_LEN {
        return Err(EntitlementError::MalformedArtifact);
    }

    let signer = trust.resolve(&protected.key_id)?;
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

    let payload: BuildIdentityPayloadV1 =
        ciborium::de::from_reader(Cursor::new(payload_bytes))
            .map_err(|_| EntitlementError::MalformedArtifact)?;

    if payload.schema_version != 1 {
        return Err(EntitlementError::UnsupportedSchema);
    }

    if !bounded_nonempty(&payload.product_id, MAX_PRODUCT_ID_LEN)
        || payload.released_at < 0
        || payload.release_sequence == 0
    {
        return Err(EntitlementError::BuildIdentityPolicyViolation);
    }

    Ok(TrustedBuildIdentity {
        product_id: payload.product_id,
        major: payload.major,
        minor: payload.minor,
        patch: payload.patch,
        released_at: payload.released_at,
        release_sequence: payload.release_sequence,
    })
}
