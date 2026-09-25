use crate::{CanonicalId, Sha256Digest};
use uuid::Uuid;

/// Fixed namespace для contract `cdm-source-id-v1`.
///
/// Значение получено один раз как:
/// UUIDv5(NAMESPACE_URL, "https://newboo.local/id/cdm-source-derived/v1").
///
/// Реализации обязаны использовать эти 16 bytes как constant и не вычислять
/// namespace из runtime URL/configuration.
pub const SOURCE_DERIVED_NAMESPACE_V1: CanonicalId = CanonicalId::from_bytes([
    0xc0, 0x2c, 0xe2, 0x1c, 0xd0, 0x44, 0x56, 0xb2, 0x95, 0xd9, 0x25, 0xa9, 0x28, 0x9c, 0x1d, 0x1f,
]);

const SOURCE_DERIVED_FRAMING_VERSION_V1: u8 = 0x01;

#[derive(Debug, Clone, Copy)]
pub struct SourceDerivedIdInput<'a> {
    pub source_hash: &'a Sha256Digest,
    pub adapter_id: &'a str,
    pub source_object_key: &'a str,
    pub semantic_role: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceDerivedIdError {
    EmptyAdapterId,
    InvalidAdapterId { index: usize, byte: u8 },
    EmptySourceObjectKey,
    EmptySemanticRole,
    InvalidSemanticRole { index: usize, byte: u8 },
    ComponentTooLong { component: SourceIdComponent },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceIdComponent {
    AdapterId,
    SourceObjectKey,
    SemanticRole,
}

/// Детерминированно строит source-derived CanonicalId по contract
/// `cdm-source-id-v1`.
///
/// `adapter_id` и `semantic_role` являются stable lowercase ASCII IDs.
/// `source_object_key` — exact UTF-8 bytes уже canonicalized adapter'ом.
/// Core не выполняет case-folding или Unicode normalization.
pub fn derive_source_canonical_id(
    input: SourceDerivedIdInput<'_>,
) -> Result<CanonicalId, SourceDerivedIdError> {
    validate_stable_ascii_id(input.adapter_id, StableIdKind::Adapter)?;
    if input.source_object_key.is_empty() {
        return Err(SourceDerivedIdError::EmptySourceObjectKey);
    }
    validate_stable_ascii_id(input.semantic_role, StableIdKind::SemanticRole)?;

    let name = source_derived_name_bytes(input)?;
    let namespace = Uuid::from_bytes(SOURCE_DERIVED_NAMESPACE_V1.into_bytes());
    let uuid = Uuid::new_v5(&namespace, &name);

    Ok(CanonicalId::from_bytes(*uuid.as_bytes()))
}

/// Создаёт новый editor-created CanonicalId как RFC 9562 UUIDv7.
///
/// Source-derived entities никогда не должны использовать эту функцию.
pub fn new_editor_canonical_id() -> CanonicalId {
    CanonicalId::from_bytes(*Uuid::now_v7().as_bytes())
}

fn source_derived_name_bytes(
    input: SourceDerivedIdInput<'_>,
) -> Result<Vec<u8>, SourceDerivedIdError> {
    let adapter = component_bytes(input.adapter_id.as_bytes(), SourceIdComponent::AdapterId)?;
    let object_key = component_bytes(
        input.source_object_key.as_bytes(),
        SourceIdComponent::SourceObjectKey,
    )?;
    let role = component_bytes(
        input.semantic_role.as_bytes(),
        SourceIdComponent::SemanticRole,
    )?;

    let capacity = 1_usize
        .saturating_add(32)
        .saturating_add(4)
        .saturating_add(adapter.len())
        .saturating_add(4)
        .saturating_add(object_key.len())
        .saturating_add(4)
        .saturating_add(role.len());
    let mut name = Vec::with_capacity(capacity);

    name.push(SOURCE_DERIVED_FRAMING_VERSION_V1);
    name.extend_from_slice(input.source_hash.as_bytes());
    append_length_prefixed(&mut name, adapter);
    append_length_prefixed(&mut name, object_key);
    append_length_prefixed(&mut name, role);

    Ok(name)
}

fn component_bytes(
    bytes: &[u8],
    component: SourceIdComponent,
) -> Result<&[u8], SourceDerivedIdError> {
    if u32::try_from(bytes.len()).is_err() {
        return Err(SourceDerivedIdError::ComponentTooLong { component });
    }
    Ok(bytes)
}

fn append_length_prefixed(output: &mut Vec<u8>, bytes: &[u8]) {
    let len = u32::try_from(bytes.len())
        .expect("component_bytes уже проверил, что длина помещается в u32");
    output.extend_from_slice(&len.to_be_bytes());
    output.extend_from_slice(bytes);
}

#[derive(Debug, Clone, Copy)]
enum StableIdKind {
    Adapter,
    SemanticRole,
}

fn validate_stable_ascii_id(value: &str, kind: StableIdKind) -> Result<(), SourceDerivedIdError> {
    if value.is_empty() {
        return match kind {
            StableIdKind::Adapter => Err(SourceDerivedIdError::EmptyAdapterId),
            StableIdKind::SemanticRole => Err(SourceDerivedIdError::EmptySemanticRole),
        };
    }

    for (index, byte) in value.bytes().enumerate() {
        let first = index == 0;
        let valid = if first {
            byte.is_ascii_lowercase() || byte.is_ascii_digit()
        } else {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || match kind {
                    StableIdKind::Adapter => matches!(byte, b'.' | b'_' | b'-'),
                    StableIdKind::SemanticRole => matches!(byte, b'.' | b'_' | b'-' | b'/'),
                }
        };

        if !valid {
            return match kind {
                StableIdKind::Adapter => {
                    Err(SourceDerivedIdError::InvalidAdapterId { index, byte })
                }
                StableIdKind::SemanticRole => {
                    Err(SourceDerivedIdError::InvalidSemanticRole { index, byte })
                }
            };
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_hash() -> Sha256Digest {
        let mut bytes = [0_u8; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::try_from(index).expect("index 0..31 помещается в u8");
        }
        Sha256Digest::from_bytes(bytes)
    }

    fn input<'a>(
        hash: &'a Sha256Digest,
        adapter_id: &'a str,
        source_object_key: &'a str,
        semantic_role: &'a str,
    ) -> SourceDerivedIdInput<'a> {
        SourceDerivedIdInput {
            source_hash: hash,
            adapter_id,
            source_object_key,
            semantic_role,
        }
    }

    #[test]
    fn project_namespace_matches_design_contract() {
        let expected = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            b"https://newboo.local/id/cdm-source-derived/v1",
        );

        assert_eq!(SOURCE_DERIVED_NAMESPACE_V1.as_bytes(), expected.as_bytes());
        assert_eq!(
            SOURCE_DERIVED_NAMESPACE_V1.to_string(),
            "c02ce21c-d044-56b2-95d9-25a9289c1d1f"
        );
    }

    #[test]
    fn source_derived_golden_vector_matches_contract_v1() {
        let hash = source_hash();
        let id = derive_source_canonical_id(input(
            &hash,
            "pub-rs",
            "contents/0x2c/seq/330",
            "cdm.story",
        ))
        .expect("валидный source-derived input");

        assert_eq!(id.to_string(), "3c3cfd8a-2347-5cf4-977d-34e4f346f6df");
    }

    #[test]
    fn repeated_source_derivation_is_identical() {
        let hash = source_hash();
        let value = input(&hash, "pub-rs", "contents/0x2c/seq/330", "cdm.story");

        let first = derive_source_canonical_id(value).expect("валидный input");
        let second = derive_source_canonical_id(value).expect("валидный input");

        assert_eq!(first, second);
    }

    #[test]
    fn each_identity_component_changes_the_result() {
        let hash = source_hash();
        let baseline = derive_source_canonical_id(input(
            &hash,
            "pub-rs",
            "contents/0x2c/seq/330",
            "cdm.story",
        ))
        .expect("baseline");

        let mut other_hash_bytes = hash.into_bytes();
        other_hash_bytes[31] ^= 0xff;
        let other_hash = Sha256Digest::from_bytes(other_hash_bytes);

        let variants = [
            derive_source_canonical_id(input(
                &other_hash,
                "pub-rs",
                "contents/0x2c/seq/330",
                "cdm.story",
            ))
            .expect("другой hash"),
            derive_source_canonical_id(input(
                &hash,
                "other-adapter",
                "contents/0x2c/seq/330",
                "cdm.story",
            ))
            .expect("другой adapter"),
            derive_source_canonical_id(input(
                &hash,
                "pub-rs",
                "contents/0x2c/seq/331",
                "cdm.story",
            ))
            .expect("другой source key"),
            derive_source_canonical_id(input(
                &hash,
                "pub-rs",
                "contents/0x2c/seq/330",
                "cdm.text-frame",
            ))
            .expect("другая semantic role"),
        ];

        assert!(variants.into_iter().all(|value| value != baseline));
    }

    #[test]
    fn length_prefix_framing_avoids_delimiter_ambiguity() {
        let hash = source_hash();

        let first =
            derive_source_canonical_id(input(&hash, "a", "bc", "d")).expect("первый tuple валиден");
        let second =
            derive_source_canonical_id(input(&hash, "ab", "c", "d")).expect("второй tuple валиден");

        assert_ne!(first, second);
    }

    #[test]
    fn stable_ids_reject_obvious_version_path_and_uppercase_adapter_names() {
        let hash = source_hash();

        assert!(matches!(
            derive_source_canonical_id(input(
                &hash,
                "pub-rs/0.1",
                "contents/0x2c/seq/330",
                "cdm.story",
            )),
            Err(SourceDerivedIdError::InvalidAdapterId { .. })
        ));
        assert!(matches!(
            derive_source_canonical_id(input(
                &hash,
                "PUB-RS",
                "contents/0x2c/seq/330",
                "cdm.story",
            )),
            Err(SourceDerivedIdError::InvalidAdapterId { .. })
        ));
    }

    #[test]
    fn empty_source_object_key_is_rejected() {
        let hash = source_hash();

        assert_eq!(
            derive_source_canonical_id(input(&hash, "pub-rs", "", "cdm.story")),
            Err(SourceDerivedIdError::EmptySourceObjectKey)
        );
    }

    #[test]
    fn editor_created_id_is_standard_uuid_v7() {
        let id = new_editor_canonical_id();
        let bytes = id.as_bytes();

        assert_eq!(bytes[6] >> 4, 0x7);
        assert_eq!(bytes[8] >> 6, 0b10);
    }
}
