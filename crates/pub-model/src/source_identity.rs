use std::{error::Error, fmt};

use uuid::Uuid;

use crate::TableCellId;

pub const SOURCE_DERIVED_NAMESPACE_V1: [u8; 16] = [
    0xc0, 0x2c, 0xe2, 0x1c, 0xd0, 0x44, 0x56, 0xb2, 0x95, 0xd9, 0x25, 0xa9, 0x28, 0x9c, 0x1d, 0x1f,
];
pub const PUB_SOURCE_ADAPTER_ID_V1: &str = "pub-rs";

const SOURCE_DERIVED_FRAMING_VERSION_V1: u8 = 0x01;
const ROLE_PAGE: &str = "cdm.page";
const ROLE_NODE: &str = "cdm.node";
const ROLE_STORY: &str = "cdm.story";
const ROLE_TABLE_CELL: &str = "cdm.table_cell";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceIdentityError {
    InvalidSourceHash,
    EmptyAdapterId,
    InvalidAdapterId { index: usize, byte: u8 },
    EmptySourceObjectKey,
    EmptySemanticRole,
    InvalidSemanticRole { index: usize, byte: u8 },
    ComponentTooLong,
}

impl fmt::Display for SourceIdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSourceHash => {
                f.write_str("source hash must be 64 lowercase hexadecimal characters")
            }
            Self::EmptyAdapterId => f.write_str("adapter id must not be empty"),
            Self::InvalidAdapterId { index, byte } => {
                write!(f, "invalid adapter id byte 0x{byte:02x} at index {index}")
            }
            Self::EmptySourceObjectKey => f.write_str("source object key must not be empty"),
            Self::EmptySemanticRole => f.write_str("semantic role must not be empty"),
            Self::InvalidSemanticRole { index, byte } => {
                write!(
                    f,
                    "invalid semantic role byte 0x{byte:02x} at index {index}"
                )
            }
            Self::ComponentTooLong => f.write_str("source identity component does not fit u32"),
        }
    }
}

impl Error for SourceIdentityError {}

pub fn derive_source_uuid_v5_v1(
    source_hash: &str,
    adapter_id: &str,
    source_object_key: &str,
    semantic_role: &str,
) -> Result<String, SourceIdentityError> {
    let source_hash = decode_lower_hex_sha256(source_hash)?;
    validate_stable_ascii_id(adapter_id, StableIdKind::Adapter)?;
    if source_object_key.is_empty() {
        return Err(SourceIdentityError::EmptySourceObjectKey);
    }
    validate_stable_ascii_id(semantic_role, StableIdKind::SemanticRole)?;

    let adapter = component_bytes(adapter_id.as_bytes())?;
    let object_key = component_bytes(source_object_key.as_bytes())?;
    let role = component_bytes(semantic_role.as_bytes())?;

    let mut name = Vec::with_capacity(
        1 + source_hash.len() + 4 + adapter.len() + 4 + object_key.len() + 4 + role.len(),
    );
    name.push(SOURCE_DERIVED_FRAMING_VERSION_V1);
    name.extend_from_slice(&source_hash);
    append_length_prefixed(&mut name, adapter);
    append_length_prefixed(&mut name, object_key);
    append_length_prefixed(&mut name, role);

    let namespace = Uuid::from_bytes(SOURCE_DERIVED_NAMESPACE_V1);
    Ok(Uuid::new_v5(&namespace, &name).hyphenated().to_string())
}

pub fn pub_contents_object_key_v1(seq_num: u32) -> String {
    format!("contents/0x2c/seq/{seq_num}")
}

pub fn pub_quill_story_object_key_v1(qsid: u32) -> String {
    format!("quill/syid/{qsid}")
}

pub fn pub_table_cell_object_key_v1(table_seq_num: u32, stored_record_index: u32) -> String {
    format!(
        "contents/0x2c/seq/{table_seq_num}/cells/stored/{stored_record_index}"
    )
}

pub fn derive_pub_page_id_v1(
    source_hash: &str,
    seq_num: u32,
) -> Result<String, SourceIdentityError> {
    derive_source_uuid_v5_v1(
        source_hash,
        PUB_SOURCE_ADAPTER_ID_V1,
        &pub_contents_object_key_v1(seq_num),
        ROLE_PAGE,
    )
}

pub fn derive_pub_node_id_v1(
    source_hash: &str,
    seq_num: u32,
) -> Result<String, SourceIdentityError> {
    derive_source_uuid_v5_v1(
        source_hash,
        PUB_SOURCE_ADAPTER_ID_V1,
        &pub_contents_object_key_v1(seq_num),
        ROLE_NODE,
    )
}

pub fn derive_pub_story_id_v1(source_hash: &str, qsid: u32) -> Result<String, SourceIdentityError> {
    derive_source_uuid_v5_v1(
        source_hash,
        PUB_SOURCE_ADAPTER_ID_V1,
        &pub_quill_story_object_key_v1(qsid),
        ROLE_STORY,
    )
}

pub fn derive_pub_table_cell_id_v1(
    source_hash: &str,
    table_seq_num: u32,
    stored_record_index: u32,
) -> Result<TableCellId, SourceIdentityError> {
    let value = derive_source_uuid_v5_v1(
        source_hash,
        PUB_SOURCE_ADAPTER_ID_V1,
        &pub_table_cell_object_key_v1(table_seq_num, stored_record_index),
        ROLE_TABLE_CELL,
    )?;
    Ok(TableCellId::from_canonical_uuid_string(value))
}

fn decode_lower_hex_sha256(value: &str) -> Result<[u8; 32], SourceIdentityError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(SourceIdentityError::InvalidSourceHash);
    }

    let mut out = [0u8; 32];
    let bytes = value.as_bytes();
    for (index, slot) in out.iter_mut().enumerate() {
        let high = hex_nibble(bytes[index * 2]).ok_or(SourceIdentityError::InvalidSourceHash)?;
        let low = hex_nibble(bytes[index * 2 + 1]).ok_or(SourceIdentityError::InvalidSourceHash)?;
        *slot = (high << 4) | low;
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn component_bytes(bytes: &[u8]) -> Result<&[u8], SourceIdentityError> {
    u32::try_from(bytes.len()).map_err(|_| SourceIdentityError::ComponentTooLong)?;
    Ok(bytes)
}

fn append_length_prefixed(output: &mut Vec<u8>, bytes: &[u8]) {
    let len = u32::try_from(bytes.len()).expect("component_bytes validated length");
    output.extend_from_slice(&len.to_be_bytes());
    output.extend_from_slice(bytes);
}

#[derive(Debug, Clone, Copy)]
enum StableIdKind {
    Adapter,
    SemanticRole,
}

fn validate_stable_ascii_id(value: &str, kind: StableIdKind) -> Result<(), SourceIdentityError> {
    if value.is_empty() {
        return match kind {
            StableIdKind::Adapter => Err(SourceIdentityError::EmptyAdapterId),
            StableIdKind::SemanticRole => Err(SourceIdentityError::EmptySemanticRole),
        };
    }

    for (index, byte) in value.bytes().enumerate() {
        let valid = if index == 0 {
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
                StableIdKind::Adapter => Err(SourceIdentityError::InvalidAdapterId { index, byte }),
                StableIdKind::SemanticRole => {
                    Err(SourceIdentityError::InvalidSemanticRole { index, byte })
                }
            };
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_golden_vector_matches_cdm_source_id_v1() {
        let source_hash = (0u8..32)
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        let id =
            derive_source_uuid_v5_v1(&source_hash, "pub-rs", "contents/0x2c/seq/330", "cdm.story")
                .expect("golden vector");

        assert_eq!(id, "3c3cfd8a-2347-5cf4-977d-34e4f346f6df");
    }

    #[test]
    fn publisher_helpers_are_role_separated_and_deterministic() {
        let source_hash = "11".repeat(32);
        let node = derive_pub_node_id_v1(&source_hash, 437).expect("node");
        let node_again = derive_pub_node_id_v1(&source_hash, 437).expect("node");
        let page = derive_pub_page_id_v1(&source_hash, 437).expect("page");
        let story = derive_pub_story_id_v1(&source_hash, 218).expect("story");
        let cell = derive_pub_table_cell_id_v1(&source_hash, 437, 3).expect("table cell");
        let cell_again = derive_pub_table_cell_id_v1(&source_hash, 437, 3).expect("table cell");

        assert_eq!(node, node_again);
        assert_eq!(cell, cell_again);
        assert_ne!(node, page);
        assert_ne!(node, story);
        assert_ne!(page, story);
        assert_ne!(cell.as_str(), node);
    }

    #[test]
    fn table_cell_helper_matches_historical_object_key_and_role_law() {
        let source_hash = "22".repeat(32);
        let expected = derive_source_uuid_v5_v1(
            &source_hash,
            PUB_SOURCE_ADAPTER_ID_V1,
            "contents/0x2c/seq/900/cells/stored/4",
            "cdm.table_cell",
        )
        .expect("historical law");
        let cell = derive_pub_table_cell_id_v1(&source_hash, 900, 4).expect("cell");
        assert_eq!(cell.as_str(), expected);
    }

    #[test]
    fn uppercase_or_malformed_source_hash_fails_closed() {
        assert_eq!(
            derive_pub_node_id_v1(&"AA".repeat(32), 1),
            Err(SourceIdentityError::InvalidSourceHash)
        );
        assert_eq!(
            derive_pub_node_id_v1("abcd", 1),
            Err(SourceIdentityError::InvalidSourceHash)
        );
    }
}
