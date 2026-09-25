//! Public-safe canonical AuthoringRevisionId V1 derivation.
//!
//! Provenance: exact identity/serialization law ported from
//! HeisLuka/pub-rs@067d3bf5c0698a5bb72f03a36d33ae13ec2a8018
//! crates/pub-model/src/revision.rs + snapshot.rs.
//!
//! This module intentionally ports only the identity codec/hash primitive.
//! It does not import frozen SourceGraph/CDM types, SemanticDiff, mutation
//! execution, or any Cloud persistence authority.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fmt, str::FromStr};

pub const AUTHORING_REVISION_SCHEMA_V1: &str = "chaptera.cdm.authoring-revision.v1";
const REVISION_DOMAIN_SEPARATOR_V1: &[u8] = b"chaptera-cdm-authoring-revision-v1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AuthoringRevisionIdV1([u8; 32]);

impl AuthoringRevisionIdV1 {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub const fn into_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Display for AuthoringRevisionIdV1 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl Serialize for AuthoringRevisionIdV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for AuthoringRevisionIdV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(D::Error::custom)
    }
}

impl FromStr for AuthoringRevisionIdV1 {
    type Err = AuthoringRevisionIdParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.len() != 64 {
            return Err(AuthoringRevisionIdParseError::InvalidLength);
        }

        let mut bytes = [0_u8; 32];
        for (index, chunk) in input.as_bytes().chunks_exact(2).enumerate() {
            let source = index * 2;
            let high = lowercase_hex_value(chunk[0])
                .ok_or(AuthoringRevisionIdParseError::InvalidHex { index: source })?;
            let low = lowercase_hex_value(chunk[1])
                .ok_or(AuthoringRevisionIdParseError::InvalidHex { index: source + 1 })?;
            bytes[index] = (high << 4) | low;
        }
        Ok(Self(bytes))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthoringRevisionIdParseError {
    InvalidLength,
    InvalidHex { index: usize },
}

impl fmt::Display for AuthoringRevisionIdParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength => formatter.write_str(
                "AuthoringRevisionIdV1 must contain exactly 64 lowercase hex characters",
            ),
            Self::InvalidHex { index } => write!(
                formatter,
                "AuthoringRevisionIdV1 contains invalid lowercase hex at position {index}"
            ),
        }
    }
}

impl std::error::Error for AuthoringRevisionIdParseError {}

#[derive(Debug)]
pub enum AuthoringRevisionIdentityError {
    Canonicalization(serde_json::Error),
}

impl fmt::Display for AuthoringRevisionIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Canonicalization(error) => {
                write!(
                    formatter,
                    "canonical revision serialization failed: {error}"
                )
            }
        }
    }
}

impl std::error::Error for AuthoringRevisionIdentityError {}

#[derive(Serialize)]
struct RevisionIdentityEnvelope<'a, G: ?Sized> {
    schema_version: &'static str,
    parent_revision_id: Option<AuthoringRevisionIdV1>,
    graph: &'a G,
}

/// Serialize with the exact deterministic JSON profile used by the canonical donor:
/// recursively sorted object keys, semantic array order, no Unicode normalization.
pub fn canonical_revision_json_v1<T: Serialize + ?Sized>(
    value: &T,
) -> Result<Vec<u8>, AuthoringRevisionIdentityError> {
    let value =
        serde_json::to_value(value).map_err(AuthoringRevisionIdentityError::Canonicalization)?;
    let mut out = Vec::new();
    write_canonical_value(&mut out, &value)
        .map_err(AuthoringRevisionIdentityError::Canonicalization)?;
    Ok(out)
}

/// Derive the canonical REVISION-MODEL-01 AuthoringRevisionId for an authoritative
/// serializable graph/projection.
///
/// The caller owns graph semantics. This function owns only the exact versioned
/// identity envelope and hash law.
pub fn derive_authoring_revision_id_v1<G: Serialize + ?Sized>(
    graph: &G,
    parent_revision_id: Option<AuthoringRevisionIdV1>,
) -> Result<AuthoringRevisionIdV1, AuthoringRevisionIdentityError> {
    let envelope = RevisionIdentityEnvelope {
        schema_version: AUTHORING_REVISION_SCHEMA_V1,
        parent_revision_id,
        graph,
    };
    let canonical = canonical_revision_json_v1(&envelope)?;

    let mut hasher = Sha256::new();
    hasher.update(REVISION_DOMAIN_SEPARATOR_V1);
    hasher.update(canonical);
    let digest: [u8; 32] = hasher.finalize().into();
    Ok(AuthoringRevisionIdV1::from_bytes(digest))
}

fn write_canonical_value(out: &mut Vec<u8>, value: &Value) -> Result<(), serde_json::Error> {
    match value {
        Value::Object(map) => {
            out.push(b'{');
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index != 0 {
                    out.push(b',');
                }
                serde_json::to_writer(&mut *out, key)?;
                out.push(b':');
                write_canonical_value(out, &map[key])?;
            }
            out.push(b'}');
        }
        Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    out.push(b',');
                }
                write_canonical_value(out, item)?;
            }
            out.push(b']');
        }
        scalar => serde_json::to_writer(out, scalar)?,
    }
    Ok(())
}

const fn lowercase_hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn frozen_donor_baseline_golden_vector_matches() {
        let graph = json!({
            "z": 1,
            "a": [3, 1, 2],
            "unicode": "é",
        });

        let revision = derive_authoring_revision_id_v1(&graph, None).unwrap();
        assert_eq!(
            revision.to_string(),
            "d0297230f2ca6d0f09fed2b5ba59a2aedc32c408c08d037fc409077b0533a610"
        );
    }

    #[test]
    fn frozen_donor_parent_sensitive_golden_vector_matches() {
        let graph = json!({
            "z": 1,
            "a": [3, 1, 2],
            "unicode": "é",
        });
        let parent = "1111111111111111111111111111111111111111111111111111111111111111"
            .parse()
            .unwrap();

        let revision = derive_authoring_revision_id_v1(&graph, Some(parent)).unwrap();
        assert_eq!(
            revision.to_string(),
            "bc330090a28de082fef1b71511805b081690c310b87f9ab2dcb797d793fb61c3"
        );
    }

    #[derive(Serialize)]
    struct Fixture {
        semantic_order: Vec<u32>,
        unordered: HashMap<String, u32>,
    }

    #[test]
    fn canonical_json_sorts_object_keys_but_preserves_array_order() {
        let mut left = HashMap::new();
        left.insert("z".into(), 1);
        left.insert("a".into(), 2);

        let mut right = HashMap::new();
        right.insert("a".into(), 2);
        right.insert("z".into(), 1);

        let a = canonical_revision_json_v1(&Fixture {
            semantic_order: vec![3, 1, 2],
            unordered: left,
        })
        .unwrap();
        let b = canonical_revision_json_v1(&Fixture {
            semantic_order: vec![3, 1, 2],
            unordered: right,
        })
        .unwrap();

        assert_eq!(a, b);
        assert_eq!(
            String::from_utf8(a).unwrap(),
            r#"{"semantic_order":[3,1,2],"unordered":{"a":2,"z":1}}"#
        );
    }

    #[test]
    fn unicode_scalars_are_not_normalized() {
        let composed = derive_authoring_revision_id_v1(&"é", None).unwrap();
        let decomposed = derive_authoring_revision_id_v1(&"e\u{301}", None).unwrap();
        assert_ne!(composed, decomposed);
    }

    #[test]
    fn revision_id_representation_is_strict_lowercase_hex() {
        let revision = derive_authoring_revision_id_v1(&json!({"x": 1}), None).unwrap();
        let encoded = serde_json::to_string(&revision).unwrap();
        let decoded: AuthoringRevisionIdV1 = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, revision);

        assert!(
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
                .parse::<AuthoringRevisionIdV1>()
                .is_err()
        );
    }

    #[test]
    fn deterministic_replay_is_stable() {
        let graph = json!({"nested": {"b": 2, "a": 1}, "items": [1, 2, 3]});
        let first = derive_authoring_revision_id_v1(&graph, None).unwrap();
        let second = derive_authoring_revision_id_v1(&graph, None).unwrap();
        assert_eq!(first, second);
    }
}
