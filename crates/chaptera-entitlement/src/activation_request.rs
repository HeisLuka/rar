use crate::{DEVICE_KEY_ID_LEN, MAX_ID_LEN, MAX_PRODUCT_ID_LEN, bounded_nonempty};
use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use thiserror::Error;

pub const ACTIVATION_REQUEST_MAX_ARTIFACT_SIZE: usize = 8 * 1024;
pub const ACTIVATION_REQUEST_MAX_FACTS_SIZE: usize = 4 * 1024;
pub(crate) const ACTIVATION_REQUEST_SIGNATURE_DOMAIN: &[u8] =
    b"Chaptera.ActivationRequest.signature.v1\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationRequestFactsV1 {
    pub schema_version: u16,
    pub request_id: String,
    pub product_id: String,
    pub requested_major: u32,
    pub request_nonce: [u8; 32],
    pub device_key_id: [u8; DEVICE_KEY_ID_LEN],
    pub device_public_key_sec1: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivationRequestEnvelopeV1 {
    signed_facts: Vec<u8>,
    signature: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedActivationRequest {
    request_id: String,
    product_id: String,
    requested_major: u32,
    request_nonce: [u8; 32],
    device_key_id: [u8; DEVICE_KEY_ID_LEN],
    device_public_key_sec1: [u8; 65],
}

impl VerifiedActivationRequest {
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn product_id(&self) -> &str {
        &self.product_id
    }

    pub fn requested_major(&self) -> u32 {
        self.requested_major
    }

    pub fn request_nonce(&self) -> &[u8; 32] {
        &self.request_nonce
    }

    pub fn device_key_id(&self) -> &[u8; DEVICE_KEY_ID_LEN] {
        &self.device_key_id
    }

    pub fn device_public_key_sec1(&self) -> &[u8; 65] {
        &self.device_public_key_sec1
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ActivationRequestError {
    #[error("activation request artifact too large")]
    ArtifactTooLarge,
    #[error("activation request is malformed")]
    MalformedArtifact,
    #[error("activation request facts are too large")]
    FactsTooLarge,
    #[error("unsupported activation request schema")]
    UnsupportedSchema,
    #[error("activation request violates V1 policy")]
    PolicyViolation,
    #[error("activation request product mismatch")]
    ProductMismatch,
    #[error("activation request DeviceKeyId does not match its public key")]
    DeviceKeyIdMismatch,
    #[error("activation request signature is invalid")]
    SignatureInvalid,
}

fn decode_exact<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, ActivationRequestError> {
    let mut cursor = Cursor::new(bytes);
    let value = ciborium::de::from_reader(&mut cursor)
        .map_err(|_| ActivationRequestError::MalformedArtifact)?;
    if cursor.position() != bytes.len() as u64 {
        return Err(ActivationRequestError::MalformedArtifact);
    }
    Ok(value)
}

fn validate_facts(facts: &ActivationRequestFactsV1) -> Result<[u8; 65], ActivationRequestError> {
    if facts.schema_version != 1 {
        return Err(ActivationRequestError::UnsupportedSchema);
    }
    if !bounded_nonempty(&facts.request_id, MAX_ID_LEN)
        || !bounded_nonempty(&facts.product_id, MAX_PRODUCT_ID_LEN)
        || facts.device_public_key_sec1.len() != 65
        || facts.device_public_key_sec1.first() != Some(&0x04)
    {
        return Err(ActivationRequestError::PolicyViolation);
    }

    let public: [u8; 65] = facts
        .device_public_key_sec1
        .as_slice()
        .try_into()
        .map_err(|_| ActivationRequestError::PolicyViolation)?;
    VerifyingKey::from_sec1_bytes(&public)
        .map_err(|_| ActivationRequestError::PolicyViolation)?;

    let derived: [u8; DEVICE_KEY_ID_LEN] = Sha256::digest(public).into();
    if derived != facts.device_key_id {
        return Err(ActivationRequestError::DeviceKeyIdMismatch);
    }
    Ok(public)
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn encode_activation_request_facts(
    facts: &ActivationRequestFactsV1,
) -> Result<Vec<u8>, ActivationRequestError> {
    validate_facts(facts)?;
    let mut encoded = Vec::new();
    ciborium::ser::into_writer(facts, &mut encoded)
        .map_err(|_| ActivationRequestError::MalformedArtifact)?;
    if encoded.len() > ACTIVATION_REQUEST_MAX_FACTS_SIZE {
        return Err(ActivationRequestError::FactsTooLarge);
    }
    Ok(encoded)
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn assemble_activation_request(
    signed_facts: Vec<u8>,
    signature: [u8; 64],
) -> Result<Vec<u8>, ActivationRequestError> {
    if signed_facts.len() > ACTIVATION_REQUEST_MAX_FACTS_SIZE {
        return Err(ActivationRequestError::FactsTooLarge);
    }
    let envelope = ActivationRequestEnvelopeV1 {
        signed_facts,
        signature: signature.to_vec(),
    };
    let mut artifact = Vec::new();
    ciborium::ser::into_writer(&envelope, &mut artifact)
        .map_err(|_| ActivationRequestError::MalformedArtifact)?;
    if artifact.len() > ACTIVATION_REQUEST_MAX_ARTIFACT_SIZE {
        return Err(ActivationRequestError::ArtifactTooLarge);
    }
    Ok(artifact)
}

pub fn verify_activation_request(
    artifact: &[u8],
    expected_product_id: &str,
) -> Result<VerifiedActivationRequest, ActivationRequestError> {
    if artifact.len() > ACTIVATION_REQUEST_MAX_ARTIFACT_SIZE {
        return Err(ActivationRequestError::ArtifactTooLarge);
    }

    let envelope: ActivationRequestEnvelopeV1 = decode_exact(artifact)?;
    if envelope.signed_facts.len() > ACTIVATION_REQUEST_MAX_FACTS_SIZE {
        return Err(ActivationRequestError::FactsTooLarge);
    }
    if envelope.signature.len() != 64 {
        return Err(ActivationRequestError::SignatureInvalid);
    }

    let facts: ActivationRequestFactsV1 = decode_exact(&envelope.signed_facts)?;
    let public = validate_facts(&facts)?;

    let verifying_key = VerifyingKey::from_sec1_bytes(&public)
        .map_err(|_| ActivationRequestError::PolicyViolation)?;
    let signature =
        Signature::from_slice(&envelope.signature).map_err(|_| ActivationRequestError::SignatureInvalid)?;

    let mut signed_message =
        Vec::with_capacity(ACTIVATION_REQUEST_SIGNATURE_DOMAIN.len() + envelope.signed_facts.len());
    signed_message.extend_from_slice(ACTIVATION_REQUEST_SIGNATURE_DOMAIN);
    signed_message.extend_from_slice(&envelope.signed_facts);
    verifying_key
        .verify(&signed_message, &signature)
        .map_err(|_| ActivationRequestError::SignatureInvalid)?;

    if facts.product_id != expected_product_id {
        return Err(ActivationRequestError::ProductMismatch);
    }

    Ok(VerifiedActivationRequest {
        request_id: facts.request_id,
        product_id: facts.product_id,
        requested_major: facts.requested_major,
        request_nonce: facts.request_nonce,
        device_key_id: facts.device_key_id,
        device_public_key_sec1: public,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::{SigningKey, signature::Signer};

    fn signed_request() -> Vec<u8> {
        let signing = SigningKey::from_slice(&[0x31; 32]).expect("test DeviceKey");
        let public = signing
            .verifying_key()
            .to_sec1_point(false)
            .as_bytes()
            .to_vec();
        let device_key_id: [u8; DEVICE_KEY_ID_LEN] = Sha256::digest(&public).into();
        let facts = ActivationRequestFactsV1 {
            schema_version: 1,
            request_id: "req-1".into(),
            product_id: "chaptera.editor".into(),
            requested_major: 2,
            request_nonce: [0x91; 32],
            device_key_id,
            device_public_key_sec1: public,
        };
        let signed_facts = encode_activation_request_facts(&facts).expect("facts");
        let mut message =
            Vec::with_capacity(ACTIVATION_REQUEST_SIGNATURE_DOMAIN.len() + signed_facts.len());
        message.extend_from_slice(ACTIVATION_REQUEST_SIGNATURE_DOMAIN);
        message.extend_from_slice(&signed_facts);
        let signature: Signature = signing.sign(&message);
        let signature_bytes: [u8; 64] = signature
            .to_bytes()
            .as_slice()
            .try_into()
            .expect("P-256 signature width");
        assemble_activation_request(signed_facts, signature_bytes).expect("request artifact")
    }

    #[test]
    fn valid_request_verifies() {
        let request =
            verify_activation_request(&signed_request(), "chaptera.editor").expect("valid request");
        assert_eq!(request.request_id(), "req-1");
        assert_eq!(request.product_id(), "chaptera.editor");
        assert_eq!(request.requested_major(), 2);
        assert_eq!(request.request_nonce(), &[0x91; 32]);
    }

    #[test]
    fn wrong_product_is_rejected() {
        let error = verify_activation_request(&signed_request(), "chaptera.reader").unwrap_err();
        assert_eq!(error, ActivationRequestError::ProductMismatch);
    }

    #[test]
    fn tampered_signed_facts_fail_signature() {
        let artifact = signed_request();
        let mut envelope: ActivationRequestEnvelopeV1 = decode_exact(&artifact).expect("envelope");
        let mut facts: ActivationRequestFactsV1 =
            decode_exact(&envelope.signed_facts).expect("facts");
        facts.request_id = "req-tampered".into();
        envelope.signed_facts = encode_activation_request_facts(&facts).expect("facts");

        let mut tampered = Vec::new();
        ciborium::ser::into_writer(&envelope, &mut tampered).expect("envelope");
        let error = verify_activation_request(&tampered, "chaptera.editor").unwrap_err();
        assert_eq!(error, ActivationRequestError::SignatureInvalid);
    }

    #[test]
    fn tampered_product_fails_signature_before_product_policy() {
        let artifact = signed_request();
        let mut envelope: ActivationRequestEnvelopeV1 = decode_exact(&artifact).expect("envelope");
        let mut facts: ActivationRequestFactsV1 =
            decode_exact(&envelope.signed_facts).expect("facts");
        facts.product_id = "chaptera.reader".into();
        envelope.signed_facts = encode_activation_request_facts(&facts).expect("facts");

        let mut changed = Vec::new();
        ciborium::ser::into_writer(&envelope, &mut changed).expect("envelope");
        let error = verify_activation_request(&changed, "chaptera.editor").unwrap_err();
        assert_eq!(error, ActivationRequestError::SignatureInvalid);
    }

    #[test]
    fn mismatched_public_key_hash_is_rejected_before_signature() {
        let artifact = signed_request();
        let mut envelope: ActivationRequestEnvelopeV1 = decode_exact(&artifact).expect("envelope");
        let mut facts: ActivationRequestFactsV1 =
            decode_exact(&envelope.signed_facts).expect("facts");
        facts.device_key_id = [0xA5; DEVICE_KEY_ID_LEN];

        let mut raw_facts = Vec::new();
        ciborium::ser::into_writer(&facts, &mut raw_facts).expect("raw facts");
        envelope.signed_facts = raw_facts;
        let mut changed = Vec::new();
        ciborium::ser::into_writer(&envelope, &mut changed).expect("envelope");

        let error = verify_activation_request(&changed, "chaptera.editor").unwrap_err();
        assert_eq!(error, ActivationRequestError::DeviceKeyIdMismatch);
    }

    #[test]
    fn malformed_signature_length_fails_closed() {
        let artifact = signed_request();
        let mut envelope: ActivationRequestEnvelopeV1 = decode_exact(&artifact).expect("envelope");
        envelope.signature.pop();
        let mut changed = Vec::new();
        ciborium::ser::into_writer(&envelope, &mut changed).expect("envelope");
        let error = verify_activation_request(&changed, "chaptera.editor").unwrap_err();
        assert_eq!(error, ActivationRequestError::SignatureInvalid);
    }
}
