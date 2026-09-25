use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const AUTHORING_TEXT_PRESET_VERSION_V1: &str = "chaptera.authoring-text-preset.v1";
pub const MAX_SAFE_EMU_V1: i64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthoringParagraphAlignmentV1 {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringParagraphDefaultsV1 {
    pub alignment: AuthoringParagraphAlignmentV1,
    pub space_before_emu: i64,
    pub space_after_emu: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringCharacterDefaultsV1 {
    pub bold: bool,
    pub italic: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringTextPresetV1 {
    pub preset_version: String,
    pub font_fingerprint: String,
    pub face_index: u32,
    pub font_size_emu: i64,
    pub paragraph_defaults: AuthoringParagraphDefaultsV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub character_defaults: Option<AuthoringCharacterDefaultsV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringTextPresetRecordV1 {
    pub preset_id: String,
    pub preset: AuthoringTextPresetV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthoringTextShapingInputV1 {
    pub preset_id: String,
    pub font_fingerprint: String,
    pub face_index: u32,
    pub font_size_emu: i64,
    pub paragraph_defaults: AuthoringParagraphDefaultsV1,
    pub character_defaults: Option<AuthoringCharacterDefaultsV1>,
    pub text: String,
    pub layout_environment_fingerprint: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthoringTextPresetError {
    UnsupportedPresetVersion,
    InvalidFontFingerprint,
    EmptyFontResource,
    FontFingerprintMismatch,
    InvalidFontSizeEmu,
    InvalidParagraphSpacingEmu,
    SerializationFailed,
}

impl AuthoringTextPresetV1 {
    pub fn new(
        font_fingerprint: String,
        face_index: u32,
        font_size_emu: i64,
        paragraph_defaults: AuthoringParagraphDefaultsV1,
        character_defaults: Option<AuthoringCharacterDefaultsV1>,
    ) -> Result<Self, AuthoringTextPresetError> {
        let preset = Self {
            preset_version: AUTHORING_TEXT_PRESET_VERSION_V1.to_owned(),
            font_fingerprint,
            face_index,
            font_size_emu,
            paragraph_defaults,
            character_defaults,
        };
        preset.validate()?;
        Ok(preset)
    }

    pub fn validate(&self) -> Result<(), AuthoringTextPresetError> {
        if self.preset_version != AUTHORING_TEXT_PRESET_VERSION_V1 {
            return Err(AuthoringTextPresetError::UnsupportedPresetVersion);
        }
        if !is_lower_sha256_hex(&self.font_fingerprint) {
            return Err(AuthoringTextPresetError::InvalidFontFingerprint);
        }
        if self.font_size_emu <= 0 || self.font_size_emu > MAX_SAFE_EMU_V1 {
            return Err(AuthoringTextPresetError::InvalidFontSizeEmu);
        }
        for spacing in [
            self.paragraph_defaults.space_before_emu,
            self.paragraph_defaults.space_after_emu,
        ] {
            if !(0..=MAX_SAFE_EMU_V1).contains(&spacing) {
                return Err(AuthoringTextPresetError::InvalidParagraphSpacingEmu);
            }
        }
        Ok(())
    }
}

pub fn font_fingerprint_v1(font_bytes: &[u8]) -> Result<String, AuthoringTextPresetError> {
    if font_bytes.is_empty() {
        return Err(AuthoringTextPresetError::EmptyFontResource);
    }
    Ok(lower_hex(&Sha256::digest(font_bytes)))
}

pub fn validate_font_resource_v1(
    preset: &AuthoringTextPresetV1,
    font_bytes: &[u8],
) -> Result<(), AuthoringTextPresetError> {
    preset.validate()?;
    let actual = font_fingerprint_v1(font_bytes)?;
    if actual != preset.font_fingerprint {
        return Err(AuthoringTextPresetError::FontFingerprintMismatch);
    }
    Ok(())
}

pub fn authoring_text_preset_id_v1(
    preset: &AuthoringTextPresetV1,
) -> Result<String, AuthoringTextPresetError> {
    preset.validate()?;
    let canonical =
        serde_json::to_vec(preset).map_err(|_| AuthoringTextPresetError::SerializationFailed)?;
    let mut hasher = Sha256::new();
    hasher.update(b"chaptera-authoring-text-preset-id-v1\0");
    hasher.update(canonical);
    Ok(format!("sha256:{}", lower_hex(&hasher.finalize())))
}

pub fn authoring_text_preset_record_v1(
    preset: &AuthoringTextPresetV1,
) -> Result<AuthoringTextPresetRecordV1, AuthoringTextPresetError> {
    Ok(AuthoringTextPresetRecordV1 {
        preset_id: authoring_text_preset_id_v1(preset)?,
        preset: preset.clone(),
    })
}

pub fn authoring_text_shaping_input_v1(
    preset: &AuthoringTextPresetV1,
    font_bytes: &[u8],
    text: &str,
    layout_environment_fingerprint: [u8; 32],
) -> Result<AuthoringTextShapingInputV1, AuthoringTextPresetError> {
    validate_font_resource_v1(preset, font_bytes)?;
    Ok(AuthoringTextShapingInputV1 {
        preset_id: authoring_text_preset_id_v1(preset)?,
        font_fingerprint: preset.font_fingerprint.clone(),
        face_index: preset.face_index,
        font_size_emu: preset.font_size_emu,
        paragraph_defaults: preset.paragraph_defaults.clone(),
        character_defaults: preset.character_defaults.clone(),
        text: text.to_owned(),
        layout_environment_fingerprint,
    })
}

fn is_lower_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font_bytes() -> &'static [u8] {
        b"explicit-pinned-font-resource-v1"
    }

    fn preset() -> AuthoringTextPresetV1 {
        AuthoringTextPresetV1::new(
            font_fingerprint_v1(font_bytes()).expect("font fingerprint"),
            0,
            12 * 12_700,
            AuthoringParagraphDefaultsV1 {
                alignment: AuthoringParagraphAlignmentV1::Left,
                space_before_emu: 0,
                space_after_emu: 0,
            },
            Some(AuthoringCharacterDefaultsV1 {
                bold: false,
                italic: false,
            }),
        )
        .expect("valid preset")
    }

    #[test]
    fn record_and_serialization_are_deterministic() {
        let preset = preset();
        let first = authoring_text_preset_record_v1(&preset).expect("record");
        let second = authoring_text_preset_record_v1(&preset).expect("record");
        assert_eq!(first, second);
        assert!(first.preset_id.starts_with("sha256:"));
        assert_eq!(first.preset_id.len(), 71);

        let encoded = serde_json::to_vec(&first).expect("serialize record");
        let decoded: AuthoringTextPresetRecordV1 =
            serde_json::from_slice(&encoded).expect("deserialize record");
        assert_eq!(first, decoded);
        decoded.preset.validate().expect("roundtrip preset");
    }

    #[test]
    fn same_explicit_inputs_produce_identical_shaping_inputs() {
        let preset = preset();
        let environment = [0x5a; 32];
        let first = authoring_text_shaping_input_v1(&preset, font_bytes(), "Chaptera", environment)
            .expect("first shaping input");
        let second =
            authoring_text_shaping_input_v1(&preset, font_bytes(), "Chaptera", environment)
                .expect("second shaping input");
        assert_eq!(first, second);

        let changed_environment =
            authoring_text_shaping_input_v1(&preset, font_bytes(), "Chaptera", [0x6b; 32])
                .expect("changed environment");
        assert_ne!(first, changed_environment);
    }

    #[test]
    fn font_bytes_are_exact_authority_and_host_fonts_are_not_inputs() {
        let preset = preset();
        assert_eq!(
            validate_font_resource_v1(&preset, b"different-font"),
            Err(AuthoringTextPresetError::FontFingerprintMismatch)
        );

        let shaping = authoring_text_shaping_input_v1(&preset, font_bytes(), "Text", [7; 32])
            .expect("shaping input");
        assert_eq!(shaping.font_fingerprint, preset.font_fingerprint);
        assert_eq!(shaping.face_index, 0);
    }

    #[test]
    fn invalid_size_fingerprint_spacing_and_version_fail_closed() {
        let mut bad_size = preset();
        bad_size.font_size_emu = 0;
        assert_eq!(
            bad_size.validate(),
            Err(AuthoringTextPresetError::InvalidFontSizeEmu)
        );

        let mut negative_spacing = preset();
        negative_spacing.paragraph_defaults.space_before_emu = -1;
        assert_eq!(
            negative_spacing.validate(),
            Err(AuthoringTextPresetError::InvalidParagraphSpacingEmu)
        );

        let mut bad_fingerprint = preset();
        bad_fingerprint.font_fingerprint = "ABC".to_owned();
        assert_eq!(
            bad_fingerprint.validate(),
            Err(AuthoringTextPresetError::InvalidFontFingerprint)
        );

        let mut future = preset();
        future.preset_version = "chaptera.authoring-text-preset.v2".to_owned();
        assert_eq!(
            future.validate(),
            Err(AuthoringTextPresetError::UnsupportedPresetVersion)
        );
    }

    #[test]
    fn unknown_fields_fail_deserialization() {
        let mut value = serde_json::to_value(preset()).expect("serialize");
        value
            .as_object_mut()
            .expect("preset object")
            .insert("publisher_stsh_id".to_owned(), serde_json::json!(123));
        assert!(serde_json::from_value::<AuthoringTextPresetV1>(value).is_err());
    }

    #[test]
    fn empty_story_can_bind_preset_without_manufacturing_text() {
        let preset = preset();
        let shaping = authoring_text_shaping_input_v1(&preset, font_bytes(), "", [0; 32])
            .expect("empty Story shaping input");
        assert!(shaping.text.is_empty());

        let serialized = serde_json::to_string(&preset).expect("serialize preset");
        let lower = serialized.to_ascii_lowercase();
        assert!(!lower.contains("stsh"));
        assert!(!lower.contains("bte"));
        assert!(!lower.contains("syid"));
    }

    #[test]
    fn empty_font_resource_is_rejected() {
        let preset = preset();
        assert_eq!(
            validate_font_resource_v1(&preset, b""),
            Err(AuthoringTextPresetError::EmptyFontResource)
        );
    }
}
