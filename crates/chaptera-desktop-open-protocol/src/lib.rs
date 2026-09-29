#![forbid(unsafe_code)]

//! Source-path-free IPC contract for Chaptera's future Windows desktop PUB-open worker.
//!
//! This crate owns only framing and transport identity. It deliberately does not
//! launch a process, establish an AppContainer/Job Object, parse PUB, or assign
//! document fidelity. The parent and worker exchange exact bytes plus explicit
//! hashes/lengths; the selected source pathname is never part of the worker
//! protocol.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const DESKTOP_OPEN_REQUEST_SCHEMA_V1: &str = "chaptera.desktop-open-request.v1";
pub const DESKTOP_OPEN_RESPONSE_SCHEMA_V1: &str = "chaptera.desktop-open-response.v1";
pub const WIRE_MAGIC_V1: [u8; 4] = *b"CHOP";
pub const WIRE_VERSION_V1: u8 = 1;
pub const CONTROL_JSON_MAX_BYTES_V1: usize = 1024 * 1024;

const FRAME_HEADER_BYTES: usize = 4 + 1 + 1 + 2 + 8 + 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameLimitsV1 {
    pub max_key_bytes: u16,
    pub max_frame_bytes: u64,
    pub max_total_bytes: u64,
    pub max_frames: u32,
}

impl FrameLimitsV1 {
    pub fn validate(self) -> Result<Self, ProtocolErrorV1> {
        if self.max_key_bytes == 0
            || self.max_frame_bytes == 0
            || self.max_total_bytes == 0
            || self.max_frames == 0
        {
            return Err(ProtocolErrorV1::InvalidLimits);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameKindV1 {
    ControlJson,
    SourcePub,
    ViewerJson,
    EditorGraphJson,
    ImageBlob,
}

impl FrameKindV1 {
    fn wire_code(self) -> u8 {
        match self {
            Self::ControlJson => 1,
            Self::SourcePub => 2,
            Self::ViewerJson => 3,
            Self::EditorGraphJson => 4,
            Self::ImageBlob => 5,
        }
    }

    fn from_wire_code(code: u8) -> Result<Self, ProtocolErrorV1> {
        match code {
            1 => Ok(Self::ControlJson),
            2 => Ok(Self::SourcePub),
            3 => Ok(Self::ViewerJson),
            4 => Ok(Self::EditorGraphJson),
            5 => Ok(Self::ImageBlob),
            _ => Err(ProtocolErrorV1::UnknownFrameKind(code)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameV1 {
    pub kind: FrameKindV1,
    pub key: String,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceIdentityV1 {
    pub sha256_hex: String,
    pub byte_len: u64,
}

impl SourceIdentityV1 {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            sha256_hex: sha256_hex(bytes),
            byte_len: u64::try_from(bytes.len()).expect("source byte length must fit u64"),
        }
    }

    pub fn validate(&self) -> Result<(), ProtocolErrorV1> {
        validate_sha256_hex(&self.sha256_hex)
    }

    pub fn matches_bytes(&self, bytes: &[u8]) -> bool {
        self.byte_len == u64::try_from(bytes.len()).unwrap_or(u64::MAX)
            && self.sha256_hex == sha256_hex(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadDescriptorV1 {
    pub kind: FrameKindV1,
    pub key: String,
    pub byte_len: u64,
    pub sha256_hex: String,
}

impl PayloadDescriptorV1 {
    pub fn from_frame(frame: &FrameV1) -> Self {
        Self {
            kind: frame.kind,
            key: frame.key.clone(),
            byte_len: u64::try_from(frame.payload.len()).expect("frame byte length must fit u64"),
            sha256_hex: sha256_hex(&frame.payload),
        }
    }

    pub fn validate(&self) -> Result<(), ProtocolErrorV1> {
        validate_key(&self.key)?;
        validate_sha256_hex(&self.sha256_hex)
    }

    pub fn matches_frame(&self, frame: &FrameV1) -> bool {
        self.kind == frame.kind
            && self.key == frame.key
            && self.byte_len == u64::try_from(frame.payload.len()).unwrap_or(u64::MAX)
            && self.sha256_hex == sha256_hex(&frame.payload)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopOpenRequestV1 {
    pub schema_version: String,
    pub security_profile: String,
    pub source: SourceIdentityV1,
    pub source_frame_key: String,
}

impl DesktopOpenRequestV1 {
    pub fn validate(&self) -> Result<(), ProtocolErrorV1> {
        if self.schema_version != DESKTOP_OPEN_REQUEST_SCHEMA_V1 {
            return Err(ProtocolErrorV1::SchemaMismatch);
        }
        if self.security_profile.is_empty() {
            return Err(ProtocolErrorV1::InvalidSecurityProfile);
        }
        self.source.validate()?;
        validate_key(&self.source_frame_key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopOpenSuccessV1 {
    pub viewer_json: PayloadDescriptorV1,
    pub editor_graph_json: PayloadDescriptorV1,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_blobs: Vec<PayloadDescriptorV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerFailureCodeV1 {
    RejectedByInputPolicy,
    ViewerOpenFailed,
    EditorOpenFailed,
    ResourceLimit,
    InternalFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesktopOpenOutcomeV1 {
    Success { payloads: DesktopOpenSuccessV1 },
    Failure { code: WorkerFailureCodeV1 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopOpenResponseV1 {
    pub schema_version: String,
    pub source: SourceIdentityV1,
    pub outcome: DesktopOpenOutcomeV1,
}

impl DesktopOpenResponseV1 {
    pub fn validate(&self) -> Result<(), ProtocolErrorV1> {
        if self.schema_version != DESKTOP_OPEN_RESPONSE_SCHEMA_V1 {
            return Err(ProtocolErrorV1::SchemaMismatch);
        }
        self.source.validate()?;
        if let DesktopOpenOutcomeV1::Success { payloads } = &self.outcome {
            payloads.viewer_json.validate()?;
            payloads.editor_graph_json.validate()?;
            for image in &payloads.image_blobs {
                image.validate()?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolErrorV1 {
    InvalidLimits,
    FrameCountLimit,
    FrameSizeLimit,
    TotalSizeLimit,
    KeyTooLong,
    EmptyKey,
    InvalidKeyUtf8,
    InvalidSha256,
    InvalidMagic,
    UnsupportedWireVersion(u8),
    UnknownFrameKind(u8),
    TruncatedFrame,
    FrameHashMismatch,
    SchemaMismatch,
    InvalidSecurityProfile,
    ControlJsonTooLarge,
    ControlJsonInvalid,
    UnexpectedFrame,
    MissingFrame,
    DuplicateFrame,
    SourceIdentityMismatch,
    ImageBlobLimit,
}

impl fmt::Display for ProtocolErrorV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ProtocolErrorV1 {}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn validate_sha256_hex(value: &str) -> Result<(), ProtocolErrorV1> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ProtocolErrorV1::InvalidSha256);
    }
    Ok(())
}

fn validate_key(value: &str) -> Result<(), ProtocolErrorV1> {
    if value.is_empty() {
        return Err(ProtocolErrorV1::EmptyKey);
    }
    Ok(())
}

pub fn encode_control_json<T: Serialize>(value: &T) -> Result<Vec<u8>, ProtocolErrorV1> {
    let bytes = serde_json::to_vec(value).map_err(|_| ProtocolErrorV1::ControlJsonInvalid)?;
    if bytes.len() > CONTROL_JSON_MAX_BYTES_V1 {
        return Err(ProtocolErrorV1::ControlJsonTooLarge);
    }
    Ok(bytes)
}

pub fn decode_control_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ProtocolErrorV1> {
    if bytes.len() > CONTROL_JSON_MAX_BYTES_V1 {
        return Err(ProtocolErrorV1::ControlJsonTooLarge);
    }
    serde_json::from_slice(bytes).map_err(|_| ProtocolErrorV1::ControlJsonInvalid)
}

pub fn encode_frames(
    frames: &[FrameV1],
    limits: FrameLimitsV1,
) -> Result<Vec<u8>, ProtocolErrorV1> {
    let limits = limits.validate()?;
    if u32::try_from(frames.len()).unwrap_or(u32::MAX) > limits.max_frames {
        return Err(ProtocolErrorV1::FrameCountLimit);
    }

    let mut total = 0_u64;
    for frame in frames {
        let key_len = frame.key.as_bytes().len();
        if key_len == 0 {
            return Err(ProtocolErrorV1::EmptyKey);
        }
        if key_len > usize::from(limits.max_key_bytes) || key_len > usize::from(u16::MAX) {
            return Err(ProtocolErrorV1::KeyTooLong);
        }
        let payload_len =
            u64::try_from(frame.payload.len()).map_err(|_| ProtocolErrorV1::FrameSizeLimit)?;
        if payload_len > limits.max_frame_bytes {
            return Err(ProtocolErrorV1::FrameSizeLimit);
        }
        let wire_len = u64::try_from(FRAME_HEADER_BYTES)
            .ok()
            .and_then(|value| value.checked_add(u64::try_from(key_len).ok()?))
            .and_then(|value| value.checked_add(payload_len))
            .ok_or(ProtocolErrorV1::TotalSizeLimit)?;
        total = total
            .checked_add(wire_len)
            .ok_or(ProtocolErrorV1::TotalSizeLimit)?;
        if total > limits.max_total_bytes {
            return Err(ProtocolErrorV1::TotalSizeLimit);
        }
    }

    let capacity = usize::try_from(total).map_err(|_| ProtocolErrorV1::TotalSizeLimit)?;
    let mut encoded = Vec::with_capacity(capacity);
    for frame in frames {
        let key = frame.key.as_bytes();
        let payload_len = u64::try_from(frame.payload.len()).expect("validated frame length");
        let digest = Sha256::digest(&frame.payload);

        encoded.extend_from_slice(&WIRE_MAGIC_V1);
        encoded.push(WIRE_VERSION_V1);
        encoded.push(frame.kind.wire_code());
        encoded.extend_from_slice(
            &u16::try_from(key.len())
                .expect("validated frame key length")
                .to_le_bytes(),
        );
        encoded.extend_from_slice(&payload_len.to_le_bytes());
        encoded.extend_from_slice(&digest);
        encoded.extend_from_slice(key);
        encoded.extend_from_slice(&frame.payload);
    }
    Ok(encoded)
}

pub fn decode_frames(bytes: &[u8], limits: FrameLimitsV1) -> Result<Vec<FrameV1>, ProtocolErrorV1> {
    let limits = limits.validate()?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limits.max_total_bytes {
        return Err(ProtocolErrorV1::TotalSizeLimit);
    }

    let mut offset = 0_usize;
    let mut frames = Vec::new();
    while offset < bytes.len() {
        if frames.len() >= usize::try_from(limits.max_frames).unwrap_or(usize::MAX) {
            return Err(ProtocolErrorV1::FrameCountLimit);
        }
        let fixed_end = offset
            .checked_add(FRAME_HEADER_BYTES)
            .ok_or(ProtocolErrorV1::TruncatedFrame)?;
        let header = bytes
            .get(offset..fixed_end)
            .ok_or(ProtocolErrorV1::TruncatedFrame)?;

        if header[0..4] != WIRE_MAGIC_V1 {
            return Err(ProtocolErrorV1::InvalidMagic);
        }
        let version = header[4];
        if version != WIRE_VERSION_V1 {
            return Err(ProtocolErrorV1::UnsupportedWireVersion(version));
        }
        let kind = FrameKindV1::from_wire_code(header[5])?;
        let key_len = usize::from(u16::from_le_bytes([header[6], header[7]]));
        if key_len == 0 {
            return Err(ProtocolErrorV1::EmptyKey);
        }
        if key_len > usize::from(limits.max_key_bytes) {
            return Err(ProtocolErrorV1::KeyTooLong);
        }

        let mut payload_len_bytes = [0_u8; 8];
        payload_len_bytes.copy_from_slice(&header[8..16]);
        let payload_len = u64::from_le_bytes(payload_len_bytes);
        if payload_len > limits.max_frame_bytes {
            return Err(ProtocolErrorV1::FrameSizeLimit);
        }
        let payload_len_usize =
            usize::try_from(payload_len).map_err(|_| ProtocolErrorV1::FrameSizeLimit)?;

        let key_start = fixed_end;
        let key_end = key_start
            .checked_add(key_len)
            .ok_or(ProtocolErrorV1::TruncatedFrame)?;
        let payload_end = key_end
            .checked_add(payload_len_usize)
            .ok_or(ProtocolErrorV1::TruncatedFrame)?;
        let key_bytes = bytes
            .get(key_start..key_end)
            .ok_or(ProtocolErrorV1::TruncatedFrame)?;
        let payload = bytes
            .get(key_end..payload_end)
            .ok_or(ProtocolErrorV1::TruncatedFrame)?;

        let key = std::str::from_utf8(key_bytes)
            .map_err(|_| ProtocolErrorV1::InvalidKeyUtf8)?
            .to_owned();
        validate_key(&key)?;

        let actual_digest = Sha256::digest(payload);
        if actual_digest[..] != header[16..48] {
            return Err(ProtocolErrorV1::FrameHashMismatch);
        }

        frames.push(FrameV1 {
            kind,
            key,
            payload: payload.to_vec(),
        });
        offset = payload_end;
    }
    Ok(frames)
}

pub fn validate_request_frames(
    request: &DesktopOpenRequestV1,
    frames: &[FrameV1],
) -> Result<(), ProtocolErrorV1> {
    request.validate()?;
    if frames.len() != 1 {
        return Err(ProtocolErrorV1::UnexpectedFrame);
    }
    let frame = &frames[0];
    if frame.kind != FrameKindV1::SourcePub || frame.key != request.source_frame_key {
        return Err(ProtocolErrorV1::UnexpectedFrame);
    }
    if !request.source.matches_bytes(&frame.payload) {
        return Err(ProtocolErrorV1::SourceIdentityMismatch);
    }
    Ok(())
}

pub fn validate_response_source(
    response: &DesktopOpenResponseV1,
    expected_source: &SourceIdentityV1,
) -> Result<(), ProtocolErrorV1> {
    response.validate()?;
    expected_source.validate()?;
    if &response.source != expected_source {
        return Err(ProtocolErrorV1::SourceIdentityMismatch);
    }
    Ok(())
}

pub fn validate_success_frames(
    success: &DesktopOpenSuccessV1,
    frames: &[FrameV1],
    max_image_blobs: u32,
) -> Result<(), ProtocolErrorV1> {
    if u32::try_from(success.image_blobs.len()).unwrap_or(u32::MAX) > max_image_blobs {
        return Err(ProtocolErrorV1::ImageBlobLimit);
    }

    if success.viewer_json.kind != FrameKindV1::ViewerJson
        || success.editor_graph_json.kind != FrameKindV1::EditorGraphJson
        || success
            .image_blobs
            .iter()
            .any(|descriptor| descriptor.kind != FrameKindV1::ImageBlob)
    {
        return Err(ProtocolErrorV1::UnexpectedFrame);
    }

    let mut descriptors = Vec::with_capacity(2 + success.image_blobs.len());
    descriptors.push(&success.viewer_json);
    descriptors.push(&success.editor_graph_json);
    descriptors.extend(success.image_blobs.iter());

    let mut descriptor_keys = BTreeSet::new();
    for descriptor in &descriptors {
        descriptor.validate()?;
        let identity = (descriptor.kind, descriptor.key.as_str());
        if !descriptor_keys.insert(identity) {
            return Err(ProtocolErrorV1::DuplicateFrame);
        }
    }

    let mut frames_by_identity = BTreeMap::new();
    for frame in frames {
        let identity = (frame.kind, frame.key.as_str());
        if frames_by_identity.insert(identity, frame).is_some() {
            return Err(ProtocolErrorV1::DuplicateFrame);
        }
    }

    if frames_by_identity.len() != descriptors.len() {
        return Err(ProtocolErrorV1::UnexpectedFrame);
    }

    for descriptor in descriptors {
        let identity = (descriptor.kind, descriptor.key.as_str());
        let frame = frames_by_identity
            .get(&identity)
            .ok_or(ProtocolErrorV1::MissingFrame)?;
        if !descriptor.matches_frame(frame) {
            return Err(ProtocolErrorV1::FrameHashMismatch);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> FrameLimitsV1 {
        FrameLimitsV1 {
            max_key_bytes: 128,
            max_frame_bytes: 1024 * 1024,
            max_total_bytes: 4 * 1024 * 1024,
            max_frames: 16,
        }
    }

    #[test]
    fn request_schema_has_no_source_path_authority() {
        let source = SourceIdentityV1::from_bytes(b"pub bytes");
        let request = DesktopOpenRequestV1 {
            schema_version: DESKTOP_OPEN_REQUEST_SCHEMA_V1.to_owned(),
            security_profile: "chaptera-untrusted-pub-v1".to_owned(),
            source,
            source_frame_key: "source".to_owned(),
        };
        let json = String::from_utf8(encode_control_json(&request).expect("request json"))
            .expect("utf8 json");
        assert!(!json.contains("source_path"));
        assert!(!json.contains("filesystem_path"));
        assert!(!json.contains("C:\\"));
        assert!(request.validate().is_ok());
    }

    #[test]
    fn source_frame_roundtrips_with_exact_identity() {
        let source_bytes = b"exact pub source".to_vec();
        let request = DesktopOpenRequestV1 {
            schema_version: DESKTOP_OPEN_REQUEST_SCHEMA_V1.to_owned(),
            security_profile: "chaptera-untrusted-pub-v1".to_owned(),
            source: SourceIdentityV1::from_bytes(&source_bytes),
            source_frame_key: "source".to_owned(),
        };
        let frames = vec![FrameV1 {
            kind: FrameKindV1::SourcePub,
            key: "source".to_owned(),
            payload: source_bytes,
        }];
        let wire = encode_frames(&frames, limits()).expect("encode source frame");
        let decoded = decode_frames(&wire, limits()).expect("decode source frame");
        assert_eq!(decoded, frames);
        validate_request_frames(&request, &decoded).expect("source identity");
    }

    #[test]
    fn decoder_rejects_oversized_declared_payload_before_copy() {
        let frame = FrameV1 {
            kind: FrameKindV1::SourcePub,
            key: "source".to_owned(),
            payload: b"x".to_vec(),
        };
        let mut wire = encode_frames(&[frame], limits()).expect("wire");
        wire[8..16].copy_from_slice(&(2_u64 * 1024 * 1024).to_le_bytes());
        assert_eq!(
            decode_frames(&wire, limits()),
            Err(ProtocolErrorV1::FrameSizeLimit)
        );
    }

    #[test]
    fn payload_tampering_fails_hash_validation() {
        let frame = FrameV1 {
            kind: FrameKindV1::ViewerJson,
            key: "viewer".to_owned(),
            payload: br#"{"viewer":true}"#.to_vec(),
        };
        let mut wire = encode_frames(&[frame], limits()).expect("wire");
        let last = wire.last_mut().expect("payload byte");
        *last ^= 0x01;
        assert_eq!(
            decode_frames(&wire, limits()),
            Err(ProtocolErrorV1::FrameHashMismatch)
        );
    }

    #[test]
    fn success_requires_exact_viewer_graph_and_image_blob_set() {
        let viewer = FrameV1 {
            kind: FrameKindV1::ViewerJson,
            key: "viewer".to_owned(),
            payload: br#"{"schema":"viewer"}"#.to_vec(),
        };
        let graph = FrameV1 {
            kind: FrameKindV1::EditorGraphJson,
            key: "editor_graph".to_owned(),
            payload: br#"{"schema":"graph"}"#.to_vec(),
        };
        let image = FrameV1 {
            kind: FrameKindV1::ImageBlob,
            key: "resource:17".to_owned(),
            payload: vec![0x89, b'P', b'N', b'G'],
        };
        let success = DesktopOpenSuccessV1 {
            viewer_json: PayloadDescriptorV1::from_frame(&viewer),
            editor_graph_json: PayloadDescriptorV1::from_frame(&graph),
            image_blobs: vec![PayloadDescriptorV1::from_frame(&image)],
        };

        validate_success_frames(&success, &[viewer.clone(), graph.clone(), image.clone()], 8)
            .expect("exact success frame set");

        let mut changed_image = image;
        changed_image.payload.push(0);
        assert_eq!(
            validate_success_frames(&success, &[viewer, graph, changed_image], 8),
            Err(ProtocolErrorV1::FrameHashMismatch)
        );
    }

    #[test]
    fn response_source_identity_cannot_drift_from_parent_observation() {
        let expected = SourceIdentityV1::from_bytes(b"source A");
        let response = DesktopOpenResponseV1 {
            schema_version: DESKTOP_OPEN_RESPONSE_SCHEMA_V1.to_owned(),
            source: SourceIdentityV1::from_bytes(b"source B"),
            outcome: DesktopOpenOutcomeV1::Failure {
                code: WorkerFailureCodeV1::ViewerOpenFailed,
            },
        };
        assert_eq!(
            validate_response_source(&response, &expected),
            Err(ProtocolErrorV1::SourceIdentityMismatch)
        );
    }

    #[test]
    fn response_failure_is_source_free_typed_state() {
        let response = DesktopOpenResponseV1 {
            schema_version: DESKTOP_OPEN_RESPONSE_SCHEMA_V1.to_owned(),
            source: SourceIdentityV1::from_bytes(b"source"),
            outcome: DesktopOpenOutcomeV1::Failure {
                code: WorkerFailureCodeV1::RejectedByInputPolicy,
            },
        };
        let json = String::from_utf8(encode_control_json(&response).expect("response json"))
            .expect("utf8 json");
        assert!(json.contains("rejected_by_input_policy"));
        assert!(!json.contains("error_message"));
        assert!(!json.contains("document_text"));
    }

    #[test]
    fn control_json_denies_unknown_fields() {
        let invalid = br#"{
          "schema_version":"chaptera.desktop-open-request.v1",
          "security_profile":"chaptera-untrusted-pub-v1",
          "source":{"sha256_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","byte_len":1},
          "source_frame_key":"source",
          "source_path":"C:\\secret.pub"
        }"#;
        assert_eq!(
            decode_control_json::<DesktopOpenRequestV1>(invalid),
            Err(ProtocolErrorV1::ControlJsonInvalid)
        );
    }

    #[test]
    fn aggregate_frame_budget_is_enforced() {
        let frames = vec![
            FrameV1 {
                kind: FrameKindV1::ViewerJson,
                key: "viewer".to_owned(),
                payload: vec![0; 128],
            },
            FrameV1 {
                kind: FrameKindV1::EditorGraphJson,
                key: "graph".to_owned(),
                payload: vec![0; 128],
            },
        ];
        let tight = FrameLimitsV1 {
            max_key_bytes: 128,
            max_frame_bytes: 1024,
            max_total_bytes: 100,
            max_frames: 8,
        };
        assert_eq!(
            encode_frames(&frames, tight),
            Err(ProtocolErrorV1::TotalSizeLimit)
        );
    }
}
