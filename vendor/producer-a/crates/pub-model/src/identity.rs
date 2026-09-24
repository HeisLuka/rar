use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

/// Стабильный 128-битный идентификатор сущности CDM.
///
/// Этот тип фиксирует только представление идентичности. Политика создания
/// source-derived и editor-created IDs относится к более высокому слою.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalId([u8; 16]);

impl CanonicalId {
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    pub const fn into_bytes(self) -> [u8; 16] {
        self.0
    }
}

impl fmt::Display for CanonicalId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = self.0;
        write!(
            formatter,
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            b[0],
            b[1],
            b[2],
            b[3],
            b[4],
            b[5],
            b[6],
            b[7],
            b[8],
            b[9],
            b[10],
            b[11],
            b[12],
            b[13],
            b[14],
            b[15],
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalIdParseError {
    InvalidLength,
    InvalidHyphen { index: usize },
    InvalidHex { index: usize },
}

impl fmt::Display for CanonicalIdParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength => formatter.write_str("CanonicalId должен содержать 36 символов"),
            Self::InvalidHyphen { index } => {
                write!(
                    formatter,
                    "в CanonicalId отсутствует дефис по позиции {index}"
                )
            }
            Self::InvalidHex { index } => {
                write!(
                    formatter,
                    "в CanonicalId невалидная hex-цифра по позиции {index}"
                )
            }
        }
    }
}

impl std::error::Error for CanonicalIdParseError {}

impl FromStr for CanonicalId {
    type Err = CanonicalIdParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.len() != 36 {
            return Err(CanonicalIdParseError::InvalidLength);
        }

        let bytes = input.as_bytes();
        for index in [8, 13, 18, 23] {
            if bytes[index] != b'-' {
                return Err(CanonicalIdParseError::InvalidHyphen { index });
            }
        }

        let mut output = [0_u8; 16];
        let mut source_index = 0;
        let mut output_index = 0;

        while source_index < bytes.len() {
            if matches!(source_index, 8 | 13 | 18 | 23) {
                source_index += 1;
                continue;
            }

            let high = hex_value(bytes[source_index]).ok_or(CanonicalIdParseError::InvalidHex {
                index: source_index,
            })?;
            let low =
                hex_value(bytes[source_index + 1]).ok_or(CanonicalIdParseError::InvalidHex {
                    index: source_index + 1,
                })?;

            output[output_index] = (high << 4) | low;
            output_index += 1;
            source_index += 2;
        }

        Ok(Self(output))
    }
}

impl Serialize for CanonicalId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for CanonicalId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(D::Error::custom)
    }
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_id_is_exactly_128_bits() {
        assert_eq!(std::mem::size_of::<CanonicalId>(), 16);
    }

    #[test]
    fn canonical_id_parses_and_formats_canonical_uuid_text() {
        let id: CanonicalId = "00112233-4455-6677-8899-aabbccddeeff"
            .parse()
            .expect("валидный UUID-текст должен разбираться");

        assert_eq!(id.to_string(), "00112233-4455-6677-8899-aabbccddeeff");
        assert_eq!(
            id.into_bytes(),
            [
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff,
            ]
        );
    }

    #[test]
    fn canonical_id_accepts_uppercase_but_formats_lowercase() {
        let id: CanonicalId = "00112233-4455-6677-8899-AABBCCDDEEFF"
            .parse()
            .expect("uppercase hex должен быть допустим");

        assert_eq!(id.to_string(), "00112233-4455-6677-8899-aabbccddeeff");
    }
}
