use crate::{BLOCK_TYPE_FIXED_8, RawContentsBlock, RawContentsBlockBody};
use pub_core::RawSpan;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Физически подтверждённый 8-byte payload семейства Oid.
///
/// Два DWORD сохраняются как отдельные слова little-endian. Этот тип не
/// объявляет Oid глобально уникальным, не декодирует role/instance и не
/// связывает значение с DwNextUniqueOid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OidIdentityPayload {
    pub dword0: u32,
    pub dword1: u32,
    pub value_source: RawSpan,
    pub block: RawContentsBlock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OidIdentityReadError {
    UnexpectedType { offset: u64, block_type: u8 },
    InconsistentBody { offset: u64 },
}

impl fmt::Display for OidIdentityReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedType { offset, block_type } => write!(
                f,
                "неподтверждённый wire type Oid payload по смещению {offset}: 0x{block_type:02X}"
            ),
            Self::InconsistentBody { offset } => write!(
                f,
                "Oid payload по смещению {offset} не содержит подтверждённые 8 байт"
            ),
        }
    }
}

impl std::error::Error for OidIdentityReadError {}

/// Декодирует только подтверждённый physical type0x28 как два u32.
///
/// Native Publisher 2002 ObjectTracking подтверждает type0x28 для Oid и
/// OidExpectedParent. Field id остаётся class-local и намеренно не
/// проверяется этой функцией.
pub fn parse_confirmed_oid_identity_payload(
    block: RawContentsBlock,
) -> Result<OidIdentityPayload, OidIdentityReadError> {
    if block.block_type != BLOCK_TYPE_FIXED_8 {
        return Err(OidIdentityReadError::UnexpectedType {
            offset: block.source.offset,
            block_type: block.block_type,
        });
    }

    let (bytes, value_source) = match &block.body {
        RawContentsBlockBody::Fixed8 {
            bytes,
            value_source,
        } => (*bytes, value_source.clone()),
        _ => {
            return Err(OidIdentityReadError::InconsistentBody {
                offset: block.source.offset,
            });
        }
    };

    let dword0 = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let dword1 = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);

    Ok(OidIdentityPayload {
        dword0,
        dword1,
        value_source,
        block,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContentsCursor, parse_confirmed_block};
    use pub_core::StreamPath;

    fn parse_block(bytes: &[u8]) -> RawContentsBlock {
        let mut cursor = ContentsCursor::new(StreamPath("/Contents".into()), bytes);
        parse_confirmed_block(&mut cursor).expect("identity block должен читаться")
    }

    #[test]
    fn decodes_observed_oid_pair_without_assigning_allocator_semantics() {
        let bytes = [0x0D, 0x28, 0x02, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00];
        let payload = parse_confirmed_oid_identity_payload(parse_block(&bytes))
            .expect("type0x28 должен декодироваться как Oid pair");

        assert_eq!(payload.dword0, 2);
        assert_eq!(payload.dword1, 7);
        assert_eq!(payload.value_source.offset, 2);
        assert_eq!(payload.value_source.len, 8);
        assert_eq!(payload.block.id, 0x0D);
    }

    #[test]
    fn keeps_field_id_class_local() {
        let bytes = [0x06, 0x28, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00];
        let payload = parse_confirmed_oid_identity_payload(parse_block(&bytes))
            .expect("тот же physical Oid wire не должен зависеть от class-local field id");

        assert_eq!(payload.dword0, 1);
        assert_eq!(payload.dword1, 1);
        assert_eq!(payload.block.id, 0x06);
    }

    #[test]
    fn rejects_u32_as_oid_payload() {
        let bytes = [0x0D, 0x20, 0x02, 0x00, 0x00, 0x00];
        let error = parse_confirmed_oid_identity_payload(parse_block(&bytes))
            .expect_err("u32 нельзя повышать до 8-byte Oid payload");

        assert_eq!(
            error,
            OidIdentityReadError::UnexpectedType {
                offset: 0,
                block_type: 0x20,
            }
        );
    }
}
