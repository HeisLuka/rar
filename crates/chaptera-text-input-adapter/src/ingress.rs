use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

pub const TEXT_INGRESS_VERSION_V1: &str = "chaptera.text-ingress.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalStoryTextV1 {
    pub protocol_version: String,
    pub text: String,
    pub scalar_len: u32,
    pub paragraph_boundary_count: u32,
    pub text_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextIngressError {
    pub message: String,
}

impl TextIngressError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for TextIngressError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TextIngressError {}

fn canonical_result(text: String) -> Result<CanonicalStoryTextV1, TextIngressError> {
    let scalar_len = u32::try_from(text.chars().count())
        .map_err(|_| TextIngressError::new("canonical_text scalar length overflows u32"))?;
    let paragraph_boundary_count = u32::try_from(text.chars().filter(|ch| *ch == '\r').count())
        .map_err(|_| TextIngressError::new("paragraph boundary count overflows u32"))?;
    let mut digest = Sha256::new();
    digest.update(text.as_bytes());
    Ok(CanonicalStoryTextV1 {
        protocol_version: TEXT_INGRESS_VERSION_V1.to_owned(),
        text,
        scalar_len,
        paragraph_boundary_count,
        text_sha256: format!("{:x}", digest.finalize()),
    })
}

pub fn normalize_external_text_v1(
    input_text: &str,
) -> Result<CanonicalStoryTextV1, TextIngressError> {
    let mut out = String::with_capacity(input_text.len());
    let mut chars = input_text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                out.push('\r');
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
            }
            '\n' => out.push('\r'),
            other => out.push(other),
        }
    }
    canonical_result(out)
}

pub fn validate_canonical_fragment_text_v1(
    text: &str,
) -> Result<CanonicalStoryTextV1, TextIngressError> {
    if text.contains('\n') {
        return Err(TextIngressError::new(
            "canonical Story fragment must not contain LF/CRLF delimiters",
        ));
    }
    canonical_result(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_newline_spellings_normalize_once() {
        let value = normalize_external_text_v1("A\r\nB\nC\rD").unwrap();
        assert_eq!(value.text, "A\rB\rC\rD");
        assert_eq!(value.scalar_len, 7);
        assert_eq!(value.paragraph_boundary_count, 3);
    }

    #[test]
    fn normalization_does_not_append_terminal_cr_or_unicode_normalize() {
        let value = normalize_external_text_v1("e\u{301}").unwrap();
        assert_eq!(value.text, "e\u{301}");
        assert_eq!(value.scalar_len, 2);
        assert!(!value.text.ends_with('\r'));
    }

    #[test]
    fn canonical_fragment_rejects_lf_but_preserves_cr() {
        assert!(validate_canonical_fragment_text_v1("A\nB").is_err());
        assert_eq!(
            validate_canonical_fragment_text_v1("A\rB")
                .unwrap()
                .text,
            "A\rB"
        );
    }
}
