use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

/// Точное десятичное значение для deterministic CDM state.
///
/// Значение хранится как нормализованная decimal string без exponent notation.
/// Арифметика намеренно не входит в этот primitive: layout engine может выбрать
/// отдельное exact/rational представление, не меняя сериализационный контракт.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Decimal(String);

impl Decimal {
    pub fn zero() -> Self {
        Self("0".to_owned())
    }

    pub fn one() -> Self {
        Self("1".to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecimalParseError {
    Empty,
    InvalidSyntax { index: usize },
}

impl fmt::Display for DecimalParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("Decimal не может быть пустым"),
            Self::InvalidSyntax { index } => {
                write!(formatter, "невалидный Decimal по позиции {index}")
            }
        }
    }
}

impl std::error::Error for DecimalParseError {}

impl FromStr for Decimal {
    type Err = DecimalParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.is_empty() {
            return Err(DecimalParseError::Empty);
        }

        let bytes = input.as_bytes();
        let negative = bytes[0] == b'-';
        let body_start = usize::from(negative);

        if body_start == bytes.len() {
            return Err(DecimalParseError::InvalidSyntax { index: 0 });
        }

        let body = &input[body_start..];
        let mut dot_index = None;

        for (relative_index, byte) in body.bytes().enumerate() {
            let absolute_index = body_start + relative_index;
            match byte {
                b'0'..=b'9' => {}
                b'.' if dot_index.is_none() => dot_index = Some(relative_index),
                _ => {
                    return Err(DecimalParseError::InvalidSyntax {
                        index: absolute_index,
                    });
                }
            }
        }

        let (integer, fraction) = match dot_index {
            Some(index) => (&body[..index], Some(&body[index + 1..])),
            None => (body, None),
        };

        if integer.is_empty() {
            return Err(DecimalParseError::InvalidSyntax { index: body_start });
        }
        if fraction.is_some_and(str::is_empty) {
            return Err(DecimalParseError::InvalidSyntax {
                index: input.len() - 1,
            });
        }

        let integer = integer.trim_start_matches('0');
        let integer = if integer.is_empty() { "0" } else { integer };
        let fraction = fraction
            .map(|value| value.trim_end_matches('0'))
            .filter(|value| !value.is_empty());

        let is_zero = integer == "0" && fraction.is_none();
        let mut normalized = String::new();
        if negative && !is_zero {
            normalized.push('-');
        }
        normalized.push_str(integer);
        if let Some(fraction) = fraction {
            normalized.push('.');
            normalized.push_str(fraction);
        }

        Ok(Self(normalized))
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for Decimal {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_normalizes_without_binary_float() {
        let value: Decimal = "-0012.3400"
            .parse()
            .expect("валидная decimal string должна разбираться");
        assert_eq!(value.as_str(), "-12.34");
    }

    #[test]
    fn decimal_normalizes_negative_zero() {
        let value: Decimal = "-0.000"
            .parse()
            .expect("нулевое decimal value должно разбираться");
        assert_eq!(value.as_str(), "0");
    }

    #[test]
    fn decimal_rejects_exponent_notation() {
        assert!(matches!(
            "1e3".parse::<Decimal>(),
            Err(DecimalParseError::InvalidSyntax { .. })
        ));
    }
}
