use crate::CanonicalId;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

/// SHA-256 digest как фиксированное 32-байтное значение.
///
/// Тип не вычисляет хэш. Он только фиксирует representation, требуемый CDM:
/// 64 lowercase hex-символа в сериализованном виде.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
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

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sha256DigestParseError {
    InvalidLength,
    InvalidHex { index: usize },
}

impl fmt::Display for Sha256DigestParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength => formatter.write_str("SHA-256 должен содержать 64 hex-символа"),
            Self::InvalidHex { index } => {
                write!(
                    formatter,
                    "в SHA-256 невалидная lowercase hex-цифра по позиции {index}"
                )
            }
        }
    }
}

impl std::error::Error for Sha256DigestParseError {}

impl FromStr for Sha256Digest {
    type Err = Sha256DigestParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.len() != 64 {
            return Err(Sha256DigestParseError::InvalidLength);
        }

        let input = input.as_bytes();
        let mut output = [0_u8; 32];

        for (output_index, chunk) in input.chunks_exact(2).enumerate() {
            let source_index = output_index * 2;
            let high = lowercase_hex_value(chunk[0]).ok_or(Sha256DigestParseError::InvalidHex {
                index: source_index,
            })?;
            let low = lowercase_hex_value(chunk[1]).ok_or(Sha256DigestParseError::InvalidHex {
                index: source_index + 1,
            })?;
            output[output_index] = (high << 4) | low;
        }

        Ok(Self(output))
    }
}

impl Serialize for Sha256Digest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(D::Error::custom)
    }
}

const fn lowercase_hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// Диапазон байтов внутри одного source carrier.
///
/// Метод end использует checked arithmetic: переполнение source range является
/// невалидным provenance, а не wrap-around адресом.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteRange {
    pub offset: u64,
    pub length: u64,
}

impl ByteRange {
    pub const fn new(offset: u64, length: u64) -> Self {
        Self { offset, length }
    }

    pub const fn end(self) -> Option<u64> {
        self.offset.checked_add(self.length)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceDescriptor {
    pub format: String,
    pub format_version: Option<String>,
    pub adapter_version: String,
    pub source_hash: Sha256Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceRole {
    Semantic,
    Relation,
    Projection,
    Cache,
    ServiceMetadata,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityClass {
    Authoritative,
    Derived,
    Cache,
    ServiceMetadata,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadConfidence {
    Exact,
    Structural,
    Inferred,
    Approximate,
    Opaque,
}

/// Минимальная единица provenance.
///
/// Raw bytes здесь не дублируются: ref указывает на carrier/object/path/range.
/// Source-specific object keys остаются строкой и не превращаются автоматически
/// в CanonicalId.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    pub format: String,
    pub adapter_version: String,
    pub source_hash: Sha256Digest,
    pub carrier: String,
    pub object_key: Option<String>,
    pub path: Option<String>,
    pub byte_range: Option<ByteRange>,
    pub role: SourceRole,
    pub authority: AuthorityClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ReadConfidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceIdentityField {
    Format,
    AdapterVersion,
    SourceHash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRefValidationError {
    PrimarySourceMismatch { field: SourceIdentityField },
    ByteRangeOverflow { range: ByteRange },
}

impl SourceRef {
    /// Проверяет ref относительно primary SourceDescriptor.
    ///
    /// Secondary-source refs должны проверяться против собственного descriptor,
    /// поэтому метод намеренно называется validate_primary_source.
    pub fn validate_primary_source(
        &self,
        source: &SourceDescriptor,
    ) -> Result<(), SourceRefValidationError> {
        if self.format != source.format {
            return Err(SourceRefValidationError::PrimarySourceMismatch {
                field: SourceIdentityField::Format,
            });
        }
        if self.adapter_version != source.adapter_version {
            return Err(SourceRefValidationError::PrimarySourceMismatch {
                field: SourceIdentityField::AdapterVersion,
            });
        }
        if self.source_hash != source.source_hash {
            return Err(SourceRefValidationError::PrimarySourceMismatch {
                field: SourceIdentityField::SourceHash,
            });
        }

        if let Some(range) = self.byte_range {
            if range.end().is_none() {
                return Err(SourceRefValidationError::ByteRangeOverflow { range });
            }
        }

        Ok(())
    }
}

/// Opaque handle на format-private state.
///
/// private_state_ref принадлежит adapter/storage layer. CDM не интерпретирует
/// его как путь, URL или blob ID и не переносит через layout boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceCapsuleRef {
    pub format: String,
    pub adapter_version: String,
    pub source_hash: Sha256Digest,
    pub private_state_ref: String,
}

impl SourceCapsuleRef {
    pub fn validate_primary_source(
        &self,
        source: &SourceDescriptor,
    ) -> Result<(), SourceIdentityField> {
        if self.format != source.format {
            return Err(SourceIdentityField::Format);
        }
        if self.adapter_version != source.adapter_version {
            return Err(SourceIdentityField::AdapterVersion);
        }
        if self.source_hash != source.source_hash {
            return Err(SourceIdentityField::SourceHash);
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreservationPolicy {
    MustPreserve,
    PreserveIfOwnerUnchanged,
    DropOnlyWithExplicitLoss,
    NonPortable,
}

/// Неизвестные/private данные, прикреплённые к canonical graph.
///
/// Concrete storage намеренно generic: formal CDM требует OpaqueStorage, но
/// его variants ещё не специфицированы. Adapter может подставить собственный
/// storage-ref type, не расширяя core выдуманным универсальным blob protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpaqueExtension<Storage> {
    pub id: CanonicalId,
    pub owner_id: Option<CanonicalId>,
    pub namespace: String,
    pub type_name: String,
    pub storage: Storage,
    pub ordering_key: Option<String>,
    pub preservation: PreservationPolicy,
    pub source_refs: Vec<SourceRef>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn source_hash() -> Sha256Digest {
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .parse()
            .expect("валидный lowercase SHA-256")
    }

    fn descriptor() -> SourceDescriptor {
        SourceDescriptor {
            format: "pub".into(),
            format_version: Some("0x2c".into()),
            adapter_version: "pub-rs/0.1".into(),
            source_hash: source_hash(),
        }
    }

    fn source_ref() -> SourceRef {
        SourceRef {
            format: "pub".into(),
            adapter_version: "pub-rs/0.1".into(),
            source_hash: source_hash(),
            carrier: "PUB/Contents".into(),
            object_key: Some("seq:330".into()),
            path: None,
            byte_range: Some(ByteRange::new(4096, 64)),
            role: SourceRole::Relation,
            authority: AuthorityClass::Authoritative,
            confidence: Some(ReadConfidence::Exact),
        }
    }

    #[test]
    fn sha256_serialization_follows_lowercase_machine_schema() {
        let digest = source_hash();
        assert_eq!(
            digest.to_string(),
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
        assert!(
            "0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef"
                .parse::<Sha256Digest>()
                .is_err()
        );
    }

    #[test]
    fn source_ref_validates_against_primary_source() {
        assert_eq!(source_ref().validate_primary_source(&descriptor()), Ok(()));
    }

    #[test]
    fn source_ref_rejects_overflowing_byte_range() {
        let mut reference = source_ref();
        reference.byte_range = Some(ByteRange::new(u64::MAX, 1));

        assert!(matches!(
            reference.validate_primary_source(&descriptor()),
            Err(SourceRefValidationError::ByteRangeOverflow { .. })
        ));
    }

    #[test]
    fn optional_confidence_is_omitted_in_json_when_absent() {
        let mut reference = source_ref();
        reference.confidence = None;

        let value = serde_json::to_value(reference).expect("SourceRef должен сериализоваться");
        assert!(value.get("confidence").is_none());
    }

    #[test]
    fn preservation_policy_uses_machine_schema_names() {
        assert_eq!(
            serde_json::to_value(PreservationPolicy::DropOnlyWithExplicitLoss)
                .expect("policy должна сериализоваться"),
            json!("drop_only_with_explicit_loss")
        );
    }

    #[test]
    fn source_capsule_ref_keeps_adapter_private_handle_opaque() {
        let capsule = SourceCapsuleRef {
            format: "pub".into(),
            adapter_version: "pub-rs/0.1".into(),
            source_hash: source_hash(),
            private_state_ref: "adapter-private:source-capsule-42".into(),
        };

        assert_eq!(capsule.validate_primary_source(&descriptor()), Ok(()));

        let value = serde_json::to_value(capsule).expect("SourceCapsuleRef должен сериализоваться");
        assert_eq!(
            value["private_state_ref"],
            json!("adapter-private:source-capsule-42")
        );
    }

    #[test]
    fn opaque_extension_storage_remains_adapter_defined() {
        let extension = OpaqueExtension {
            id: CanonicalId::from_bytes([1; 16]),
            owner_id: None,
            namespace: "pub/fopt".into(),
            type_name: "unknown-property".into(),
            storage: json!({
                "adapter_ref": "fopt:shape=17:property=0x1234"
            }),
            ordering_key: Some("0007".into()),
            preservation: PreservationPolicy::MustPreserve,
            source_refs: vec![source_ref()],
        };

        let value = serde_json::to_value(extension).expect("extension должен сериализоваться");
        assert_eq!(value["preservation"], json!("must_preserve"));
        assert_eq!(
            value["storage"]["adapter_ref"],
            json!("fopt:shape=17:property=0x1234")
        );
    }

    #[test]
    fn confidence_and_authority_remain_independent_axes() {
        let mut reference = source_ref();
        reference.confidence = Some(ReadConfidence::Inferred);
        reference.authority = AuthorityClass::Unknown;

        assert_eq!(reference.confidence, Some(ReadConfidence::Inferred));
        assert_eq!(reference.authority, AuthorityClass::Unknown);
    }
}
