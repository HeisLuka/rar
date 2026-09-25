use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TableCellId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableCellIdError {
    InvalidCanonicalUuid,
}

impl fmt::Display for TableCellIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("table cell id must be canonical hyphenated UUID text")
    }
}

impl std::error::Error for TableCellIdError {}

impl TableCellId {
    pub fn parse(value: &str) -> Result<Self, TableCellIdError> {
        let bytes = value.as_bytes();
        if bytes.len() != 36 {
            return Err(TableCellIdError::InvalidCanonicalUuid);
        }
        for (index, byte) in bytes.iter().copied().enumerate() {
            if matches!(index, 8 | 13 | 18 | 23) {
                if byte != b'-' {
                    return Err(TableCellIdError::InvalidCanonicalUuid);
                }
            } else if !(byte.is_ascii_digit()
                || (b'a'..=b'f').contains(&byte)
                || (b'A'..=b'F').contains(&byte))
            {
                return Err(TableCellIdError::InvalidCanonicalUuid);
            }
        }
        Ok(Self(value.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn from_canonical_uuid_string(value: String) -> Self {
        debug_assert!(Self::parse(&value).is_ok());
        Self(value)
    }
}

impl fmt::Display for TableCellId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for TableCellId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for TableCellId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_cell_id_preserves_historical_canonical_uuid_wire() {
        let id = TableCellId::parse("00112233-4455-6677-8899-AABBCCDDEEFF").expect("valid id");
        assert_eq!(id.as_str(), "00112233-4455-6677-8899-aabbccddeeff");
        assert_eq!(
            serde_json::to_string(&id).expect("serialize"),
            "\"00112233-4455-6677-8899-aabbccddeeff\""
        );
        let reopened: TableCellId =
            serde_json::from_str("\"00112233-4455-6677-8899-aabbccddeeff\"").expect("read");
        assert_eq!(reopened, id);
    }

    #[test]
    fn malformed_identity_fails_closed() {
        for value in [
            "",
            "00112233445566778899aabbccddeeff",
            "00112233-4455-6677-8899-aabbccddeezz",
        ] {
            assert_eq!(
                TableCellId::parse(value),
                Err(TableCellIdError::InvalidCanonicalUuid)
            );
        }
    }
}
